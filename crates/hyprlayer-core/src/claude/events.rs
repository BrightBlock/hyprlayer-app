use serde::{Deserialize, Serialize};

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

const EXTERNALLY_TERMINATED: i32 = 0x4001_0004;

#[cfg(unix)]
fn is_stop_signal(signal: i32) -> bool {
    matches!(
        signal,
        libc::SIGHUP | libc::SIGINT | libc::SIGTERM | libc::SIGKILL
    )
}

#[cfg(not(unix))]
fn is_stop_signal(_signal: i32) -> bool {
    false
}

fn is_stop_exit_code(code: i32) -> bool {
    matches!(code, 129 | 130 | 137 | 143)
}

impl ExitOutcome {
    pub fn from_unclean_exit(code: Option<i32>, signal: Option<i32>) -> Self {
        if let Some(signal) = signal {
            return if is_stop_signal(signal) {
                ExitOutcome::Killed
            } else {
                ExitOutcome::Crashed
            };
        }
        match code {
            Some(EXTERNALLY_TERMINATED) => ExitOutcome::Killed,
            Some(code) if is_stop_exit_code(code) => ExitOutcome::Killed,
            _ => ExitOutcome::Crashed,
        }
    }

    pub fn from_status(status: &std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(status);
        #[cfg(not(unix))]
        let signal = None;
        Self::from_unclean_exit(status.code(), signal)
    }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    Init {
        claude_session_id: String,
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
        questions: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunched_serializes_with_kind_tag() {
        assert_eq!(
            serde_json::to_string(&SessionEvent::Relaunched {}).unwrap(),
            r#"{"kind":"relaunched"}"#
        );
        assert_eq!(SessionEvent::Relaunched {}.kind(), "relaunched");
    }

    #[test]
    fn rewound_serializes_with_kind_tag() {
        assert_eq!(
            serde_json::to_string(&SessionEvent::Rewound {}).unwrap(),
            r#"{"kind":"rewound"}"#
        );
        assert_eq!(SessionEvent::Rewound {}.kind(), "rewound");
    }

    #[test]
    fn windows_external_termination_is_a_kill_not_a_crash() {
        assert_eq!(
            ExitOutcome::from_unclean_exit(Some(1_073_807_364), None),
            ExitOutcome::Killed
        );
        assert_eq!(
            ExitOutcome::from_unclean_exit(Some(1), None),
            ExitOutcome::Crashed
        );
        assert_eq!(
            ExitOutcome::from_unclean_exit(None, None),
            ExitOutcome::Crashed
        );
    }

    #[test]
    fn unix_stop_signals_are_kills_but_faults_are_crashes() {
        for signal in [1, 2, 15, 9] {
            assert_eq!(
                ExitOutcome::from_unclean_exit(None, Some(signal)),
                ExitOutcome::Killed,
                "signal {signal}"
            );
        }
        for signal in [6, 11] {
            assert_eq!(
                ExitOutcome::from_unclean_exit(None, Some(signal)),
                ExitOutcome::Crashed,
                "signal {signal}"
            );
        }
    }

    #[test]
    fn stop_exit_codes_are_kills_not_crashes() {
        for code in [129, 130, 137, 143] {
            assert_eq!(
                ExitOutcome::from_unclean_exit(Some(code), None),
                ExitOutcome::Killed,
                "exit {code}"
            );
        }
        for code in [1, 2, 127, 128] {
            assert_eq!(
                ExitOutcome::from_unclean_exit(Some(code), None),
                ExitOutcome::Crashed,
                "exit {code}"
            );
        }
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
    fn turn_status_parse_is_tolerant() {
        assert_eq!(TurnStatus::parse("completed"), TurnStatus::Completed);
        assert_eq!(TurnStatus::parse("failed"), TurnStatus::Failed);
        assert_eq!(TurnStatus::parse("aborted"), TurnStatus::Interrupted);
        assert_eq!(TurnStatus::parse("anything-else"), TurnStatus::Unknown);
    }
}
