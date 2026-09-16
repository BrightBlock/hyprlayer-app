use std::path::PathBuf;

use crate::scripts::ScriptKind;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("hyprlayer config not found at {path}")]
    ConfigMissing { path: PathBuf },

    #[error("hyprlayer config schema version {found} is newer than this app supports (max {max})")]
    ConfigVersionTooNew { found: u64, max: u64 },

    #[error("hyprlayer config is schema version {found} and needs migration to {required}")]
    ConfigNeedsMigration { found: u64, required: u64 },

    #[error("failed to parse hyprlayer config at {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("required binary `{name}` was not found")]
    BinaryMissing { name: String },

    #[error("`{name}` {found} is older than the minimum supported {min}")]
    BinaryTooOld {
        name: String,
        found: String,
        min: String,
    },

    #[error(fmt = fmt_exec)]
    Exec {
        program: String,
        exit: Option<i32>,
        stderr_tail: String,
    },

    #[error("`{program}` timed out after {seconds}s")]
    ExecTimeout { program: String, seconds: u64 },

    #[error("failed to parse output of `{program}`: {detail}")]
    OutputParse { program: String, detail: String },

    #[error("stored session event could not be decoded: {detail}")]
    SessionEventDecode { detail: String },

    #[error("session {session_id} already has a different event recorded at seq {seq}")]
    SessionSequenceConflict { session_id: String, seq: i64 },

    #[error("session {session_id} was rewound; the stream is now at epoch {epoch}")]
    SessionEpochAdvanced { session_id: String, epoch: u64 },

    #[error("Thoughts are not configured yet.")]
    ThoughtsNotConfigured,

    #[error("GitHub Copilot models are no longer supported")]
    OpencodeProviderUnsupported,

    #[error("OpenCode does not currently offer `{model}`")]
    OpencodeModelUnavailable { model: String },

    #[error("session {session_id} has no user turn #{user_turn}")]
    UserTurnNotFound { session_id: String, user_turn: u32 },

    #[error("No live session with id {session_id}")]
    SessionNotFound { session_id: String },

    #[error("another Hyprlayer runtime is already using the profile at {path}")]
    RuntimeInUse { path: PathBuf },

    #[error("the runtime has already been started")]
    RuntimeAlreadyStarted,

    #[error("the runtime has been stopped")]
    RuntimeStopped,

    #[error("credential lookup failed: {detail}")]
    CredentialLookup { detail: String },

    #[error("no {provider} credentials found")]
    ProviderCredentialsMissing { provider: String },

    #[error("{provider} request failed: {detail}")]
    ProviderRequestFailed { provider: String, detail: String },

    #[error("enter an API key first")]
    ApiKeyEmpty,

    #[error("{provider} produced no title")]
    TitleGenerationEmpty { provider: String },

    #[error("{detail}")]
    Opencode { detail: String },

    #[error("runtime migration ledger at {path} is not valid JSON: {detail}")]
    RuntimeMigrationLedgerInvalid { path: PathBuf, detail: String },

    #[error(
        "runtime migration ledger at {path} is version {found}, newer than this app supports (max {max})"
    )]
    RuntimeMigrationLedgerTooNew { path: PathBuf, found: u64, max: u64 },

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("worktree has {0} uncommitted change(s)")]
    WorktreeDirty(u32),

    #[error("the project checkout has {count} uncommitted change(s)")]
    CheckoutDirty { count: u32 },

    #[error("branch `{branch}` has {count} unpushed/unmerged commit(s)")]
    WorktreeUnpushed { branch: String, count: u32 },

    #[error("unsafe or out-of-worktree path: {path}")]
    InvalidPath { path: String },

    #[error("failed to parse {path}: {message}")]
    RepoSettingsParse { path: PathBuf, message: String },

    #[error("failed to parse Omarchy palette at {path}: {message}")]
    OmarchyPaletteParse { path: PathBuf, message: String },

    #[error("{detail}")]
    ProfileInvalid { detail: String },

    #[error("profile \"{name}\" already exists")]
    ProfileExists { name: String },

    #[error("{detail}")]
    BranchPrefixInvalid { detail: String },

    #[error("No hyprlayer config exists yet; a username is needed to create one.")]
    BootstrapUserRequired,

    #[error("{path} already exists")]
    RepoAlreadyExists { path: PathBuf, offer_remove: bool },

    #[error("{path} is not a directory")]
    PathNotADirectory { path: String },

    #[error("{path} is a bare git repository")]
    BareRepo { path: String },

    #[error("{path} is already added as a {existing_kind} project.")]
    ProjectKindConflict {
        path: String,
        existing_kind: &'static str,
    },

    #[error("{path} is a git repository.")]
    FolderIsGitRepo { path: String },

    #[error("A repository URL is required.")]
    RepoUrlRequired,

    #[error("Couldn't derive a project name from {url}.")]
    RepoNameUndeducible { url: String },

    #[error("A project name is required.")]
    ProjectNameRequired,

    #[error("`{name}` can't be used as a folder name.")]
    ProjectNameInvalid { name: String },

    #[error("`{name}` can't be used as a GitHub repository name.")]
    GithubNameInvalid { name: String },

    #[error("This repository already points at {origin}.")]
    GithubOriginExists { origin: String },

    #[error("{}", .0.message)]
    GithubOperation(crate::repos::GithubFailure),

    #[error(fmt = fmt_active_workstreams)]
    ActiveWorkstreams { count: usize },

    #[error("No workstream with id {id}")]
    WorkstreamNotFound { id: String },

    #[error("Workstreams need a title.")]
    WorkstreamTitleRequired,

    #[error("{path} is no longer a directory.")]
    WorkstreamFolderMissing { path: String },

    #[error("This project is a git repository, so its workstreams need a branch.")]
    WorkstreamModeNeedsBranch,

    #[error("Pick a branch to start the {mode} workstream from.")]
    WorkstreamNeedsBranch { mode: &'static str },

    #[error("`{branch}` is not a local or remote branch.")]
    WorkstreamBranchInvalid { branch: String },

    #[error("This project already has an in-place workstream.")]
    InplaceWorkstreamExists,

    #[error("The Archive script failed. Fix it and retry, or skip it to archive anyway.")]
    ArchiveScriptFailed,

    #[error(fmt = fmt_script_empty)]
    ScriptEmpty { kind: ScriptKind },

    #[error("{title} is not archived.")]
    WorkstreamNotArchived { title: String, actionable: bool },

    #[error("The backup directory for this workstream is gone ({path}).")]
    WorkstreamArchiveMissing { path: String },

    #[error("Another workstream took the original path ({path}).")]
    WorkstreamRestoreTargetOccupied { path: String },

    #[error("This workstream runs in a plain folder, so it has no thoughts links.")]
    WorkstreamFolderNoThoughts,

    #[error("The worktree for this workstream is missing ({path}).")]
    WorkstreamWorktreeMissing { path: String },

    #[error("{path} is not a git repository")]
    NotAGitRepo { path: String },

    #[error("The document changed on disk since you opened it.")]
    DocConflict,

    #[error("Document is not in the thoughts index: {id}")]
    DocNotIndexed { id: String },

    #[error("document was written but not indexable")]
    DocNotIndexable,

    #[error("{path} is not inside any configured thoughts root")]
    DocOutsideRoots { path: String },

    #[error("{path} already exists")]
    DocExists { path: String },

    #[error("No hyprlayer project is mapped at {path}")]
    ProjectNotFound { path: String },

    #[error("The {backend} backend has no local document tree to create files in.")]
    BackendNotFilesystem { backend: String },

    #[error("No thoughts repository is configured.")]
    ThoughtsRepoMissing,

    #[error("The {backend} backend does not sync through a git repository.")]
    BackendNotGit { backend: String },

    #[error("{detail}")]
    AgentBundleUpdateFailed { detail: String },

    #[error("Bundle versions require both Claude Code and Codex to be provisioned.")]
    AgentBundleNotManageable,

    #[error("No agent bundle is published for version {version}.")]
    AgentBundleVersionUnavailable { version: String },

    #[error("That file is too large to be a prompt export.")]
    PromptsFileTooLarge,

    #[error("Couldn't read {path}: {detail}")]
    PromptsFileUnreadable { path: String, detail: String },

    #[error("{detail}")]
    PromptsFileInvalid { detail: String },

    #[error("The prompt file changed on disk after it was previewed.")]
    PromptsSourceChanged,

    #[error("Node.js (npm) isn't installed, so OpenCode can't be installed automatically.")]
    NpmMissing,

    #[error("Couldn't install OpenCode via npm. {detail}")]
    OpencodeInstallFailed { detail: String },

    #[error("That install method isn't available on this platform.")]
    CliInstallUnsupported,

    #[error("{action} is not available while connected to the remote host {host}.")]
    RemoteUnsupported { action: String, host: String },

    #[error("This workstream runs in a plain folder, so it has no branches or PRs.")]
    WorkstreamFolderNoGit,

    #[error(fmt = fmt_pr_not_found)]
    PrNotFound { pr_number: u32, detail: String },

    // Keeps the legacy `internal` code (v0.7 envelope, pinned by the workstreams
    // fixture), like TerminalCommandEmpty below.
    #[error("base branch cannot be empty")]
    WorkstreamBaseBranchEmpty,

    // Keeps the legacy `internal` code (v0.7 envelope, pinned by the terminals
    // fixture): commands/terminal.rs's agent_open guard predates a domain-specific
    // code and this reproduces it exactly rather than inventing a new one.
    #[error("multiplexer engine command is empty")]
    TerminalCommandEmpty,

    #[error("{original}")]
    RollbackFailed {
        original: Box<CoreError>,
        path: PathBuf,
        reason: String,
    },

    #[error("could not patch frontmatter: {0}")]
    Frontmatter(#[from] crate::docs::frontmatter::FmError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn fmt_exec(
    program: &str,
    exit: &Option<i32>,
    stderr_tail: &str,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    let detail = stderr_tail.trim();
    if !detail.is_empty() {
        return f.write_str(detail);
    }
    match exit {
        Some(code) => write!(f, "`{program}` failed with exit code {code}"),
        None => write!(f, "`{program}` was terminated"),
    }
}

fn fmt_active_workstreams(count: &usize, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
        f,
        "This project still has {count} active workstream{}.",
        if *count == 1 { "" } else { "s" }
    )
}

