//! What an agent connection may ask: every `Request` classified, default deny.

use std::collections::HashSet;

use hyprlayer_core::index::QueryResult;
use hyprlayer_core::runtime::agent_control::{AgentControlService, AgentIdentity, ProjectScope};
use hyprlayer_core::runtime::goals::GoalTarget;
use hyprlayer_core::{CoreError, Result};
use hyprlayer_models::{RevealTarget, ScriptKind};

use crate::rpc::{Request, Response};

pub(crate) enum Access {
    Read(Scope),
    /// Mutates: `full` only, never report-only or an unasked codex session.
    Full(Scope),
    /// Never open to an agent, at any level; the reason names why.
    Excluded(&'static str),
    Denied,
}

pub(crate) enum Scope {
    /// A project path, which must be the caller's.
    Project(String),
    Workstream(String),
    Session(String),
    /// A PTY agent the server launched; never a plain shell or a script.
    Terminal(String),
    Goal(String),
    /// A goal on the caller's own session or one it spawned.
    OwnGoal(String),
    Doc(String),
    /// A doc mapped to the caller's project; shared and global docs belong to no project.
    WritableDoc(String),
    /// A writable doc too: an attached doc becomes readable through the workstream.
    AttachDoc {
        workstream_id: String,
        doc_path: String,
    },
    /// Shown in the caller's own workstream; a doc must be one it may read.
    Reveal(RevealTarget),
    /// A list verb: its result is cut to the caller's project after dispatch.
    Filtered,
    /// Acts only on the caller itself; read verbs only.
    Caller,
    /// A session the caller spawned, at any depth; `or_self` admits the caller too.
    Descendant {
        session_id: String,
        or_self: bool,
    },
}

fn descendant(session_id: &str) -> Access {
    Access::Full(Scope::Descendant {
        session_id: session_id.to_string(),
        or_self: false,
    })
}

/// No wildcard arm: a new RPC fails to compile until it is classified here.
pub(crate) fn access(request: &Request) -> Access {
    match request {
        Request::RemoveProject { .. } => Access::Excluded("it removes a project"),
        Request::QueryDocs { .. } => Access::Read(Scope::Filtered),
        Request::GetDoc { id, .. } => Access::Read(Scope::Doc(id.clone())),
        Request::SaveDoc { id, .. } => Access::Full(Scope::WritableDoc(id.clone())),
        // `create` files a global-scope doc outside the project even when one is named.
        Request::CreateDoc { input } if input.scope == "global" => {
            Access::Excluded("a global document is the user's")
        }
        Request::CreateDoc { input } => match &input.project_repo_path {
            Some(project) => Access::Full(Scope::Project(project.clone())),
            None => Access::Excluded("a document outside every project is the user's"),
        },
        Request::ListWorkstreams { .. } => Access::Read(Scope::Filtered),
        Request::DiscardProjectChanges { .. } => Access::Excluded("it discards uncommitted work"),
        Request::ListWorkstreamDocs { id, .. } => Access::Read(Scope::Workstream(id.clone())),
        Request::CreateWorkstream { project_path, .. } => {
            Access::Full(Scope::Project(project_path.clone()))
        }
        Request::SetWorkstreamStage { id, .. } => Access::Full(Scope::Workstream(id.clone())),
        Request::AttachWorkstreamDoc { id, doc_path, .. } => Access::Full(Scope::AttachDoc {
            workstream_id: id.clone(),
            doc_path: doc_path.clone(),
        }),
        Request::ArchiveWorkstream { .. } => Access::Excluded("it ends a workstream"),
        Request::DeleteArchivedWorkstream { .. } => {
            Access::Excluded("it deletes a workstream for good")
        }
        Request::WorktreeGitStatus { id, .. } => Access::Read(Scope::Workstream(id.clone())),
        Request::WorktreeDiscard { .. } => Access::Excluded("it discards uncommitted work"),
        Request::GetGoal { goal_id, .. } => Access::Read(Scope::Goal(goal_id.clone())),
        Request::StartGoal { request } => match &request.target {
            GoalTarget::Session { session_id } => Access::Full(Scope::Descendant {
                session_id: session_id.clone(),
                or_self: true,
            }),
            GoalTarget::NewSession { workstream_id, .. } => {
                Access::Full(Scope::Workstream(workstream_id.clone()))
            }
        },
        Request::PauseGoal { goal_id } => Access::Full(Scope::OwnGoal(goal_id.clone())),
        Request::ResumeGoal { goal_id } => Access::Full(Scope::OwnGoal(goal_id.clone())),
        Request::StopGoal { goal_id } => Access::Full(Scope::OwnGoal(goal_id.clone())),
        Request::UpdateGoal { goal_id, .. } => Access::Full(Scope::OwnGoal(goal_id.clone())),
        Request::ListGoals { .. } => Access::Read(Scope::Filtered),
        Request::WorkstreamPrStatus { id, .. } => Access::Read(Scope::Workstream(id.clone())),
        Request::MergeWorkstreamPr { .. } => Access::Excluded("merging is the user's call"),
        Request::RespondPermission { .. } => {
            Access::Excluded("only the user answers a permission prompt")
        }
        Request::SetSessionPermissionMode { .. } => {
            Access::Excluded("an agent never changes a permission mode")
        }
        Request::ListSessions { .. } => Access::Read(Scope::Filtered),
        Request::GetSessionLog { session_id, .. } => {
            Access::Read(Scope::Session(session_id.clone()))
        }
        Request::ListSessionTurns { session_id, .. } => {
            Access::Read(Scope::Session(session_id.clone()))
        }
        Request::ListActiveSessions { .. } => Access::Read(Scope::Filtered),
        Request::SetAppSettings { .. } => Access::Excluded("settings are the user's"),
        Request::AccountStatus { .. } => Access::Excluded("the Cloud account is the user's"),
        Request::AccountStartSignIn { .. } => Access::Excluded("the Cloud account is the user's"),
        Request::AccountFinishSignIn { .. } => Access::Excluded("the Cloud account is the user's"),
        Request::AccountSignOut { .. } => Access::Excluded("the Cloud account is the user's"),
        Request::ListPtyAgents { .. } => Access::Read(Scope::Filtered),
        Request::AnnounceAgent { .. } => Access::Read(Scope::Caller),
        Request::NotifyUser { .. } => Access::Read(Scope::Caller),
        Request::RequestAttention { .. } => Access::Read(Scope::Caller),
        Request::SpawnSession { request } | Request::SpawnAgent { request } => {
            Access::Full(Scope::Workstream(request.workstream_id.clone()))
        }
        Request::WaitSession { session_id, .. } => Access::Full(Scope::Session(session_id.clone())),
        Request::SendAgentMessage { session_id, .. } => {
            Access::Full(Scope::Session(session_id.clone()))
        }
        Request::InterruptSession { session_id } => descendant(session_id),
        Request::KillSession { session_id } => descendant(session_id),
        Request::SetSessionArchived { session_id, .. } => descendant(session_id),
        Request::AnswerQuestion { session_id, .. } => descendant(session_id),
        Request::DeleteSession { session_id } => descendant(session_id),
        Request::SetSessionTitle { session_id, .. } => Access::Full(Scope::Descendant {
            session_id: session_id.clone(),
            or_self: true,
        }),
        Request::SpawnPtyAgent { request } => {
            Access::Full(Scope::Workstream(request.workstream_id.clone()))
        }
        // Core also refuses a target the caller did not spawn.
        Request::SendPtyMessage { terminal_id, .. } => {
            Access::Full(Scope::Terminal(terminal_id.clone()))
        }
        Request::ReadTerminal { terminal_id, .. } => {
            Access::Read(Scope::Terminal(terminal_id.clone()))
        }
        Request::WaitPtyAgent { terminal_id, .. } => {
            Access::Full(Scope::Terminal(terminal_id.clone()))
        }
        Request::InterruptPtyAgent { terminal_id } => {
            Access::Full(Scope::Terminal(terminal_id.clone()))
        }
        Request::KillPtyAgent { terminal_id } => Access::Full(Scope::Terminal(terminal_id.clone())),
        Request::RequestReveal { target } => Access::Full(Scope::Reveal(target.clone())),
        Request::ScriptStart {
            kind: ScriptKind::Archive,
            ..
        } => Access::Excluded("the archive script is the user's"),
        Request::ScriptStart { workstream_id, .. } => {
            Access::Full(Scope::Workstream(workstream_id.clone()))
        }
        Request::ScriptStop { workstream_id } => {
            Access::Full(Scope::Workstream(workstream_id.clone()))
        }
        Request::AddReviewComment { req } => {
            Access::Full(Scope::Workstream(req.workstream_id.clone()))
        }
        Request::ResolveReviewComment { workstream_id, .. } => {
            Access::Full(Scope::Workstream(workstream_id.clone()))
        }
        Request::GetScriptRuntime { workstream_id, .. } => {
            Access::Read(Scope::Workstream(workstream_id.clone()))
        }
        Request::ListRunningScripts { .. } => Access::Read(Scope::Filtered),
        Request::GetScriptOutput { workstream_id, .. } => {
            Access::Read(Scope::Workstream(workstream_id.clone()))
        }
        Request::SaveAgentFile { .. } => Access::Excluded("agent definitions are the user's"),
        Request::DeleteAgentFile { .. } => Access::Excluded("agent definitions are the user's"),
        Request::InstallSkill { .. } => Access::Excluded("skill installs are the user's"),
        Request::RemoveSkill { .. } => Access::Excluded("skill installs are the user's"),
        Request::RestoreSkills { .. } => Access::Excluded("skill installs are the user's"),
        Request::UpdateAgentBundle { .. } => Access::Excluded("the agent bundle is the user's"),
        Request::InstallAgentBundleVersion { .. } => {
            Access::Excluded("the agent bundle is the user's")
        }
        Request::RepairAgentBundle { .. } => Access::Excluded("the agent bundle is the user's"),
        Request::GetBootstrap { .. }
        | Request::GetProject { .. }
        | Request::InspectRepoPath { .. }
        | Request::ImportProject { .. }
        | Request::ImportLocalFolder { .. }
        | Request::CloneProject { .. }
        | Request::CreateProject { .. }
        | Request::ListGithubOwners { .. }
        | Request::PublishProjectToGithub { .. }
        | Request::ReorderProjects { .. }
        | Request::ListRecentRepos { .. }
        | Request::ListThoughtsProfiles { .. }
        | Request::CreateThoughtsProfile { .. }
        | Request::SyncThoughts { .. }
        | Request::RunBackendHealth { .. }
        | Request::GetWorktreeFiles { .. }
        | Request::SetWorktreeFiles { .. }
        | Request::CheckWorktreeFiles { .. }
        | Request::BrowseDirectory { .. }
        | Request::FindProject { .. }
        | Request::ProjectSourceOf { .. }
        | Request::CountDocs { .. }
        | Request::MarkDocRead { .. }
        | Request::MarkDocUnread { .. }
        | Request::PatchDocFrontmatter { .. }
        | Request::SaveDocParts { .. }
        | Request::ResolveRelatedDocs { .. }
        | Request::ResolveDocPath { .. }
        | Request::DocWebUrl { .. }
        | Request::GetThoughtsGitStatus { .. }
        | Request::ListAllWorkstreams { .. }
        | Request::ListWorkstreamBases { .. }
        | Request::FetchProjectRemote { .. }
        | Request::GetBranchPrefixes { .. }
        | Request::SetBranchPrefixes { .. }
        | Request::RenameWorkstream { .. }
        | Request::PreviewBranchRename { .. }
        | Request::RenameWorkstreamBranch { .. }
        | Request::SetWorkstreamBaseBranch { .. }
        | Request::UpdateWorkstreamFromBase { .. }
        | Request::PullWorkstreamBranch { .. }
        | Request::FetchWorkstreamRemote { .. }
        | Request::ListProjectMemory { .. }
        | Request::SaveProjectMemoryFile { .. }
        | Request::ListArchivedWorkstreams { .. }
        | Request::RestoreWorkstream { .. }
        | Request::RefreshArchivedWorkstreams { .. }
        | Request::KeepArchivedWorkstream { .. }
        | Request::PurgeArchivedWorkstreams { .. }
        | Request::ReprovisionWorkstream { .. }
        | Request::WorkstreamPromptContext { .. }
        | Request::WorktreeStage { .. }
        | Request::WorktreeUnstage { .. }
        | Request::WorktreeFileSides { .. }
        | Request::ReviewChangedFiles { .. }
        | Request::ReviewFileSides { .. }
        | Request::GetReview { .. }
        | Request::SummarizeValidationReport { .. }
        | Request::SetReviewFileViewed { .. }
        | Request::EditReviewComment { .. }
        | Request::DeleteReviewComment { .. }
        | Request::ResolveReviewNote { .. }
        | Request::SubmitReview { .. }
        | Request::MarkReviewRoundDelivered { .. }
        | Request::DeriveGoal { .. }
        | Request::WorktreeListDir { .. }
        | Request::WorktreeReadFile { .. }
        | Request::WorktreeWriteFile { .. }
        | Request::ListSlashCommands { .. }
        | Request::WorktreeSearchFiles { .. }
        | Request::ListWorkstreamCommits { .. }
        | Request::WorktreeCommitFiles { .. }
        | Request::WorktreeCommitFileSides { .. }
        | Request::WatchWorktree { .. }
        | Request::ListWorkstreamOpenPrs { .. }
        | Request::PreviewWorkstreamPr { .. }
        | Request::AttachWorkstreamPr { .. }
        | Request::DetachWorkstreamPr { .. }
        | Request::SendUserMessage { .. }
        | Request::CancelQueuedMessage { .. }
        | Request::SetSessionModel { .. }
        | Request::SetSessionEffort { .. }
        | Request::SetSessionHarness { .. }
        | Request::AnnounceClient { .. }
        | Request::WatchSessions { .. }
        | Request::RecapSession { .. }
        | Request::LaunchAndConverse { .. }
        | Request::ImportAttachment { .. }
        | Request::SaveAttachment { .. }
        | Request::ReadAttachment { .. }
        | Request::SetBinaryOverride { .. }
        | Request::RedetectBinaries { .. }
        | Request::DetectCliInstallMethods { .. }
        | Request::HyprlayerCliUpdateCheck { .. }
        | Request::UpdateHyprlayerCli { .. }
        | Request::InstallHyprlayerCli { .. }
        | Request::GetAppSettings { .. }
        | Request::GithubUsername { .. }
        | Request::DefaultUsername { .. }
        | Request::ClaudeSyncModels { .. }
        | Request::SetQuitConfirm { .. }
        | Request::DefaultShellName { .. }
        | Request::WorktreeRoot { .. }
        | Request::SetForeground { .. }
        | Request::SetEnvOverlay { .. }
        | Request::OpencodeCatalog { .. }
        | Request::OpencodeAuthList { .. }
        | Request::OpencodeSetProviderKey { .. }
        | Request::OpencodeRemoveProvider { .. }
        | Request::SetOpencodeDefaultModel { .. }
        | Request::InstallOpencode { .. }
        | Request::ClaudeAuthStatus { .. }
        | Request::ClaudeProbe { .. }
        | Request::CodexProbe { .. }
        | Request::CodexLoginWithApiKey { .. }
        | Request::CodexLogout { .. }
        | Request::OllamaStatus { .. }
        | Request::OllamaSetEndpoint { .. }
        | Request::OllamaSetSelectedModels { .. }
        | Request::OllamaSetDefaultModel { .. }
        | Request::GenerateTitleClaude { .. }
        | Request::GenerateTitleOpencode { .. }
        | Request::GenerateTitleCodex { .. }
        | Request::TerminalWrite { .. }
        | Request::TerminalResize { .. }
        | Request::TerminalFocus { .. }
        | Request::TerminalTheme { .. }
        | Request::TerminalFrameAck { .. }
        | Request::TerminalClose { .. }
        | Request::AgentResize { .. }
        | Request::CaptureAgentSession { .. }
        | Request::MintCodexSession { .. }
        | Request::AgentSessionExists { .. }
        | Request::ReportAgentSignal { .. }
        | Request::ReportAgentStatus { .. }
        | Request::ModelSwitchPending { .. }
        | Request::PtyAgentSetModel { .. }
        | Request::PtyAgentSetEffort { .. }
        | Request::PtyAgentSetPermissionMode { .. }
        | Request::GetScripts { .. }
        | Request::SetScripts { .. }
        | Request::ScriptClearOutput { .. }
        | Request::GetAiStatus { .. }
        | Request::ProbeDefaultThoughts { .. }
        | Request::InitThoughtsRepo { .. }
        | Request::PreviewPromptImport { .. }
        | Request::ApplyPromptImport { .. }
        | Request::ExportPrompts { .. }
        | Request::ListAgentFiles { .. }
        | Request::NewAgentFilePath { .. }
        | Request::AgentContextFiles { .. }
        | Request::AgentMcpServers { .. }
        | Request::AgentSkills { .. }
        | Request::ListSkillFiles { .. }
        | Request::ReadSkillFile { .. }
        | Request::ReadSkillAsset { .. }
        | Request::PreviewSkillInstall { .. }
        | Request::SearchSkillSource { .. }
        | Request::SkillLeaderboard { .. }
        | Request::ListSkillLocks { .. }
        | Request::CheckSkillUpdates { .. }
        | Request::PreviewSkillUpdate { .. }
        | Request::ListSkillfishCandidates { .. }
        | Request::AgentHarnessCounts { .. }
        | Request::ExportAgentFile { .. }
        | Request::AgentBundleStatus { .. }
        | Request::AgentBundleVersions { .. } => Access::Denied,
    }
}

/// A filtered response's cut: the caller's project, and the page a doc query asked for.
pub(crate) struct Filter {
    scope: ProjectScope,
    page: Option<Page>,
}

struct Page {
    offset: Option<u32>,
    limit: Option<u32>,
}

/// Refuses what the agent may not ask; `Some` when the response must be filtered to its project.
/// A doc query loses its paging here, so the cut is taken over every match.
pub(crate) async fn authorize(
    control: &AgentControlService<'_>,
    identity: &AgentIdentity,
    request: &mut Request,
) -> Result<Option<Filter>> {
    let scope = match access(request) {
        Access::Read(scope) => scope,
        Access::Full(Scope::Caller) => {
            return Err(denied(format!(
                "{} has no target to scope",
                request.command()
            )))
        }
        Access::Full(scope) => {
            control.require_full(identity).await?;
            scope
        }
        Access::Excluded(reason) => {
            return Err(denied(format!(
                "{} is never open to agents: {reason}",
                request.command()
            )))
        }
        Access::Denied => {
            return Err(denied(format!(
                "{} is not open to agents",
                request.command()
            )))
        }
    };
    match scope {
        Scope::Project(path) => control.check_project(identity, &path)?,
        Scope::Workstream(id) => control.check_workstream(identity, &id).await?,
        Scope::Session(id) => control.check_session(identity, &id).await?,
        Scope::Terminal(id) => control.check_terminal(identity, &id).await?,
        Scope::Goal(id) => control.check_goal(identity, &id).await?,
        Scope::OwnGoal(id) => control.check_own_goal(identity, &id).await?,
        Scope::Doc(id) => control.check_doc(identity, &id, false).await?,
        Scope::WritableDoc(id) => control.check_doc(identity, &id, true).await?,
        Scope::AttachDoc {
            workstream_id,
            doc_path,
        } => {
            control.check_workstream(identity, &workstream_id).await?;
            control.check_doc(identity, &doc_path, true).await?
        }
        Scope::Reveal(RevealTarget::Doc { path }) => {
            control.check_doc(identity, &path, false).await?
        }
        Scope::Reveal(RevealTarget::File { .. } | RevealTarget::Diff { .. }) => {}
        Scope::Filtered => {
            let scope = control.project_scope(identity).await?;
            let page = match request {
                Request::QueryDocs { query } => Some(Page {
                    offset: query.offset.take(),
                    limit: query.limit.take(),
                }),
                _ => None,
            };
            return Ok(Some(Filter { scope, page }));
        }
        Scope::Caller => {}
        Scope::Descendant {
            session_id,
            or_self,
        } => {
            control.check_session(identity, &session_id).await?;
            control
                .check_descendant(identity, &session_id, or_self)
                .await?
        }
    }
    match request {
        Request::StartGoal { request } => {
            control
                .check_goal_checks(identity, &request.spec.checks)
                .await?
        }
        Request::UpdateGoal { patch, .. } => {
            if let Some(checks) = &patch.checks {
                control.check_goal_checks(identity, checks).await?
            }
        }
        _ => {}
    }
    Ok(None)
}

/// Fails closed: a filtered verb whose response this doesn't know is refused.
pub(crate) fn filter(response: Response, filter: Filter) -> Result<Response> {
    let Filter { scope, page } = filter;
    let ours = |workstream_id: &str| scope.workstream_ids.contains(workstream_id);
    Ok(match response {
        Response::ListWorkstreams(mut v) => {
            v.retain(|w| ours(&w.item.id));
            Response::ListWorkstreams(v)
        }
        Response::ListSessions(mut v) => {
            v.retain(|s| ours(&s.workstream_id));
            Response::ListSessions(v)
        }
        Response::ListActiveSessions(mut v) => {
            v.retain(|a| ours(&a.workstream.id));
            Response::ListActiveSessions(v)
        }
        Response::ListPtyAgents(mut v) => {
            v.retain(|a| ours(&a.workstream_id));
            Response::ListPtyAgents(v)
        }
        Response::ListRunningScripts(mut v) => {
            v.retain(|id| ours(id));
            Response::ListRunningScripts(v)
        }
        Response::ListGoals(mut v) => {
            v.retain(|g| ours(&g.workstream_id));
            Response::ListGoals(v)
        }
        Response::QueryDocs(result) => Response::QueryDocs(cut_docs(result, &scope, page)),
        other => {
            return Err(denied(format!(
                "{} has no project filter for agents",
                other.command()
            )))
        }
    })
}

/// Recounts the totals over the docs the caller may read, then applies its page.
fn cut_docs(mut result: QueryResult, scope: &ProjectScope, page: Option<Page>) -> QueryResult {
    result
        .items
        .retain(|d| scope.allows_doc(d.project.as_deref()));
    result.total = result.items.len() as u32;
    result.unread = result.items.iter().filter(|d| d.unread).count() as u32;
    result.projects = result
        .items
        .iter()
        .filter_map(|d| d.project.as_deref().or(d.fm.project.as_deref()))
        .collect::<HashSet<_>>()
        .len() as u32;
    if let Some(Page { offset, limit }) = page {
        let offset = offset.unwrap_or(0) as usize;
        let limit = limit.map_or(usize::MAX, |l| l as usize);
        result.items = result.items.into_iter().skip(offset).take(limit).collect();
    }
    result
}

fn denied(detail: String) -> CoreError {
    CoreError::AgentControlDenied { detail }
}
