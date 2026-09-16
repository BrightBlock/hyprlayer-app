use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_server::client::EventStream;
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
    let (terminal_id, stream_id, events) = state
        .client
        .terminal_open_stream(workstream_id, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(state.client.clone(), stream_id, events, on_event);
    Ok(terminal_id)
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_attach(
    state: State<'_, AppState>,
    terminal_id: String,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<bool> {
    match state
        .client
        .terminal_attach_stream(terminal_id)
        .await
        .map_err(AppError::from)?
    {
        Some((stream_id, events)) => {
            pump(state.client.clone(), stream_id, events, on_event);
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_write(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> CmdResult<()> {
    state
        .client
        .terminal_write(terminal_id, data)
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
        .client
        .terminal_resize(terminal_id, cols, rows)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_close(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    state
        .client
        .terminal_close(terminal_id)
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
    let (terminal_id, stream_id, events) = state
        .client
        .spawn_cli_terminal_stream(args, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(state.client.clone(), stream_id, events, on_event);
    Ok(terminal_id)
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
    let (terminal_id, stream_id, events) = state
        .client
        .spawn_shell_terminal_stream(command, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(state.client.clone(), stream_id, events, on_event);
    Ok(terminal_id)
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
    let (terminal_id, stream_id, events) = state
        .client
        .agent_open_stream(workstream_id, command, args, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(state.client.clone(), stream_id, events, on_event);
    Ok(terminal_id)
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
        .client
        .agent_resize(terminal_id, cols, rows)
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
        .client
        .capture_agent_session(workstream_id, engine_id, after_ms, exclude)
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
        .client
        .mint_codex_session(workstream_id, command)
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
        .client
        .agent_session_exists(workstream_id, engine_id, session_id)
        .await
        .map_err(AppError::from)
}

/// Pumps one server-side stream into the Tauri channel the frontend holds.
/// Shared by every `Channel<TerminalEvent>` command in this module and by
/// `commands::scripts::script_run`, since a terminal stream and a script's
/// own terminal stream are the same shape once both come off the wire.
pub(crate) fn pump(
    client: std::sync::Arc<hyprlayer_server::client::RpcClient>,
    stream_id: u64,
    mut events: EventStream<TerminalEvent>,
    on_event: Channel<TerminalEvent>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = events.recv().await {
            if on_event.send(ev).is_err() {
                let _ = client.cancel_stream(stream_id).await;
                break;
            }
        }
    });
}

#[tauri::command]
#[specta::specta]
pub async fn default_shell_name(state: State<'_, AppState>) -> CmdResult<String> {
    state
        .client
        .default_shell_name()
        .await
        .map_err(AppError::from)
}
