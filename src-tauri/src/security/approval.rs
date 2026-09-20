use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const TTL: Duration = Duration::from_secs(600);
const TERMINAL_RETENTION: Duration = Duration::from_secs(60);
const CAPACITY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalScope {
    pub context: String,
    pub task: String,
    pub execution_root: String,
    pub policy_revision: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Pending,
    Approved,
    Denied,
    Expired,
    Consumed,
    LaunchFailed,
    Invalidated,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApprovalStatus {
    pub approval_id: String,
    pub task: String,
    /// A display-safe identifier for the originating conversation. The raw
    /// conversation key is never returned to MCP or the desktop UI.
    pub context_hash: String,
    pub execution_root: String,
    pub tool: String,
    /// Soft capabilities that caused this request to require approval. These
    /// are enum-derived labels, never user-provided command text.
    pub capabilities: Vec<String>,
    /// A bounded, secret-free description suitable for the desktop review
    /// surface (for example `exec_command: curl (+1 args)`).
    pub operation_summary: String,
    /// Snapshot of the execution isolation policy at request creation.
    pub isolation: String,
    pub decision: ApprovalDecision,
    pub remaining_seconds: u64,
    pub created_at_epoch_seconds: u64,
    pub expires_at_epoch_seconds: u64,
}

struct Record {
    scope: ApprovalScope,
    fingerprint: String,
    tool: String,
    capabilities: Vec<String>,
    operation_summary: String,
    isolation: String,
    request_id: Option<String>,
    created: Instant,
    created_at_epoch_seconds: u64,
    sequence: u64,
    decision: ApprovalDecision,
}

impl Record {
    fn effective_decision(&self, now: Instant) -> ApprovalDecision {
        let remaining = TTL.saturating_sub(now.saturating_duration_since(self.created));
        if remaining.is_zero()
            && matches!(
                self.decision,
                ApprovalDecision::Pending | ApprovalDecision::Approved
            )
        {
            ApprovalDecision::Expired
        } else {
            self.decision
        }
    }

    fn status(&self, id: &str, now: Instant) -> ApprovalStatus {
        let remaining = TTL.saturating_sub(now.saturating_duration_since(self.created));
        let context_hash = format!("{:x}", Sha256::digest(self.scope.context.as_bytes()));
        ApprovalStatus {
            approval_id: id.into(),
            task: self.scope.task.clone(),
            context_hash,
            execution_root: self.scope.execution_root.clone(),
            tool: self.tool.clone(),
            capabilities: self.capabilities.clone(),
            operation_summary: self.operation_summary.clone(),
            isolation: self.isolation.clone(),
            decision: self.effective_decision(now),
            remaining_seconds: remaining.as_secs(),
            created_at_epoch_seconds: self.created_at_epoch_seconds,
            expires_at_epoch_seconds: self
                .created_at_epoch_seconds
                .saturating_add(TTL.as_secs()),
        }
    }
}

pub struct ApprovalStore {
    records: Mutex<HashMap<String, Record>>,
    next_sequence: Mutex<u64>,
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            next_sequence: Mutex::new(0),
        }
    }
}

fn fingerprint(tool: &str, args: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let ordered: std::collections::BTreeMap<_, _> = map
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect();
                serde_json::to_value(ordered).expect("JSON object")
            }
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    let encoded = serde_json::to_vec(&(tool, canonical(args))).expect("JSON arguments");
    format!("{:x}", Sha256::digest(encoded))
}

impl ApprovalStore {
    fn cleanup_locked(records: &mut HashMap<String, Record>, now: Instant) {
        for record in records.values_mut() {
            if matches!(
                record.decision,
                ApprovalDecision::Pending | ApprovalDecision::Approved
            ) && record.effective_decision(now) == ApprovalDecision::Expired
            {
                record.decision = ApprovalDecision::Expired;
            }
        }
        records.retain(|_, record| {
            let terminal = matches!(
                record.decision,
                ApprovalDecision::Denied
                    | ApprovalDecision::Expired
                    | ApprovalDecision::Consumed
                    | ApprovalDecision::LaunchFailed
                    | ApprovalDecision::Invalidated
            );
            if !terminal {
                return true;
            }
            let retention = if record.decision == ApprovalDecision::Expired {
                TTL + TERMINAL_RETENTION
            } else {
                TERMINAL_RETENTION
            };
            now.saturating_duration_since(record.created) < retention
        });
    }

