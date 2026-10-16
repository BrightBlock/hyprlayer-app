use std::path::{Path, PathBuf};

use hyprlayer_models::project::{CreateStep, StepStatus};
use hyprlayer_models::RuntimeEvent;

use crate::config::{
    self, bootstrap_default_profile, get_profile, list_profiles, validate_profile_name,
    ThoughtsProfile, DEFAULT_GLOBAL_DIR, DEFAULT_REPOS_DIR,
};
use crate::error::{CoreError, Result};
use crate::hyprcli::{AiStatus, HyprCli, StorageInfo};
use crate::process::binaries::{BinaryKind, BinaryStatus};
use crate::repo_settings;
use crate::repos;
use crate::settings::AppSettings;
use crate::store::{NewProject, ProjectRecord, ProjectSource};
use crate::thoughts_repo::{self, classify_root, RootState};
use crate::worktree::seed::{self, SeedReport};
use crate::worktree::thoughts_links;
use serde::{Deserialize, Serialize};

use super::HyprlayerRuntime;

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

/// A relative path would resolve against the daemon's working directory.
fn thoughts_location(raw: &str) -> Result<PathBuf> {
    let path = config::expand_path(raw.trim());
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(CoreError::ThoughtsLocationInvalid {
            path: raw.to_string(),
        })
    }
}

fn exec_stderr(err: &CoreError) -> String {
    match err {
        CoreError::Exec { stderr_tail, .. } => stderr_tail.clone(),
        other => other.to_string(),
    }
}

// A failed rollback is the difference between "try again" and "every retry now
// dies on repo_exists", so it has to reach the caller attached to whatever error
// triggered the rollback.
async fn rollback_dir(path: &Path, err: CoreError) -> CoreError {
    tracing::warn!("rolling back {} after: {err}", path.display());
    let owned = path.to_path_buf();
    match blocking(move || std::fs::remove_dir_all(owned).map_err(CoreError::Io)).await {
        Ok(()) => err,
        Err(reason_err) => CoreError::RollbackFailed {
            original: Box::new(err),
            path: path.to_path_buf(),
            reason: reason_err.to_string(),
        },
    }
}

