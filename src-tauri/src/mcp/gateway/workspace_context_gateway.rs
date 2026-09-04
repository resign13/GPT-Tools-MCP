use super::*;
use crate::mcp::workspace_context::{PinOptions, WorkspaceContextPin};

impl GatewayRouter {
    pub(super) fn pin_workspace_context(&self, session_key: &str, args: &Value) -> Value {
        let options = match PinOptions::from_value(args) {
            Ok(options) => options,
            Err(error) => return error.tool_value(None),
        };
        let (_, _, initial) = match self.binding_snapshot(session_key) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(_permit) = self.acquire_permit(&initial.lock_key, true) else {
            return context_error(
                "WORKSPACE_CONTEXT_BUSY",
                "The workspace is busy; retry the pin request shortly.",
            );
        };
        let (host, target, snapshot) = match self.binding_snapshot(session_key) {
            Ok(value) => value,
            Err(error) => return error,
        };
        if !same_binding_context(&initial, &snapshot) {
            return context_error(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The workspace context changed while this pin request was waiting.",
            );
        }
        if let Some(existing) = snapshot.context_lock.clone() {
            let mut pin = existing.lock().expect("workspace context lock");
            return match pin.validate() {
                Ok(_) => {
                    let mut value = context_error(
                        "WORKSPACE_CONTEXT_ALREADY_PINNED",
                        "This conversation already has a pinned workspace context.",
                    );
                    value["error"]["details"] = pin.snapshot("valid", None);
                    value
                }
                Err(error) => error.tool_value(Some(&pin)),
            };
        }
        if snapshot.context.sessions.has_active_sessions() {
            return context_error(
                "WORKSPACE_CONTEXT_BUSY",
                "Stop active command sessions before pinning another execution root.",
            );
        }
        let pin = match WorkspaceContextPin::create(
            PathBuf::from(&target.path).as_path(),
            &options,
            snapshot.session_key_source.as_str(),
        ) {
            Ok(pin) => pin,
            Err(error) => return error.tool_value(None),
        };
        let identity = pin.git_identity();
        let context = match self.build_context_at_root(
            &host,
            &target,
            pin.active_root.clone(),
            Some(identity),
        ) {
            Ok(context) => context,
            Err(error) => return error,
        };
        let context_lock = Arc::new(Mutex::new(pin));
        let mut bindings = self.bindings.lock().expect("gateway binding lock");
        let Some(binding) = bindings.get_mut(session_key) else {
            return tool_err_code(
                "workspace_not_selected",
                "The workspace binding expired while pinning.",
                "gateway",
            );
        };
        if binding.workspace_id != snapshot.workspace_id
            || binding.fingerprint != snapshot.fingerprint
            || !Arc::ptr_eq(&binding.context, &snapshot.context)
            || binding.context_lock.is_some()
        {
            return context_error(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The workspace binding changed while pinning; retry after checking the context.",
            );
        }
        binding.context = context;
        binding.lock_key = workspace_lock_key(
            &context_lock
                .lock()
                .expect("workspace context lock")
                .active_root
                .to_string_lossy(),
        );
        binding.context_lock = Some(context_lock.clone());
        binding.last_used = Instant::now();
        let pin = context_lock.lock().expect("workspace context lock");
        let mut result = pin.snapshot("valid", None);
        result["workspace_id"] = Value::String(target.id);
        tool_ok(result)
    }

    pub(super) fn get_workspace_context(&self, session_key: &str) -> Value {
        let (_, _, initial) = match self.binding_snapshot(session_key) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(_permit) = self.acquire_permit(&initial.lock_key, false) else {
            return context_error(
                "WORKSPACE_CONTEXT_BUSY",
                "The workspace is busy; retry the context check shortly.",
            );
        };
        let (_, target, snapshot) = match self.binding_snapshot(session_key) {
            Ok(value) => value,
            Err(error) => return error,
        };
        if !same_binding_context(&initial, &snapshot) {
            return context_error(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The workspace context changed while this check was waiting.",
            );
        }
        let Some(context_lock) = snapshot.context_lock else {
            return tool_ok(json!({
                "locked": false,
                "status": "unlocked",
                "workspace_id": target.id,
                "configured_root": snapshot.context.workspace_path(),
                "active_root": snapshot.context.workspace_path(),
                "session_key_source": snapshot.session_key_source.as_str()
            }));
        };
        let mut pin = context_lock.lock().expect("workspace context lock");
        match pin.validate() {
            Ok(_) => tool_ok(pin.snapshot("valid", None)),
            Err(error) => {
                // A failed validation must be a failed tool result.  Returning
                // `ok: true` with an embedded drift_error lets a caller ignore
                // the authoritative context check and continue on a stale pin.
                error.tool_value(Some(&pin))
            }
        }
    }

