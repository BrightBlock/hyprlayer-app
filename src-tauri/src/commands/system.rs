use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use hyprlayer_core::branch_prefix;
use hyprlayer_core::hyprcli::{CliUpdateCheck, HyprCli};
use hyprlayer_core::omarchy;
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
/// other apps" dialog, which names Hyprlayer, macOS attributes a child's
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

/// Whether Omarchy is installed, and its active theme resolved into
/// hyprlayer's token vocabulary — see [`omarchy::status`] for why those are two
/// answers and not one.
///
/// Registered on every platform and gated inside `omarchy::status` by a path
/// probe, the same shape as `play_sound` (:63-74) and
/// `open_full_disk_access_settings` (:94-101). No `#[cfg]` is needed:
/// `omarchy::current_dir()` cannot resolve off Linux, because
/// `dirs::state_dir()` returns None on macOS and Windows, so this reports
/// `installed: false` there.
///
/// Infallible by design, so no `CmdResult` — a parse failure is logged in core
/// and collapses to `theme: None` rather than raising an `AppError`.
#[tauri::command]
#[specta::specta]
pub fn get_omarchy_theme() -> omarchy::OmarchyStatus {
    omarchy::status()
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

/// Whether this write actually changes the branch prefix, comparing what the
/// user set rather than how it was spelled.
///
/// Both sides are trimmed, and that asymmetry is the whole point: the incoming
/// value has just been normalized while the stored one may predate any
/// normalization at all (nothing validated or trimmed this field before the
/// branch-type picker landed). Comparing normalized against raw would read a stored
/// `" my feature/"` as changed on every unrelated write — the whitespace alone
/// differs — so validation would fire and block a prompt pin or a folder
/// collapse, which is exactly the outcome scoping the validation exists to
/// prevent.
fn branch_prefix_changed(incoming: Option<&str>, current: Option<&str>) -> bool {
    incoming.map(str::trim) != current.map(str::trim)
}

#[tauri::command]
#[specta::specta]
pub async fn set_app_settings(
    state: State<'_, AppState>,
    mut settings: AppSettings,
) -> CmdResult<AppSettings> {
    // Normalize before comparing, so what gets validated is exactly what gets
    // stored. `validate_prefix` checks the TRIMMED content (a pasted
    // `" feature/"` is a legal prefix with incidental whitespace), so storing
    // the raw string would let a value pass validation and then fail later at
    // `git worktree add`. The list gets the same treatment as
    // `set_branch_prefixes` gives the per-repo one — same editor, same bytes.
    settings.git.branch_prefix = settings.git.branch_prefix.map(|p| p.trim().to_string());
    settings.git.prefixes = settings.git.prefixes.map(super::normalize_branch_prefixes);

    // Validate only what this write actually CHANGES. `set_app_settings` is a
    // whole-object replace (`src/lib/queries.ts:415-451`) called from twelve
    // unrelated surfaces — an agent-folder collapse (`AgentLibrary.tsx:111`), a
    // prompt pin (`PromptLibrary.tsx:151`), a keybinding
    // (`KeyboardSettings.tsx:180`), each harness tab, the full-disk-access
    // dismissal (`FullDiskAccessNotice.tsx:24`) — and every one of them resends
    // `git.branchPrefix` untouched. Validating it unconditionally would mean a
    // user with an already-bad stored prefix could no longer collapse a folder
    // or pin a prompt, and `useSetAppSettings`'s optimistic write + rollback
    // (`queries.ts:424-439`) would show those changes land and silently revert.
    let current = state.settings.get().await;
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

    state.settings.set(settings).await?;
    Ok(state.settings.get().await)
}

/// Fold newly-offered Claude models into a CURATED picker list, and record what
/// the catalog offered this time.
///
/// Without this, a user who ever curated Settings → Models never sees a model
/// Anthropic ships later, their stored list is an allowlist frozen at the day
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
            // `None` means "show everything", already current, nothing to add.
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
    // Homebrew on macOS, and on Linux too: Homebrew-on-Linux installs the same
    // tap with the same command. `available` gates it on `brew` actually being
    // on PATH, so a Linux box without it still sees the manual instructions
    // rather than a button that can't work, and without this the Linux
    // onboarding step listed nothing at all, dead-ending on "install one of the
    // options below" with no options below it.
    #[cfg(not(target_os = "windows"))]
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
    candidates
}

/// hyprlayer CLI install options for this OS (macOS + Linux → Homebrew;
/// Windows → winget + Scoop), each flagged by whether its package manager is
/// installed.
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

/// Whether a newer `hyprlayer` is installable, via the CLI's own
/// `self-update --check`.
///
/// Every field `None` means no answer, not "up to date": the check exits
/// non-zero when its install method's release source is unreachable. The
/// caller shows nothing in that case.
#[tauri::command]
#[specta::specta]
pub async fn hyprlayer_cli_update_check(state: State<'_, AppState>) -> CmdResult<CliUpdateCheck> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    HyprCli::new(&state.binaries)
        .self_update_check(&cwd)
        .await
        .map_err(AppError::from)
}

/// What one `hyprlayer self-update` run did.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CliUpdateResult {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Run `hyprlayer self-update`, then re-probe the binaries.
///
/// A non-zero exit is data, not an `Err`: the CLI dispatches to the user's
/// package manager, and one that wants to prompt gets `/dev/null` on stdin and
/// dies — so the only useful thing left is the manager's own words next to the
/// CLI's upgrade hint. `Err` stays for never having run it at all.
///
/// Re-detects and emits on every path including failure: a partially applied
/// package-manager upgrade still moves the binary.
#[tauri::command]
#[specta::specta]
pub async fn update_hyprlayer_cli(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<CliUpdateResult> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let output = HyprCli::new(&state.binaries)
        .self_update(&cwd)
        .await
        .map_err(AppError::from)?;

    state.binaries.redetect().await;
    let _ = BinariesChanged.emit(&app);

    Ok(CliUpdateResult {
        ok: output.exit_code == Some(0),
        exit_code: output.exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// True when `manager --version` exits cleanly through the login shell.
async fn package_manager_available(shell: &Path, path: &std::ffi::OsStr, manager: &str) -> bool {
    Exec::new(shell)
        .args(super::terminal::login_shell_args(&format!(
            "{manager} --version"
        )))
        .env("PATH", path)
        .timeout(Duration::from_secs(15))
        .run()
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression guard for the scoped-validation property. `set_app_settings`
    /// is a whole-object replace called from twelve unrelated surfaces, every one
    /// of which resends `git.branchPrefix` untouched. A user whose stored prefix
    /// is already invalid (nothing validated this field before the branch-type picker)
    /// must still be able to pin a prompt or collapse an agent folder, so
    /// echoing a bad value back must not read as a change.
    #[test]
    fn resending_a_stored_invalid_prefix_is_not_a_change() {
        // Whitespace-padded AND invalid after trimming: the shape that would
        // have compared unequal if only the incoming side were normalized.
        let stored = Some(" my feature/");
        assert!(!branch_prefix_changed(stored, stored));
        // Which is what keeps validation from firing on an unrelated write.
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
