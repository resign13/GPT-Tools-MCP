use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde_json::{json, Value};
use tokio::process::Command;

use std::sync::Arc;

use crate::tools::context::ToolContext;
use crate::tools::authorized_invocation::{AuthorizedInvocation, InvocationReservation};
use crate::tools::session::{ExecSession, SessionStore};
use crate::tools::workspace::{tool_ok, WorkspaceError};

struct LaunchPlan {
    program: String,
    args: Vec<String>,
}

pub fn exec_command(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    let invocation = AuthorizedInvocation::new(ctx, "exec_command", args);
    exec_command_authorized(ctx, args, &invocation)
}

pub(crate) fn exec_command_authorized(
    ctx: &ToolContext,
    args: &Value,
    invocation: &AuthorizedInvocation,
) -> Result<Value, WorkspaceError> {
    ctx.validate_execution_context()?;
    let cmd = args
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkspaceError::invalid_argument("cmd is required"))?;
    let workdir_raw = args
        .get("workdir")
        .or_else(|| args.get("cwd"))
        .and_then(Value::as_str)
        .unwrap_or(".");
    let workdir = ctx.resolve_command_cwd(workdir_raw)?;
    if !workdir.path.is_dir() {
        return Err(WorkspaceError::not_a_directory(
            "workdir is not a directory",
        ));
    }
    let filesystem_scope = args
        .get("filesystem_scope")
        .and_then(Value::as_str)
        .unwrap_or(if ctx.host_access() { "host" } else { "workspace" })
        .to_string();
    validate_child_process_scope(ctx, args)?;
    let native = if !ctx.host_access() || matches!(cmd.trim(), "pwd" | "ls") {
        run_native_diagnostic(ctx, cmd, &workdir.path)?
    } else { None };
    if let Some(result) = native {
        let mut result = result;
        if let Some(object) = result.as_object_mut() {
            object.insert(
                "filesystem_scope".into(),
                Value::String(filesystem_scope.clone()),
            );
            object.insert("sandbox_enforced".into(), Value::Bool(false));
            object.insert(
                "execution_boundary".into(),
                Value::String("native_builtin".into()),
            );
            object.insert("isolation_backend".into(), json!("native_builtin"));
            object.insert("sandbox_bypass".into(), json!(ctx.host_access()));
            object.insert("fallback_allowed".into(), json!(false));
            object.insert("child_process".into(), Value::Bool(false));
            object.insert("transport_ok".into(), Value::Bool(true));
            object.insert("command_ok".into(), Value::Bool(true));
        }
        return Ok(tool_ok(result));
    }
    // A Git workspace must never fall back to direct child-process execution.
    // The platform manager will allow this only after an enforced sandbox exists.
    ctx.require_exec_sandbox()?;
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(30_000);
    let max_output = args
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(65_536) as usize;
    let yield_ms = args
        .get("yield_time_ms")
        .and_then(Value::as_u64)
        .unwrap_or(1000)
        .min(30_000);
    let tty = args.get("tty").and_then(Value::as_bool).unwrap_or(false);
    let stdin_text = args.get("stdin").and_then(Value::as_str).unwrap_or("");
    let environment = parse_user_environment_for_context(args, ctx.host_access())?;

    // Reserve the private handoff only after argument/path/backend preflight.
    // A validation failure therefore never consumes a launch authorization.
    ctx.validate_execution_context()?;
    let reservation = invocation.reserve(ctx, "exec_command", args)?;

    let result = tauri::async_runtime::block_on(async {
        run_command(
            ctx,
            cmd,
            &workdir.path,
            Duration::from_millis(timeout_ms),
            Duration::from_millis(yield_ms),
            max_output,
            tty,
            stdin_text,
            &environment,
            Some(reservation),
        )
        .await
    });

    match result {
        Ok(mut out) => {
            if let Some(object) = out.as_object_mut() {
                let sandbox_enforced = object
                    .get("sandbox_enforced")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                object.insert("filesystem_scope".into(), Value::String(filesystem_scope));
                object.insert("sandbox_enforced".into(), Value::Bool(sandbox_enforced));
                if object.get("execution_boundary").is_none() {
                    object.insert(
                        "execution_boundary".into(),
                        Value::String(
                            if sandbox_enforced {
                                "windows_appcontainer"
                            } else {
                                "policy_only"
                            }
                            .into(),
                        ),
                    );
                }
                object.insert("isolation_backend".into(), json!(ctx.exec_sandbox_status().implementation));
                object.insert("sandbox_bypass".into(), json!(ctx.host_access()));
                object.insert("fallback_allowed".into(), json!(false));
                object.insert("child_process".into(), Value::Bool(true));
            }
            Ok(tool_ok(out))
        }
        Err(error) => match execution_failure_result(&error, cmd, &workdir.path) {
            Some(mut result) => {
                if let Some(object) = result.as_object_mut() {
                    object.insert("filesystem_scope".into(), json!(filesystem_scope));
                    object.insert("isolation_backend".into(), json!(ctx.exec_sandbox_status().implementation));
                    object.insert("sandbox_bypass".into(), json!(ctx.host_access()));
                    object.insert("fallback_allowed".into(), json!(false));
                    if ctx.host_access() {
                        object.insert("execution_boundary".into(), json!("host"));
                        object.insert("sandbox_enforced".into(), json!(false));
                    }
                }
                Ok(tool_ok(result))
            },
            None => Err(error),
        },
    }
}

fn validate_child_process_scope(ctx: &ToolContext, args: &Value) -> Result<(), WorkspaceError> {
    let scope = args
        .get("filesystem_scope")
        .and_then(Value::as_str)
        .unwrap_or("workspace");
    if ctx.host_access() {
        return if args.get("filesystem_scope").is_none() || scope == "host" { Ok(()) } else {
            Err(WorkspaceError::Tool { code: "ISOLATION_CAPABILITY_UNSATISFIED", message: "Host execution does not enforce workspace isolation.".into(), category: "permission", retryable: false })
        };
    }
    match scope {
        "workspace" => Ok(()),
        "host" => Err(WorkspaceError::ToolDetails {
            code: "EXTERNAL_EXECUTION_NOT_ALLOWED",
            message: "exec_command 只允许在 Workspace 内执行，Workspace 外执行已禁用。".into(),
            category: "permission",
            retryable: false,
            details: json!({
                "stage": "policy",
                "filesystem_scope": "host",
                "sandbox_enforced": false,
                "recoverable": false,
                "suggestion": "将 filesystem_scope 设置为 workspace，并在当前 Workspace 内执行"
            }),
        }),
        _ => Err(WorkspaceError::invalid_argument(
            "filesystem_scope must be workspace",
        )),
    }
}

