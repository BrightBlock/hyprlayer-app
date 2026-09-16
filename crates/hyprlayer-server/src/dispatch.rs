//! One arm per command: each `Dispatch` method calls the runtime service the
//! Tauri command used to call, and lowers `CoreError` to the wire envelope.
//! Where the desktop used to compose git/fs helpers around a service call,
//! that composition now runs here, against the server's filesystem.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprlayer_core::agents::{AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay};
use hyprlayer_core::branch_prefix::{self, BranchPrefix};
use hyprlayer_core::claude::ClaudeProbe;
use hyprlayer_core::codex::CodexProbe;
use hyprlayer_core::config::ThoughtsProfile;
use hyprlayer_core::docs::frontmatter::FmPatch;
use hyprlayer_core::gitops::{
    self, CommitEntry, DirEntry, FileSides, GitFileEntry, ThoughtsRepoStatus, WorktreeGitStatus,
};
use hyprlayer_core::hyprcli::{AiStatus, AiVersions, CliUpdateCheck};
use hyprlayer_core::index::{DocMeta, QueryResult, ThoughtQuery};
use hyprlayer_core::models::session::QuestionAnswers;
use hyprlayer_core::opencode::catalog::OpencodeCatalog;
use hyprlayer_core::pr::{MergeMethod, PrStatus, PrSummary};
use hyprlayer_core::process::binaries::{BinaryKind, BinaryStatus};
use hyprlayer_core::repo_settings::{self, ScriptSettings};
use hyprlayer_core::repos::{GithubRepo, RepoPathStatus};
use hyprlayer_core::runtime::agents::{BundleInstall, BundleReport, BundleStatus};
use hyprlayer_core::runtime::attachments::{ImportAttachmentRequest, SaveAttachmentRequest};
use hyprlayer_core::runtime::library::{CreateDocumentRequest, Document};
use hyprlayer_core::runtime::projects::{
    BackendHealth, BootstrapResult, CloneProjectRequest, CreateProjectRequest, DirListing,
    GithubOwnersOutcome, ImportFolderRequest, ImportRepositoryRequest, ProjectInspection,
    ProjectView, PublishProjectRequest, RunOnboardingRequest, SyncThoughtsRequest,
    SyncThoughtsResult, ThoughtsProbe, ThoughtsSource,
};
use hyprlayer_core::runtime::prompts::{
    PromptExportRequest, PromptExportResult, PromptImportDecision, PromptImportPreview,
    PromptImportRequest, PromptImportResult,
};
use hyprlayer_core::runtime::providers::{ClaudeAuthStatus, ModelWindow, OllamaStatus};
use hyprlayer_core::runtime::scripts::{
    RunLifecycleRequest, ScriptOutputRequest, ScriptRuntimeRequest, ScriptSettingsRequest,
    StopScriptRequest,
};
use hyprlayer_core::runtime::sessions::ActiveSession;
use hyprlayer_core::runtime::settings_service::{
    CliInstallMethod, CliInstallMethodId, CliUpdateResult,
};
use hyprlayer_core::runtime::terminals::{
    default_shell, CaptureSessionRequest, CloseTerminalRequest, MintCodexSessionRequest,
    ResizeAgentTerminalRequest, ResizeTerminalRequest, SessionExistsRequest, WriteTerminalRequest,
};
use hyprlayer_core::runtime::workstreams::{
    ArchiveCleanupReport, ArchiveWorkstreamRequest, CleanupArchivedRequest,
    CreateWorkstreamRequest, HeadBranchObservation, ListWorkstreamsRequest, PurgeWorkstreamRequest,
    RenameWorkstreamRequest, ReprovisionWorkstreamRequest, RestoreWorkstreamRequest,
    UpdateWorkstreamRequest, WorkstreamBases, WorkstreamDocumentsRequest,
    WorkstreamPrStatusRequest,
};
use hyprlayer_core::settings::AppSettings;
use hyprlayer_core::skills::{self, AgentSkill, SlashCommand};
use hyprlayer_core::worktree::seed::SeedReport;
use hyprlayer_core::worktree::thoughts_links::{self, LinkStatus};
use hyprlayer_core::worktree::{self, archive_dir_for, PullOutcome};
use hyprlayer_core::{
    CoreError, HyprlayerRuntime, ProjectSource, ScriptKind, Session, SessionLogEntry, Stage,
    Workstream, WorkstreamDoc, WorktreeMode,
};
use hyprlayer_models::ScriptRuntime;
use hyprlayer_proto::WireError;

use crate::rpc::{
    ArchivedWorkstreamView, CreatedProject, Dispatch, PromptContext, ResolvedBranchPrefixes,
    WorkstreamView, WorktreeFile,
};
use crate::streams::StreamState;

pub struct Server {
    runtime: Arc<HyprlayerRuntime>,
    streams: StreamState,
}

impl Server {
    pub fn new(runtime: Arc<HyprlayerRuntime>) -> Self {
        Self {
            runtime,
            streams: StreamState::default(),
        }
    }

    pub(crate) fn runtime(&self) -> Arc<HyprlayerRuntime> {
        self.runtime.clone()
    }

    pub(crate) fn streams(&self) -> &StreamState {
        &self.streams
    }
}

pub fn wire_error(e: CoreError) -> WireError {
    WireError {
        code: e.code().to_string(),
        message: e.to_string(),
        remedy: e.remedy(),
    }
}

pub(crate) trait Wire<T> {
    fn wire(self) -> Result<T, WireError>;
}

impl<T> Wire<T> for Result<T, CoreError> {
    fn wire(self) -> Result<T, WireError> {
        self.map_err(wire_error)
    }
}

async fn blocking<T, F>(label: &str, f: F) -> Result<T, WireError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| WireError::internal(format!("{label} task: {e}")))
}

const WORKTREE_GONE: &str = "The worktree may have been deleted; archive this workstream.";
const COMMIT_GONE: &str =
    "This commit may no longer be on the branch after a rebase or squash-merge; \
     reopen the file from the Commits tab.";

