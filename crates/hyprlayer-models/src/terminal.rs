use serde::{Deserialize, Serialize};

use crate::agent::Stage;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TerminalEvent {
    Output {
        /// Base64.
        data: String,
        /// Where `data` starts in the terminal's output; a reattach from its end replays only what follows.
        // A JS number is exact far past any terminal's output.
        #[serde(default)]
        #[cfg_attr(feature = "specta", specta(type = Option<f64>))]
        offset: Option<u64>,
    },
    /// The terminal's size from here on: a viewer renders at it, not at its own fit.
    Resized {
        cols: u16,
        rows: u16,
    },
    Exited {
        code: Option<i32>,
    },
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
    /// An app action's first turn; the server composes it and types it in once the TUI is ready.
    #[serde(default)]
    pub first_prompt: Option<PtyFirstPrompt>,
}

/// The stage prompt a native session would open with, for a PTY agent to run instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PtyFirstPrompt {
    pub stage: Stage,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub attached_docs: Vec<String>,
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