fn run_native_diagnostic(
    ctx: &ToolContext,
    cmd: &str,
    cwd: &Path,
) -> Result<Option<Value>, WorkspaceError> {
    if crate::tools::policy::command_has_shell_syntax(cmd) {
        return Ok(None);
    }
    let parts = shell_words::split(cmd)
        .map_err(|_| WorkspaceError::invalid_argument("Invalid command syntax"))?;
    if parts.is_empty() {
        return Ok(None);
    }

    let command = parts[0].to_ascii_lowercase();
    let stdout = match command.as_str() {
        "pwd" if parts.len() == 1 => Some(format!("{}\n", cwd.display())),
        "ls" | "dir" => Some(list_directory(ctx, cwd, &parts[1..])?),
        "which" if parts.len() == 2 => {
            let path = which::which(&parts[1]).map_err(|_| WorkspaceError::Tool {
                code: "COMMAND_NOT_FOUND",
                message: format!("Program not found on PATH: {}", parts[1]),
                category: "runtime",
                retryable: false,
            })?;
            Some(format!("{}\n", path.display()))
        }
        "echo" => Some(format!("{}\n", parts[1..].join(" "))),
        _ => None,
    };

    Ok(stdout.map(|stdout| {
        json!({
            "command": cmd,
            "resolved_cwd": cwd.display().to_string(),
            "status": "exited",
            "termination_reason": "exited",
            "recoverable": false,
            "suggestion": "命令已完成",
            "exit_code": 0,
            "stdout": stdout,
            "stderr": "",
            "stdout_truncated": false,
            "stderr_truncated": false,
            "duration_ms": 0,
            "elapsed_ms": 0,
            "execution_mode": "native_builtin",
            "command_runner": "native_builtin",
            "warnings": ["native diagnostic without child process"]
        })
    }))
}

