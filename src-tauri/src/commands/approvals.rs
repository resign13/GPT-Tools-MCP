use tauri::{State, WebviewWindow};

use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::security::approval::ApprovalStatus;

fn ensure_main_window(window: &WebviewWindow) -> AppResult<()> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(AppError::Message(
            "审批操作只允许来自主桌面窗口。".into(),
        ))
    }
}

fn approval_error(code: &str) -> AppError {
    AppError::Message(format!("approval operation failed: {code}"))
}

/// Return pending requests for the local desktop approval surface.
///
/// The MCP listener never exposes this aggregate view; it can only query a
/// status belonging to its own conversation context.
#[tauri::command]
pub fn list_pending_approvals(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<Vec<ApprovalStatus>> {
    ensure_main_window(&window)?;
    Ok(state.approvals.pending())
}

/// Decide one opaque approval id from the trusted desktop window. The command
/// accepts no capability or command payload, so the UI cannot widen the
/// immutable request captured by the MCP dispatcher.
#[tauri::command]
pub fn decide_approval(
    window: WebviewWindow,
    state: State<'_, AppState>,
    approval_id: String,
    approve: bool,
) -> AppResult<ApprovalStatus> {
    ensure_main_window(&window)?;
    let id = approval_id.trim();
    if id.is_empty() {
        return Err(AppError::Message("approval_id is required".into()));
    }
    state
        .approvals
        .decide(id, approve)
        .map_err(approval_error)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::security::approval::ApprovalStore;

    #[test]
    fn approval_error_does_not_include_request_payload() {
        let error = approval_error("approval_not_pending").to_string();
        assert_eq!(error, "approval operation failed: approval_not_pending");
    }

    #[test]
    fn shared_store_type_is_sendable_for_tauri_state() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<ApprovalStore>>();
    }
}
