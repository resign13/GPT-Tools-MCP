use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::io::AsyncReadExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::Child;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

#[cfg(windows)]
use crate::security::exec_sandbox::{SandboxChild, SandboxProcess};
use crate::tools::context::ToolContext;
use crate::tools::workspace::{tool_ok, WorkspaceError};
use serde_json::{json, Value};

const SESSION_BUFFER_BYTES: usize = 1_048_576;
const SESSION_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) type SessionStdin = Box<dyn AsyncWrite + Send + Unpin>;
type SessionReader = Box<dyn AsyncRead + Send + Unpin>;

enum ManagedProcess {
    Tokio(Child),
    #[cfg(windows)]
    Sandbox(SandboxProcess),
}

pub(crate) struct ManagedChild {
    process: ManagedProcess,
    stdin: Option<SessionStdin>,
    stdout: Option<SessionReader>,
    stderr: Option<SessionReader>,
}

impl ManagedChild {
    fn from_tokio(mut child: Child) -> Self {
        let stdin = child
            .stdin
            .take()
            .map(|stream| Box::new(stream) as SessionStdin);
        let stdout = child
            .stdout
            .take()
            .map(|stream| Box::new(stream) as SessionReader);
        let stderr = child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as SessionReader);
        Self {
            process: ManagedProcess::Tokio(child),
            stdin,
            stdout,
            stderr,
        }
    }

    pub(crate) fn from_sandbox(child: SandboxChild) -> Self {
        let (process, stdin, stdout, stderr) = child.into_parts();
        Self {
            process: ManagedProcess::Sandbox(process),
            stdin: stdin.map(|stream| Box::new(stream) as SessionStdin),
            stdout: stdout.map(|stream| Box::new(stream) as SessionReader),
            stderr: stderr.map(|stream| Box::new(stream) as SessionReader),
        }
    }

    fn take_stdin(&mut self) -> Option<SessionStdin> {
        self.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<SessionReader> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<SessionReader> {
        self.stderr.take()
    }

    fn id(&self) -> Option<u32> {
        match &self.process {
            ManagedProcess::Tokio(child) => child.id(),
            #[cfg(windows)]
            ManagedProcess::Sandbox(process) => process.id(),
        }
    }

    fn start_kill(&mut self) -> io::Result<()> {
        match &mut self.process {
            ManagedProcess::Tokio(child) => child.start_kill(),
            #[cfg(windows)]
            ManagedProcess::Sandbox(process) => process.kill_tree(),
        }
    }

    async fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        match &mut self.process {
            ManagedProcess::Tokio(child) => child.wait().await,
            #[cfg(windows)]
            ManagedProcess::Sandbox(process) => process.wait().await,
        }
    }

    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        match &mut self.process {
            ManagedProcess::Tokio(child) => child.try_wait(),
            #[cfg(windows)]
            ManagedProcess::Sandbox(process) => process.try_wait(),
        }
    }

    #[cfg(windows)]
    fn has_job(&self) -> bool {
        matches!(self.process, ManagedProcess::Sandbox(_))
    }
}

#[derive(Default)]
pub struct SessionStore {
    sessions: Mutex<HashMap<String, Arc<ExecSession>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionShutdownReport {
    pub requested: usize,
    pub terminated: usize,
    pub timed_out: usize,
}

impl SessionShutdownReport {
    pub fn remaining(self) -> usize {
        self.requested.saturating_sub(self.terminated)
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: ExecSession) -> Arc<ExecSession> {
        let arc = Arc::new(session);
        self.sessions
            .lock()
            .expect("sessions lock")
            .insert(arc.session_id.clone(), arc.clone());
        arc
    }

    pub(crate) async fn refresh_host_completions(&self) {
        let sessions = self.sessions.lock().expect("sessions lock").values().cloned().collect::<Vec<_>>();
        for session in sessions {
            if session.execution_boundary == "host" { session.refresh_status().await; }
        }
    }

