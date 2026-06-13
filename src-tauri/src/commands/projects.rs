use std::path::Path;

use hyprlayer_core::config::HyprlayerConfig;
use hyprlayer_core::hyprcli::{HyprCli, StorageInfo};
use hyprlayer_core::process::binaries::{BinaryKind, BinaryState, BinaryStatus};
use hyprlayer_core::worktree::thoughts_links;
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;
use tauri_specta::Event;

use super::db_task;
use crate::error::{AppError, CmdResult};
use crate::settings::AppSettings;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Absolute path of the code repository (the repoMappings key).
    pub path: String,
    pub mapped_name: String,
    pub profile: Option<String>,
    pub backend_kind: String,
    pub exists_on_disk: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum ConfigSummary {
    /// No config file: first-run — onboarding should offer bootstrap.
    Missing { path: String },
    /// Config exists but can't be used (parse error, future schema, ...).
    Error {
        path: String,
        code: String,
        message: String,
        remedy: Option<String>,
    },
    Ready {
        path: String,
        user: String,
        backend_kind: String,
        configured: bool,
    },
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub config: ConfigSummary,
    pub projects: Vec<Project>,
    pub binaries: Vec<BinaryStatus>,
    pub settings: AppSettings,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: Project,
    pub storage: StorageInfo,
    pub thoughts_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub checks: Vec<HealthCheck>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub output: String,
}

fn projects_from_config(config: &HyprlayerConfig) -> Vec<Project> {
    let Some(thoughts) = &config.thoughts else {
        return Vec::new();
    };
    let mut projects: Vec<Project> = thoughts
        .repo_mappings
        .iter()
        .map(|(path, mapping)| {
            let effective = thoughts.effective_config_for(path);
            Project {
                path: path.clone(),
                mapped_name: mapping.repo().to_string(),
                profile: effective.profile_name.clone(),
                backend_kind: effective.backend.kind().as_str().to_string(),
                exists_on_disk: Path::new(path).is_dir(),
            }
        })
        .collect();
    projects.sort_by(|a, b| a.mapped_name.cmp(&b.mapped_name));
    projects
}

/// Mapping entries the app itself added for work-item worktrees (ADR-0002
/// exception #2) are skill-resolution infrastructure, not projects — drop
/// them from every project listing. Two nets: worktree paths of every
/// workstream row (archived rows included, so a mapping whose uninit failed
/// stays filtered), plus anything under the worktree root (catches strays
/// with no row at all). A store error degrades to the prefix net, loudly.
async fn filter_worktree_mappings(state: &AppState, projects: &mut Vec<Project>) {
    let worktree_root = state.worktrees.root().to_path_buf();
    let known: std::collections::HashSet<String> =
        match db_task(state.db.clone(), |db| db.list_worktree_paths()).await {
            Ok(paths) => paths.into_iter().collect(),
            Err(e) => {
                log::warn!("could not load workstream worktree paths: {}", e.message);
                Default::default()
            }
        };

    projects
        .retain(|p| !known.contains(&p.path) && !Path::new(&p.path).starts_with(&worktree_root));
}