async fn remove_dir_all_bg(path: &Path) -> Result<()> {
    let owned = path.to_path_buf();
    blocking(move || std::fs::remove_dir_all(owned).map_err(CoreError::Io)).await
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

fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(CoreError::ProjectNameRequired);
    }
    if name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(CoreError::ProjectNameInvalid {
            name: name.to_string(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct ThoughtsTarget<'a> {
    dir: Option<&'a str>,
    profile: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub path: String,
    pub mapped_name: String,
    pub thoughts_project: String,
    pub profile: Option<String>,
    pub backend_kind: String,
    pub exists_on_disk: bool,
    pub origin_url: Option<String>,
    pub source: ProjectSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResult {
    pub config: ConfigSummary,
    pub projects: Vec<ProjectView>,
    pub binaries: Vec<BinaryStatus>,
    pub settings: AppSettings,
    /// Where a clone lands when the user names no location — `settings.repos_root`
    /// if set, else this machine's default. Sent because the client cannot work
    /// it out: against a remote host its own home directory is the wrong
    /// machine's, and prefilling one sent `/Users/...` to a Linux server.
    ///
    /// `default` because `PROTOCOL_VERSION` is deliberately pinned at 1 until
    /// the server ships, so a client and a server built at different times are
    /// the normal case during development rather than an error. Without it,
    /// adding this field made an older server fail *bootstrap* deserialization
    /// — the whole app, not one feature — with a raw serde error the strict
    /// handshake had already waved through. New response fields stay tolerant
    /// while the version is pinned.
    #[serde(default)]
    pub default_repos_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInspection {
    pub project: ProjectView,
    pub storage: Option<StorageInfo>,
    pub thoughts_root: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportRepositoryRequest {
    pub path: String,
    pub branch: Option<String>,
    pub directory_name: Option<String>,
    pub thoughts_dir: Option<String>,
    pub profile: Option<String>,
    pub bootstrap_user: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportFolderRequest {
    pub path: String,
    pub directory_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CloneProjectRequest {
    pub url: String,
    pub location: Option<String>,
    pub name: Option<String>,
    pub thoughts_dir: Option<String>,
    pub profile: Option<String>,
    pub bootstrap_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubTarget {
    pub owner: String,
    pub name: String,
    pub private: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug)]
pub struct CreateProjectResult {
    pub project: ProjectView,
    pub github_url: Option<String>,
    pub github_error: Option<CoreError>,
}

#[derive(Debug, Clone)]
pub struct PublishProjectRequest {
    pub op_id: String,
    pub path: String,
    pub owner: String,
    pub name: String,
    pub is_private: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GithubOwnersOutcome {
    SignedOut,
    Unavailable { message: String },
    Ready { owners: Vec<repos::GithubOwner> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendHealthCheck {
    pub id: String,
    pub label: String,
    pub status: HealthStatus,
    pub detail: Option<String>,
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendHealth {
    pub checks: Vec<BackendHealthCheck>,
}

#[derive(Debug, Clone)]
pub struct SyncThoughtsRequest {
    pub path: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncThoughtsResult {
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtsProbe {
    pub default_path: String,
    pub exists: bool,
    pub is_git_repo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone)]
pub struct RunOnboardingRequest {
    pub user: String,
    pub source: ThoughtsSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DirBrowseEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_repo: bool,
    /// The server's own join of `name` onto the listed directory — the
    /// client never guesses a separator for a server whose OS it does not
    /// know (plan Phase 1 §8).
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<DirBrowseEntry>,
}

/// Excludes `.git` the way `gitops::list_dir` does — nobody browsing for a
/// project to import wants it offered as a candidate.
fn list_directory(dir: &Path) -> DirListing {
    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for ent in read.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let is_repo = is_dir && repos::is_git_repo(&ent.path());
            let path = ent.path().display().to_string();
            entries.push(DirBrowseEntry {
                name,
                is_dir,
                is_repo,
                path,
            });
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    DirListing {
        path: dir.display().to_string(),
        parent: dir.parent().map(|p| p.display().to_string()),
        entries,
    }
}

fn pass_if(ok: bool) -> HealthStatus {
    if ok {
        HealthStatus::Pass
    } else {
        HealthStatus::Fail
    }
}

// Progress is a runtime event, so every emission carries the id of the call
// that produced it: a second create (or a publish retry) running concurrently
// would otherwise drive whichever panel happens to be listening.
struct Progress<'a> {
    runtime: &'a HyprlayerRuntime,
    op_id: &'a str,
}

impl Progress<'_> {
    fn emit(&self, step: CreateStep, status: StepStatus, detail: Option<&str>) {
        let _ = self
            .runtime
            .events
            .send(RuntimeEvent::ProjectCreateProgress {
                op_id: self.op_id.to_string(),
                step,
                status,
                detail: detail.map(str::to_string),
            });
    }
}

pub struct ProjectService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn projects(&self) -> ProjectService<'_> {
        ProjectService { runtime: self }
    }
}

impl ProjectService<'_> {
    async fn ensure_config(&self, bootstrap_user: Option<String>) -> Result<()> {
        if self.runtime.config.exists() {
            return Ok(());
        }
        let user = bootstrap_user.unwrap_or_default();
        if user.trim().is_empty() {
            return Err(CoreError::BootstrapUserRequired);
        }
        config::write_bootstrap_config(self.runtime.config.path(), &user)
    }

    async fn repos_root(&self, location: Option<&str>) -> PathBuf {
        let home = Some(self.runtime.environment.home_dir.as_path());
        if let Some(l) = location.map(str::trim).filter(|l| !l.is_empty()) {
            return repos::expand_user_path(l, home);
        }
        self.runtime
            .settings
            .get()
            .await
            .repos_root
            .map(|r| repos::expand_user_path(&r, home))
            .unwrap_or_else(|| self.runtime.paths.default_repos_root.clone())
    }

    async fn list_enriched(&self) -> Vec<ProjectView> {
        let db = self.runtime.db.clone();
        let records = match blocking(move || db.list_projects()).await {
            Ok(records) => records,
            Err(e) => {
                tracing::warn!("could not load project registry: {e}");
                return Vec::new();
            }
        };
        let loaded = self.runtime.config.load().await.ok();
        let thoughts = loaded.as_ref().and_then(|l| l.config.thoughts.as_ref());
        records
            .into_iter()
            .map(|r: ProjectRecord| {
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
                ProjectView {
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

    async fn config_summary(&self) -> ConfigSummary {
        let path = self.runtime.config.path().display().to_string();
        match self.runtime.config.load().await {
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
            Err(err) => ConfigSummary::Error {
                path,
                code: err.code().to_string(),
                message: err.to_string(),
                remedy: err.remedy(),
            },
        }
    }

    async fn effective_thoughts_root(&self, repo_path: &str) -> Option<String> {
        let loaded = self.runtime.config.load().await.ok()?;
        let thoughts = loaded.config.thoughts.as_ref()?;
        let effective = thoughts.effective_config_for(repo_path);
        effective
            .backend
            .thoughts_root()
            .map(|p| p.display().to_string())
    }

    pub async fn bootstrap(&self) -> Result<BootstrapResult> {
        ensure_running(self.runtime)?;
        Ok(BootstrapResult {
            config: self.config_summary().await,
            projects: self.list_enriched().await,
            binaries: self.runtime.binaries.status_all().await,
            settings: self.runtime.settings.get().await,
            default_repos_root: self.repos_root(None).await.display().to_string(),
        })
    }

    /// Looks up one registered project by its exact registered path,
    /// returning `Ok(None)` rather than an error when it isn't registered —
    /// callers decide what a missing project means for their command.
    pub async fn find(&self, path: &str) -> Result<Option<ProjectView>> {
        ensure_running(self.runtime)?;
        Ok(self
            .list_enriched()
            .await
            .into_iter()
            .find(|p| p.path == path))
    }

    pub async fn source_of(&self, path: &str) -> ProjectSource {
        let key = path.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_project_record(&key))
            .await
            .ok()
            .flatten()
            .map(|r| r.source)
            .unwrap_or(ProjectSource::Disk)
    }

    pub fn inspect_repo_path(&self, path: &str) -> repos::RepoPathStatus {
        repos::classify_repo_path(Path::new(path))
    }

    /// Lists a directory on the server's filesystem — the read half a future
    /// remote picker needs, since a client-side file dialog has nothing to
    /// pick from on a machine it isn't running on. `path` defaults to the
    /// server's home directory; a missing or unreadable one falls back to it
    /// too, rather than erroring, since there is nothing a client browsing
    /// blind can do about a bad path except try another one.
    pub fn browse(&self, path: Option<String>) -> DirListing {
        let home = &self.runtime.environment.home_dir;
        let requested = path.map(PathBuf::from).unwrap_or_else(|| home.clone());
        let dir = if requested.is_dir() {
            requested
        } else {
            home.clone()
        };
        let dir = dunce::canonicalize(&dir).unwrap_or(dir);
        list_directory(&dir)
    }

    pub async fn inspect(&self, path: &str) -> Result<Option<ProjectInspection>> {
        let Some(project) = self.find(path).await? else {
            return Ok(None);
        };

        if !project.source.is_git() {
            return Ok(Some(ProjectInspection {
                project,
                storage: None,
                thoughts_root: None,
            }));
        }

        let storage = HyprCli::new(&self.runtime.binaries)
            .storage_info(Path::new(path))
            .await?;
        let thoughts_root = self.effective_thoughts_root(path).await;

        Ok(Some(ProjectInspection {
            project,
            storage: Some(storage),
            thoughts_root,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    async fn register_repo(
        &self,
        git: &Path,
        repo: &Path,
        name: &str,
        thoughts: ThoughtsTarget<'_>,
        kind: ProjectSource,
        owned: bool,
    ) -> Result<ProjectView> {
        if let Err(e) = HyprCli::new(&self.runtime.binaries)
            .thoughts_init(repo, thoughts.dir.unwrap_or(name), thoughts.profile)
            .await
        {
            return Err(if owned {
                rollback_dir(repo, e).await
            } else {
                e
            });
        }

        let origin = repos::origin_url(git, repo, &self.runtime.child_env().await).await;
        let path = repo.display().to_string();
        let row = NewProject {
            path: path.clone(),
            name: name.to_string(),
            origin_url: origin,
            source: kind,
        };
        let db = self.runtime.db.clone();
        if let Err(e) = blocking(move || db.insert_project(row)).await {
            return Err(if owned {
                rollback_dir(repo, e).await
            } else {
                let _ = HyprCli::new(&self.runtime.binaries)
                    .thoughts_uninit(repo)
                    .await;
                e
            });
        }

        self.runtime.config.invalidate().await;
        self.runtime.rebuild_thoughts_index().await;

        self.find(&path).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::other(format!(
                "project not found in registry after write: {path}"
            )))
        })
    }

    async fn register_disk_project(
        &self,
        git: &Path,
        repo: &Path,
        directory_name: Option<String>,
        thoughts: ThoughtsTarget<'_>,
    ) -> Result<ProjectView> {
        let repo = dunce::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
        let name = project_name(directory_name, &repo.to_string_lossy(), &repo);
        self.register_repo(git, &repo, &name, thoughts, ProjectSource::Disk, false)
            .await
    }

    pub async fn import_repository(&self, req: ImportRepositoryRequest) -> Result<ProjectView> {
        ensure_running(self.runtime)?;
        let ImportRepositoryRequest {
            path,
            branch,
            directory_name,
            thoughts_dir,
            profile,
            bootstrap_user,
        } = req;
        let repo = Path::new(&path);
        let status = repos::classify_repo_path(repo);

        match status {
            repos::RepoPathStatus::Missing => {
                return Err(CoreError::PathNotADirectory { path: path.clone() });
            }
            repos::RepoPathStatus::Bare => {
                return Err(CoreError::BareRepo { path: path.clone() });
            }
            _ => {}
        }
        if let Some(e) = repos::unreadable_repo(repo) {
            tracing::warn!("import refused: {e}");
            return Err(e);
        }

        let canonical = dunce::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
        let key = canonical.display().to_string();
        let db = self.runtime.db.clone();
        let existing = {
            let key = key.clone();
            blocking(move || db.get_project_record(&key)).await?
        };
        if existing.is_some_and(|r| !r.source.is_git()) {
            return Err(CoreError::ProjectKindConflict {
                path: path.clone(),
                existing_kind: "local folder",
            });
        }

        self.ensure_config(bootstrap_user).await?;

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;

        // Only a `.git` this call creates may be rolled back.
        let created = matches!(status, repos::RepoPathStatus::NonGit) && !repos::is_git_repo(repo);

        let setup: Result<()> = match status {
            repos::RepoPathStatus::NonGit => {
                let branch = branch
                    .as_deref()
                    .map(str::trim)
                    .filter(|b| !b.is_empty())
                    .unwrap_or("main");
                repos::init_repo_with_commit(&git, repo, branch, &self.runtime.child_env().await)
                    .await
            }
            repos::RepoPathStatus::Unborn => {
                repos::commit_empty_initial(&git, repo, &self.runtime.child_env().await).await
            }
            _ => Ok(()),
        };

        let thoughts = ThoughtsTarget {
            dir: thoughts_dir.as_deref(),
            profile: profile.as_deref(),
        };
        let result = match setup {
            Ok(()) => {
                self.register_disk_project(&git, repo, directory_name, thoughts)
                    .await
            }
            Err(e) => Err(e),
        };

        match result {
            Err(e) if created => Err(rollback_dir(&repo.join(".git"), e).await),
            other => other,
        }
    }

    pub async fn import_folder(&self, req: ImportFolderRequest) -> Result<ProjectView> {
        ensure_running(self.runtime)?;
        let ImportFolderRequest {
            path,
            directory_name,
        } = req;
        let dir = Path::new(&path);
        match repos::classify_repo_path(dir) {
            repos::RepoPathStatus::Missing => {
                return Err(CoreError::PathNotADirectory { path: path.clone() });
            }
            repos::RepoPathStatus::NonGit => {}
            _ => return Err(CoreError::FolderIsGitRepo { path: path.clone() }),
        }

        let dir = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let name = project_name(directory_name, &dir.to_string_lossy(), &dir);
        let registered = dir.display().to_string();
        let db = self.runtime.db.clone();
        let existing = {
            let key = registered.clone();
            blocking(move || db.get_project_record(&key)).await?
        };
        if existing.is_some_and(|r| r.source.is_git()) {
            return Err(CoreError::ProjectKindConflict {
                path: path.clone(),
                existing_kind: "repository",
            });
        }

        let db = self.runtime.db.clone();
        {
            let row = NewProject {
                path: registered.clone(),
                name,
                origin_url: None,
                source: ProjectSource::Folder,
            };
            blocking(move || db.insert_project(row)).await?;
        }

        let view = self.find(&registered).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::other(format!(
                "project not found in registry after write: {registered}"
            )))
        })?;
        let _ = self.runtime.events.send(RuntimeEvent::ProjectsChanged);
        Ok(view)
    }

    async fn provision_project(
        &self,
        source: &str,
        name: &str,
        thoughts: ThoughtsTarget<'_>,
        kind: ProjectSource,
        location: Option<&str>,
    ) -> Result<ProjectView> {
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;

        let dest = self.repos_root(location).await.join(name);
        if dest.exists() {
            return Err(CoreError::RepoAlreadyExists {
                path: dest,
                offer_remove: true,
            });
        }

        repos::clone_repo(&git, source, &dest, &self.runtime.child_env().await).await?;

        self.register_repo(&git, &dest, name, thoughts, kind, true)
            .await
    }

    pub async fn clone(&self, req: CloneProjectRequest) -> Result<ProjectView> {
        ensure_running(self.runtime)?;
        let CloneProjectRequest {
            url,
            location,
            name,
            thoughts_dir,
            profile,
            bootstrap_user,
        } = req;
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err(CoreError::RepoUrlRequired);
        }

        self.ensure_config(bootstrap_user).await?;

        let name = name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| repos::repo_name_from_url(&url))
            .ok_or_else(|| CoreError::RepoNameUndeducible { url: url.clone() })?;

        let thoughts = ThoughtsTarget {
            dir: thoughts_dir.as_deref(),
            profile: profile.as_deref(),
        };
        self.provision_project(
            &url,
            &name,
            thoughts,
            ProjectSource::Clone,
            location.as_deref(),
        )
        .await
    }

    async fn sync_origin(&self, git: &Path, repo: &Path) -> Result<Option<ProjectView>> {
        let path = repo.display().to_string();
        let db = self.runtime.db.clone();
        let record = {
            let key = path.clone();
            blocking(move || db.get_project_record(&key)).await?
        };
        let Some(record) = record else {
            return Ok(None);
        };
        let origin = repos::origin_url(git, repo, &self.runtime.child_env().await).await;
        if origin == record.origin_url {
            return Ok(None);
        }
        let db = self.runtime.db.clone();
        blocking(move || {
            db.insert_project(NewProject {
                path: record.path,
                name: record.name,
                origin_url: origin,
                source: record.source,
            })
        })
        .await?;
        Ok(self.find(&path).await.ok().flatten())
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_to_github(
        &self,
        progress: &Progress<'_>,
        git: &Path,
        repo: &Path,
        branch: &str,
        target: &GithubTarget,
    ) -> Result<String> {
        let fail = |step: CreateStep| {
            progress.emit(step, StepStatus::Failed, None);
            if matches!(step, CreateStep::GithubRepo) {
                progress.emit(CreateStep::Push, StepStatus::Skipped, None);
            }
        };

        let owner = target.owner.trim().to_string();
        let name = repos::github_repo_name(&target.name);
        if owner.is_empty() || repos::is_reserved_repo_name(&name) {
            return Err(CoreError::GithubNameInvalid {
                name: target.name.clone(),
            });
        }

        progress.emit(CreateStep::GithubRepo, StepStatus::Running, None);

        let origin = repos::origin_url(git, repo, &self.runtime.child_env().await).await;
        let url =
            match repos::plan_publish(origin.as_deref(), &owner, &name) {
                repos::PublishPlan::PushOnly { url } => url,
                repos::PublishPlan::Conflict { origin } => {
                    fail(CreateStep::GithubRepo);
                    return Err(CoreError::GithubOriginExists { origin });
                }
                repos::PublishPlan::Create => {
                    let gh = match self.runtime.binaries.require(BinaryKind::Gh).await {
                        Ok(gh) => gh,
                        Err(e) => {
                            fail(CreateStep::GithubRepo);
                            return Err(e);
                        }
                    };
                    match repos::create_github_repo(
                        &gh,
                        repo,
                        &owner,
                        &name,
                        target.private,
                        &self.runtime.child_env().await,
                    )
                    .await
                    {
                        Ok(url) => url,
                        Err(e) => {
                            fail(CreateStep::GithubRepo);
                            return Err(CoreError::GithubOperation(
                                repos::classify_github_failure(&exec_stderr(&e), &owner, &name),
                            ));
                        }
                    }
                }
            };
        progress.emit(CreateStep::GithubRepo, StepStatus::Done, Some(&url));

        progress.emit(CreateStep::Push, StepStatus::Running, None);
        if let Err(e) =
            repos::push_upstream(git, repo, branch, &self.runtime.child_env().await).await
        {
            fail(CreateStep::Push);
            return Err(CoreError::GithubOperation(repos::classify_push_failure(
                &exec_stderr(&e),
            )));
        }
        progress.emit(CreateStep::Push, StepStatus::Done, Some(branch));
        Ok(url)
    }

    pub async fn create(&self, req: CreateProjectRequest) -> Result<CreateProjectResult> {
        ensure_running(self.runtime)?;
        let CreateProjectRequest {
            op_id,
            name,
            location,
            branch,
            scaffold,
            github,
            thoughts_dir,
            profile,
            bootstrap_user,
        } = req;
        let progress = Progress {
            runtime: self.runtime,
            op_id: &op_id,
        };
        let name = name.trim().to_string();
        validate_project_name(&name)?;
        self.ensure_config(bootstrap_user).await?;

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;

        let dest = self.repos_root(location.as_deref()).await.join(&name);
        let branch = branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or("main")
            .to_string();

        // `create_dir` is the claim: it fails with AlreadyExists rather than
        // adopting a directory someone else owns, which matters because everything
        // below stages the whole tree, may push it, and deletes it on failure.
        progress.emit(CreateStep::Folder, StepStatus::Running, None);
        let claimed = blocking({
            let dest = dest.clone();
            move || {
                // Both name their path: the caller still matches `AlreadyExists`
                // on `kind`, but a bare io error here says only "Permission
                // denied" about a directory it declines to identify.
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        CoreError::Io(std::io::Error::new(
                            e.kind(),
                            format!("could not create {}: {e}", parent.display()),
                        ))
                    })?;
                }
                std::fs::create_dir(&dest).map_err(|e| {
                    CoreError::Io(std::io::Error::new(
                        e.kind(),
                        format!("could not create {}: {e}", dest.display()),
                    ))
                })
            }
        })
        .await;
        if let Err(e) = claimed {
            progress.emit(CreateStep::Folder, StepStatus::Failed, None);
            return Err(match e {
                CoreError::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists => {
                    CoreError::RepoAlreadyExists {
                        path: dest,
                        offer_remove: false,
                    }
                }
                other => other,
            });
        }
        progress.emit(
            CreateStep::Folder,
            StepStatus::Done,
            Some(&dest.display().to_string()),
        );

        progress.emit(CreateStep::Repository, StepStatus::Running, None);
        if let Err(e) =
            repos::init_repo(&git, &dest, &branch, &self.runtime.child_env().await).await
        {
            progress.emit(CreateStep::Repository, StepStatus::Failed, None);
            return Err(rollback_dir(&dest, e).await);
        }
        progress.emit(CreateStep::Repository, StepStatus::Done, Some(&branch));

        progress.emit(CreateStep::Commit, StepStatus::Running, None);
        let commit: Result<Vec<String>> = async {
            let written = blocking({
                let dest = dest.clone();
                let name = name.clone();
                move || repos::write_scaffold(&dest, &name, &scaffold)
            })
            .await?;
            repos::stage_all(&git, &dest, &self.runtime.child_env().await).await?;
            repos::commit_empty_initial(&git, &dest, &self.runtime.child_env().await).await?;
            Ok(written)
        }
        .await;
        let written = match commit {
            Ok(written) => written,
            Err(e) => {
                progress.emit(CreateStep::Commit, StepStatus::Failed, None);
                return Err(rollback_dir(&dest, e).await);
            }
        };
        let commit_detail = if written.is_empty() {
            "empty initial commit".to_string()
        } else {
            written.join(" · ")
        };
        progress.emit(CreateStep::Commit, StepStatus::Done, Some(&commit_detail));

        // Registration runs before publishing so its failure path is a clean
        // rollback of a purely local repo. Publishing first would mean a failed
        // register had to choose between deleting a pushed tree and stranding an
        // unregistered one.
        progress.emit(CreateStep::Thoughts, StepStatus::Running, None);
        let thoughts = ThoughtsTarget {
            dir: thoughts_dir.as_deref(),
            profile: profile.as_deref(),
        };
        let project = match self
            .register_repo(&git, &dest, &name, thoughts, ProjectSource::Clone, true)
            .await
        {
            Ok(project) => project,
            Err(e) => {
                progress.emit(CreateStep::Thoughts, StepStatus::Failed, None);
                return Err(e);
            }
        };
        progress.emit(
            CreateStep::Thoughts,
            StepStatus::Done,
            Some(thoughts_dir.as_deref().unwrap_or(&name)),
        );

        let mut github_url = None;
        let mut github_error = None;
        match github.as_ref() {
            Some(target) => {
                match self
                    .publish_to_github(&progress, &git, &dest, &branch, target)
                    .await
                {
                    Ok(url) => github_url = Some(url),
                    Err(e) => github_error = Some(e),
                }
            }
            None => {
                progress.emit(CreateStep::GithubRepo, StepStatus::Skipped, None);
                progress.emit(CreateStep::Push, StepStatus::Skipped, None);
            }
        }
        // The project row was written before origin existed, so re-read it.
        let project = match self.sync_origin(&git, &dest).await {
            Ok(Some(refreshed)) => refreshed,
            _ => project,
        };

        Ok(CreateProjectResult {
            project,
            github_url,
            github_error,
        })
    }

    pub async fn github_owners(&self) -> Result<GithubOwnersOutcome> {
        ensure_running(self.runtime)?;
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return Ok(GithubOwnersOutcome::SignedOut);
        };
        match repos::github_owners(&gh, &self.runtime.child_env().await).await {
            Ok(owners) => Ok(GithubOwnersOutcome::Ready { owners }),
            Err(e) => {
                let detail = exec_stderr(&e);
                if repos::is_github_auth_failure(&detail) {
                    return Ok(GithubOwnersOutcome::SignedOut);
                }
                tracing::debug!("gh owners unavailable: {e}");
                Ok(GithubOwnersOutcome::Unavailable {
                    message: if detail.trim().is_empty() {
                        "GitHub could not be reached.".to_string()
                    } else {
                        detail
                    },
                })
            }
        }
    }

    pub async fn publish(&self, req: PublishProjectRequest) -> Result<String> {
        ensure_running(self.runtime)?;
        let PublishProjectRequest {
            op_id,
            path,
            owner,
            name,
            is_private,
        } = req;
        let progress = Progress {
            runtime: self.runtime,
            op_id: &op_id,
        };
        let repo = PathBuf::from(&path);

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        let branch = crate::gitops::current_branch(&repo).unwrap_or_else(|| "main".to_string());

        let published = self
            .publish_to_github(
                &progress,
                &git,
                &repo,
                &branch,
                &GithubTarget {
                    owner,
                    name,
                    private: is_private,
                },
            )
            .await;

        // `gh repo create --remote origin` wires up the remote before the push runs,
        // so a failed publish can still have changed origin. Record it either way,
        // which is what makes the next retry a push rather than a re-create.
        let _ = self.sync_origin(&git, &repo).await;
        published
    }

    pub async fn recent_repos(&self) -> Result<Vec<repos::GithubRepo>> {
        ensure_running(self.runtime)?;
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return Ok(Vec::new());
        };
        match repos::github_repos(&gh, 100, &self.runtime.child_env().await).await {
            Ok(list) => Ok(list),
            Err(e) => {
                tracing::debug!("gh repo list unavailable: {e}");
                Ok(Vec::new())
            }
        }
    }

    pub async fn remove(&self, path: &str, delete_files: bool) -> Result<()> {
        ensure_running(self.runtime)?;
        let repo = PathBuf::from(path);

        let db = self.runtime.db.clone();
        let active = {
            let key = path.to_string();
            blocking(move || db.list_workstreams(&key, false)).await?
        }
        .len();
        if active > 0 {
            return Err(CoreError::ActiveWorkstreams { count: active });
        }

        let git_backed = self.source_of(path).await.is_git();

        if git_backed && repo.is_dir() {
            let _ = HyprCli::new(&self.runtime.binaries)
                .thoughts_uninit(&repo)
                .await;
        }

        let db = self.runtime.db.clone();
        {
            let key = path.to_string();
            blocking(move || db.delete_project(&key)).await?;
        }

        if delete_files && repo.is_dir() {
            if let Err(e) = remove_dir_all_bg(&repo).await {
                tracing::warn!("could not delete {}: {e}", repo.display());
            }
        }

        if git_backed {
            self.runtime.config.invalidate().await;
            self.runtime.rebuild_thoughts_index().await;
        }
        Ok(())
    }

    pub async fn reorder(&self, paths: Vec<String>) -> Result<()> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        blocking(move || db.reorder_projects(&paths)).await
    }

    pub async fn profiles(&self) -> Result<Vec<ThoughtsProfile>> {
        ensure_running(self.runtime)?;
        let thoughts = match self.runtime.config.load().await {
            Ok(loaded) => loaded.config.thoughts.clone(),
            Err(CoreError::ConfigMissing { .. }) => None,
            Err(e) => return Err(e),
        };
        blocking(move || {
            Ok(match thoughts {
                Some(t) => list_profiles(&t),
                None => vec![bootstrap_default_profile()],
            })
        })
        .await
    }

    pub async fn create_profile(
        &self,
        name: &str,
        thoughts_repo_hint: Option<String>,
    ) -> Result<ThoughtsProfile> {
        ensure_running(self.runtime)?;
        let name = name.trim().to_string();
        validate_profile_name(&name)?;

        let loaded = self.runtime.config.load().await?;
        if loaded
            .config
            .thoughts
            .as_ref()
            .is_some_and(|t| t.profiles.contains_key(&name))
        {
            return Err(CoreError::ProfileExists { name });
        }

        let repo = thoughts_repo_hint
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("~/{name}-thoughts"));

        let root = config::expand_path(&repo);
        if matches!(
            classify_root(&root, DEFAULT_REPOS_DIR, DEFAULT_GLOBAL_DIR),
            RootState::Scaffoldable | RootState::Adoptable
        ) {
            let git = self.runtime.binaries.require(BinaryKind::Git).await?;
            let user = loaded
                .config
                .thoughts
                .as_ref()
                .map(|t| t.user.as_str())
                .unwrap_or_default();
            thoughts_repo::scaffold(
                &git,
                &root,
                user,
                DEFAULT_REPOS_DIR,
                DEFAULT_GLOBAL_DIR,
                &self.runtime.child_env().await,
            )
            .await?;
        }

        let cwd = self
            .runtime
            .config
            .path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        HyprCli::new(&self.runtime.binaries)
            .thoughts_profile_create(&cwd, &name, &repo, DEFAULT_REPOS_DIR, DEFAULT_GLOBAL_DIR)
            .await?;

        self.runtime.config.invalidate().await;

        let loaded = self.runtime.config.load().await?;
        let thoughts = loaded
            .config
            .thoughts
            .clone()
            .ok_or(CoreError::ThoughtsNotConfigured)?;
        blocking(move || Ok(get_profile(&thoughts, &name)))
            .await?
            .ok_or_else(|| {
                CoreError::Io(std::io::Error::other(
                    "profile missing from config after CLI create",
                ))
            })
    }

    pub async fn sync_thoughts(&self, req: SyncThoughtsRequest) -> Result<SyncThoughtsResult> {
        ensure_running(self.runtime)?;
        let output = HyprCli::new(&self.runtime.binaries)
            .thoughts_sync(Path::new(&req.path), req.message.as_deref())
            .await?;
        self.runtime.config.invalidate().await;
        Ok(SyncThoughtsResult { output })
    }

    pub async fn backend_health(&self, path: &str) -> Result<BackendHealth> {
        ensure_running(self.runtime)?;
        let mut checks = Vec::new();

        let git_backed = self.source_of(path).await.is_git();

        if !git_backed {
            let dir = Path::new(path);
            let exists = dir.is_dir();
            checks.push(BackendHealthCheck {
                id: "folder.exists".into(),
                label: "Project folder".into(),
                status: pass_if(exists),
                detail: Some(path.to_string()),
                remedy: (!exists).then(|| {
                    "The folder was moved or deleted. Remove the project and re-add it.".to_string()
                }),
            });
            return Ok(BackendHealth { checks });
        }

        match HyprCli::new(&self.runtime.binaries)
            .storage_info(Path::new(path))
            .await
        {
            Ok(info) => {
                checks.push(BackendHealthCheck {
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

                checks.push(BackendHealthCheck {
                    id: "backend.kind".into(),
                    label: "Storage backend".into(),
                    status: HealthStatus::Pass,
                    detail: Some(info.backend.clone()),
                    remedy: None,
                });
            }
            Err(err) => {
                checks.push(BackendHealthCheck {
                    id: "project.mapped".into(),
                    label: "Project mapping".into(),
                    status: HealthStatus::Fail,
                    detail: Some(err.to_string()),
                    remedy: err.remedy(),
                });
            }
        }

        checks.extend(self.thoughts_health_checks(path).await);

        Ok(BackendHealth { checks })
    }

    async fn thoughts_health_checks(&self, path: &str) -> Vec<BackendHealthCheck> {
        let Ok(loaded) = self.runtime.config.load().await else {
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
            checks.push(BackendHealthCheck {
                id: "thoughts.root".into(),
                label: "Thoughts storage root".into(),
                status: pass_if(exists),
                detail: Some(root.display().to_string()),
                remedy: (!exists).then(|| "The configured thoughts directory does not exist. Re-run `hyprlayer thoughts init` or fix the path in the hyprlayer config.".into()),
            });
        }

        let repo = Path::new(path);
        let repo_thoughts = repo.join(crate::worktree::THOUGHTS_DIR);
        let broken = thoughts_links::broken_thoughts_links(repo, &effective);
        let links_ok = broken.is_empty();
        checks.push(BackendHealthCheck {
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

    pub async fn worktree_files(&self, path: &str) -> Result<Vec<String>> {
        ensure_running(self.runtime)?;
        let path = path.to_string();
        blocking(move || repo_settings::read(Path::new(&path)).map(|s| s.worktree.copy)).await
    }

    pub async fn set_worktree_files(
        &self,
        path: &str,
        patterns: Vec<String>,
    ) -> Result<Vec<String>> {
        ensure_running(self.runtime)?;
        let cleaned: Vec<String> = patterns
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let p = path.to_string();
        let saved = cleaned.clone();
        blocking(move || repo_settings::set_worktree_copy(Path::new(&p), &cleaned)).await?;
        Ok(saved)
    }

    pub async fn check_worktree_files(&self, path: &str) -> Result<SeedReport> {
        ensure_running(self.runtime)?;
        let path = path.to_string();
        blocking(move || {
            let repo = Path::new(&path);
            let patterns = repo_settings::read(repo)
                .map(|s| s.worktree.copy)
                .unwrap_or_default();
            Ok(seed::check_worktree_patterns(repo, &patterns))
        })
        .await
    }

    pub async fn ai_status(&self) -> Result<AiStatus> {
        ensure_running(self.runtime)?;
        let cwd = self.runtime.environment.home_dir.clone();
        HyprCli::new(&self.runtime.binaries).ai_status(&cwd).await
    }

    pub fn probe_default_thoughts(&self) -> ThoughtsProbe {
        let path = self.runtime.environment.home_dir.join("thoughts");
        ThoughtsProbe {
            exists: path.exists(),
            is_git_repo: repos::is_git_repo(&path),
            default_path: path.to_string_lossy().into_owned(),
        }
    }

    pub async fn run_onboarding(&self, req: RunOnboardingRequest) -> Result<BootstrapResult> {
        ensure_running(self.runtime)?;
        let RunOnboardingRequest { user, source } = req;
        let user = user.trim();
        if user.is_empty() {
            return Err(CoreError::BootstrapUserRequired);
        }
        let resolved: PathBuf = match &source {
            ThoughtsSource::Clone { url, location } => {
                let dest = match location.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
                    Some(loc) => thoughts_location(loc)?,
                    None => self.runtime.environment.home_dir.join("thoughts"),
                };
                let git = self.runtime.binaries.require(BinaryKind::Git).await?;
                repos::clone_repo(&git, url, &dest, &self.runtime.child_env().await).await?;
                dest
            }
            ThoughtsSource::Disk { path } => {
                let p = thoughts_location(path)?;
                if !repos::is_git_repo(&p) {
                    return Err(CoreError::NotAGitRepo {
                        path: p.display().to_string(),
                    });
                }
                p
            }
            ThoughtsSource::Create { path } => {
                let p = thoughts_location(path)?;
                let git = self.runtime.binaries.require(BinaryKind::Git).await?;
                thoughts_repo::scaffold(
                    &git,
                    &p,
                    user,
                    DEFAULT_REPOS_DIR,
                    DEFAULT_GLOBAL_DIR,
                    &self.runtime.child_env().await,
                )
                .await?;
                p
            }
        };

        let thoughts_repo_str = resolved.to_string_lossy().into_owned();
        config::ensure_thoughts_config(self.runtime.config.path(), user, &thoughts_repo_str)?;

        self.runtime.config.invalidate().await;

        self.relink_mapped_checkouts().await;

        self.runtime.config.invalidate().await;
        let stuck = self.runtime.config.load().await.is_ok_and(|loaded| {
            loaded
                .config
                .thoughts
                .as_ref()
                .and_then(|t| t.backend.thoughts_root())
                .is_some_and(|root| root == resolved)
        });
        if !stuck {
            tracing::warn!(
                "thoughts repoint was clobbered by a concurrent config writer; re-applying"
            );
            config::ensure_thoughts_config(self.runtime.config.path(), user, &thoughts_repo_str)?;
            self.runtime.config.invalidate().await;
            self.relink_mapped_checkouts().await;
            self.runtime.config.invalidate().await;
        }

        self.runtime.rebuild_thoughts_index().await;

        self.bootstrap().await
    }

    async fn relink_mapped_checkouts(&self) {
        let Ok(loaded) = self.runtime.config.load().await else {
            return;
        };
        let Some(thoughts) = loaded.config.thoughts.as_ref() else {
            return;
        };
        let cli = HyprCli::new(&self.runtime.binaries);
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
                tracing::warn!(
                    "thoughts re-init at {path_str} failed ({init_err}); relinking manually"
                );
            }
            if !thoughts_links::broken_thoughts_links(path, &effective).is_empty() {
                if let Err(e) = thoughts_links::provision_thoughts(path, &effective) {
                    tracing::warn!("manual relink at {path_str} failed: {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_lists_a_directory_and_flags_a_git_repo() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("plain")).unwrap();
        std::fs::create_dir_all(root.path().join("repo/.git")).unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        std::fs::write(root.path().join("readme.md"), "hi").unwrap();

        let listing = list_directory(root.path());

        // list_directory itself does not canonicalize — browse() does that
        // before calling it — so this is the raw path handed in.
        assert_eq!(listing.path, root.path().display().to_string());
        assert_eq!(
            listing.parent.as_deref(),
            root.path()
                .parent()
                .map(|p| p.display().to_string())
                .as_deref()
        );
        // The top level's own .git is excluded, so three entries, not four.
        assert_eq!(listing.entries.len(), 3);

        let by_name = |name: &str| listing.entries.iter().find(|e| e.name == name).unwrap();
        assert!(by_name("plain").is_dir);
        assert!(!by_name("plain").is_repo);
        assert!(by_name("repo").is_dir);
        assert!(by_name("repo").is_repo);
        assert!(!by_name("readme.md").is_dir);
        assert!(!by_name("readme.md").is_repo);
    }
}