    pub fn get(&self, session_id: &str) -> Result<Arc<ExecSession>, WorkspaceError> {
        self.sessions
            .lock()
            .expect("sessions lock")
            .get(session_id)
            .cloned()
            .ok_or_else(|| WorkspaceError::Tool {
                code: "SESSION_NOT_FOUND",
                message: format!("Session not found: {session_id}"),
                category: "not_found",
                retryable: false,
            })
    }

    pub fn remove(&self, session_id: &str) {
        self.sessions
            .lock()
            .expect("sessions lock")
            .remove(session_id);
    }

    pub fn has_active_sessions(&self) -> bool {
        self.sessions
            .lock()
            .expect("sessions lock")
            .values()
            .any(|session| !session.has_exited())
    }

    /// Stop every session owned by one execution context. The map is
    /// snapshotted before awaiting so no store lock is held while a process or
    /// reader task is being terminated.
    pub async fn shutdown_context(
        &self,
        execution_fingerprint: &str,
        reason: &str,
    ) -> Result<SessionShutdownReport, WorkspaceError> {
        self.shutdown_matching(Some(execution_fingerprint), reason).await
    }

    /// Stop all sessions before a listener/context is dropped. A timed-out
    /// process is retained by a background reaper instead of being forgotten,
    /// so dropping the ToolContext cannot orphan its child process.
    pub async fn shutdown_all(
        &self,
        reason: &str,
    ) -> Result<SessionShutdownReport, WorkspaceError> {
        self.shutdown_matching(None, reason).await
    }

    async fn shutdown_matching(
        &self,
        execution_fingerprint: Option<&str>,
        reason: &str,
    ) -> Result<SessionShutdownReport, WorkspaceError> {
        let sessions = {
            let guard = self.sessions.lock().map_err(|_| WorkspaceError::Tool {
                code: "SESSION_STORE_UNAVAILABLE",
                message: "Session store is unavailable during shutdown.".into(),
                category: "runtime",
                retryable: true,
            })?;
            guard
                .iter()
                .filter(|(_, session)| {
                    execution_fingerprint
                        .map(|fingerprint| session.belongs_to_context(fingerprint))
                        .unwrap_or(true)
                })
                .map(|(session_id, session)| (session_id.clone(), session.clone()))
                .collect::<Vec<_>>()
        };

        let mut report = SessionShutdownReport {
            requested: sessions.len(),
            terminated: 0,
            timed_out: 0,
        };
        for (session_id, session) in sessions {
            if session.shutdown_and_wait(reason, SESSION_SHUTDOWN_TIMEOUT).await {
                report.terminated += 1;
                let mut guard = self.sessions.lock().map_err(|_| WorkspaceError::Tool {
                    code: "SESSION_STORE_UNAVAILABLE",
                    message: "Session store is unavailable during shutdown.".into(),
                    category: "runtime",
                    retryable: true,
                })?;
                if guard
                    .get(&session_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &session))
                {
                    guard.remove(&session_id);
                }
            } else {
                report.timed_out += 1;
                reap_session_in_background(session);
            }
        }
        Ok(report)
    }
}

pub struct ExecSession {
    pub session_id: String,
    pub(crate) child: AsyncMutex<ManagedChild>,
    pub(crate) stdin: AsyncMutex<Option<SessionStdin>>,
    stdin_open: Mutex<bool>,
    interactive: bool,
    stdout: Mutex<Vec<u8>>,
    stderr: Mutex<Vec<u8>>,
    stdout_total: Mutex<usize>,
    stderr_total: Mutex<usize>,
    pub started_at: Instant,
    pub exit_code: Mutex<Option<i32>>,
    exited: AtomicBool,
    termination_reason: Mutex<Option<String>>,
    reader_tasks: AsyncMutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
    execution_fingerprint: String,
    sandbox_enforced: bool,
    execution_boundary: &'static str,
    host_completion: Mutex<Option<(crate::harness::Harness, String, crate::tools::execution_context::ExecutionContext)>>,
}

