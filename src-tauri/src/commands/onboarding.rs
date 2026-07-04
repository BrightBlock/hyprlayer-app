//! Onboarding-specific IPC commands: AI status and thoughts-repo setup.
//!
//! These commands are used exclusively by the `/onboarding` wizard and the AI
//! reconfigure dialog in Settings. They do not touch the project registry —
//! that remains in `projects.rs`.

use std::path::{Path, PathBuf};

use hyprlayer_core::config;
use hyprlayer_core::hyprcli::{AiStatus, HyprCli};
use hyprlayer_core::repos;
use hyprlayer_core::worktree::thoughts_links;
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
/// config, the `ai` section is preserved and `thoughts` is injected; if a
/// `thoughts` section already exists (half-written or from a previous run),
/// it is repointed at the selected repo rather than refused.
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
    config::ensure_thoughts_config(state.config.path(), user.trim(), &thoughts_repo_str)
        .map_err(AppError::from)?;

    // 3. Drop the cached config so the relink sweep resolves the new root.
    state.config.invalidate().await;

    // 4. Existing `<checkout>/thoughts/` links are absolute paths into the OLD
    //    root — a repoint alone would leave every mapped repo and worktree
    //    silently writing there. Re-init each live one against the new config.
    relink_mapped_checkouts(&state).await;

    // 5. A CLI invocation racing the steps above (e.g. a workstream created
    //    mid-submit runs `thoughts init`, a stale read-modify-write of
    //    config.json) can clobber the repoint — observed live. Verify the
    //    write stuck; re-merge into the latest file (preserving mappings the
    //    racer added) and re-sweep, once.
    state.config.invalidate().await;
    let stuck = state.config.load().await.is_ok_and(|loaded| {
        loaded
            .config
            .thoughts
            .as_ref()
            .and_then(|t| t.backend.thoughts_root())
            .is_some_and(|root| root == resolved)
    });
    if !stuck {
        log::warn!("thoughts repoint was clobbered by a concurrent config writer; re-applying");
        config::ensure_thoughts_config(state.config.path(), user.trim(), &thoughts_repo_str)
            .map_err(AppError::from)?;
        state.config.invalidate().await;
        relink_mapped_checkouts(&state).await;
        state.config.invalidate().await;
    }

    // 6. Emit and rebuild so the rest of the app sees the new config.
    let _ = crate::events::ConfigChanged.emit(&app);
    crate::rebuild_thoughts_index(app).await;

    // 7. Return the updated bootstrap so the wizard can confirm ready state.
    Ok(Bootstrap {
        config: config_summary(&state).await,
        projects: registry_projects(&state).await,
        binaries: state.binaries.status_all().await,
        settings: state.settings.get().await,
        prompt_defaults: Default::default(),
    })
}

/// Re-link `thoughts/` for every mapped checkout that still exists on disk, so
/// a repoint doesn't leave stale links writing into the old thoughts repo.
///
/// CLI-first per path — `thoughts init --yes` owns the target tree, links, and
/// mapping (the same pattern as workstream creation) — with manual relinking
/// as the fallback. Serial on purpose: every CLI run rewrites config.json, so
/// concurrent inits would race the tmp+rename. Profile-pinned mappings resolve
/// their own backend via `effective_config_for` and short-circuit as healthy;
/// paths that are gone are skipped (relinking would resurrect them — pruning
/// belongs to the CLI's orphan cleanup). Best-effort: failures are logged,
/// never surfaced — one repo on a detached drive must not fail the wizard.
async fn relink_mapped_checkouts(state: &AppState) {
    let Ok(loaded) = state.config.load().await else {
        return;
    };
    let Some(thoughts) = loaded.config.thoughts.as_ref() else {
        return;
    };
    let cli = HyprCli::new(&state.binaries);
    for path_str in thoughts.repo_mappings.keys() {
        let path = Path::new(path_str);
        if !path.is_dir() {
            continue;
        }
        let effective = thoughts.effective_config_for(path_str);
        if !effective.backend.kind().uses_filesystem() {
            continue;
        }
        let Some(mapped) = effective.mapped_name.as_deref() else {
            continue;
        };
        if thoughts_links::broken_thoughts_links(path, &effective).is_empty() {
            continue; // already points at the current root
        }
        if let Err(init_err) = cli
            .thoughts_init(path, mapped, effective.profile_name.as_deref())
            .await
        {
            log::warn!("thoughts re-init at {path_str} failed ({init_err}); relinking manually");
        }
        // `thoughts init` creates the target tree and any missing links, but
        // leaves resolving-but-stale links in place (verified against CLI
        // v1.5.8) — exactly the state a repoint produces. Force-replace
        // whatever it left broken; provisioning is idempotent over its links.
        if !thoughts_links::broken_thoughts_links(path, &effective).is_empty() {
            if let Err(e) = thoughts_links::provision_thoughts(path, &effective) {
                log::warn!("manual relink at {path_str} failed: {e}");
            }
        }
    }
}
