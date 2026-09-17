use hyprlayer_core::config::ThoughtsProfile;
use hyprlayer_core::hyprcli::StorageInfo;
use hyprlayer_core::process::binaries::BinaryStatus;
use hyprlayer_core::repos;
use hyprlayer_core::runtime::projects as core_projects;
use hyprlayer_core::store::ProjectSource;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_specta::Event;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;
use hyprlayer_core::settings::AppSettings;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

impl From<core_projects::ProjectView> for Project {
    fn from(v: core_projects::ProjectView) -> Self {
        Self {
            path: v.path,
            mapped_name: v.mapped_name,
            thoughts_project: v.thoughts_project,
            profile: v.profile,
            backend_kind: v.backend_kind,
            exists_on_disk: v.exists_on_disk,
            origin_url: v.origin_url,
            source: v.source,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

impl From<core_projects::ConfigSummary> for ConfigSummary {
    fn from(s: core_projects::ConfigSummary) -> Self {
        match s {
            core_projects::ConfigSummary::Missing { path } => Self::Missing { path },
            core_projects::ConfigSummary::Error {
                path,
                code,
                message,
                remedy,
            } => Self::Error {
                path,
                code,
                message,
                remedy,
            },
            core_projects::ConfigSummary::Ready {
                path,
                user,
                backend_kind,
                configured,
            } => Self::Ready {
                path,
                user,
                backend_kind,
                configured,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub config: ConfigSummary,
    pub projects: Vec<Project>,
    pub binaries: Vec<BinaryStatus>,
    pub settings: AppSettings,
    pub prompt_defaults: PromptDefaults,
    pub default_repos_root: String,
}

impl From<core_projects::BootstrapResult> for Bootstrap {
    fn from(r: core_projects::BootstrapResult) -> Self {
        Self {
            config: r.config.into(),
            projects: r.projects.into_iter().map(Project::from).collect(),
            binaries: r.binaries,
            settings: r.settings,
            prompt_defaults: PromptDefaults::default(),
            default_repos_root: r.default_repos_root,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: Project,
    pub storage: Option<StorageInfo>,
    pub thoughts_root: Option<String>,
}

impl From<core_projects::ProjectInspection> for ProjectDetail {
    fn from(i: core_projects::ProjectInspection) -> Self {
        Self {
            project: i.project.into(),
            storage: i.storage,
            thoughts_root: i.thoughts_root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    #[allow(dead_code)]
    Warn,
    Fail,
}

impl From<core_projects::HealthStatus> for CheckStatus {
    fn from(s: core_projects::HealthStatus) -> Self {
        match s {
            core_projects::HealthStatus::Pass => Self::Pass,
            core_projects::HealthStatus::Fail => Self::Fail,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
    pub remedy: Option<String>,
}

impl From<core_projects::BackendHealthCheck> for HealthCheck {
    fn from(c: core_projects::BackendHealthCheck) -> Self {
        Self {
            id: c.id,
            label: c.label,
            status: c.status.into(),
            detail: c.detail,
            remedy: c.remedy,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub checks: Vec<HealthCheck>,
}

impl From<core_projects::BackendHealth> for HealthReport {
    fn from(h: core_projects::BackendHealth) -> Self {
        Self {
            checks: h.checks.into_iter().map(HealthCheck::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub output: String,
}

impl From<core_projects::SyncThoughtsResult> for SyncReport {
    fn from(r: core_projects::SyncThoughtsResult) -> Self {
        Self { output: r.output }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_bootstrap(state: State<'_, AppState>) -> CmdResult<Bootstrap> {
    let result = state
        .client()?
        .get_bootstrap()
        .await
        .map_err(AppError::from)?;
    Ok(Bootstrap::from(result))
}

#[tauri::command]
#[specta::specta]
pub async fn get_project(state: State<'_, AppState>, path: String) -> CmdResult<ProjectDetail> {
    let inspection = state
        .client()?
        .get_project(path.clone())
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::project_not_found(&path))?;
    Ok(ProjectDetail::from(inspection))
}

#[tauri::command]
#[specta::specta]
pub async fn inspect_repo_path(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<repos::RepoPathStatus> {
    state
        .client()?
        .inspect_repo_path(path)
        .await
        .map_err(AppError::from)
}

// The server-side read half of `PathField`'s remote picker; every path command
// here now takes its path from the machine the path lives on.
#[tauri::command]
#[specta::specta]
pub async fn browse_directory(
    state: State<'_, AppState>,
    path: Option<String>,
) -> CmdResult<core_projects::DirListing> {
    state
        .client()?
        .browse_directory(path)
        .await
        .map_err(AppError::from)
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
    let view = state
        .client()?
        .import_project(
            path,
            branch,
            directory_name,
            thoughts_dir,
            profile,
            bootstrap_user,
        )
        .await
        .map_err(AppError::from)?;
    let _ = crate::events::ConfigChanged.emit(&app);
    Ok(Project::from(view))
}

#[tauri::command]
#[specta::specta]
pub async fn import_local_folder(
    state: State<'_, AppState>,
    path: String,
    directory_name: Option<String>,
) -> CmdResult<Project> {
    let view = state
        .client()?
        .import_local_folder(path, directory_name)
        .await
        .map_err(AppError::from)?;
    Ok(Project::from(view))
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
    let view = state
        .client()?
        .clone_project(url, location, name, thoughts_dir, profile, bootstrap_user)
        .await
        .map_err(AppError::from)?;
    let _ = crate::events::ConfigChanged.emit(&app);
    Ok(Project::from(view))
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GithubTarget {
    pub owner: String,
    pub name: String,
    pub private: bool,
}

impl From<GithubTarget> for core_projects::GithubTarget {
    fn from(t: GithubTarget) -> Self {
        Self {
            owner: t.owner,
            name: t.name,
            private: t.private,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub op_id: String,
    pub name: String,
    pub location: Option<String>,
    pub branch: Option<String>,
    pub scaffold: repos::Scaffold,
    pub github: Option<GithubTarget>,
    pub thoughts_dir: Option<String>,
    pub profile: Option<String>,
    pub bootstrap_user: Option<String>,
}

impl From<CreateProjectRequest> for core_projects::CreateProjectRequest {
    fn from(r: CreateProjectRequest) -> Self {
        Self {
            op_id: r.op_id,
            name: r.name,
            location: r.location,
            branch: r.branch,
            scaffold: r.scaffold,
            github: r.github.map(Into::into),
            thoughts_dir: r.thoughts_dir,
            profile: r.profile,
            bootstrap_user: r.bootstrap_user,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectOutcome {
    pub project: Project,
    pub github_url: Option<String>,
    pub github_error: Option<AppError>,
}

// Three answers, not two: conflating "gh failed" with "you are not signed in"
// is what silently drops the GitHub half of a create and then caches the lie.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum GithubOwners {
    SignedOut,
    Unavailable { message: String },
    Ready { owners: Vec<repos::GithubOwner> },
}

impl From<core_projects::GithubOwnersOutcome> for GithubOwners {
    fn from(o: core_projects::GithubOwnersOutcome) -> Self {
        match o {
            core_projects::GithubOwnersOutcome::SignedOut => Self::SignedOut,
            core_projects::GithubOwnersOutcome::Unavailable { message } => {
                Self::Unavailable { message }
            }
            core_projects::GithubOwnersOutcome::Ready { owners } => Self::Ready { owners },
        }
    }
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn create_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    req: CreateProjectRequest,
) -> CmdResult<CreateProjectOutcome> {
    let result = state
        .client()?
        .create_project(req.into())
        .await
        .map_err(AppError::from)?;
    let _ = crate::events::ConfigChanged.emit(&app);
    Ok(CreateProjectOutcome {
        project: Project::from(result.project),
        github_url: result.github_url,
        github_error: result.github_error.map(AppError::from),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_owners(state: State<'_, AppState>) -> CmdResult<GithubOwners> {
    let outcome = state
        .client()?
        .list_github_owners()
        .await
        .map_err(AppError::from)?;
    Ok(GithubOwners::from(outcome))
}

#[tauri::command]
#[specta::specta]
pub async fn publish_project_to_github(
    state: State<'_, AppState>,
    op_id: String,
    path: String,
    owner: String,
    name: String,
    is_private: bool,
) -> CmdResult<String> {
    if state
        .client()?
        .find_project(path.clone())
        .await
        .map_err(AppError::from)?
        .is_none()
    {
        return Err(AppError::project_not_found(&path));
    }
    state
        .client()?
        .publish_project_to_github(op_id, path, owner, name, is_private)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    delete_files: bool,
) -> CmdResult<()> {
    let git_backed = state
        .client()?
        .project_source_of(path.clone())
        .await
        .map_err(AppError::from)?
        .is_git();
    state
        .client()?
        .remove_project(path, delete_files)
        .await
        .map_err(AppError::from)?;
    if git_backed {
        let _ = crate::events::ConfigChanged.emit(&app);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_projects(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<()> {
    state
        .client()?
        .reorder_projects(paths)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_repos(state: State<'_, AppState>) -> CmdResult<Vec<repos::GithubRepo>> {
    state
        .client()?
        .list_recent_repos()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_thoughts_profiles(state: State<'_, AppState>) -> CmdResult<Vec<ThoughtsProfile>> {
    state
        .client()?
        .list_thoughts_profiles()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn create_thoughts_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    thoughts_repo: Option<String>,
) -> CmdResult<ThoughtsProfile> {
    let profile = state
        .client()?
        .create_thoughts_profile(name, thoughts_repo)
        .await
        .map_err(AppError::from)?;
    let _ = crate::events::ConfigChanged.emit(&app);
    Ok(profile)
}

#[tauri::command]
#[specta::specta]
pub async fn sync_thoughts(
    state: State<'_, AppState>,
    path: String,
    message: Option<String>,
) -> CmdResult<SyncReport> {
    let result = state
        .client()?
        .sync_thoughts(path, message)
        .await
        .map_err(AppError::from)?;
    Ok(SyncReport::from(result))
}

#[tauri::command]
#[specta::specta]
pub async fn run_backend_health(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<HealthReport> {
    let result = state
        .client()?
        .run_backend_health(path)
        .await
        .map_err(AppError::from)?;
    Ok(HealthReport::from(result))
}

#[tauri::command]
#[specta::specta]
pub async fn get_worktree_files(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<Vec<String>> {
    state
        .client()?
        .get_worktree_files(path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_worktree_files(
    state: State<'_, AppState>,
    path: String,
    patterns: Vec<String>,
) -> CmdResult<Vec<String>> {
    state
        .client()?
        .set_worktree_files(path, patterns)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn check_worktree_files(
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<hyprlayer_core::worktree::seed::SeedReport> {
    state
        .client()?
        .check_worktree_files(path)
        .await
        .map_err(AppError::from)
}
