//! Agent-session IPC: start a stage session, drive it (turns, permissions,
//! interrupt, kill), and attach/replay its transcript.
//!
//! Each live session has one **forwarder task** that owns the broadcast
//! subscription: it persists every event to `session_events`, advances the
//! `sessions` row state, and pushes events onto the per-session Tauri Channel.
//! Re-attaching a UI replays the persisted log then tails the live stream.

use std::path::PathBuf;
use std::sync::Arc;

use hyprlayer_core::claude::{ClaudeDriver, PermissionMode, SessionEvent, SpawnConfig};
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

/// Kill a live session (if any) and wait — bounded — until its forwarder
/// observes the exit and removes it from the registry. Shared by
/// [`restart_session`] (so a relaunch can't race the still-dying process) and
/// [`crate::commands::workstreams::archive_workstream`] (so archiving tears the
/// session down instead of leaking a process + driver against a deleted worktree).
pub(crate) async fn kill_and_await_exit(state: &AppState, session_id: &str) {
    if let Some(driver) = state.sessions.get(session_id).await {
        driver.kill();
        let mut waited_ms = 0u64;
        while state.sessions.get(session_id).await.is_some() && waited_ms < 10_000 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            waited_ms += 50;
        }
    }
}

/// The live driver for a session, or `session_not_found`. The single lookup the
/// turn/permission/interrupt/kill commands share instead of each poking
/// `state.sessions` directly.
async fn live_driver(state: &AppState, session_id: &str) -> CmdResult<Arc<ClaudeDriver>> {
    state
        .sessions
        .get(session_id)
        .await
        .ok_or_else(|| session_not_found(session_id))
}

/// Everything `launch_session` needs that isn't shared. `resume` toggles
/// `--resume` (continue claude's conversation) vs a fresh `--session-id`, and
/// `start_seq` is where the persisted log should continue (0 fresh, else the
/// existing event count so a resume appends rather than collides).
struct LaunchParams {
    app: tauri::AppHandle,
    session: Session,
    project_path: String,
    model: Option<String>,
    effort: Option<String>,
    /// User-chosen `--permission-mode` override for this spawn, or `None` to use
    /// the stage's default posture (see [`pipeline::policy_for`]).
    permission_mode: Option<PermissionMode>,
    resume: bool,
    start_seq: i64,
    channel: Channel<SessionEvent>,
}

/// Spawn the claude driver for a session and wire up its forwarder — the launch
/// pipeline shared by [`start_stage_session`] and [`resume_session`].
async fn launch_session(state: &AppState, params: LaunchParams) -> CmdResult<Arc<ClaudeDriver>> {
    let LaunchParams {
        app,
        session,
        project_path,
        model,
        effort,
        permission_mode,
        resume,
        start_seq,
        channel,
    } = params;

    let claude = state
        .binaries
        .require(BinaryKind::Claude)
        .await
        .map_err(AppError::from)?;

    // Grant read access to the thoughts root (outside the worktree) so skills
    // can write research/plan docs through the symlinks.
    let effective = effective_for(state, &project_path).await?;
    let add_dirs: Vec<PathBuf> = effective.backend.thoughts_root().into_iter().collect();
    let policy = pipeline::policy_for(Stage::parse(&session.stage));
    // The stage sets a default permission posture; an explicit user choice in the
    // composer (e.g. "Bypass permissions") overrides just the mode, keeping the
    // stage's allow/deny tool lists as guardrails.
    let permission_mode = permission_mode.unwrap_or(policy.permission_mode);

    // The child inherits our env; we only override PATH with the resolver's
    // augmented PATH (login-shell + well-known dirs) so it finds git/gh/etc.
    let path = state.binaries.augmented_path().await;
    let env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];

    let (driver, rx) = ClaudeDriver::spawn(SpawnConfig {
        claude_bin: claude,
        cwd: PathBuf::from(&session.cwd),
        session_id: session.id.clone(),
        permission_mode,
        allowed_tools: policy.allowed_tools,
        disallowed_tools: policy.disallowed_tools,
        add_dirs,
        // claude persisted the conversation under the id we passed as
        // `--session-id`, so resume by that same id.
        resume: resume.then(|| session.id.clone()),
        fork: false,
        max_budget_usd: None,
        model,
        effort,
        env,
    })
    .map_err(AppError::from)?;

    if let Some(pid) = driver.pid() {
        let sid = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.record_orphan(pid as i64, &sid)
        })
        .await;
    }

    let driver = Arc::new(driver);
    state
        .sessions
        .insert(session.id.clone(), driver.clone())
        .await;

    spawn_forwarder(ForwarderCtx {
        app,
        db: state.db.clone(),
        registry: state.sessions.clone(),
        channel,
        session_id: session.id.clone(),
        workstream_id: session.workstream_id.clone(),
        rx,
        start_seq,
    });

    Ok(driver)
}

