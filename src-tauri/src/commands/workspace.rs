use std::path::PathBuf;

use tauri::State;

use crate::app_state::{bootstrap_workspace, teardown_workspace, AppState};
use crate::error::{AppError, AppResult};
use crate::platform::open_path_in_file_manager;
use crate::tunnel::drop_workspace as drop_tunnel_workspace;
use crate::tools::policy::PolicySettings;
use crate::workspace::resources::{
    assign_free_workspace_ports, validate_workspace_resources_update,
};
use crate::workspace::WorkspaceProfile;

#[tauri::command]
pub fn list_workspaces(state: State<'_, AppState>) -> AppResult<Vec<WorkspaceProfile>> {
    state.with_workspaces(|store| Ok(store.list().to_vec()))
}

#[tauri::command]
pub fn create_workspace(
    state: State<'_, AppState>,
    path: String,
    name: Option<String>,
) -> AppResult<WorkspaceProfile> {
    state.with_workspaces(|store| {
        let mut profile = WorkspaceProfile::new(path, name);
        // Create should not fail just because default ports are already claimed.
        // Pick free ports now; start/update still enforce conflict checks.
        assign_free_workspace_ports(store.list(), &mut profile)?;
        bootstrap_workspace(store, &profile.id)?;
        store.add(profile.clone())?;
        Ok(profile)
    })
}

#[tauri::command]
pub fn update_workspace(state: State<'_, AppState>, profile: WorkspaceProfile) -> AppResult<()> {
    state.with_workspaces(|store| {
        let current = store
            .get(&profile.id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {}", profile.id)))?;
        validate_permission_configuration(&profile)?;
        validate_gateway_allowlist(store.list(), &profile)?;
        validate_workspace_resources_update(store.list(), &current, &profile)?;
        store.update(profile)
    })
}

fn validate_permission_configuration(profile: &WorkspaceProfile) -> AppResult<()> {
    PolicySettings::from_runtime(&profile.runtime).map_err(|error| {
        AppError::Message(format!("runtime permission configuration invalid: {error}"))
    })?;
    PolicySettings::from_actions_config(&profile.actions).map_err(|error| {
        AppError::Message(format!("actions permission configuration invalid: {error}"))
    })?;
    Ok(())
}

fn validate_gateway_allowlist(
    profiles: &[WorkspaceProfile],
    profile: &WorkspaceProfile,
) -> AppResult<()> {
    let configured = profile.gateway.enabled || !profile.gateway.workspace_ids.is_empty();
    if !configured {
        return Ok(());
    }
    if !profile
        .gateway
        .workspace_ids
        .iter()
        .any(|id| id == &profile.id)
    {
        return Err(AppError::Message(
            "gateway host workspace must be present in workspace_ids".into(),
        ));
    }
    for workspace_id in &profile.gateway.workspace_ids {
        if profiles
            .iter()
            .all(|candidate| candidate.id != *workspace_id)
        {
            return Err(AppError::Message(format!(
                "gateway workspace not found: {workspace_id}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_gateway_allowlist, validate_permission_configuration};
    use crate::error::AppError;
    use crate::workspace::WorkspaceProfile;

    fn profile(id: &str) -> WorkspaceProfile {
        let mut profile = WorkspaceProfile::new(format!("C:/workspace/{id}"), Some(id.into()));
        profile.id = id.into();
        profile
    }

    #[test]
    fn gateway_allowlist_requires_existing_host_and_targets() {
        let host = profile("host");
        let target = profile("target");
        let profiles = vec![host.clone(), target];
        let mut configured = host.clone();
        configured.gateway.enabled = true;
        configured.gateway.workspace_ids = vec!["host".into(), "target".into()];
        assert!(validate_gateway_allowlist(&profiles, &configured).is_ok());

        configured.gateway.workspace_ids.push("deleted".into());
        assert!(matches!(
            validate_gateway_allowlist(&profiles, &configured),
            Err(AppError::Message(message)) if message.contains("deleted")
        ));
    }

    #[test]
    fn legacy_disabled_empty_gateway_remains_saveable() {
        let host = profile("host");
        assert!(validate_gateway_allowlist(std::slice::from_ref(&host), &host).is_ok());
    }

    #[test]
    fn permission_configuration_is_validated_before_persisting() {
        let profile = profile("host");
        assert!(validate_permission_configuration(&profile).is_ok());

        let mut invalid = profile.clone();
        invalid.runtime.permission_policy_version = 2;
        assert!(matches!(
            validate_permission_configuration(&invalid),
            Err(AppError::Message(message)) if message.contains("runtime permission configuration")
        ));

        let mut invalid_actions = profile;
        invalid_actions.actions.permission_policy_version = 1;
        invalid_actions.actions.permission_mode = "trusted".into();
        assert!(matches!(
            validate_permission_configuration(&invalid_actions),
            Err(AppError::Message(message)) if message.contains("actions permission configuration")
        ));
    }
}

#[tauri::command]
pub fn open_workspace_directory(path: String) -> AppResult<()> {
    let path = PathBuf::from(path.trim());
    open_path_in_file_manager(&path)
}

#[tauri::command]
pub fn delete_workspace(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let profile = state.with_workspaces(|store| {
        store
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {id}")))
    })?;
    tauri::async_runtime::block_on(drop_tunnel_workspace(&id))?;
    state.with_runtime(|runtime| {
        runtime.drop_workspace(&profile);
        Ok(())
    })?;
    state.with_workspaces(|store| {
        if store.remove(&id)?.is_some() {
            teardown_workspace(store, &id)?;
        }
        Ok(())
    })
}