impl ExecSession {
    pub fn new(child: Child) -> Self {
        Self::new_with_mode(child, false)
    }

    pub fn new_with_mode(child: Child, interactive: bool) -> Self {
        Self::new_with_mode_and_fingerprint(child, interactive, String::new())
    }

    pub fn new_with_mode_and_fingerprint(
        child: Child,
        interactive: bool,
        execution_fingerprint: String,
    ) -> Self {
        Self::new_inner(
            ManagedChild::from_tokio(child),
            interactive,
            execution_fingerprint,
            false,
            "policy_only",
        )
    }

    #[cfg(windows)]
    pub(crate) fn new_with_sandbox_child(
        child: SandboxChild,
        interactive: bool,
        execution_fingerprint: String,
    ) -> Self {
        Self::new_inner(
            ManagedChild::from_sandbox(child),
            interactive,
            execution_fingerprint,
            true,
            "windows_appcontainer",
        )
    }

    #[cfg(windows)]
    pub(crate) fn new_with_host_child(child: SandboxChild, interactive: bool, fingerprint: String) -> Self {
        Self::new_inner(ManagedChild::from_sandbox(child), interactive, fingerprint, false, "host")
    }

    fn new_inner(
        mut managed_child: ManagedChild,
        interactive: bool,
        execution_fingerprint: String,
        sandbox_enforced: bool,
        execution_boundary: &'static str,
    ) -> Self {
        let session_id = Uuid::new_v4().to_string();
        let stdin = managed_child.take_stdin();
        let stdin_open = stdin.is_some();
        Self {
            session_id,
            child: AsyncMutex::new(managed_child),
            stdin: AsyncMutex::new(stdin),
            stdin_open: Mutex::new(stdin_open),
            interactive,
            stdout: Mutex::new(Vec::new()),
            stderr: Mutex::new(Vec::new()),
            stdout_total: Mutex::new(0),
            stderr_total: Mutex::new(0),
            started_at: Instant::now(),
            exit_code: Mutex::new(None),
            exited: AtomicBool::new(false),
            termination_reason: Mutex::new(None),
            reader_tasks: AsyncMutex::new(Vec::new()),
            execution_fingerprint,
            sandbox_enforced,
            execution_boundary,
            host_completion: Mutex::new(None),
        }
    }

    pub(crate) fn track_host_completion(&self, ctx: &ToolContext) {
        if let Ok(Some(task)) = ctx.harness.current_task() {
            *self.host_completion.lock().expect("completion lock") = Some((ctx.harness.clone(), task.id, ctx.execution_snapshot()));
        }
    }

    fn belongs_to_context(&self, fingerprint: &str) -> bool {
        self.execution_fingerprint.is_empty() || self.execution_fingerprint == fingerprint
    }

    pub async fn spawn_readers(self: &Arc<Self>) {
        let stdout = {
            let mut guard = self.child.lock().await;
            guard.take_stdout()
        };
        let stderr = {
            let mut guard = self.child.lock().await;
            guard.take_stderr()
        };
        if let Some(stream) = stdout {
            let session = Arc::clone(self);
            let task = tauri::async_runtime::spawn(async move {
                session.read_stream(stream, true).await;
            });
            self.reader_tasks.lock().await.push(task);
        }
        if let Some(stream) = stderr {
            let session = Arc::clone(self);
            let task = tauri::async_runtime::spawn(async move {
                session.read_stream(stream, false).await;
            });
            self.reader_tasks.lock().await.push(task);
        }
    }

