use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprlayer_core::branch_prefix::{self, BranchPrefix, PrefixSource};
use hyprlayer_core::gitops::{CommitEntry, DirEntry, FileSides, GitFileEntry, WorktreeGitStatus};
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::pr::{self, MergeMethod, PrStatus, PrSummary};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::repo_settings;
use hyprlayer_core::scripts::ScriptKind;
use hyprlayer_core::skills::SlashCommand;
use hyprlayer_core::store::{
    NewWorkstream, Workstream, WorkstreamDoc, WorkstreamStatus, WorktreeMode,
};
use hyprlayer_core::worktree::thoughts_links::{self, LinkStatus};
use hyprlayer_core::worktree::{
    self, archive_dir_for, CreateMode, CreateSpec, PullOutcome, RemovePolicy, WorktreeStatus,
};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use super::sessions::kill_and_await_exit;
use super::{blocking, db_task, effective_for};
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamView {
    pub item: Workstream,
    pub git: WorktreeStatus,
    pub seed: Option<hyprlayer_core::worktree::seed::SeedReport>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamBases {
    pub branches: Vec<String>,
    pub local_branches: Vec<String>,
    pub remote_branches: Vec<String>,
    pub default_base: String,
    pub current_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBranchPrefixes {
    pub prefixes: Vec<BranchPrefix>,
    pub default_prefix: String,
    pub repo: Vec<BranchPrefix>,
    pub source: PrefixSource,
}

fn item_not_found(id: &str) -> AppError {
    AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {id}"),
        remedy: None,
    }
}

async fn load_item(state: &AppState, id: &str) -> CmdResult<Workstream> {
    let id_owned = id.to_string();
    db_task(state.db.clone(), move |db| db.get_workstream(&id_owned))
        .await?
        .ok_or_else(|| item_not_found(id))
}

fn pr_dir(item: &Workstream) -> &Path {
    let worktree = Path::new(&item.worktree_path);
    if worktree.is_dir() {
        worktree
    } else {
        Path::new(&item.project_path)
    }
}

fn item_view(item: Workstream, link: Option<LinkStatus>) -> WorkstreamView {
    let dir = Path::new(&item.worktree_path);
    let git = if item.is_folder() {
        worktree::folder_status(dir)
    } else {
        worktree::status(dir, &item.base_branch, link)
    };
    WorkstreamView {
        item,
        git,
        seed: None,
    }
}

async fn status_batch<F>(items: Vec<Workstream>, link_for: F) -> CmdResult<Vec<WorkstreamView>>
where
    F: Fn(&Workstream) -> Option<LinkStatus> + Send + Sync + 'static,
{
    let link_for = Arc::new(link_for);
    let handles: Vec<_> = items
        .into_iter()
        .map(|item| {
            let link_for = link_for.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let link = link_for(&item);
                item_view(item, link)
            })
        })
        .collect();
    let mut views = Vec::with_capacity(handles.len());
    for handle in handles {
        views.push(
            handle
                .await
                .map_err(|e| AppError::internal(format!("status task: {e}")))?,
        );
    }
    Ok(views)
}

