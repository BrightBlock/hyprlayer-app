//! The command table: one line per request/response command, generating the
//! `Request` and `Response` enums, the server's `Dispatch` trait, and the typed
//! methods on `RpcClient`. It lives here rather than in `hyprlayer-proto`
//! because its types are core's (ADR-0009, Phase 2 note). The 11 streaming
//! commands join in Phase 4.
//!
//! Arguments mirror the Tauri command's; results are the core service's own
//! DTOs where one exists, or one of the mirror structs below where the desktop
//! used to assemble the view itself.

use std::collections::BTreeMap;

use hyprlayer_core::agents::{
    AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay, HarnessCounts,
};
use hyprlayer_core::branch_prefix::{BranchPrefix, PrefixSource};
use hyprlayer_core::claude::ClaudeProbe;
use hyprlayer_core::codex::CodexProbe;
use hyprlayer_core::config::ThoughtsProfile;
use hyprlayer_core::docs::frontmatter::FmPatch;
use hyprlayer_core::gitops::{
    CommitEntry, DirEntry, FileSides, GitFileEntry, ThoughtsRepoStatus, WorktreeGitStatus,
};
use hyprlayer_core::hyprcli::{AiStatus, AiVersions, CliUpdateCheck};
use hyprlayer_core::index::{DocCounts, DocMeta, QueryResult, ThoughtQuery};
use hyprlayer_core::models::session::QuestionAnswers;
use hyprlayer_core::models::{SkillScope, SkillTarget};
use hyprlayer_core::opencode::catalog::OpencodeCatalog;
use hyprlayer_core::opencode::config::OpencodeAuthEntry;
use hyprlayer_core::pr::{MergeMethod, PrStatus, PrSummary};
use hyprlayer_core::process::binaries::{BinaryKind, BinaryStatus};
use hyprlayer_core::repo_settings::ScriptSettings;
use hyprlayer_core::repos::{GithubRepo, RepoPathStatus};
use hyprlayer_core::review::diff::{ReviewFileList, ReviewFileSides};
use hyprlayer_core::runtime::agents::{BundleInstall, BundleReport, BundleStatus};
use hyprlayer_core::runtime::library::{
    CreateDocumentRequest, Document, FrontmatterEdit, RelatedLink,
};
use hyprlayer_core::runtime::projects::{
    BackendHealth, BootstrapResult, CreateProjectRequest, DirListing, GithubOwnersOutcome,
    ProjectInspection, ProjectView, SyncThoughtsResult, ThoughtsProbe, ThoughtsSource,
};
use hyprlayer_core::runtime::prompts::{
    PromptExportRequest, PromptExportResult, PromptImportDecision, PromptImportPreview,
    PromptImportResult,
};
use hyprlayer_core::runtime::providers::{ClaudeAuthStatus, OllamaStatus};
use hyprlayer_core::runtime::reviews::{
    AddReviewCommentRequest, ReviewComment, ReviewView, SetReviewFileViewedRequest,
    SubmitReviewRequest, SubmitReviewResult,
};
use hyprlayer_core::runtime::sessions::ActiveSession;
use hyprlayer_core::runtime::settings_service::{
    CliInstallMethod, CliInstallMethodId, CliUpdateResult,
};
use hyprlayer_core::runtime::workstreams::{
    ArchiveCleanupReport, ArchiveInfo, ProjectMemory, WorkstreamBases,
};
use hyprlayer_core::settings::AppSettings;
use hyprlayer_core::skill_sources::skills_sh::{
    SkillLeaderboardPage, SkillSearchResult, SkillsShView,
};
use hyprlayer_core::skill_sources::{
    SkillInstallPreview, SkillInstallReport, SkillLockStatus, SkillRestoreReport, SkillSourceInput,
    SkillUpdate, SkillfishCandidate,
};
use hyprlayer_core::skills::{AgentSkill, SkillAssetText, SkillFile, SlashCommand};
use hyprlayer_core::worktree::seed::SeedReport;
use hyprlayer_core::worktree::{PullOutcome, WorktreeStatus};
use hyprlayer_core::{
    LogCursor, ProjectSource, ScriptKind, Session, SessionLogEntry, Stage, Workstream,
    WorkstreamDoc, WorktreeMode,
};
use hyprlayer_models::{ScriptRuntime, SessionRev, SessionTurns};
use hyprlayer_proto::WireError;
use serde::{Deserialize, Serialize};

