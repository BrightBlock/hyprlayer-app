use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;

pub use hyprlayer_core::runtime::providers::{OllamaModelInfo, OllamaStatus};

#[tauri::command]
#[specta::specta]
pub async fn ollama_status(state: State<'_, AppState>) -> CmdResult<OllamaStatus> {
    Ok(state.runtime.providers().ollama_status().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_endpoint(
    state: State<'_, AppState>,
    endpoint: Option<String>,
) -> CmdResult<OllamaStatus> {
    let status = state
        .runtime
        .providers()
        .ollama_set_endpoint(endpoint)
        .await?;
    state.runtime.providers().invalidate_opencode_server().await;
    Ok(status)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_selected_models(
    state: State<'_, AppState>,
    models: Vec<String>,
) -> CmdResult<()> {
    state
        .runtime
        .providers()
        .ollama_set_selected_models(models)
        .await?;
    state.runtime.providers().invalidate_opencode_server().await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_default_model(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<()> {
    Ok(state
        .runtime
        .providers()
        .ollama_set_default_model(model)
        .await?)
}