fn list_directory(
    ctx: &ToolContext,
    cwd: &Path,
    args: &[String],
) -> Result<String, WorkspaceError> {
    let target = match args {
        [] => cwd.to_path_buf(),
        [path] => ctx.resolve_command_path(cwd, path)?.path,
        _ => {
            return Err(WorkspaceError::invalid_argument(
                "ls/dir accepts at most one directory path",
            ))
        }
    };
    if !target.is_dir() {
        return Err(WorkspaceError::not_a_directory(
            "ls/dir target is not a directory",
        ));
    }

    let mut entries = std::fs::read_dir(target)
        .map_err(|error| WorkspaceError::ToolDetails {
            code: "DIRECTORY_READ_FAILED",
            message: format!("Failed to read directory: {error}"),
            category: "runtime",
            retryable: true,
            details: json!({
                "stage": "native_builtin",
                "reason": "directory_read_failed",
                "retryable": true
            }),
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    entries.sort_unstable();
    Ok(if entries.is_empty() {
        String::new()
    } else {
        format!("{}\n", entries.join("\n"))
    })
}

async fn spawn_session(
    ctx: &ToolContext,
    program: &str,
    args: &[String],
    cwd: &Path,
    interactive: bool,
    user_env: &[(String, String)],
) -> Result<std::sync::Arc<ExecSession>, WorkspaceError> {
    #[cfg(windows)]
    if ctx.host_access() {
        let (program, args) = sandbox_invocation(program, args);
        let policy = ctx.sandbox_policy(cwd.to_path_buf(), Vec::new());
        let child = crate::security::exec_sandbox::spawn_host(&policy, &program, &args, user_env)
            .map_err(|error| WorkspaceError::Tool { code: "HOST_PROCESS_START_FAILED", message: error.to_string(), category: "runtime", retryable: true })?;
        let session = ExecSession::new_with_host_child(child, interactive, ctx.execution_fingerprint());
        session.track_host_completion(ctx);
        return Ok(ctx.sessions.insert(session));
    }
    #[cfg(windows)]
    if ctx.requires_strict_exec_isolation() {
        let git_program = is_git_program(program);
        let (mut sandbox_program, sandbox_args) = if git_program {
            // A Git `cmd\git.exe` shim starts another process and loads the
            // MinGW runtime through the host PATH.  Keep the sandbox launch
            // single-process; `stage_git_runtime` supplies the private binary.
            (program.to_string(), args.to_vec())
        } else {
            sandbox_invocation(program, args)
        };
        let mut readonly_roots = if git_program {
            Vec::new()
        } else {
            toolchain_roots(&sandbox_program)
        };
        if !git_program && sandbox_program != program {
            readonly_roots.extend(toolchain_roots(program));
        }
        let git_identity = git_program.then(|| ctx.git_identity()).flatten();
        if let Some(identity) = git_identity.as_ref() {
            // A linked worktree keeps its Git metadata in the repository's
            // common `.git` directory, which is outside the execution root.
            // Grant only those metadata roots read/execute access; the
            // worktree itself remains the only writable root.
            readonly_roots.push(identity.git_dir.clone());
            readonly_roots.push(identity.git_common_dir.clone());
            // Git's built-in commands may dispatch a helper from the
            // installation's libexec tree.  Keep that tree read-only; the
            // executable and its loader DLLs are staged into the sandbox.
            readonly_roots.extend(git_runtime_roots(program));
        }
        readonly_roots.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
        readonly_roots.dedup();
        let mut policy = ctx.sandbox_policy(cwd.to_path_buf(), readonly_roots);
        if git_program {
            if git_identity.is_none() {
                return Err(WorkspaceError::ToolDetails {
                    code: "WORKSPACE_CONTEXT_MISMATCH",
                    message: "Git command has no validated worktree identity.".into(),
                    category: "security",
                    retryable: false,
                    details: json!({"stage": "exec_preflight", "reason": "git_identity_missing"}),
                });
            }
            sandbox_program = stage_git_runtime(program, &policy.temp_root)?;
            // Git for Windows probes the process cwd before honoring GIT_DIR.
            // A user-profile temp path can require traverse permissions on
            // several protected ancestors inside an AppContainer.  Start in
            // the system temp directory (which is already AppContainer-readable)
            // and use the explicit Git variables below to pin repository
            // operations to the validated execution worktree.
            let system_temp = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .map(|root| root.join("Temp"))
                .filter(|path| path.is_dir());
            policy.startup_directory = system_temp.unwrap_or_else(|| policy.temp_root.clone());
        }
        let temp = policy.temp_root.to_string_lossy().into_owned();
        let git_config = format!("{temp}\\gitconfig");
        let mut env = vec![
            ("TMP".into(), temp.clone()),
            ("TEMP".into(), temp),
            // Keep user-home based tool configuration inside the explicitly
            // granted sandbox temp directory. In particular, Git otherwise
            // probes an inaccessible `/dev/null` global config path from its
            // MSYS runtime when running as an AppContainer.
            (
                "HOME".into(),
                policy.temp_root.to_string_lossy().into_owned(),
            ),
            (
                "USERPROFILE".into(),
                policy.temp_root.to_string_lossy().into_owned(),
            ),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_SYSTEM".into(), git_config.clone()),
            ("GIT_CONFIG_GLOBAL".into(), git_config),
            ("PYTHONUTF8".into(), "1".into()),
            ("PYTHONIOENCODING".into(), "utf-8".into()),
            ("PYTHONLEGACYWINDOWSSTDIO".into(), "0".into()),
        ];
        env.extend(user_env.iter().cloned());
        if git_program {
            let temp_root = policy.temp_root.clone();
            env.extend([
                (
                    "GIT_DIR".into(),
                    platform_command_path(&ctx.execution_root().join(".git"))
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    "GIT_WORK_TREE".into(),
                    platform_command_path(&ctx.execution_root())
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    "GIT_CEILING_DIRECTORIES".into(),
                    platform_command_path(&temp_root)
                        .to_string_lossy()
                        .into_owned(),
                ),
            ]);
            let install_root = git_install_root(program).ok_or_else(|| {
                git_runtime_staging_error(
                    "Git installation root could not be resolved.",
                    json!({"stage": "exec_preflight", "reason": "git_install_root_missing"}),
                )
            })?;
            let source_bin = install_root.join("mingw64").join("bin");
            let source_core = install_root
                .join("mingw64")
                .join("libexec")
                .join("git-core");
            let source_usr_bin = install_root.join("usr").join("bin");
            let staged_bin = Path::new(&sandbox_program)
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let mut path_parts = vec![
                staged_bin.to_string_lossy().into_owned(),
                source_core.to_string_lossy().into_owned(),
                source_bin.to_string_lossy().into_owned(),
                source_usr_bin.to_string_lossy().into_owned(),
            ];
            if let Some(system_root) = std::env::var_os("SystemRoot") {
                path_parts.push(
                    PathBuf::from(system_root)
                        .join("System32")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            env.push((
                "GIT_EXEC_PATH".into(),
                source_core.to_string_lossy().into_owned(),
            ));
            let templates = install_root
                .join("mingw64")
                .join("share")
                .join("git-core")
                .join("templates");
            if templates.is_dir() {
                env.push((
                    "GIT_TEMPLATE_DIR".into(),
                    templates.to_string_lossy().into_owned(),
                ));
            }
            env.push(("PATH".into(), path_parts.join(";")));
        }
        let child = ctx.spawn_sandbox(&policy, &sandbox_program, &sandbox_args, &env)?;
        let session = ctx.sessions.insert(ExecSession::new_with_sandbox_child(
            child,
            interactive,
            ctx.execution_fingerprint(),
        ));
        return Ok(session);
    }

    let mut command = command_for_program(program, args);
    command
        .current_dir(platform_command_path(cwd))
        .envs(user_env.iter().map(|(key, value)| (key, value)))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONLEGACYWINDOWSSTDIO", "0");

    let child = command.spawn().map_err(|e| WorkspaceError::ToolDetails {
        code: "COMMAND_SPAWN_FAILED",
        message: format!("Failed to start command: {e}"),
        category: "runtime",
        retryable: true,
        details: json!({
            "termination_reason": "spawn_failed",
            "recoverable": true,
            "suggestion": "检查命令路径、权限和运行时环境后重试"
        }),
    })?;

    Ok(ctx
        .sessions
        .insert(ExecSession::new_with_mode_and_fingerprint(
            child,
            interactive,
            ctx.execution_fingerprint(),
        )))
}

#[cfg(windows)]
fn is_git_program(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("git"))
}

#[cfg(windows)]
fn sandbox_invocation(program: &str, args: &[String]) -> (String, Vec<String>) {
    if is_git_program(program) {
        return (program.to_string(), args.to_vec());
    }
    let extension = Path::new(program)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("bat") | Some("cmd") => {
            let shell = which::which("cmd.exe")
                .unwrap_or_else(|_| Path::new("C:\\Windows\\System32\\cmd.exe").to_path_buf());
            (
                shell.to_string_lossy().into_owned(),
                vec![
                    "/d".into(),
                    "/s".into(),
                    "/c".into(),
                    windows_batch_command_line(program, args),
                ],
            )
        }
        Some("ps1") => {
            let shell = which::which("pwsh")
                .or_else(|_| which::which("powershell"))
                .unwrap_or_else(|_| {
                    Path::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
                        .to_path_buf()
                });
            let mut invocation = vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                windows_command_path(program),
            ];
            invocation.extend(args.iter().cloned());
            (shell.to_string_lossy().into_owned(), invocation)
        }
        _ => (program.to_string(), args.to_vec()),
    }
}

#[cfg(windows)]
fn toolchain_roots(program: &str) -> Vec<std::path::PathBuf> {
    let path = Path::new(program);
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let mut roots = vec![parent.to_path_buf()];
    if let Some(grandparent) = parent.parent() {
        if grandparent != Path::new("\\") && grandparent.components().count() > 1 {
            roots.push(grandparent.to_path_buf());
        }
    }
    roots
}

#[cfg(windows)]
fn git_runtime_roots(program: &str) -> Vec<std::path::PathBuf> {
    let Some(install_root) = git_install_root(program) else {
        return Vec::new();
    };
    [
        install_root.join("mingw64").join("bin"),
        install_root.join("usr").join("bin"),
        install_root.join("mingw64").join("libexec"),
        install_root
            .join("mingw64")
            .join("libexec")
            .join("git-core"),
        install_root.join("libexec").join("git-core"),
    ]
    .into_iter()
    .filter(|root| root.is_dir())
    .collect()
}

#[cfg(windows)]
fn git_install_root(program: impl AsRef<Path>) -> Option<std::path::PathBuf> {
    let path = program.as_ref();
    let parent = path.parent()?;
    if path_component_is(parent, "cmd") {
        return parent.parent().map(Path::to_path_buf);
    }
    if path_component_is(parent, "bin")
        && parent
            .parent()
            .is_some_and(|value| path_component_is(value, "mingw64"))
    {
        return parent
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
    }
    None
}

#[cfg(windows)]
fn path_component_is(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

#[cfg(windows)]
fn git_runtime_staging_error(message: impl Into<String>, details: Value) -> WorkspaceError {
    WorkspaceError::ToolDetails {
        code: "GIT_RUNTIME_STAGING_FAILED",
        message: message.into(),
        category: "security",
        retryable: true,
        details,
    }
}

#[cfg(windows)]
fn stage_git_runtime(program: &str, temp_root: &Path) -> Result<String, WorkspaceError> {
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    static STAGE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = STAGE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            git_runtime_staging_error(
                "Git runtime staging lock was poisoned.",
                json!({"stage": "lock", "reason": "lock_poisoned"}),
            )
        })?;

    let program_path = fs::canonicalize(program).map_err(|error| {
        git_runtime_staging_error(
            format!("Git executable is unavailable: {error}"),
            json!({"stage": "source", "program": program}),
        )
    })?;
    let install_root = git_install_root(&program_path).ok_or_else(|| {
        git_runtime_staging_error(
            "Git installation root could not be inferred from the executable path.",
            json!({"stage": "source", "program": program_path}),
        )
    })?;
    let source_bin = install_root.join("mingw64").join("bin");
    let source_git = source_bin.join("git.exe");
    if !source_git.is_file() {
        return Err(git_runtime_staging_error(
            "Git for Windows MinGW executable was not found.",
            json!({"stage": "source", "git": source_git}),
        ));
    }
    let source_git = fs::canonicalize(&source_git).map_err(|error| {
        git_runtime_staging_error(
            format!("Git runtime path could not be resolved: {error}"),
            json!({"stage": "source", "git": source_git}),
        )
    })?;

    fs::create_dir_all(temp_root).map_err(|error| {
        git_runtime_staging_error(
            format!("Sandbox temporary directory could not be created: {error}"),
            json!({"stage": "destination", "temp_root": temp_root}),
        )
    })?;
    let runtime_root = temp_root.join("git-runtime");
    let staged_bin = runtime_root.join("mingw64").join("bin");
    let staged_git = staged_bin.join("git.exe");
    let marker = runtime_root.join(".source");
    let source_key = source_git.to_string_lossy().into_owned();

    if staged_git.is_file()
        && fs::read_to_string(&marker)
            .map(|value| value.trim() == source_key)
            .unwrap_or(false)
    {
        return Ok(staged_git.to_string_lossy().into_owned());
    }
    if runtime_root.exists() {
        fs::remove_dir_all(&runtime_root).map_err(|error| {
            git_runtime_staging_error(
                format!("Incomplete staged Git runtime could not be replaced: {error}"),
                json!({"stage": "destination", "runtime_root": runtime_root}),
            )
        })?;
    }
    fs::create_dir_all(&staged_bin).map_err(|error| {
        git_runtime_staging_error(
            format!("Staged Git runtime directory could not be created: {error}"),
            json!({"stage": "destination", "runtime_root": runtime_root}),
        )
    })?;
    fs::copy(&source_git, &staged_git).map_err(|error| {
        git_runtime_staging_error(
            format!("Git executable could not be staged: {error}"),
            json!({"stage": "copy", "source": source_git, "destination": staged_git}),
        )
    })?;

    // `git.exe` has a small, stable MinGW loader dependency set.  Keep the
    // staged runtime private instead of granting the AppContainer access to
    // the complete Git installation tree.
    for dependency in [
        "libiconv-2.dll",
        "libintl-8.dll",
        "libpcre2-8-0.dll",
        "libwinpthread-1.dll",
        "zlib1.dll",
    ] {
        let source = source_bin.join(dependency);
        let destination = staged_bin.join(dependency);
        if !source.is_file() {
            return Err(git_runtime_staging_error(
                format!("Git runtime dependency is missing: {dependency}"),
                json!({"stage": "dependency", "source": source}),
            ));
        }
        fs::copy(&source, &destination).map_err(|error| {
            git_runtime_staging_error(
                format!("Git runtime dependency could not be staged: {error}"),
                json!({"stage": "dependency", "source": source, "destination": destination}),
            )
        })?;
    }

    fs::write(&marker, source_key).map_err(|error| {
        git_runtime_staging_error(
            format!("Staged Git runtime marker could not be written: {error}"),
            json!({"stage": "destination", "marker": marker}),
        )
    })?;
    Ok(staged_git.to_string_lossy().into_owned())
}

