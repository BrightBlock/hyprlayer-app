use std::path::PathBuf;

use hyprlayer_core::runtime::terminals as core_terminals;
use hyprlayer_core::terminal::TerminalEvent;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn terminal_open(
    state: State<'_, AppState>,
    workstream_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let attachment = state
        .runtime
        .terminals()
        .open(core_terminals::OpenTerminalRequest {
            workstream_id,
            cols,
            rows,
        })
        .await
        .map_err(AppError::from)?;
    Ok(pump(attachment, on_event))
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_attach(
    state: State<'_, AppState>,
    terminal_id: String,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<bool> {
    let attachment = state
        .runtime
        .terminals()
        .attach(core_terminals::AttachTerminalRequest { terminal_id })
        .await
        .map_err(AppError::from)?;
    Ok(match attachment {
        Some(attachment) => {
            pump(attachment, on_event);
            true
        }
        None => false,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_write(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> CmdResult<()> {
    state
        .runtime
        .terminals()
        .write(core_terminals::WriteTerminalRequest { terminal_id, data })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_resize(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    state
        .runtime
        .terminals()
        .resize(core_terminals::ResizeTerminalRequest {
            terminal_id,
            cols,
            rows,
        })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_close(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    state
        .runtime
        .terminals()
        .close(core_terminals::CloseTerminalRequest { terminal_id })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn spawn_cli_terminal(
    state: State<'_, AppState>,
    args: Vec<String>,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let attachment = state
        .runtime
        .terminals()
        .open_cli(core_terminals::OpenCliTerminalRequest { args, cols, rows })
        .await
        .map_err(AppError::from)?;
    Ok(pump(attachment, on_event))
}

#[tauri::command]
#[specta::specta]
pub async fn spawn_shell_terminal(
    state: State<'_, AppState>,
    command: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let attachment = state
        .runtime
        .terminals()
        .open_shell(core_terminals::OpenShellTerminalRequest {
            command,
            cols,
            rows,
        })
        .await
        .map_err(AppError::from)?;
    Ok(pump(attachment, on_event))
}

#[tauri::command]
#[specta::specta]
pub async fn agent_open(
    state: State<'_, AppState>,
    workstream_id: String,
    command: String,
    args: Vec<String>,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let attachment = state
        .runtime
        .terminals()
        .open_agent(core_terminals::OpenAgentTerminalRequest {
            workstream_id,
            command,
            args,
            cols,
            rows,
        })
        .await
        .map_err(AppError::from)?;
    Ok(pump(attachment, on_event))
}

#[tauri::command]
#[specta::specta]
pub async fn agent_resize(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    state
        .runtime
        .terminals()
        .resize_agent(core_terminals::ResizeAgentTerminalRequest {
            terminal_id,
            cols,
            rows,
        })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn capture_agent_session(
    state: State<'_, AppState>,
    workstream_id: String,
    engine_id: String,
    after_ms: f64,
    exclude: Vec<String>,
) -> CmdResult<Option<String>> {
    state
        .runtime
        .terminals()
        .capture_session(core_terminals::CaptureSessionRequest {
            workstream_id,
            engine_id,
            after_ms,
            exclude,
        })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn mint_codex_session(
    state: State<'_, AppState>,
    workstream_id: String,
    command: String,
) -> CmdResult<Option<String>> {
    state
        .runtime
        .terminals()
        .mint_codex_session(core_terminals::MintCodexSessionRequest {
            workstream_id,
            command,
        })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_session_exists(
    state: State<'_, AppState>,
    workstream_id: String,
    engine_id: String,
    session_id: String,
) -> CmdResult<bool> {
    state
        .runtime
        .terminals()
        .session_exists(core_terminals::SessionExistsRequest {
            workstream_id,
            engine_id,
            session_id,
        })
        .await
        .map_err(AppError::from)
}

fn pump(
    attachment: core_terminals::TerminalAttachment,
    on_event: Channel<TerminalEvent>,
) -> String {
    let core_terminals::TerminalAttachment {
        terminal_id,
        mut live,
    } = attachment;
    let id = terminal_id.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = live.recv().await {
            if on_event.send(ev).is_err() {
                break;
            }
        }
    });
    id
}

pub(crate) fn default_shell() -> PathBuf {
    core_terminals::default_shell()
}

pub(crate) fn login_shell_args(command: &str) -> Vec<String> {
    core_terminals::login_shell_args(command)
}

#[tauri::command]
#[specta::specta]
pub fn default_shell_name() -> CmdResult<String> {
    Ok(default_shell()
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("shell")
        .to_string())
}
