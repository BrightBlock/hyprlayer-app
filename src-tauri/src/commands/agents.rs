use serde::Serialize;
use tauri::State;

use hyprlayer_core::agent_bundle::{self, BundleSource};
use hyprlayer_core::agents::{AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay};
use hyprlayer_core::hyprcli::AiVersions;
use hyprlayer_core::runtime::agents::{BundleInstall, BundleReport, BundleStatus};
use hyprlayer_core::skills::AgentSkill;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_agent_files(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<AgentFile>> {
    state
        .runtime
        .agents()
        .list(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn save_agent_file(state: State<'_, AppState>, agent: AgentFile) -> CmdResult<AgentFile> {
    state
        .runtime
        .agents()
        .save(agent)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_agent_file(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    state
        .runtime
        .agents()
        .delete(path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn new_agent_file_path(
    state: State<'_, AppState>,
    name: String,
    source: String,
    project_path: Option<String>,
) -> CmdResult<Option<String>> {
    state
        .runtime
        .agents()
        .new_file_path(name, source, project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_context_files(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<AgentContextFile>> {
    state
        .runtime
        .agents()
        .context_files(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_mcp_servers(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<AgentMcpServer>> {
    state
        .runtime
        .agents()
        .mcp_servers(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_skills(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<AgentSkill>> {
    state
        .runtime
        .agents()
        .skills(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn export_agent_file(
    state: State<'_, AppState>,
    agent: AgentFile,
    overlay: AgentOverlay,
) -> CmdResult<String> {
    state
        .runtime
        .agents()
        .export(agent, overlay)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleStatus {
    pub manageable: bool,
    pub installed_sha: Option<String>,
    pub latest_sha: Option<String>,
    pub installed_version: Option<String>,
    pub desired_version: Option<String>,
    pub last_checked: Option<f64>,
    pub update_available: bool,
}

impl From<BundleStatus> for AgentBundleStatus {
    fn from(status: BundleStatus) -> Self {
        Self {
            manageable: status.manageable,
            installed_sha: status.installed_sha,
            latest_sha: status.latest_sha,
            installed_version: status.installed_version,
            desired_version: status.desired_version,
            last_checked: status.last_checked,
            update_available: status.update_available,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn agent_bundle_status(state: State<'_, AppState>) -> CmdResult<AgentBundleStatus> {
    state
        .runtime
        .agents()
        .bundle_status()
        .await
        .map(AgentBundleStatus::from)
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleReport {
    pub source: BundleSource,
    pub stdout: String,
}

impl From<BundleReport> for AgentBundleReport {
    fn from(report: BundleReport) -> Self {
        Self {
            source: report.source,
            stdout: report.stdout,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn update_agent_bundle(state: State<'_, AppState>) -> CmdResult<AgentBundleReport> {
    state
        .runtime
        .agents()
        .update_bundle()
        .await
        .map(AgentBundleReport::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_bundle_versions(state: State<'_, AppState>) -> CmdResult<AiVersions> {
    state
        .runtime
        .agents()
        .bundle_versions()
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleInstall {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub source: BundleSource,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub resolved: Option<agent_bundle::CollisionReport>,
}

impl From<BundleInstall> for AgentBundleInstall {
    fn from(install: BundleInstall) -> Self {
        Self {
            ok: install.ok,
            exit_code: install.exit_code,
            source: install.source,
            stdout: install.stdout,
            stderr: install.stderr,
            resolved: install.resolved,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn repair_agent_bundle(state: State<'_, AppState>) -> CmdResult<AgentBundleInstall> {
    state
        .runtime
        .agents()
        .repair_bundle()
        .await
        .map(AgentBundleInstall::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn install_agent_bundle_version(
    state: State<'_, AppState>,
    version: Option<String>,
) -> CmdResult<AgentBundleInstall> {
    state
        .runtime
        .agents()
        .install_bundle_version(version)
        .await
        .map(AgentBundleInstall::from)
        .map_err(AppError::from)
}
