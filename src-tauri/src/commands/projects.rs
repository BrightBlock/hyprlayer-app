use std::path::{Path, PathBuf};

use hyprlayer_core::hyprcli::{HyprCli, StorageInfo};
use hyprlayer_core::process::binaries::{BinaryKind, BinaryState, BinaryStatus};
use hyprlayer_core::repos;
use hyprlayer_core::store::NewProject;
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
    /// Absolute path of the app-owned clone (`~/hyprlayer/repos/<name>`).
    pub path: String,
    pub mapped_name: String,
    pub profile: Option<String>,
    pub backend_kind: String,
    pub exists_on_disk: bool,
    /// The git source this clone tracks; None for a local-only repo.
    pub origin_url: Option<String>,
    /// `clone` | `disk` — how it was added.
    pub source: String,
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

/// `Pass` when the condition holds, else `Fail` — the common health verdict.
fn pass_if(ok: bool) -> CheckStatus {
    if ok {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    }
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

/// The project list: the app's explicit registry — NOT the thoughts config's
/// `repoMappings`, which also carry every worktree's `thoughts init` entry.
/// Each row is enriched with its backend/profile from the effective config,
/// best-effort (a missing/locked config just yields the git default).
async fn registry_projects(state: &AppState) -> Vec<Project> {
    let records = match db_task(state.db.clone(), |db| db.list_projects()).await {
        Ok(records) => records,
        Err(e) => {
            log::warn!("could not load project registry: {}", e.message);
            return Vec::new();
        }
    };
    let loaded = state.config.load().await.ok();
    let thoughts = loaded.as_ref().and_then(|l| l.config.thoughts.as_ref());
    records
        .into_iter()
        .map(|r| {
            let eff = thoughts.map(|t| t.effective_config_for(&r.path));
            let profile = eff.as_ref().and_then(|e| e.profile_name.clone());
            let backend_kind = eff
                .as_ref()
                .map(|e| e.backend.kind().as_str().to_string())
                .unwrap_or_else(|| "git".to_string());
            Project {
                exists_on_disk: Path::new(&r.path).is_dir(),
                path: r.path,
                mapped_name: r.name,
                profile,
                backend_kind,
                origin_url: r.origin_url,
                source: r.source,
            }
        })
        .collect()
}

/// The config health summary shown during onboarding. The project list is
/// derived separately now — see [`registry_projects`].
async fn config_summary(state: &AppState) -> ConfigSummary {
    let path = state.config.path().display().to_string();
    match state.config.load().await {
        Ok(loaded) => {
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
            ConfigSummary::Ready {
                path,
                user,
                backend_kind,
                configured,
            }
        }
        Err(CoreError::ConfigMissing { .. }) => ConfigSummary::Missing { path },
        Err(err) => {
            let app_err = AppError::from(err);
            ConfigSummary::Error {
                path,
                code: app_err.code,
                message: app_err.message,
                remedy: app_err.remedy,
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_bootstrap(state: State<'_, AppState>) -> CmdResult<Bootstrap> {
    Ok(Bootstrap {
        config: config_summary(&state).await,
        projects: registry_projects(&state).await,
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
    let project = find_project(&registry_projects(&state).await, &path)?;

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

/// Ensure a hyprlayer config exists before the CLI runs (the ADR-0002
/// bootstrap exception): on a true first run, write the minimal v4 config from
/// the supplied username, then hand ownership back to the CLI.
async fn ensure_config(state: &AppState, bootstrap_user: Option<String>) -> CmdResult<()> {
    if state.config.exists() {
        return Ok(());
    }
    let user = bootstrap_user.unwrap_or_default();
    if user.trim().is_empty() {
        return Err(AppError {
            code: "bootstrap_user_required".into(),
            message: "No hyprlayer config exists yet; a username is needed to create one.".into(),
            remedy: Some(
                "Enter the username your thought documents should be attributed to.".into(),
            ),
        });
    }
    hyprlayer_core::config::write_bootstrap_config(state.config.path(), &user)
        .map_err(AppError::from)
}

/// Where new project clones land: an explicit per-call `location`, else the
/// configured repos root, else `~/hyprlayer/repos`.
async fn repos_root(state: &AppState, location: Option<&str>) -> PathBuf {
    if let Some(l) = location.map(str::trim).filter(|l| !l.is_empty()) {
        return PathBuf::from(l);
    }
    state
        .settings
        .get()
        .await
        .repos_root
        .map(PathBuf::from)
        .unwrap_or_else(repos::default_repos_root)
}

/// A project directory name: an explicit override, else derived from the
/// source URL, else the on-disk folder name.
fn project_name(directory_name: Option<String>, source: &str, repo: &Path) -> String {
    directory_name
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .or_else(|| repos::repo_name_from_url(source))
        .unwrap_or_else(|| {
            repo.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unnamed_repo")
                .to_string()
        })
}

/// Clone `source` into the repos root as `<name>`, provision thoughts inside
/// the clone, and register it. The clone is rolled back if provisioning or
/// registration fails, so a half-set-up project never lingers.
async fn provision_project(
    app: &tauri::AppHandle,
    state: &AppState,
    source: &str,
    name: &str,
    kind: &str,
    location: Option<&str>,
) -> CmdResult<Project> {
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;

    let dest = repos_root(state, location).await.join(name);
    if dest.exists() {
        return Err(AppError {
            code: "repo_exists".into(),
            message: format!("{} already exists", dest.display()),
            remedy: Some("Remove it first, or choose a different name or location.".into()),
        });
    }

    repos::clone_repo(&git, source, &dest)
        .await
        .map_err(AppError::from)?;

    // Provision thoughts in the clone; roll the clone back on failure so we
    // never leave an unregistered directory behind.
    if let Err(e) = HyprCli::new(&state.binaries)
        .thoughts_init(&dest, name, None)
        .await
    {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(AppError::from(e));
    }

    let origin = repos::origin_url(&git, &dest).await;
    let path = dest.display().to_string();
    let row = NewProject {
        path: path.clone(),
        name: name.to_string(),
        origin_url: origin,
        source: kind.to_string(),
    };
    if let Err(e) = db_task(state.db.clone(), move |db| db.insert_project(row)).await {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }

    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(app);
    crate::rebuild_thoughts_index(app.clone()).await;

    find_project(&registry_projects(state).await, &path)
}

/// Import a project from a local git repo on disk: re-clone it from its
/// `origin` (so the app's copy tracks the same hosted source) into
/// `~/hyprlayer/repos`, or — for a local-only repo with no remote — clone the
/// directory itself. Worktrees are then always cut from that stable clone.
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

    ensure_config(&state, bootstrap_user).await?;

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    // Prefer the repo's own origin as the clone source so the app's copy tracks
    // the same hosted remote; fall back to the local directory for a repo with
    // no remote.
    let source = repos::origin_url(&git, repo)
        .await
        .unwrap_or_else(|| path.clone());

    let name = project_name(directory_name, &source, repo);
    provision_project(&app, &state, &source, &name, "disk", None).await
}

/// Clone a hosted repo (e.g. a GitHub URL) into `~/hyprlayer/repos` and
/// register it as a project.
#[tauri::command]
#[specta::specta]
pub async fn clone_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
    location: Option<String>,
    name: Option<String>,
    bootstrap_user: Option<String>,
) -> CmdResult<Project> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err(AppError {
            code: "invalid_url".into(),
            message: "A repository URL is required.".into(),
            remedy: None,
        });
    }

    ensure_config(&state, bootstrap_user).await?;

    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| repos::repo_name_from_url(&url))
        .ok_or_else(|| AppError {
            code: "invalid_url".into(),
            message: format!("Couldn't derive a project name from {url}."),
            remedy: Some("Add an explicit name, or check the URL.".into()),
        })?;

    provision_project(&app, &state, &url, &name, "clone", location.as_deref()).await
}

/// Remove a project from the registry. With `delete_files`, also delete the
/// clone from disk — but only when it sits inside the repos root and has no
/// active workstreams, so we never destroy a repo whose worktrees are live.
#[tauri::command]
#[specta::specta]
pub async fn remove_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    delete_files: bool,
) -> CmdResult<()> {
    let repo = PathBuf::from(&path);

    // Drop the thoughts repoMapping (best-effort; the directory may be gone).
    if repo.is_dir() {
        let _ = HyprCli::new(&state.binaries).thoughts_uninit(&repo).await;
    }

    let path_for_db = path.clone();
    db_task(state.db.clone(), move |db| db.delete_project(&path_for_db)).await?;

    if delete_files && repo.is_dir() {
        let root = repos_root(&state, None).await;
        let path_for_count = path.clone();
        let active = db_task(state.db.clone(), move |db| {
            db.list_workstreams(&path_for_count, false)
        })
        .await
        .map(|w| w.len())
        .unwrap_or(usize::MAX); // unknown → don't delete
        if repo.starts_with(&root) && active == 0 {
            let _ = std::fs::remove_dir_all(&repo);
        }
    }

    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(&app);
    crate::rebuild_thoughts_index(app.clone()).await;
    Ok(())
}

/// The user's GitHub repos for the clone dialog's "Recent repos" list. Returns
/// an empty list (never an error) when gh is missing or unauthenticated — the
/// URL field still works, so recents are purely a convenience.
#[tauri::command]
#[specta::specta]
pub async fn list_recent_repos(state: State<'_, AppState>) -> CmdResult<Vec<repos::GithubRepo>> {
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(Vec::new());
    };
    match repos::github_repos(&gh, 100).await {
        Ok(list) => Ok(list),
        Err(e) => {
            log::debug!("gh repo list unavailable: {e}");
            Ok(Vec::new())
        }
    }
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
                status: pass_if(info.mapped_name.is_some()),
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

    // 3 & 4. Thoughts root + per-repo symlinks (filesystem backends only).
    checks.extend(thoughts_health_checks(&state, &path).await);

    Ok(HealthReport { checks })
}

/// The thoughts-storage health checks: the storage root exists, and the repo's
/// `thoughts/` symlinks are intact. Empty for non-filesystem backends or when
/// thoughts aren't configured — the early returns keep the happy path flat.
async fn thoughts_health_checks(state: &AppState, path: &str) -> Vec<HealthCheck> {
    let Ok(loaded) = state.config.load().await else {
        return Vec::new();
    };
    let Some(thoughts) = loaded.config.thoughts.as_ref() else {
        return Vec::new();
    };
    let effective = thoughts.effective_config_for(path);
    if !effective.backend.kind().uses_filesystem() {
        return Vec::new();
    }

    let mut checks = Vec::new();

    // 3. Thoughts root exists.
    if let Some(root) = effective.backend.thoughts_root() {
        let exists = root.is_dir();
        checks.push(HealthCheck {
            id: "thoughts.root".into(),
            label: "Thoughts storage root".into(),
            status: pass_if(exists),
            detail: Some(root.display().to_string()),
            remedy: (!exists).then(|| "The configured thoughts directory does not exist. Re-run `hyprlayer thoughts init` or fix the path in the hyprlayer config.".into()),
        });
    }

    // 4. Per-repo thoughts symlinks — same definition of link health as the
    // per-workstream status chips.
    let repo = Path::new(path);
    let repo_thoughts = repo.join(hyprlayer_core::worktree::THOUGHTS_DIR);
    let missing = thoughts_links::missing_thoughts_links(repo, &effective);
    let links_ok = missing.is_empty();
    checks.push(HealthCheck {
        id: "thoughts.symlinks".into(),
        label: "Repository thoughts/ links".into(),
        status: pass_if(links_ok),
        detail: Some(if links_ok {
            repo_thoughts.display().to_string()
        } else {
            format!("missing: thoughts/{}", missing.join(", thoughts/"))
        }),
        remedy: (!links_ok).then(|| {
            "Run `hyprlayer thoughts init` in the repository to recreate the links.".into()
        }),
    });

    checks
}
