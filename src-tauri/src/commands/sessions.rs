//! Agent-session IPC: start a stage session, drive it (turns, permissions,
//! interrupt, kill), and attach/replay its transcript.
//!
//! Each live session has one **forwarder task** that owns the broadcast
//! subscription: it persists every event to `session_events`, advances the
//! `sessions` row state, and pushes events onto the per-session Tauri Channel.
//! Re-attaching a UI replays the persisted log then tails the live stream.

use std::path::PathBuf;
use std::sync::Arc;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::{ClaudeDriver, PermissionMode, SessionEvent, SpawnConfig};
use hyprlayer_core::pipeline::{self, Stage};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::{Db, NewSession, Session, StoredEvent, Workstream};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::Manager;
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
async fn live_driver(state: &AppState, session_id: &str) -> CmdResult<Arc<dyn AgentDriver>> {
    state
        .sessions
        .get(session_id)
        .await
        .ok_or_else(|| session_not_found(session_id))
}

/// Fetch a persisted session by id, or fail with the standard not-found error —
/// the DB twin of [`live_driver`], shared by the commands that mutate a session.
async fn load_session(state: &AppState, id: &str) -> CmdResult<Session> {
    let owned = id.to_string();
    db_task(state.db.clone(), move |db| db.get_session(&owned))
        .await?
        .ok_or_else(|| session_not_found(id))
}

/// Fetch a workstream by id, or fail.
async fn load_workstream(state: &AppState, id: &str) -> CmdResult<Workstream> {
    let owned = id.to_string();
    db_task(state.db.clone(), move |db| db.get_workstream(&owned))
        .await?
        .ok_or_else(|| AppError {
            code: "workstream_not_found".into(),
            message: format!("No workstream with id {id}"),
            remedy: None,
        })
}

