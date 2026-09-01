use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprlayer_core::branch_prefix::{self, BranchPrefix, PrefixSource};
use hyprlayer_core::config::EffectiveConfig;
use hyprlayer_core::gitops::{CommitEntry, DirEntry, FileSides, GitFileEntry, WorktreeGitStatus};
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::pr::{self, MergeMethod, PrStatus, PrSummary};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::repo_settings;
use hyprlayer_core::runtime::scripts as core_scripts;
use hyprlayer_core::runtime::workstreams as core_workstreams;
use hyprlayer_core::scripts::ScriptKind;
use hyprlayer_core::skills::SlashCommand;
use hyprlayer_core::store::{Workstream, WorkstreamDoc, WorktreeMode};
use hyprlayer_core::worktree::thoughts_links::{self, LinkStatus};
use hyprlayer_core::worktree::{self, archive_dir_for, PullOutcome, WorktreeStatus};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;

use super::{blocking, effective_for};
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

async fn load_item(state: &AppState, id: &str) -> CmdResult<Workstream> {
    state
        .runtime
        .workstreams()
        .find(id)
        .await
        .map_err(AppError::from)
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
    state: &AppState,
    mut views: Vec<WorkstreamView>,
) -> Vec<WorkstreamView> {
    let observed = views
        .iter()
        .map(|v| core_workstreams::HeadBranchObservation {
            item: v.item.clone(),
            head_branch: v.git.branch.clone(),
        })
        .collect();
    let adoptions = match state
        .runtime
        .workstreams()
        .adopt_head_branches(observed)
        .await
    {
        Ok(adoptions) => adoptions,
        Err(e) => {
            log::warn!("branch adoption write failed, retrying next sweep: {e}");
            return views;
        }
    };
    for adoption in adoptions {
        if let Some(view) = views
            .iter_mut()
            .find(|v| v.item.id == adoption.workstream_id)
        {
            view.item.branch = adoption.branch;
            view.item.pr_number = None;
        }
    }
    views
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstreams(
    state: State<'_, AppState>,
    project_path: String,
    include_archived: bool,
) -> CmdResult<Vec<WorkstreamView>> {
    let items = state
        .runtime
        .workstreams()
        .list(core_workstreams::ListWorkstreamsRequest {
            project_path,
            include_archived,
        })
        .await
        .map_err(AppError::from)?;

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
    Ok(adopt_head_branches(&state, views).await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_all_workstreams(state: State<'_, AppState>) -> CmdResult<Vec<WorkstreamView>> {
    let items = state
        .runtime
        .workstreams()
        .list_all()
        .await
        .map_err(AppError::from)?;

    let mut effective: HashMap<String, EffectiveConfig> = HashMap::new();
    for path in items
        .iter()
        .map(|item| item.project_path.clone())
        .collect::<BTreeSet<_>>()
    {
        if let Ok(eff) = effective_for(&state, &path).await {
            effective.insert(path, eff);
        }
    }
    let views = status_batch(items, move |item| {
        effective
            .get(&item.project_path)
            .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff))
    })
    .await?;
    Ok(adopt_head_branches(&state, views).await)
}

#[tauri::command]
#[specta::specta]
pub async fn watch_worktree(
    state: State<'_, AppState>,
    workstream_id: Option<String>,
) -> CmdResult<()> {
    state
        .runtime
        .workstreams()
        .watch(workstream_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_bases(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<WorkstreamBases> {
    let git = state
        .runtime
        .settings()
        .binaries()
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
        .runtime
        .settings()
        .binaries()
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
    if !state
        .runtime
        .projects()
        .source_of(&project_path)
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

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn create_workstream(
    state: State<'_, AppState>,
    project_path: String,
    title: String,
    base: Option<String>,
    mode: WorktreeMode,
    branch: Option<String>,
    branch_prefix: Option<String>,
) -> CmdResult<WorkstreamView> {
    let inplace = mode == WorktreeMode::Inplace;
    let result = state
        .runtime
        .workstreams()
        .create(core_workstreams::CreateWorkstreamRequest {
            project_path,
            title,
            base,
            mode,
            branch,
            branch_prefix,
        })
        .await
        .map_err(AppError::from)?;

    // TODO: Gate setup scripts on repository trust.
    if !inplace {
        let _ = state
            .runtime
            .scripts()
            .run_lifecycle(core_scripts::RunLifecycleRequest {
                workstream: result.item.clone(),
                kind: ScriptKind::Setup,
            })
            .await;
    }

    let mut view = item_view(result.item, result.link);
    view.seed = result.seed;
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_prefixes(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ResolvedBranchPrefixes> {
    super::ensure_registered_project(&state, &project_path).await?;
    let git = state.runtime.settings().get().await.git;
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
    let cleaned = hyprlayer_core::branch_prefix::normalize_prefixes(prefixes);
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
    state
        .runtime
        .workstreams()
        .rename(core_workstreams::RenameWorkstreamRequest { id, title })
        .await
        .map_err(AppError::from)?;
    Ok(())
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
    if let Ok(gh) = state
        .runtime
        .settings()
        .binaries()
        .require(BinaryKind::Gh)
        .await
    {
        let dir = pr_dir(&item);
        if let Ok(Some(status)) = pr::pr_status(&gh, dir, &item.pr_ref()).await {
            if status.state == pr::PrState::Open {
                pr::edit_pr_base(&gh, dir, &item.pr_ref(), &base)
                    .await
                    .map_err(AppError::from)?;
            }
        }
    }
    state
        .runtime
        .workstreams()
        .set_base_branch(&id, &base)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn update_workstream_from_base(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<PullOutcome> {
    let item = load_item(&state, &id).await?;
    let git = state
        .runtime
        .settings()
        .binaries()
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
        .runtime
        .settings()
        .binaries()
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
    state
        .runtime
        .workstreams()
        .update(core_workstreams::UpdateWorkstreamRequest {
            id,
            stage: Some(stage),
        })
        .await
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn attach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
    role: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    state
        .runtime
        .workstreams()
        .attach_doc(&id, &doc_path, &role)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    state
        .runtime
        .workstreams()
        .detach_doc(&id, &doc_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_docs(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    state
        .runtime
        .workstreams()
        .documents(core_workstreams::WorkstreamDocumentsRequest { id })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn archive_workstream(
    state: State<'_, AppState>,
    id: String,
    skip_scripts: bool,
) -> CmdResult<()> {
    state
        .runtime
        .workstreams()
        .archive(core_workstreams::ArchiveWorkstreamRequest { id, skip_scripts })
        .await
        .map_err(AppError::from)?;
    Ok(())
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
    let items = state
        .runtime
        .workstreams()
        .list_archived()
        .await
        .map_err(AppError::from)?;
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
    let result = state
        .runtime
        .workstreams()
        .restore(core_workstreams::RestoreWorkstreamRequest { id })
        .await
        .map_err(AppError::from)?;
    Ok(item_view(result.item, result.link))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_archived_workstream(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .runtime
        .workstreams()
        .purge(core_workstreams::PurgeWorkstreamRequest { id })
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupReport {
    pub deleted: u32,
    pub failed: u32,
}

impl From<core_workstreams::ArchiveCleanupReport> for ArchiveCleanupReport {
    fn from(r: core_workstreams::ArchiveCleanupReport) -> Self {
        Self {
            deleted: r.deleted,
            failed: r.failed,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn cleanup_archived_workstreams(
    state: State<'_, AppState>,
    before_ms: Option<f64>,
) -> CmdResult<ArchiveCleanupReport> {
    state
        .runtime
        .workstreams()
        .cleanup_archived(core_workstreams::CleanupArchivedRequest { before_ms })
        .await
        .map(ArchiveCleanupReport::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn reprovision_workstream(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorkstreamView> {
    let result = state
        .runtime
        .workstreams()
        .reprovision(core_workstreams::ReprovisionWorkstreamRequest { id })
        .await
        .map_err(AppError::from)?;
    Ok(item_view(result.item, Some(result.link)))
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
        Ok(hyprlayer_core::skills::list_slash_commands(
            p,
            dirs::home_dir().as_deref(),
        ))
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
    state
        .runtime
        .workstreams()
        .pr_status(core_workstreams::WorkstreamPrStatusRequest { id })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_open_prs(
    state: State<'_, AppState>,
    id: String,
    query: String,
) -> CmdResult<Vec<PrSummary>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state
        .runtime
        .settings()
        .binaries()
        .require(BinaryKind::Gh)
        .await
    else {
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
    let Ok(gh) = state
        .runtime
        .settings()
        .binaries()
        .require(BinaryKind::Gh)
        .await
    else {
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
        .runtime
        .settings()
        .binaries()
        .require(BinaryKind::Gh)
        .await
        .map_err(AppError::from)?;
    let status = match pr::pr_status(&gh, pr_dir(&item), &pr_number.to_string()).await {
        Ok(Some(status)) => status,
        Ok(None) => return Err(attach_pr_error(pr_number, None)),
        Err(e) => return Err(attach_pr_error(pr_number, Some(e))),
    };
    state
        .runtime
        .workstreams()
        .set_pr(&id, Some(pr_number))
        .await
        .map_err(AppError::from)?;
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
    state
        .runtime
        .workstreams()
        .set_pr(&id, None)
        .await
        .map_err(AppError::from)
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
        .runtime
        .settings()
        .binaries()
        .require(BinaryKind::Gh)
        .await
        .map_err(AppError::from)?;
    let auto = state.runtime.settings().get().await.git.automerge;
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
