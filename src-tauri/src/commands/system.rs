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
    hyprlayer_core::system::full_disk_access()
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
    omarchy::status()
}

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
    crate::migrate_config_if_needed(&state.binaries, &state.config, &app).await;
    Ok(statuses)
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    Ok(state.settings.get().await)
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
    settings.git.prefixes = settings.git.prefixes.map(super::normalize_branch_prefixes);

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

#[tauri::command]
#[specta::specta]
pub async fn detect_editors() -> CmdResult<Vec<EditorTarget>> {
    tokio::task::spawn_blocking(editors::detect)
        .await
        .map_err(|e| AppError::internal(format!("editor detection failed: {e}")))
}

#[tauri::command]
#[specta::specta]
pub async fn github_username(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(None);
    };
    Ok(hyprlayer_core::pr::github_login(&gh).await)
}

#[tauri::command]
#[specta::specta]
pub fn default_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CliInstallMethodId {
    Homebrew,
    Winget,
    Scoop,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CliInstallMethod {
    pub id: CliInstallMethodId,
    pub label: String,
    pub command: String,
    pub available: bool,
}

struct CliCandidate {
    id: CliInstallMethodId,
    label: &'static str,
    command: &'static str,
    probe: &'static str,
}

fn cli_install_candidates() -> &'static [CliCandidate] {
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

#[tauri::command]
#[specta::specta]
pub async fn hyprlayer_cli_update_check(state: State<'_, AppState>) -> CmdResult<CliUpdateCheck> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    HyprCli::new(&state.binaries)
        .self_update_check(&cwd)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CliUpdateResult {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

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