#[allow(clippy::too_many_arguments)]
async fn run_command(
    ctx: &ToolContext,
    cmd: &str,
    cwd: &Path,
    limit: Duration,
    yield_time: Duration,
    max_output: usize,
    tty: bool,
    stdin_text: &str,
    user_env: &[(String, String)],
    reservation: Option<InvocationReservation<'_>>,
) -> Result<Value, WorkspaceError> {
    let launch = if ctx.host_access() { let (program, args) = shell_launch(cmd); LaunchPlan { program, args } } else { parse_and_resolve(cmd, cwd, ctx, &ctx.policy)? };
    let start = Instant::now();
    let session = spawn_session(ctx, &launch.program, &launch.args, cwd, tty, user_env).await?;
    if let Some(reservation) = reservation {
        reservation.commit();
    }
    session.spawn_readers().await;
    let deadline = start + limit;

    if yield_time.is_zero() {
        let snapshot = session.snapshot(max_output);
        spawn_timeout_monitor(ctx.sessions.clone(), session.clone(), deadline);
        return Ok(merge_exec_result(snapshot, start, cmd, cwd, true));
    }

    if !tty && !stdin_text.is_empty() {
        let mut stdin_guard = session.stdin.lock().await;
        if let Some(stdin) = stdin_guard.as_mut() {
            use tokio::io::AsyncWriteExt;
            if !stdin_text.is_empty() {
                stdin
                    .write_all(stdin_text.as_bytes())
                    .await
                    .map_err(|_| WorkspaceError::Tool {
                        code: "SESSION_CLOSED",
                        message: "Failed to write stdin.".into(),
                        category: "runtime",
                        retryable: false,
                    })?;
            }
            let _ = stdin.shutdown().await;
        }
        *stdin_guard = None;
        session.mark_stdin_closed();
    }

    loop {
        session.refresh_status().await;
        if session.has_exited() {
            session.wait_for_readers().await;
            let snapshot = session.snapshot(max_output);
            ctx.sessions.remove(&session.session_id);
            return Ok(merge_exec_result(snapshot, start, cmd, cwd, false));
        }
        if !tty && Instant::now() >= deadline {
            session.mark_termination_reason("timeout");
            session.kill_and_wait().await;
            session.refresh_status().await;
            session.wait_for_readers().await;
            let snapshot = session.snapshot(max_output);
            // Snapshot is embedded; schedule eviction so abandoned timeouts do not linger.
            schedule_session_eviction(ctx.sessions.clone(), session.session_id.clone());
            return Err(WorkspaceError::ToolDetails {
                code: "TIMEOUT",
                message: "Command timed out.".into(),
                category: "runtime",
                retryable: true,
                details: json!({
                    "termination_reason": "timeout",
                    "recoverable": true,
                    "suggestion": "读取 output_refs，调整 timeout_ms 后重试",
                    "session": snapshot
                }),
            });
        }
        if Instant::now() - start >= yield_time || tty {
            let snapshot = session.snapshot(max_output);
            spawn_timeout_monitor(ctx.sessions.clone(), session.clone(), deadline);
            return Ok(merge_exec_result(snapshot, start, cmd, cwd, true));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// How long a timed-out / background session stays readable before map eviction.
const SESSION_EVICT_AFTER_TIMEOUT: Duration = Duration::from_secs(30);

fn spawn_timeout_monitor(
    sessions: Arc<SessionStore>,
    session: Arc<ExecSession>,
    deadline: Instant,
) {
    tauri::async_runtime::spawn(async move {
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(remaining).await;
        session.refresh_status().await;
        if !session.has_exited() {
            session.mark_termination_reason("timeout");
            session.kill_and_wait().await;
            session.refresh_status().await;
            session.wait_for_readers().await;
        }
        // Keep the session briefly so clients can still read_output / probe status.
        schedule_session_eviction(sessions, session.session_id.clone());
    });
}

fn schedule_session_eviction(sessions: Arc<SessionStore>, session_id: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SESSION_EVICT_AFTER_TIMEOUT).await;
        sessions.remove(&session_id);
    });
}

