//! Agent-session IPC: start a stage session, drive it (turns, permissions,
//! interrupt, kill), and attach/replay its transcript.
//!
//! Each live session has one **forwarder task** that owns the broadcast
//! subscription: it persists every event to `session_events`, advances the
//! `sessions` row state, and pushes events onto the per-session Tauri Channel.
//! Re-attaching a UI replays the persisted log then tails the live stream.

use std::path::PathBuf;
use std::sync::Arc;

use hyprlayer_core::claude::{ClaudeDriver, ExitOutcome, SessionEvent, SpawnConfig};
use hyprlayer_core::pipeline::{self, Stage};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::{Db, NewSession, Session, StoredEvent};
use tauri::ipc::Channel;
use tauri::State;
use tauri_specta::Event;
use tokio::sync::broadcast;

use super::{db_task, effective_for};
use crate::error::{AppError, CmdResult};
use crate::events::SessionLifecycle;
use crate::session_registry::SessionRegistry;
use crate::state::AppState;

fn session_not_found(id: &str) -> AppError {
    AppError {
        code: "session_not_found".into(),
        message: format!("No live session with id {id}"),
        remedy: Some("The session may have ended; start a new one.".into()),
    }
}

/// Start a session for a workstream stage. Composes the stage skill prompt,
/// spawns the driver under the concurrency cap, and streams events back over
/// `on_event`. Returns the persisted [`Session`] row.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn start_stage_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workstream_id: String,
    stage: String,
    input: String,
    attached_docs: Vec<String>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let ws = {
        let id = workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .ok_or_else(|| AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {workstream_id}"),
        remedy: None,
    })?;

    // Reserve a slot up front: refuse rather than pile on concurrent agents.
    let permit = state.sessions.try_acquire().ok_or_else(|| AppError {
        code: "too_many_sessions".into(),
        message: "The maximum number of concurrent agent sessions is already running.".into(),
        remedy: Some("Wait for a session to finish, or stop one, then try again.".into()),
    })?;

    let claude = state
        .binaries
        .require(BinaryKind::Claude)
        .await
        .map_err(AppError::from)?;

    // Grant read access to the thoughts root (outside the worktree) so skills
    // can write research/plan docs through the symlinks.
    let effective = effective_for(&state, &ws.project_path).await?;
    let add_dirs: Vec<PathBuf> = effective.backend.thoughts_root().into_iter().collect();

    let stage_kind = Stage::parse(&stage);
    let policy = pipeline::policy_for(stage_kind);
    let prompt = pipeline::compose_prompt(stage_kind, &input, &attached_docs);

    // Persist the session row before spawning, so a crash before init still
    // leaves a record.
    let session_id = uuid::Uuid::new_v4().to_string();
    let session = {
        let new = NewSession {
            id: session_id.clone(),
            workstream_id: ws.id.clone(),
            stage: stage_kind.as_str().to_string(),
            cwd: ws.worktree_path.clone(),
        };
        db_task(state.db.clone(), move |db| db.insert_session(new)).await?
    };

    // The child inherits our env; we only override PATH with the resolver's
    // augmented PATH (login-shell + well-known dirs) so it finds git/gh/etc.
    let path = state.binaries.augmented_path().await;
    let env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];

    let (driver, rx) = ClaudeDriver::spawn(SpawnConfig {
        claude_bin: claude,
        cwd: PathBuf::from(&ws.worktree_path),
        session_id: session_id.clone(),
        permission_mode: policy.permission_mode,
        allowed_tools: policy.allowed_tools,
        disallowed_tools: policy.disallowed_tools,
        add_dirs,
        resume: None,
        fork: false,
        max_budget_usd: None,
        model: None,
        env,
    })
    .map_err(AppError::from)?;

    if let Some(pid) = driver.pid() {
        let sid = session_id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.record_orphan(pid as i64, &sid)
        })
        .await;
    }

    let driver = Arc::new(driver);
    state
        .sessions
        .insert(session_id.clone(), driver.clone())
        .await;

    spawn_forwarder(ForwarderCtx {
        app,
        db: state.db.clone(),
        registry: state.sessions.clone(),
        channel: on_event,
        session_id: session_id.clone(),
        workstream_id: ws.id.clone(),
        rx,
        permit,
    });

    // Kick off the first turn (the composed skill prompt).
    driver.send_user_turn(&prompt).map_err(AppError::from)?;

    Ok(session)
}

