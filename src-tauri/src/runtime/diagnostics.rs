//! Read-only weak references to live contexts; policy remains owned by ToolContext.
use crate::{
    app_state::AppState,
    error::{AppError, AppResult},
    tools::{policy::PolicySettings, ToolContext},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
};
static CONTEXTS: LazyLock<Mutex<HashMap<(String, String), Weak<ToolContext>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(crate) fn register(id: &str, service: &str, context: &Arc<ToolContext>) {
    let mut contexts = CONTEXTS.lock().expect("diagnostics lock");
    contexts.retain(|_, value| value.strong_count() > 0);
    contexts.insert((id.into(), service.into()), Arc::downgrade(context));
}
#[tauri::command]
pub fn get_execution_policy_status(
    state: tauri::State<'_, AppState>,
    id: String,
    service: String,
) -> AppResult<Value> {
    if service != "mcp" && service != "actions" {
        return Err(AppError::Message("Unknown service".into()));
    }
    let configured = state.with_workspaces(|store| {
        let target = store
            .get(&id)
            .ok_or_else(|| AppError::Message("Workspace not found".into()))?;
        let target_policy = if service == "actions" {
            PolicySettings::from_actions_config(&target.actions)
        } else {
            PolicySettings::from_runtime(&target.runtime)
        }
        .map_err(AppError::Message)?;
        if service == "mcp" {
            if let Some(host) = store
                .list()
                .iter()
                .find(|p| p.gateway.enabled && p.gateway.workspace_ids.contains(&id))
            {
                let host_policy =
                    PolicySettings::from_runtime(&host.runtime).map_err(AppError::Message)?;
                return Ok(crate::mcp::gateway::intersect_policies(
                    &host_policy,
                    &target_policy,
                ));
            }
        }
        Ok(target_policy)
    })?;
    let context = CONTEXTS
        .lock()
        .expect("diagnostics lock")
        .get(&(id, service))
        .and_then(Weak::upgrade);
    let Some(context) = context else {
        return Ok(json!({"running_context": false, "configured_policy": configured.permissions}));
    };
    let current = &context.policy;
    let pending = configured.permissions != current.permissions
        || configured.allowed_commands != current.allowed_commands
        || configured.workspace_local_entries != current.workspace_local_entries
        || configured.workspace_script_extensions != current.workspace_script_extensions
        || configured.max_patch_bytes != current.max_patch_bytes;
    let status = crate::tools::dispatch::permission_status(&context)
        .map_err(|e| AppError::Message(e.message()))?;
    Ok(
        json!({"running_context": true, "restart_required": pending, "configured_policy": configured.permissions, "effective": status}),
    )
}
