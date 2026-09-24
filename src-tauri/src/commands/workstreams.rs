use hyprlayer_core::branch_prefix::{BranchPrefix, PrefixSource};
use hyprlayer_core::gitops::{CommitEntry, DirEntry, FileSides, GitFileEntry, WorktreeGitStatus};
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::pr::{MergeMethod, PrStatus, PrSummary};
use hyprlayer_core::runtime::workstreams as core_workstreams;
use hyprlayer_core::skills::SlashCommand;
use hyprlayer_core::store::{Workstream, WorkstreamDoc, WorktreeMode};
use hyprlayer_core::worktree::{PullOutcome, WorktreeStatus};
use hyprlayer_server::rpc;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub use hyprlayer_core::runtime::workstreams::{ArchiveInfo, ProjectMemory, WorkstreamBases};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamView {
    pub item: Workstream,
    pub git: WorktreeStatus,
    pub seed: Option<hyprlayer_core::worktree::seed::SeedReport>,
}

impl From<rpc::WorkstreamView> for WorkstreamView {
    fn from(v: rpc::WorkstreamView) -> Self {
        Self {
            item: v.item,
            git: v.git,
            seed: v.seed,
        }
    }
}

fn views(views: Vec<rpc::WorkstreamView>) -> Vec<WorkstreamView> {
    views.into_iter().map(WorkstreamView::from).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBranchPrefixes {
    pub prefixes: Vec<BranchPrefix>,
    pub default_prefix: String,
    pub repo: Vec<BranchPrefix>,
    pub source: PrefixSource,
}

impl From<rpc::ResolvedBranchPrefixes> for ResolvedBranchPrefixes {
    fn from(r: rpc::ResolvedBranchPrefixes) -> Self {
        Self {
            prefixes: r.prefixes,
            default_prefix: r.default_prefix,
            repo: r.repo,
            source: r.source,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstreams(
    state: State<'_, AppState>,
    project_path: String,
    include_archived: bool,
) -> CmdResult<Vec<WorkstreamView>> {
    state
        .client()?
        .list_workstreams(project_path, include_archived)
        .await
        .map(views)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_all_workstreams(state: State<'_, AppState>) -> CmdResult<Vec<WorkstreamView>> {
    state
        .client()?
        .list_all_workstreams()
        .await
        .map(views)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn watch_worktree(
    state: State<'_, AppState>,
    workstream_id: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .watch_worktree(workstream_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_bases(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<WorkstreamBases> {
    state
        .client()?
        .list_workstream_bases(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_project_remote(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<()> {
    state
        .client()?
        .fetch_project_remote(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn discard_project_changes(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<u32> {
    state
        .client()?
        .discard_project_changes(project_path)
        .await
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
    state
        .client()?
        .create_workstream(project_path, title, base, mode, branch, branch_prefix)
        .await
        .map(WorkstreamView::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_branch_prefixes(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ResolvedBranchPrefixes> {
    super::ensure_registered_project(&state, &project_path).await?;
    state
        .client()?
        .get_branch_prefixes(project_path)
        .await
        .map(ResolvedBranchPrefixes::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_branch_prefixes(
    state: State<'_, AppState>,
    project_path: String,
    prefixes: Vec<BranchPrefix>,
) -> CmdResult<Vec<BranchPrefix>> {
    super::ensure_registered_project(&state, &project_path).await?;
    state
        .client()?
        .set_branch_prefixes(project_path, prefixes)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workstream(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> CmdResult<()> {
    state
        .client()?
        .rename_workstream(id, title)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_workstream_base_branch(
    state: State<'_, AppState>,
    id: String,
    base: String,
) -> CmdResult<()> {
    state
        .client()?
        .set_workstream_base_branch(id, base)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn update_workstream_from_base(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<PullOutcome> {
    state
        .client()?
        .update_workstream_from_base(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_workstream_remote(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .client()?
        .fetch_workstream_remote(id)
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
        .client()?
        .set_workstream_stage(id, stage)
        .await
        .map_err(AppError::from)
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
        .client()?
        .attach_workstream_doc(id, doc_path, role)
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
        .client()?
        .detach_workstream_doc(id, doc_path)
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
        .client()?
        .list_workstream_docs(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_project_memory(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<ProjectMemory> {
    state
        .client()?
        .list_project_memory(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn save_project_memory_file(
    state: State<'_, AppState>,
    id: String,
    name: String,
    content: String,
    base_mtime_ms: f64,
) -> CmdResult<()> {
    state
        .client()?
        .save_project_memory_file(id, name, content, base_mtime_ms)
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
        .client()?
        .archive_workstream(id, skip_scripts)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedWorkstreamView {
    pub item: Workstream,
    pub archive_path: String,
    pub archive_exists: bool,
    pub info: ArchiveInfo,
}

impl From<rpc::ArchivedWorkstreamView> for ArchivedWorkstreamView {
    fn from(v: rpc::ArchivedWorkstreamView) -> Self {
        Self {
            item: v.item,
            archive_path: v.archive_path,
            archive_exists: v.archive_exists,
            info: v.info,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_archived_workstreams(
    state: State<'_, AppState>,
) -> CmdResult<Vec<ArchivedWorkstreamView>> {
    state
        .client()?
        .list_archived_workstreams()
        .await
        .map(|items| {
            items
                .into_iter()
                .map(ArchivedWorkstreamView::from)
                .collect()
        })
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn restore_workstream(
    state: State<'_, AppState>,
    id: String,
    target: Option<String>,
) -> CmdResult<WorkstreamView> {
    state
        .client()?
        .restore_workstream(id, target)
        .await
        .map(WorkstreamView::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_archived_workstream(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .client()?
        .delete_archived_workstream(id)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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
pub async fn refresh_archived_workstreams(state: State<'_, AppState>) -> CmdResult<u32> {
    state
        .client()?
        .refresh_archived_workstreams()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn keep_archived_workstream(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .client()?
        .keep_archived_workstream(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn purge_archived_workstreams(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> CmdResult<ArchiveCleanupReport> {
    state
        .client()?
        .purge_archived_workstreams(ids)
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
    state
        .client()?
        .reprovision_workstream(id)
        .await
        .map(WorkstreamView::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_git_status(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorktreeGitStatus> {
    state
        .client()?
        .worktree_git_status(id)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptContext {
    pub branch: String,
    pub base_branch: String,
    pub changed_files: String,
    pub diff: String,
    pub last_commit: String,
}

impl From<rpc::PromptContext> for PromptContext {
    fn from(c: rpc::PromptContext) -> Self {
        Self {
            branch: c.branch,
            base_branch: c.base_branch,
            changed_files: c.changed_files,
            diff: c.diff,
            last_commit: c.last_commit,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn workstream_prompt_context(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<PromptContext> {
    state
        .client()?
        .workstream_prompt_context(id)
        .await
        .map(PromptContext::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_stage(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    state
        .client()?
        .worktree_stage(id, paths)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_unstage(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    state
        .client()?
        .worktree_unstage(id, paths)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_discard(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    state
        .client()?
        .worktree_discard(id, paths)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_file_sides(
    state: State<'_, AppState>,
    id: String,
    path: String,
    staged: bool,
) -> CmdResult<FileSides> {
    state
        .client()?
        .worktree_file_sides(id, path, staged)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_commits(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<CommitEntry>> {
    state
        .client()?
        .list_workstream_commits(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_commit_files(
    state: State<'_, AppState>,
    id: String,
    sha: String,
) -> CmdResult<Vec<GitFileEntry>> {
    state
        .client()?
        .worktree_commit_files(id, sha)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_commit_file_sides(
    state: State<'_, AppState>,
    id: String,
    sha: String,
    path: String,
) -> CmdResult<FileSides> {
    state
        .client()?
        .worktree_commit_file_sides(id, sha, path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_list_dir(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<Vec<DirEntry>> {
    state
        .client()?
        .worktree_list_dir(id, path)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeFile {
    pub content: String,
    pub mtime_ms: f64,
}

impl From<rpc::WorktreeFile> for WorktreeFile {
    fn from(f: rpc::WorktreeFile) -> Self {
        Self {
            content: f.content,
            mtime_ms: f.mtime_ms,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_read_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<WorktreeFile> {
    state
        .client()?
        .worktree_read_file(id, path)
        .await
        .map(WorktreeFile::from)
        .map_err(AppError::from)
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
    state
        .client()?
        .worktree_write_file(id, path, content, base_mtime_ms)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_slash_commands(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<SlashCommand>> {
    state
        .client()?
        .list_slash_commands(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn worktree_search_files(
    state: State<'_, AppState>,
    id: String,
    query: String,
    limit: u32,
) -> CmdResult<Vec<String>> {
    state
        .client()?
        .worktree_search_files(id, query, limit)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn workstream_pr_status(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Option<PrStatus>> {
    state
        .client()?
        .workstream_pr_status(id)
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
    state
        .client()?
        .list_workstream_open_prs(id, query)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn preview_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    pr_number: u32,
) -> CmdResult<Option<PrSummary>> {
    state
        .client()?
        .preview_workstream_pr(id, pr_number)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn attach_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    pr_number: u32,
) -> CmdResult<Option<PrStatus>> {
    state
        .client()?
        .attach_workstream_pr(id, pr_number)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_pr(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .client()?
        .detach_workstream_pr(id)
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
    state
        .client()?
        .merge_workstream_pr(id, method)
        .await
        .map_err(AppError::from)
}
