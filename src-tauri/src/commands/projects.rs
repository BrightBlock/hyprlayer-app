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

use super::{blocking, db_task};
use crate::error::{AppError, CmdResult};
use crate::settings::AppSettings;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// Absolute path of the app-owned clone (`~/hyprlayer/repos/<name>`).
    pub path: String,
    /// Human-facing project name (the app registry name, e.g. `brightblock.ai`).
    pub mapped_name: String,
    /// The thoughts `repoMappings` key this repo resolves to — what docs are
    /// indexed and worktrees created under. CLI-sanitized, so it can differ from
    /// `mapped_name` (`brightblock.ai` → `brightblock_ai`). Filter docs by THIS.
    pub thoughts_project: String,
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

/// The stock bodies of the built-in prompts — what Settings → Prompts shows,
/// edits against, and resets to. Owned by `hyprlayer_core::pipeline` (the
/// composer), surfaced here so the frontend never mirrors the literals.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptDefaults {
    pub commit: String,
    /// A template — `{{base_branch}}` resolves at send time.
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
    /// Stock built-in prompt bodies (see [`PromptDefaults`]).
    pub prompt_defaults: PromptDefaults,
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

/// The app's explicit project registry — NOT the thoughts config's `repoMappings`
/// (those also carry every worktree's `thoughts init` entry). Each row is enriched
/// best-effort with backend/profile; a missing/locked config yields the git default.
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
            // The CLI-sanitized `repoMappings` key, separate from the display
            // name so doc queries can filter on it. Falls back to the registry
            // name when the project isn't mapped into thoughts yet.
            let thoughts_project = eff
                .as_ref()
                .and_then(|e| e.mapped_name.clone())
                .unwrap_or_else(|| r.name.clone());
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

/// The config health summary shown during onboarding. The project list is
/// derived separately now — see [`registry_projects`].
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

/// Recursively delete a directory tree on the blocking pool — a repo clone's
/// `node_modules`/`.git`/`target` would stall a tokio worker for seconds if
/// deleted synchronously. Best-effort; the caller decides what a failure means.
async fn remove_dir_all_bg(path: &Path) {
    let path = path.to_path_buf();
    let _ = blocking("remove_dir", move || std::fs::remove_dir_all(path)).await;
}

/// Provision thoughts inside `repo`, register it as a project, and surface it.
/// `owned` = a fresh clone the app created vs. an in-place disk import. On
/// failure we roll back so no half-set-up project lingers: an owned clone is
/// deleted outright; an in-place repo (the user's working copy) is never
/// deleted — only its thoughts provisioning is undone.
async fn register_repo(
    app: &tauri::AppHandle,
    state: &AppState,
    git: &Path,
    repo: &Path,
    name: &str,
    kind: &str,
    owned: bool,
) -> CmdResult<Project> {
    if let Err(e) = HyprCli::new(&state.binaries)
        .thoughts_init(repo, name, None)
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
        source: kind.to_string(),
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

    register_repo(app, state, &git, &dest, name, kind, true).await
}

/// Import a local on-disk git repo as a project, registered *in place* — never
/// re-cloned or copied (that wastes disk and diverges from the user's working
/// copy). Worktrees are cut directly from it via `git worktree add`.
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

    // Canonicalize for a stable registered path (no trailing slash, symlinks
    // resolved) so later lookups/removals match exactly. `dunce` strips the
    // Windows `\\?\` verbatim prefix `std::fs::canonicalize` adds — it leaks
    // into project URLs and trips git/tools.
    let repo = dunce::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let name = project_name(directory_name, &repo.to_string_lossy(), &repo);
    register_repo(&app, &state, &git, &repo, &name, "disk", false).await
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

/// Remove a project from the registry; with `delete_files`, also delete its
/// directory (clones and in-place imports alike — user opts in per removal).
/// Refused while active workstreams exist: they own live worktrees cut from this
/// repo, so removal would orphan them.
#[tauri::command]
#[specta::specta]
pub async fn remove_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    delete_files: bool,
) -> CmdResult<()> {
    let repo = PathBuf::from(&path);

    // Active workstreams own live worktrees cut from this repo; removal would
    // orphan them, so refuse until they're archived.
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

    // Drop the thoughts repoMapping (best-effort; the directory may be gone).
    if repo.is_dir() {
        let _ = HyprCli::new(&state.binaries).thoughts_uninit(&repo).await;
    }

    let path_for_db = path.clone();
    db_task(state.db.clone(), move |db| db.delete_project(&path_for_db)).await?;

    // Guarded above (no active workstreams), so deleting on opt-in is safe.
    if delete_files && repo.is_dir() {
        remove_dir_all_bg(&repo).await;
    }

    state.config.invalidate().await;
    let _ = crate::events::ConfigChanged.emit(&app);
    crate::rebuild_thoughts_index(app.clone()).await;
    Ok(())
}

/// Persist the user's drag-to-reorder of the project sidebar. `paths` is the full
/// project list in its new top-to-bottom order; each project's stored `position`
/// is set to its index, so the next `get_bootstrap` returns them in this order.
#[tauri::command]
#[specta::specta]
pub async fn reorder_projects(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<()> {
    db_task(state.db.clone(), move |db| db.reorder_projects(&paths)).await?;
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
    let _ = &state; // project path comes from the arg; state kept for signature symmetry
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

/// Dry-run the project's worktree-copy patterns against its checkout so the UI
/// can flag missing copy files (e.g. a `.env` absent on disk) BEFORE a workstream
/// is created. Mirrors `create_workstream`'s seed step, minus the copy. Never
/// errors — an unreadable settings file yields an empty (clean) report.
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
    /// `add_project` canonicalizes the repo path for stable identity. On Windows
    /// `std::fs::canonicalize` returns a `\\?\` verbatim path, which leaks into
    /// project URLs and trips git/tools; `dunce::canonicalize` must strip it.
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
