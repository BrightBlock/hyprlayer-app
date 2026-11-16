use serde::{Deserialize, Serialize};

use crate::agent_control::{PtyAgentState, PtyAgentStatus, UiRequest};
use crate::goal::{GoalEndReason, GoalPauseReason, GoalStatus, GoalStep};
use crate::project::{CreateStep, StepStatus};
use crate::script::{RunPhase, ScriptKind};
use crate::session::{ClientRef, RevisedTurn, SessionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ThoughtsIndexUpdateKind {
    Updated,
    Removed,
    Rescanned,
}

/// Why a doc's Library state changed: a read toggle, or an agent wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum DocStateChange {
    Read,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuntimeEvent {
    ConfigChanged,
    BinariesChanged,
    ThoughtsIndexUpdated {
        update: ThoughtsIndexUpdateKind,
        ids: Vec<String>,
    },
    /// Never folded into `ThoughtsIndexUpdated`, whose ids clients credit to active workstreams.
    DocStateChanged {
        ids: Vec<String>,
        change: DocStateChange,
    },
    ProjectsChanged,
    PrStatusChanged {
        workstream_id: String,
        /// The row's `observed_pr` changed, so workstream lists are stale too.
        #[serde(default)]
        observed_pr_changed: bool,
    },
    WorktreeChanged {
        workstream_id: String,
    },
    ReviewChanged {
        workstream_id: String,
    },
    WorkstreamBranchChanged {
        workstream_id: String,
        branch: String,
        previous: String,
    },
    SessionLifecycle {
        session_id: String,
        workstream_id: String,
        state: SessionState,
    },
    SessionRevised {
        session_id: String,
        workstream_id: String,
        rev: u64,
        turns: Vec<RevisedTurn>,
        author: Option<ClientRef>,
        working: bool,
        at: f64,
    },
    SessionReset {
        session_id: String,
        workstream_id: String,
        rev: u64,
    },
    SessionViewers {
        session_id: String,
        viewers: Vec<ClientRef>,
    },
    ScriptRunChanged {
        workstream_id: String,
        kind: ScriptKind,
        phase: RunPhase,
        exit_code: Option<i32>,
        port: Option<u16>,
    },
    ProjectCreateProgress {
        op_id: String,
        step: CreateStep,
        status: StepStatus,
        detail: Option<String>,
    },
    /// `reason` is the goal's last reason, or the tool or question of the ask while it waits.
    GoalChanged {
        goal_id: String,
        session_id: String,
        workstream_id: String,
        status: GoalStatus,
        step: Option<GoalStep>,
        pause_reason: Option<GoalPauseReason>,
        end_reason: Option<GoalEndReason>,
        reason: Option<String>,
        iteration: u32,
        max_iterations: u32,
    },
    /// Re-read `account_status`; carries no identity or token.
    AccountChanged,
    PtyAgentChanged {
        terminal_id: String,
        workstream_id: String,
        engine_id: String,
        state: PtyAgentState,
        provider_session_id: Option<String>,
        activity: Option<String>,
        last_reply: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        status: PtyAgentStatus,
    },
    UiRequested {
        request: UiRequest,
    },
}