    pub(super) fn unpin_workspace_context(&self, session_key: &str, args: &Value) -> Value {
        if args.get("confirm").and_then(Value::as_bool) != Some(true) {
            return context_error(
                "WORKSPACE_CONTEXT_CONFIRMATION_REQUIRED",
                "unpin_workspace_context requires confirm=true.",
            );
        }
        let (_, _, initial) = match self.binding_snapshot(session_key) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let Some(_permit) = self.acquire_permit(&initial.lock_key, true) else {
            return context_error(
                "WORKSPACE_CONTEXT_BUSY",
                "The workspace is busy; retry the unpin request shortly.",
            );
        };
        let (host, target, snapshot) = match self.binding_snapshot(session_key) {
            Ok(value) => value,
            Err(error) => return error,
        };
        if !same_binding_context(&initial, &snapshot) {
            return context_error(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The workspace context changed while this unpin request was waiting.",
            );
        }
        if snapshot.context_lock.is_none() {
            return tool_ok(json!({
                "locked": false,
                "status": "unlocked",
                "workspace_id": target.id,
                "active_root": snapshot.context.workspace_path()
            }));
        }
        if snapshot.context.sessions.has_active_sessions() {
            return context_error(
                "WORKSPACE_CONTEXT_BUSY",
                "Stop active command sessions before unpinning the workspace context.",
            );
        }
        let context = match self.build_context(&host, &target) {
            Ok(context) => context,
            Err(error) => return error,
        };
        let mut bindings = self.bindings.lock().expect("gateway binding lock");
        let Some(binding) = bindings.get_mut(session_key) else {
            return tool_err_code(
                "workspace_not_selected",
                "The workspace binding expired while unpinning.",
                "gateway",
            );
        };
        if binding.workspace_id != snapshot.workspace_id
            || binding.fingerprint != snapshot.fingerprint
            || !Arc::ptr_eq(&binding.context, &snapshot.context)
        {
            return context_error(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The workspace binding changed while unpinning.",
            );
        }
        binding.context = context;
        binding.context_lock = None;
        binding.lock_key = workspace_lock_key(&target.path);
        binding.last_used = Instant::now();
        tool_ok(json!({
            "locked": false,
            "status": "unlocked",
            "workspace_id": target.id,
            "configured_root": target.path,
            "active_root": target.path
        }))
    }
}

pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "pin_workspace_context",
            "title": "Pin workspace context",
            "description": "Lock this conversation to an existing Git worktree inside the selected workspace and rebuild all project tools on that root.",
            "inputSchema": {
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "expected_branch": {"type": "string", "minLength": 1},
                    "expires_in_minutes": {"type": "integer", "minimum": 5, "maximum": 480, "default": 120},
                    "allow_protected_branch": {"type": "boolean", "default": false},
                    "confirm": {"type": "boolean", "default": false}
                },
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}
        }),
        json!({
            "name": "get_workspace_context",
            "title": "Get workspace context",
            "description": "Validate and return the selected workspace, active Git worktree, branch, HEAD, and lock expiry for this conversation.",
            "inputSchema": {"type": "object", "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }),
        json!({
            "name": "unpin_workspace_context",
            "title": "Unpin workspace context",
            "description": "End the developer session and rebuild project tools on the configured workspace root.",
            "inputSchema": {"type": "object", "required": ["confirm"], "properties": {"confirm": {"type": "boolean"}}, "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false}
        }),
    ]
}

pub(super) fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "pin_workspace_context" | "get_workspace_context" | "unpin_workspace_context"
    )
}

fn context_error(code: &'static str, message: &'static str) -> Value {
    tool_err_code(code, message, "workspace_context")
}
