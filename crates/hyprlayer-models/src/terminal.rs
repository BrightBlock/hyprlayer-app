use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TerminalEvent {
    Output { data: String },
    Exited { code: Option<i32> },
}

/// Which provider session a PTY agent launch starts or resumes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PtyLaunchSession {
    pub session_id: Option<String>,
    pub resume: bool,
    pub bypass: bool,
    /// A session-only model for engines that take one at launch.
    #[serde(default)]
    pub model: Option<String>,
    /// The CLI's own mode key to start in, so a relaunch keeps the session's mode.
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// A session-only effort for engines that take one at launch.
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PtyLaunched {
    pub terminal_id: String,
    /// Assigned or minted by the server; a hooked codex learns its id from `SessionStart`.
    pub provider_session_id: Option<String>,
    pub hooked: bool,
}
