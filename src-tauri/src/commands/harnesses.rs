use tauri::State;

use hyprlayer_core::claude::ClaudeProbe;
use hyprlayer_core::codex::CodexProbe;
use hyprlayer_core::opencode::config as oc;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;
use hyprlayer_core::opencode::support::{
    opencode_model_is_supported, opencode_provider_is_supported,
};

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
pub async fn opencode_auth_list() -> CmdResult<Vec<oc::OpencodeAuthEntry>> {
    oc::list_auth(&crate::opencode_auth_file()).map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_set_provider_key(
    state: State<'_, AppState>,
    provider: String,
    key: String,
) -> CmdResult<Vec<oc::OpencodeAuthEntry>> {
    if !opencode_provider_is_supported(&provider) {
        return Err(AppError {
            code: "opencode_provider_unsupported".into(),
            message: format!("OpenCode provider `{provider}` is no longer supported"),
            remedy: Some("Remove its saved credential from the provider list.".into()),
        });
    }
    let auth_path = crate::opencode_auth_file();
    oc::set_provider_key(&auth_path, &provider, &key).map_err(AppError::from)?;
    state
        .client()?
        .invalidate_opencode_server()
        .await
        .map_err(AppError::from)?;
    oc::list_auth(&auth_path).map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_remove_provider(
    state: State<'_, AppState>,
    provider: String,
) -> CmdResult<Vec<oc::OpencodeAuthEntry>> {
    let auth_path = crate::opencode_auth_file();
    oc::remove_provider(&auth_path, &provider).map_err(AppError::from)?;
    state
        .client()?
        .invalidate_opencode_server()
        .await
        .map_err(AppError::from)?;
    oc::list_auth(&auth_path).map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_set_default_model(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<()> {
    let normalized = model.filter(|m| !m.is_empty());
    if normalized
        .as_deref()
        .is_some_and(|model| !opencode_model_is_supported(model))
    {
        return Err(AppError {
            code: "opencode_provider_unsupported".into(),
            message: "GitHub Copilot models are no longer supported".into(),
            remedy: Some("Choose a model from a supported OpenCode provider.".into()),
        });
    }
    state
        .client()?
        .set_opencode_default_model(normalized.clone())
        .await
        .map_err(AppError::from)?;
    oc::set_default_model(&crate::opencode_config_file(), normalized.as_deref())
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
