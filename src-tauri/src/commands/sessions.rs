use std::sync::Arc;

use hyprlayer_core::agent::Harness;
use hyprlayer_core::claude::{ExitOutcome, PermissionMode, SessionEvent};
use hyprlayer_core::models::session::{QuestionAnswers, SessionLogEntry};
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::store::Session;
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

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
}

impl From<StartOpts> for StreamStartOpts {
    fn from(opts: StartOpts) -> Self {
        Self {
            harness: opts.harness,
            model: opts.model,
            effort: opts.effort,
            permission_mode: opts.permission_mode,
            prompt_name: opts.prompt_name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StartedSession {
    pub session: Session,
    pub prompt: String,
}

/// Pumps one server-side session stream into the Tauri channel the frontend
/// holds. Lag recovery now happens inside the server's own stream forwarder
/// (`hyprlayer-server::streams::run_stream`), so this is just `pump`'s shape
/// with an early return on the terminal `Exited` event.
fn spawn_live_pump(
    client: Arc<RpcClient>,
    stream_id: u64,
    mut events: EventStream<SessionEvent>,
    on_event: Channel<SessionEvent>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let terminal = matches!(event, SessionEvent::Exited { .. });
            if on_event.send(event).is_err() {
                let _ = client.cancel_stream(stream_id).await;
                return;
            }
            if terminal {
                return;
            }
        }
    });
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
    on_event: Channel<SessionEvent>,
) -> CmdResult<StartedSession> {
    let (session, prompt, stream_id, events) = state
        .client
        .start_stage_session_stream(workstream_id, stage, input, attached_docs, opts.into())
        .await
        .map_err(AppError::from)?;
    let session_id = session.id.clone();
    spawn_live_pump(state.client.clone(), stream_id, events, on_event.clone());

    // The launch is fire-and-forget, exactly as before: the command returns
    // as soon as the stream is open, and this task's only job is to surface
    // a launch failure onto that same channel. Unlike the streaming commands
    // above, this has no server-side counterpart — the server ran
    // `prepare_start` synchronously; only `launch_and_converse` is deferred.
    let client = state.client.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = client.launch_and_converse(session_id).await {
            emit_start_failure(&on_event, &AppError::from(e).message);
        }
    });

    Ok(StartedSession { session, prompt })
}

fn emit_start_failure(channel: &Channel<SessionEvent>, reason: &str) {
    let _ = channel.send(SessionEvent::AssistantText {
        text: format!("⚠ Failed to start the agent: {reason}"),
        parent_id: None,
        injected: false,
    });
    let _ = channel.send(SessionEvent::Exited {
        outcome: ExitOutcome::SpawnFailed,
        code: None,
        stderr_tail: String::new(),
    });
}

#[tauri::command]
#[specta::specta]
pub async fn resume_session(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let (session, stream_id, events) = state
        .client
        .resume_session_stream(session_id, model, effort, permission_mode)
        .await
        .map_err(AppError::from)?;
    spawn_live_pump(state.client.clone(), stream_id, events, on_event);
    Ok(session)
}

#[tauri::command]
#[specta::specta]
pub async fn restart_session(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let (session, stream_id, events) = state
        .client
        .restart_session_stream(session_id, model, effort, permission_mode)
        .await
        .map_err(AppError::from)?;
    spawn_live_pump(state.client.clone(), stream_id, events, on_event);
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
    on_event: Channel<SessionEvent>,
) -> CmdResult<RewindResult> {
    let (session, live_rewound, stream_id, events) = state
        .client
        .rewind_session_stream(session_id, user_turn, permission_mode)
        .await
        .map_err(AppError::from)?;
    spawn_live_pump(state.client.clone(), stream_id, events, on_event);
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
) -> CmdResult<()> {
    state
        .client
        .send_user_message(session_id, text, attached_docs, prompt_name)
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
        .client
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
        .client
        .answer_question(session_id, request_id, answers)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn interrupt_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .client
        .interrupt_session(session_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn kill_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .client
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
        .client
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
        .client
        .set_session_effort(session_id, effort)
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
        .client
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
        .client
        .get_session_log(session_id)
        .await
        .map_err(AppError::from)?;
    Ok(entries.into_iter().map(SessionLogEntryDto::from).collect())
}

#[tauri::command]
#[specta::specta]
pub async fn attach_session(
    state: State<'_, AppState>,
    session_id: String,
    on_event: Channel<SessionEvent>,
) -> CmdResult<()> {
    let (stream_id, events) = state
        .client
        .attach_session_stream(session_id)
        .await
        .map_err(AppError::from)?;
    spawn_live_pump(state.client.clone(), stream_id, events, on_event);
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
        .client
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