use crate::client::RpcClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkstreamView {
    pub item: Workstream,
    pub git: WorktreeStatus,
    pub seed: Option<SeedReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedWorkstreamView {
    pub item: Workstream,
    pub archive_path: String,
    pub archive_exists: bool,
    pub info: ArchiveInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedBranchPrefixes {
    pub prefixes: Vec<BranchPrefix>,
    pub default_prefix: String,
    pub repo: Vec<BranchPrefix>,
    pub source: PrefixSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptContext {
    pub branch: String,
    pub base_branch: String,
    pub changed_files: String,
    pub diff: String,
    pub last_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeFile {
    pub content: String,
    pub mtime_ms: f64,
}

/// `CreateProjectResult` with its `CoreError` lowered to the wire envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedProject {
    pub project: ProjectView,
    pub github_url: Option<String>,
    pub github_error: Option<WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnouncedClient {
    pub features: Vec<String>,
}

/// A session a client views and the rev it last applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedSession {
    pub session_id: String,
    pub rev: u64,
}

hyprlayer_proto::commands! {
    client = RpcClient;

    GetBootstrap: get_bootstrap() -> BootstrapResult;
    GetProject: get_project(path: String) -> Option<ProjectInspection>;
    InspectRepoPath: inspect_repo_path(path: String) -> RepoPathStatus;
    ImportProject: import_project(path: String, branch: Option<String>, directory_name: Option<String>, thoughts_dir: Option<String>, profile: Option<String>, bootstrap_user: Option<String>) -> ProjectView;
    ImportLocalFolder: import_local_folder(path: String, directory_name: Option<String>) -> ProjectView;
    CloneProject: clone_project(url: String, location: Option<String>, name: Option<String>, thoughts_dir: Option<String>, profile: Option<String>, bootstrap_user: Option<String>) -> ProjectView;
    CreateProject: create_project(req: CreateProjectRequest) -> CreatedProject;
    ListGithubOwners: list_github_owners() -> GithubOwnersOutcome;
    PublishProjectToGithub: publish_project_to_github(op_id: String, path: String, owner: String, name: String, is_private: bool) -> String;
    RemoveProject: remove_project(path: String, delete_files: bool) -> ();
    ReorderProjects: reorder_projects(paths: Vec<String>) -> ();
    ListRecentRepos: list_recent_repos() -> Vec<GithubRepo>;
    ListThoughtsProfiles: list_thoughts_profiles() -> Vec<ThoughtsProfile>;
    CreateThoughtsProfile: create_thoughts_profile(name: String, thoughts_repo: Option<String>) -> ThoughtsProfile;
    SyncThoughts: sync_thoughts(path: String, message: Option<String>) -> SyncThoughtsResult;
    RunBackendHealth: run_backend_health(path: String) -> BackendHealth;
    GetWorktreeFiles: get_worktree_files(path: String) -> Vec<String>;
    SetWorktreeFiles: set_worktree_files(path: String, patterns: Vec<String>) -> Vec<String>;
    CheckWorktreeFiles: check_worktree_files(path: String) -> SeedReport;
    BrowseDirectory: browse_directory(path: Option<String>) -> DirListing;
    FindProject: find_project(path: String) -> Option<ProjectView>;
    ProjectSourceOf: project_source_of(path: String) -> ProjectSource;

    QueryDocs: query_docs(query: ThoughtQuery) -> QueryResult;
    CountDocs: count_docs(project: Option<String>, filters: Option<ThoughtQuery>) -> DocCounts;
    MarkDocRead: mark_doc_read(id: String) -> ();
    MarkDocUnread: mark_doc_unread(id: String) -> ();
    GetDoc: get_doc(id: String) -> Document;
    SaveDoc: save_doc(id: String, content: String, base_mtime_ms: f64) -> DocMeta;
    PatchDocFrontmatter: patch_doc_frontmatter(id: String, patch: FmPatch, base_mtime_ms: f64) -> DocMeta;
    SaveDocParts: save_doc_parts(id: String, body: String, frontmatter: FrontmatterEdit, base_mtime_ms: f64) -> DocMeta;
    ResolveRelatedDocs: resolve_related_docs(id: String, values: Vec<String>) -> Vec<RelatedLink>;
    CreateDoc: create_doc(input: CreateDocumentRequest) -> DocMeta;
    ResolveDocPath: resolve_doc_path(id: String) -> String;
    DocWebUrl: doc_web_url(id: String) -> Option<String>;
    GetThoughtsGitStatus: get_thoughts_git_status(project_path: String) -> ThoughtsRepoStatus;

    ListWorkstreams: list_workstreams(project_path: String, include_archived: bool) -> Vec<WorkstreamView>;
    ListAllWorkstreams: list_all_workstreams() -> Vec<WorkstreamView>;
    ListWorkstreamBases: list_workstream_bases(project_path: String) -> WorkstreamBases;
    FetchProjectRemote: fetch_project_remote(project_path: String) -> ();
    DiscardProjectChanges: discard_project_changes(project_path: String) -> u32;
    CreateWorkstream: create_workstream(project_path: String, title: String, base: Option<String>, mode: WorktreeMode, branch: Option<String>, branch_prefix: Option<String>) -> WorkstreamView;
    GetBranchPrefixes: get_branch_prefixes(project_path: String) -> ResolvedBranchPrefixes;
    SetBranchPrefixes: set_branch_prefixes(project_path: String, prefixes: Vec<BranchPrefix>) -> Vec<BranchPrefix>;
    RenameWorkstream: rename_workstream(id: String, title: String) -> ();
    SetWorkstreamBaseBranch: set_workstream_base_branch(id: String, base: String) -> ();
    UpdateWorkstreamFromBase: update_workstream_from_base(id: String) -> PullOutcome;
    FetchWorkstreamRemote: fetch_workstream_remote(id: String) -> ();
    SetWorkstreamStage: set_workstream_stage(id: String, stage: Stage) -> ();
    AttachWorkstreamDoc: attach_workstream_doc(id: String, doc_path: String, role: String) -> Vec<WorkstreamDoc>;
    DetachWorkstreamDoc: detach_workstream_doc(id: String, doc_path: String) -> Vec<WorkstreamDoc>;
    ListWorkstreamDocs: list_workstream_docs(id: String) -> Vec<WorkstreamDoc>;
    ListProjectMemory: list_project_memory(id: String) -> ProjectMemory;
    SaveProjectMemoryFile: save_project_memory_file(id: String, name: String, content: String, base_mtime_ms: f64) -> ();
    ArchiveWorkstream: archive_workstream(id: String, skip_scripts: bool) -> ();
    ListArchivedWorkstreams: list_archived_workstreams() -> Vec<ArchivedWorkstreamView>;
    RestoreWorkstream: restore_workstream(id: String, target: Option<String>) -> WorkstreamView;
    DeleteArchivedWorkstream: delete_archived_workstream(id: String) -> ();
    CleanupArchivedWorkstreams: cleanup_archived_workstreams(before_ms: Option<f64>) -> ArchiveCleanupReport;
    RefreshArchivedWorkstreams: refresh_archived_workstreams() -> u32;
    KeepArchivedWorkstream: keep_archived_workstream(id: String) -> ();
    PurgeArchivedWorkstreams: purge_archived_workstreams(ids: Vec<String>) -> ArchiveCleanupReport;
    ReprovisionWorkstream: reprovision_workstream(id: String) -> WorkstreamView;
    WorktreeGitStatus: worktree_git_status(id: String) -> WorktreeGitStatus;
    WorkstreamPromptContext: workstream_prompt_context(id: String) -> PromptContext;
    WorktreeStage: worktree_stage(id: String, paths: Vec<String>) -> ();
    WorktreeUnstage: worktree_unstage(id: String, paths: Vec<String>) -> ();
    WorktreeDiscard: worktree_discard(id: String, paths: Vec<String>) -> ();
    WorktreeFileSides: worktree_file_sides(id: String, path: String, staged: bool) -> FileSides;
    ReviewChangedFiles: review_changed_files(workstream_id: String) -> ReviewFileList;
    ReviewFileSides: review_file_sides(workstream_id: String, path: String) -> ReviewFileSides;
    GetReview: get_review(workstream_id: String) -> ReviewView;
    SetReviewFileViewed: set_review_file_viewed(req: SetReviewFileViewedRequest) -> ();
    AddReviewComment: add_review_comment(req: AddReviewCommentRequest) -> ReviewComment;
    EditReviewComment: edit_review_comment(workstream_id: String, comment_id: String, body: String) -> ();
    DeleteReviewComment: delete_review_comment(workstream_id: String, comment_id: String) -> ();
    ResolveReviewComment: resolve_review_comment(workstream_id: String, comment_id: String, resolved: bool) -> ();
    ResolveReviewNote: resolve_review_note(workstream_id: String, note_id: String, resolved: bool) -> ();
    SubmitReview: submit_review(req: SubmitReviewRequest) -> SubmitReviewResult;
    MarkReviewRoundDelivered: mark_review_round_delivered(workstream_id: String, round: u32) -> ();
    WorktreeListDir: worktree_list_dir(id: String, path: String) -> Vec<DirEntry>;
    WorktreeReadFile: worktree_read_file(id: String, path: String) -> WorktreeFile;
    WorktreeWriteFile: worktree_write_file(id: String, path: String, content: String, base_mtime_ms: f64) -> f64;
    ListSlashCommands: list_slash_commands(id: String) -> Vec<SlashCommand>;
    WorktreeSearchFiles: worktree_search_files(id: String, query: String, limit: u32) -> Vec<String>;
    ListWorkstreamCommits: list_workstream_commits(id: String) -> Vec<CommitEntry>;
    WorktreeCommitFiles: worktree_commit_files(id: String, sha: String) -> Vec<GitFileEntry>;
    WorktreeCommitFileSides: worktree_commit_file_sides(id: String, sha: String, path: String) -> FileSides;
    WatchWorktree: watch_worktree(workstream_id: Option<String>) -> ();
    WorkstreamPrStatus: workstream_pr_status(id: String) -> Option<PrStatus>;
    ListWorkstreamOpenPrs: list_workstream_open_prs(id: String, query: String) -> Vec<PrSummary>;
    PreviewWorkstreamPr: preview_workstream_pr(id: String, pr_number: u32) -> Option<PrSummary>;
    AttachWorkstreamPr: attach_workstream_pr(id: String, pr_number: u32) -> Option<PrStatus>;
    DetachWorkstreamPr: detach_workstream_pr(id: String) -> ();
    MergeWorkstreamPr: merge_workstream_pr(id: String, method: MergeMethod) -> ();

    SendUserMessage: send_user_message(session_id: String, text: String, attached_docs: Vec<String>, prompt_name: Option<String>, client_message_id: Option<String>, base_rev: Option<u64>) -> ();
    RespondPermission: respond_permission(session_id: String, request_id: String, allow: bool, message: Option<String>) -> ();
    AnswerQuestion: answer_question(session_id: String, request_id: String, answers: QuestionAnswers) -> ();
    InterruptSession: interrupt_session(session_id: String) -> ();
    KillSession: kill_session(session_id: String) -> ();
    SetSessionModel: set_session_model(session_id: String, model: Option<String>) -> ();
    SetSessionEffort: set_session_effort(session_id: String, effort: Option<String>) -> ();
    ListSessions: list_sessions(workstream_id: String) -> Vec<Session>;
    GetSessionLog: get_session_log(session_id: String) -> Vec<SessionLogEntry>;
    ListSessionTurns: list_session_turns(session_id: String, cursor: LogCursor, limit: u32) -> SessionTurns;
    AnnounceClient: announce_client(client_id: String, device: String, capabilities: Vec<String>) -> AnnouncedClient;
    WatchSessions: watch_sessions(sessions: Vec<WatchedSession>) -> Vec<SessionRev>;
    ListActiveSessions: list_active_sessions() -> Vec<ActiveSession>;
    SetSessionTitle: set_session_title(session_id: String, title: Option<String>) -> ();
    SetSessionArchived: set_session_archived(session_id: String, archived: bool) -> ();
    DeleteSession: delete_session(session_id: String) -> ();
    LaunchAndConverse: launch_and_converse(session_id: String) -> ();

    ImportAttachment: import_attachment(workstream_id: String, src_path: String) -> String;
    SaveAttachment: save_attachment(workstream_id: String, file_name: String, data: Vec<u8>) -> String;
    ReadAttachment: read_attachment(path: String) -> Vec<u8>;

    SetBinaryOverride: set_binary_override(kind: BinaryKind, path: Option<String>) -> Vec<BinaryStatus>;
    RedetectBinaries: redetect_binaries() -> Vec<BinaryStatus>;
    DetectCliInstallMethods: detect_cli_install_methods() -> Vec<CliInstallMethod>;
    HyprlayerCliUpdateCheck: hyprlayer_cli_update_check() -> CliUpdateCheck;
    UpdateHyprlayerCli: update_hyprlayer_cli() -> CliUpdateResult;
    InstallHyprlayerCli: install_hyprlayer_cli(method: CliInstallMethodId) -> ();
    GetAppSettings: get_app_settings() -> AppSettings;
    SetAppSettings: set_app_settings(settings: AppSettings) -> AppSettings;
    GithubUsername: github_username() -> Option<String>;
    DefaultUsername: default_username() -> String;
    ClaudeSyncModels: claude_sync_models(add: Vec<String>, seen: Vec<String>) -> AppSettings;
    SetQuitConfirm: set_quit_confirm(confirm: bool) -> ();
    DefaultShellName: default_shell_name() -> String;
    WorktreeRoot: worktree_root() -> String;
    SetForeground: set_foreground(foreground: bool) -> ();
    SetEnvOverlay: set_env_overlay(vars: BTreeMap<String, String>) -> ();

    OpencodeCatalog: opencode_catalog(force: bool) -> OpencodeCatalog;
    OpencodeAuthList: opencode_auth_list() -> Vec<OpencodeAuthEntry>;
    OpencodeSetProviderKey: opencode_set_provider_key(provider: String, key: String) -> Vec<OpencodeAuthEntry>;
    OpencodeRemoveProvider: opencode_remove_provider(provider: String) -> Vec<OpencodeAuthEntry>;
    SetOpencodeDefaultModel: set_opencode_default_model(model: Option<String>) -> ();
    InstallOpencode: install_opencode() -> ();
    ClaudeAuthStatus: claude_auth_status() -> ClaudeAuthStatus;
    ClaudeProbe: claude_probe() -> ClaudeProbe;
    CodexProbe: codex_probe() -> CodexProbe;
    CodexLoginWithApiKey: codex_login_with_api_key(api_key: String) -> CodexProbe;
    CodexLogout: codex_logout() -> CodexProbe;
    OllamaStatus: ollama_status() -> OllamaStatus;
    OllamaSetEndpoint: ollama_set_endpoint(endpoint: Option<String>) -> OllamaStatus;
    OllamaSetSelectedModels: ollama_set_selected_models(models: Vec<String>) -> ();
    OllamaSetDefaultModel: ollama_set_default_model(model: Option<String>) -> ();
    GenerateTitleClaude: generate_title_claude(flag: String, prompt: String) -> String;
    GenerateTitleOpencode: generate_title_opencode(model: String, prompt: String) -> String;
    GenerateTitleCodex: generate_title_codex(model: String, prompt: String) -> String;

    TerminalWrite: terminal_write(terminal_id: String, data: String) -> ();
    TerminalResize: terminal_resize(terminal_id: String, cols: u16, rows: u16) -> ();
    TerminalClose: terminal_close(terminal_id: String) -> ();
    AgentResize: agent_resize(terminal_id: String, cols: u16, rows: u16) -> ();
    CaptureAgentSession: capture_agent_session(workstream_id: String, engine_id: String, after_ms: f64, exclude: Vec<String>) -> Option<String>;
    MintCodexSession: mint_codex_session(workstream_id: String, command: String) -> Option<String>;
    AgentSessionExists: agent_session_exists(workstream_id: String, engine_id: String, session_id: String) -> bool;

    GetScripts: get_scripts(project_path: String) -> ScriptSettings;
    SetScripts: set_scripts(project_path: String, scripts: ScriptSettings) -> ();
    ScriptStop: script_stop(workstream_id: String) -> ();
    GetScriptRuntime: get_script_runtime(workstream_id: String) -> ScriptRuntime;
    ListRunningScripts: list_running_scripts() -> Vec<String>;
    GetScriptOutput: get_script_output(workstream_id: String, kind: ScriptKind) -> Option<String>;
    ScriptClearOutput: script_clear_output(workstream_id: String, kind: ScriptKind, terminal_id: Option<String>) -> ();

    GetAiStatus: get_ai_status() -> AiStatus;
    ProbeDefaultThoughts: probe_default_thoughts() -> ThoughtsProbe;
    InitThoughtsRepo: init_thoughts_repo(user: String, source: ThoughtsSource) -> BootstrapResult;

    PreviewPromptImport: preview_prompt_import(source: String, taken_chords: Vec<String>) -> PromptImportPreview;
    ApplyPromptImport: apply_prompt_import(decision: PromptImportDecision) -> PromptImportResult;
    ExportPrompts: export_prompts(request: PromptExportRequest) -> PromptExportResult;

    ListAgentFiles: list_agent_files(project_path: Option<String>) -> Vec<AgentFile>;
    SaveAgentFile: save_agent_file(agent: AgentFile) -> AgentFile;
    DeleteAgentFile: delete_agent_file(path: String) -> ();
    NewAgentFilePath: new_agent_file_path(name: String, source: String, project_path: Option<String>) -> Option<String>;
    AgentContextFiles: agent_context_files(project_path: Option<String>) -> Vec<AgentContextFile>;
    AgentMcpServers: agent_mcp_servers(project_path: Option<String>) -> Vec<AgentMcpServer>;
    AgentSkills: agent_skills(project_path: Option<String>) -> Vec<AgentSkill>;
    ListSkillFiles: list_skill_files(project_path: Option<String>) -> Vec<SkillFile>;
    ReadSkillFile: read_skill_file(project_path: Option<String>, path: String) -> String;
    ReadSkillAsset: read_skill_asset(project_path: Option<String>, skill_path: String, rel_path: String) -> SkillAssetText;
    PreviewSkillInstall: preview_skill_install(project_path: Option<String>, source: SkillSourceInput) -> SkillInstallPreview;
    InstallSkill: install_skill(project_path: Option<String>, token: String, scope: SkillScope, targets: Vec<SkillTarget>) -> SkillInstallReport;
    RemoveSkill: remove_skill(project_path: Option<String>, scope: SkillScope, name: String) -> ();
    SearchSkillSource: search_skill_source(query: String) -> Vec<SkillSearchResult>;
    SkillLeaderboard: skill_leaderboard(view: SkillsShView, page: u32) -> SkillLeaderboardPage;
    ListSkillLocks: list_skill_locks(project_path: Option<String>) -> Vec<SkillLockStatus>;
    CheckSkillUpdates: check_skill_updates(project_path: Option<String>) -> Vec<SkillUpdate>;
    PreviewSkillUpdate: preview_skill_update(project_path: Option<String>, scope: SkillScope, name: String) -> SkillInstallPreview;
    RestoreSkills: restore_skills(project_path: Option<String>, scope: SkillScope) -> SkillRestoreReport;
    ListSkillfishCandidates: list_skillfish_candidates(project_path: Option<String>) -> Vec<SkillfishCandidate>;
    AgentHarnessCounts: agent_harness_counts() -> Vec<HarnessCounts>;
    ExportAgentFile: export_agent_file(agent: AgentFile, overlay: AgentOverlay) -> String;
    AgentBundleStatus: agent_bundle_status() -> BundleStatus;
    UpdateAgentBundle: update_agent_bundle() -> BundleReport;
    AgentBundleVersions: agent_bundle_versions() -> AiVersions;
    InstallAgentBundleVersion: install_agent_bundle_version(version: Option<String>) -> BundleInstall;
    RepairAgentBundle: repair_agent_bundle() -> BundleInstall;
}
