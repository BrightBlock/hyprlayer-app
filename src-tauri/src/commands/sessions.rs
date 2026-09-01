use hyprlayer_core::agent::Harness;
use hyprlayer_core::claude::{ExitOutcome, PermissionMode, SessionEvent};
use hyprlayer_core::models::session::{QuestionAnswers, SessionLogEntry};
use hyprlayer_core::opencode::support::opencode_model_is_supported;
use hyprlayer_core::pipeline::{self, Stage};
use hyprlayer_core::runtime::sessions::{
    LaunchSessionParams, RewindSessionRequest, SessionStreamError, StartStageSessionRequest,
};
use hyprlayer_core::store::{Session, Workstream};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri::State;
use tokio::sync::mpsc;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub fn session_not_found(id: &str) -> AppError {
    AppError {
        code: "session_not_found".into(),
        message: format!("No live session with id {id}"),
        remedy: Some("The session may have ended; start a new one.".into()),
    }
}

async fn load_session(state: &AppState, id: &str) -> CmdResult<Session> {
    state
        .runtime
        .sessions()
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| session_not_found(id))
}

async fn load_workstream(state: &AppState, id: &str) -> CmdResult<Workstream> {
    state
        .runtime
        .workstreams()
        .find(id)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Default, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct StartOpts {
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub prompt_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StartedSession {
    pub session: Session,
    pub prompt: String,
}

fn spawn_live_pump(
    app: tauri::AppHandle,
    session_id: String,
    mut live: mpsc::Receiver<Result<SessionEvent, SessionStreamError>>,
    on_event: Channel<SessionEvent>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            match live.recv().await {
                Some(Ok(event)) => {
                    let terminal = matches!(event, SessionEvent::Exited { .. });
                    if on_event.send(event).is_err() {
                        return;
                    }
                    if terminal {
                        return;
                    }
                }
                Some(Err(SessionStreamError::Lagged {
                    epoch,
                    last_persisted_seq,
                })) => {
                    let state = app.state::<AppState>();
                    let sessions = state.runtime.sessions();
                    let recovered = sessions
                        .log_after(&session_id, epoch, last_persisted_seq)
                        .await;
                    let entries = match recovered {
                        Ok(entries) => entries,
                        Err(e) => {
                            log::warn!("session {session_id}: lag recovery failed: {e}");
                            let _ = on_event.send(SessionEvent::ProtocolError {
                                detail: format!("could not recover from a lagged stream: {e}"),
                                raw: String::new(),
                            });
                            return;
                        }
                    };
                    for entry in entries {
                        if on_event.send(entry.event).is_err() {
                            return;
                        }
                    }
                    match sessions.attach(&session_id).await {
                        Ok(attachment) => live = attachment.live,
                        Err(e) => {
                            log::warn!("session {session_id}: reattach after lag failed: {e}");
                            let _ = on_event.send(SessionEvent::ProtocolError {
                                detail: format!("could not reattach after a lagged stream: {e}"),
                                raw: String::new(),
                            });
                            return;
                        }
                    }
                }
                None => return,
            }
        }
    });
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn start_stage_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workstream_id: String,
    stage: Stage,
    input: String,
    attached_docs: Vec<String>,
    opts: StartOpts,
    on_event: Channel<SessionEvent>,
) -> CmdResult<StartedSession> {
    let StartOpts {
        harness,
        model,
        effort,
        permission_mode,
        prompt_name,
    } = opts;
    let workstream = load_workstream(&state, &workstream_id).await?;

    let prepared = state
        .runtime
        .sessions()
        .prepare_start(StartStageSessionRequest {
            workstream,
            stage,
            input,
            attached_docs,
            harness,
            model,
            effort,
            permission_mode,
            prompt_name,
        })
        .await
        .map_err(AppError::from)?;

    let session = prepared.session.clone();
    let prompt = prepared.prompt.clone();
    let session_id = session.id.clone();

    let attachment = state
        .runtime
        .sessions()
        .attach(&session_id)
        .await
        .map_err(AppError::from)?;
    spawn_live_pump(app.clone(), session_id, attachment.live, on_event.clone());

    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if let Err(e) = state
            .runtime
            .sessions()
            .launch_and_converse(prepared.launch)
            .await
        {
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
#[allow(clippy::too_many_arguments)]
pub async fn resume_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let session = load_session(&state, &session_id).await?;
    let sessions = state.runtime.sessions();

    if sessions.live_driver(&session_id).await.is_some() {
        let requested = permission_mode;
        let running = sessions.permission_mode(&session_id).await;
        if requested.is_none() || requested == running {
            let attachment = sessions.attach(&session_id).await.map_err(AppError::from)?;
            spawn_live_pump(app, session_id, attachment.live, on_event);
            return Ok(session);
        }
        sessions.kill_and_await_exit(&session_id).await;
    }

    let ws = load_workstream(&state, &session.workstream_id).await?;

    let attachment = sessions.attach(&session_id).await.map_err(AppError::from)?;
    spawn_live_pump(app, session_id, attachment.live, on_event);

    sessions
        .launch(LaunchSessionParams {
            session: session.clone(),
            project_path: ws.project_path,
            model,
            effort,
            permission_mode,
            resume: true,
            boundary_on_relaunch: true,
        })
        .await
        .map_err(AppError::from)?;

    Ok(session)
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn restart_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let session = load_session(&state, &session_id).await?;
    let sessions = state.runtime.sessions();

    sessions.kill_and_await_exit(&session_id).await;

    let ws = load_workstream(&state, &session.workstream_id).await?;

    let attachment = sessions.attach(&session_id).await.map_err(AppError::from)?;
    spawn_live_pump(app, session_id, attachment.live, on_event);

    sessions
        .launch(LaunchSessionParams {
            session: session.clone(),
            project_path: ws.project_path,
            model,
            effort,
            permission_mode,
            resume: true,
            boundary_on_relaunch: true,
        })
        .await
        .map_err(AppError::from)?;

    Ok(session)
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RewindResult {
    pub session: Session,
    pub live_rewound: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn rewind_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    user_turn: u32,
    permission_mode: Option<PermissionMode>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<RewindResult> {
    let session = load_session(&state, &session_id).await?;
    let workstream = load_workstream(&state, &session.workstream_id).await?;
    let sessions = state.runtime.sessions();

    let attachment = sessions.attach(&session_id).await.map_err(AppError::from)?;
    spawn_live_pump(app, session_id, attachment.live, on_event);

    let outcome = sessions
        .rewind(RewindSessionRequest {
            session,
            workstream,
            user_turn,
            permission_mode,
        })
        .await
        .map_err(AppError::from)?;

    Ok(RewindResult {
        session: outcome.session,
        live_rewound: outcome.live_rewound,
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
    let sessions = state.runtime.sessions();
    let op = sessions.op_lock(&session_id).await;
    let _op_guard = op.lock().await;

    let prompt = pipeline::append_docs(text, &attached_docs);
    sessions
        .live_driver(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?
        .send_user_turn(&prompt, None, prompt_name.as_deref())
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
        .runtime
        .sessions()
        .live_driver(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?
        .respond_permission(&request_id, allow, message.as_deref())
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
        .runtime
        .sessions()
        .live_driver(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?
        .answer_question(&request_id, answers)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn interrupt_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .runtime
        .sessions()
        .live_driver(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?
        .interrupt()
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn kill_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    state
        .runtime
        .sessions()
        .live_driver(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?
        .kill();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_model(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
) -> CmdResult<()> {
    if let Some(model_id) = model.as_deref() {
        if load_session(&state, &session_id).await?.harness == Harness::Opencode {
            if !opencode_model_is_supported(model_id) {
                return Err(AppError::from(CoreError::OpencodeProviderUnsupported));
            }
            if let Some(server) = state.runtime.providers().opencode_server_if_running().await {
                if server.knows_model(model_id).await == Some(false) {
                    return Err(AppError::from(CoreError::OpencodeModelUnavailable {
                        model: model_id.to_string(),
                    }));
                }
            }
        }
    }
    if let Some(driver) = state.runtime.sessions().live_driver(&session_id).await {
        driver.set_model(model);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_effort(
    state: State<'_, AppState>,
    session_id: String,
    effort: Option<String>,
) -> CmdResult<()> {
    if let Some(driver) = state.runtime.sessions().live_driver(&session_id).await {
        driver.set_effort(effort);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn list_sessions(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<Vec<Session>> {
    state
        .runtime
        .sessions()
        .list(&workstream_id)
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
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
        .runtime
        .sessions()
        .log(&session_id)
        .await
        .map_err(AppError::from)?;
    Ok(entries.into_iter().map(SessionLogEntryDto::from).collect())
}

#[tauri::command]
#[specta::specta]
pub async fn attach_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    on_event: Channel<SessionEvent>,
) -> CmdResult<()> {
    let attachment = state
        .runtime
        .sessions()
        .attach(&session_id)
        .await
        .map_err(AppError::from)?;
    for entry in attachment.replay {
        let _ = on_event.send(entry.event);
    }
    spawn_live_pump(app, session_id, attachment.live, on_event);
    Ok(())
}

#[derive(Debug, Clone, Serialize, specta::Type)]
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
        .runtime
        .sessions()
        .list_active()
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
