//! The app-facing session event stream — what the UI renders and the store
//! persists. The Tauri shell forwards these over a per-session Channel and
//! writes them to `session_events` for replay.

use serde::{Deserialize, Serialize};

/// How a claude process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ExitOutcome {
    /// Emitted a terminal `result` then exited 0.
    Completed,
    /// Stopped after an app-initiated interrupt.
    Interrupted,
    /// Exited without a terminal `result` (or non-zero) — treated as a crash.
    Crashed,
    /// We killed the process tree (hard stop / cleanup).
    Killed,
    /// Failed to spawn / disappeared immediately.
    SpawnFailed,
}

impl ExitOutcome {
    /// The persisted `sessions.state` string for a terminal outcome. Lives next
    /// to the enum so storage can't drift from the variants.
    pub fn state(self) -> &'static str {
        match self {
            ExitOutcome::Completed => "completed",
            ExitOutcome::Interrupted => "interrupted",
            ExitOutcome::Crashed => "crashed",
            ExitOutcome::Killed => "killed",
            ExitOutcome::SpawnFailed => "spawn_failed",
        }
    }
}

/// One event in a session's lifetime. Ordered; the sequence per session is
/// monotonic. Serialized tagged so the frontend can switch on `kind`. Tool
/// inputs are carried as compact JSON strings (not `serde_json::Value`) so the
/// type exports cleanly through specta; the UI parses them for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    /// Session is up; claude assigned its own id (used for `--resume`).
    Init {
        claude_session_id: String,
        model: Option<String>,
    },
    /// Streaming text delta (live typing). Coalesced into `AssistantText` on
    /// turn completion — the UI shows partials, then replaces with the final.
    PartialText {
        text: String,
        parent_id: Option<String>,
    },
    /// A completed assistant text block.
    AssistantText {
        text: String,
        parent_id: Option<String>,
    },
    /// A completed thinking block (shown collapsed).
    Thinking {
        text: String,
        parent_id: Option<String>,
    },
    /// The agent invoked a tool. `input` is compact JSON. `parent_id` is the
    /// `Task` tool-use id when this runs inside a subagent (else null).
    ToolUse {
        id: String,
        name: String,
        input: String,
        parent_id: Option<String>,
    },
    /// Result of a tool the agent ran.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        parent_id: Option<String>,
    },
    /// Permission prompt — the UI renders an Allow/Deny card; reply via the
    /// driver's `respond_permission` with this `request_id`. `input` is compact
    /// JSON of the tool's arguments.
    PermissionAsk {
        request_id: String,
        tool_name: String,
        input: String,
    },
    /// A live context-window reading (input tokens in context), emitted per
    /// assistant message so the meter updates DURING a multi-tool turn — not
    /// only at `TurnDone`. Persisted (it's tiny) so the meter survives a restart
    /// even when a turn was interrupted before its `TurnDone` ever landed.
    Usage { context_tokens: u32 },
    /// A turn finished; carries cumulative cost so far and the context-window
    /// fill (input tokens in context for the turn) for the UI's context meter.
    TurnDone {
        cost_usd: Option<f64>,
        num_turns: Option<u32>,
        context_tokens: Option<u32>,
    },
    /// An unparseable / unknown stdout line — surfaced, never fatal.
    ProtocolError { detail: String, raw: String },
    /// The process ended. Terminal: no further events for this session.
    Exited {
        outcome: ExitOutcome,
        code: Option<i32>,
        stderr_tail: String,
    },
}

impl SessionEvent {
    /// The stored `kind` tag — identical to this event's serde tag. Lives next to
    /// the enum so the persisted log can't drift from the wire name.
    pub fn kind(&self) -> &'static str {
        match self {
            SessionEvent::Init { .. } => "init",
            SessionEvent::PartialText { .. } => "partialText",
            SessionEvent::AssistantText { .. } => "assistantText",
            SessionEvent::Thinking { .. } => "thinking",
            SessionEvent::ToolUse { .. } => "toolUse",
            SessionEvent::ToolResult { .. } => "toolResult",
            SessionEvent::PermissionAsk { .. } => "permissionAsk",
            SessionEvent::Usage { .. } => "usage",
            SessionEvent::TurnDone { .. } => "turnDone",
            SessionEvent::ProtocolError { .. } => "protocolError",
            SessionEvent::Exited { .. } => "exited",
        }
    }
}