async fn config_summary(state: &AppState) -> (ConfigSummary, Vec<Project>) {
    let path = state.config.path().display().to_string();
    match state.config.load().await {
        Ok(loaded) => {
            let mut projects = projects_from_config(&loaded.config);
            filter_worktree_mappings(state, &mut projects).await;
            let (user, backend_kind, configured) = loaded
                .config
                .thoughts
                .as_ref()
                .map(|t| {
                    (
                        t.user.clone(),
                        t.backend.kind().as_str().to_string(),
                        t.is_configured(),
                    )
                })
                .unwrap_or_else(|| (String::new(), "git".into(), false));
            (
                ConfigSummary::Ready {
                    path,
                    user,
                    backend_kind,
                    configured,
                },
                projects,
            )
        }
        Err(CoreError::ConfigMissing { .. }) => (ConfigSummary::Missing { path }, Vec::new()),
        Err(err) => {
            let app_err = AppError::from(err);
            (
                ConfigSummary::Error {
                    path,
                    code: app_err.code,
                    message: app_err.message,
                    remedy: app_err.remedy,
                },
                Vec::new(),
            )
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_bootstrap(state: State<'_, AppState>) -> CmdResult<Bootstrap> {
    let (config, projects) = config_summary(&state).await;
    Ok(Bootstrap {
        config,
        projects,
        binaries: state.binaries.status_all().await,
        settings: state.settings.get().await,
    })
}

fn find_project(projects: &[Project], path: &str) -> CmdResult<Project> {
    projects
        .iter()
        .find(|p| p.path == path)
        .cloned()
        .ok_or_else(|| AppError::project_not_found(path))
}

#[tauri::command]
#[specta::specta]
pub async fn get_project(state: State<'_, AppState>, path: String) -> CmdResult<ProjectDetail> {
    let (_, projects) = config_summary(&state).await;
    let project = find_project(&projects, &path)?;

    let storage = HyprCli::new(&state.binaries)
        .storage_info(Path::new(&path))
        .await?;

    let thoughts_root = effective_thoughts_root(&state, &path).await;

    Ok(ProjectDetail {
        project,
        storage,
        thoughts_root,
    })
}

async fn effective_thoughts_root(state: &AppState, repo_path: &str) -> Option<String> {
    let loaded = state.config.load().await.ok()?;
    let thoughts = loaded.config.thoughts.as_ref()?;
    let effective = thoughts.effective_config_for(repo_path);
    effective
        .backend
        .thoughts_root()
        .map(|p| p.display().to_string())
}

/// Add a repository as a hyprlayer project. Wraps `hyprlayer thoughts init
/// --yes`; on a true first run (no config anywhere) it first writes the
/// minimal bootstrap config — the documented ADR-0002 exception — and the CLI
/// owns the file from then on.
#[tauri::command]
#[specta::specta]
pub async fn add_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    directory_name: Option<String>,
    bootstrap_user: Option<String>,
) -> CmdResult<Project> {
    let repo = Path::new(&path);
    if !repo.is_dir() {
        return Err(AppError {
            code: "invalid_path".into(),
            message: format!("{path} is not a directory"),
            remedy: None,
        });
    }
    if !repo.join(".git").exists() {
        return Err(AppError {
            code: "not_a_git_repo".into(),
            message: format!("{path} is not a git repository"),
            remedy: Some("hyprlayer projects must be git repositories (`git init` first).".into()),
        });
    }

    if !state.config.exists() {
        let user = bootstrap_user.unwrap_or_default();
        if user.trim().is_empty() {
            return Err(AppError {
                code: "bootstrap_user_required".into(),
                message: "No hyprlayer config exists yet; a username is needed to create one."
                    .into(),
                remedy: Some(
                    "Enter the username your thought documents should be attributed to.".into(),
                ),
            });
        }
        hyprlayer_core::config::write_bootstrap_config(state.config.path(), &user)
            .map_err(AppError::from)?;
    }

    let dir_name = directory_name
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| {
            repo.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unnamed_repo")
                .to_string()
        });

    HyprCli::new(&state.binaries)
        .thoughts_init(repo, &dir_name, None)
        .await
        .map_err(AppError::from)?;

    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(&app);
    crate::rebuild_thoughts_index(app.clone()).await;

    let (_, projects) = config_summary(&state).await;
    find_project(&projects, &path)
}

#[tauri::command]
#[specta::specta]
pub async fn sync_thoughts(
    state: State<'_, AppState>,
    path: String,
    message: Option<String>,
) -> CmdResult<SyncReport> {
    let output = HyprCli::new(&state.binaries)
        .thoughts_sync(Path::new(&path), message.as_deref())
        .await?;
    // The CLI may have rewritten its config (e.g. orphan cleanup).
    state.config.invalidate().await;
    Ok(SyncReport { output })
}

#[tauri::command]
#[specta::specta]
pub async fn run_backend_health(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<HealthReport> {
    let mut checks = Vec::new();

    // 1. Required binaries — one concurrent probe pass for all four kinds,
    // not four serialized probes (each can hit its full timeout cold).
    let statuses = state.binaries.status_all().await;
    let status_of = |kind: BinaryKind| {
        statuses
            .iter()
            .find(|s| s.kind == kind)
            .cloned()
            .expect("status_all covers every binary kind")
    };
    for kind in [BinaryKind::Hyprlayer, BinaryKind::Git, BinaryKind::Claude] {
        let status = status_of(kind);
        let (check_status, detail, remedy) = match &status.state {
            BinaryState::Ok => (
                CheckStatus::Pass,
                status
                    .version
                    .as_ref()
                    .map(|v| format!("{v} at {}", status.path.as_deref().unwrap_or("?"))),
                None,
            ),
            BinaryState::Missing => (
                CheckStatus::Fail,
                None,
                AppError::from(CoreError::BinaryMissing {
                    name: status.name.clone(),
                })
                .remedy,
            ),
            BinaryState::TooOld { min } => (
                CheckStatus::Fail,
                status.version.clone(),
                Some(format!("Update {} to {min} or newer.", status.name)),
            ),
        };
        checks.push(HealthCheck {
            id: format!("binary.{}", status.name),
            label: format!("{} CLI", status.name),
            status: check_status,
            detail,
            remedy,
        });
    }

    // gh is optional: warn, never fail.
    let gh = status_of(BinaryKind::Gh);
    checks.push(HealthCheck {
        id: "binary.gh".into(),
        label: "GitHub CLI (optional)".into(),
        status: if matches!(gh.state, BinaryState::Ok) {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        },
        detail: gh.version.clone(),
        remedy: if matches!(gh.state, BinaryState::Ok) {
            None
        } else {
            Some("Without gh, PR creation degrades to opening a browser link.".into())
        },
    });

    // 2. Project mapping resolves via the CLI.
    match HyprCli::new(&state.binaries)
        .storage_info(Path::new(&path))
        .await
    {
        Ok(info) => {
            checks.push(HealthCheck {
                id: "project.mapped".into(),
                label: "Project mapping".into(),
                status: if info.mapped_name.is_some() {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                detail: info.mapped_name.clone(),
                remedy: if info.mapped_name.is_some() {
                    None
                } else {
                    Some("Run `hyprlayer thoughts init` in this repository.".into())
                },
            });

            checks.push(HealthCheck {
                id: "backend.kind".into(),
                label: "Storage backend".into(),
                status: CheckStatus::Pass,
                detail: Some(info.backend.clone()),
                remedy: None,
            });
        }
        Err(err) => {
            let app_err = AppError::from(err);
            checks.push(HealthCheck {
                id: "project.mapped".into(),
                label: "Project mapping".into(),
                status: CheckStatus::Fail,
                detail: Some(app_err.message),
                remedy: app_err.remedy,
            });
        }
    }

    // 3. Thoughts root exists (filesystem backends only).
    if let Ok(loaded) = state.config.load().await {
        if let Some(thoughts) = loaded.config.thoughts.as_ref() {
            let effective = thoughts.effective_config_for(&path);
            if effective.backend.kind().uses_filesystem() {
                if let Some(root) = effective.backend.thoughts_root() {
                    let exists = root.is_dir();
                    checks.push(HealthCheck {
                        id: "thoughts.root".into(),
                        label: "Thoughts storage root".into(),
                        status: if exists {
                            CheckStatus::Pass
                        } else {
                            CheckStatus::Fail
                        },
                        detail: Some(root.display().to_string()),
                        remedy: if exists {
                            None
                        } else {
                            Some("The configured thoughts directory does not exist. Re-run `hyprlayer thoughts init` or fix the path in the hyprlayer config.".into())
                        },
                    });
                }

                // 4. Per-repo thoughts symlinks — same definition of link
                // health as the per-workstream status chips.
                let repo_thoughts = Path::new(&path).join(hyprlayer_core::worktree::THOUGHTS_DIR);
                let missing = thoughts_links::missing_thoughts_links(Path::new(&path), &effective);
                checks.push(HealthCheck {
                    id: "thoughts.symlinks".into(),
                    label: "Repository thoughts/ links".into(),
                    status: if missing.is_empty() {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    },
                    detail: if missing.is_empty() {
                        Some(repo_thoughts.display().to_string())
                    } else {
                        Some(format!("missing: thoughts/{}", missing.join(", thoughts/")))
                    },
                    remedy: if missing.is_empty() {
                        None
                    } else {
                        Some("Run `hyprlayer thoughts init` in the repository to recreate the links.".into())
                    },
                });
            }
        }
    }

    Ok(HealthReport { checks })
}
