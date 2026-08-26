use std::path::{Path, PathBuf};

use hyprlayer_core::config::{
    bootstrap_default_profile, get_profile, list_profiles, validate_profile_name, ThoughtsProfile,
    DEFAULT_GLOBAL_DIR, DEFAULT_REPOS_DIR,
};
use hyprlayer_core::hyprcli::{HyprCli, StorageInfo};
use hyprlayer_core::process::binaries::{BinaryKind, BinaryStatus};
use hyprlayer_core::repos;
use hyprlayer_core::store::{NewProject, ProjectSource};
use hyprlayer_core::thoughts_repo::{classify_root, scaffold as scaffold_thoughts_root, RootState};
use hyprlayer_core::worktree::thoughts_links;
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;
use tauri_specta::Event;

use super::{blocking, db_task};
use crate::error::{AppError, CmdResult};
use crate::settings::AppSettings;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub mapped_name: String,
    pub thoughts_project: String,
    pub profile: Option<String>,
    pub backend_kind: String,
    pub exists_on_disk: bool,
    pub origin_url: Option<String>,
    pub source: ProjectSource,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum ConfigSummary {
    Missing {
        path: String,
    },
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
pub struct PromptDefaults {
    pub commit: String,
    pub ship: String,
}

impl Default for PromptDefaults {
    fn default() -> Self {
        Self {
            commit: hyprlayer_core::pipeline::COMMIT_PROMPT_DEFAULT.to_string(),
            ship: hyprlayer_core::pipeline::SHIP_PROMPT_TEMPLATE.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub config: ConfigSummary,
    pub projects: Vec<Project>,
    pub binaries: Vec<BinaryStatus>,
    pub settings: AppSettings,
    pub prompt_defaults: PromptDefaults,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: Project,
    pub storage: Option<StorageInfo>,
    pub thoughts_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    #[allow(dead_code)]
    Warn,
    Fail,
}

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

pub(crate) async fn registry_projects(state: &AppState) -> Vec<Project> {
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
            let thoughts_project = if r.source.is_git() {
                eff.as_ref()
                    .and_then(|e| e.mapped_name.clone())
                    .unwrap_or_else(|| r.name.clone())
            } else {
                String::new()
            };
            Project {
                exists_on_disk: Path::new(&r.path).is_dir(),
                path: r.path,
                mapped_name: r.name,
                thoughts_project,
                profile,
                backend_kind,
                origin_url: r.origin_url,
                source: r.source,
            }
        })
        .collect()
}

pub(crate) async fn project_source(state: &AppState, path: &str) -> ProjectSource {
    let path = path.to_string();
    db_task(state.db.clone(), move |db| db.get_project_record(&path))
        .await
        .ok()
        .flatten()
        .map(|r| r.source)
        .unwrap_or(ProjectSource::Disk)
}

pub(crate) async fn config_summary(state: &AppState) -> ConfigSummary {
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
        prompt_defaults: PromptDefaults::default(),
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

    if !project.source.is_git() {
        return Ok(ProjectDetail {
            project,
            storage: None,
            thoughts_root: None,
        });
    }

    let storage = HyprCli::new(&state.binaries)
        .storage_info(Path::new(&path))
        .await?;

    let thoughts_root = effective_thoughts_root(&state, &path).await;

    Ok(ProjectDetail {
        project,
        storage: Some(storage),
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

async fn remove_dir_all_bg(path: &Path) {
    let path = path.to_path_buf();
    let _ = blocking("remove_dir", move || std::fs::remove_dir_all(path)).await;
}

#[derive(Clone, Copy, Default)]
struct ThoughtsTarget<'a> {
    dir: Option<&'a str>,
    profile: Option<&'a str>,
}

#[allow(clippy::too_many_arguments)]
async fn register_repo(
    app: &tauri::AppHandle,
    state: &AppState,
    git: &Path,
    repo: &Path,
    name: &str,
    thoughts: ThoughtsTarget<'_>,
    kind: ProjectSource,
    owned: bool,
) -> CmdResult<Project> {
    if let Err(e) = HyprCli::new(&state.binaries)
        .thoughts_init(repo, thoughts.dir.unwrap_or(name), thoughts.profile)
        .await
    {
        if owned {
            remove_dir_all_bg(repo).await;
        }
        return Err(AppError::from(e));
    }

    let origin = repos::origin_url(git, repo).await;
    let path = repo.display().to_string();
    let row = NewProject {
        path: path.clone(),
        name: name.to_string(),
        origin_url: origin,
        source: kind,
    };
    if let Err(e) = db_task(state.db.clone(), move |db| db.insert_project(row)).await {
        if owned {
            remove_dir_all_bg(repo).await;
        } else {
            let _ = HyprCli::new(&state.binaries).thoughts_uninit(repo).await;
        }
        return Err(e);
    }

    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(app);
    crate::rebuild_thoughts_index(app.clone()).await;

    find_project(&registry_projects(state).await, &path)
}

async fn provision_project(
    app: &tauri::AppHandle,
    state: &AppState,
    source: &str,
    name: &str,
    thoughts: ThoughtsTarget<'_>,
    kind: ProjectSource,
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

    register_repo(app, state, &git, &dest, name, thoughts, kind, true).await
}

#[tauri::command]
#[specta::specta]
pub fn inspect_repo_path(path: String) -> CmdResult<repos::RepoPathStatus> {
    Ok(repos::classify_repo_path(Path::new(&path)))
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn import_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    branch: Option<String>,
    directory_name: Option<String>,
    thoughts_dir: Option<String>,
    profile: Option<String>,
    bootstrap_user: Option<String>,
) -> CmdResult<Project> {
    let repo = Path::new(&path);
    let status = repos::classify_repo_path(repo);

    match status {
        repos::RepoPathStatus::Missing => {
            return Err(AppError {
                code: "invalid_path".into(),
                message: format!("{path} is not a directory"),
                remedy: None,
            });
        }
        repos::RepoPathStatus::Bare => {
            return Err(AppError {
                code: "bare_repo".into(),
                message: format!("{path} is a bare git repository"),
                remedy: Some(
                    "Choose a normal working copy instead. Worktrees can't be created from a bare repo."
                        .into(),
                ),
            });
        }
        _ => {}
    }

    let canonical = dunce::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let key = canonical.display().to_string();
    let existing = db_task(state.db.clone(), move |db| db.get_project_record(&key)).await?;
    if existing.is_some_and(|r| !r.source.is_git()) {
        return Err(AppError {
            code: "project_kind_conflict".into(),
            message: format!("{path} is already added as a local folder project."),
            remedy: Some("Remove the project first, then re-add it as a repository.".into()),
        });
    }

    ensure_config(&state, bootstrap_user).await?;

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;

    let created = matches!(status, repos::RepoPathStatus::NonGit);

    let setup: Result<(), AppError> = match status {
        repos::RepoPathStatus::NonGit => {
            let branch = branch
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .unwrap_or("main");
            repos::init_repo_with_commit(&git, repo, branch)
                .await
                .map_err(AppError::from)
        }
        repos::RepoPathStatus::Unborn => repos::commit_empty_initial(&git, repo)
            .await
            .map_err(AppError::from),
        _ => Ok(()),
    };

    let thoughts = ThoughtsTarget {
        dir: thoughts_dir.as_deref(),
        profile: profile.as_deref(),
    };
    let result = match setup {
        Ok(()) => register_disk_project(&app, &state, &git, repo, directory_name, thoughts).await,
        Err(e) => Err(e),
    };

    if result.is_err() && created {
        remove_dir_all_bg(&repo.join(".git")).await;
    }

    result
}

#[tauri::command]
#[specta::specta]
pub async fn import_local_folder(
    state: State<'_, AppState>,
    path: String,
    directory_name: Option<String>,
) -> CmdResult<Project> {
    let dir = Path::new(&path);
    match repos::classify_repo_path(dir) {
        repos::RepoPathStatus::Missing => {
            return Err(AppError {
                code: "invalid_path".into(),
                message: format!("{path} is not a directory"),
                remedy: None,
            });
        }
        repos::RepoPathStatus::NonGit => {}
        _ => {
            return Err(AppError {
                code: "folder_is_git_repo".into(),
                message: format!("{path} is a git repository."),
                remedy: Some(
                    "Add it as a repository instead, so it keeps its branches and thoughts.".into(),
                ),
            });
        }
    }

    let dir = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let name = project_name(directory_name, &dir.to_string_lossy(), &dir);
    let registered = dir.display().to_string();
    let key = registered.clone();
    let existing = db_task(state.db.clone(), move |db| db.get_project_record(&key)).await?;
    if existing.is_some_and(|r| r.source.is_git()) {
        return Err(AppError {
            code: "project_kind_conflict".into(),
            message: format!("{path} is already added as a repository project."),
            remedy: Some("Remove the project first, then re-add it as a folder.".into()),
        });
    }
    db_task(state.db.clone(), {
        let row = NewProject {
            path: registered.clone(),
            name,
            origin_url: None,
            source: ProjectSource::Folder,
        };
        move |db| db.insert_project(row)
    })
    .await?;

    find_project(&registry_projects(&state).await, &registered)
}

async fn register_disk_project(
    app: &tauri::AppHandle,
    state: &AppState,
    git: &Path,
    repo: &Path,
    directory_name: Option<String>,
    thoughts: ThoughtsTarget<'_>,
) -> CmdResult<Project> {
    let repo = dunce::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let name = project_name(directory_name, &repo.to_string_lossy(), &repo);
    register_repo(
        app,
        state,
        git,
        &repo,
        &name,
        thoughts,
        ProjectSource::Disk,
        false,
    )
    .await
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn clone_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
    location: Option<String>,
    name: Option<String>,
    thoughts_dir: Option<String>,
    profile: Option<String>,
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

    let thoughts = ThoughtsTarget {
        dir: thoughts_dir.as_deref(),
        profile: profile.as_deref(),
    };
    provision_project(
        &app,
        &state,
        &url,
        &name,
        thoughts,
        ProjectSource::Clone,
        location.as_deref(),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    delete_files: bool,
) -> CmdResult<()> {
    let repo = PathBuf::from(&path);

    let path_for_count = path.clone();
    let active = db_task(state.db.clone(), move |db| {
        db.list_workstreams(&path_for_count, false)
    })
    .await?
    .len();
    if active > 0 {
        return Err(AppError {
            code: "active_workstreams".into(),
            message: format!(
                "This project still has {active} active workstream{}.",
                if active == 1 { "" } else { "s" }
            ),
            remedy: Some("Archive its workstreams first, then remove the project.".into()),
        });
    }

    let git_backed = project_source(&state, &path).await.is_git();

    if git_backed && repo.is_dir() {
        let _ = HyprCli::new(&state.binaries).thoughts_uninit(&repo).await;
    }

    let path_for_db = path.clone();
    db_task(state.db.clone(), move |db| db.delete_project(&path_for_db)).await?;

    if delete_files && repo.is_dir() {
        remove_dir_all_bg(&repo).await;
    }

    if git_backed {
        state.config.invalidate().await;
        let _ = crate::events::ConfigChanged.emit(&app);
        crate::rebuild_thoughts_index(app.clone()).await;
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_projects(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<()> {
    db_task(state.db.clone(), move |db| db.reorder_projects(&paths)).await?;
    Ok(())
}

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
pub async fn list_thoughts_profiles(state: State<'_, AppState>) -> CmdResult<Vec<ThoughtsProfile>> {
    let thoughts = match state.config.load().await {
        Ok(loaded) => loaded.config.thoughts.clone(),
        Err(CoreError::ConfigMissing { .. }) => None,
        Err(e) => return Err(AppError::from(e)),
    };
    blocking("list_thoughts_profiles", move || match thoughts {
        Some(t) => list_profiles(&t),
        None => vec![bootstrap_default_profile()],
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn create_thoughts_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    thoughts_repo: Option<String>,
) -> CmdResult<ThoughtsProfile> {
    let name = name.trim().to_string();
    validate_profile_name(&name).map_err(AppError::from)?;

    let loaded = state.config.load().await.map_err(AppError::from)?;
    if loaded
        .config
        .thoughts
        .as_ref()
        .is_some_and(|t| t.profiles.contains_key(&name))
    {
        return Err(AppError::from(CoreError::ProfileExists { name }));
    }

    let repo = thoughts_repo
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("~/{name}-thoughts"));

    let root = hyprlayer_core::config::expand_path(&repo);
    if matches!(
        classify_root(&root, DEFAULT_REPOS_DIR, DEFAULT_GLOBAL_DIR),
        RootState::Scaffoldable | RootState::Adoptable
    ) {
        let git = state
            .binaries
            .require(BinaryKind::Git)
            .await
            .map_err(AppError::from)?;
        let user = loaded
            .config
            .thoughts
            .as_ref()
            .map(|t| t.user.as_str())
            .unwrap_or_default();
        scaffold_thoughts_root(&git, &root, user, DEFAULT_REPOS_DIR, DEFAULT_GLOBAL_DIR)
            .await
            .map_err(AppError::from)?;
    }

    let cwd = state
        .config
        .path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    HyprCli::new(&state.binaries)
        .thoughts_profile_create(&cwd, &name, &repo, DEFAULT_REPOS_DIR, DEFAULT_GLOBAL_DIR)
        .await
        .map_err(AppError::from)?;

    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(&app);

    let loaded = state.config.load().await.map_err(AppError::from)?;
    let thoughts = loaded
        .config
        .thoughts
        .clone()
        .ok_or_else(AppError::thoughts_not_configured)?;
    blocking("create_thoughts_profile", move || {
        get_profile(&thoughts, &name)
    })
    .await?
    .ok_or_else(|| AppError::internal("profile missing from config after CLI create"))
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

    let git_backed = project_source(&state, &path).await.is_git();

    if !git_backed {
        let dir = Path::new(&path);
        let exists = dir.is_dir();
        checks.push(HealthCheck {
            id: "folder.exists".into(),
            label: "Project folder".into(),
            status: pass_if(exists),
            detail: Some(path.clone()),
            remedy: (!exists).then(|| {
                "The folder was moved or deleted. Remove the project and re-add it.".into()
            }),
        });
        return Ok(HealthReport { checks });
    }

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

    checks.extend(thoughts_health_checks(&state, &path).await);

    Ok(HealthReport { checks })
}

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

    let repo = Path::new(path);
    let repo_thoughts = repo.join(hyprlayer_core::worktree::THOUGHTS_DIR);
    let broken = thoughts_links::broken_thoughts_links(repo, &effective);
    let links_ok = broken.is_empty();
    checks.push(HealthCheck {
        id: "thoughts.symlinks".into(),
        label: "Repository thoughts/ links".into(),
        status: pass_if(links_ok),
        detail: Some(if links_ok {
            repo_thoughts.display().to_string()
        } else {
            format!("broken or stale: thoughts/{}", broken.join(", thoughts/"))
        }),
        remedy: (!links_ok).then(|| {
            "Run `hyprlayer thoughts init` in the repository to recreate the links.".into()
        }),
    });

    checks
}

#[tauri::command]
#[specta::specta]
pub async fn get_worktree_files(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<Vec<String>> {
    let _ = &state;
    blocking("read_repo_settings", move || {
        hyprlayer_core::repo_settings::read(std::path::Path::new(&path)).map(|s| s.worktree.copy)
    })
    .await?
    .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_worktree_files(
    state: State<'_, AppState>,
    path: String,
    patterns: Vec<String>,
) -> CmdResult<Vec<String>> {
    let _ = &state;
    let cleaned: Vec<String> = patterns
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let p = path.clone();
    let saved = cleaned.clone();
    blocking("write_repo_settings", move || {
        hyprlayer_core::repo_settings::set_worktree_copy(std::path::Path::new(&p), &cleaned)
    })
    .await?
    .map_err(AppError::from)?;
    Ok(saved)
}

#[tauri::command]
#[specta::specta]
pub async fn check_worktree_files(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<hyprlayer_core::worktree::seed::SeedReport> {
    let _ = &state;
    blocking("check_worktree_files", move || {
        let repo = std::path::Path::new(&path);
        let patterns = hyprlayer_core::repo_settings::read(repo)
            .map(|s| s.worktree.copy)
            .unwrap_or_default();
        hyprlayer_core::worktree::seed::check_worktree_patterns(repo, &patterns)
    })
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonicalized_project_path_has_no_verbatim_prefix() {
        let here = dunce::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert!(
            !here.to_string_lossy().starts_with(r"\\?\"),
            "registered project path must not carry the \\\\?\\ prefix: {}",
            here.display(),
        );
    }
}