#[tauri::command]
#[specta::specta]
pub async fn send_user_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> CmdResult<()> {
    let driver = state
        .sessions
        .get(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?;
    driver.send_user_turn(&text).map_err(AppError::from)
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
    let driver = state
        .sessions
        .get(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?;
    driver
        .respond_permission(&request_id, allow, message.as_deref())
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn interrupt_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    let driver = state
        .sessions
        .get(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?;
    driver.interrupt().map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn kill_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    let driver = state
        .sessions
        .get(&session_id)
        .await
        .ok_or_else(|| session_not_found(&session_id))?;
    driver.kill();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn list_sessions(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<Vec<Session>> {
    db_task(state.db.clone(), move |db| db.list_sessions(&workstream_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_session_log(
    state: State<'_, AppState>,
    session_id: String,
) -> CmdResult<Vec<StoredEvent>> {
    db_task(state.db.clone(), move |db| db.session_events(&session_id)).await
}

/// Replay a session's persisted transcript into a fresh Channel, then tail the
/// live stream if it's still running. Lets the UI re-open a session it
/// navigated away from without losing history.
#[tauri::command]
#[specta::specta]
pub async fn attach_session(
    state: State<'_, AppState>,
    session_id: String,
    on_event: Channel<SessionEvent>,
) -> CmdResult<()> {
    let id = session_id.clone();
    let events = db_task(state.db.clone(), move |db| db.session_events(&id)).await?;
    for stored in events {
        if let Ok(event) = serde_json::from_str::<SessionEvent>(&stored.payload) {
            let _ = on_event.send(event);
        }
    }

    // Still live? Tail it (forward-only — the original forwarder owns
    // persistence and the concurrency permit).
    if let Some(driver) = state.sessions.get(&session_id).await {
        let mut rx = driver.subscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let terminal = matches!(event, SessionEvent::Exited { .. });
                        if on_event.send(event).is_err() || terminal {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
    Ok(())
}

/* ------------------------------ Forwarder ------------------------------- */

struct ForwarderCtx {
    app: tauri::AppHandle,
    db: Arc<Db>,
    registry: Arc<SessionRegistry>,
    channel: Channel<SessionEvent>,
    session_id: String,
    workstream_id: String,
    rx: broadcast::Receiver<SessionEvent>,
    permit: tokio::sync::OwnedSemaphorePermit,
}

fn spawn_forwarder(ctx: ForwarderCtx) {
    let ForwarderCtx {
        app,
        db,
        registry,
        channel,
        session_id,
        workstream_id,
        mut rx,
        permit,
    } = ctx;

    tauri::async_runtime::spawn(async move {
        let _permit = permit; // released when the session ends (loop break)
        let mut seq: i64 = 0;

        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("session {session_id} forwarder lagged {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };

            // Persist for replay.
            if let Ok(payload) = serde_json::to_string(&event) {
                let _ = db.append_session_event(&session_id, seq, event_kind(&event), &payload);
                seq += 1;
            }

            // Advance the session row + emit coarse lifecycle.
            let mut terminal = false;
            match &event {
                SessionEvent::Init {
                    claude_session_id,
                    model,
                } => {
                    let _ = db.mark_session_init(&session_id, claude_session_id, model.as_deref());
                    emit_lifecycle(&app, &session_id, &workstream_id, "running");
                }
                SessionEvent::PermissionAsk { .. } => {
                    let _ = db.set_session_state(&session_id, "awaiting_permission");
                    emit_lifecycle(&app, &session_id, &workstream_id, "awaiting_permission");
                }
                SessionEvent::TurnDone { cost_usd, .. } => {
                    if let Some(cost) = cost_usd {
                        let _ = db.set_session_cost(&session_id, *cost);
                    }
                    let _ = db.set_session_state(&session_id, "running");
                }
                SessionEvent::Exited {
                    outcome,
                    stderr_tail,
                    ..
                } => {
                    let state = outcome_state(*outcome);
                    let summary = (!stderr_tail.is_empty()).then_some(stderr_tail.as_str());
                    let _ = db.finish_session(&session_id, state, summary);
                    let _ = db.clear_orphans_for(&session_id);
                    registry.remove(&session_id).await;
                    emit_lifecycle(&app, &session_id, &workstream_id, state);
                    terminal = true;
                }
                _ => {}
            }

            let _ = channel.send(event);
            if terminal {
                break;
            }
        }
    });
}

fn emit_lifecycle(app: &tauri::AppHandle, session_id: &str, workstream_id: &str, state: &str) {
    let _ = SessionLifecycle {
        session_id: session_id.to_string(),
        workstream_id: workstream_id.to_string(),
        state: state.to_string(),
    }
    .emit(app);
}

/// The stored `kind` tag, matching the serde tag on [`SessionEvent`].
fn event_kind(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::Init { .. } => "init",
        SessionEvent::PartialText { .. } => "partialText",
        SessionEvent::AssistantText { .. } => "assistantText",
        SessionEvent::Thinking { .. } => "thinking",
        SessionEvent::ToolUse { .. } => "toolUse",
        SessionEvent::ToolResult { .. } => "toolResult",
        SessionEvent::PermissionAsk { .. } => "permissionAsk",
        SessionEvent::TurnDone { .. } => "turnDone",
        SessionEvent::ProtocolError { .. } => "protocolError",
        SessionEvent::Exited { .. } => "exited",
    }
}

fn outcome_state(outcome: ExitOutcome) -> &'static str {
    match outcome {
        ExitOutcome::Completed => "completed",
        ExitOutcome::Interrupted => "interrupted",
        ExitOutcome::Crashed => "crashed",
        ExitOutcome::Killed => "killed",
        ExitOutcome::SpawnFailed => "spawn_failed",
    }
}