    pub fn request(
        &self,
        scope: ApprovalScope,
        tool: &str,
        args: &Value,
    ) -> Result<ApprovalStatus, &'static str> {
        self.request_with_id(scope, tool, args, None)
    }

    /// Create or resume a one-shot request. An explicit request id represents
    /// one client retry attempt; an omitted id only reuses a still-live request.
    pub fn request_with_id(
        &self,
        scope: ApprovalScope,
        tool: &str,
        args: &Value,
        request_id: Option<&str>,
    ) -> Result<ApprovalStatus, &'static str> {
        self.request_with_details(
            scope,
            tool,
            args,
            request_id,
            Vec::new(),
            String::new(),
            String::new(),
        )
    }

    /// Create a request with server-derived display metadata. The metadata is
    /// captured together with the effect fingerprint, so the desktop never
    /// has to trust a client-supplied command summary when deciding.
    pub fn request_with_details(
        &self,
        scope: ApprovalScope,
        tool: &str,
        args: &Value,
        request_id: Option<&str>,
        capabilities: Vec<String>,
        operation_summary: String,
        isolation: String,
    ) -> Result<ApprovalStatus, &'static str> {
        let now = Instant::now();
        let fingerprint = fingerprint(tool, args);
        let mut records = self
            .records
            .lock()
            .map_err(|_| "approval_store_unavailable")?;
        Self::cleanup_locked(&mut records, now);

        let request_id = request_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(request_id) = request_id.as_deref() {
            if let Some((id, record)) = records
                .iter()
                .find(|(_, record)| record.request_id.as_deref() == Some(request_id))
            {
                if record.scope != scope || record.tool != tool || record.fingerprint != fingerprint
                {
                    return Err("approval_request_conflict");
                }
                return Ok(record.status(id, now));
            }
        } else if let Some((id, record)) = records.iter().find(|(_, record)| {
            record.scope == scope
                && record.tool == tool
                && record.fingerprint == fingerprint
                && matches!(
                    record.effective_decision(now),
                    ApprovalDecision::Pending | ApprovalDecision::Approved
                )
        }) {
            return Ok(record.status(id, now));
        }
        if records.len() >= CAPACITY {
            return Err("approval_queue_full");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let sequence = {
            let mut next = self
                .next_sequence
                .lock()
                .map_err(|_| "approval_store_unavailable")?;
            let sequence = *next;
            *next = next.saturating_add(1);
            sequence
        };
        let created_at_epoch_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let record = Record {
            scope,
            fingerprint,
            tool: tool.into(),
            capabilities,
            operation_summary,
            isolation,
            request_id,
            created: now,
            created_at_epoch_seconds,
            sequence,
            decision: ApprovalDecision::Pending,
        };
        let status = record.status(&id, now);
        records.insert(id, record);
        Ok(status)
    }

    pub fn status(&self, context: &str, id: &str) -> Option<ApprovalStatus> {
        let mut records = self.records.lock().ok()?;
        Self::cleanup_locked(&mut records, Instant::now());
        let record = records.get(id)?;
        (record.scope.context == context).then(|| record.status(id, Instant::now()))
    }

    /// Only desktop IPC adapters may expose this method; never register it as MCP.
    pub fn decide(&self, id: &str, approve: bool) -> Result<ApprovalStatus, &'static str> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| "approval_store_unavailable")?;
        Self::cleanup_locked(&mut records, Instant::now());
        let record = records.get_mut(id).ok_or("approval_not_found")?;
        if record.status(id, Instant::now()).decision != ApprovalDecision::Pending {
            return Err("approval_not_pending");
        }
        record.decision = if approve {
            ApprovalDecision::Approved
        } else {
            ApprovalDecision::Denied
        };
        Ok(record.status(id, Instant::now()))
    }

    pub fn consume(
        &self,
        id: &str,
        scope: &ApprovalScope,
        tool: &str,
        args: &Value,
    ) -> Result<(), &'static str> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| "approval_store_unavailable")?;
        Self::cleanup_locked(&mut records, Instant::now());
        let record = records.get_mut(id).ok_or("approval_not_found")?;
        // A different caller must not invalidate the owner's request.
        if record.scope.context != scope.context {
            return Err("approval_not_found");
        }
        if record.scope != *scope || record.fingerprint != fingerprint(tool, args) {
            record.decision = ApprovalDecision::Invalidated;
            return Err("approval_invalidated");
        }
        match record.status(id, Instant::now()).decision {
            ApprovalDecision::Approved => {
                record.decision = ApprovalDecision::Consumed;
                Ok(())
            }
            ApprovalDecision::Expired => Err("approval_expired"),
            ApprovalDecision::Denied => Err("approval_denied"),
            ApprovalDecision::Consumed => Err("approval_consumed"),
            ApprovalDecision::LaunchFailed => Err("approval_launch_failed"),
            ApprovalDecision::Invalidated => Err("approval_invalidated"),
            ApprovalDecision::Pending => Err("approval_required"),
        }
    }

    /// Record that an approved operation reached the runner but no process
    /// could be started. The receipt is terminal: callers must create a new
    /// request and repeat policy/approval evaluation instead of replaying the
    /// old authorization token.
    pub fn mark_launch_failed(
        &self,
        id: &str,
        scope: &ApprovalScope,
        tool: &str,
        args: &Value,
    ) -> Result<ApprovalStatus, &'static str> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| "approval_store_unavailable")?;
        Self::cleanup_locked(&mut records, Instant::now());
        let record = records.get_mut(id).ok_or("approval_not_found")?;
        if record.scope.context != scope.context {
            return Err("approval_not_found");
        }
        if record.scope != *scope || record.fingerprint != fingerprint(tool, args) {
            record.decision = ApprovalDecision::Invalidated;
            return Err("approval_invalidated");
        }
        match record.decision {
            ApprovalDecision::Consumed => {
                record.decision = ApprovalDecision::LaunchFailed;
                Ok(record.status(id, Instant::now()))
            }
            ApprovalDecision::LaunchFailed => Ok(record.status(id, Instant::now())),
            ApprovalDecision::Pending => Err("approval_required"),
            ApprovalDecision::Approved => Err("approval_not_consumed"),
            ApprovalDecision::Denied => Err("approval_denied"),
            ApprovalDecision::Expired => Err("approval_expired"),
            ApprovalDecision::Invalidated => Err("approval_invalidated"),
        }
    }

    pub fn pending(&self) -> Vec<ApprovalStatus> {
        let Ok(mut records) = self.records.lock() else {
            return Vec::new();
        };
        let now = Instant::now();
        Self::cleanup_locked(&mut records, now);
        let mut pending: Vec<_> = records
            .iter()
            .map(|(id, record)| (record.sequence, id, record.status(id, now)))
            .filter(|(_, _, status)| status.decision == ApprovalDecision::Pending)
            .map(|(sequence, id, status)| (sequence, id.to_string(), status))
            .collect();
        pending.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
        });
        pending.into_iter().map(|(_, _, status)| status).collect()
    }

    pub fn remove_context(&self, context: &str) {
        if let Ok(mut records) = self.records.lock() {
            Self::cleanup_locked(&mut records, Instant::now());
            records.retain(|_, record| record.scope.context != context);
        }
    }

    /// Revoke every approval tied to one execution root. Runtime shutdown and
    /// policy restarts use this boundary so an approval cannot outlive the
    /// listener generation that created it, while approvals for other
    /// workspaces remain intact.
    pub fn remove_execution_root(&self, execution_root: &str) -> usize {
        let Ok(mut records) = self.records.lock() else {
            return 0;
        };
        Self::cleanup_locked(&mut records, Instant::now());
        let target = normalized_root(execution_root);
        let before = records.len();
        records.retain(|_, record| normalized_root(&record.scope.execution_root) != target);
        before.saturating_sub(records.len())
    }
}

