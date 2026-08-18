//! IPC surface for Claude Code agent definitions. Thin by design (ADR-0001):
//! discovery, parsing and rendering live in `hyprlayer_core::agents`.

use std::path::{Path, PathBuf};

use hyprlayer_core::agents::{self, AgentContextFile, AgentFile, AgentMcpServer, AgentOverlay};
use hyprlayer_core::skills::AgentSkill;

use crate::commands::blocking;
use crate::error::{AppError, CmdResult};

/// Every agent visible from `project_path`, that project's own plus the user's.
#[tauri::command]
#[specta::specta]
pub async fn list_agent_files(project_path: Option<String>) -> CmdResult<Vec<AgentFile>> {
    blocking("list_agent_files", move || {
        Ok(agents::list_agents(project_path.as_deref().map(Path::new)))
    })
    .await?
}

/// Write `agent` back to its own `.md` file, returning it as re-read from disk.
#[tauri::command]
#[specta::specta]
pub async fn save_agent_file(agent: AgentFile) -> CmdResult<AgentFile> {
    blocking("save_agent_file", move || {
        agents::save_agent(&agent).map_err(AppError::from)
    })
    .await?
}

/// Delete an agent definition, the file IS the agent.
#[tauri::command]
#[specta::specta]
pub async fn delete_agent_file(path: String) -> CmdResult<()> {
    blocking("delete_agent_file", move || {
        agents::delete_agent(Path::new(&path)).map_err(AppError::from)
    })
    .await?
}

/// The path a new agent called `name` would occupy in `source`'s directory.
/// Returns `None` when that root doesn't apply (a project agent with no project).
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

/// Docs in the project worth appending to an agent's prompt.
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

/// MCP servers claude is configured with, for the Capabilities tab.
#[tauri::command]
#[specta::specta]
pub async fn agent_mcp_servers(project_path: Option<String>) -> CmdResult<Vec<AgentMcpServer>> {
    blocking("agent_mcp_servers", move || {
        Ok(agents::mcp_servers(project_path.as_deref().map(Path::new)))
    })
    .await?
}

/// Skills an agent's `skills:` key can name, for the Capabilities tab.
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

/// Write a portable `.agent.json` to Downloads; returns where it landed.
#[tauri::command]
#[specta::specta]
pub async fn export_agent_file(agent: AgentFile, overlay: AgentOverlay) -> CmdResult<String> {
    blocking("export_agent_file", move || {
        agents::export_agent(&agent, &overlay).map_err(AppError::from)
    })
    .await?
}
