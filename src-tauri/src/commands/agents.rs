use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use hyprlayer_core::agent_bundle::{self, BundleSource};
use hyprlayer_core::agents::{self, AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay};
use hyprlayer_core::hyprcli::{AiVersions, BundleSelection, HyprCli};
use hyprlayer_core::skills::AgentSkill;

use crate::commands::blocking;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_agent_files(project_path: Option<String>) -> CmdResult<Vec<AgentFile>> {
    blocking("list_agent_files", move || {
        Ok(agents::list_agents(project_path.as_deref().map(Path::new)))
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn save_agent_file(agent: AgentFile) -> CmdResult<AgentFile> {
    blocking("save_agent_file", move || {
        agents::save_agent(&agent).map_err(AppError::from)
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn delete_agent_file(path: String) -> CmdResult<()> {
    blocking("delete_agent_file", move || {
        agents::delete_agent(Path::new(&path)).map_err(AppError::from)
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn new_agent_file_path(
    name: String,
    source: String,
    project_path: Option<String>,
) -> CmdResult<Option<String>> {
    blocking("new_agent_file_path", move || {
        let dir: Option<PathBuf> =
            agents::agents_dir_for(project_path.as_deref().map(Path::new), &source);
        Ok(dir.map(|d| {
            d.join(format!("{}.md", agents::file_stem_for(&name)))
                .to_string_lossy()
                .into_owned()
        }))
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn agent_context_files(project_path: Option<String>) -> CmdResult<Vec<AgentContextFile>> {
    blocking("agent_context_files", move || {
        Ok(agents::context_candidates(
            project_path.as_deref().map(Path::new),
        ))
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn agent_mcp_servers(project_path: Option<String>) -> CmdResult<Vec<AgentMcpServer>> {
    blocking("agent_mcp_servers", move || {
        Ok(agents::mcp_servers(project_path.as_deref().map(Path::new)))
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn agent_skills(project_path: Option<String>) -> CmdResult<Vec<AgentSkill>> {
    blocking("agent_skills", move || {
        Ok(hyprlayer_core::skills::list_skills(
            project_path.as_deref().map(Path::new),
        ))
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn export_agent_file(agent: AgentFile, overlay: AgentOverlay) -> CmdResult<String> {
    blocking("export_agent_file", move || {
        agents::export_agent(&agent, &overlay).map_err(AppError::from)
    })
    .await?
}

const BUNDLE_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REFS_BYTES: usize = 1024 * 1024;

async fn fetch_latest_bundle_sha() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(BUNDLE_PROBE_TIMEOUT)
        .user_agent("hyprlayer-app")
        .build()
        .ok()?;
    let mut resp = client
        .get(agent_bundle::BUNDLE_REFS_URL)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_REFS_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_REFS_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    agent_bundle::parse_master_sha(&String::from_utf8_lossy(&body))
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

#[tauri::command]
#[specta::specta]
pub async fn agent_bundle_status(state: State<'_, AppState>) -> CmdResult<AgentBundleStatus> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let status = HyprCli::new(&state.binaries)
        .ai_status(&cwd)
        .await
        .map_err(AppError::from)?;
    let manageable = agent_bundle::is_manageable_pair(&status);
    let desired = agent_bundle::desired_version(
        status.pinned_version.as_deref(),
        status.binary_version.as_deref(),
    )
    .map(str::to_string);

    if !manageable {
        return Ok(AgentBundleStatus {
            manageable,
            installed_sha: status.agents_installed_sha,
            latest_sha: None,
            installed_version: status.assets_version,
            desired_version: desired,
            last_checked: status.last_agent_check,
            update_available: false,
        });
    }

    if let Some(desired) = desired {
        let update_available = agent_bundle::is_stale(status.assets_version.as_deref(), &desired);
        return Ok(AgentBundleStatus {
            manageable,
            installed_sha: status.agents_installed_sha,
            latest_sha: None,
            installed_version: status.assets_version,
            desired_version: Some(desired),
            last_checked: status.last_agent_check,
            update_available,
        });
    }

    let latest_sha = fetch_latest_bundle_sha().await;
    let update_available = latest_sha.as_deref().is_some_and(|latest| {
        agent_bundle::is_stale(status.agents_installed_sha.as_deref(), latest)
    });
    Ok(AgentBundleStatus {
        manageable,
        installed_sha: status.agents_installed_sha,
        latest_sha,
        installed_version: status.assets_version,
        desired_version: None,
        last_checked: status.last_agent_check,
        update_available,
    })
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentBundleReport {
    pub source: BundleSource,
    pub stdout: String,
}

#[tauri::command]
#[specta::specta]
pub async fn update_agent_bundle(state: State<'_, AppState>) -> CmdResult<AgentBundleReport> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let stdout = HyprCli::new(&state.binaries)
        .ai_reinstall(&cwd)
        .await
        .map_err(|e| AppError {
            code: "agent_bundle_update_failed".into(),
            message: e.to_string(),
            remedy: Some(
                "Check your network connection and try again. The CLI's own error is under \"Technical details\"."
                    .into(),
            ),
        })?;

    Ok(AgentBundleReport {
        source: agent_bundle::reinstall_source(&stdout),
        stdout,
    })
}

const VERSION_LIST_LIMIT: u32 = 10;

async fn require_manageable(cli: &HyprCli<'_>, cwd: &Path) -> CmdResult<()> {
    let status = cli.ai_status(cwd).await.map_err(AppError::from)?;
    if !agent_bundle::is_manageable_pair(&status) {
        return Err(AppError {
            code: "agent_bundle_not_manageable".into(),
            message: "Bundle versions require both Claude Code and Codex to be provisioned.".into(),
            remedy: Some("Update the Hyprlayer CLI, then run `hyprlayer ai reinstall`.".into()),
        });
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn agent_bundle_versions(state: State<'_, AppState>) -> CmdResult<AiVersions> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let cli = HyprCli::new(&state.binaries);
    require_manageable(&cli, &cwd).await?;
    cli.ai_versions(&cwd, VERSION_LIST_LIMIT)
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

#[tauri::command]
#[specta::specta]
pub async fn repair_agent_bundle(state: State<'_, AppState>) -> CmdResult<AgentBundleInstall> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let cli = HyprCli::new(&state.binaries);
    let mut output = cli
        .ai_reinstall_selected(&cwd, BundleSelection::Force)
        .await
        .map_err(AppError::from)?;

    // the CLI refuses to overwrite an occupied path, so clear what blocks it and retry once
    let mut resolved: Option<agent_bundle::CollisionReport> = None;
    if output.exit_code != Some(0) {
        if let Ok(status) = cli.ai_status(&cwd).await {
            let stamp = agent_bundle::backup_stamp(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            let report = agent_bundle::clear_link_collisions(&status, &stamp);
            if !report.is_empty() {
                output = cli
                    .ai_reinstall_selected(&cwd, BundleSelection::Force)
                    .await
                    .map_err(AppError::from)?;
                resolved = Some(report);
            }
        }
    }

    Ok(AgentBundleInstall {
        ok: output.exit_code == Some(0),
        exit_code: output.exit_code,
        source: agent_bundle::reinstall_source(&output.stdout),
        stdout: output.stdout,
        stderr: output.stderr,
        resolved,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn install_agent_bundle_version(
    state: State<'_, AppState>,
    version: Option<String>,
) -> CmdResult<AgentBundleInstall> {
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let cli = HyprCli::new(&state.binaries);
    require_manageable(&cli, &cwd).await?;

    if let Some(v) = version.as_deref() {
        let listed = cli
            .ai_versions(&cwd, VERSION_LIST_LIMIT)
            .await
            .map_err(AppError::from)?;
        if !listed.versions.iter().any(|e| e.version == v) {
            return Err(AppError {
                code: "agent_bundle_version_unavailable".into(),
                message: format!("No agent bundle is published for version {v}."),
                remedy: Some(
                    "Pick a version from the list. Installing an unlisted version would fall back to an unversioned bundle."
                        .into(),
                ),
            });
        }
    }

    let selection = match version.as_deref() {
        Some(v) => BundleSelection::Version(v),
        None => BundleSelection::Unpin,
    };
    let output = cli
        .ai_reinstall_selected(&cwd, selection)
        .await
        .map_err(AppError::from)?;

    if output.exit_code != Some(0) {
        return Ok(AgentBundleInstall {
            ok: false,
            exit_code: output.exit_code,
            source: agent_bundle::reinstall_source(&output.stdout),
            stdout: output.stdout,
            stderr: output.stderr,
            resolved: None,
        });
    }

    Ok(AgentBundleInstall {
        ok: true,
        exit_code: output.exit_code,
        source: agent_bundle::reinstall_source(&output.stdout),
        stdout: output.stdout,
        stderr: output.stderr,
        resolved: None,
    })
}
