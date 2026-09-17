use hyprlayer_core::branch_prefix;
use hyprlayer_core::hyprcli::CliUpdateCheck;
use hyprlayer_core::omarchy;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryStatus};
use hyprlayer_core::system::FullDiskAccess;
use tauri::{Manager, State};

use crate::editors::{self, EditorTarget};
use crate::error::{AppError, CmdResult};
use crate::state::AppState;
use hyprlayer_core::settings::AppSettings;

pub use hyprlayer_core::runtime::settings_service::{
    CliInstallMethod, CliInstallMethodId, CliUpdateResult,
};

#[tauri::command]
#[specta::specta]
pub async fn set_binary_override(
    state: State<'_, AppState>,
    kind: BinaryKind,
    path: Option<String>,
) -> CmdResult<Vec<BinaryStatus>> {
    // Only setting one: the path names a binary the *server* has to run, and
    // Locate… picks it on the client. Clearing (`None`) is valid either way.
    if path.is_some() {
        if let Some(err) = state.remote_unsupported("Choosing a binary on this machine") {
            return Err(err);
        }
    }
    state
        .client()?
        .set_binary_override(kind, path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub fn play_sound() -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/afplay")
            .arg("/System/Library/Sounds/Funk.aiff")
            .spawn()
            .map_err(|e| AppError::internal(format!("afplay: {e}")))?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn full_disk_access_status() -> FullDiskAccess {
    hyprlayer_core::system::full_disk_access(dirs::home_dir().as_deref())
}

#[tauri::command]
#[specta::specta]
pub fn open_full_disk_access_settings() -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .spawn()
        .map_err(|e| AppError::internal(format!("open: {e}")))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_omarchy_theme() -> omarchy::OmarchyStatus {
    let dir = omarchy::current_dir(dirs::state_dir().as_deref(), dirs::config_dir().as_deref());
    omarchy::status(dir.as_deref())
}

#[tauri::command]
#[specta::specta]
pub async fn quit_app(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dont_ask_again: bool,
) -> CmdResult<()> {
    if dont_ask_again {
        state
            .client()?
            .set_quit_confirm(false)
            .await
            .map_err(AppError::from)?;
    }
    // The server outlives the app on purpose (ADR-0009) — quitting never
    // shuts down the runtime, so there is nothing to await here.
    app.exit(0);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn redetect_binaries(state: State<'_, AppState>) -> CmdResult<Vec<BinaryStatus>> {
    state
        .client()?
        .redetect_binaries()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    state
        .client()?
        .get_app_settings()
        .await
        .map_err(AppError::from)
}

fn branch_prefix_changed(incoming: Option<&str>, current: Option<&str>) -> bool {
    incoming.map(str::trim) != current.map(str::trim)
}

#[tauri::command]
#[specta::specta]
pub async fn set_app_settings(
    state: State<'_, AppState>,
    mut settings: AppSettings,
) -> CmdResult<AppSettings> {
    settings.git.branch_prefix = settings.git.branch_prefix.map(|p| p.trim().to_string());
    settings.git.prefixes = settings
        .git
        .prefixes
        .map(hyprlayer_core::branch_prefix::normalize_prefixes);

    let current = state
        .client()?
        .get_app_settings()
        .await
        .map_err(AppError::from)?;
    if branch_prefix_changed(
        settings.git.branch_prefix.as_deref(),
        current.git.branch_prefix.as_deref(),
    ) {
        if let Some(p) = settings.git.branch_prefix.as_deref() {
            branch_prefix::validate_prefix(p).map_err(AppError::from)?;
        }
    }
    if settings.git.prefixes != current.git.prefixes {
        branch_prefix::validate_prefixes(settings.git.prefixes.as_deref().unwrap_or(&[]))
            .map_err(AppError::from)?;
    }

    state
        .client()?
        .set_app_settings(settings)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn claude_sync_models(
    state: State<'_, AppState>,
    add: Vec<String>,
    seen: Vec<String>,
) -> CmdResult<AppSettings> {
    state
        .client()?
        .claude_sync_models(add, seen)
        .await
        .map_err(AppError::from)
}

// Every candidate this returns is on the desktop's own machine, which is
// meaningless as an "open with" target once its path comes from a remote
// server (plan Phase 5 §5) — so this is where editor launching, which has no
// command of its own (it calls `tauri-plugin-opener` straight from the
// frontend), actually gets disabled: no editors, nothing to launch with.
#[tauri::command]
#[specta::specta]
pub async fn detect_editors(state: State<'_, AppState>) -> CmdResult<Vec<EditorTarget>> {
    if let Some(err) = state.remote_unsupported("Opening a path with a local editor") {
        return Err(err);
    }
    tokio::task::spawn_blocking(editors::detect)
        .await
        .map_err(|e| AppError::internal(format!("editor detection failed: {e}")))
}

#[tauri::command]
#[specta::specta]
pub async fn github_username(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    state
        .client()?
        .github_username()
        .await
        .map_err(AppError::from)
}

// The server's own `USER`, not the client's (ADR-0009, plan Phase 5 §5): a
// branch is created by the server's git, so a name from whichever laptop
// happened to be attached would be wrong the moment a second one attaches.
// Stays infallible so its signature, and so `bindings.ts`, are unchanged —
// takes `AppHandle` rather than `State` because an async command with a
// borrowed `State` argument must return a `Result` (a Tauri constraint, not
// a choice here).
#[tauri::command]
#[specta::specta]
pub async fn default_username(app: tauri::AppHandle) -> String {
    let Some(state) = app.try_state::<AppState>() else {
        return "user".to_string();
    };
    let Ok(client) = state.client() else {
        return "user".to_string();
    };
    client
        .default_username()
        .await
        .unwrap_or_else(|_| "user".to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn detect_cli_install_methods(
    state: State<'_, AppState>,
) -> CmdResult<Vec<CliInstallMethod>> {
    state
        .client()?
        .detect_cli_install_methods()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn install_hyprlayer_cli(
    state: State<'_, AppState>,
    method: CliInstallMethodId,
) -> CmdResult<()> {
    state
        .client()?
        .install_hyprlayer_cli(method)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn hyprlayer_cli_update_check(state: State<'_, AppState>) -> CmdResult<CliUpdateCheck> {
    state
        .client()?
        .hyprlayer_cli_update_check()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn update_hyprlayer_cli(state: State<'_, AppState>) -> CmdResult<CliUpdateResult> {
    state
        .client()?
        .update_hyprlayer_cli()
        .await
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resending_a_stored_invalid_prefix_is_not_a_change() {
        let stored = Some(" my feature/");
        assert!(!branch_prefix_changed(stored, stored));
        assert!(!branch_prefix_changed(
            Some("my feature/"),
            Some("my feature/")
        ));
    }

    #[test]
    fn incidental_whitespace_is_not_a_change() {
        assert!(!branch_prefix_changed(Some(" fix/ "), Some("fix/")));
        assert!(!branch_prefix_changed(Some("fix/"), Some(" fix/ ")));
    }

    #[test]
    fn a_real_edit_is_a_change() {
        assert!(branch_prefix_changed(Some("fix/"), Some("feat/")));
        assert!(branch_prefix_changed(Some("fix/"), None));
        assert!(branch_prefix_changed(None, Some("fix/")));
        assert!(!branch_prefix_changed(None, None));
    }
}