    pub async fn wait_for_readers(&self) {
        let mut tasks = self.reader_tasks.lock().await;
        while let Some(task) = tasks.pop() {
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), task).await;
        }
    }

    async fn read_stream<T>(&self, mut stream: T, is_stdout: bool)
    where
        T: tokio::io::AsyncRead + Unpin,
    {
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    if is_stdout {
                        let mut data = self.stdout.lock().expect("stdout lock");
                        data.extend_from_slice(chunk);
                        *self.stdout_total.lock().expect("stdout_total lock") += n;
                        trim_buffer(&mut data, SESSION_BUFFER_BYTES);
                    } else {
                        let mut data = self.stderr.lock().expect("stderr lock");
                        data.extend_from_slice(chunk);
                        *self.stderr_total.lock().expect("stderr_total lock") += n;
                        trim_buffer(&mut data, SESSION_BUFFER_BYTES);
                    }
                }
                Err(_) => break,
            }
        }
    }

    pub async fn kill_and_wait(&self) {
        let _ = self
            .terminate_and_wait(None, std::time::Duration::from_secs(30))
            .await;
    }

    async fn shutdown_and_wait(&self, reason: &str, timeout: std::time::Duration) -> bool {
        self.terminate_and_wait(Some(reason), timeout).await
    }

    async fn terminate_and_wait(
        &self,
        reason: Option<&str>,
        timeout: std::time::Duration,
    ) -> bool {
        if let Some(reason) = reason {
            self.mark_termination_reason(reason);
        }
        let result = tokio::time::timeout(timeout, async {
            let mut child = self.child.lock().await;
            let _ = child.start_kill();
            child.wait().await
        })
        .await;
        match result {
            Ok(Ok(status)) => {
                self.record_exit_status(status);
                self.wait_for_readers().await;
                true
            }
            Ok(Err(_)) => {
                self.refresh_status().await;
                self.wait_for_readers().await;
                self.has_exited()
            }
            Err(_) => {
                self.refresh_status().await;
                false
            }
        }
    }

    pub async fn refresh_status(&self) {
        let mut child = self.child.lock().await;
        if let Ok(Some(status)) = child.try_wait() {
            // A completed host command must not leave orphan descendants holding
            // pipe readers (and their Arc<ExecSession>/Job) alive after eviction.
            if self.execution_boundary == "host" && !self.has_exited() {
                let _ = child.start_kill();
            }
            self.record_exit_status(status);
        }
    }

    fn record_exit_status(&self, status: std::process::ExitStatus) {
        if let Some((harness, task_id, mut execution)) = self.host_completion.lock().expect("completion lock").take() {
            if execution.validate().is_ok() {
                let _ = harness.refresh_host_expected_state(&task_id);
            }
        }
        *self.exit_code.lock().expect("exit_code lock") = status.code();
        self.exited.store(true, Ordering::Release);
        *self.stdin_open.lock().expect("stdin_open lock") = false;
        let mut reason = self.termination_reason.lock().expect("termination lock");
        if reason.is_none() {
            *reason = Some("exited".into());
        }
    }

    pub(crate) fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    pub fn mark_termination_reason(&self, reason: &str) {
        *self.termination_reason.lock().expect("termination lock") = Some(reason.to_string());
    }

    pub(crate) fn mark_stdin_closed(&self) {
        *self.stdin_open.lock().expect("stdin_open lock") = false;
    }

    pub async fn is_running(&self) -> bool {
        self.refresh_status().await;
        !self.has_exited()
    }

    pub fn retained_stream_bytes(&self, stream: &str) -> (Vec<u8>, usize) {
        match stream {
            "stderr" => {
                let data = self.stderr.lock().expect("stderr lock").clone();
                let total = *self.stderr_total.lock().expect("stderr_total lock");
                (data, total)
            }
            _ => {
                let data = self.stdout.lock().expect("stdout lock").clone();
                let total = *self.stdout_total.lock().expect("stdout_total lock");
                (data, total)
            }
        }
    }

    pub fn snapshot(&self, max_output_bytes: usize) -> Value {
        let stdout_bytes = self.stdout.lock().expect("stdout lock").clone();
        let stderr_bytes = self.stderr.lock().expect("stderr lock").clone();
        let stdout = truncate_tail(&stdout_bytes, max_output_bytes);
        let stderr = truncate_tail(&stderr_bytes, max_output_bytes);
        let exit_code = *self.exit_code.lock().expect("exit_code lock");
        let termination_reason = self
            .termination_reason
            .lock()
            .expect("termination lock")
            .clone();
        let status = if self.has_exited() {
            "exited"
        } else {
            "running"
        };
        let reason = termination_reason.as_deref().unwrap_or("running");
        let command_ok = match reason {
            "exited" => Some(exit_code.is_some_and(|code| code == 0)),
            "running" => None,
            _ => Some(false),
        };
        json!({
            "session_id": self.session_id,
            "interactive": self.interactive,
            "stdin_open": *self.stdin_open.lock().expect("stdin_open lock"),
            "status": status,
            "termination_reason": reason,
            "recoverable": matches!(reason, "timeout" | "killed" | "spawn_failed" | "server_restart"),
            "suggestion": match reason {
                "timeout" => "读取保留输出，调整 timeout_ms 后重试",
                "killed" => "确认终止原因后重新执行命令",
                "exited" => "检查 exit_code 和 stderr",
                "crashed" => "检查 stderr 后重试或恢复工作区",
                _ => "继续读取 session 或等待进程结束",
            },
            "exit_code": exit_code,
            "transport_ok": true,
            "command_ok": command_ok,
            "sandbox_enforced": self.sandbox_enforced,
            "execution_boundary": self.execution_boundary,
            "stdout": stdout.content,
            "stderr": stderr.content,
            "stdout_truncated": stdout.truncated,
            "stderr_truncated": stderr.truncated,
            "elapsed_ms": self.started_at.elapsed().as_millis(),
            "output_refs": {
                "stdout": format!("session:{}:stdout", self.session_id),
                "stderr": format!("session:{}:stderr", self.session_id)
            }
        })
    }
}