pub fn exec_health_check(ctx: &ToolContext) -> Result<Value, WorkspaceError> {
    ctx.validate_execution_context()?;
    let start = Instant::now();
    let cwd = ctx.execution_root();
    #[cfg(windows)]
    let probe = r#"cmd.exe /d /c "echo exec-health && echo exec-health-stderr 1>&2""#;
    #[cfg(not(windows))]
    let probe = r#"sh -c "printf exec-health; printf exec-health-stderr >&2""#;

    let mut response = json!({
        "worker": {"alive": true},
        "session_create": false,
        "command_run": false,
        "stdout_capture": false,
        "stderr_capture": false,
        "duration_ms": start.elapsed().as_millis(),
        "next_actions": []
    });

    if let Err(error) = ctx.require_exec_sandbox() {
        response["status"] = Value::String("error".into());
        response["summary"] = Value::String(
            "exec health check 未执行：strict workspace isolation 尚未具备 OS sandbox".into(),
        );
        response["error"] = error.to_error_value();
        response["next_actions"] = json!(["启用 Windows AppContainer sandbox 后重试"]);
        response["duration_ms"] = json!(start.elapsed().as_millis());
        return Ok(tool_ok(response));
    }

    let result = tauri::async_runtime::block_on(run_command(
        ctx,
        probe,
        &cwd,
        Duration::from_secs(5),
        Duration::from_secs(5),
        16_384,
        false,
        "",
        &[],
        None,
    ));

    match result {
        Ok(snapshot) => {
            let session_created = snapshot.get("session_id").is_some();
            let command_run = snapshot.get("exit_code").and_then(Value::as_i64) == Some(0);
            let stdout_capture = snapshot
                .get("stdout")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("exec-health"));
            let stderr_capture = snapshot
                .get("stderr")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("exec-health-stderr"));
            let healthy = session_created && command_run && stdout_capture && stderr_capture;
            response["session_create"] = Value::Bool(session_created);
            response["command_run"] = Value::Bool(command_run);
            response["stdout_capture"] = Value::Bool(stdout_capture);
            response["stderr_capture"] = Value::Bool(stderr_capture);
            response["status"] = Value::String(if healthy { "success" } else { "error" }.into());
            response["summary"] = Value::String(if healthy {
                "exec worker、session、命令执行和 stdout/stderr 捕获均正常".into()
            } else {
                "exec health check 未通过，请查看 probe 结果".into()
            });
            response["probe"] = snapshot;
            if !healthy {
                response["next_actions"] = json!(["检查 exec worker 日志", "重启运行时"]);
            }
        }
        Err(error) => {
            response["status"] = Value::String("error".into());
            response["summary"] = Value::String("exec session 创建或探针执行失败".into());
            response["error"] = error.to_error_value();
            response["next_actions"] = json!(["检查 exec worker 日志", "重启运行时"]);
        }
    }
    response["duration_ms"] = json!(start.elapsed().as_millis());
    Ok(tool_ok(response))
}

