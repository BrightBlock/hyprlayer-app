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
    /// The [`SessionState`] a terminal outcome persists as. Lives next to the
    /// enum so storage can't drift from the variants.
    pub fn state(self) -> SessionState {
        match self {
            ExitOutcome::Completed => SessionState::Completed,
            ExitOutcome::Interrupted => SessionState::Interrupted,
            ExitOutcome::Crashed => SessionState::Crashed,
            ExitOutcome::Killed => SessionState::Killed,
            ExitOutcome::SpawnFailed => SessionState::SpawnFailed,
        }
    }
}

/// A session's full lifecycle state — persisted in `sessions.state` and carried
/// by the `session-lifecycle` event. The five terminal variants mirror
/// [`ExitOutcome`]; `Idle` is a live-but-between-turns session. sqlite stores
/// `as_str` TEXT; rows read back through the tolerant [`SessionState::parse`]
/// (unknown ⇒ `Crashed`: terminal and conservative, so the UI never treats a
/// state it doesn't understand as a live session to re-attach).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Starting,
    Running,
    AwaitingPermission,
    AwaitingQuestion,
    Completed,
    Interrupted,
    Killed,
    SpawnFailed,
    Crashed,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Idle => "idle",
            SessionState::Starting => "starting",
            SessionState::Running => "running",
            SessionState::AwaitingPermission => "awaiting_permission",
            SessionState::AwaitingQuestion => "awaiting_question",
            SessionState::Completed => "completed",
            SessionState::Interrupted => "interrupted",
            SessionState::Crashed => "crashed",
            SessionState::Killed => "killed",
            SessionState::SpawnFailed => "spawn_failed",
        }
    }

    /// Tolerant row-read: unknown ⇒ `Crashed` (see the enum docs).
    pub fn parse(s: &str) -> Self {
        match s {
            "idle" => SessionState::Idle,
            "starting" => SessionState::Starting,
            "running" => SessionState::Running,
            "awaiting_permission" => SessionState::AwaitingPermission,
            "awaiting_question" => SessionState::AwaitingQuestion,
            "completed" => SessionState::Completed,
            "interrupted" => SessionState::Interrupted,
            "killed" => SessionState::Killed,
            "spawn_failed" => SessionState::SpawnFailed,
            _ => SessionState::Crashed,
        }
    }
}