fn trim_buffer(buf: &mut Vec<u8>, limit: usize) {
    if buf.len() > limit {
        let drop = buf.len() - limit;
        buf.drain(..drop);
    }
}

struct Truncated {
    content: String,
    truncated: bool,
}

fn truncate_tail(bytes: &[u8], max_bytes: usize) -> Truncated {
    let truncated = bytes.len() > max_bytes;
    let take = bytes.len().min(max_bytes);
    Truncated {
        content: String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(take)..]).into_owned(),
        truncated,
    }
}

pub fn read_output(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    let output_ref = args
        .get("output_ref")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkspaceError::invalid_argument("output_ref is required"))?;
    let parts: Vec<&str> = output_ref.split(':').collect();
    if parts.len() != 3 || parts[0] != "session" {
        return Err(WorkspaceError::invalid_argument(
            "output_ref must look like session:<id>:stdout, session:<id>:stderr, or session:<id>:full",
        ));
    }
    let session_id = parts[1];
    let ref_stream = parts[2];
    if ref_stream != "stdout" && ref_stream != "stderr" && ref_stream != "full" {
        return Err(WorkspaceError::invalid_argument(
            "output_ref stream must be stdout, stderr, or full",
        ));
    }
    let session = ctx.sessions.get(session_id)?;
    ensure_context_match(ctx, &session)?;
    tauri::async_runtime::block_on(session.refresh_status());

    let requested_stream = args.get("stream").and_then(Value::as_str).unwrap_or("");
    let stream = if ref_stream == "stdout" || ref_stream == "stderr" {
        ref_stream
    } else if requested_stream == "stdout" || requested_stream == "stderr" {
        requested_stream
    } else {
        "stdout"
    };

    let (data, total_stream_bytes) = session.retained_stream_bytes(stream);
    let requested_offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(4096)
        .clamp(1, 1_048_576) as usize;
    let buffer_offset = requested_offset.min(data.len());
    let chunk = &data[buffer_offset..data.len().min(buffer_offset + limit)];
    let next_offset = if buffer_offset + chunk.len() < total_stream_bytes {
        Some((buffer_offset + chunk.len()) as u64)
    } else {
        None
    };

    Ok(tool_ok(json!({
        "output_ref": output_ref,
        "stream_output_ref": format!("session:{session_id}:{stream}"),
        "stream": stream,
        "offset": buffer_offset,
        "requested_offset": requested_offset,
        "limit": limit,
        "content": String::from_utf8_lossy(chunk),
        "next_offset": next_offset,
        "total_retained_bytes": data.len(),
        "total_stream_bytes": total_stream_bytes,
        "truncated": next_offset.is_some(),
        "warnings": if ref_stream == "full" {
            vec!["legacy full output_ref defaults to stdout; use output_refs for stable stream paging"]
        } else {
            Vec::<&str>::new()
        }
    })))
}

