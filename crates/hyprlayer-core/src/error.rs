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

    #[error("`{program}` exited with {exit:?}: {stderr_tail}")]
    Exec {
        program: String,
        exit: Option<i32>,
        stderr_tail: String,
    },

    #[error("`{program}` timed out after {seconds}s")]
    ExecTimeout { program: String, seconds: u64 },

    #[error("failed to parse output of `{program}`: {detail}")]
    OutputParse { program: String, detail: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
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
                "Run any hyprlayer command in a terminal (e.g. `hyprlayer storage info`) to upgrade the config file — the app picks it up automatically.".into(),
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
            CoreError::Io(_) => "io",
        }
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;