fn fmt_script_empty(kind: &ScriptKind, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
        f,
        "No {} script is configured for this project.",
        kind.as_str()
    )
}

fn fmt_pr_not_found(
    pr_number: &u32,
    detail: &str,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    if detail.is_empty() {
        write!(f, "No pull request #{pr_number} found in this repository")
    } else {
        f.write_str(detail)
    }
}

impl CoreError {
    pub fn remedy(&self) -> Option<String> {
        match self {
            CoreError::ConfigMissing { .. } => Some(
                "Run `hyprlayer thoughts init` in a repository, or add a project from the app to bootstrap the config.".into(),
            ),
            CoreError::ConfigVersionTooNew { .. } => {
                Some("Update the Hyprlayer app to a newer version.".into())
            }
            CoreError::ConfigNeedsMigration { .. } => Some(
                "Run any hyprlayer command in a terminal (e.g. `hyprlayer storage info`) to upgrade the config file. The app picks it up on the next launch.".into(),
            ),
            CoreError::BinaryMissing { name } => Some(match name.as_str() {
                "hyprlayer" => {
                    "Install the hyprlayer CLI: `brew install brightblock/tap/hyprlayer` (macOS) or `winget install BrightBlock.Hyprlayer` (Windows), or set its path in Settings.".into()
                }
                "claude" => {
                    "Install Claude Code from https://claude.com/claude-code, or set its path in Settings.".into()
                }
                "gh" => "Install the GitHub CLI from https://cli.github.com (optional: PRs degrade to browser links without it).".into(),
                "git" => "Install git (e.g. via Xcode Command Line Tools or https://git-scm.com).".into(),
                other => format!("Install `{other}` or set its path in Settings."),
            }),
            CoreError::BinaryTooOld { name, min, .. } => {
                Some(format!("Update `{name}` to version {min} or newer."))
            }
            CoreError::WorkstreamWorktreeMissing { .. } => {
                Some("The worktree may have been deleted; archive this workstream.".into())
            }
            CoreError::Exec { program, stderr_tail, .. } => {
                if program.contains("gh") && stderr_tail.contains("auth") {
                    Some("Run `gh auth login` in a terminal.".into())
                } else {
                    None
                }
            }
            CoreError::WorktreeDirty(_) => Some(
                "Commit or discard the changes in the worktree first, or archive with 'discard changes'.".into(),
            ),
            CoreError::CheckoutDirty { .. } => Some(
                "Commit or discard the changes in the project checkout first, or start this workstream in a dedicated worktree instead.".into(),
            ),
            CoreError::WorktreeUnpushed { .. } => Some(
                "Push the branch (or merge it) first, or archive with 'discard changes' to drop the commits.".into(),
            ),
            CoreError::RepoSettingsParse { .. } => {
                Some("Fix the TOML syntax in .hyprlayer/settings.toml.".into())
            }
            CoreError::ProfileExists { .. } => {
                Some("Pick a different name, or select the existing profile.".into())
            }
            CoreError::BranchPrefixInvalid { .. } => Some(
                "Use letters, digits, '-', '_' and '/' — for example `fix/` or `hotfix-`.".into(),
            ),
            CoreError::RuntimeInUse { .. } => Some(
                "Quit any other Hyprlayer window or headless process using this profile, then try again."
                    .into(),
            ),
            CoreError::ProviderCredentialsMissing { provider } => Some(format!(
                "Log in with the {provider} CLI, or set an API key in Settings."
            )),
            CoreError::RuntimeMigrationLedgerInvalid { .. } => Some(
                "Restore runtime-migrations.json from a backup, or delete it to let migrations rerun."
                    .into(),
            ),
            CoreError::RuntimeMigrationLedgerTooNew { .. } => {
                Some("Update Hyprlayer to a version that can read this profile.".into())
            }
            CoreError::ThoughtsNotConfigured => {
                Some("Add the repository as a project first.".into())
            }
            CoreError::OpencodeProviderUnsupported => {
                Some("Choose a model from a supported OpenCode provider.".into())
            }
            CoreError::OpencodeModelUnavailable { .. } => Some(
                "Refresh the model list in Settings → OpenCode and pick a current model.".into(),
            ),
            CoreError::SessionNotFound { .. } => {
                Some("The session may have ended; start a new one.".into())
            }
            CoreError::NpmMissing => Some(
                "Install Node.js (it includes npm) and click Refresh, or install OpenCode with \
                 Scoop (scoop install opencode), Chocolatey (choco install opencode), or WSL. \
                 You can also use Locate… to point at an existing opencode.exe."
                    .into(),
            ),
            CoreError::OpencodeInstallFailed { .. } => Some(
                "Install Node.js/npm, or install OpenCode with Scoop \
                 (scoop install opencode), Chocolatey (choco install opencode), or WSL. \
                 then click Refresh. You can also use Locate… to point at an existing \
                 opencode.exe."
                    .into(),
            ),
            CoreError::RemoteUnsupported { host, .. } => Some(format!(
                "Do this on {host} directly, or connect to the local server."
            )),
            CoreError::BootstrapUserRequired => Some(
                "Enter the username your thought documents should be attributed to.".into(),
            ),
            CoreError::RepoAlreadyExists { offer_remove, .. } => Some(if *offer_remove {
                "Remove it first, or choose a different name or location.".into()
            } else {
                "Choose a different name or location.".into()
            }),
            CoreError::BareRepo { .. } => Some(
                "Choose a normal working copy instead. Worktrees can't be created from a bare repo."
                    .into(),
            ),
            CoreError::ProjectKindConflict { existing_kind, .. } => {
                Some(if *existing_kind == "local folder" {
                    "Remove the project first, then re-add it as a repository.".into()
                } else {
                    "Remove the project first, then re-add it as a folder.".into()
                })
            }
            CoreError::FolderIsGitRepo { .. } => Some(
                "Add it as a repository instead, so it keeps its branches and thoughts.".into(),
            ),
            CoreError::RepoNameUndeducible { .. } => {
                Some("Add an explicit name, or check the URL.".into())
            }
            CoreError::ProjectNameInvalid { .. } => {
                Some("Use a plain name, with no slashes.".into())
            }
            CoreError::GithubNameInvalid { .. } => {
                Some("Use letters, digits, dots, dashes or underscores.".into())
            }
            CoreError::GithubOriginExists { .. } => {
                Some("Remove or repoint the `origin` remote first, then publish again.".into())
            }
            CoreError::GithubOperation(failure) => failure.remedy.clone(),
            CoreError::ActiveWorkstreams { .. } => {
                Some("Archive its workstreams first, then remove the project.".into())
            }
            CoreError::WorkstreamFolderMissing { .. } => Some(
                "The folder was moved or deleted. Remove the project and re-add it.".into(),
            ),
            CoreError::WorkstreamModeNeedsBranch => {
                Some("Pick a branch, or create a new one.".into())
            }
            CoreError::WorkstreamBranchInvalid { .. } => {
                Some("Pick a branch from the list, or create a new one.".into())
            }
            CoreError::InplaceWorkstreamExists => Some(
                "Archive it first, or create the workstream in a dedicated worktree.".into(),
            ),
            CoreError::ArchiveScriptFailed => {
                Some("Re-run archive, or choose Skip to archive without the script.".into())
            }
            CoreError::ScriptEmpty { .. } => {
                Some("Add commands in Project Settings → Scripts.".into())
            }
            CoreError::WorkstreamNotArchived { actionable, .. } => {
                actionable.then(|| "Archive it first.".to_string())
            }
            CoreError::WorkstreamArchiveMissing { .. } => {
                Some("Delete the archived row to remove it from the list.".into())
            }
            CoreError::WorkstreamRestoreTargetOccupied { .. } => Some(
                "Move or archive the workstream at that path, then try again.".into(),
            ),
            CoreError::NotAGitRepo { .. } => Some(
                "Choose a directory that contains a `.git` folder, or use \"Create new\" to initialise one."
                    .into(),
            ),
            CoreError::RollbackFailed { path, reason, .. } => Some(format!(
                "{} could not be cleaned up ({reason}). Remove it manually before trying again.",
                path.display()
            )),
            CoreError::DocConflict => Some(
                "Reload the document to pick up the external changes, then re-apply yours.".into(),
            ),
            CoreError::DocNotIndexed { .. } => {
                Some("It may have been deleted or moved; refresh the library.".into())
            }
            CoreError::DocExists { .. } => Some("Pick a different title.".into()),
            CoreError::ProjectNotFound { .. } => {
                Some("Add the repository as a project first.".into())
            }
            CoreError::BackendNotFilesystem { .. } => {
                Some("Create the document in the backend's own app instead.".into())
            }
            CoreError::AgentBundleUpdateFailed { .. } => Some(
                "Check your network connection and try again. The CLI's own error is under \"Technical details\".".into(),
            ),
            CoreError::AgentBundleNotManageable => {
                Some("Update the Hyprlayer CLI, then run `hyprlayer ai reinstall`.".into())
            }
            CoreError::AgentBundleVersionUnavailable { .. } => Some(
                "Pick a version from the list. Installing an unlisted version would fall back to an unversioned bundle.".into(),
            ),
            CoreError::PromptsFileTooLarge => {
                Some("Choose a .hlprompts file exported from Hyprlayer.".into())
            }
            CoreError::PromptsFileUnreadable { .. } => {
                Some("The file may be corrupted or not a Hyprlayer prompt export.".into())
            }
            CoreError::PromptsSourceChanged => {
                Some("Choose the file again to review what it holds now.".into())
            }
            CoreError::Frontmatter(_) => {
                Some("Edit the frontmatter directly in the editor instead.".into())
            }
            _ => None,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            CoreError::ConfigMissing { .. } => "config_missing",
            CoreError::ConfigVersionTooNew { .. } => "config_version_too_new",
            CoreError::ConfigNeedsMigration { .. } => "config_needs_migration",
            CoreError::ConfigParse { .. } => "config_parse",
            CoreError::BinaryMissing { .. } => "binary_missing",
            CoreError::BinaryTooOld { .. } => "binary_too_old",
            CoreError::Exec { .. } => "exec_failed",
            CoreError::ExecTimeout { .. } => "exec_timeout",
            CoreError::OutputParse { .. } => "output_parse",
            CoreError::SessionEventDecode { .. } => "session_event_decode",
            CoreError::SessionSequenceConflict { .. } => "session_sequence_conflict",
            CoreError::SessionEpochAdvanced { .. } => "session_epoch_advanced",
            CoreError::ThoughtsNotConfigured => "config_missing",
            CoreError::OpencodeProviderUnsupported => "opencode_provider_unsupported",
            CoreError::OpencodeModelUnavailable { .. } => "opencode_model_unavailable",
            CoreError::UserTurnNotFound { .. } => "user_turn_not_found",
            CoreError::SessionNotFound { .. } => "session_not_found",
            CoreError::RuntimeInUse { .. } => "runtime_in_use",
            CoreError::RuntimeAlreadyStarted => "runtime_already_started",
            CoreError::RuntimeStopped => "runtime_stopped",
            CoreError::CredentialLookup { .. } => "credential_lookup",
            CoreError::ProviderCredentialsMissing { .. } => "provider_credentials_missing",
            CoreError::ProviderRequestFailed { .. } => "provider_request_failed",
            CoreError::ApiKeyEmpty => "api_key_empty",
            CoreError::TitleGenerationEmpty { .. } => "title_generation_empty",
            CoreError::Opencode { .. } => "opencode",
            CoreError::RuntimeMigrationLedgerInvalid { .. } => "runtime_migration_ledger_invalid",
            CoreError::RuntimeMigrationLedgerTooNew { .. } => "runtime_migration_ledger_too_new",
            CoreError::Db(_) => "db",
            CoreError::WorktreeDirty(_) => "worktree_dirty",
            CoreError::CheckoutDirty { .. } => "checkout_dirty",
            CoreError::WorktreeUnpushed { .. } => "worktree_unpushed",
            CoreError::InvalidPath { .. } => "invalid_path",
            CoreError::RepoSettingsParse { .. } => "repo_settings_parse",
            CoreError::OmarchyPaletteParse { .. } => "omarchy_palette_parse",
            CoreError::ProfileInvalid { .. } => "invalid_profile_name",
            CoreError::ProfileExists { .. } => "profile_exists",
            CoreError::BranchPrefixInvalid { .. } => "invalid_branch_prefix",
            CoreError::BootstrapUserRequired => "bootstrap_user_required",
            CoreError::RepoAlreadyExists { .. } => "repo_exists",
            CoreError::PathNotADirectory { .. } => "invalid_path",
            CoreError::BareRepo { .. } => "bare_repo",
            CoreError::ProjectKindConflict { .. } => "project_kind_conflict",
            CoreError::FolderIsGitRepo { .. } => "folder_is_git_repo",
            CoreError::RepoUrlRequired => "invalid_url",
            CoreError::RepoNameUndeducible { .. } => "invalid_url",
            CoreError::ProjectNameRequired => "invalid_name",
            CoreError::ProjectNameInvalid { .. } => "invalid_name",
            CoreError::GithubNameInvalid { .. } => "github_name_invalid",
            CoreError::GithubOriginExists { .. } => "origin_exists",
            CoreError::GithubOperation(failure) => failure.code,
            CoreError::ActiveWorkstreams { .. } => "active_workstreams",
            CoreError::WorkstreamNotFound { .. } => "workstream_not_found",
            CoreError::WorkstreamTitleRequired => "invalid_title",
            CoreError::WorkstreamFolderMissing { .. } => "folder_missing",
            CoreError::WorkstreamModeNeedsBranch => "invalid_mode",
            CoreError::WorkstreamNeedsBranch { .. } => "invalid_branch",
            CoreError::WorkstreamBranchInvalid { .. } => "invalid_branch",
            CoreError::InplaceWorkstreamExists => "inplace_exists",
            CoreError::ArchiveScriptFailed => "archive_script_failed",
            CoreError::ScriptEmpty { .. } => "script_empty",
            CoreError::WorkstreamNotArchived { .. } => "not_archived",
            CoreError::WorkstreamArchiveMissing { .. } => "archive_missing",
            CoreError::WorkstreamRestoreTargetOccupied { .. } => "restore_target_occupied",
            CoreError::WorkstreamFolderNoThoughts => "folder_no_thoughts",
            CoreError::WorkstreamWorktreeMissing { .. } => "worktree_missing",
            CoreError::NotAGitRepo { .. } => "not_a_git_repo",
            CoreError::DocConflict => "doc_conflict",
            CoreError::DocNotIndexed { .. } => "doc_not_found",
            // Not a doc-specific code: the v0.7 envelope for a document that
            // survived its write but produced no index entry is the generic
            // `internal`, pinned by the library fixture in app-errors.json.
            CoreError::DocNotIndexable => "internal",
            CoreError::DocOutsideRoots { .. } => "doc_outside_roots",
            CoreError::DocExists { .. } => "doc_exists",
            CoreError::ProjectNotFound { .. } => "project_not_found",
            CoreError::BackendNotFilesystem { .. } => "backend_not_filesystem",
            CoreError::ThoughtsRepoMissing => "backend_not_filesystem",
            CoreError::BackendNotGit { .. } => "backend_not_git",
            CoreError::AgentBundleUpdateFailed { .. } => "agent_bundle_update_failed",
            CoreError::AgentBundleNotManageable => "agent_bundle_not_manageable",
            CoreError::AgentBundleVersionUnavailable { .. } => "agent_bundle_version_unavailable",
            CoreError::PromptsFileTooLarge => "prompts_file_too_large",
            CoreError::PromptsFileUnreadable { .. } => "prompts_file_unreadable",
            CoreError::PromptsFileInvalid { .. } => "prompts_file_invalid",
            CoreError::PromptsSourceChanged => "prompts_source_changed",
            CoreError::NpmMissing => "npm_missing",
            CoreError::OpencodeInstallFailed { .. } => "opencode_install_failed",
            CoreError::CliInstallUnsupported => "cli_install_unsupported",
            CoreError::RemoteUnsupported { .. } => "remote_unsupported",
            CoreError::WorkstreamFolderNoGit => "folder_no_git",
            CoreError::PrNotFound { .. } => "pr_not_found",
            CoreError::WorkstreamBaseBranchEmpty => "internal",
            CoreError::TerminalCommandEmpty => "internal",
            CoreError::RollbackFailed { original, .. } => original.code(),
            CoreError::Frontmatter(_) => "frontmatter_patch",
            CoreError::Io(_) => "io",
        }
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(exit: Option<i32>, stderr_tail: &str) -> CoreError {
        CoreError::Exec {
            program: "git".into(),
            exit,
            stderr_tail: stderr_tail.into(),
        }
    }

    #[test]
    fn exec_with_stderr_renders_only_the_command_output() {
        let msg = exec(Some(1), "fatal: not a git repository").to_string();
        assert_eq!(msg, "fatal: not a git repository");
        assert!(!msg.contains("Some("), "leaked Option Debug: {msg}");
    }

    #[test]
    fn exec_without_stderr_names_the_program_and_code() {
        let msg = exec(Some(1), "").to_string();
        assert_eq!(msg, "`git` failed with exit code 1");
        assert!(!msg.ends_with(':'), "dangling colon: {msg}");
    }

    #[test]
    fn exec_whitespace_only_stderr_is_treated_as_empty() {
        assert_eq!(
            exec(Some(1), "  \n ").to_string(),
            "`git` failed with exit code 1"
        );
    }

    #[test]
    fn exec_without_exit_code_says_terminated() {
        let msg = exec(None, "").to_string();
        assert_eq!(msg, "`git` was terminated");
        assert!(!msg.contains("None"), "leaked Option Debug: {msg}");
    }

    #[test]
    fn exec_code_is_unchanged() {
        assert_eq!(exec(Some(1), "boom").code(), "exec_failed");
    }
}