pub fn write_stdin(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    let session_id = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkspaceError::invalid_argument("session_id is required"))?;
    let session = ctx.sessions.get(session_id)?;
    ensure_context_match(ctx, &session)?;
    let chars = args.get("chars").and_then(Value::as_str).unwrap_or("");
    let max_output_bytes = args
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(65_536) as usize;

    let running = tauri::async_runtime::block_on(session.is_running());
    if !running {
        if !chars.is_empty() {
            return Err(WorkspaceError::Tool {
                code: "SESSION_CLOSED",
                message: "Session is closed; stdin write blocked.".into(),
                category: "runtime",
                retryable: false,
            });
        }
        return Ok(tool_ok(session.snapshot(max_output_bytes)));
    }

    if !chars.is_empty() {
        let mut stdin_guard = tauri::async_runtime::block_on(session.stdin.lock());
        let stdin = stdin_guard.as_mut().ok_or_else(|| WorkspaceError::Tool {
            code: "SESSION_CLOSED",
            message: "Session stdin is closed.".into(),
            category: "runtime",
            retryable: false,
        })?;
        use tokio::io::AsyncWriteExt;
        tauri::async_runtime::block_on(async {
            stdin
                .write_all(chars.as_bytes())
                .await
                .map_err(|_| WorkspaceError::Tool {
                    code: "SESSION_CLOSED",
                    message: "Session stdin is closed.".into(),
                    category: "runtime",
                    retryable: false,
                })
        })?;
        let _ = tauri::async_runtime::block_on(stdin.flush());
    }

    let yield_ms = args
        .get("yield_time_ms")
        .and_then(Value::as_u64)
        .unwrap_or(1000)
        .min(30_000);
    std::thread::sleep(std::time::Duration::from_millis(yield_ms));
    tauri::async_runtime::block_on(session.refresh_status());
    Ok(tool_ok(session.snapshot(max_output_bytes)))
}

pub fn kill_session(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    let session_id = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkspaceError::invalid_argument("session_id is required"))?;
    let session = ctx.sessions.get(session_id)?;
    ensure_context_match(ctx, &session)?;
    let max_output_bytes = args
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(65_536) as usize;
    let wait_ms = args
        .get("wait_ms")
        .and_then(Value::as_u64)
        .unwrap_or(5000)
        .min(30_000);
    let signal = args.get("signal").and_then(Value::as_str).unwrap_or("TERM");

    let running = tauri::async_runtime::block_on(session.is_running());
    let mut killed = false;
    let mut status = "exited";
    let mut evicted = true;

    if running {
        session.mark_termination_reason("killed");
        tauri::async_runtime::block_on(async {
            let (pid, has_job) = {
                let child = session.child.lock().await;
                (child.id(), {
                    #[cfg(windows)]
                    {
                        child.has_job()
                    }
                    #[cfg(not(windows))]
                    {
                        false
                    }
                })
            };
            if has_job {
                let mut child = session.child.lock().await;
                let _ = child.start_kill();
            } else if let Some(pid) = pid {
                send_session_signal(pid, signal);
            } else {
                let mut child = session.child.lock().await;
                let _ = child.start_kill();
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(wait_ms), async {
                let mut child = session.child.lock().await;
                let _ = child.wait().await;
            })
            .await;
        });
        tauri::async_runtime::block_on(session.refresh_status());
        if tauri::async_runtime::block_on(session.is_running()) {
            status = "terminating";
            evicted = false;
        } else {
            killed = true;
            status = "killed";
        }
    }

    let mut payload = session.snapshot(max_output_bytes);
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("killed".into(), json!(killed));
        obj.insert("status".into(), json!(status));
        obj.insert("evicted".into(), json!(evicted));
        if status == "terminating" {
            obj.insert(
                "warnings".into(),
                json!(["Process did not exit after kill; session retained for retry"]),
            );
        }
    }

    if evicted {
        ctx.sessions.remove(session_id);
    }

    Ok(tool_ok(payload))
}

