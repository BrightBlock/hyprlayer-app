use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent::{Harness, PermissionMode, Stage};
use crate::goal::GoalBrief;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ExitOutcome {
    Completed,
    Interrupted,
    Crashed,
    Killed,
    Detached,
    SpawnFailed,
}

impl ExitOutcome {
    pub fn state(self) -> SessionState {
        match self {
            ExitOutcome::Completed => SessionState::Completed,
            ExitOutcome::Interrupted => SessionState::Interrupted,
            ExitOutcome::Crashed => SessionState::Crashed,
            ExitOutcome::Killed => SessionState::Killed,
            ExitOutcome::Detached => SessionState::Idle,
            ExitOutcome::SpawnFailed => SessionState::SpawnFailed,
        }
    }

    pub fn is_reportable(self) -> bool {
        !matches!(self, ExitOutcome::Detached)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Completed,
    Failed,
    Interrupted,
    BudgetExhausted,
    Unknown,
}

impl TurnStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "completed" => TurnStatus::Completed,
            "failed" => TurnStatus::Failed,
            "interrupted" | "aborted" | "cancelled" => TurnStatus::Interrupted,
            "budget_exhausted" => TurnStatus::BudgetExhausted,
            _ => TurnStatus::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    pub header: String,
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct QuestionAnswers(pub BTreeMap<String, String>);

impl QuestionAnswers {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, question: &str) -> Option<&str> {
        self.0.get(question).map(String::as_str)
    }
}

