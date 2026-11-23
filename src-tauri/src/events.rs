use hyprlayer_core::claude::SessionState;
use hyprlayer_core::models::goal::{GoalEndReason, GoalPauseReason, GoalStatus, GoalStep};
use hyprlayer_core::models::{
    ClientRef, DocStateChange, PtyAgentState, PtyAgentStatus, TurnKind, UiRequest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ConfigChanged;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct OmarchyThemeChanged;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct BinariesChanged;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct AccountChanged;

/// Everything ssh is waiting on the user for, after any change.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SshPromptsChanged {
    pub prompts: Vec<crate::ssh::SshPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsIndexUpdated {
    pub kind: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct DocStateChanged {
    pub ids: Vec<String>,
    pub change: DocStateChange,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusChanged {
    pub workstream_id: String,
    pub observed_pr_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamsChanged {
    pub project_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct LeadQueueChanged {
    pub project_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeChanged {
    pub workstream_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ReviewChanged {
    pub workstream_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct GoalChanged {
    pub goal_id: String,
    pub session_id: String,
    pub workstream_id: String,
    pub status: GoalStatus,
    pub step: Option<GoalStep>,
    pub pause_reason: Option<GoalPauseReason>,
    pub end_reason: Option<GoalEndReason>,
    pub reason: Option<String>,
    pub iteration: u32,
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct PtyAgentChanged {
    pub terminal_id: String,
    pub workstream_id: String,
    pub engine_id: String,
    pub state: PtyAgentState,
    pub provider_session_id: Option<String>,
    pub activity: Option<String>,
    pub last_reply: Option<String>,
    pub title: Option<String>,
    pub status: PtyAgentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct UiRequested {
    pub request: UiRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamBranchChanged {
    pub workstream_id: String,
    pub branch: String,
    pub previous: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionLifecycle {
    pub session_id: String,
    pub workstream_id: String,
    pub state: SessionState,
}

/// Revs and seqs are `f64` here: specta exports no `u64`/`i64`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RevisedTurnDto {
    pub seq: f64,
    pub kind: TurnKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionRevised {
    pub session_id: String,
    pub workstream_id: String,
    pub rev: f64,
    pub turns: Vec<RevisedTurnDto>,
    pub author: Option<ClientRef>,
    pub working: bool,
    pub at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionReset {
    pub session_id: String,
    pub workstream_id: String,
    pub rev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionViewers {
    pub session_id: String,
    pub viewers: Vec<ClientRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRunChanged {
    pub workstream_id: String,
    pub kind: hyprlayer_core::scripts::ScriptKind,
    pub phase: hyprlayer_core::models::script::RunPhase,
    pub exit_code: Option<i32>,
    pub port: Option<u16>,
}

pub use hyprlayer_core::models::project::{CreateStep, StepStatus};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCreateProgress {
    pub op_id: String,
    pub step: CreateStep,
    pub status: StepStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct QuitRequested;

/// The RPC connection to the server ended. Nothing respawns the server
/// (ADR-0009), so the frontend offers a relaunch.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ServerDisconnected;

/// Wakes the window's reconnect loop: a hidden webview's timers and late replies stall, an event still runs.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ReconnectDue;

/// The connection lost runtime events; re-read what they would have changed.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct EventsMissed;
