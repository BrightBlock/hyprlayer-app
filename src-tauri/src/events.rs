use hyprlayer_core::claude::SessionState;
use hyprlayer_core::models::{ClientRef, DocStateChange, TurnKind};
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