fn ensure_context_match(ctx: &ToolContext, session: &ExecSession) -> Result<(), WorkspaceError> {
    let fingerprint = ctx.execution_fingerprint();
    if session.belongs_to_context(&fingerprint) {
        return Ok(());
    }
    Err(WorkspaceError::ToolDetails {
        code: "WORKSPACE_CONTEXT_MISMATCH",
        message: "The command session belongs to a different execution context.".into(),
        category: "workspace_context",
        retryable: true,
        details: json!({
            "session_fingerprint": "redacted",
            "current_fingerprint": fingerprint,
        }),
    })
}

fn reap_session_in_background(session: Arc<ExecSession>) {
    tauri::async_runtime::spawn(async move {
        let status = {
            let mut child = session.child.lock().await;
            child.wait().await.ok()
        };
        if let Some(status) = status {
            session.record_exit_status(status);
        }
        session.wait_for_readers().await;
    });
}

#[cfg(unix)]
fn send_session_signal(pid: u32, signal: &str) {
    let sig = match signal {
        "KILL" => libc::SIGKILL,
        "INT" => libc::SIGINT,
        _ => libc::SIGTERM,
    };
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

#[cfg(windows)]
fn send_session_signal(pid: u32, _signal: &str) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    fn short_lived_child() -> Child {
        #[cfg(windows)]
        {
            Command::new("cmd.exe")
                .args(["/d", "/c", "exit", "0"])
                .spawn()
                .expect("spawn test child")
        }
        #[cfg(not(windows))]
        {
            Command::new("sh")
                .args(["-c", "exit 0"])
                .spawn()
                .expect("spawn test child")
        }
    }

    #[test]
    fn shutdown_all_removes_terminated_sessions() {
        let store = SessionStore::new();
        let session = store.insert(ExecSession::new_with_mode_and_fingerprint(
            short_lived_child(),
            false,
            "context-a".into(),
        ));

        let report = tauri::async_runtime::block_on(store.shutdown_all("server_restart"))
            .expect("shutdown report");
        assert_eq!(report.requested, 1);
        assert_eq!(report.terminated, 1);
        assert_eq!(report.timed_out, 0);
        assert_eq!(report.remaining(), 0);
        assert!(store.get(&session.session_id).is_err());
    }

    #[test]
    fn shutdown_context_does_not_touch_other_contexts() {
        let store = SessionStore::new();
        let first = store.insert(ExecSession::new_with_mode_and_fingerprint(
            short_lived_child(),
            false,
            "context-a".into(),
        ));
        let second = store.insert(ExecSession::new_with_mode_and_fingerprint(
            short_lived_child(),
            false,
            "context-b".into(),
        ));

        let report = tauri::async_runtime::block_on(
            store.shutdown_context("context-a", "workspace_context_changed"),
        )
        .expect("shutdown report");
        assert_eq!(report.requested, 1);
        assert_eq!(report.terminated, 1);
        assert!(store.get(&first.session_id).is_err());
        assert!(store.get(&second.session_id).is_ok());

        let _ = tauri::async_runtime::block_on(store.shutdown_all("server_restart"));
    }
}
