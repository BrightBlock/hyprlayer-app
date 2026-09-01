use std::time::Duration;

use tauri::State;
use tauri_specta::Event;

use hyprlayer_core::claude::ClaudeProbe;
use hyprlayer_core::codex::CodexProbe;
use hyprlayer_core::opencode::config as oc;
use hyprlayer_core::process::Exec;

use crate::error::{AppError, CmdResult};
use crate::events::BinariesChanged;
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
    Ok(state.runtime.providers().opencode_catalog(force).await?)
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
    state.runtime.providers().invalidate_opencode_server().await;
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
    state.runtime.providers().invalidate_opencode_server().await;
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
        .runtime
        .settings()
        .update(|s| s.providers.opencode.default_model = normalized.clone())
        .await?;
    oc::set_default_model(&crate::opencode_config_file(), normalized.as_deref())
        .map_err(AppError::from)
}

pub use hyprlayer_core::runtime::providers::ClaudeAuthStatus;

#[tauri::command]
#[specta::specta]
pub async fn claude_auth_status(state: State<'_, AppState>) -> CmdResult<ClaudeAuthStatus> {
    Ok(state.runtime.providers().claude_auth_status().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn claude_probe(state: State<'_, AppState>) -> CmdResult<ClaudeProbe> {
    Ok(state.runtime.providers().claude_probe().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_probe(state: State<'_, AppState>) -> CmdResult<CodexProbe> {
    Ok(state.runtime.providers().codex_probe().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_login_with_api_key(
    state: State<'_, AppState>,
    api_key: String,
) -> CmdResult<CodexProbe> {
    Ok(state
        .runtime
        .providers()
        .codex_login_with_api_key(api_key)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_logout(state: State<'_, AppState>) -> CmdResult<CodexProbe> {
    Ok(state.runtime.providers().codex_logout().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn install_opencode(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let binaries = state.runtime.settings().binaries();
    let path = binaries.augmented_path().await;
    let shell = super::terminal::default_shell();

    #[cfg(windows)]
    {
        let has_npm = Exec::new(&shell)
            .args([
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "npm --version".to_string(),
            ])
            .env("PATH", path.as_os_str())
            .timeout(Duration::from_secs(20))
            .run()
            .await
            .is_ok();
        if !has_npm {
            return Err(AppError {
                code: "npm_missing".into(),
                message: "Node.js (npm) isn't installed, so OpenCode can't be installed automatically."
                    .into(),
                remedy: Some(
                    "Install Node.js (it includes npm) and click Refresh, or install OpenCode with \
                     Scoop (scoop install opencode), Chocolatey (choco install opencode), or WSL. \
                     You can also use Locate… to point at an existing opencode.exe."
                        .into(),
                ),
            });
        }
    }

    #[cfg(not(windows))]
    let args = vec![
        "-lc".to_string(),
        "curl -fsSL https://opencode.ai/install | bash".to_string(),
    ];
    #[cfg(windows)]
    let args = vec![
        "-NoProfile".to_string(),
        "-Command".to_string(),
        "npm install -g opencode-ai".to_string(),
    ];

    let result = Exec::new(&shell)
        .args(args)
        .env("PATH", path)
        .timeout(Duration::from_secs(300))
        .run()
        .await;

    #[cfg(windows)]
    result.map_err(|e| AppError {
        code: "opencode_install_failed".into(),
        message: format!("Couldn't install OpenCode via npm. {e}"),
        remedy: Some(
            "Install Node.js/npm, or install OpenCode with Scoop \
             (scoop install opencode), Chocolatey (choco install opencode), or WSL. \
             then click Refresh. You can also use Locate… to point at an existing \
             opencode.exe."
                .into(),
        ),
    })?;
    #[cfg(not(windows))]
    result.map_err(AppError::from)?;

    binaries.redetect().await;
    let _ = BinariesChanged.emit(&app);
    Ok(())
}