/// Start a session for a workstream stage. Composes the stage skill prompt,
/// spawns the driver, and streams events back over `on_event`. Returns the
/// persisted [`Session`] row.
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
    model: Option<String>,
    effort: Option<String>,
    // Optional `--permission-mode` override (`default` | `acceptEdits` | `plan` |
    // `bypassPermissions`); `None`/unrecognized keeps the stage's default posture.
    permission_mode: Option<String>,
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

    let stage_kind = Stage::parse(&stage);
    let prompt = pipeline::compose_prompt(stage_kind, &input, &attached_docs);

    // Persist the session row before spawning, so a crash before init still
    // leaves a record.
    let session = {
        let new = NewSession {
            id: uuid::Uuid::new_v4().to_string(),
            workstream_id: ws.id.clone(),
            stage: stage_kind.as_str().to_string(),
            cwd: ws.worktree_path.clone(),
        };
        db_task(state.db.clone(), move |db| db.insert_session(new)).await?
    };

    let driver = launch_session(
        &state,
        LaunchParams {
            app,
            session: session.clone(),
            project_path: ws.project_path,
            model,
            effort,
            permission_mode: permission_mode
                .as_deref()
                .and_then(PermissionMode::from_flag),
            resume: false,
            start_seq: 0,
            channel: on_event,
        },
    )
    .await?;

    // Kick off the first turn (the composed skill prompt).
    driver
        .send_user_turn(&prompt, Some(&session.stage))
        .map_err(AppError::from)?;

    Ok(session)
}

/// Resume a previously-started session that's no longer live (e.g. after an app
/// restart) by re-spawning claude with `--resume`, so the conversation continues
/// instead of forcing a new session. The event log appends after its existing
/// tail. Returns the (existing) [`Session`] row.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn resume_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
    effort: Option<String>,
    // Optional `--permission-mode` override for the resumed launch. Unlike a live
    // session (which can't switch into `bypassPermissions`), a resume is a fresh
    // process, so any mode applies. `None`/unrecognized keeps the stage default.
    permission_mode: Option<String>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let session = {
        let id = session_id.clone();
        db_task(state.db.clone(), move |db| db.get_session(&id)).await?
    }
    .ok_or_else(|| session_not_found(&session_id))?;

    // Already live (a race, an HMR/UI reload that dropped the frontend's channel
    // while the backend process survived, or a session that never actually
    // died): don't re-spawn — re-wire the caller's new channel to the running
    // event stream so events flow again. Without this the new channel gets
    // nothing and the UI is pinned on "Connecting to claude…" forever even
    // though the turn runs fine.
    if let Some(driver) = state.sessions.get(&session_id).await {
        tail_live(driver, on_event);
        return Ok(session);
    }

    let ws = {
        let id = session.workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .ok_or_else(|| AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {}", session.workstream_id),
        remedy: None,
    })?;

    // Continue the persisted log after its existing tail, and re-mark the row
    // running while the resumed process spins back up.
    let start_seq = {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| db.session_events(&id))
            .await?
            .len() as i64
    };
    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.set_session_state(&id, "running")
        })
        .await;
    }

    launch_session(
        &state,
        LaunchParams {
            app,
            session: session.clone(),
            project_path: ws.project_path,
            model,
            effort,
            // A resume CAN carry a permission override (it's a fresh process):
            // this is how the UI restarts a live session into a new mode (e.g.
            // bypass, which a running session can't switch into). `None` keeps
            // the stage default.
            permission_mode: permission_mode
                .as_deref()
                .and_then(PermissionMode::from_flag),
            resume: true,
            start_seq,
            channel: on_event,
        },
    )
    .await?;

    Ok(session)
}

/// Restart a *live* session into a new permission mode. Claude can't switch a
/// running session into `bypassPermissions` (it's a launch-time decision), so
/// the UI applies a mode change by relaunching the process. We do it atomically
/// on the backend — kill the live driver, wait for it to actually exit (the
/// forwarder drops it from the registry on its `Exited`), then re-spawn with
/// `--resume` and the new mode — so the frontend can't race the resume onto the
/// still-dying old process (which would keep the old mode). Conversation is
/// preserved; the event log appends after its tail.
#[tauri::command]
#[specta::specta]
pub async fn restart_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    permission_mode: Option<String>,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let session = {
        let id = session_id.clone();
        db_task(state.db.clone(), move |db| db.get_session(&id)).await?
    }
    .ok_or_else(|| session_not_found(&session_id))?;

    // Kill the live process (if any) and wait until the forwarder removes it from
    // the registry — i.e. it has truly exited — before we re-spawn, so the
    // frontend can't race the resume onto the still-dying old process.
    kill_and_await_exit(&state, &session_id).await;

    let ws = {
        let id = session.workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .ok_or_else(|| AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {}", session.workstream_id),
        remedy: None,
    })?;

    let start_seq = {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| db.session_events(&id))
            .await?
            .len() as i64
    };
    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.set_session_state(&id, "running")
        })
        .await;
    }

    launch_session(
        &state,
        LaunchParams {
            app,
            session: session.clone(),
            project_path: ws.project_path,
            model: None,
            effort: None,
            permission_mode: permission_mode
                .as_deref()
                .and_then(PermissionMode::from_flag),
            resume: true,
            start_seq,
            channel: on_event,
        },
    )
    .await?;

    Ok(session)
}

