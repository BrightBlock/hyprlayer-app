use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub use hyprlayer_core::runtime::providers::{OllamaModelInfo, OllamaStatus};

#[tauri::command]
#[specta::specta]
pub async fn ollama_status(state: State<'_, AppState>) -> CmdResult<OllamaStatus> {
    state
        .client()?
        .ollama_status()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_endpoint(
    state: State<'_, AppState>,
    endpoint: Option<String>,
) -> CmdResult<OllamaStatus> {
    state
        .client()?
        .ollama_set_endpoint(endpoint)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_selected_models(
    state: State<'_, AppState>,
    models: Vec<String>,
) -> CmdResult<()> {
    state
        .client()?
        .ollama_set_selected_models(models)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_default_model(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .ollama_set_default_model(model)
        .await
        .map_err(AppError::from)
}
