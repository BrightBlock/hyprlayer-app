use std::path::PathBuf;

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

    #[error(transparent)]
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