fn execution_failure_result(error: &WorkspaceError, command: &str, cwd: &Path) -> Option<Value> {
    let code = match &error {
        WorkspaceError::Tool { code, .. } | WorkspaceError::ToolDetails { code, .. } => *code,
    };
    if !matches!(
        code,
        "COMMAND_REJECTED" | "COMMAND_SPAWN_FAILED" | "HOST_PROCESS_START_FAILED" | "TIMEOUT"
    ) {
        return None;
    }

    let error_value = error.to_error_value();
    let details = error_value
        .get("details")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut result = details.get("session").cloned().unwrap_or_else(|| {
        json!({
            "status": "spawn_failed",
            "termination_reason": "spawn_failed",
            "recoverable": error_value["retryable"].as_bool().unwrap_or(false),
            "exit_code": Value::Null,
            "stdout": "",
            "stderr": "",
            "stdout_truncated": false,
            "stderr_truncated": false
        })
    });
    if let Some(object) = result.as_object_mut() {
        let sandbox_enforced = object
            .get("sandbox_enforced")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let execution_boundary = object
            .get("execution_boundary")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if sandbox_enforced {
                    "windows_appcontainer".into()
                } else {
                    "policy_only".into()
                }
            });
        object.insert("command".into(), json!(command));
        object.insert("resolved_cwd".into(), json!(cwd.display().to_string()));
        object.insert(
            "execution_mode".into(),
            json!(if sandbox_enforced {
                "sandbox"
            } else {
                "direct"
            }),
        );
        object.insert("filesystem_scope".into(), json!("workspace"));
        object.insert("sandbox_enforced".into(), Value::Bool(sandbox_enforced));
        object.insert("execution_boundary".into(), json!(execution_boundary));
        object.insert("child_process".into(), Value::Bool(true));
        object.insert("transport_ok".into(), Value::Bool(true));
        object.insert("command_ok".into(), Value::Bool(false));
        object.insert("error".into(), error_value);
        if code == "TIMEOUT" {
            object.insert("termination_reason".into(), json!("timeout"));
        } else {
            object.insert("status".into(), json!("spawn_failed"));
            object.insert("termination_reason".into(), json!("spawn_failed"));
        }
    }
    Some(result)
}

fn merge_exec_result(
    mut snapshot: Value,
    start: Instant,
    command: &str,
    cwd: &Path,
    keep_session: bool,
) -> Value {
    if let Some(obj) = snapshot.as_object_mut() {
        let duration_ms = start.elapsed().as_millis();
        obj.insert("command".into(), json!(command));
        obj.insert("resolved_cwd".into(), json!(cwd.display().to_string()));
        obj.insert("duration_ms".into(), json!(duration_ms));
        obj.insert("elapsed_ms".into(), json!(duration_ms));
        obj.insert("transport_ok".into(), Value::Bool(true));
        let command_ok = match obj
            .get("termination_reason")
            .and_then(Value::as_str)
            .unwrap_or("running")
        {
            "exited" => obj
                .get("exit_code")
                .and_then(Value::as_i64)
                .map(|exit_code| exit_code == 0)
                .or(Some(false)),
            "running" => None,
            _ => Some(false),
        };
        obj.insert(
            "command_ok".into(),
            command_ok.map(Value::Bool).unwrap_or(Value::Null),
        );
        let sandbox_enforced = obj
            .get("sandbox_enforced")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        obj.insert(
            "execution_mode".into(),
            json!(if sandbox_enforced {
                "sandbox"
            } else {
                "direct"
            }),
        );
        if obj.get("execution_boundary").is_none() {
            obj.insert(
                "execution_boundary".into(),
                json!(if sandbox_enforced {
                    "windows_appcontainer"
                } else {
                    "policy_only"
                }),
            );
        }
        obj.insert(
            "warnings".into(),
            json!(if keep_session {
                vec!["session retained for read_output/write_stdin/kill_session"]
            } else if sandbox_enforced {
                vec!["sandboxed execution without shell"]
            } else {
                vec!["direct execution without shell"]
            }),
        );
    }
    snapshot
}

fn parse_and_resolve(
    cmd: &str,
    cwd: &Path,
    ctx: &ToolContext,
    policy: &crate::tools::policy::PolicySettings,
) -> Result<LaunchPlan, WorkspaceError> {
    if crate::tools::policy::command_has_shell_syntax(cmd) {
        let (program, args) = shell_launch(cmd);
        return Ok(LaunchPlan {
            program,
            args,
        });
    }
    let parts = shell_words::split(cmd)
        .map_err(|_| WorkspaceError::invalid_argument("Invalid command syntax"))?;
    if parts.is_empty() {
        return Err(WorkspaceError::invalid_argument("Empty command"));
    }

    let program = resolve_program(&parts[0], cwd, ctx, policy)?;
    Ok(LaunchPlan {
        program,
        args: parts[1..].to_vec(),
    })
}

fn parse_user_environment_for_context(args: &Value, host: bool) -> Result<Vec<(String, String)>, WorkspaceError> {
    let Some(environment) = args.get("env") else {
        return Ok(Vec::new());
    };
    let object = environment
        .as_object()
        .ok_or_else(|| WorkspaceError::invalid_argument("env must be an object of strings"))?;
    let mut values = Vec::with_capacity(object.len());
    for (key, value) in object {
        let value = value
            .as_str()
            .ok_or_else(|| WorkspaceError::invalid_argument("env values must be strings"))?;
        let upper = key.to_ascii_uppercase();
        if key.is_empty()
            || key.contains(['=', '\0'])
            || value.contains('\0')
            || (!host && matches!(
                upper.as_str(),
                "PATH"
                    | "PATHEXT"
                    | "TMP"
                    | "TEMP"
                    | "HOME"
                    | "USERPROFILE"
                    | "GIT_DIR"
                    | "GIT_WORK_TREE"
                    | "GIT_CONFIG_NOSYSTEM"
                    | "GIT_CONFIG_SYSTEM"
                    | "GIT_CONFIG_GLOBAL"
                    | "GIT_EXEC_PATH"
                    | "GIT_TEMPLATE_DIR"
            ))
        {
            return Err(WorkspaceError::ToolDetails {
                code: "ENVIRONMENT_KEY_PROTECTED",
                message: format!("Environment variable cannot override sandbox key: {key}"),
                category: "security",
                retryable: false,
                details: json!({"key": key}),
            });
        }
        values.push((key.clone(), value.to_string()));
    }
    values.sort_by(|left, right| left.0.to_ascii_uppercase().cmp(&right.0.to_ascii_uppercase()));
    Ok(values)
}

#[cfg(windows)]
fn shell_launch(command: &str) -> (String, Vec<String>) {
    let shell = which::which("cmd.exe")
        .unwrap_or_else(|_| Path::new("C:\\Windows\\System32\\cmd.exe").to_path_buf());
    (
        shell.to_string_lossy().into_owned(),
        vec!["/d".into(), "/s".into(), "/c".into(), command.into()],
    )
}

#[cfg(not(windows))]
fn shell_launch(command: &str) -> (String, Vec<String>) {
    let shell = which::which("sh").unwrap_or_else(|_| Path::new("/bin/sh").to_path_buf());
    (shell.to_string_lossy().into_owned(), vec!["-c".into(), command.into()])
}