fn file_conflict() -> WireError {
    WireError {
        code: "file_conflict".into(),
        message: "The file changed on disk since you opened it.".into(),
        remedy: Some(
            "Reload the file to pick up the external changes, then re-apply your edits.".into(),
        ),
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

async fn status_batch<F>(
    items: Vec<Workstream>,
    link_for: F,
) -> Result<Vec<WorkstreamView>, WireError>
where
    F: Fn(&Workstream) -> Option<LinkStatus> + Send + Sync + 'static,
{
    let link_for = Arc::new(link_for);
    let handles: Vec<_> = items
        .into_iter()
        .map(|item| {
            let link_for = link_for.clone();
            tokio::task::spawn_blocking(move || {
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
                .map_err(|e| WireError::internal(format!("status task: {e}")))?,
        );
    }
    Ok(views)
}

impl Server {
    async fn adopt_head_branches(&self, mut views: Vec<WorkstreamView>) -> Vec<WorkstreamView> {
        let observed = views
            .iter()
            .map(|v| HeadBranchObservation {
                item: v.item.clone(),
                head_branch: v.git.branch.clone(),
            })
            .collect();
        let adoptions = match self
            .runtime
            .workstreams()
            .adopt_head_branches(observed)
            .await
        {
            Ok(adoptions) => adoptions,
            Err(e) => {
                tracing::warn!("branch adoption write failed, retrying next sweep: {e}");
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

    async fn with_worktree_base_remedy<T, F>(
        &self,
        id: &str,
        op: &'static str,
        remedy: &'static str,
        f: F,
    ) -> Result<T, WireError>
    where
        T: Send + 'static,
        F: FnOnce(&Path, &str) -> Result<T, CoreError> + Send + 'static,
    {
        let item = self.runtime.workstreams().find(id).await.wire()?;
        blocking(op, move || {
            f(Path::new(&item.worktree_path), &item.base_branch)
        })
        .await?
        .map_err(|e| WireError {
            code: op.to_string(),
            message: e.to_string(),
            remedy: Some(remedy.into()),
        })
    }

    async fn with_worktree_base<T, F>(
        &self,
        id: &str,
        op: &'static str,
        f: F,
    ) -> Result<T, WireError>
    where
        T: Send + 'static,
        F: FnOnce(&Path, &str) -> Result<T, CoreError> + Send + 'static,
    {
        self.with_worktree_base_remedy(id, op, WORKTREE_GONE, f)
            .await
    }

    async fn with_worktree<T, F>(&self, id: &str, op: &'static str, f: F) -> Result<T, WireError>
    where
        T: Send + 'static,
        F: FnOnce(&Path) -> Result<T, CoreError> + Send + 'static,
    {
        self.with_worktree_base(id, op, move |p: &Path, _: &str| f(p))
            .await
    }

    async fn with_worktree_remedy<T, F>(
        &self,
        id: &str,
        op: &'static str,
        remedy: &'static str,
        f: F,
    ) -> Result<T, WireError>
    where
        T: Send + 'static,
        F: FnOnce(&Path) -> Result<T, CoreError> + Send + 'static,
    {
        self.with_worktree_base_remedy(id, op, remedy, move |p: &Path, _: &str| f(p))
            .await
    }
}

impl Dispatch for Server {
    async fn get_bootstrap(&self) -> Result<BootstrapResult, WireError> {
        self.runtime.projects().bootstrap().await.wire()
    }

    async fn get_project(&self, path: String) -> Result<Option<ProjectInspection>, WireError> {
        self.runtime.projects().inspect(&path).await.wire()
    }

    async fn inspect_repo_path(&self, path: String) -> Result<RepoPathStatus, WireError> {
        Ok(self.runtime.projects().inspect_repo_path(&path))
    }

    async fn import_project(
        &self,
        path: String,
        branch: Option<String>,
        directory_name: Option<String>,
        thoughts_dir: Option<String>,
        profile: Option<String>,
        bootstrap_user: Option<String>,
    ) -> Result<ProjectView, WireError> {
        self.runtime
            .projects()
            .import_repository(ImportRepositoryRequest {
                path,
                branch,
                directory_name,
                thoughts_dir,
                profile,
                bootstrap_user,
            })
            .await
            .wire()
    }

    async fn import_local_folder(
        &self,
        path: String,
        directory_name: Option<String>,
    ) -> Result<ProjectView, WireError> {
        self.runtime
            .projects()
            .import_folder(ImportFolderRequest {
                path,
                directory_name,
            })
            .await
            .wire()
    }

    async fn clone_project(
        &self,
        url: String,
        location: Option<String>,
        name: Option<String>,
        thoughts_dir: Option<String>,
        profile: Option<String>,
        bootstrap_user: Option<String>,
    ) -> Result<ProjectView, WireError> {
        self.runtime
            .projects()
            .clone(CloneProjectRequest {
                url,
                location,
                name,
                thoughts_dir,
                profile,
                bootstrap_user,
            })
            .await
            .wire()
    }

    async fn create_project(&self, req: CreateProjectRequest) -> Result<CreatedProject, WireError> {
        let result = self.runtime.projects().create(req).await.wire()?;
        Ok(CreatedProject {
            project: result.project,
            github_url: result.github_url,
            github_error: result.github_error.map(wire_error),
        })
    }

    async fn list_github_owners(&self) -> Result<GithubOwnersOutcome, WireError> {
        self.runtime.projects().github_owners().await.wire()
    }

    async fn publish_project_to_github(
        &self,
        op_id: String,
        path: String,
        owner: String,
        name: String,
        is_private: bool,
    ) -> Result<String, WireError> {
        self.runtime
            .projects()
            .publish(PublishProjectRequest {
                op_id,
                path,
                owner,
                name,
                is_private,
            })
            .await
            .wire()
    }

    async fn remove_project(&self, path: String, delete_files: bool) -> Result<(), WireError> {
        self.runtime
            .projects()
            .remove(&path, delete_files)
            .await
            .wire()
    }

    async fn reorder_projects(&self, paths: Vec<String>) -> Result<(), WireError> {
        self.runtime.projects().reorder(paths).await.wire()
    }

    async fn list_recent_repos(&self) -> Result<Vec<GithubRepo>, WireError> {
        self.runtime.projects().recent_repos().await.wire()
    }

    async fn list_thoughts_profiles(&self) -> Result<Vec<ThoughtsProfile>, WireError> {
        self.runtime.projects().profiles().await.wire()
    }

    async fn create_thoughts_profile(
        &self,
        name: String,
        thoughts_repo: Option<String>,
    ) -> Result<ThoughtsProfile, WireError> {
        self.runtime
            .projects()
            .create_profile(&name, thoughts_repo)
            .await
            .wire()
    }

    async fn sync_thoughts(
        &self,
        path: String,
        message: Option<String>,
    ) -> Result<SyncThoughtsResult, WireError> {
        self.runtime
            .projects()
            .sync_thoughts(SyncThoughtsRequest { path, message })
            .await
            .wire()
    }

    async fn run_backend_health(&self, path: String) -> Result<BackendHealth, WireError> {
        self.runtime.projects().backend_health(&path).await.wire()
    }

    async fn get_worktree_files(&self, path: String) -> Result<Vec<String>, WireError> {
        self.runtime.projects().worktree_files(&path).await.wire()
    }

    async fn set_worktree_files(
        &self,
        path: String,
        patterns: Vec<String>,
    ) -> Result<Vec<String>, WireError> {
        self.runtime
            .projects()
            .set_worktree_files(&path, patterns)
            .await
            .wire()
    }

    async fn check_worktree_files(&self, path: String) -> Result<SeedReport, WireError> {
        self.runtime
            .projects()
            .check_worktree_files(&path)
            .await
            .wire()
    }

    async fn browse_directory(&self, path: Option<String>) -> Result<DirListing, WireError> {
        Ok(self.runtime.projects().browse(path))
    }

    async fn find_project(&self, path: String) -> Result<Option<ProjectView>, WireError> {
        self.runtime.projects().find(&path).await.wire()
    }

    async fn project_source_of(&self, path: String) -> Result<ProjectSource, WireError> {
        Ok(self.runtime.projects().source_of(&path).await)
    }

    async fn query_docs(&self, query: ThoughtQuery) -> Result<QueryResult, WireError> {
        self.runtime.library().query(query).await.wire()
    }

    async fn get_doc(&self, id: String) -> Result<Document, WireError> {
        self.runtime.library().get(&id).await.wire()
    }

    async fn save_doc(
        &self,
        id: String,
        content: String,
        base_mtime_ms: f64,
    ) -> Result<DocMeta, WireError> {
        self.runtime
            .library()
            .save(&id, content, base_mtime_ms)
            .await
            .wire()
    }

    async fn patch_doc_frontmatter(
        &self,
        id: String,
        patch: FmPatch,
        base_mtime_ms: f64,
    ) -> Result<DocMeta, WireError> {
        self.runtime
            .library()
            .patch_frontmatter(&id, patch, base_mtime_ms)
            .await
            .wire()
    }

    async fn create_doc(&self, input: CreateDocumentRequest) -> Result<DocMeta, WireError> {
        self.runtime.library().create(input).await.wire()
    }

    async fn resolve_doc_path(&self, id: String) -> Result<String, WireError> {
        self.runtime
            .library()
            .resolve_path(&id)
            .wire()
            .map(|path| path.to_string_lossy().into_owned())
    }

    async fn doc_web_url(&self, id: String) -> Result<Option<String>, WireError> {
        let Ok(abs) = self.runtime.library().resolve_path(&id) else {
            return Ok(None);
        };
        Ok(
            tokio::task::spawn_blocking(move || gitops::file_web_url(&abs))
                .await
                .ok()
                .flatten(),
        )
    }

    async fn get_thoughts_git_status(
        &self,
        project_path: String,
    ) -> Result<ThoughtsRepoStatus, WireError> {
        self.runtime
            .library()
            .thoughts_git_status(&project_path)
            .await
            .wire()
    }

    async fn list_workstreams(
        &self,
        project_path: String,
        include_archived: bool,
    ) -> Result<Vec<WorkstreamView>, WireError> {
        let items = self
            .runtime
            .workstreams()
            .list(ListWorkstreamsRequest {
                project_path,
                include_archived,
            })
            .await
            .wire()?;
        let effective = match items.first() {
            Some(first) => Some(
                self.runtime
                    .settings()
                    .effective_config_for(&first.project_path)
                    .await
                    .wire()?,
            ),
            None => None,
        };
        let views = status_batch(items, move |item| {
            effective
                .as_ref()
                .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff))
        })
        .await?;
        Ok(self.adopt_head_branches(views).await)
    }

    async fn list_all_workstreams(&self) -> Result<Vec<WorkstreamView>, WireError> {
        let items = self.runtime.workstreams().list_all().await.wire()?;
        let mut effective = std::collections::HashMap::new();
        for path in items
            .iter()
            .map(|item| item.project_path.clone())
            .collect::<std::collections::BTreeSet<_>>()
        {
            if let Ok(eff) = self.runtime.settings().effective_config_for(&path).await {
                effective.insert(path, eff);
            }
        }
        let views = status_batch(items, move |item| {
            effective
                .get(&item.project_path)
                .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff))
        })
        .await?;
        Ok(self.adopt_head_branches(views).await)
    }

    async fn list_workstream_bases(
        &self,
        project_path: String,
    ) -> Result<WorkstreamBases, WireError> {
        self.runtime
            .workstreams()
            .list_bases(&project_path)
            .await
            .wire()
    }

    async fn fetch_project_remote(&self, project_path: String) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .fetch_project_remote(&project_path)
            .await
            .wire()
    }

    async fn discard_project_changes(&self, project_path: String) -> Result<u32, WireError> {
        if !self
            .runtime
            .projects()
            .source_of(&project_path)
            .await
            .is_git()
        {
            return Err(WireError {
                code: "invalid_mode".into(),
                message: "This project is a plain folder, so it has no git changes to discard."
                    .into(),
                remedy: None,
            });
        }
        blocking("discard_project_changes", move || {
            gitops::discard_tracked_changes(Path::new(&project_path))
        })
        .await?
        .wire()
    }

    async fn create_workstream(
        &self,
        project_path: String,
        title: String,
        base: Option<String>,
        mode: WorktreeMode,
        branch: Option<String>,
        branch_prefix: Option<String>,
    ) -> Result<WorkstreamView, WireError> {
        let inplace = mode == WorktreeMode::Inplace;
        let result = self
            .runtime
            .workstreams()
            .create(CreateWorkstreamRequest {
                project_path,
                title,
                base,
                mode,
                branch,
                branch_prefix,
            })
            .await
            .wire()?;

        // TODO: Gate setup scripts on repository trust.
        if !inplace {
            let _ = self
                .runtime
                .scripts()
                .run_lifecycle(RunLifecycleRequest {
                    workstream: result.item.clone(),
                    kind: ScriptKind::Setup,
                })
                .await;
        }

        let seed = result.seed;
        let mut view = blocking("create_workstream_status", move || {
            item_view(result.item, result.link)
        })
        .await?;
        view.seed = seed;
        Ok(view)
    }

    async fn get_branch_prefixes(
        &self,
        project_path: String,
    ) -> Result<ResolvedBranchPrefixes, WireError> {
        let git = self.runtime.settings().get().await.git;
        let repo_dir = PathBuf::from(&project_path);
        let settings = blocking("read_branch_prefixes", move || {
            repo_settings::read(&repo_dir)
        })
        .await?
        .wire()?;
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

    async fn set_branch_prefixes(
        &self,
        project_path: String,
        prefixes: Vec<BranchPrefix>,
    ) -> Result<Vec<BranchPrefix>, WireError> {
        let cleaned = branch_prefix::normalize_prefixes(prefixes);
        branch_prefix::validate_prefixes(&cleaned).wire()?;
        let repo = PathBuf::from(&project_path);
        let saved = cleaned.clone();
        blocking("write_branch_prefixes", move || {
            repo_settings::set_branch_prefixes(&repo, &cleaned)
        })
        .await?
        .wire()?;
        Ok(saved)
    }

    async fn rename_workstream(&self, id: String, title: String) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .rename(RenameWorkstreamRequest { id, title })
            .await
            .wire()?;
        Ok(())
    }

    async fn set_workstream_base_branch(&self, id: String, base: String) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .set_base_branch(&id, &base)
            .await
            .wire()
    }

    async fn update_workstream_from_base(&self, id: String) -> Result<PullOutcome, WireError> {
        self.runtime
            .workstreams()
            .update_from_base(&id)
            .await
            .wire()
    }

    async fn fetch_workstream_remote(&self, id: String) -> Result<(), WireError> {
        self.runtime.workstreams().fetch_remote(&id).await.wire()
    }

    async fn set_workstream_stage(&self, id: String, stage: Stage) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .update(UpdateWorkstreamRequest {
                id,
                stage: Some(stage),
            })
            .await
            .wire()?;
        Ok(())
    }

    async fn attach_workstream_doc(
        &self,
        id: String,
        doc_path: String,
        role: String,
    ) -> Result<Vec<WorkstreamDoc>, WireError> {
        self.runtime
            .workstreams()
            .attach_doc(&id, &doc_path, &role)
            .await
            .wire()
    }

    async fn detach_workstream_doc(
        &self,
        id: String,
        doc_path: String,
    ) -> Result<Vec<WorkstreamDoc>, WireError> {
        self.runtime
            .workstreams()
            .detach_doc(&id, &doc_path)
            .await
            .wire()
    }

    async fn list_workstream_docs(&self, id: String) -> Result<Vec<WorkstreamDoc>, WireError> {
        self.runtime
            .workstreams()
            .documents(WorkstreamDocumentsRequest { id })
            .await
            .wire()
    }

    async fn archive_workstream(&self, id: String, skip_scripts: bool) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .archive(ArchiveWorkstreamRequest { id, skip_scripts })
            .await
            .wire()?;
        Ok(())
    }

    async fn list_archived_workstreams(&self) -> Result<Vec<ArchivedWorkstreamView>, WireError> {
        let items = self.runtime.workstreams().list_archived().await.wire()?;
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

    async fn restore_workstream(&self, id: String) -> Result<WorkstreamView, WireError> {
        let result = self
            .runtime
            .workstreams()
            .restore(RestoreWorkstreamRequest { id })
            .await
            .wire()?;
        blocking("restore_workstream_status", move || {
            item_view(result.item, result.link)
        })
        .await
    }

    async fn delete_archived_workstream(&self, id: String) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .purge(PurgeWorkstreamRequest { id })
            .await
            .wire()
    }

    async fn cleanup_archived_workstreams(
        &self,
        before_ms: Option<f64>,
    ) -> Result<ArchiveCleanupReport, WireError> {
        self.runtime
            .workstreams()
            .cleanup_archived(CleanupArchivedRequest { before_ms })
            .await
            .wire()
    }

    async fn reprovision_workstream(&self, id: String) -> Result<WorkstreamView, WireError> {
        let result = self
            .runtime
            .workstreams()
            .reprovision(ReprovisionWorkstreamRequest { id })
            .await
            .wire()?;
        blocking("reprovision_workstream_status", move || {
            item_view(result.item, Some(result.link))
        })
        .await
    }

    async fn worktree_git_status(&self, id: String) -> Result<WorktreeGitStatus, WireError> {
        self.with_worktree(&id, "worktree_git_status", |p| {
            gitops::worktree_git_status(p)
        })
        .await
    }

    async fn workstream_prompt_context(&self, id: String) -> Result<PromptContext, WireError> {
        let item = self.runtime.workstreams().find(&id).await.wire()?;
        let branch = item.branch.clone();
        let base_branch = item
            .base_branch
            .strip_prefix("origin/")
            .unwrap_or(&item.base_branch)
            .to_string();
        let git = blocking("workstream_prompt_context", move || {
            gitops::prompt_git_vars(Path::new(&item.worktree_path), true, true, true)
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

    async fn worktree_stage(&self, id: String, paths: Vec<String>) -> Result<(), WireError> {
        self.with_worktree(&id, "worktree_stage", move |p| {
            gitops::stage_files(p, &paths)
        })
        .await
    }

    async fn worktree_unstage(&self, id: String, paths: Vec<String>) -> Result<(), WireError> {
        self.with_worktree(&id, "worktree_unstage", move |p| {
            gitops::unstage_files(p, &paths)
        })
        .await
    }

    async fn worktree_discard(&self, id: String, paths: Vec<String>) -> Result<(), WireError> {
        self.with_worktree(&id, "worktree_discard", move |p| {
            gitops::discard_files(p, &paths)
        })
        .await
    }

    async fn worktree_file_sides(
        &self,
        id: String,
        path: String,
        staged: bool,
    ) -> Result<FileSides, WireError> {
        self.with_worktree(&id, "worktree_file_sides", move |p| {
            gitops::worktree_file_sides(p, &path, staged)
        })
        .await
    }

    async fn worktree_list_dir(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<DirEntry>, WireError> {
        self.with_worktree(&id, "worktree_list_dir", move |p| {
            gitops::list_dir(p, &path)
        })
        .await
    }

    async fn worktree_read_file(
        &self,
        id: String,
        path: String,
    ) -> Result<WorktreeFile, WireError> {
        self.with_worktree(&id, "worktree_read_file", move |p| {
            let content = gitops::read_text_file(p, &path)?;
            let mtime_ms = gitops::file_mtime_ms(p, &path)?;
            Ok(WorktreeFile { content, mtime_ms })
        })
        .await
    }

    async fn worktree_write_file(
        &self,
        id: String,
        path: String,
        content: String,
        base_mtime_ms: f64,
    ) -> Result<f64, WireError> {
        let item = self.runtime.workstreams().find(&id).await.wire()?;
        let worktree = PathBuf::from(&item.worktree_path);
        blocking("worktree_write_file", move || -> Result<f64, WireError> {
            if base_mtime_ms > 0.0 {
                if let Ok(current) = gitops::file_mtime_ms(&worktree, &path) {
                    if (current - base_mtime_ms).abs() > 0.5 {
                        return Err(file_conflict());
                    }
                }
            }
            gitops::write_text_file(&worktree, &path, &content).wire()?;
            gitops::file_mtime_ms(&worktree, &path).wire()
        })
        .await?
    }

    async fn list_slash_commands(&self, id: String) -> Result<Vec<SlashCommand>, WireError> {
        self.with_worktree(&id, "list_slash_commands", |p| {
            Ok(skills::list_slash_commands(p, dirs::home_dir().as_deref()))
        })
        .await
    }

    async fn worktree_search_files(
        &self,
        id: String,
        query: String,
        limit: u32,
    ) -> Result<Vec<String>, WireError> {
        self.with_worktree(&id, "worktree_search_files", move |p| {
            gitops::search_files(p, &query, limit as usize)
        })
        .await
    }

    async fn list_workstream_commits(&self, id: String) -> Result<Vec<CommitEntry>, WireError> {
        self.with_worktree_base(&id, "list_workstream_commits", |p, base| {
            gitops::list_commits(p, base)
        })
        .await
    }

    async fn worktree_commit_files(
        &self,
        id: String,
        sha: String,
    ) -> Result<Vec<GitFileEntry>, WireError> {
        self.with_worktree_remedy(&id, "worktree_commit_files", COMMIT_GONE, move |p| {
            gitops::commit_files(p, &sha)
        })
        .await
    }

    async fn worktree_commit_file_sides(
        &self,
        id: String,
        sha: String,
        path: String,
    ) -> Result<FileSides, WireError> {
        self.with_worktree_remedy(&id, "worktree_commit_file_sides", COMMIT_GONE, move |p| {
            gitops::commit_file_sides(p, &sha, &path)
        })
        .await
    }

    async fn watch_worktree(&self, workstream_id: Option<String>) -> Result<(), WireError> {
        self.runtime.workstreams().watch(workstream_id).await.wire()
    }

    async fn workstream_pr_status(&self, id: String) -> Result<Option<PrStatus>, WireError> {
        self.runtime
            .workstreams()
            .pr_status(WorkstreamPrStatusRequest { id })
            .await
            .wire()
    }

    async fn list_workstream_open_prs(
        &self,
        id: String,
        query: String,
    ) -> Result<Vec<PrSummary>, WireError> {
        self.runtime
            .workstreams()
            .list_open_prs(&id, &query)
            .await
            .wire()
    }

    async fn preview_workstream_pr(
        &self,
        id: String,
        pr_number: u32,
    ) -> Result<Option<PrSummary>, WireError> {
        self.runtime
            .workstreams()
            .preview_pr(&id, pr_number)
            .await
            .wire()
    }

    async fn attach_workstream_pr(
        &self,
        id: String,
        pr_number: u32,
    ) -> Result<Option<PrStatus>, WireError> {
        self.runtime
            .workstreams()
            .attach_pr(&id, pr_number)
            .await
            .wire()
    }

    async fn detach_workstream_pr(&self, id: String) -> Result<(), WireError> {
        self.runtime.workstreams().find(&id).await.wire()?;
        self.runtime.workstreams().set_pr(&id, None).await.wire()
    }

    async fn merge_workstream_pr(&self, id: String, method: MergeMethod) -> Result<(), WireError> {
        self.runtime
            .workstreams()
            .merge_pr(&id, method)
            .await
            .wire()
    }

    async fn send_user_message(
        &self,
        session_id: String,
        text: String,
        attached_docs: Vec<String>,
        prompt_name: Option<String>,
    ) -> Result<(), WireError> {
        self.runtime
            .sessions()
            .send(&session_id, &text, &attached_docs, prompt_name.as_deref())
            .await
            .wire()
    }

    async fn respond_permission(
        &self,
        session_id: String,
        request_id: String,
        allow: bool,
        message: Option<String>,
    ) -> Result<(), WireError> {
        self.runtime
            .sessions()
            .respond_permission(&session_id, &request_id, allow, message.as_deref())
            .await
            .wire()
    }

    async fn answer_question(
        &self,
        session_id: String,
        request_id: String,
        answers: QuestionAnswers,
    ) -> Result<(), WireError> {
        self.runtime
            .sessions()
            .answer(&session_id, &request_id, answers)
            .await
            .wire()
    }

    async fn interrupt_session(&self, session_id: String) -> Result<(), WireError> {
        self.runtime.sessions().interrupt(&session_id).await.wire()
    }

    async fn kill_session(&self, session_id: String) -> Result<(), WireError> {
        self.runtime.sessions().kill(&session_id).await.wire()
    }

    async fn set_session_model(
        &self,
        session_id: String,
        model: Option<String>,
    ) -> Result<(), WireError> {
        self.runtime
            .sessions()
            .set_model(&session_id, model)
            .await
            .wire()
    }

    async fn set_session_effort(
        &self,
        session_id: String,
        effort: Option<String>,
    ) -> Result<(), WireError> {
        self.runtime
            .sessions()
            .set_effort(&session_id, effort)
            .await
            .wire()
    }

    async fn list_sessions(&self, workstream_id: String) -> Result<Vec<Session>, WireError> {
        self.runtime.sessions().list(&workstream_id).await.wire()
    }

    async fn get_session_log(&self, session_id: String) -> Result<Vec<SessionLogEntry>, WireError> {
        self.runtime.sessions().log(&session_id).await.wire()
    }

    async fn list_active_sessions(&self) -> Result<Vec<ActiveSession>, WireError> {
        self.runtime.sessions().list_active().await.wire()
    }

    async fn import_attachment(
        &self,
        workstream_id: String,
        src_path: String,
    ) -> Result<String, WireError> {
        self.runtime
            .attachments()
            .import(ImportAttachmentRequest {
                workstream_id,
                src_path,
            })
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .wire()
    }

    async fn save_attachment(
        &self,
        workstream_id: String,
        file_name: String,
        data: Vec<u8>,
    ) -> Result<String, WireError> {
        self.runtime
            .attachments()
            .save(SaveAttachmentRequest {
                workstream_id,
                file_name,
                data,
            })
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .wire()
    }

    async fn set_binary_override(
        &self,
        kind: BinaryKind,
        path: Option<String>,
    ) -> Result<Vec<BinaryStatus>, WireError> {
        self.runtime
            .settings()
            .set_binary_override(kind, path)
            .await
            .wire()
    }

    async fn redetect_binaries(&self) -> Result<Vec<BinaryStatus>, WireError> {
        Ok(self.runtime.settings().redetect_binaries().await)
    }

    async fn detect_cli_install_methods(&self) -> Result<Vec<CliInstallMethod>, WireError> {
        Ok(self.runtime.settings().detect_cli_install_methods().await)
    }

    async fn hyprlayer_cli_update_check(&self) -> Result<CliUpdateCheck, WireError> {
        self.runtime
            .settings()
            .hyprlayer_cli_update_check()
            .await
            .wire()
    }

    async fn update_hyprlayer_cli(&self) -> Result<CliUpdateResult, WireError> {
        self.runtime.settings().update_hyprlayer_cli().await.wire()
    }

    async fn install_hyprlayer_cli(&self, method: CliInstallMethodId) -> Result<(), WireError> {
        self.runtime
            .settings()
            .install_hyprlayer_cli(method)
            .await
            .wire()
    }

    async fn get_app_settings(&self) -> Result<AppSettings, WireError> {
        Ok(self.runtime.settings().get().await)
    }

    async fn set_app_settings(&self, settings: AppSettings) -> Result<AppSettings, WireError> {
        self.runtime.settings().set(settings).await.wire()?;
        Ok(self.runtime.settings().get().await)
    }

    async fn github_username(&self) -> Result<Option<String>, WireError> {
        Ok(self.runtime.settings().github_username().await)
    }

    async fn default_username(&self) -> Result<String, WireError> {
        Ok(self.runtime.settings().default_username().await)
    }

    async fn claude_sync_models(
        &self,
        add: Vec<String>,
        seen: Vec<String>,
    ) -> Result<AppSettings, WireError> {
        self.runtime
            .settings()
            .update(|s| {
                let claude = &mut s.providers.claude;
                if let Some(visible) = claude.visible_models.as_mut() {
                    for key in &add {
                        if !visible.contains(key) {
                            visible.push(key.clone());
                        }
                    }
                }
                claude.seen_model_keys = Some(seen);
            })
            .await
            .wire()
    }

    async fn set_quit_confirm(&self, confirm: bool) -> Result<(), WireError> {
        self.runtime
            .settings()
            .update(|s| s.quit.confirm = confirm)
            .await
            .wire()?;
        Ok(())
    }

    async fn default_shell_name(&self) -> Result<String, WireError> {
        Ok(default_shell()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("shell")
            .to_string())
    }

    async fn opencode_catalog(&self, force: bool) -> Result<OpencodeCatalog, WireError> {
        self.runtime
            .providers()
            .opencode_catalog(force)
            .await
            .wire()
    }

    async fn invalidate_opencode_server(&self) -> Result<(), WireError> {
        self.runtime.providers().invalidate_opencode_server().await;
        Ok(())
    }

    async fn set_opencode_default_model(&self, model: Option<String>) -> Result<(), WireError> {
        self.runtime
            .settings()
            .update(|s| s.providers.opencode.default_model = model)
            .await
            .wire()?;
        Ok(())
    }

    async fn install_opencode(&self) -> Result<(), WireError> {
        self.runtime.settings().install_opencode().await.wire()
    }

    async fn claude_auth_status(&self) -> Result<ClaudeAuthStatus, WireError> {
        self.runtime.providers().claude_auth_status().await.wire()
    }

    async fn claude_probe(&self) -> Result<ClaudeProbe, WireError> {
        self.runtime.providers().claude_probe().await.wire()
    }

    async fn codex_probe(&self) -> Result<CodexProbe, WireError> {
        self.runtime.providers().codex_probe().await.wire()
    }

    async fn codex_login_with_api_key(&self, api_key: String) -> Result<CodexProbe, WireError> {
        self.runtime
            .providers()
            .codex_login_with_api_key(api_key)
            .await
            .wire()
    }

    async fn codex_logout(&self) -> Result<CodexProbe, WireError> {
        self.runtime.providers().codex_logout().await.wire()
    }

    async fn claude_model_windows(&self) -> Result<Vec<ModelWindow>, WireError> {
        self.runtime.providers().claude_model_windows().await.wire()
    }

    async fn ollama_status(&self) -> Result<OllamaStatus, WireError> {
        self.runtime.providers().ollama_status().await.wire()
    }

    async fn ollama_set_endpoint(
        &self,
        endpoint: Option<String>,
    ) -> Result<OllamaStatus, WireError> {
        let status = self
            .runtime
            .providers()
            .ollama_set_endpoint(endpoint)
            .await
            .wire()?;
        self.runtime.providers().invalidate_opencode_server().await;
        Ok(status)
    }

    async fn ollama_set_selected_models(&self, models: Vec<String>) -> Result<(), WireError> {
        self.runtime
            .providers()
            .ollama_set_selected_models(models)
            .await
            .wire()?;
        self.runtime.providers().invalidate_opencode_server().await;
        Ok(())
    }

    async fn ollama_set_default_model(&self, model: Option<String>) -> Result<(), WireError> {
        self.runtime
            .providers()
            .ollama_set_default_model(model)
            .await
            .wire()
    }

    async fn generate_title_claude(
        &self,
        flag: String,
        prompt: String,
    ) -> Result<String, WireError> {
        self.runtime
            .providers()
            .generate_title_claude(&flag, &prompt)
            .await
            .wire()
    }

    async fn generate_title_opencode(
        &self,
        model: String,
        prompt: String,
    ) -> Result<String, WireError> {
        self.runtime
            .providers()
            .generate_title_opencode(&model, &prompt)
            .await
            .wire()
    }

    async fn generate_title_codex(
        &self,
        model: String,
        prompt: String,
    ) -> Result<String, WireError> {
        self.runtime
            .providers()
            .generate_title_codex(&model, &prompt)
            .await
            .wire()
    }

    async fn terminal_write(&self, terminal_id: String, data: String) -> Result<(), WireError> {
        self.runtime
            .terminals()
            .write(WriteTerminalRequest { terminal_id, data })
            .await
            .wire()
    }

    async fn terminal_resize(
        &self,
        terminal_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), WireError> {
        self.runtime
            .terminals()
            .resize(ResizeTerminalRequest {
                terminal_id,
                cols,
                rows,
            })
            .await
            .wire()
    }

    async fn terminal_close(&self, terminal_id: String) -> Result<(), WireError> {
        self.runtime
            .terminals()
            .close(CloseTerminalRequest { terminal_id })
            .await
            .wire()
    }

    async fn agent_resize(
        &self,
        terminal_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), WireError> {
        self.runtime
            .terminals()
            .resize_agent(ResizeAgentTerminalRequest {
                terminal_id,
                cols,
                rows,
            })
            .await
            .wire()
    }

    async fn capture_agent_session(
        &self,
        workstream_id: String,
        engine_id: String,
        after_ms: f64,
        exclude: Vec<String>,
    ) -> Result<Option<String>, WireError> {
        self.runtime
            .terminals()
            .capture_session(CaptureSessionRequest {
                workstream_id,
                engine_id,
                after_ms,
                exclude,
            })
            .await
            .wire()
    }

    async fn mint_codex_session(
        &self,
        workstream_id: String,
        command: String,
    ) -> Result<Option<String>, WireError> {
        self.runtime
            .terminals()
            .mint_codex_session(MintCodexSessionRequest {
                workstream_id,
                command,
            })
            .await
            .wire()
    }

    async fn agent_session_exists(
        &self,
        workstream_id: String,
        engine_id: String,
        session_id: String,
    ) -> Result<bool, WireError> {
        self.runtime
            .terminals()
            .session_exists(SessionExistsRequest {
                workstream_id,
                engine_id,
                session_id,
            })
            .await
            .wire()
    }

    async fn get_scripts(&self, project_path: String) -> Result<ScriptSettings, WireError> {
        self.runtime
            .scripts()
            .settings(ScriptSettingsRequest {
                project_path,
                set: None,
            })
            .await
            .wire()
    }

    async fn set_scripts(
        &self,
        project_path: String,
        scripts: ScriptSettings,
    ) -> Result<(), WireError> {
        self.runtime
            .scripts()
            .settings(ScriptSettingsRequest {
                project_path,
                set: Some(scripts),
            })
            .await
            .wire()?;
        Ok(())
    }

    async fn script_stop(&self, workstream_id: String) -> Result<(), WireError> {
        self.runtime
            .scripts()
            .stop(StopScriptRequest { workstream_id })
            .await
            .wire()
    }

    async fn get_script_runtime(&self, workstream_id: String) -> Result<ScriptRuntime, WireError> {
        self.runtime
            .scripts()
            .runtime(ScriptRuntimeRequest { workstream_id })
            .await
            .wire()
    }

    async fn list_running_scripts(&self) -> Result<Vec<String>, WireError> {
        self.runtime.scripts().running_ids().await.wire()
    }

    async fn get_script_output(
        &self,
        workstream_id: String,
        kind: ScriptKind,
    ) -> Result<Option<String>, WireError> {
        self.runtime
            .scripts()
            .output(ScriptOutputRequest {
                workstream_id,
                kind,
                clear: false,
                terminal_id: None,
            })
            .await
            .wire()
    }

    async fn script_clear_output(
        &self,
        workstream_id: String,
        kind: ScriptKind,
        terminal_id: Option<String>,
    ) -> Result<(), WireError> {
        self.runtime
            .scripts()
            .output(ScriptOutputRequest {
                workstream_id,
                kind,
                clear: true,
                terminal_id,
            })
            .await
            .wire()?;
        Ok(())
    }

    async fn get_ai_status(&self) -> Result<AiStatus, WireError> {
        self.runtime.projects().ai_status().await.wire()
    }

    async fn probe_default_thoughts(&self) -> Result<ThoughtsProbe, WireError> {
        Ok(self.runtime.projects().probe_default_thoughts())
    }

    async fn init_thoughts_repo(
        &self,
        user: String,
        source: ThoughtsSource,
    ) -> Result<BootstrapResult, WireError> {
        self.runtime
            .projects()
            .run_onboarding(RunOnboardingRequest { user, source })
            .await
            .wire()
    }

    async fn preview_prompt_import(
        &self,
        path: String,
        taken_chords: Vec<String>,
    ) -> Result<PromptImportPreview, WireError> {
        self.runtime
            .prompts()
            .preview_import(PromptImportRequest { path, taken_chords })
            .await
            .wire()
    }

    async fn apply_prompt_import(
        &self,
        decision: PromptImportDecision,
    ) -> Result<PromptImportResult, WireError> {
        self.runtime.prompts().apply_import(decision).await.wire()
    }

    async fn export_prompts(
        &self,
        request: PromptExportRequest,
    ) -> Result<PromptExportResult, WireError> {
        self.runtime.prompts().export(request).await.wire()
    }

    async fn list_agent_files(
        &self,
        project_path: Option<String>,
    ) -> Result<Vec<AgentFile>, WireError> {
        self.runtime.agents().list(project_path).await.wire()
    }

    async fn save_agent_file(&self, agent: AgentFile) -> Result<AgentFile, WireError> {
        self.runtime.agents().save(agent).await.wire()
    }

    async fn delete_agent_file(&self, path: String) -> Result<(), WireError> {
        self.runtime.agents().delete(path).await.wire()
    }

    async fn new_agent_file_path(
        &self,
        name: String,
        source: String,
        project_path: Option<String>,
    ) -> Result<Option<String>, WireError> {
        self.runtime
            .agents()
            .new_file_path(name, source, project_path)
            .await
            .wire()
    }

    async fn agent_context_files(
        &self,
        project_path: Option<String>,
    ) -> Result<Vec<AgentContextFile>, WireError> {
        self.runtime
            .agents()
            .context_files(project_path)
            .await
            .wire()
    }

    async fn agent_mcp_servers(
        &self,
        project_path: Option<String>,
    ) -> Result<Vec<AgentMcpServer>, WireError> {
        self.runtime.agents().mcp_servers(project_path).await.wire()
    }

    async fn agent_skills(
        &self,
        project_path: Option<String>,
    ) -> Result<Vec<AgentSkill>, WireError> {
        self.runtime.agents().skills(project_path).await.wire()
    }

    async fn export_agent_file(
        &self,
        agent: AgentFile,
        overlay: AgentOverlay,
    ) -> Result<String, WireError> {
        self.runtime.agents().export(agent, overlay).await.wire()
    }

    async fn agent_bundle_status(&self) -> Result<BundleStatus, WireError> {
        self.runtime.agents().bundle_status().await.wire()
    }

    async fn update_agent_bundle(&self) -> Result<BundleReport, WireError> {
        self.runtime.agents().update_bundle().await.wire()
    }

    async fn agent_bundle_versions(&self) -> Result<AiVersions, WireError> {
        self.runtime.agents().bundle_versions().await.wire()
    }

    async fn install_agent_bundle_version(
        &self,
        version: Option<String>,
    ) -> Result<BundleInstall, WireError> {
        self.runtime
            .agents()
            .install_bundle_version(version)
            .await
            .wire()
    }

    async fn repair_agent_bundle(&self) -> Result<BundleInstall, WireError> {
        self.runtime.agents().repair_bundle().await.wire()
    }

    async fn worktree_root(&self) -> Result<String, WireError> {
        Ok(self.runtime.worktree_root().to_string_lossy().into_owned())
    }

    async fn set_foreground(&self, foreground: bool) -> Result<(), WireError> {
        self.runtime.set_foreground(foreground);
        Ok(())
    }

    async fn set_env_overlay(&self, vars: BTreeMap<String, String>) -> Result<(), WireError> {
        self.runtime.set_env_overlay(vars);
        Ok(())
    }

    async fn launch_and_converse(&self, session_id: String) -> Result<(), WireError> {
        let Some(params) = crate::streams::take_pending_launch(self, &session_id) else {
            return Err(WireError::protocol(format!(
                "no prepared launch for session {session_id}"
            )));
        };
        self.runtime
            .sessions()
            .launch_and_converse(params)
            .await
            .wire()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::time::Duration;

    use base64::Engine as _;
    use hyprlayer_core::claude::SessionEvent;
    use hyprlayer_core::store::{Db, NewSession, NewWorkstream};
    use hyprlayer_core::terminal::TerminalEvent;
    use hyprlayer_core::{
        Harness, NoCredentialLookup, RuntimeEnvironment, RuntimeEvent, RuntimeOptions, RuntimePaths,
    };
    use tokio::sync::mpsc;

    use super::*;
    use crate::client::RpcClient;
    use crate::streams::{self, StreamRegistry, StreamSource};
    use crate::SocketServer;

    // The same hermetic shape as crates/hyprlayer-core/tests/headless_runtime.rs:
    // an unspawnable SHELL keeps the login-shell probe off the developer's rc files.
    fn environment(home: &Path) -> RuntimeEnvironment {
        let mut process_env = BTreeMap::new();
        process_env.insert(
            OsString::from("SHELL"),
            OsString::from(home.join("no-such-shell")),
        );
        RuntimeEnvironment {
            home_dir: home.to_path_buf(),
            process_cwd: home.to_path_buf(),
            process_env,
            env_overlay: Default::default(),
        }
    }

    fn paths(root: &Path) -> RuntimePaths {
        RuntimePaths {
            hyprlayer_config_file: root.join("config/hyprlayer.json"),
            app_config_dir: root.join("app-config"),
            app_data_dir: root.join("app-data"),
            default_worktree_root: root.join("worktrees"),
            default_repos_root: root.join("repos"),
            opencode_config_file: root.join("opencode/opencode.json"),
            opencode_auth_file: root.join("opencode/auth.json"),
            opencode_data_dir: root.join("opencode/data"),
            claude_config_dir: root.join("claude"),
            claude_managed_settings_files: Vec::new(),
            codex_home: root.join("codex"),
            agent_roots: Vec::new(),
            download_dir: root.join("downloads"),
        }
    }

    fn write_inputs(paths: &RuntimePaths) {
        for dir in [
            paths.hyprlayer_config_file.parent().unwrap(),
            &paths.app_config_dir,
            &paths.app_data_dir,
            paths.opencode_config_file.parent().unwrap(),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&paths.hyprlayer_config_file, r#"{"version": 4}"#).unwrap();
        std::fs::write(paths.app_config_dir.join("settings.json"), "{}").unwrap();
        std::fs::write(&paths.opencode_config_file, "{}").unwrap();
        std::fs::write(&paths.opencode_auth_file, "{}").unwrap();
    }

    async fn open_runtime(root: &Path) -> Arc<HyprlayerRuntime> {
        let paths = paths(root);
        write_inputs(&paths);
        let options = RuntimeOptions::new(environment(&root.join("home")), paths)
            .with_credential_lookup(Arc::new(NoCredentialLookup));
        let runtime = HyprlayerRuntime::open(options).await.expect("open");
        runtime.start().await.expect("start");
        runtime
    }

    #[tokio::test]
    async fn import_folder_over_the_socket_matches_the_direct_call() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let server = SocketServer::bind(runtime.clone(), &root.path().join("app-data"))
            .await
            .expect("bind");
        let socket_path = server.socket_path().to_path_buf();
        let stop = server.stop_handle();
        let serving = tokio::spawn(server.run());

        let client = RpcClient::connect_socket(&socket_path)
            .await
            .expect("connect");
        let folder = root.path().join("projects/notes");
        std::fs::create_dir_all(&folder).unwrap();
        let over_the_wire = client
            .import_local_folder(folder.display().to_string(), None)
            .await
            .expect("import over the socket");
        let direct = runtime
            .projects()
            .find(&over_the_wire.path)
            .await
            .expect("registry read")
            .expect("the imported project is registered");
        assert_eq!(
            serde_json::to_value(&over_the_wire).unwrap(),
            serde_json::to_value(&direct).unwrap()
        );

        let wire_err = client.get_doc("not-indexed".into()).await.unwrap_err();
        let direct_err = runtime.library().get("not-indexed").await.unwrap_err();
        assert_eq!(wire_err.code, direct_err.code());
        assert_eq!(wire_err.message, direct_err.to_string());
        assert_eq!(wire_err.remedy, direct_err.remedy());

        let (a, b, c) = tokio::join!(
            client.inspect_repo_path(folder.display().to_string()),
            client.list_sessions("none".into()),
            client.get_app_settings(),
        );
        a.expect("concurrent call one");
        assert!(b.expect("concurrent call two").is_empty());
        c.expect("concurrent call three");

        let mut events = client
            .take_events()
            .expect("the event stream is handed out once");
        assert!(client.take_events().is_none());
        let announced = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Some(RuntimeEvent::ProjectsChanged) => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(
            announced,
            "the import's ProjectsChanged must reach the client as an event frame"
        );

        stop.stop();
        serving.await.unwrap().unwrap();
        assert_eq!(
            client.get_app_settings().await.unwrap_err().code,
            "server_disconnected"
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn a_terminal_stream_delivers_shell_output_over_the_data_plane() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let server = SocketServer::bind(runtime.clone(), &root.path().join("app-data"))
            .await
            .expect("bind");
        let socket_path = server.socket_path().to_path_buf();
        let stop = server.stop_handle();
        let serving = tokio::spawn(server.run());

        let client = RpcClient::connect_socket(&socket_path)
            .await
            .expect("connect");
        let (_, _, mut events) = client
            .spawn_shell_terminal_stream("printf MARKER_OK".into(), 80, 24)
            .await
            .expect("open a shell terminal stream over the socket");

        let mut seen = String::new();
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = events.recv().await {
                if let TerminalEvent::Output { data } = event {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .unwrap_or_default();
                    seen.push_str(&String::from_utf8_lossy(&decoded));
                    if seen.contains("MARKER_OK") {
                        return true;
                    }
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(found, "expected MARKER_OK on the data plane, got {seen:?}");

        stop.stop();
        serving.await.unwrap().unwrap();
        runtime.shutdown().await;
    }

    fn seed_session(db: &Db, session_id: &str) {
        db.insert_workstream(NewWorkstream {
            project_path: "/p".into(),
            title: "t".into(),
            slug: format!("t-{session_id}"),
            branch: format!("hyprlayer/{session_id}"),
            worktree_path: "/wt".into(),
            base_branch: "main".into(),
            worktree_mode: WorktreeMode::Isolated,
        })
        .unwrap();
        let ws = db.list_active_workstreams().unwrap().pop().unwrap();
        db.insert_session(NewSession {
            id: session_id.into(),
            workstream_id: ws.id,
            stage: Stage::General,
            harness: Harness::Claude,
            cwd: "/wt".into(),
        })
        .unwrap();
    }

    fn turn_done(cost: f64) -> SessionEvent {
        SessionEvent::TurnDone {
            cost_usd: Some(cost),
            num_turns: None,
            context_tokens: None,
            turn_status: None,
        }
    }

    /// Forces the session attachment's 513-slot delivery queue
    /// (`runtime/sessions/delivery.rs:8`, `:49`) past its bound by ingressing
    /// far more events than a reader draining at capacity 1 can keep up with,
    /// then asserts the stream's own recovery — `log_after` plus a
    /// re-`attach`, both run inside `run_stream` — hands the client a gapless,
    /// ordered sequence with no sign a lag ever happened.
    #[tokio::test]
    async fn a_lagged_session_stream_recovers_with_a_gapless_ordered_sequence() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;

        let session_id = "sess-lag".to_string();
        seed_session(&runtime.test_db(), &session_id);

        let attachment = runtime
            .sessions()
            .attach(&session_id)
            .await
            .expect("attach");
        let ingress = runtime.test_session_ingress(&session_id).await;

        // Capacity 1 and undrained: `run_stream` stalls on its own second
        // send, so nothing keeps reading `live` while the burst below lands.
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1);
        let registry = StreamRegistry::default();
        streams::spawn_stream(
            runtime.clone(),
            1,
            StreamSource::Session {
                session_id: session_id.clone(),
                replay: attachment.replay,
                live: attachment.live,
            },
            tx,
            registry.clone(),
        );

        const TOTAL: usize = 600;
        for i in 0..TOTAL {
            ingress.send(turn_done(i as f64)).await.unwrap();
        }
        // Let the actor outrun the stalled reader and force the overflow
        // before this test ever drains a single frame.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let mut decoder = hyprlayer_proto::frame::Decoder::new();
        let mut costs = Vec::with_capacity(TOTAL);
        let ok = tokio::time::timeout(Duration::from_secs(10), async {
            while costs.len() < TOTAL {
                let Some(bytes) = rx.recv().await else {
                    return false;
                };
                decoder.feed(&bytes);
                let Some(hyprlayer_proto::frame::Frame::Data(payload)) =
                    decoder.next_frame().unwrap()
                else {
                    panic!("a stream event is always a data frame");
                };
                let (_, event_bytes) = hyprlayer_proto::frame::decode_stream_frame(&payload)
                    .expect("payload carries a stream id");
                match serde_json::from_slice::<SessionEvent>(event_bytes).unwrap() {
                    SessionEvent::TurnDone { cost_usd, .. } => costs.push(cost_usd),
                    other => panic!("expected TurnDone, got {other:?}"),
                }
            }
            true
        })
        .await
        .unwrap_or(false);

        assert!(ok, "only received {} of {TOTAL} events", costs.len());
        let expected: Vec<Option<f64>> = (0..TOTAL).map(|i| Some(i as f64)).collect();
        assert_eq!(
            costs, expected,
            "the recovered sequence must be gapless and ordered"
        );

        registry.abort(1);
        runtime.shutdown().await;
    }
}
