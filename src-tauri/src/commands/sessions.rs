use std::sync::Arc;

use hyprlayer_core::agent::Harness;
use hyprlayer_core::claude::{ExitOutcome, PermissionMode, SessionEvent};
use hyprlayer_core::models::session::{LogCursor, QuestionAnswers, SessionLogEntry};
use hyprlayer_core::models::{ClientRef, SessionRev, SessionTurns};
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::store::Session;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::rpc::WatchedSession;
use hyprlayer_server::streams::{
    SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use crate::client_identity::ClientIdentity;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct StartOpts {
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub prompt_name: Option<String>,
    pub client_message_id: Option<String>,
}

impl From<StartOpts> for StreamStartOpts {
    fn from(opts: StartOpts) -> Self {
        Self {
            harness: opts.harness,
            model: opts.model,
            effort: opts.effort,
            permission_mode: opts.permission_mode,
            prompt_name: opts.prompt_name,
            client_message_id: opts.client_message_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StartedSession {
    pub session: Session,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResumedSession {
    pub session: Session,
    pub permission_mode: Option<PermissionMode>,
}

/// `seq`/`rev`/`author` are null for an ephemeral event and for a server without transcript sync.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionStreamItemDto {
    pub seq: Option<f64>,
    pub rev: Option<f64>,
    pub author: Option<ClientRef>,
    pub event: SessionEvent,
}

impl From<SessionFrame> for SessionStreamItemDto {
    fn from(frame: SessionFrame) -> Self {
        match frame {
            SessionFrame::Item(item) => Self {
                seq: item.seq.map(|seq| seq as f64),
                rev: Some(item.rev as f64),
                author: item.author,
                event: item.event,
            },
            SessionFrame::Bare(event) => Self {
                seq: None,
                rev: None,
                author: None,
                event,
            },
        }
    }
}

/// Pumps one server-side session stream into the Tauri channel the frontend
/// holds. Lag recovery now happens inside the server's own stream forwarder
/// (`hyprlayer-server::streams::run_stream`), so this is just `pump`'s shape
/// with an early return on the terminal `Exited` event.
fn spawn_live_pump(
    client: Arc<RpcClient>,
    stream_id: u64,
    mut events: SessionEvents,
    on_event: Channel<SessionStreamItemDto>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(frame) = events.recv().await {
            let item = SessionStreamItemDto::from(frame);
            let terminal = matches!(item.event, SessionEvent::Exited { .. });
            if on_event.send(item).is_err() {
                log::debug!("session stream {stream_id}: the window stopped listening");
                let _ = client.cancel_stream(stream_id).await;
                return;
            }
            if terminal {
                return;
            }
        }
        // Ended with no `Exited`: the connection dropped or a frame did not decode.
        log::info!("session stream {stream_id} ended before the session exited");
    });
}

/// The server answered with another command's response; nothing will read its stream.
async fn unexpected_response(client: &RpcClient, stream_id: u64, command: &str) -> AppError {
    let _ = client.cancel_stream(stream_id).await;
    AppError::internal(format!(
        "{command} was answered with another command's response"
    ))
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn start_stage_session(
    state: State<'_, AppState>,
    workstream_id: String,
    stage: Stage,
    input: String,
    attached_docs: Vec<String>,
    opts: StartOpts,
    on_event: Channel<SessionStreamItemDto>,
) -> CmdResult<StartedSession> {
    let client = state.client()?;
    let (response, stream_id, events) = client
        .open_session_stream(StreamRequest::StartStageSession {
            workstream_id,
            stage,
            input,
            attached_docs,
            opts: opts.into(),
            echo_user_messages: true,
            with_meta: state.transcript_sync(),
        })
        .await
        .map_err(AppError::from)?;
    let StreamResponse::StartStageSession {
        session, prompt, ..
    } = response
    else {
        return Err(unexpected_response(&client, stream_id, "start_stage_session").await);
    };
    let session_id = session.id.clone();
    spawn_live_pump(client.clone(), stream_id, events, on_event.clone());

    // The launch is fire-and-forget, exactly as before: the command returns
    // as soon as the stream is open, and this task's only job is to surface
    // a launch failure onto that same channel. Unlike the streaming commands
    // above, this has no server-side counterpart — the server ran
    // `prepare_start` synchronously; only `launch_and_converse` is deferred.
    tauri::async_runtime::spawn(async move {
        if let Err(e) = client.launch_and_converse(session_id.clone()).await {
            log::warn!(
                "session {session_id} failed to launch: {}: {}",
                e.code,
                e.message
            );
            emit_start_failure(&on_event, &AppError::from(e).message);
        }
    });

    Ok(StartedSession { session, prompt })
}

fn emit_start_failure(channel: &Channel<SessionStreamItemDto>, reason: &str) {
    let failure = [
        SessionEvent::AssistantText {
            text: format!("⚠ Failed to start the agent: {reason}"),
            parent_id: None,
            injected: false,
        },
        SessionEvent::Exited {
            outcome: ExitOutcome::SpawnFailed,
            code: None,
            stderr_tail: String::new(),
        },
    ];
    for event in failure {
        let _ = channel.send(SessionFrame::Bare(event).into());
    }
}

/// A cursor crossing the Tauri boundary: `seq` is `f64` because a specta `i64`
/// exports as a BigInt the TS generator rejects.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LogCursorDto {
    Empty,
    After { seq: f64, created_at: f64 },
}

impl From<LogCursor> for LogCursorDto {
    fn from(cursor: LogCursor) -> Self {
        match cursor {
            LogCursor::Empty => LogCursorDto::Empty,
            LogCursor::After { seq, created_at } => LogCursorDto::After {
                seq: seq as f64,
                created_at,
            },
        }
    }
}

impl From<LogCursorDto> for LogCursor {
    fn from(dto: LogCursorDto) -> Self {
        match dto {
            LogCursorDto::Empty => LogCursor::Empty,
            LogCursorDto::After { seq, created_at } => LogCursor::After {
                seq: seq as i64,
                created_at,
            },
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn resume_session(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    cursor: Option<LogCursorDto>,
    on_event: Channel<SessionStreamItemDto>,
) -> CmdResult<ResumedSession> {
    let client = state.client()?;
    let (response, stream_id, events) = client
        .open_session_stream(StreamRequest::ResumeSession {
            session_id,
            model,
            effort,
            permission_mode,
            cursor: cursor.map(LogCursor::from),
            echo_user_messages: true,
            with_meta: state.transcript_sync(),
        })
        .await
        .map_err(AppError::from)?;
    let StreamResponse::Session {
        session,
        permission_mode,
        ..
    } = response
    else {
        return Err(unexpected_response(&client, stream_id, "resume_session").await);
    };
    spawn_live_pump(client, stream_id, events, on_event);
    Ok(ResumedSession {
        session,
        permission_mode,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn restart_session(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionStreamItemDto>,
) -> CmdResult<Session> {
    let client = state.client()?;
    let (response, stream_id, events) = client
        .open_session_stream(StreamRequest::RestartSession {
            session_id,
            model,
            effort,
            permission_mode,
            echo_user_messages: true,
            with_meta: state.transcript_sync(),
        })
        .await
        .map_err(AppError::from)?;
    let StreamResponse::Session { session, .. } = response else {
        return Err(unexpected_response(&client, stream_id, "restart_session").await);
    };
    spawn_live_pump(client, stream_id, events, on_event);
    Ok(session)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RewindResult {
    pub session: Session,
    pub live_rewound: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn rewind_session(
    state: State<'_, AppState>,
    session_id: String,
    user_turn: u32,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionStreamItemDto>,
) -> CmdResult<RewindResult> {
    let client = state.client()?;
    let (response, stream_id, events) = client
        .open_session_stream(StreamRequest::RewindSession {
            session_id,
            user_turn,
            permission_mode,
            echo_user_messages: true,
            with_meta: state.transcript_sync(),
        })
        .await
        .map_err(AppError::from)?;
    let StreamResponse::RewindSession {
        session,
        live_rewound,
        ..
    } = response
    else {
        return Err(unexpected_response(&client, stream_id, "rewind_session").await);
    };
    spawn_live_pump(client, stream_id, events, on_event);
    Ok(RewindResult {
        session,
        live_rewound,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn send_user_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
    attached_docs: Vec<String>,
    prompt_name: Option<String>,
    client_message_id: Option<String>,
    base_rev: Option<f64>,
) -> CmdResult<()> {
    state
        .client()?
        .send_user_message(
            session_id,
            text,
            attached_docs,
            prompt_name,
            client_message_id,
            base_rev.map(|rev| rev as u64),
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_title(
    state: State<'_, AppState>,
    session_id: String,
    title: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .set_session_title(session_id, title)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn recap_session(
    state: State<'_, AppState>,
    session_id: String,
) -> CmdResult<Option<String>> {
    state
        .client()?
        .recap_session(session_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_archived(
    state: State<'_, AppState>,
    session_id: String,
    archived: bool,
) -> CmdResult<()> {
    state
        .client()?
        .set_session_archived(session_id, archived)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .client()?
        .delete_session(session_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn respond_permission(
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    allow: bool,
    message: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .respond_permission(session_id, request_id, allow, message)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn answer_question(
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    answers: QuestionAnswers,
) -> CmdResult<()> {
    state
        .client()?
        .answer_question(session_id, request_id, answers)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn interrupt_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .client()?
        .interrupt_session(session_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_queued_message(
    state: State<'_, AppState>,
    session_id: String,
    client_message_id: String,
) -> CmdResult<bool> {
    state
        .client()?
        .cancel_queued_message(session_id, client_message_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn kill_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .client()?
        .kill_session(session_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_model(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .set_session_model(session_id, model)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_effort(
    state: State<'_, AppState>,
    session_id: String,
    effort: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .set_session_effort(session_id, effort)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_permission_mode(
    state: State<'_, AppState>,
    session_id: String,
    mode: Option<PermissionMode>,
) -> CmdResult<()> {
    state
        .client()?
        .set_session_permission_mode(session_id, mode)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_sessions(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<Vec<Session>> {
    state
        .client()?
        .list_sessions(workstream_id)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogEntryDto {
    pub seq: f64,
    pub kind: String,
    pub event: SessionEvent,
    pub created_at: f64,
}

impl From<SessionLogEntry> for SessionLogEntryDto {
    fn from(entry: SessionLogEntry) -> Self {
        Self {
            seq: entry.seq as f64,
            kind: entry.kind,
            event: entry.event,
            created_at: entry.created_at,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_session_log(
    state: State<'_, AppState>,
    session_id: String,
) -> CmdResult<Vec<SessionLogEntryDto>> {
    let entries = state
        .client()?
        .get_session_log(session_id)
        .await
        .map_err(AppError::from)?;
    Ok(entries.into_iter().map(SessionLogEntryDto::from).collect())
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurnsDto {
    pub entries: Vec<SessionLogEntryDto>,
    pub next_cursor: LogCursorDto,
    pub remaining: u32,
    pub rev: f64,
    pub reset_rev: f64,
    pub working: bool,
    pub in_flight: u32,
}

impl From<SessionTurns> for SessionTurnsDto {
    fn from(turns: SessionTurns) -> Self {
        Self {
            entries: turns
                .entries
                .into_iter()
                .map(SessionLogEntryDto::from)
                .collect(),
            next_cursor: turns.next_cursor.into(),
            remaining: turns.remaining,
            rev: turns.rev as f64,
            reset_rev: turns.reset_rev as f64,
            working: turns.working,
            in_flight: turns.in_flight,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_session_turns(
    state: State<'_, AppState>,
    session_id: String,
    cursor: LogCursorDto,
    limit: u32,
) -> CmdResult<SessionTurnsDto> {
    let turns = state
        .client()?
        .list_session_turns(session_id, cursor.into(), limit)
        .await
        .map_err(AppError::from)?;
    Ok(turns.into())
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WatchedSessionDto {
    pub session_id: String,
    /// The rev this view last applied.
    pub rev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionRevDto {
    pub session_id: String,
    pub rev: f64,
    pub reset_rev: f64,
    pub working: bool,
    pub in_flight: u32,
    pub viewers: Vec<ClientRef>,
}

impl From<SessionRev> for SessionRevDto {
    fn from(rev: SessionRev) -> Self {
        Self {
            session_id: rev.session_id,
            rev: rev.rev as f64,
            reset_rev: rev.reset_rev as f64,
            working: rev.working,
            in_flight: rev.in_flight,
            viewers: rev.viewers,
        }
    }
}

/// Replaces this connection's watched set; the reply recovers any dropped sync event.
#[tauri::command]
#[specta::specta]
pub async fn watch_sessions(
    state: State<'_, AppState>,
    sessions: Vec<WatchedSessionDto>,
) -> CmdResult<Vec<SessionRevDto>> {
    let sessions = sessions
        .into_iter()
        .map(|s| WatchedSession {
            session_id: s.session_id,
            rev: s.rev as u64,
        })
        .collect();
    let revs = state
        .client()?
        .watch_sessions(sessions)
        .await
        .map_err(AppError::from)?;
    Ok(revs.into_iter().map(SessionRevDto::from).collect())
}

/// Local: the identity this app announces on every connect.
#[tauri::command]
#[specta::specta]
pub fn get_client_identity(identity: State<'_, ClientIdentity>) -> CmdResult<ClientIdentity> {
    Ok(identity.inner().clone())
}

#[tauri::command]
#[specta::specta]
pub async fn attach_session(
    state: State<'_, AppState>,
    session_id: String,
    on_event: Channel<SessionStreamItemDto>,
) -> CmdResult<()> {
    let client = state.client()?;
    let (response, stream_id, events) = client
        .open_session_stream(StreamRequest::AttachSession {
            session_id,
            echo_user_messages: true,
            with_meta: state.transcript_sync(),
        })
        .await
        .map_err(AppError::from)?;
    let StreamResponse::AttachSession { .. } = response else {
        return Err(unexpected_response(&client, stream_id, "attach_session").await);
    };
    spawn_live_pump(client, stream_id, events, on_event);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSessionView {
    pub session: Session,
    pub workstream_title: String,
    pub project_path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn list_active_sessions(state: State<'_, AppState>) -> CmdResult<Vec<ActiveSessionView>> {
    let active = state
        .client()?
        .list_active_sessions()
        .await
        .map_err(AppError::from)?;
    Ok(active
        .into_iter()
        .map(|a| ActiveSessionView {
            workstream_title: a.workstream.title,
            project_path: a.workstream.project_path,
            session: a.session,
        })
        .collect())
}