async fn adopt_head_branches(
    app: &AppHandle,
    db: Arc<hyprlayer_core::store::Db>,
    mut views: Vec<WorkstreamView>,
) -> Vec<WorkstreamView> {
    let changes: Vec<(usize, String, String)> = views
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            let next = v.item.branch_to_adopt(v.git.branch.as_deref())?;
            Some((i, v.item.branch.clone(), next.to_string()))
        })
        .collect();
    if changes.is_empty() {
        return views;
    }

    let writes: Vec<(String, String)> = changes
        .iter()
        .map(|(i, _, next)| (views[*i].item.id.clone(), next.clone()))
        .collect();
    if let Err(e) = db_task(db, move |db| {
        for (id, branch) in &writes {
            db.update_workstream_branch(id, branch)?;
        }
        Ok(())
    })
    .await
    {
        log::warn!(
            "branch adoption write failed, retrying next sweep: {}",
            e.message
        );
        return views;
    }

    for (i, previous, next) in changes {
        log::info!(
            "workstream {} adopted branch {next} (was {previous})",
            views[i].item.id
        );
        let _ = crate::events::WorkstreamBranchChanged {
            workstream_id: views[i].item.id.clone(),
            branch: next.clone(),
            previous,
        }
        .emit(app);
        views[i].item.branch = next;
        views[i].item.pr_number = None;
    }
    views
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstreams(
    app: AppHandle,
    state: State<'_, AppState>,
    project_path: String,
    include_archived: bool,
) -> CmdResult<Vec<WorkstreamView>> {
    let items = db_task(state.db.clone(), move |db| {
        db.list_workstreams(&project_path, include_archived)
    })
    .await?;

    let effective = if let Some(first) = items.first() {
        Some(effective_for(&state, &first.project_path).await?)
    } else {
        None
    };
    let views = status_batch(items, move |item| {
        effective
            .as_ref()
            .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff))
    })
    .await?;
    Ok(adopt_head_branches(&app, state.db.clone(), views).await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_all_workstreams(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Vec<WorkstreamView>> {
    let items = db_task(state.db.clone(), |db| db.list_active_workstreams()).await?;
    let views = status_batch(items, |_| None).await?;
    Ok(adopt_head_branches(&app, state.db.clone(), views).await)
}

#[tauri::command]
#[specta::specta]
pub async fn watch_worktree(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workstream_id: Option<String>,
) -> CmdResult<()> {
    let mut guard = state.worktree_watch.lock().await;
    let Some(workstream_id) = workstream_id else {
        *guard = None;
        return Ok(());
    };
    if guard.as_ref().is_some_and(|(id, _)| id == &workstream_id) {
        return Ok(());
    }
    *guard = None;

    let ws = load_item(&state, &workstream_id).await?;
    let path = PathBuf::from(&ws.worktree_path);
    if !path.is_dir() {
        return Ok(());
    }

    let app = app.clone();
    let wsid = workstream_id.clone();
    let handle = blocking("watch_worktree", move || {
        let git_dir = hyprlayer_core::gitops::git_dir(&path).unwrap_or_else(|| path.join(".git"));
        hyprlayer_core::index::watcher::watch_worktree(
            &path,
            &git_dir,
            Box::new(move || {
                let _ = crate::events::WorktreeChanged {
                    workstream_id: wsid.clone(),
                }
                .emit(&app);
            }),
        )
    })
    .await?
    .map_err(AppError::from)?;

    *guard = Some((workstream_id, handle));
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_bases(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<WorkstreamBases> {
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let repo = PathBuf::from(&project_path);
    let default_base = worktree::detect_base_branch(&git, &repo)
        .await
        .unwrap_or_default();
    let (branches, local_branches, remote_branches, current_branch) =
        blocking("list_branches", move || {
            let branches = hyprlayer_core::gitops::list_branches(&repo)?;
            let local_branches = hyprlayer_core::gitops::list_local_branches(&repo)?;
            let remote_branches = hyprlayer_core::gitops::list_remote_branches(&repo)?
                .into_iter()
                .filter(|r| match hyprlayer_core::gitops::local_name_for_remote(r) {
                    Some(local) => !local_branches.iter().any(|l| l == local),
                    None => false,
                })
                .collect();
            let current_branch = hyprlayer_core::gitops::current_branch(&repo);
            Ok::<_, CoreError>((branches, local_branches, remote_branches, current_branch))
        })
        .await?
        .map_err(AppError::from)?;
    Ok(WorkstreamBases {
        branches,
        local_branches,
        remote_branches,
        default_base,
        current_branch,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_project_remote(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<()> {
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    worktree::fetch_remote(&git, Path::new(&project_path))
        .await
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn discard_project_changes(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<u32> {
    if !super::projects::project_source(&state, &project_path)
        .await
        .is_git()
    {
        return Err(AppError {
            code: "invalid_mode".into(),
            message: "This project is a plain folder, so it has no git changes to discard.".into(),
            remedy: None,
        });
    }
    blocking("discard_project_changes", move || {
        hyprlayer_core::gitops::discard_tracked_changes(Path::new(&project_path))
    })
    .await?
    .map_err(AppError::from)
}

struct AdoptedBranch {
    branch: String,
    created: bool,
}

async fn adopt_branch(
    state: &AppState,
    project_path: &str,
    branch: String,
) -> CmdResult<AdoptedBranch> {
    let repo = PathBuf::from(project_path);
    let probe = repo.clone();
    let (locals, remotes) = blocking("list_branches_for_adopt", move || {
        let locals = hyprlayer_core::gitops::list_local_branches(&probe)?;
        let remotes = hyprlayer_core::gitops::list_remote_branches(&probe)?;
        Ok::<_, CoreError>((locals, remotes))
    })
    .await?
    .map_err(AppError::from)?;

    if locals.contains(&branch) {
        return Ok(AdoptedBranch {
            branch,
            created: false,
        });
    }

    if remotes.contains(&branch) {
        let local = hyprlayer_core::gitops::local_name_for_remote(&branch)
            .unwrap_or(branch.as_str())
            .to_string();
        if locals.contains(&local) {
            return Ok(AdoptedBranch {
                branch: local,
                created: false,
            });
        }
        let git = state
            .binaries
            .require(BinaryKind::Git)
            .await
            .map_err(AppError::from)?;
        let _ = worktree::fetch_remote(&git, &repo).await;
        worktree::create_tracking_branch(&git, &repo, &local, &branch)
            .await
            .map_err(AppError::from)?;
        return Ok(AdoptedBranch {
            branch: local,
            created: true,
        });
    }

    Err(AppError {
        code: "invalid_branch".into(),
        message: format!("`{branch}` is not a local or remote branch."),
        remedy: Some("Pick a branch from the list, or create a new one.".into()),
    })
}

async fn create_folder_workstream(
    state: &AppState,
    project_path: String,
    title: String,
) -> CmdResult<WorkstreamView> {
    let dir = PathBuf::from(&project_path);
    if !dir.is_dir() {
        return Err(AppError {
            code: "folder_missing".into(),
            message: format!("{project_path} is no longer a directory."),
            remedy: Some(
                "The folder was moved or deleted. Remove the project and re-add it.".into(),
            ),
        });
    }

    let new_item = NewWorkstream {
        slug: worktree::unique_slug(&title),
        title,
        branch: String::new(),
        base_branch: String::new(),
        worktree_path: project_path.clone(),
        project_path,
        worktree_mode: WorktreeMode::Folder,
    };
    let item = db_task(state.db.clone(), move |db| db.insert_workstream(new_item)).await?;
    Ok(item_view(item, None))
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn create_workstream(
    app: AppHandle,
    state: State<'_, AppState>,
    project_path: String,
    title: String,
    base: Option<String>,
    mode: WorktreeMode,
    branch: Option<String>,
    branch_prefix: Option<String>,
) -> CmdResult<WorkstreamView> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(AppError {
            code: "invalid_title".into(),
            message: "Workstreams need a title.".into(),
            remedy: None,
        });
    }

    if !super::projects::project_source(&state, &project_path)
        .await
        .is_git()
    {
        return create_folder_workstream(&state, project_path, title).await;
    }

    let picked = branch
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());
    let (branch_choice, branch_is_ours) = match picked {
        Some(b) => {
            let adopted = adopt_branch(&state, &project_path, b).await?;
            (Some(adopted.branch), adopted.created)
        }
        None => (None, false),
    };

    let needs_branch = |m: &str| {
        branch_choice.clone().ok_or_else(|| AppError {
            code: "invalid_branch".into(),
            message: format!("Pick a branch to start the {m} workstream from."),
            remedy: None,
        })
    };
    let create_mode = match mode {
        WorktreeMode::Isolated => CreateMode::Isolated,
        WorktreeMode::Existing => CreateMode::Existing {
            branch: needs_branch("existing-branch")?,
        },
        WorktreeMode::Inplace => CreateMode::InPlace {
            branch: needs_branch("in-place")?,
        },
        WorktreeMode::Folder => {
            return Err(AppError {
                code: "invalid_mode".into(),
                message: "This project is a git repository, so its workstreams need a branch."
                    .into(),
                remedy: Some("Pick a branch, or create a new one.".into()),
            })
        }
    };
    let inplace = mode == WorktreeMode::Inplace;

    if inplace {
        let pp = project_path.clone();
        let active = db_task(state.db.clone(), move |db| db.list_workstreams(&pp, false)).await?;
        if active.iter().any(|w| w.is_inplace()) {
            return Err(AppError {
                code: "inplace_exists".into(),
                message: "This project already has an in-place workstream.".into(),
                remedy: Some(
                    "Archive it first, or create the workstream in a dedicated worktree.".into(),
                ),
            });
        }
    }

    let effective = effective_for(&state, &project_path).await?;
    let mapped = effective
        .mapped_name
        .clone()
        .ok_or_else(|| AppError::project_not_found(&project_path))?;

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let picked = branch_prefix
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let branch_prefix = match picked {
        Some(p) => p,
        None => state.settings.get().await.git.effective_branch_prefix(),
    };
    if matches!(create_mode, CreateMode::Isolated) {
        hyprlayer_core::branch_prefix::validate_prefix(&branch_prefix).map_err(AppError::from)?;
    }
    let base_override = base.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
    let created = state
        .worktrees
        .create(
            &git,
            &CreateSpec {
                project_repo: PathBuf::from(&project_path),
                mapped_name: mapped.clone(),
                title: title.clone(),
                base_override,
                branch_prefix,
                mode: create_mode,
            },
        )
        .await
        .map_err(AppError::from)?;

    let worktree_path = PathBuf::from(&created.path);

    let (link, seed) = if inplace {
        let link = thoughts_links::check_thoughts(&worktree_path, &effective);
        (link, None)
    } else {
        let cli_init = HyprCli::new(&state.binaries)
            .thoughts_init(&worktree_path, &mapped, effective.profile_name.as_deref())
            .await;
        let provision = match cli_init {
            Ok(_) => Ok(thoughts_links::check_thoughts(&worktree_path, &effective)),
            Err(init_err) => {
                log::warn!("thoughts init in worktree failed ({init_err}); provisioning manually");
                (|| -> Result<LinkStatus, CoreError> {
                    let link = thoughts_links::provision_thoughts(&worktree_path, &effective)?;
                    thoughts_links::add_worktree_mapping(
                        state.config.path(),
                        &worktree_path,
                        &mapped,
                        effective.profile_name.as_deref(),
                    )?;
                    Ok(link)
                })()
            }
        };
        let link = match provision {
            Ok(link) => link,
            Err(e) => {
                let _ = state
                    .worktrees
                    .remove(
                        &git,
                        Path::new(&project_path),
                        &worktree_path,
                        &created.branch,
                        &created.base_branch,
                        RemovePolicy::ForceDiscard,
                        mode == WorktreeMode::Isolated || branch_is_ours,
                    )
                    .await;
                return Err(AppError::from(e));
            }
        };
        state.config.invalidate().await;

        let seed = {
            let repo = PathBuf::from(&project_path);
            let wt = worktree_path.clone();
            blocking("seed_worktree", move || {
                let patterns = hyprlayer_core::repo_settings::read(&repo)
                    .map(|s| s.worktree.copy)
                    .unwrap_or_else(|e| {
                        log::warn!("read .hyprlayer/settings.toml: {e}");
                        Vec::new()
                    });
                hyprlayer_core::worktree::seed::seed_worktree(&repo, &wt, &patterns)
            })
            .await
            .unwrap_or_default()
        };
        if !seed.errors.is_empty() || !seed.missing.is_empty() {
            log::warn!(
                "worktree seed: copied {}, missing {:?}, errors {:?}",
                seed.copied.len(),
                seed.missing,
                seed.errors
            );
        } else if !seed.copied.is_empty() {
            log::info!("worktree seed: copied {} file(s)", seed.copied.len());
        }
        (link, Some(seed))
    };

    let new_item = NewWorkstream {
        project_path,
        title,
        slug: created.slug.clone(),
        branch: created.branch.clone(),
        worktree_path: created.path.clone(),
        base_branch: created.base_branch.clone(),
        worktree_mode: mode,
    };
    let item = db_task(state.db.clone(), move |db| db.insert_workstream(new_item)).await?;

    // TODO: Gate setup scripts on repository trust.
    if !inplace {
        super::scripts::run_lifecycle_script(&app, &state, &item, ScriptKind::Setup).await;
    }

    let mut view = item_view(item, Some(link));
    view.seed = seed;
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_prefixes(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ResolvedBranchPrefixes> {
    super::ensure_registered_project(&state, &project_path).await?;
    let git = state.settings.get().await.git;
    let repo_dir = PathBuf::from(&project_path);
    let settings = blocking("read_branch_prefixes", move || {
        repo_settings::read(&repo_dir)
    })
    .await?
    .map_err(AppError::from)?;
    let repo = settings.branches.prefixes;
    let global = git.prefixes.as_deref();
    let prefixes = branch_prefix::resolve_prefixes(global, &repo)
        .into_iter()
        .map(BranchPrefix::with_effective_keywords)
        .collect();
    Ok(ResolvedBranchPrefixes {
        prefixes,
        default_prefix: git.effective_branch_prefix(),
        source: branch_prefix::prefix_source(global, &repo),
        repo,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_branch_prefixes(
    state: State<'_, AppState>,
    project_path: String,
    prefixes: Vec<BranchPrefix>,
) -> CmdResult<Vec<BranchPrefix>> {
    super::ensure_registered_project(&state, &project_path).await?;
    let cleaned = super::normalize_branch_prefixes(prefixes);
    branch_prefix::validate_prefixes(&cleaned).map_err(AppError::from)?;
    let repo = PathBuf::from(&project_path);
    let saved = cleaned.clone();
    blocking("write_branch_prefixes", move || {
        repo_settings::set_branch_prefixes(&repo, &cleaned)
    })
    .await?
    .map_err(AppError::from)?;
    Ok(saved)
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workstream(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> CmdResult<()> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.update_workstream_title(&id, title.trim())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_workstream_base_branch(
    state: State<'_, AppState>,
    id: String,
    base: String,
) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    let base = base.trim().to_string();
    if base.is_empty() {
        return Err(AppError::internal("base branch cannot be empty"));
    }
    if let Ok(gh) = state.binaries.require(BinaryKind::Gh).await {
        let dir = pr_dir(&item);
        if let Ok(Some(status)) = pr::pr_status(&gh, dir, &item.pr_ref()).await {
            if status.state == pr::PrState::Open {
                pr::edit_pr_base(&gh, dir, &item.pr_ref(), &base)
                    .await
                    .map_err(AppError::from)?;
            }
        }
    }
    db_task(state.db.clone(), move |db| {
        db.update_workstream_base_branch(&id, &base)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_workstream_from_base(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<PullOutcome> {
    let item = load_item(&state, &id).await?;
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    worktree::pull_base(&git, Path::new(&item.worktree_path), &item.base_branch)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_workstream_remote(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    worktree::fetch_remote(&git, Path::new(&item.worktree_path))
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_workstream_stage(
    state: State<'_, AppState>,
    id: String,
    stage: Stage,
) -> CmdResult<()> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.update_workstream_stage(&id, stage)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn attach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
    role: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.attach_workstream_doc(&id, &doc_path, &role)?;
        db.workstream_docs(&id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.detach_workstream_doc(&id, &doc_path)?;
        db.workstream_docs(&id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_docs(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    db_task(state.db.clone(), move |db| db.workstream_docs(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn archive_workstream(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    skip_scripts: bool,
) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;

    let sessions = {
        let id = id.clone();
        db_task(state.db.clone(), move |db| db.list_sessions(&id))
            .await
            .unwrap_or_default()
    };
    for session in &sessions {
        kill_and_await_exit(&state, &session.id).await;
    }

    state.terminals.remove_for_workstream(&id).await;
    state.scripts.clear_workstream(&id).await;

    if item.shares_project_dir() {
        return db_task(state.db.clone(), move |db| db.archive_workstream(&id)).await;
    }

    if !skip_scripts
        && matches!(
            super::scripts::run_lifecycle_script(&app, &state, &item, ScriptKind::Archive).await,
            Some(crate::script_registry::RunPhase::Failed)
        )
    {
        return Err(AppError {
            code: "archive_script_failed".into(),
            message: "The Archive script failed. Fix it and retry, or skip it to archive anyway."
                .into(),
            remedy: Some("Re-run archive, or choose Skip to archive without the script.".into()),
        });
    }

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let project_path = Path::new(&item.project_path);
    let worktree_path = Path::new(&item.worktree_path);
    let archive_path = archive_dir_for(project_path, &item.slug);
    let live = worktree::is_live_worktree(worktree_path);

    if state.settings.get().await.git.delete_branch_on_archive && item.app_owns_branch() {
        if let Ok(gh) = state.binaries.require(BinaryKind::Gh).await {
            if let Ok(Some(status)) = pr::pr_status(&gh, pr_dir(&item), &item.pr_ref()).await {
                if status.state == pr::PrState::Merged {
                    if let Err(e) =
                        worktree::delete_remote_branch(&git, project_path, &item.branch).await
                    {
                        log::warn!(
                            "delete remote branch {} on archive failed: {e}",
                            item.branch
                        );
                    }
                }
            }
        }
    }

    let mut unmapped = false;
    if live {
        match HyprCli::new(&state.binaries)
            .thoughts_uninit(worktree_path)
            .await
        {
            Ok(_) => unmapped = true,
            Err(e) => log::warn!("thoughts uninit in worktree failed: {e}"),
        }
    }

    if live && state.settings.get().await.git.prune_ignored_on_archive {
        let keep = hyprlayer_core::repo_settings::read(project_path)
            .map(|s| s.worktree.copy)
            .unwrap_or_default();
        if let Err(e) = state
            .worktrees
            .clean_ignored(&git, worktree_path, &keep)
            .await
        {
            log::warn!("prune ignored files before archive failed: {e}");
        }
    }

    let moved = state
        .worktrees
        .archive(&git, project_path, worktree_path, &archive_path)
        .await
        .map_err(AppError::from)?;
    if moved {
        if let Err(e) = worktree::ensure_archive_excluded(project_path) {
            log::warn!("could not write .git/info/exclude for archive: {e}");
        }
    }

    if !unmapped {
        if let Err(e) = thoughts_links::remove_worktree_mapping(state.config.path(), worktree_path)
        {
            log::warn!("could not remove worktree mapping: {e}");
        }
    }
    state.config.invalidate().await;

    db_task(state.db.clone(), move |db| db.archive_workstream(&id)).await
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedWorkstreamView {
    pub item: Workstream,
    pub archive_path: String,
    pub archive_exists: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn list_archived_workstreams(
    state: State<'_, AppState>,
) -> CmdResult<Vec<ArchivedWorkstreamView>> {
    let items = db_task(state.db.clone(), |db| db.list_archived_workstreams()).await?;
    blocking("list_archived", move || {
        items
            .into_iter()
            .map(|item| {
                let archive_path = archive_dir_for(Path::new(&item.project_path), &item.slug);
                let archive_exists = item.shares_project_dir() || archive_path.exists();
                ArchivedWorkstreamView {
                    archive_path: archive_path.display().to_string(),
                    archive_exists,
                    item,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn restore_workstream(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorkstreamView> {
    let item = load_item(&state, &id).await?;
    if item.status != WorkstreamStatus::Archived {
        return Err(AppError {
            code: "not_archived".into(),
            message: format!("{} is not archived.", item.title),
            remedy: None,
        });
    }

    if item.shares_project_dir() {
        let link = if item.is_folder() {
            None
        } else {
            let effective = effective_for(&state, &item.project_path).await?;
            Some(thoughts_links::check_thoughts(
                Path::new(&item.worktree_path),
                &effective,
            ))
        };
        let id_for_db = id.clone();
        db_task(state.db.clone(), move |db| {
            db.unarchive_workstream(&id_for_db)
        })
        .await?;
        let item = load_item(&state, &id).await?;
        return Ok(item_view(item, link));
    }

    let project_path = Path::new(&item.project_path);
    let worktree_path = Path::new(&item.worktree_path);
    let archive_path = archive_dir_for(project_path, &item.slug);

    if !archive_path.exists() {
        return Err(AppError {
            code: "archive_missing".into(),
            message: format!(
                "The backup directory for this workstream is gone ({}).",
                archive_path.display()
            ),
            remedy: Some("Delete the archived row to remove it from the list.".into()),
        });
    }
    if worktree_path.exists() {
        return Err(AppError {
            code: "restore_target_occupied".into(),
            message: format!(
                "Another workstream took the original path ({}).",
                worktree_path.display()
            ),
            remedy: Some("Move or archive the workstream at that path, then try again.".into()),
        });
    }

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    state
        .worktrees
        .restore(&git, project_path, &archive_path, worktree_path)
        .await
        .map_err(AppError::from)?;

    let effective = effective_for(&state, &item.project_path).await?;
    let link = match thoughts_links::provision_thoughts(worktree_path, &effective) {
        Ok(link) => Some(link),
        Err(e) => {
            log::warn!("thoughts re-provision after restore failed: {e}");
            None
        }
    };
    if let Some(mapped) = effective.mapped_name.as_deref() {
        if let Err(e) = thoughts_links::add_worktree_mapping(
            state.config.path(),
            worktree_path,
            mapped,
            effective.profile_name.as_deref(),
        ) {
            log::warn!("could not re-add worktree mapping after restore: {e}");
        }
    }
    state.config.invalidate().await;

    let id_for_db = id.clone();
    db_task(state.db.clone(), move |db| {
        db.unarchive_workstream(&id_for_db)
    })
    .await?;

    let item = load_item(&state, &id).await?;
    Ok(item_view(item, link))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_archived_workstream(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    if item.status != WorkstreamStatus::Archived {
        return Err(AppError {
            code: "not_archived".into(),
            message: format!("{} is not archived.", item.title),
            remedy: Some("Archive it first.".into()),
        });
    }
    if item.is_folder() {
        return db_task(state.db.clone(), move |db| db.delete_workstream(&id)).await;
    }
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let project_path = Path::new(&item.project_path);
    let archive_path = archive_dir_for(project_path, &item.slug);
    let delete_branch =
        state.settings.get().await.git.delete_branch_on_archive && item.app_owns_branch();

    state
        .worktrees
        .purge_archive(
            &git,
            project_path,
            &archive_path,
            &item.branch,
            delete_branch,
        )
        .await
        .map_err(AppError::from)?;

    db_task(state.db.clone(), move |db| db.delete_workstream(&id)).await
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupReport {
    pub deleted: u32,
    pub failed: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn cleanup_archived_workstreams(
    state: State<'_, AppState>,
    before_ms: Option<f64>,
) -> CmdResult<ArchiveCleanupReport> {
    let items = db_task(state.db.clone(), |db| db.list_archived_workstreams()).await?;
    let items: Vec<_> = items
        .into_iter()
        .filter(|w| before_ms.is_none_or(|cut| w.archived_at.is_some_and(|a| a < cut)))
        .collect();

    let git = if items.iter().any(|w| !w.is_folder()) {
        Some(
            state
                .binaries
                .require(BinaryKind::Git)
                .await
                .map_err(AppError::from)?,
        )
    } else {
        None
    };
    let delete_branch_setting = state.settings.get().await.git.delete_branch_on_archive;

    let mut deleted = 0u32;
    let mut failed = 0u32;

    for w in items {
        let id = w.id.clone();

        if !w.is_folder() {
            let git = git.as_ref().expect("required above for non-folder rows");
            let project_path = PathBuf::from(&w.project_path);
            let archive_path = archive_dir_for(&project_path, &w.slug);
            let delete_branch = delete_branch_setting && w.app_owns_branch();

            if let Err(e) = state
                .worktrees
                .purge_archive(git, &project_path, &archive_path, &w.branch, delete_branch)
                .await
            {
                log::warn!("cleanup_archived_workstreams: purge {id} failed: {e}");
                failed += 1;
                continue;
            }
        }

        match db_task(state.db.clone(), move |db| db.delete_workstream(&id)).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                log::warn!("cleanup_archived_workstreams: db delete failed: {e:?}");
                failed += 1;
            }
        }
    }

    Ok(ArchiveCleanupReport { deleted, failed })
}

#[tauri::command]
#[specta::specta]
pub async fn reprovision_workstream(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorkstreamView> {
    let item = load_item(&state, &id).await?;
    if item.is_folder() {
        return Err(AppError {
            code: "folder_no_thoughts".into(),
            message: "This workstream runs in a plain folder, so it has no thoughts links.".into(),
            remedy: None,
        });
    }
    let effective = effective_for(&state, &item.project_path).await?;
    let link = thoughts_links::provision_thoughts(Path::new(&item.worktree_path), &effective)
        .map_err(AppError::from)?;
    Ok(item_view(item, Some(link)))
}

const WORKTREE_GONE: &str = "The worktree may have been deleted; archive this workstream.";

async fn with_worktree_base_remedy<T, F>(
    state: &State<'_, AppState>,
    id: &str,
    op: &'static str,
    remedy: &'static str,
    f: F,
) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path, &str) -> Result<T, CoreError> + Send + 'static,
{
    let item = load_item(state, id).await?;
    blocking(op, move || {
        f(Path::new(&item.worktree_path), &item.base_branch)
    })
    .await?
    .map_err(|e| AppError {
        code: op.to_string(),
        message: e.to_string(),
        remedy: Some(remedy.into()),
    })
}

async fn with_worktree_base<T, F>(
    state: &State<'_, AppState>,
    id: &str,
    op: &'static str,
    f: F,
) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path, &str) -> Result<T, CoreError> + Send + 'static,
{
    with_worktree_base_remedy(state, id, op, WORKTREE_GONE, f).await
}

async fn with_worktree<T, F>(
    state: &State<'_, AppState>,
    id: &str,
    op: &'static str,
    f: F,
) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T, CoreError> + Send + 'static,
{
    with_worktree_base(state, id, op, move |p: &Path, _: &str| f(p)).await
}

async fn with_worktree_remedy<T, F>(
    state: &State<'_, AppState>,
    id: &str,
    op: &'static str,
    remedy: &'static str,
    f: F,
) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T, CoreError> + Send + 'static,
{
    with_worktree_base_remedy(state, id, op, remedy, move |p: &Path, _: &str| f(p)).await
}

const COMMIT_GONE: &str =
    "This commit may no longer be on the branch after a rebase or squash-merge; \
     reopen the file from the Commits tab.";

#[tauri::command]
#[specta::specta]
pub async fn worktree_git_status(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorktreeGitStatus> {
    with_worktree(&state, &id, "worktree_git_status", |p| {
        hyprlayer_core::gitops::worktree_git_status(p)
    })
    .await
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptContext {
    pub branch: String,
    pub base_branch: String,
    pub changed_files: String,
    pub diff: String,
    pub last_commit: String,
}

#[tauri::command]
#[specta::specta]
pub async fn workstream_prompt_context(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<PromptContext> {
    let item = load_item(&state, &id).await?;
    let branch = item.branch.clone();
    let base_branch = item
        .base_branch
        .strip_prefix("origin/")
        .unwrap_or(&item.base_branch)
        .to_string();
    let git = blocking("workstream_prompt_context", move || {
        hyprlayer_core::gitops::prompt_git_vars(Path::new(&item.worktree_path), true, true, true)
    })
    .await?;
    Ok(PromptContext {
        branch,
        base_branch,
        changed_files: git.changed_files,
        diff: git.diff,
        last_commit: git.last_commit,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_stage(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    with_worktree(&state, &id, "worktree_stage", move |p| {
        hyprlayer_core::gitops::stage_files(p, &paths)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_unstage(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    with_worktree(&state, &id, "worktree_unstage", move |p| {
        hyprlayer_core::gitops::unstage_files(p, &paths)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_discard(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    with_worktree(&state, &id, "worktree_discard", move |p| {
        hyprlayer_core::gitops::discard_files(p, &paths)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_file_sides(
    state: State<'_, AppState>,
    id: String,
    path: String,
    staged: bool,
) -> CmdResult<FileSides> {
    with_worktree(&state, &id, "worktree_file_sides", move |p| {
        hyprlayer_core::gitops::worktree_file_sides(p, &path, staged)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_commits(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<CommitEntry>> {
    with_worktree_base(&state, &id, "list_workstream_commits", |p, base| {
        hyprlayer_core::gitops::list_commits(p, base)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_commit_files(
    state: State<'_, AppState>,
    id: String,
    sha: String,
) -> CmdResult<Vec<GitFileEntry>> {
    with_worktree_remedy(
        &state,
        &id,
        "worktree_commit_files",
        COMMIT_GONE,
        move |p| hyprlayer_core::gitops::commit_files(p, &sha),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_commit_file_sides(
    state: State<'_, AppState>,
    id: String,
    sha: String,
    path: String,
) -> CmdResult<FileSides> {
    with_worktree_remedy(
        &state,
        &id,
        "worktree_commit_file_sides",
        COMMIT_GONE,
        move |p| hyprlayer_core::gitops::commit_file_sides(p, &sha, &path),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_list_dir(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<Vec<DirEntry>> {
    with_worktree(&state, &id, "worktree_list_dir", move |p| {
        hyprlayer_core::gitops::list_dir(p, &path)
    })
    .await
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeFile {
    pub content: String,
    pub mtime_ms: f64,
}

fn file_conflict() -> AppError {
    AppError {
        code: "file_conflict".into(),
        message: "The file changed on disk since you opened it.".into(),
        remedy: Some(
            "Reload the file to pick up the external changes, then re-apply your edits.".into(),
        ),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_read_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<WorktreeFile> {
    with_worktree(&state, &id, "worktree_read_file", move |p| {
        let content = hyprlayer_core::gitops::read_text_file(p, &path)?;
        let mtime_ms = hyprlayer_core::gitops::file_mtime_ms(p, &path)?;
        Ok(WorktreeFile { content, mtime_ms })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_write_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
    content: String,
    base_mtime_ms: f64,
) -> CmdResult<f64> {
    let item = load_item(&state, &id).await?;
    let worktree = PathBuf::from(&item.worktree_path);
    blocking("worktree_write_file", move || -> CmdResult<f64> {
        if base_mtime_ms > 0.0 {
            if let Ok(current) = hyprlayer_core::gitops::file_mtime_ms(&worktree, &path) {
                if (current - base_mtime_ms).abs() > 0.5 {
                    return Err(file_conflict());
                }
            }
        }
        hyprlayer_core::gitops::write_text_file(&worktree, &path, &content)?;
        Ok(hyprlayer_core::gitops::file_mtime_ms(&worktree, &path)?)
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn list_slash_commands(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<SlashCommand>> {
    with_worktree(&state, &id, "list_slash_commands", |p| {
        Ok(hyprlayer_core::skills::list_slash_commands(p))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_search_files(
    state: State<'_, AppState>,
    id: String,
    query: String,
    limit: u32,
) -> CmdResult<Vec<String>> {
    with_worktree(&state, &id, "worktree_search_files", move |p| {
        hyprlayer_core::gitops::search_files(p, &query, limit as usize)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn workstream_pr_status(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Option<PrStatus>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(None);
    };
    let pr_ref = item.pr_ref();
    pr::pr_status(&gh, Path::new(&item.worktree_path), &pr_ref)
        .await
        .map_err(|e| {
            log::debug!("gh pr status unavailable for {pr_ref}: {e}");
            AppError::from(e)
        })
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_open_prs(
    state: State<'_, AppState>,
    id: String,
    query: String,
) -> CmdResult<Vec<PrSummary>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(Vec::new());
    };
    let dir = pr_dir(&item);
    match pr::list_open_prs(&gh, dir, &query, 50).await {
        Ok(prs) => Ok(prs),
        Err(e) => {
            log::debug!("gh pr list unavailable for {}: {e}", item.project_path);
            Ok(Vec::new())
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn preview_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    pr_number: u32,
) -> CmdResult<Option<PrSummary>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(None);
    };
    match pr::pr_summary(&gh, pr_dir(&item), &pr_number.to_string()).await {
        Ok(summary) => Ok(summary),
        Err(e) => {
            log::debug!("gh pr preview unavailable for #{pr_number}: {e}");
            Ok(None)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn attach_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    pr_number: u32,
) -> CmdResult<Option<PrStatus>> {
    let item = load_item(&state, &id).await?;
    if item.is_folder() {
        return Err(AppError {
            code: "folder_no_git".into(),
            message: "This workstream runs in a plain folder, so it has no branches or PRs.".into(),
            remedy: None,
        });
    }
    let gh = state
        .binaries
        .require(BinaryKind::Gh)
        .await
        .map_err(AppError::from)?;
    let status = match pr::pr_status(&gh, pr_dir(&item), &pr_number.to_string()).await {
        Ok(Some(status)) => status,
        Ok(None) => return Err(attach_pr_error(pr_number, None)),
        Err(e) => return Err(attach_pr_error(pr_number, Some(e))),
    };
    db_task(state.db.clone(), move |db| {
        db.set_workstream_pr(&id, Some(pr_number))
    })
    .await?;
    Ok(Some(status))
}

fn attach_pr_error(pr_number: u32, err: Option<CoreError>) -> AppError {
    let detail = match err {
        Some(CoreError::Exec { stderr_tail, .. }) => clean_gh_stderr(&stderr_tail),
        _ => String::new(),
    };
    let message = if detail.is_empty() {
        format!("No pull request #{pr_number} found in this repository")
    } else {
        detail
    };
    AppError {
        code: "pr_not_found".into(),
        message,
        remedy: None,
    }
}

fn clean_gh_stderr(stderr: &str) -> String {
    let mut s = stderr.trim();
    s = s.strip_prefix("GraphQL: ").unwrap_or(s);
    if let Some(open) = s.rfind(" (") {
        if s.ends_with(')') && !s[open + 2..s.len() - 1].contains(' ') {
            s = s[..open].trim_end();
        }
    }
    s.to_string()
}

#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_pr(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| db.set_workstream_pr(&id, None)).await
}

#[tauri::command]
#[specta::specta]
pub async fn merge_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    method: MergeMethod,
) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    let gh = state
        .binaries
        .require(BinaryKind::Gh)
        .await
        .map_err(AppError::from)?;
    let auto = state.settings.get().await.git.automerge;
    pr::merge_pr(
        &gh,
        Path::new(&item.worktree_path),
        &item.pr_ref(),
        method,
        auto,
    )
    .await
    .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_gh_stderr_strips_graphql_decoration() {
        assert_eq!(
            clean_gh_stderr(
                "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)"
            ),
            "Could not resolve to a PullRequest with the number of 999."
        );
        assert_eq!(clean_gh_stderr("  not found  "), "not found");
        assert_eq!(
            clean_gh_stderr("failed (try again later)"),
            "failed (try again later)"
        );
    }

    #[test]
    fn attach_pr_error_uses_gh_message_then_falls_back() {
        let err = CoreError::Exec {
            program: "gh pr view".into(),
            exit: Some(1),
            stderr_tail: "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)".into(),
        };
        let app = attach_pr_error(999, Some(err));
        assert_eq!(
            app.message,
            "Could not resolve to a PullRequest with the number of 999."
        );
        assert!(!app.message.contains("gh"));
        assert_eq!(
            attach_pr_error(7, None).message,
            "No pull request #7 found in this repository"
        );
    }
}