/// Composer choices for a new session, bundled so the command stays within
/// specta's argument-count limit.
#[derive(Debug, Default, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct StartOpts {
    /// Which harness to run: `"claude"` (default) or `"opencode"`.
    pub harness: Option<String>,
    /// For Claude: a model key/flag. For OpenCode: a `"provider/model"` id.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `--permission-mode` override (`default` | `acceptEdits` | `plan` |
    /// `bypassPermissions`); `None`/unrecognized keeps the stage's default.
    pub permission_mode: Option<String>,
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
async fn launch_session(state: &AppState, params: LaunchParams) -> CmdResult<Arc<dyn AgentDriver>> {
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

    // The `--permission-mode` the process is actually launched with, recorded in
    // the registry so a later `resume_session` can tell a stale re-attach from a
    // real no-change resume. `None` for OpenCode (no permission modes).
    let mut launched_mode: Option<PermissionMode> = None;
    let (driver, rx): (Arc<dyn AgentDriver>, broadcast::Receiver<SessionEvent>) =
        if session.harness == "opencode" {
            // OpenCode: drive a session on the shared `opencode serve`. Tools and
            // permissions are OpenCode's own concern, so the stage policy /
            // permission-mode / add-dirs don't apply here; `model` is a
            // `"provider/model"` id chosen in the composer.
            let bin = state
                .binaries
                .require(BinaryKind::OpenCode)
                .await
                .map_err(AppError::from)?;
            let path = state.binaries.augmented_path().await;
            // App-owned OpenCode data home, so its session DB is ours (never the
            // user's possibly-incompatible global ~/.local/share/opencode).
            let data_dir = app
                .path()
                .app_data_dir()
                .map(|d| d.join("opencode"))
                .map_err(|e| AppError::internal(format!("resolve app data dir: {e}")))?;
            let server = state
                .opencode
                .ensure(bin, path, data_dir)
                .await
                .map_err(AppError::internal)?;
            let (driver, rx) = crate::opencode::OpenCodeDriver::spawn(server, &session.cwd, model)
                .await
                .map_err(AppError::internal)?;
            let driver: Arc<dyn AgentDriver> = driver;
            (driver, rx)
        } else {
            let claude = state
                .binaries
                .require(BinaryKind::Claude)
                .await
                .map_err(AppError::from)?;

            // Grant read access to the thoughts root (outside the worktree) so
            // skills can write research/plan docs through the symlinks.
            let effective = effective_for(state, &project_path).await?;
            let add_dirs: Vec<PathBuf> = effective.backend.thoughts_root().into_iter().collect();
            let policy = pipeline::policy_for(Stage::parse(&session.stage));
            // The stage sets a default permission posture; an explicit user choice
            // in the composer (e.g. "Bypass permissions") overrides just the mode,
            // keeping the stage's allow/deny tool lists as guardrails.
            let permission_mode = permission_mode.unwrap_or(policy.permission_mode);
            launched_mode = Some(permission_mode);

            // The child inherits our env; we override PATH with the resolver's
            // augmented PATH so it finds git/gh/etc, and — for enterprise "API
            // key" auth — inject the Anthropic credentials/routing so the spawned
            // `claude` uses them instead of the user's logged-in session.
            let path = state.binaries.augmented_path().await;
            let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
            let claude_cfg = state.settings.get().await.providers.claude;
            env.extend(super::claude_auth_env(&claude_cfg));

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
                chrome: claude_cfg.chrome_enabled,
                env,
            })
            .map_err(AppError::from)?;
            let driver: Arc<dyn AgentDriver> = Arc::new(driver);
            (driver, rx)
        };

    if let Some(pid) = driver.pid() {
        let sid = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.record_orphan(pid as i64, &sid)
        })
        .await;
    }

    state
        .sessions
        .insert(session.id.clone(), driver.clone(), launched_mode)
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
    opts: StartOpts,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let StartOpts {
        harness,
        model,
        effort,
        permission_mode,
    } = opts;
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
    // Ship names the workstream's target branch in its directive; other stages
    // ignore it. So a re-targeted workstream opens its PR against e.g. `develop`.
    let prompt = pipeline::compose_prompt(stage_kind, &input, &ws.base_branch, &attached_docs);

    // Persist the session row before spawning, so a crash before init still
    // leaves a record.
    let session = {
        let new = NewSession {
            id: uuid::Uuid::new_v4().to_string(),
            workstream_id: ws.id.clone(),
            stage: stage_kind.as_str().to_string(),
            harness: match harness.as_deref() {
                Some("opencode") => "opencode".to_string(),
                _ => "claude".to_string(),
            },
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
    // while the backend process survived, or a session that never actually died).
    // Normally we don't re-spawn — we just re-wire the caller's new channel to the
    // running event stream so events flow again (without this the new channel gets
    // nothing and the UI is pinned on "Connecting to claude…" forever even though
    // the turn runs fine).
    //
    // BUT the reload may have outlived a permission-mode change: the tab now wants
    // (say) bypass, while the survivor process still runs the mode it launched
    // with. claude binds its mode at launch, so a silent re-attach would leave the
    // UI badge advertising a mode the process isn't in — and because the frontend
    // then believes the entry already holds the new mode, it never relaunches to
    // fix it (the picker no-ops on "same mode"). So when the requested mode is
    // resolvable AND differs from what's actually running, fall through to the
    // relaunch path below (kill + `--resume` into the new mode, conversation
    // preserved). Otherwise just re-wire.
    if let Some(driver) = state.sessions.get(&session_id).await {
        let requested = permission_mode.as_deref().and_then(PermissionMode::from_flag);
        let running = state.sessions.permission_mode(&session_id).await;
        if requested.is_none() || requested == running {
            tail_live(driver, on_event);
            return Ok(session);
        }
        // Drop the stale-mode process; the resume path below relaunches it with
        // the requested mode. Wait for it to actually exit first so the relaunch
        // can't race the still-dying old process.
        kill_and_await_exit(&state, &session_id).await;
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

/// Rewind a session to the N-th (1-based) user turn, discarding everything
/// after it: kill the live driver, delete persisted events at the cutoff seq
/// onward, then re-spawn with `--resume` so the same conversation can continue
/// — claude's underlying history is unchanged (the SDK has no "rewind"
/// control), but our UI and the persisted log match the truncated state, and
/// the user can re-prompt from that point. `user_turn` is the 1-based ordinal
/// of the clicked user bubble (i.e. the N-th `userMessage` event). Returns
/// the existing [`Session`].
#[tauri::command]
#[specta::specta]
pub async fn rewind_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    user_turn: u32,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let session = load_session(&state, &session_id).await?;

    // Resolve the cutoff (the seq of the N-th userMessage). Everything at or
    // after this seq is dropped — the target user turn is removed, so the
    // composer can re-send a (possibly edited) version of it.
    let cutoff = {
        let id = session.id.clone();
        let n = user_turn as usize;
        db_task(state.db.clone(), move |db| db.nth_user_message_seq(&id, n)).await?
    }
    .ok_or_else(|| AppError {
        code: "user_turn_not_found".into(),
        message: format!("session {session_id} has no user turn #{user_turn}"),
        remedy: None,
    })?;

    // Resolve the workstream before any destructive step, so a missing-row error
    // can't strand us with the log already cut.
    let ws = load_workstream(&state, &session.workstream_id).await?;

    // Take the live process down before mutating the log so its forwarder can't
    // race-append events past the truncation point and re-emerge a turn we
    // dropped.
    kill_and_await_exit(&state, &session_id).await;

    // Cut the tail, but keep the removed rows in hand: if the re-spawn below
    // fails, a launch error must not cost the conversation. We restore them and
    // surface the error rather than leaving a truncated, process-less session.
    let removed = {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| {
            db.cut_session_events_from(&id, cutoff)
        })
        .await?
    };

    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.set_session_state(&id, "running")
        })
        .await;
    }

    let launched = launch_session(
        &state,
        LaunchParams {
            app,
            session: session.clone(),
            project_path: ws.project_path,
            model: None,
            effort: None,
            permission_mode: None,
            resume: true,
            start_seq: cutoff,
            channel: on_event,
        },
    )
    .await;

    if let Err(e) = launched {
        // Roll the cut back so the session is recoverable via a normal resume.
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.restore_session_events(&id, &removed)?;
            db.set_session_state(&id, "idle")
        })
        .await;
        return Err(e);
    }

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

