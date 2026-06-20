//! Onboarding-specific IPC commands: AI status and thoughts-repo setup.
//!
//! These commands are used exclusively by the `/onboarding` wizard and the AI
//! reconfigure dialog in Settings. They do not touch the project registry —
//! that remains in `projects.rs`.

use std::path::PathBuf;

use hyprlayer_core::config;
use hyprlayer_core::hyprcli::{AiStatus, HyprCli};
use hyprlayer_core::repos;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_specta::Event;

use super::projects::{config_summary, registry_projects, Bootstrap};
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// How the user wants to set up their thoughts repository.
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum ThoughtsSource {
    /// Clone a remote git repository (e.g. a private GitHub thoughts repo).
    /// `location` is where the clone is placed.
    /// When None, defaults to `~/thoughts`.
    Clone {
        url: String,
        /// Absolute path where the cloned repo will be placed.
        /// When None, defaults to `~/thoughts`.
        location: Option<String>,
    },
    /// Use an existing local git repository.
    Disk {
        /// Absolute path to the existing git repository.
        path: String,
    },
    /// Initialise a fresh git repository at the given path.
    Create {
        /// Absolute path where the new repo will be created.
        path: String,
    },
}

/// Read the AI tool install state without running `ai configure`.
/// Gates the AI step: if already installed, the step shows "configured" with
/// a "Reconfigure" option; if not, it prompts the user to run `ai configure`
/// in the embedded terminal.
#[tauri::command]
#[specta::specta]
pub async fn get_ai_status(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    HyprCli::new(&state.binaries)
        .ai_status(&cwd)
        .await
        .map_err(AppError::from)
}

/// Probe of the default `~/thoughts` location, so the wizard can pick a sensible
/// initial mode: if it's already a git repo, default to "open from disk" and
/// prefill the path; otherwise default to "create" at the same path.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsProbe {
    /// Absolute `~/thoughts` path (tilde expanded), for prefilling the field.
    pub default_path: String,
    /// Whether that path exists on disk.
    pub exists: bool,
    /// Whether it's already a git repository.
    pub is_git_repo: bool,
}

/// Inspect `~/thoughts` so the thoughts step can auto-select disk-vs-create.
/// Infallible: a missing home dir falls back to a relative `thoughts`.
#[tauri::command]
#[specta::specta]
pub fn probe_default_thoughts() -> ThoughtsProbe {
    let path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("thoughts");
    ThoughtsProbe {
        exists: path.exists(),
        is_git_repo: repos::is_git_repo(&path),
        default_path: path.to_string_lossy().into_owned(),
    }
}

/// Check out or initialise the user's thoughts repository and write the
/// `thoughts` section into `config.json`, then return the updated bootstrap.
///
/// This is the "Confirm" action in the thoughts-repo step of the wizard. The
/// write is merge-tolerant: if `ai configure` has already created an ai-only
/// config, the `ai` section is preserved and `thoughts` is injected.
///
/// On success the config watcher will eventually emit `ConfigChanged`, but we
/// also invalidate and emit immediately so the wizard's "Step 3 complete" state
/// doesn't wait for the 2-second poll cycle.
#[tauri::command]
#[specta::specta]
pub async fn init_thoughts_repo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    user: String,
    source: ThoughtsSource,
) -> CmdResult<Bootstrap> {
    // 1. Resolve the thoughts-repo path on disk. All user-supplied paths run
    //    through `expand_path` so a leading `~` becomes the real home dir —
    //    otherwise the filesystem would create a literal `~` directory.
    let resolved: PathBuf = match &source {
        ThoughtsSource::Clone { url, location } => {
            // The clone lands AT this path (not under it), so the repo is always
            // `~/thoughts` regardless of the source repo's own name.
            let dest = match location.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
                Some(loc) => config::expand_path(loc),
                None => dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("thoughts"),
            };
            let git = state
                .binaries
                .require(hyprlayer_core::process::binaries::BinaryKind::Git)
                .await
                .map_err(AppError::from)?;
            repos::clone_repo(&git, url, &dest)
                .await
                .map_err(AppError::from)?;
            dest
        }
        ThoughtsSource::Disk { path } => {
            let p = config::expand_path(path);
            if !repos::is_git_repo(&p) {
                return Err(AppError {
                    code: "not_a_git_repo".into(),
                    message: format!("{} is not a git repository", p.display()),
                    remedy: Some(
                        "Choose a directory that contains a `.git` folder, or use \
                         \"Create new\" to initialise one."
                            .into(),
                    ),
                });
            }
            p
        }
        ThoughtsSource::Create { path } => {
            let p = config::expand_path(path);
            let git = state
                .binaries
                .require(hyprlayer_core::process::binaries::BinaryKind::Git)
                .await
                .map_err(AppError::from)?;
            repos::init_repo(&git, &p).await.map_err(AppError::from)?;
            p
        }
    };

    // 2. Write / merge the `thoughts` section into config.json.
    let thoughts_repo_str = resolved.to_string_lossy().into_owned();
    config::ensure_thoughts_config(state.config.path(), user.trim(), &thoughts_repo_str).map_err(
        |e| {
            // A clean fact + action split — the raw error already says "use
            // Settings or the CLI", so don't repeat it in both fields.
            if e.to_string().contains("already configured") {
                AppError {
                    code: "thoughts_already_configured".into(),
                    message: "Your thoughts repository is already configured.".into(),
                    remedy: Some("Use Settings or the hyprlayer CLI to change it.".into()),
                }
            } else {
                AppError::from(e)
            }
        },
    )?;

    // 3. Invalidate, emit, and rebuild so the rest of the app sees the new config.
    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(&app);
    crate::rebuild_thoughts_index(app).await;

    // 4. Return the updated bootstrap so the wizard can confirm ready state.
    Ok(Bootstrap {
        config: config_summary(&state).await,
        projects: registry_projects(&state).await,
        binaries: state.binaries.status_all().await,
        settings: state.settings.get().await,
    })
}
