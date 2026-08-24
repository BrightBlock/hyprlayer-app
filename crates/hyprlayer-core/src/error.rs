use std::path::PathBuf;

/// Errors produced by hyprlayer-core. Each variant carries enough context to
/// render an actionable message in the UI; `remedy()` supplies the suggested
/// fix shown alongside it.
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

    /// The project's own checkout is dirty, blocking an in-place branch switch.
    /// Distinct from [`CoreError::WorktreeDirty`]: there is no worktree here to
    /// commit in or archive, so the remedies are different.
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

    #[error(transparent)]
    Frontmatter(#[from] crate::docs::frontmatter::FmError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The command's own stderr is the whole message whenever it wrote one: that is
/// the part the user can act on, and the `program`/exit framing around it is
/// noise they did not ask for. `attach_pr_error`
/// (`src-tauri/src/commands/workstreams.rs:1836`) already strips that framing by
/// hand for `gh`, and `codex/protocol.rs:817` applies the same
/// prefer-output-else-name-the-code shape.
///
/// A silent failure has nothing to show, so it falls back to naming the program
/// rather than rendering an empty string. `exit: None` means a signal killed the
/// child on Unix (`ExitStatus::code()` returns `None` there), so it says the
/// process was terminated instead of printing a placeholder code.
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
    /// A short, user-facing suggestion for how to fix the error, when one exists.
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
                } else if program.contains("hyprlayer")
                    && stderr_tail.contains("No AI tool configured")
                {
                    // Settings › AI tool runs `hyprlayer ai configure --force`
                    // in an embedded terminal (src/routes/Settings.tsx:660), so
                    // this is a real in-app fix, not just a restatement of the
                    // CLI's own advice.
                    Some("Open Settings › AI tool and click Configure.".into())
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
            _ => None,
        }
    }

    /// Stable machine-readable code for the frontend to branch on.
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
        // The old format left a dangling colon with nothing after it.
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

    #[test]
    fn exec_remedy_points_at_the_ai_tool_card() {
        let err = CoreError::Exec {
            program: "hyprlayer".into(),
            exit: Some(1),
            stderr_tail: "Error: No AI tool configured. Run 'hyprlayer ai configure' first.".into(),
        };
        assert!(err.remedy().is_some_and(|r| r.contains("Settings")));
    }
}
