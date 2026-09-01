use hyprlayer_core::claude::SessionState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ConfigChanged;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct OmarchyThemeChanged;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct BinariesChanged;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsIndexUpdated {
    pub kind: String,
    pub ids: Vec<String>,
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