/// Switch a live session's model so its next turn uses it (OpenCode sends a model
/// per prompt, so it can change mid-session). A no-op when the session isn't live
/// — the tab's selection still rides into a fresh session at launch — and for
/// Claude, whose model is fixed at launch (its driver ignores this).
#[tauri::command]
#[specta::specta]
pub async fn set_session_model(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
) -> CmdResult<()> {
    if let Some(driver) = state.sessions.get(&session_id).await {
        driver.set_model(model);
    }
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
fn tail_live(driver: Arc<dyn AgentDriver>, on_event: Channel<SessionEvent>) {
    let mut rx = driver.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let terminal = matches!(event, SessionEvent::Exited { .. });
                    // `UserMessage` is persist-only — the live UI already showed it
                    // optimistically when the user sent it, and the replayed log
                    // carries the earlier ones. Forwarding it here (on re-attach to
                    // a still-live session) would double-stamp the next turn, so we
                    // skip it exactly as the owning forwarder does.
                    if !matches!(event, SessionEvent::UserMessage { .. })
                        && on_event.send(event).is_err()
                    {
                        break;
                    }
                    if terminal {
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

/// What one session event writes to the `sessions` row, captured as owned data
/// so the write can run on the blocking pool instead of inline on the async
/// runtime. Mirrors the per-variant updates the forwarder used to do directly.
enum RowWrite {
    None,
    Init {
        claude_session_id: String,
        model: Option<String>,
    },
    State(&'static str),
    TurnDone {
        cost: Option<f64>,
    },
    Exited {
        state: &'static str,
        summary: Option<String>,
    },
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

        // A relaunch (resume/restart/rewind) advances `start_seq` past the prior
        // generation's events; emit the `Relaunched` boundary before the new
        // process's stream (see the event's doc). Skipped on a fresh start.
        if start_seq > 0 {
            let event = SessionEvent::Relaunched {};
            let kind = event.kind();
            if let Ok(payload) = serde_json::to_string(&event) {
                let _ = channel.send(event);
                let cur_seq = seq;
                let db = db.clone();
                let sid = session_id.clone();
                let persisted = tauri::async_runtime::spawn_blocking(move || {
                    db.append_session_event(&sid, cur_seq, kind, &payload).is_ok()
                })
                .await
                .unwrap_or(false);
                if persisted {
                    seq += 1;
                }
            }
        }

        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("session {session_id} forwarder lagged {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };

            // `UserMessage` is persist-only: the UI already showed it optimistically
            // when the user sent it, so forwarding it would double-stamp the turn.
            let is_user_message = matches!(event, SessionEvent::UserMessage { .. });

            // `PartialText` is the streaming live-typing delta; it is ephemeral —
            // the coalesced `AssistantText` block is the durable record — so it is
            // never persisted, only forwarded. Skipping it removes a per-token
            // write storm. Usage pings stay persisted (tiny) so the context meter
            // survives a restart even when a turn was interrupted before its
            // result event landed.
            let payload = (!matches!(event, SessionEvent::PartialText { .. }))
                .then(|| serde_json::to_string(&event).ok())
                .flatten();
            let kind = event.kind();

            // Extract the session-row update (owned) and the coarse lifecycle to
            // emit, so the actual writes can run on the blocking pool.
            let (row_write, lifecycle, terminal) = match &event {
                SessionEvent::Init {
                    claude_session_id,
                    model,
                } => (
                    RowWrite::Init {
                        claude_session_id: claude_session_id.clone(),
                        model: model.clone(),
                    },
                    Some("running"),
                    false,
                ),
                SessionEvent::PermissionAsk { .. } => (
                    RowWrite::State("awaiting_permission"),
                    Some("awaiting_permission"),
                    false,
                ),
                SessionEvent::QuestionAsk { .. } => (
                    RowWrite::State("awaiting_question"),
                    Some("awaiting_question"),
                    false,
                ),
                SessionEvent::TurnDone { cost_usd, .. } => {
                    (RowWrite::TurnDone { cost: *cost_usd }, None, false)
                }
                SessionEvent::Exited {
                    outcome,
                    stderr_tail,
                    ..
                } => {
                    let state = outcome.state();
                    let summary = (!stderr_tail.is_empty()).then(|| stderr_tail.clone());
                    (RowWrite::Exited { state, summary }, Some(state), true)
                }
                _ => (RowWrite::None, None, false),
            };

            // Forward to the live UI first, so streaming latency never waits on a
            // DB write. (PartialText takes this path and then skips the DB hop.)
            if !is_user_message {
                let _ = channel.send(event);
            }

            // Persist + advance the session row off the async runtime — the core
            // store is blocking and the rest of the app already wraps it in
            // spawn_blocking; the forwarder was the one hot path still writing
            // inline. Awaited (not fire-and-forget) so appends keep their seq
            // order. Skipped entirely when the event neither persists nor mutates
            // the row (e.g. PartialText), to avoid a pool round-trip per token.
            let has_db_work = payload.is_some() || !matches!(row_write, RowWrite::None);
            if has_db_work {
                let cur_seq = seq;
                let db = db.clone();
                let sid = session_id.clone();
                let persisted = tauri::async_runtime::spawn_blocking(move || {
                    let persisted = payload.is_some();
                    if let Some(payload) = payload {
                        let _ = db.append_session_event(&sid, cur_seq, kind, &payload);
                    }
                    match row_write {
                        RowWrite::Init {
                            claude_session_id,
                            model,
                        } => {
                            let _ =
                                db.mark_session_init(&sid, &claude_session_id, model.as_deref());
                        }
                        RowWrite::State(state) => {
                            let _ = db.set_session_state(&sid, state);
                        }
                        RowWrite::TurnDone { cost } => {
                            if let Some(cost) = cost {
                                let _ = db.set_session_cost(&sid, cost);
                            }
                            let _ = db.set_session_state(&sid, "running");
                        }
                        RowWrite::Exited { state, summary } => {
                            let _ = db.finish_session(&sid, state, summary.as_deref());
                            let _ = db.clear_orphans_for(&sid);
                        }
                        RowWrite::None => {}
                    }
                    persisted
                })
                .await
                .unwrap_or(false);
                if persisted {
                    seq += 1;
                }
            }

            // Drop the live driver from the registry on exit, then emit the coarse
            // lifecycle transition for the sidebar/toasts.
            if terminal {
                registry.remove(&session_id).await;
            }
            if let Some(state) = lifecycle {
                emit_lifecycle(&app, &session_id, &workstream_id, state);
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

/// View returned by [`list_active_sessions`]: one live session joined to its
/// workstream's display name and project path.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSessionView {
    pub session: Session,
    pub workstream_title: String,
    pub project_path: String,
}

/// All agents currently live in the registry (across every project), each
/// joined to its workstream's title + project path, newest first. Drives the
/// global search palette's "Agents" section, so even agents whose tab was
/// never opened are findable.
#[tauri::command]
#[specta::specta]
pub async fn list_active_sessions(state: State<'_, AppState>) -> CmdResult<Vec<ActiveSessionView>> {
    let ids = state.sessions.live_ids().await;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let db = state.db.clone();
    db_task(db, move |db| {
        let mut out = Vec::new();
        for id in ids {
            let Some(session) = db.get_session(&id)? else {
                continue;
            };
            let Some(ws) = db.get_workstream(&session.workstream_id)? else {
                continue;
            };
            out.push(ActiveSessionView {
                workstream_title: ws.title,
                project_path: ws.project_path,
                session,
            });
        }
        out.sort_by(|a, b| {
            b.session
                .started_at
                .partial_cmp(&a.session.started_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    })
    .await
}