impl From<BTreeMap<String, String>> for QuestionAnswers {
    fn from(map: BTreeMap<String, String>) -> Self {
        Self(map)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    Init {
        #[serde(rename = "claudeSessionId")]
        provider_session_id: String,
        model: Option<String>,
        // Tool ids carry a lossy form of these; the UI names MCP tools by the real one.
        #[serde(default)]
        mcp_servers: Option<Vec<String>>,
    },
    UserMessage {
        text: String,
        stage: Option<String>,
        #[serde(default)]
        prompt_name: Option<String>,
        #[serde(default)]
        client_message_id: Option<String>,
        #[serde(default)]
        author: Option<ClientRef>,
    },
    PartialText {
        text: String,
        parent_id: Option<String>,
    },
    AssistantText {
        text: String,
        parent_id: Option<String>,
        #[serde(default)]
        injected: bool,
        // The harness rejected its credentials; this text is its error.
        #[serde(default)]
        auth_required: bool,
    },
    Thinking {
        text: String,
        parent_id: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
        parent_id: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        parent_id: Option<String>,
    },
    PermissionAsk {
        request_id: String,
        tool_name: String,
        input: String,
        // Parallel asks can share a tool name; only this ties a result to its ask.
        #[serde(default)]
        tool_use_id: Option<String>,
    },
    QuestionAsk {
        request_id: String,
        questions: Vec<Question>,
    },
    // The ask left the harness's pending set: a client answered it, or the harness dropped it.
    PermissionResolved {
        request_id: String,
        allow: bool,
    },
    QuestionAnswered {
        request_id: String,
    },
    Usage {
        context_tokens: u32,
        // The thread's cumulative uncached input plus output tokens.
        #[serde(default)]
        thread_tokens: Option<u32>,
    },
    ContextWindow {
        tokens: u32,
    },
    TurnDone {
        cost_usd: Option<f64>,
        num_turns: Option<u32>,
        context_tokens: Option<u32>,
        #[serde(default)]
        turn_status: Option<String>,
        // The thread's cumulative uncached input plus output tokens.
        #[serde(default)]
        thread_tokens: Option<u32>,
    },
    CompactBoundary {
        trigger: String,
        pre_tokens: Option<u32>,
    },
    CommandLifecycle {
        command_uuid: String,
        state: String,
    },
    BackgroundTaskUpdate {
        task_id: String,
        tool_use_id: Option<String>,
        status: String,
    },
    // The harness changed its own mode, as Claude does leaving plan mode.
    PermissionModeChanged {
        mode: PermissionMode,
    },
    // Planning layered over the permission mode: codex's plan collaboration
    // mode, OpenCode's plan agent.
    PlanModeChanged {
        active: bool,
    },
    // Quiet unless a server restart cut the harness off; clients draw no boundary then.
    Relaunched {
        #[serde(default)]
        quiet: bool,
    },
    Rewound {},
    ProtocolError {
        detail: String,
        raw: String,
    },
    Exited {
        outcome: ExitOutcome,
        code: Option<i32>,
        stderr_tail: String,
    },
}

impl SessionEvent {
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
            SessionEvent::PermissionResolved { .. } => "permissionResolved",
            SessionEvent::QuestionAnswered { .. } => "questionAnswered",
            SessionEvent::Usage { .. } => "usage",
            SessionEvent::ContextWindow { .. } => "contextWindow",
            SessionEvent::TurnDone { .. } => "turnDone",
            SessionEvent::CompactBoundary { .. } => "compactBoundary",
            SessionEvent::CommandLifecycle { .. } => "commandLifecycle",
            SessionEvent::BackgroundTaskUpdate { .. } => "backgroundTaskUpdate",
            SessionEvent::PermissionModeChanged { .. } => "permissionModeChanged",
            SessionEvent::PlanModeChanged { .. } => "planModeChanged",
            SessionEvent::Relaunched { .. } => "relaunched",
            SessionEvent::Rewound { .. } => "rewound",
            SessionEvent::ProtocolError { .. } => "protocolError",
            SessionEvent::Exited { .. } => "exited",
        }
    }

    /// Kinds a revision rides on; `thinking`/`toolUse` ride with the next commit.
    pub fn turn_kind(&self) -> Option<TurnKind> {
        match self {
            SessionEvent::UserMessage { .. } => Some(TurnKind::User),
            SessionEvent::AssistantText { .. } | SessionEvent::Thinking { .. } => {
                Some(TurnKind::Assistant)
            }
            SessionEvent::ToolUse { .. } | SessionEvent::ToolResult { .. } => Some(TurnKind::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workstream_id: String,
    pub stage: Stage,
    pub harness: Harness,
    #[serde(rename = "claudeSessionId")]
    pub provider_session_id: Option<String>,
    pub cwd: String,
    pub state: SessionState,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub exit_summary: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub archived_at: Option<f64>,
    /// Mid-turn or starting; `state` alone reads `running` for an idle live harness.
    #[serde(default)]
    pub working: bool,
    /// When the session's newest log event landed; `None` before its first.
    #[serde(default)]
    pub last_active_at: Option<f64>,
    /// Only a running or paused goal; an ended one is read through `get_goal`.
    #[serde(default)]
    pub goal: Option<GoalBrief>,
    #[serde(default)]
    pub origin: SessionOrigin,
    /// The session this one came from: its spawning agent, or the implement session a validation checks.
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// What it launched with, or was asked to start on; a retry of a failed start shows it.
    #[serde(default)]
    pub effort: Option<String>,
    /// The mode its start, last launch or last mode change asked for; `None` is Auto, the stage's own.
    #[serde(default)]
    pub requested_permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub lead: bool,
}

impl Session {
    /// The parent only when an agent spawned this row; a person's link (validate → implement) is no spawn.
    pub fn spawn_parent(&self) -> Option<&str> {
        match self.origin {
            SessionOrigin::Agent => self.parent_session_id.as_deref(),
            SessionOrigin::User | SessionOrigin::Goal => None,
        }
    }
}

/// Who started a session: the user, a goal, or another agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    #[default]
    User,
    Goal,
    Agent,
}

impl SessionOrigin {
    pub fn try_parse(s: &str) -> Option<SessionOrigin> {
        match s {
            "user" => Some(SessionOrigin::User),
            "goal" => Some(SessionOrigin::Goal),
            "agent" => Some(SessionOrigin::Agent),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> SessionOrigin {
        SessionOrigin::try_parse(s).unwrap_or(SessionOrigin::User)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SessionOrigin::User => "user",
            SessionOrigin::Goal => "goal",
            SessionOrigin::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SessionLogEntry {
    pub seq: i64,
    pub kind: String,
    pub event: SessionEvent,
    pub created_at: f64,
}

/// What a client folded before resuming. `Empty` still needs a replay: it holds no rows.
/// `Tail` and `Before` only start a `list_session_turns` page that reads back from the end.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LogCursor {
    Empty,
    After { seq: i64, created_at: f64 },
    Tail,
    Before { seq: i64, created_at: f64 },
}

/// Who wrote a turn: display-only, it grants nothing (ADR-0009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ClientRef {
    pub client_id: String,
    pub device: String,
}

impl ClientRef {
    /// The prefix the server gives an agent's turns; desktops apply them like their own.
    pub const AGENT_PREFIX: &'static str = "agent:";

    pub fn is_agent(&self) -> bool {
        self.client_id.starts_with(Self::AGENT_PREFIX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum TurnKind {
    User,
    Assistant,
    Tool,
}

/// A row that became visible with a revision; `thinking`/`toolUse` rows ride with the next commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RevisedTurn {
    pub seq: i64,
    pub kind: TurnKind,
}

/// One page of a catch-up read; its session fields match the rows only when `remaining` is 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurns {
    pub entries: Vec<SessionLogEntry>,
    /// The page's last raw row, so an undecodable tail still advances the read.
    pub next_cursor: LogCursor,
    pub remaining: u32,
    pub rev: u64,
    pub reset_rev: u64,
    pub working: bool,
    /// Other authors' sends accepted but not yet committed, as the caller sees them.
    pub in_flight: u32,
    /// A `Tail` or `Before` page's next read back; `None` once it holds the first row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_cursor: Option<LogCursor>,
    /// A `Tail` page's asks older than it that the live harness still waits on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_asks: Vec<SessionLogEntry>,
    /// Every ask a `Tail` page's live harness waits on, read after the page; `None` when not reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_request_ids: Option<Vec<String>>,
}

/// A session-stream item with its log position; `seq` is `None` for an ephemeral event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStreamItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    /// Set with `seq`: the row's cursor is `{ seq, created_at }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<f64>,
    pub rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<ClientRef>,
    pub event: SessionEvent,
}

/// A watched session's state for one caller; the server fills `viewers`, which it owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SessionRev {
    pub session_id: String,
    pub rev: u64,
    pub reset_rev: u64,
    pub working: bool,
    pub in_flight: u32,
    pub viewers: Vec<ClientRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunched_carries_quiet_and_rewound_only_its_tag() {
        assert_eq!(
            serde_json::to_string(&SessionEvent::Relaunched { quiet: false }).unwrap(),
            r#"{"kind":"relaunched","quiet":false}"#
        );
        // Rows stored before `quiet` existed were all drawn.
        assert_eq!(
            serde_json::from_str::<SessionEvent>(r#"{"kind":"relaunched"}"#).unwrap(),
            SessionEvent::Relaunched { quiet: false }
        );
        assert_eq!(
            serde_json::to_string(&SessionEvent::Rewound {}).unwrap(),
            r#"{"kind":"rewound"}"#
        );
    }

    #[test]
    fn init_keeps_the_legacy_wire_name_for_the_provider_session_id() {
        let json = serde_json::to_string(&SessionEvent::Init {
            provider_session_id: "abc".into(),
            model: None,
            mcp_servers: None,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"kind":"init","claudeSessionId":"abc","model":null,"mcpServers":null}"#
        );
        match serde_json::from_str::<SessionEvent>(&json).unwrap() {
            SessionEvent::Init {
                provider_session_id,
                ..
            } => assert_eq!(provider_session_id, "abc"),
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn session_keeps_the_legacy_wire_name_for_the_provider_session_id() {
        let session = Session {
            id: "s1".into(),
            workstream_id: "w1".into(),
            stage: Stage::General,
            harness: Harness::Claude,
            provider_session_id: Some("abc".into()),
            cwd: "/wt".into(),
            state: SessionState::Idle,
            model: None,
            cost_usd: None,
            started_at: 0.0,
            ended_at: None,
            exit_summary: None,
            title: None,
            preview: None,
            archived_at: None,
            working: false,
            last_active_at: None,
            goal: None,
            origin: SessionOrigin::User,
            parent_session_id: None,
            effort: None,
            requested_permission_mode: None,
            lead: false,
        };
        let value: serde_json::Value = serde_json::to_value(&session).unwrap();
        assert_eq!(value["claudeSessionId"], "abc");
        assert!(value.get("providerSessionId").is_none());
    }

    #[test]
    fn question_answers_serialize_as_a_bare_object() {
        let answers = QuestionAnswers(BTreeMap::from([("Tabs or spaces?".into(), "Tabs".into())]));
        assert_eq!(
            serde_json::to_string(&answers).unwrap(),
            r#"{"Tabs or spaces?":"Tabs"}"#
        );
    }

    #[test]
    fn detached_is_unreportable_and_replays_idle() {
        assert!(!ExitOutcome::Detached.is_reportable());
        assert_eq!(ExitOutcome::Detached.state(), SessionState::Idle);
        for outcome in [
            ExitOutcome::Completed,
            ExitOutcome::Interrupted,
            ExitOutcome::Crashed,
            ExitOutcome::Killed,
            ExitOutcome::SpawnFailed,
        ] {
            assert!(outcome.is_reportable(), "{outcome:?}");
        }
    }

    #[test]
    fn session_state_round_trips_through_its_stored_string() {
        for state in [
            SessionState::Idle,
            SessionState::Starting,
            SessionState::Running,
            SessionState::AwaitingPermission,
            SessionState::AwaitingQuestion,
            SessionState::Completed,
            SessionState::Interrupted,
            SessionState::Killed,
            SessionState::SpawnFailed,
            SessionState::Crashed,
        ] {
            assert_eq!(SessionState::parse(state.as_str()), state);
        }
        assert_eq!(SessionState::parse("who-knows"), SessionState::Crashed);
    }

    #[test]
    fn turn_status_parse_is_tolerant() {
        assert_eq!(TurnStatus::parse("completed"), TurnStatus::Completed);
        assert_eq!(TurnStatus::parse("failed"), TurnStatus::Failed);
        assert_eq!(TurnStatus::parse("aborted"), TurnStatus::Interrupted);
        assert_eq!(TurnStatus::parse("anything-else"), TurnStatus::Unknown);
    }
}
