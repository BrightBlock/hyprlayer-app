use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub use hyprlayer_core::runtime::providers::ModelWindow;

#[tauri::command]
#[specta::specta]
pub async fn claude_model_windows(state: State<'_, AppState>) -> CmdResult<Vec<ModelWindow>> {
    state
        .client
        .claude_model_windows()
        .await
        .map_err(AppError::from)
}