fn normalized_root(value: &str) -> String {
    let path = Path::new(value);
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope() -> ApprovalScope {
        ApprovalScope {
            context: "session-a".into(),
            task: "task".into(),
            execution_root: "D:/project".into(),
            policy_revision: "v1".into(),
        }
    }

    #[test]
    fn permission_approval_is_once_and_argument_bound() {
        let store = ApprovalStore::default();
        let args = json!({"cmd":"build", "nested":{"b":2,"a":1}});
        let pending = store.request(scope(), "exec_command", &args).unwrap();
        assert_eq!(
            store
                .request(scope(), "exec_command", &args)
                .unwrap()
                .approval_id,
            pending.approval_id
        );
        assert!(store
            .consume(&pending.approval_id, &scope(), "exec_command", &args)
            .is_err());
        store.decide(&pending.approval_id, true).unwrap();
        store
            .consume(&pending.approval_id, &scope(), "exec_command", &args)
            .unwrap();
        assert_eq!(
            store.consume(&pending.approval_id, &scope(), "exec_command", &args),
            Err("approval_consumed")
        );
        let next = store.request(scope(), "exec_command", &args).unwrap();
        assert_eq!(next.decision, ApprovalDecision::Pending);
        assert_ne!(next.approval_id, pending.approval_id);

        let retry = store
            .request_with_id(scope(), "exec_command", &args, Some("attempt-1"))
            .unwrap();
        store.decide(&retry.approval_id, true).unwrap();
        store
            .consume(&retry.approval_id, &scope(), "exec_command", &args)
            .unwrap();
        assert_eq!(
            store
                .request_with_id(scope(), "exec_command", &args, Some("attempt-1"))
                .unwrap()
                .decision,
            ApprovalDecision::Consumed
        );
    }

    #[test]
    fn permission_approval_denial_expiry_and_context_isolation() {
        let store = ApprovalStore::default();
        let a = store.request(scope(), "exec_command", &json!({})).unwrap();
        let mut other = scope();
        other.context = "session-b".into();
        assert!(store.status(&other.context, &a.approval_id).is_none());
        assert_eq!(
            store.consume(&a.approval_id, &other, "exec_command", &json!({})),
            Err("approval_not_found")
        );
        store.decide(&a.approval_id, false).unwrap();
        assert_eq!(
            store.consume(&a.approval_id, &scope(), "exec_command", &json!({})),
            Err("approval_denied")
        );
        let b = store
            .request(scope(), "exec_command", &json!({"cmd":"other"}))
            .unwrap();
        store.decide(&b.approval_id, true).unwrap();
        store
            .records
            .lock()
            .unwrap()
            .get_mut(&b.approval_id)
            .unwrap()
            .created = Instant::now() - TTL;
        assert_eq!(
            store.consume(
                &b.approval_id,
                &scope(),
                "exec_command",
                &json!({"cmd":"other"})
            ),
            Err("approval_expired")
        );
    }

    #[test]
    fn permission_approval_invalidates_changed_scope_and_limits_capacity() {
        let store = ApprovalStore::default();
        let a = store.request(scope(), "exec_command", &json!({})).unwrap();
        store.decide(&a.approval_id, true).unwrap();
        let mut changed = scope();
        changed.policy_revision = "v2".into();
        assert_eq!(
            store.consume(&a.approval_id, &changed, "exec_command", &json!({})),
            Err("approval_invalidated")
        );
        for n in 1..CAPACITY {
            store
                .request(scope(), "exec_command", &json!({"n":n}))
                .unwrap();
        }
        assert!(store
            .request(scope(), "exec_command", &json!({"overflow":true}))
            .is_err());
        store.remove_context("session-a");
        assert!(store.pending().is_empty());
        assert!(store.request(scope(), "exec_command", &json!({})).is_ok());
    }

    #[test]
    fn permission_approval_consumption_is_atomic_under_concurrent_retries() {
        let store = std::sync::Arc::new(ApprovalStore::default());
        let pending = store
            .request(scope(), "exec_command", &json!({"cmd":"build"}))
            .unwrap();
        store.decide(&pending.approval_id, true).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let id = pending.approval_id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .consume(&id, &scope(), "exec_command", &json!({"cmd":"build"}))
                        .is_ok()
                })
            })
            .collect();
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok())
                .filter(|ok| *ok)
                .count(),
            1
        );
    }

    #[test]
    fn terminal_records_are_reclaimed_after_retention() {
        let store = ApprovalStore::default();
        let pending = store.request(scope(), "exec_command", &json!({})).unwrap();
        store.decide(&pending.approval_id, false).unwrap();
        store
            .records
            .lock()
            .unwrap()
            .get_mut(&pending.approval_id)
            .unwrap()
            .created = Instant::now() - TERMINAL_RETENTION;
        assert!(store.request(scope(), "exec_command", &json!({})).is_ok());
        assert!(store.status("session-a", &pending.approval_id).is_none());
    }

    #[test]
    fn removing_an_execution_root_keeps_other_workspace_approvals() {
        let store = ApprovalStore::default();
        let mut first = scope();
        first.execution_root = "D:/project".into();
        let mut second = scope();
        second.context = "session-b".into();
        second.execution_root = "D:/other".into();
        let removed = store
            .request(first, "exec_command", &json!({"cmd": "custom"}))
            .unwrap();
        let kept = store
            .request(second, "exec_command", &json!({"cmd": "custom"}))
            .unwrap();

        assert_eq!(store.remove_execution_root("d:\\project\\"), 1);
        assert!(store.status("session-a", &removed.approval_id).is_none());
        assert!(store.status("session-b", &kept.approval_id).is_some());
    }

    #[test]
    fn detailed_request_contains_only_server_derived_review_metadata() {
        let store = ApprovalStore::default();
        let status = store
            .request_with_details(
                scope(),
                "exec_command",
                &json!({"cmd": "custom --token SECRET"}),
                None,
                vec!["unlisted_executable".into(), "custom_environment".into()],
                "exec_command: custom (+2 args)".into(),
                "strict".into(),
            )
            .unwrap();
        assert_eq!(status.capabilities, ["unlisted_executable", "custom_environment"]);
        assert_eq!(status.operation_summary, "exec_command: custom (+2 args)");
        assert_eq!(status.isolation, "strict");
    }

    #[test]
    fn launch_failure_is_terminal_and_requires_a_new_request() {
        let store = ApprovalStore::default();
        let args = json!({"cmd": "custom-tool"});
        let pending = store.request(scope(), "exec_command", &args).unwrap();
        store.decide(&pending.approval_id, true).unwrap();
        let consumed = store
            .consume(&pending.approval_id, &scope(), "exec_command", &args);
        assert!(consumed.is_ok());

        let failed = store
            .mark_launch_failed(&pending.approval_id, &scope(), "exec_command", &args)
            .unwrap();
        assert_eq!(failed.decision, ApprovalDecision::LaunchFailed);
        assert_eq!(
            store.consume(&pending.approval_id, &scope(), "exec_command", &args),
            Err("approval_launch_failed")
        );

        let retry = store
            .request_with_id(scope(), "exec_command", &args, Some("new-attempt"))
            .unwrap();
        assert_eq!(retry.decision, ApprovalDecision::Pending);
        assert_ne!(retry.approval_id, pending.approval_id);
    }
}