#[tauri::command]
#[specta::specta]
pub async fn send_user_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
    attached_docs: Vec<String>,
) -> CmdResult<()> {
    // Compose the doc list server-side (same `Relevant documents:` format as
    // session start) so the wire format lives only in `pipeline`, not in the
    // frontend. The optimistic bubble still shows just `text` + pin cards.
    let prompt = pipeline::append_docs(text, &attached_docs);
    live_driver(&state, &session_id)
        .await?
        .send_user_turn(&prompt, None)
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
    live_driver(&state, &session_id)
        .await?
        .respond_permission(&request_id, allow, message.as_deref())
        .map_err(AppError::from)
}

/// Answer an `AskUserQuestion` prompt. `answers` is a JSON object mapping each
/// question's text to the chosen answer string (multi-select choices joined by
/// ", "); an empty object means the user dismissed without answering.
#[tauri::command]
#[specta::specta]
pub async fn answer_question(
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    answers: String,
) -> CmdResult<()> {
    let answers = serde_json::from_str::<serde_json::Value>(&answers)
        .ok()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    live_driver(&state, &session_id)
        .await?
        .answer_question(&request_id, answers)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn interrupt_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    live_driver(&state, &session_id)
        .await?
        .interrupt()
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn kill_session(state: State<'_, AppState>, session_id: String) -> CmdResult<()> {
    live_driver(&state, &session_id).await?.kill();
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
    // persistence and registry bookkeeping).
    if let Some(driver) = state.sessions.get(&session_id).await {
        tail_live(driver, on_event);
    }
    Ok(())
}

/// Forward a live session's broadcast events onto a fresh Channel, forward-only:
/// the owning forwarder still handles persistence, state, and registry
/// bookkeeping. Stops at the first terminal event or once the UI channel closes.
/// Shared by [`attach_session`] (after replaying the log) and [`resume_session`]
/// when the backend session is still live — e.g. an HMR/UI reload dropped the
/// frontend's channel but the claude process survived, so there's nothing to
/// re-spawn; we just re-wire the new channel to the running stream.
fn tail_live(driver: Arc<ClaudeDriver>, on_event: Channel<SessionEvent>) {
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

/* ------------------------------ Forwarder ------------------------------- */

struct ForwarderCtx {
    app: tauri::AppHandle,
    db: Arc<Db>,
    registry: Arc<SessionRegistry>,
    channel: Channel<SessionEvent>,
    session_id: String,
    workstream_id: String,
    rx: broadcast::Receiver<SessionEvent>,
    /// First persisted-event sequence number — 0 for a fresh session, or the
    /// existing event count when resuming (so the log appends, not collides).
    start_seq: i64,
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
        start_seq,
    } = ctx;

    tauri::async_runtime::spawn(async move {
        let mut seq: i64 = start_seq;

        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("session {session_id} forwarder lagged {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };

            // Persist for replay — usage pings included (tiny), so the context
            // meter survives a restart even if the turn was interrupted before
            // its result event ever landed.
            if let Ok(payload) = serde_json::to_string(&event) {
                let _ = db.append_session_event(&session_id, seq, event.kind(), &payload);
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
                SessionEvent::QuestionAsk { .. } => {
                    let _ = db.set_session_state(&session_id, "awaiting_question");
                    emit_lifecycle(&app, &session_id, &workstream_id, "awaiting_question");
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
                    let state = outcome.state();
                    let summary = (!stderr_tail.is_empty()).then_some(stderr_tail.as_str());
                    let _ = db.finish_session(&session_id, state, summary);
                    let _ = db.clear_orphans_for(&session_id);
                    registry.remove(&session_id).await;
                    emit_lifecycle(&app, &session_id, &workstream_id, state);
                    terminal = true;
                }
                _ => {}
            }

            // `UserMessage` is persist-only: the UI already showed it optimistically
            // when the user sent it, so forwarding it would double-stamp the turn.
            // Every other event drives the live transcript.
            if !matches!(event, SessionEvent::UserMessage { .. }) {
                let _ = channel.send(event);
            }
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
