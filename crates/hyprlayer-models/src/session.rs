use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent::{Harness, Stage};

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
    Unknown,
}

impl TurnStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "completed" => TurnStatus::Completed,
            "failed" => TurnStatus::Failed,
            "interrupted" | "aborted" | "cancelled" => TurnStatus::Interrupted,
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
    },
    UserMessage {
        text: String,
        stage: Option<String>,
        #[serde(default)]
        prompt_name: Option<String>,
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
    },
    QuestionAsk {
        request_id: String,
        questions: Vec<Question>,
    },
    Usage {
        context_tokens: u32,
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
    Relaunched {},
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunched_and_rewound_serialize_with_only_their_kind_tag() {
        assert_eq!(
            serde_json::to_string(&SessionEvent::Relaunched {}).unwrap(),
            r#"{"kind":"relaunched"}"#
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
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"kind":"init","claudeSessionId":"abc","model":null}"#
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