impl From<ExitOutcome> for SessionState {
    fn from(outcome: ExitOutcome) -> Self {
        outcome.state()
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
    /// A user turn the app sent to claude. claude never echoes the human prompt
    /// on stdout (it's input), so the driver emits this itself purely so the turn
    /// is persisted and replays after a restart. It is NOT forwarded to the live
    /// UI (which already shows the message optimistically) — persist-only, so the
    /// live transcript isn't double-stamped. `stage` is the stage label for the
    /// session's opening prompt, else null (a follow-up).
    UserMessage {
        text: String,
        stage: Option<String>,
        /// Name of the custom prompt this turn ran, else null for a typed turn.
        /// `text` stays the exact body the agent received (vars substituted), so
        /// the UI can collapse it into a card titled with this name and expand it
        /// back to what was actually sent. Defaulted: rows persisted before this
        /// field existed still deserialize.
        #[serde(default)]
        prompt_name: Option<String>,
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
    /// The agent invoked `AskUserQuestion` — rather than a yes/no permission
    /// gate, the UI renders the questions interactively above the composer and
    /// the user picks answers; reply via the driver's `answer_question` with this
    /// `request_id`. `questions` is compact JSON of the `questions` array (each:
    /// `question`/`header`/`multiSelect`/`options[]{label,description}`).
    QuestionAsk {
        request_id: String,
        questions: String,
    },
    /// A live context-window reading (input tokens in context), emitted per
    /// assistant message so the meter updates DURING a multi-tool turn — not
    /// only at `TurnDone`. Persisted (it's tiny) so the meter survives a restart
    /// even when a turn was interrupted before its `TurnDone` ever landed.
    Usage { context_tokens: u32 },
    /// The model's context window in tokens — the meter's DENOMINATOR (`Usage` is
    /// the numerator). Emitted once at session start when the backend knows the
    /// value authoritatively: an Ollama model is launched with a fixed `num_ctx`
    /// the UI can't infer from the `provider/model` id. Absent for models we
    /// don't pin (Claude, hosted OpenCode providers) — the UI then falls back to
    /// its own per-model table. Persisted so it survives a restart/replay.
    ContextWindow { tokens: u32 },
    /// A turn finished; carries cumulative cost so far and the context-window
    /// fill (input tokens in context for the turn) for the UI's context meter.
    TurnDone {
        cost_usd: Option<f64>,
        num_turns: Option<u32>,
        context_tokens: Option<u32>,
    },
    /// claude auto- or manually compacted the conversation. The transcript
    /// renders a divider so a silent compaction stretch is legible instead of
    /// looking like a stuck turn. `trigger` is "auto" or "manual"; `pre_tokens`
    /// is the context fill just before the compact, when reported.
    CompactBoundary {
        trigger: String,
        pre_tokens: Option<u32>,
    },
    /// A command queued into a running session — a scheduled-wakeup prompt, a slash
    /// command queued behind a busy turn — started or finished. The transcript renders
    /// a seam on `started`, because claude never echoes the injected prompt to us: the
    /// turn simply begins, and without this marker it reads as the agent spontaneously
    /// resuming. `completed`/`cancelled` close the bracket and render nothing (the
    /// turn's own content already shows how it ended). Both carry the same
    /// `command_uuid`.
    CommandLifecycle {
        command_uuid: String,
        state: String,
    },
    /// A background shell task (spawned by `Bash{run_in_background:true}`) reached
    /// a terminal state. claude emits the underlying `task_notification`
    /// asynchronously the moment the shell finishes — even if the agent never
    /// polled it — so this is how the UI learns a fire-and-forget background task
    /// is done and drops it from the live "background tasks" indicator (the
    /// poll/stop paths miss the common case where the agent just moves on).
    /// `status` is "completed" or "failed"; `tool_use_id` is the spawning `Bash`
    /// call (the UI's direct transcript key), `task_id` the background shell id.
    BackgroundTaskUpdate {
        task_id: String,
        tool_use_id: Option<String>,
        status: String,
    },
    /// A new `claude` process took over an existing session via `--resume`
    /// (permission/model change, app-restart resume, log-only rewind fallback).
    /// The forwarder emits it once at the start of a relaunch — distinct from
    /// claude's per-turn `init` — so the UI drops background tasks the now-dead
    /// prior process owned (their completion notification can never arrive).
    Relaunched {},
    /// A LIVE rewind truncated this session's conversation IN PLACE: the same
    /// `claude` process had its model memory peeled by `rewind_conversation` (see
    /// [`ClaudeDriver::rewind_to_ordinal`]). Emitted once, like [`Relaunched`], so
    /// the UI settles the transcript at the new boundary — but the process is
    /// UNCHANGED, so (unlike a relaunch) its still-running background tasks can
    /// still complete-notify and must NOT be dropped.
    Rewound {},
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
            SessionEvent::UserMessage { .. } => "userMessage",
            SessionEvent::PartialText { .. } => "partialText",
            SessionEvent::AssistantText { .. } => "assistantText",
            SessionEvent::Thinking { .. } => "thinking",
            SessionEvent::ToolUse { .. } => "toolUse",
            SessionEvent::ToolResult { .. } => "toolResult",
            SessionEvent::PermissionAsk { .. } => "permissionAsk",
            SessionEvent::QuestionAsk { .. } => "questionAsk",
            SessionEvent::Usage { .. } => "usage",
            SessionEvent::ContextWindow { .. } => "contextWindow",
            SessionEvent::TurnDone { .. } => "turnDone",
            SessionEvent::CompactBoundary { .. } => "compactBoundary",
            SessionEvent::CommandLifecycle { .. } => "commandLifecycle",
            SessionEvent::BackgroundTaskUpdate { .. } => "backgroundTaskUpdate",
            SessionEvent::Relaunched { .. } => "relaunched",
            SessionEvent::Rewound { .. } => "rewound",
            SessionEvent::ProtocolError { .. } => "protocolError",
            SessionEvent::Exited { .. } => "exited",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the wire shape the UI keys on (`kind: "relaunched"`, no payload).
    #[test]
    fn relaunched_serializes_with_kind_tag() {
        assert_eq!(
            serde_json::to_string(&SessionEvent::Relaunched {}).unwrap(),
            r#"{"kind":"relaunched"}"#
        );
        assert_eq!(SessionEvent::Relaunched {}.kind(), "relaunched");
    }

    /// The live-rewind boundary is a distinct kind from `relaunched` — the UI
    /// settles the transcript at it but must keep the (same-process) background
    /// tasks, so the tag can't collide.
    #[test]
    fn rewound_serializes_with_kind_tag() {
        assert_eq!(
            serde_json::to_string(&SessionEvent::Rewound {}).unwrap(),
            r#"{"kind":"rewound"}"#
        );
        assert_eq!(SessionEvent::Rewound {}.kind(), "rewound");
    }
}
