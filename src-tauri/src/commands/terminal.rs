use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use hyprlayer_core::models::{
    PtyAgent, PtyControlOutcome, PtyLaunchSession, PtyLaunched, TerminalTheme,
};
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_proto::WireError;
use hyprlayer_server::client::{RpcClient, TerminalStream};
use tauri::ipc::Channel;
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// Whether the desktop's terminals are dark, and their colours, kept so a reconnect can resend them.
#[derive(Default)]
pub struct TerminalColors {
    dark: Mutex<Option<bool>>,
    theme: Mutex<Option<TerminalTheme>>,
}

impl TerminalColors {
    pub fn overlay(&self) -> Option<(String, String)> {
        let dark = (*self.dark.lock().unwrap_or_else(PoisonError::into_inner))?;
        Some(colorfgbg(dark))
    }

    pub fn theme(&self) -> Option<TerminalTheme> {
        *self.theme.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub async fn send_theme(client: &RpcClient, theme: TerminalTheme) -> Result<(), WireError> {
    if !client.server_features().screen_frames {
        return Ok(());
    }
    // server-compat: gated on screen_frames
    client.terminal_theme(theme).await
}

// rxvt's "fg;bg" palette indices, which claude's "auto" theme and vim read before any OSC 11 reply.
fn colorfgbg(dark: bool) -> (String, String) {
    let value = if dark { "15;0" } else { "0;15" };
    ("COLORFGBG".to_string(), value.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn set_terminal_color_scheme(
    state: State<'_, AppState>,
    colors: State<'_, TerminalColors>,
    dark: bool,
) -> CmdResult<()> {
    *colors.dark.lock().unwrap_or_else(PoisonError::into_inner) = Some(dark);
    state
        .client()?
        .set_env_overlay(BTreeMap::from([colorfgbg(dark)]))
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_terminal_theme(
    state: State<'_, AppState>,
    colors: State<'_, TerminalColors>,
    theme: TerminalTheme,
) -> CmdResult<()> {
    *colors.theme.lock().unwrap_or_else(PoisonError::into_inner) = Some(theme);
    send_theme(&*state.client()?, theme)
        .await
        .map_err(AppError::from)
}

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
        .client()?
        .terminal_open_stream(workstream_id, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(&state, state.client()?, stream_id, events, on_event);
    Ok(terminal_id)
}

#[tauri::command]
#[specta::specta]
/// From `offset` when the server still holds it; each output event says where it starts.
// `f64`: specta won't hand TypeScript a `u64`.
pub async fn terminal_attach(
    state: State<'_, AppState>,
    terminal_id: String,
    offset: Option<f64>,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<bool> {
    match state
        .client()?
        .terminal_attach_stream(terminal_id, offset.map(|at| at as u64))
        .await
        .map_err(AppError::from)?
    {
        Some(attached) => {
            pump(
                &state,
                state.client()?,
                attached.stream_id,
                attached.events,
                on_event,
            );
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
        .client()?
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
        .client()?
        .terminal_resize(terminal_id, cols, rows)
        .await
        .map_err(AppError::from)
}

/// The user is at this terminal: a terminal shared with another desktop takes this one's size.
#[tauri::command]
#[specta::specta]
pub async fn terminal_focus(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    state
        .client()?
        .terminal_focus(terminal_id)
        .await
        .map_err(AppError::from)
}

/// A terminal shared with another desktop takes this one's size, and keeps it until another takes it.
#[tauri::command]
#[specta::specta]
pub async fn terminal_take_size(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    let client = state.client()?;
    if !client.server_features().size_owner {
        return Ok(());
    }
    // server-compat: gated on size_owner
    client
        .terminal_take_size(terminal_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn terminal_close(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    state
        .client()?
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
        .client()?
        .spawn_cli_terminal_stream(args, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(&state, state.client()?, stream_id, events, on_event);
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
        .client()?
        .spawn_shell_terminal_stream(command, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(&state, state.client()?, stream_id, events, on_event);
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
        .client()?
        .agent_open_stream(workstream_id, command, args, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(&state, state.client()?, stream_id, events, on_event);
    Ok(terminal_id)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_launch(
    state: State<'_, AppState>,
    workstream_id: String,
    engine_id: String,
    session: PtyLaunchSession,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<PtyLaunched> {
    let (launched, stream_id, events) = state
        .client()?
        .pty_launch_stream(workstream_id, engine_id, session, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(&state, state.client()?, stream_id, events, on_event);
    Ok(launched)
}

#[tauri::command]
#[specta::specta]
pub async fn list_pty_agents(
    state: State<'_, AppState>,
    workstream_id: Option<String>,
) -> CmdResult<Vec<PtyAgent>> {
    state
        .client()?
        .list_pty_agents(workstream_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_agent_set_model(
    state: State<'_, AppState>,
    terminal_id: String,
    model: String,
) -> CmdResult<PtyControlOutcome> {
    state
        .client()?
        .pty_agent_set_model(terminal_id, model)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_agent_set_effort(
    state: State<'_, AppState>,
    terminal_id: String,
    effort: String,
) -> CmdResult<()> {
    state
        .client()?
        .pty_agent_set_effort(terminal_id, effort)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_agent_set_permission_mode(
    state: State<'_, AppState>,
    terminal_id: String,
    mode: String,
) -> CmdResult<()> {
    state
        .client()?
        .pty_agent_set_permission_mode(terminal_id, mode)
        .await
        .map_err(AppError::from)
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
        .client()?
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
        .client()?
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
        .client()?
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
        .client()?
        .agent_session_exists(workstream_id, engine_id, session_id)
        .await
        .map_err(AppError::from)
}

/// Pumps one server-side stream into the Tauri channel the frontend holds.
/// Shared by every `Channel<TerminalEvent>` command in this module and by
/// `commands::scripts::script_run`, since a terminal stream and a script's
/// own terminal stream are the same shape once both come off the wire.
pub(crate) fn pump(
    state: &AppState,
    client: std::sync::Arc<hyprlayer_server::client::RpcClient>,
    stream_id: u64,
    mut events: TerminalStream,
    on_event: Channel<TerminalEvent>,
) {
    let cancel = client.clone();
    state
        .streams()
        .spawn(on_event.id(), client, stream_id, async move {
            while let Some(ev) = events.recv().await {
                if on_event.send(ev).is_err() {
                    log::debug!("terminal stream {stream_id}: the window stopped listening");
                    let _ = cancel.cancel_stream(stream_id).await;
                    break;
                }
            }
        });
}

#[tauri::command]
#[specta::specta]
// `f64`: specta won't hand TypeScript a `u64`.
pub async fn terminal_frame_ack(
    state: State<'_, AppState>,
    channel_id: u32,
    seq: f64,
) -> CmdResult<()> {
    state
        .streams()
        .ack_frame(channel_id, seq as u64)
        .await
        .map_err(AppError::from)
}

/// Ends the server streams behind channels the webview released; their terminals and sessions keep running.
#[tauri::command]
#[specta::specta]
pub async fn terminal_release(state: State<'_, AppState>, channel_ids: Vec<u32>) -> CmdResult<()> {
    state.streams().release(&channel_ids);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn default_shell_name(state: State<'_, AppState>) -> CmdResult<String> {
    state
        .client()?
        .default_shell_name()
        .await
        .map_err(AppError::from)
}
