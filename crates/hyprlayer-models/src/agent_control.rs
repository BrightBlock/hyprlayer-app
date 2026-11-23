use serde::{Deserialize, Serialize};

use crate::agent::{Harness, PermissionMode, Stage};
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum AgentControlLevel {
    Off,
    Read,
    Full,
}

impl AgentControlLevel {
    pub fn try_parse(s: &str) -> Option<AgentControlLevel> {
        match s {
            "off" => Some(AgentControlLevel::Off),
            "read" => Some(AgentControlLevel::Read),
            "full" => Some(AgentControlLevel::Full),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> AgentControlLevel {
        AgentControlLevel::try_parse(s).unwrap_or(AgentControlLevel::Read)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AgentControlLevel::Off => "off",
            AgentControlLevel::Read => "read",
            AgentControlLevel::Full => "full",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LeadView {
    pub project_path: String,
    pub workstream_id: Option<String>,
    pub session_id: Option<String>,
    pub driving: Vec<String>,
    pub live_children: u32,
    pub project_rules: Option<String>,
    pub remote_control: bool,
    pub remote_control_url: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// The agent a control call speaks for: a composer session or a PTY terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentCaller {
    Session { session_id: String },
    Terminal { terminal_id: String },
}

impl AgentCaller {
    pub fn id(&self) -> &str {
        match self {
            AgentCaller::Session { session_id } => session_id,
            AgentCaller::Terminal { terminal_id } => terminal_id,
        }
    }
}

/// A hooked PTY agent's state, folded from its hook signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PtyAgentState {
    Starting,
    Working,
    AwaitingPermission,
    AwaitingQuestion,
    Idle,
    Exited,
}

impl PtyAgentState {
    pub fn try_parse(s: &str) -> Option<PtyAgentState> {
        match s {
            "starting" => Some(PtyAgentState::Starting),
            "working" => Some(PtyAgentState::Working),
            "awaiting_permission" => Some(PtyAgentState::AwaitingPermission),
            "awaiting_question" => Some(PtyAgentState::AwaitingQuestion),
            "idle" => Some(PtyAgentState::Idle),
            "exited" => Some(PtyAgentState::Exited),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> PtyAgentState {
        PtyAgentState::try_parse(s).unwrap_or(PtyAgentState::Idle)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PtyAgentState::Starting => "starting",
            PtyAgentState::Working => "working",
            PtyAgentState::AwaitingPermission => "awaiting_permission",
            PtyAgentState::AwaitingQuestion => "awaiting_question",
            PtyAgentState::Idle => "idle",
            PtyAgentState::Exited => "exited",
        }
    }
}

/// A server-launched PTY agent whose engine reports through hooks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PtyAgent {
    pub terminal_id: String,
    pub workstream_id: String,
    pub engine_id: String,
    pub state: PtyAgentState,
    pub provider_session_id: Option<String>,
    pub activity: Option<String>,
    pub last_reply: Option<String>,
    /// The agent that launched it; desktops adopt these in a background tab.
    #[serde(default)]
    pub spawned_by: Option<AgentCaller>,
    /// Unhooked entries are listed only when an agent spawned them; their state is never folded.
    #[serde(default = "listed_hooked")]
    pub hooked: bool,
    /// Generated from its first prompt; set once.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: PtyAgentStatus,
}

/// What a hooked PTY agent's own CLI last reported about its settings; never an app-side estimate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PtyAgentStatus {
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The CLI's own mode key (`default`, `acceptEdits`, `plan`, `auto`, `bypassPermissions`…), which `PermissionMode` doesn't cover.
    pub permission_mode: Option<String>,
    /// codex's Plan collaboration mode, which sits beside its permissions.
    pub planning: bool,
    pub context: Option<PtyAgentContext>,
    /// Whether the bar may change these on the running agent.
    pub controllable: bool,
}

/// A model change from the bar: applied in the running TUI, or unconfirmed while idle, so the desktop relaunches with `--resume` and `--model`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PtyControlOutcome {
    Applied,
    Relaunch,
}

/// Context use as the CLI reported it; `None` on the agent until it reports any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PtyAgentContext {
    pub tokens: u32,
    pub window: u32,
    /// claude's own rounded `used_percentage`; codex reports none.
    pub percent: Option<u32>,
}

/// One claude statusLine run: the whole of model, effort and context, as the face read them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentStatusReport {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context: Option<PtyAgentContext>,
}

/// A server that predates `hooked` listed hooked agents only.
fn listed_hooked() -> bool {
    true
}

/// One hook call's payload, already trimmed by the `hook` face.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSignal {
    pub event: String,
    pub session_id: Option<String>,
    pub source: Option<String>,
    /// `SessionEnd`'s reason, or the error that ended a `StopFailure` turn.
    pub reason: Option<String>,
    pub tool_name: Option<String>,
    pub tool_target: Option<String>,
    pub notification_type: Option<String>,
    pub last_assistant_message: Option<String>,
    pub turn_id: Option<String>,
    /// `UserPromptSubmit`'s prompt, cut to 2 KiB: the tab title's source.
    pub prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub effort: Option<String>,
    pub model: Option<String>,
    /// `PostModelSwitch`'s new model.
    pub to_model: Option<String>,
    /// codex's rollout file, which holds the effort and token counts no hook carries.
    pub transcript_path: Option<String>,
    /// A write tool's targets, absolute, uncut: `tool_target` is cut for display.
    pub written_paths: Vec<String>,
}

