use serde::{Deserialize, Serialize};

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
    ProjectsChanged,
    PrStatusChanged {
        workstream_id: String,
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
}
