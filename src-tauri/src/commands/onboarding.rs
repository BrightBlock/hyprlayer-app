use std::path::{Path, PathBuf};

use hyprlayer_core::config;
use hyprlayer_core::config::{DEFAULT_GLOBAL_DIR, DEFAULT_REPOS_DIR};
use hyprlayer_core::hyprcli::{AiStatus, HyprCli};
use hyprlayer_core::repos;
use hyprlayer_core::thoughts_repo;
use hyprlayer_core::worktree::thoughts_links;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_specta::Event;

use super::projects::{config_summary, registry_projects, Bootstrap};
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum ThoughtsSource {
    Clone {
        url: String,
        location: Option<String>,
    },
    Disk {
        path: String,
    },
    Create {
        path: String,
    },
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_status(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    HyprCli::new(&state.binaries)
        .ai_status(&cwd)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsProbe {
    pub default_path: String,
    pub exists: bool,
    pub is_git_repo: bool,
}

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

#[tauri::command]
#[specta::specta]
pub async fn init_thoughts_repo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    user: String,
    source: ThoughtsSource,
) -> CmdResult<Bootstrap> {
    let resolved: PathBuf = match &source {
        ThoughtsSource::Clone { url, location } => {
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
            thoughts_repo::scaffold(&git, &p, user.trim(), DEFAULT_REPOS_DIR, DEFAULT_GLOBAL_DIR)
                .await
                .map_err(AppError::from)?;
            p
        }
    };

    let thoughts_repo_str = resolved.to_string_lossy().into_owned();
    config::ensure_thoughts_config(state.config.path(), user.trim(), &thoughts_repo_str)
        .map_err(AppError::from)?;

    state.config.invalidate().await;

    relink_mapped_checkouts(&state).await;

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

    let _ = crate::events::ConfigChanged.emit(&app);
    crate::rebuild_thoughts_index(app).await;

    Ok(Bootstrap {
        config: config_summary(&state).await,
        projects: registry_projects(&state).await,
        binaries: state.binaries.status_all().await,
        settings: state.settings.get().await,
        prompt_defaults: Default::default(),
    })
}

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
            continue;
        }
        if let Err(init_err) = cli
            .thoughts_init(path, mapped, effective.profile_name.as_deref())
            .await
        {
            log::warn!("thoughts re-init at {path_str} failed ({init_err}); relinking manually");
        }
        if !thoughts_links::broken_thoughts_links(path, &effective).is_empty() {
            if let Err(e) = thoughts_links::provision_thoughts(path, &effective) {
                log::warn!("manual relink at {path_str} failed: {e}");
            }
        }
    }
}