/// A claude worktree hook, which the server answers rather than folds into its agent's state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all_fields = "camelCase")]
pub enum ClaudeWorktreeEvent {
    Create {
        name: String,
        cwd: String,
        session_id: Option<String>,
    },
    Remove {
        worktree_path: String,
    },
    SubagentStop {
        agent_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClaudeWorktreeAnswer {
    /// Hyprlayer's workstream holds the worktree at this path.
    Path(String),
    /// Not hyprlayer's: the hook does what claude would have done with no hook installed.
    Fallback,
}

/// What `announce_agent` tells an agent about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentWhoami {
    pub caller: AgentCaller,
    pub workstream_id: String,
    pub project_path: String,
    pub level: AgentControlLevel,
    pub permission_mode: Option<PermissionMode>,
    pub depth: u32,
    pub parent: Option<AgentCaller>,
    pub report_only: bool,
}

/// An agent's ask of every attached desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UiRequest {
    OpenSessionTab {
        workstream_id: String,
        session_id: String,
        focus: bool,
        // The mode the child launched in, so the tab never relaunches it above that.
        #[serde(default)]
        permission_mode: Option<PermissionMode>,
        // The model and effort the child launched with, so its tab's bar shows them.
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
    },
    OpenTerminalTab {
        workstream_id: String,
        terminal_id: String,
        engine_id: String,
        provider_session_id: Option<String>,
        bypass: bool,
    },
    Notify {
        workstream_id: String,
        title: String,
        body: String,
        // The session it is about; a desktop already streaming that session notifies on its own.
        #[serde(default)]
        session_id: Option<String>,
        // The PTY agent it is about; a desktop whose tab binds that terminal notifies on its own.
        #[serde(default)]
        terminal_id: Option<String>,
        // Raised by a project's Lead; only these reach the Lead dock.
        #[serde(default)]
        from_lead: bool,
    },
    Reveal {
        workstream_id: String,
        target: RevealTarget,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RevealTarget {
    Doc { path: String },
    File { path: String },
    Diff { path: String },
}

/// What a wait on another agent stops at; each also stops at the ones after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum WaitUntil {
    Idle,
    NeedsYou,
    Exited,
}

impl WaitUntil {
    pub fn try_parse(s: &str) -> Option<WaitUntil> {
        match s {
            "idle" => Some(WaitUntil::Idle),
            "needs_you" => Some(WaitUntil::NeedsYou),
            "exited" => Some(WaitUntil::Exited),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> WaitUntil {
        WaitUntil::try_parse(s).unwrap_or(WaitUntil::Idle)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WaitUntil::Idle => "idle",
            WaitUntil::NeedsYou => "needs_you",
            WaitUntil::Exited => "exited",
        }
    }
}

/// Where a wait left its target; `state` is `idle`, `needs_you`, `exited` or `working`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WaitOutcome {
    pub state: String,
    pub last_reply: Option<String>,
    pub stalled: bool,
    pub timed_out: bool,
}

/// An agent's ask for a new composer session; unset fields follow the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SpawnSessionRequest {
    pub workstream_id: String,
    #[serde(default)]
    pub stage: Option<Stage>,
    pub prompt: String,
    #[serde(default)]
    pub harness: Option<Harness>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub attached_docs: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// What an agent's spawn started: a composer session, or a PTY agent where app actions open.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpawnedAgent {
    Session { session: Session },
    Pty { agent: PtyAgent },
}

/// An agent's ask for a PTY agent; `bypass` only from a bypass caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SpawnPtyAgentRequest {
    pub workstream_id: String,
    pub engine_id: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub bypass: bool,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WakeCondition {
    After {
        secs: u32,
    },
    At {
        unix_secs: u64,
    },
    /// The branch's tip on the remote moves from where it was when the wait began.
    Ref {
        branch: String,
        #[serde(default)]
        remote: Option<String>,
    },
    Pr {
        workstream_id: String,
        until: PrWakeUntil,
    },
    Session {
        session_id: String,
        until: WaitUntil,
    },
    Terminal {
        terminal_id: String,
        until: WaitUntil,
    },
    /// Any other agent in the caller's project reaching `until` from a state before it.
    AnyAgent {
        until: WaitUntil,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum PrWakeUntil {
    Opened,
    Merged,
    /// Merged counts too.
    Closed,
    ChecksDone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Wake {
    pub id: String,
    pub condition: WakeCondition,
    pub deadline_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageDelivery {
    Sent,
    /// Mid-turn, a slash command or what follows one waits for the agent's next idle.
    Queued,
    /// The running turn was stopped, and the message is the agent's next turn.
    Interrupted,
}
