use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use hyprlayer_core::process::binaries::{BinaryKind, BinaryStatus};
use hyprlayer_core::process::Exec;
use hyprlayer_core::system::FullDiskAccess;
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

/// Whether the app holds Full Disk Access.
///
/// macOS never prompts for this grant, so its absence is silent: gated reads
/// just fail. Meanwhile any CLI the app hosts that reaches another app's
/// container (`op`, most visibly) trips the *promptable* "access data from
/// other apps" dialog, which names Hyprlayer — macOS attributes a child's
/// access to its responsible process. Holding Full Disk Access supersedes that
/// check, so the dialog never fires.
#[tauri::command]
#[specta::specta]
pub fn full_disk_access_status() -> FullDiskAccess {
    hyprlayer_core::system::full_disk_access()
}

/// Open the Full Disk Access pane. The grant can only be added by hand, and it
/// takes effect only after the app restarts.
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

/// Confirmed quit. The menu/window handlers only ever *ask* (see
/// `crate::request_quit`), so this is the one path that actually exits once the
/// prompt is up. `dont_ask_again` persists before the exit, so the choice can't
/// be lost to the process going away mid-write.
#[tauri::command]
#[specta::specta]
pub async fn quit_app(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dont_ask_again: bool,
) -> CmdResult<()> {
    if dont_ask_again {
        state.settings.update(|s| s.quit.confirm = false).await?;
    }
    app.exit(0);
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

/// Fold newly-offered Claude models into a CURATED picker list, and record what
/// the catalog offered this time.
///
/// Without this, a user who ever curated Settings → Models never sees a model
/// Anthropic ships later — their stored list is an allowlist frozen at the day
/// they edited it, and the only cure is "Reset to default", which also undoes
/// their removals. `add` is strictly additive and `seen` is the watermark that
/// keeps it additive: a model the user deliberately removed is in `seen`, so it
/// is never re-added, while a model that simply didn't exist then is not.
///
/// Deliberately a narrow command rather than a `set_app_settings` round-trip:
/// that one replaces the whole object, so a second app instance writing
/// concurrently would lose the other's edits (two dev builds share one
/// `settings.json`). This mutates the two fields under the store's lock.
#[tauri::command]
#[specta::specta]
pub async fn claude_sync_models(
    state: State<'_, AppState>,
    add: Vec<String>,
    seen: Vec<String>,
) -> CmdResult<AppSettings> {
    state
        .settings
        .update(|s| {
            let claude = &mut s.providers.claude;
            // `None` means "show everything" — already current, nothing to add.
            if let Some(visible) = claude.visible_models.as_mut() {
                for key in &add {
                    if !visible.contains(key) {
                        visible.push(key.clone());
                    }
                }
            }
            claude.seen_model_keys = Some(seen);
        })
        .await
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

/* ----------------------- hyprlayer CLI install ------------------------- */

/// A package manager that can install the hyprlayer CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CliInstallMethodId {
    Homebrew,
    Winget,
    Scoop,
}

/// One way to install the hyprlayer CLI: the command to run and whether its
/// package manager is present.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CliInstallMethod {
    pub id: CliInstallMethodId,
    /// Package manager name, e.g. "Homebrew".
    pub label: String,
    /// Install command, run through the login shell.
    pub command: String,
    pub available: bool,
}

/// A package-manager install option for this OS, before availability probing.
struct CliCandidate {
    id: CliInstallMethodId,
    label: &'static str,
    command: &'static str,
    /// Binary that must resolve for the manager to count as installed.
    probe: &'static str,
}

fn cli_install_candidates() -> &'static [CliCandidate] {
    #[cfg(target_os = "macos")]
    let candidates: &[CliCandidate] = &[CliCandidate {
        id: CliInstallMethodId::Homebrew,
        label: "Homebrew",
        command: "brew install brightblock/tap/hyprlayer",
        probe: "brew",
    }];
    #[cfg(target_os = "windows")]
    let candidates: &[CliCandidate] = &[
        CliCandidate {
            id: CliInstallMethodId::Winget,
            label: "winget",
            // --force: winget correlates the desktop app's ARP entry ("Hyprlayer" /
            // BrightBlock) with this package and converts the install into an upgrade,
            // which fails on portable-vs-NSIS (#8894). Force skips that correlation.
            command: "winget install BrightBlock.Hyprlayer --force --accept-source-agreements --accept-package-agreements",
            probe: "winget",
        },
        CliCandidate {
            id: CliInstallMethodId::Scoop,
            label: "Scoop",
            command: "scoop install hyprlayer",
            probe: "scoop",
        },
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let candidates: &[CliCandidate] = &[];
    candidates
}

/// hyprlayer CLI install options for this OS (macOS → Homebrew; Windows →
/// winget + Scoop), each flagged by whether its package manager is installed.
#[tauri::command]
#[specta::specta]
pub async fn detect_cli_install_methods(
    state: State<'_, AppState>,
) -> CmdResult<Vec<CliInstallMethod>> {
    let path = state.binaries.augmented_path().await;
    let shell = super::terminal::default_shell();

    let mut methods = Vec::new();
    for c in cli_install_candidates() {
        methods.push(CliInstallMethod {
            id: c.id,
            label: c.label.to_string(),
            command: c.command.to_string(),
            available: package_manager_available(&shell, &path, c.probe).await,
        });
    }
    Ok(methods)
}

/// Install the hyprlayer CLI via the chosen package manager, in the background
/// (no terminal). Re-detects on success so the UI flips to "installed".
#[tauri::command]
#[specta::specta]
pub async fn install_hyprlayer_cli(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    method: CliInstallMethodId,
) -> CmdResult<()> {
    let command = cli_install_candidates()
        .iter()
        .find(|c| c.id == method)
        .map(|c| c.command)
        .ok_or_else(|| AppError {
            code: "cli_install_unsupported".into(),
            message: "That install method isn't available on this platform.".into(),
            remedy: None,
        })?;

    let path = state.binaries.augmented_path().await;
    let shell = super::terminal::default_shell();

    Exec::new(&shell)
        .args(super::terminal::login_shell_args(command))
        .env("PATH", path)
        .timeout(Duration::from_secs(300))
        .run()
        .await
        .map_err(AppError::from)?;

    state.binaries.redetect().await;
    let _ = BinariesChanged.emit(&app);
    Ok(())
}

/// True when `manager --version` exits cleanly through the login shell.
async fn package_manager_available(shell: &Path, path: &std::ffi::OsStr, manager: &str) -> bool {
    Exec::new(shell)
        .args(super::terminal::login_shell_args(&format!("{manager} --version")))
        .env("PATH", path)
        .timeout(Duration::from_secs(15))
        .run()
        .await
        .is_ok()
}