fn resolve_program(
    raw: &str,
    cwd: &Path,
    ctx: &ToolContext,
    policy: &crate::tools::policy::PolicySettings,
) -> Result<String, WorkspaceError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WorkspaceError::invalid_argument("Empty program"));
    }

    let explicit_path = trimmed.contains(['/', '\\']);
    let candidate = if Path::new(trimmed).is_absolute() {
        Path::new(trimmed).to_path_buf()
    } else {
        cwd.join(trimmed)
    };
    if candidate.is_file() {
        let resolved = candidate.canonicalize().map_err(|_| WorkspaceError::Tool {
            code: "COMMAND_REJECTED",
            message: format!("Program not found: {trimmed}"),
            category: "runtime",
            retryable: false,
        })?;
        ctx.validate_path(&resolved, crate::tools::PathIntent::CommandCwd)
            .map_err(|_| WorkspaceError::Tool {
                code: "EXECUTABLE_OUTSIDE_WORKSPACE",
                message: format!("Workspace 外可执行文件被拒绝: {trimmed}"),
                category: "security",
                retryable: false,
            })?;
        let extension = resolved
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!(".{}", value.to_ascii_lowercase()))
            .unwrap_or_default();
        if policy.workspace_local_entries
            && (policy.auto_approves_permissions()
                || extension.is_empty()
                || policy.workspace_script_extensions.contains(&extension))
        {
            return Ok(resolved.to_string_lossy().into_owned());
        }
        return Err(WorkspaceError::Tool {
            code: "COMMAND_REJECTED",
            message: format!("Workspace 本地入口未获允许: {trimmed}"),
            category: "policy",
            retryable: false,
        });
    }

    if explicit_path {
        return Err(WorkspaceError::Tool {
            code: "COMMAND_REJECTED",
            message: format!("Program not found: {trimmed}"),
            category: "runtime",
            retryable: false,
        });
    }

    which::which(trimmed)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|_| WorkspaceError::Tool {
            code: "COMMAND_REJECTED",
            message: format!("Program not found on PATH: {trimmed}"),
            category: "runtime",
            retryable: false,
        })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::tools::context::ToolContext;
    use crate::tools::dispatch::call_tool;
    use serde_json::json;
    use tempfile::tempdir;

    fn assert_failure_result(error: WorkspaceError, expected_code: &str) {
        let result = execution_failure_result(&error, "missing-command", Path::new("C:/workspace"))
            .expect("应转换为统一执行结果");
        assert_eq!(result["transport_ok"], true);
        assert_eq!(result["command_ok"], false);
        assert_eq!(result["status"], "spawn_failed");
        assert_eq!(result["error"]["code"], expected_code);
    }

    #[test]
    fn 程序不存在时返回统一执行结果() {
        assert_failure_result(
            WorkspaceError::Tool {
                code: "COMMAND_REJECTED",
                message: "Program not found on PATH: missing-command".into(),
                category: "runtime",
                retryable: false,
            },
            "COMMAND_REJECTED",
        );
    }

    #[test]
    fn 启动失败时返回统一执行结果() {
        assert_failure_result(
            WorkspaceError::ToolDetails {
                code: "COMMAND_SPAWN_FAILED",
                message: "Failed to start command".into(),
                category: "runtime",
                retryable: true,
                details: json!({"recoverable": true}),
            },
            "COMMAND_SPAWN_FAILED",
        );
    }

    #[test]
    fn resolves_an_arbitrarily_named_workspace_local_entry() {
        let workspace = tempdir().expect("workspace");
        let harness = tempdir().expect("harness");
        let entry = workspace.path().join("scripts").join("anything.cmd");
        std::fs::create_dir_all(entry.parent().expect("parent")).expect("scripts");
        std::fs::write(&entry, "echo test").expect("entry");
        let context =
            ToolContext::for_test(workspace.path().to_path_buf(), harness.path().to_path_buf())
                .expect("context");
        let resolved = resolve_program(
            "scripts/anything.cmd",
            workspace.path(),
            &context,
            &crate::tools::policy::PolicySettings::default(),
        )
        .expect("workspace entry resolves");
        assert_eq!(
            std::path::Path::new(&resolved),
            entry.canonicalize().unwrap()
        );
    }

    #[test]
    fn full_access_allows_workspace_script_extensions_without_expanding_scope() {
        let workspace = tempdir().expect("workspace");
        let harness = tempdir().expect("harness");
        let entry = workspace.path().join("scripts").join("run.sh");
        std::fs::create_dir_all(entry.parent().expect("scripts")).expect("scripts");
        std::fs::write(&entry, "echo test").expect("entry");
        let context = ToolContext::for_test(
            workspace.path().to_path_buf(),
            harness.path().to_path_buf(),
        )
        .expect("context");
        let mut policy = crate::tools::policy::PolicySettings::default();
        policy.permissions = crate::tools::policy::ExecutionPolicy::from_config(
            "full_access",
            1,
            crate::tools::policy::IsolationPolicy::Strict,
        )
        .expect("full access policy");

        let resolved = resolve_program("scripts/run.sh", workspace.path(), &context, &policy)
            .expect("full access workspace script");
        assert_eq!(std::path::Path::new(&resolved), entry.canonicalize().unwrap());

        let outside = workspace
            .path()
            .parent()
            .expect("workspace parent")
            .join("outside-script.sh");
        std::fs::write(&outside, "echo outside").expect("outside entry");
        let error = resolve_program(&outside.to_string_lossy(), workspace.path(), &context, &policy)
            .expect_err("full access must retain workspace boundary");
        assert!(error.to_string().contains("Workspace 外可执行文件被拒绝"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_hidden_creation_flags_match_frpc_no_window_pattern() {
        assert_eq!(
            windows_hidden_creation_flags(),
            0x0000_0200 | 0x0800_0000,
            "must keep CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_scripts_use_their_platform_runners() {
        let batch = command_for_program("C:/workspace/run-anything.cmd", &[]);
        assert_eq!(batch.as_std().get_program().to_string_lossy(), "cmd.exe");
        assert!(batch.as_std().get_args().any(|arg| arg == "/c"));
        assert_eq!(
            windows_batch_command_line(
                r"\\?\C:\workspace\Life Brain\run & tooling.cmd",
                &["argument & value".to_string()]
            ),
            r#"call "C:\workspace\Life Brain\run & tooling.cmd" "argument & value""#
        );

        let script = command_for_program("C:/workspace/run-anything.ps1", &[]);
        let runner = script
            .as_std()
            .get_program()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(runner.contains("powershell") || runner.contains("pwsh"));
        assert!(script.as_std().get_args().any(|arg| arg == "-File"));

        // Ensure console-subsystem programs (python.exe) also go through the
        // hidden-window flag path; Command does not expose creation_flags for
        // direct assertion, so this only verifies construction still succeeds.
        let python =
            command_for_program("C:/Python312/python.exe", &["-c".into(), "print(1)".into()]);
        assert_eq!(
            python.as_std().get_program().to_string_lossy(),
            "C:/Python312/python.exe"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_workspace_scripts_and_python_unicode_execute_successfully() {
        let workspace = tempdir().expect("workspace");
        let harness = tempdir().expect("harness");
        std::fs::write(
            workspace.path().join("any-name.cmd"),
            "@echo tooling-cmd-ok\r\n",
        )
        .expect("cmd script");
        std::fs::write(
            workspace.path().join("any-name.ps1"),
            "Write-Output 'tooling-powershell-ok'\r\n",
        )
        .expect("powershell script");
        std::fs::write(
            workspace.path().join("workflow_probe.py"),
            "print('workflow-ok')\n",
        )
        .expect("python module");
        let ctx =
            ToolContext::for_test(workspace.path().to_path_buf(), harness.path().to_path_buf())
                .expect("context");

        for command in [
            "any-name.cmd",
            "any-name.ps1",
            "cmd /c echo tooling-cmd-ok",
            "powershell -NoProfile -Command \"Write-Output tooling-powershell-ok\"",
            "python -c \"print('中文输出正常 ✅')\"",
        ] {
            let output = call_tool(
                &ctx,
                "exec_command",
                &json!({ "cmd": command, "timeout_ms": 10_000, "yield_time_ms": 10_000 }),
            );
            assert_eq!(output["ok"], true, "{command}: {output}");
            assert_eq!(output["command_ok"], true, "{command}: {output}");
        }

        for _ in 0..10 {
            let output = call_tool(
                &ctx,
                "exec_command",
                &json!({ "cmd": "python -m workflow_probe", "timeout_ms": 10_000 }),
            );
            assert_eq!(output["command_ok"], true, "{output}");
            assert!(output["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("workflow-ok"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_batch_scripts_preserve_space_paths_and_arguments() {
        let parent = tempdir().expect("workspace parent");
        let workspace = parent.path().join("Life Brain 中文");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let harness = tempdir().expect("harness");
        let ctx = ToolContext::for_test(workspace.clone(), harness.path().to_path_buf())
            .expect("context");

        for extension in ["cmd", "bat"] {
            let script_name = format!("run & tooling.{extension}");
            std::fs::write(
                workspace.join(&script_name),
                "@echo off\r\nif not \"%~1\"==\"argument & value\" exit /b 7\r\necho tooling-space-path-ok\r\n",
            )
            .expect("batch script");

            let command = format!(r#""{script_name}" "argument & value""#);
            let output = call_tool(
                &ctx,
                "exec_command",
                &json!({ "cmd": command, "timeout_ms": 10_000, "yield_time_ms": 10_000 }),
            );
            assert_eq!(output["command_ok"], true, "{script_name}: {output}");
            let stdout = output["stdout"].as_str().unwrap_or_default();
            assert!(
                stdout.contains("tooling-space-path-ok"),
                "{script_name}: {output}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_workspace_scripts_preserve_space_paths_and_arguments() {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempdir().expect("workspace parent");
        let workspace = parent.path().join("Life Brain 中文");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let harness = tempdir().expect("harness");
        let script_name = "run tooling";
        let script_path = workspace.join(script_name);
        std::fs::write(
            &script_path,
            "#!/bin/sh\nprintf 'tooling-space-path-ok\\n'\nprintf 'argument=[%s]\\n' \"$1\"\n",
        )
        .expect("shell script");
        let mut permissions = std::fs::metadata(&script_path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions).expect("script executable");

        let ctx = ToolContext::for_test(workspace, harness.path().to_path_buf()).expect("context");
        let command = format!(r#""{script_name}" "argument with spaces""#);
        let output = call_tool(
            &ctx,
            "exec_command",
            &json!({ "cmd": command, "timeout_ms": 10_000, "yield_time_ms": 10_000 }),
        );
        assert_eq!(output["command_ok"], true, "{output}");
        let stdout = output["stdout"].as_str().unwrap_or_default();
        assert!(stdout.contains("tooling-space-path-ok"), "{output}");
        assert!(
            stdout.contains("argument=[argument with spaces]"),
            "{output}"
        );
    }
}

#[cfg(windows)]
fn windows_hidden_creation_flags() -> u32 {
    // Match frpc/cloudflared: hide console-subsystem children (python/cmd/powershell)
    // so remote exec_command does not flash a console or steal focus.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
}

fn command_for_program(program: &str, args: &[String]) -> Command {
    #[cfg(windows)]
    {
        let extension = Path::new(program)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("bat") | Some("cmd") => {
                let mut command = Command::new("cmd.exe");
                command.args(["/d", "/s", "/c"]);
                command
                    .as_std_mut()
                    .raw_arg(windows_batch_command_line(program, args));
                command.creation_flags(windows_hidden_creation_flags());
                return command;
            }
            Some("ps1") => {
                let shell = which::which("pwsh")
                    .or_else(|_| which::which("powershell"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("powershell.exe"));
                let mut command = Command::new(shell);
                command
                    .args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-File",
                        windows_command_path(program).as_str(),
                    ])
                    .args(args);
                command.creation_flags(windows_hidden_creation_flags());
                return command;
            }
            _ => {}
        }
    }

    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(windows_hidden_creation_flags());
    command
}

#[cfg(windows)]
fn windows_batch_command_line(program: &str, args: &[String]) -> String {
    let mut command_line = String::from("call ");
    command_line.push_str(&windows_batch_token(&windows_command_path(program)));
    for arg in args {
        command_line.push(' ');
        command_line.push_str(&windows_batch_token(arg));
    }
    command_line
}

#[cfg(windows)]
fn windows_batch_token(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn platform_command_path(path: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::path::PathBuf::from(windows_command_path(&path.to_string_lossy()))
    }
    #[cfg(not(windows))]
    path.to_path_buf()
}

#[cfg(windows)]
fn windows_command_path(path: &str) -> String {
    path.strip_prefix("\\\\?\\").unwrap_or(path).to_string()
}
