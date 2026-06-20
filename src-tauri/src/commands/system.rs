use std::path::PathBuf;

use hyprlayer_core::process::binaries::{BinaryKind, BinaryStatus};
use tauri::State;
use tauri_specta::Event;

use crate::editors::{self, EditorTarget};
use crate::error::{AppError, CmdResult};
use crate::events::BinariesChanged;
use crate::settings::AppSettings;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn set_binary_override(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    kind: BinaryKind,
    path: Option<String>,
) -> CmdResult<Vec<BinaryStatus>> {
    // Persist first: if the settings write fails, the live resolver is left
    // untouched, so in-memory and on-disk state never diverge.
    state
        .settings
        .update(|s| match &path {
            Some(p) => {
                s.binary_overrides.insert(kind, p.clone());
            }
            None => {
                s.binary_overrides.remove(&kind);
            }
        })
        .await?;

    state
        .binaries
        .set_override(kind, path.map(PathBuf::from))
        .await;

    let statuses = state.binaries.status_all().await;
    let _ = BinariesChanged.emit(&app);
    Ok(statuses)
}

/// Play the default OS notification sound out-of-band from the toast.
///
/// On macOS 13+ the system silently drops the `soundName` on
/// `NSUserNotification` (which `tauri-plugin-notification`'s backend uses),
/// so a banner-attached sound is unreliable. We decouple: fire-and-forget
/// `afplay` on a known system sound file. The visual banner still goes
/// through the plugin; the sound is independent and reliable across
/// macOS versions.
///
/// On Windows/Linux this is a no-op (the plugin's sound field works there).
#[tauri::command]
#[specta::specta]
pub fn play_sound() -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    {
        // System default sound; ships with every macOS install. Spawn detached
        // so we don't block the IPC reply on playback.
        let _ = std::process::Command::new("/usr/bin/afplay")
            .arg("/System/Library/Sounds/Funk.aiff")
            .spawn()
            .map_err(|e| AppError::internal(format!("afplay: {e}")))?;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn redetect_binaries(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Vec<BinaryStatus>> {
    state.binaries.redetect().await;
    let statuses = state.binaries.status_all().await;
    let _ = BinariesChanged.emit(&app);
    // A newly-installed hyprlayer may now be able to migrate a pre-v4 config
    // this session was stuck on.
    crate::migrate_config_if_needed(&state.binaries, &state.config, &app).await;
    Ok(statuses)
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    Ok(state.settings.get().await)
}

#[tauri::command]
#[specta::specta]
pub async fn set_app_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> CmdResult<AppSettings> {
    state.settings.set(settings).await?;
    Ok(state.settings.get().await)
}

/// Installed editors/terminals plus the OS file manager, for the "Open in…"
/// menu. Pure filesystem probing (no child processes), so it's safe to call on
/// every menu open; the frontend caches it anyway.
#[tauri::command]
#[specta::specta]
pub async fn detect_editors() -> CmdResult<Vec<EditorTarget>> {
    // Detection rasterizes app icons via `sips`, so keep it off the async
    // runtime's worker threads.
    tokio::task::spawn_blocking(editors::detect)
        .await
        .map_err(|e| AppError::internal(format!("editor detection failed: {e}")))
}

/// The authenticated GitHub login, for the "username/" branch-prefix option.
/// `None` when gh is missing or not logged in (the UI just hides the option).
#[tauri::command]
#[specta::specta]
pub async fn github_username(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(None);
    };
    Ok(hyprlayer_core::pr::github_login(&gh).await)
}

/// The shell username used to prefill the onboarding "Your name" field. Mirrors
/// the CLI's `prompt_for_username` default (`$USER` → `$USERNAME` → `"user"`)
/// exactly: the embedded `thoughts init`/`ai configure` PTY inherits this same
/// process environment, so this is precisely the default the CLI would show.
#[tauri::command]
#[specta::specta]
pub fn default_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}
