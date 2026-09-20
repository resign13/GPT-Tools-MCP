use tauri::State;

use crate::app_state::AppState;
use crate::error::AppResult;
use crate::platform::open_url as platform_open_url;
use crate::update::{check_app_update as check_update, UpdateCheckResult};

#[tauri::command]
pub fn open_url(url: String) -> AppResult<()> {
    platform_open_url(&url)
}

#[tauri::command]
pub async fn check_app_update(state: State<'_, AppState>) -> AppResult<UpdateCheckResult> {
    let settings = state.with_settings(|store| Ok(store.settings()))?;
    check_update(&settings).await
}
