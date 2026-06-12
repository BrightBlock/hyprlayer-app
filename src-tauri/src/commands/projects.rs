use std::path::Path;

use hyprlayer_core::config::HyprlayerConfig;
use hyprlayer_core::hyprcli::{HyprCli, StorageInfo};
use hyprlayer_core::process::binaries::{BinaryKind, BinaryState, BinaryStatus};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;

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

async fn config_summary(state: &AppState) -> (ConfigSummary, Vec<Project>) {
    let path = state.config.path().display().to_string();
    match state.config.load().await {
        Ok(loaded) => {
            let projects = projects_from_config(&loaded.config);
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

#[tauri::command]
#[specta::specta]
pub async fn list_projects(state: State<'_, AppState>) -> CmdResult<Vec<Project>> {
    let (_, projects) = config_summary(&state).await;
    Ok(projects)
}

fn find_project(projects: &[Project], path: &str) -> CmdResult<Project> {
    projects
        .iter()
        .find(|p| p.path == path)
        .cloned()
        .ok_or_else(|| AppError {
            code: "project_not_found".into(),
            message: format!("No hyprlayer project is mapped at {path}"),
            remedy: Some("Add the repository as a project first.".into()),
        })
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

    // 1. Required binaries.
    for kind in [BinaryKind::Hyprlayer, BinaryKind::Git, BinaryKind::Claude] {
        let status = state.binaries.status(kind).await;
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
    let gh = state.binaries.status(BinaryKind::Gh).await;
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

                // 4. Per-repo thoughts symlinks.
                let repo_thoughts = Path::new(&path).join("thoughts");
                let mut expected: Vec<String> = vec!["shared".into(), "global".into()];
                if !effective.user.is_empty() {
                    expected.insert(0, effective.user.clone());
                }
                let missing: Vec<String> = expected
                    .iter()
                    .filter(|name| !repo_thoughts.join(name.as_str()).exists())
                    .cloned()
                    .collect();
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
