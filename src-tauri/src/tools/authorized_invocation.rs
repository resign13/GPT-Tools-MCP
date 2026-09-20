use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::{json, Value};

use crate::tools::context::ToolContext;
use crate::tools::workspace::WorkspaceError;

/// A server-created, one-shot authorization handoff from dispatch to a
/// process runner. Its fields are private so request JSON cannot construct or
/// alter the execution context represented by the handoff.
#[derive(Debug)]
pub(crate) struct AuthorizedInvocation {
    tool: String,
    effect_args: Value,
    execution_fingerprint: String,
    policy_revision: String,
    state: AtomicU8,
}

const AVAILABLE: u8 = 0;
const RESERVED: u8 = 1;
const COMMITTED: u8 = 2;
const LAUNCH_FAILED: u8 = 3;
const INVALIDATED: u8 = 4;

/// A short-lived launch reservation. Dropping it before the runner confirms a
/// process was created records a terminal launch failure, so the same private
/// handoff cannot be replayed.
#[derive(Debug)]
pub(crate) struct InvocationReservation<'a> {
    invocation: &'a AuthorizedInvocation,
    committed: bool,
}

impl InvocationReservation<'_> {
    pub(crate) fn commit(mut self) {
        self.invocation.state.store(COMMITTED, Ordering::Release);
        self.committed = true;
    }
}

impl Drop for InvocationReservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.invocation
                .state
                .store(LAUNCH_FAILED, Ordering::Release);
        }
    }
}

impl AuthorizedInvocation {
    pub(crate) fn new(ctx: &ToolContext, tool: &str, args: &Value) -> Self {
        Self {
            tool: tool.to_string(),
            effect_args: effect_args(args),
            execution_fingerprint: ctx.execution_fingerprint(),
            policy_revision: ctx.permission_revision(),
            state: AtomicU8::new(AVAILABLE),
        }
    }

    /// Validate the exact operation and current context, then atomically
    /// consume this authorization. A reservation cannot be reused for a
    /// second spawn, and any context/policy/effect mismatch invalidates it.
    pub(crate) fn reserve(
        &self,
        ctx: &ToolContext,
        tool: &str,
        args: &Value,
    ) -> Result<InvocationReservation<'_>, WorkspaceError> {
        let context_matches = self.execution_fingerprint == ctx.execution_fingerprint();
        let policy_matches = self.policy_revision == ctx.permission_revision();
        let effect_matches = self.tool == tool && self.effect_args == effect_args(args);
        if !context_matches || !policy_matches || !effect_matches {
            self.state.store(INVALIDATED, Ordering::Release);
            return Err(WorkspaceError::ToolDetails {
                code: "AUTHORIZED_INVOCATION_INVALID",
                message: "The authorized operation no longer matches the active execution context.".into(),
                category: "permission",
                retryable: true,
                details: json!({
                    "context_matches": context_matches,
                    "policy_matches": policy_matches,
                    "effect_matches": effect_matches,
                    "reservation_consumed": true
                }),
            });
        }
        match self.state.compare_exchange(
            AVAILABLE,
            RESERVED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(InvocationReservation {
                invocation: self,
                committed: false,
            }),
            Err(COMMITTED) => Err(WorkspaceError::Tool {
                code: "AUTHORIZED_INVOCATION_CONSUMED",
                message: "The authorized operation has already been reserved for launch.".into(),
                category: "permission",
                retryable: false,
            }),
            Err(LAUNCH_FAILED) => Err(WorkspaceError::Tool {
                code: "AUTHORIZED_INVOCATION_LAUNCH_FAILED",
                message: "The authorized operation previously failed to launch.".into(),
                category: "permission",
                retryable: false,
            }),
            Err(RESERVED) => Err(WorkspaceError::Tool {
                code: "AUTHORIZED_INVOCATION_IN_FLIGHT",
                message: "The authorized operation is already launching.".into(),
                category: "permission",
                retryable: true,
            }),
            Err(_) => Err(WorkspaceError::Tool {
                code: "AUTHORIZED_INVOCATION_INVALID",
                message: "The authorized operation has been invalidated.".into(),
                category: "permission",
                retryable: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context() -> (tempfile::TempDir, ToolContext) {
        let workspace = tempfile::tempdir().expect("workspace");
        let harness = tempfile::tempdir().expect("harness");
        let ctx = ToolContext::for_test(workspace.path().to_path_buf(), harness.path().to_path_buf())
            .expect("context");
        (workspace, ctx)
    }

    #[test]
    fn reservation_commits_once() {
        let (_workspace, ctx) = context();
        let args = json!({"cmd": "echo ok"});
        let invocation = AuthorizedInvocation::new(&ctx, "exec_command", &args);
        let reservation = invocation.reserve(&ctx, "exec_command", &args).unwrap();
        reservation.commit();
        let error = invocation
            .reserve(&ctx, "exec_command", &args)
            .expect_err("committed handoff must not be reused");
        assert!(matches!(error, WorkspaceError::Tool { code: "AUTHORIZED_INVOCATION_CONSUMED", .. }));
    }

    #[test]
    fn dropped_reservation_is_terminal_launch_failure() {
        let (_workspace, ctx) = context();
        let args = json!({"cmd": "echo ok"});
        let invocation = AuthorizedInvocation::new(&ctx, "exec_command", &args);
        drop(invocation.reserve(&ctx, "exec_command", &args).unwrap());
        let error = invocation
            .reserve(&ctx, "exec_command", &args)
            .expect_err("failed launch handoff must not be reused");
        assert!(matches!(error, WorkspaceError::Tool { code: "AUTHORIZED_INVOCATION_LAUNCH_FAILED", .. }));
    }

    #[test]
    fn context_mismatch_invalidates_handoff() {
        let (_workspace, first) = context();
        let (_other_workspace, second) = context();
        let args = json!({"cmd": "echo ok"});
        let invocation = AuthorizedInvocation::new(&first, "exec_command", &args);
        let error = invocation
            .reserve(&second, "exec_command", &args)
            .expect_err("different execution context must be rejected");
        assert!(matches!(error, WorkspaceError::ToolDetails { code: "AUTHORIZED_INVOCATION_INVALID", .. }));
    }
}

fn effect_args(args: &Value) -> Value {
    let Some(object) = args.as_object() else {
        return args.clone();
    };
    let mut effect = object.clone();
    effect.remove("approval_id");
    effect.remove("client_request_id");
    Value::Object(effect)
}
