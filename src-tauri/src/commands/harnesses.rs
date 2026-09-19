use tauri::State;

use hyprlayer_core::claude::ClaudeProbe;
use hyprlayer_core::codex::CodexProbe;
use hyprlayer_core::opencode::config::OpencodeAuthEntry;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub use hyprlayer_core::opencode::catalog::{CatalogModel, CatalogProvider, OpencodeCatalog};

#[tauri::command]
#[specta::specta]
pub async fn opencode_catalog(
    state: State<'_, AppState>,
    force: bool,
) -> CmdResult<OpencodeCatalog> {
    state
        .client()?
        .opencode_catalog(force)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_auth_list(state: State<'_, AppState>) -> CmdResult<Vec<OpencodeAuthEntry>> {
    state
        .client()?
        .opencode_auth_list()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_set_provider_key(
    state: State<'_, AppState>,
    provider: String,
    key: String,
) -> CmdResult<Vec<OpencodeAuthEntry>> {
    state
        .client()?
        .opencode_set_provider_key(provider, key)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_remove_provider(
    state: State<'_, AppState>,
    provider: String,
) -> CmdResult<Vec<OpencodeAuthEntry>> {
    state
        .client()?
        .opencode_remove_provider(provider)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_set_default_model(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .set_opencode_default_model(model)
        .await
        .map_err(AppError::from)
}

pub use hyprlayer_core::runtime::providers::ClaudeAuthStatus;

#[tauri::command]
#[specta::specta]
pub async fn claude_auth_status(state: State<'_, AppState>) -> CmdResult<ClaudeAuthStatus> {
    state
        .client()?
        .claude_auth_status()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn claude_probe(state: State<'_, AppState>) -> CmdResult<ClaudeProbe> {
    state.client()?.claude_probe().await.map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_probe(state: State<'_, AppState>) -> CmdResult<CodexProbe> {
    state.client()?.codex_probe().await.map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_login_with_api_key(
    state: State<'_, AppState>,
    api_key: String,
) -> CmdResult<CodexProbe> {
    state
        .client()?
        .codex_login_with_api_key(api_key)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_logout(state: State<'_, AppState>) -> CmdResult<CodexProbe> {
    state.client()?.codex_logout().await.map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn install_opencode(state: State<'_, AppState>) -> CmdResult<()> {
    state
        .client()?
        .install_opencode()
        .await
        .map_err(AppError::from)
}
