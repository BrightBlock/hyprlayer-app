//! Agent-session IPC: start a session, drive it (turns, permissions, interrupt,
//! kill), and attach/replay its transcript.
//!
//! Each live session has one forwarder task owning the broadcast subscription: it
//! persists events to `session_events`, advances the `sessions` row, and pushes onto
//! the per-session Channel. Re-attach replays the log then tails the live stream.

use std::path::PathBuf;
use std::sync::Arc;

use hyprlayer_core::agent::{AgentDriver, Harness};
use hyprlayer_core::claude::{
    ClaudeDriver, ExitOutcome, PermissionMode, SessionEvent, SessionState, SpawnConfig,
};
use hyprlayer_core::codex::{CodexDriver, SpawnConfig as CodexSpawnConfig};
use hyprlayer_core::pipeline::{self, Stage};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::{Db, NewSession, Session, StoredEvent, Workstream};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri::State;
use tauri_specta::Event;
use tokio::sync::{broadcast, oneshot};

use super::{db_task, effective_for};
use crate::error::{AppError, CmdResult};
use crate::events::SessionLifecycle;
use crate::session_registry::{ForwarderHandle, SessionRegistry};
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
///
/// Detached, not killed: both callers are the app reorganizing itself, not the
/// user ending the conversation, so the exit leaves no mark on the transcript.
pub(crate) async fn kill_and_await_exit(state: &AppState, session_id: &str) {
    if let Some(driver) = state.sessions.get(session_id).await {
        driver.kill_detached();
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
    /// Which harness to run; `None` defaults to Claude.
    pub harness: Option<Harness>,
    /// For Claude: a model key/flag. For OpenCode: a `"provider/model"` id.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `--permission-mode` override; `None` keeps the stage's default posture.
    pub permission_mode: Option<PermissionMode>,
    /// Name of the custom prompt that opened this session, when one did. Rides
    /// onto the persisted `UserMessage` so the transcript collapses the body
    /// into a card on replay.
    pub prompt_name: Option<String>,
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

    // The permission mode the process is actually launched with, recorded in the
    // registry so a later `resume_session` can tell a stale re-attach from a real
    // no-change resume. `None` for OpenCode, which has no permission modes;
    // Codex maps ours onto its approval-policy/sandbox pair, so it records one.
    let mut launched_mode: Option<PermissionMode> = None;
    let (driver, rx): (Arc<dyn AgentDriver>, broadcast::Receiver<SessionEvent>) =
        match session.harness {
            Harness::Opencode => {
                // OpenCode: drive a session on the shared `opencode serve`. Tools
                // and permissions are OpenCode's own concern, so the stage policy /
                // permission-mode / add-dirs don't apply here; `model` is a
                // `"provider/model"` id chosen in the composer.
                let bin = state
                    .binaries
                    .require(BinaryKind::OpenCode)
                    .await
                    .map_err(AppError::from)?;
                let path = state.binaries.augmented_path().await;
                // App-owned OpenCode data home, so its session DB is ours (never
                // the user's possibly-incompatible global ~/.local/share/opencode).
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
                let (driver, rx) =
                    crate::opencode::OpenCodeDriver::spawn(server, &session.cwd, model)
                        .await
                        .map_err(AppError::internal)?;
                let driver: Arc<dyn AgentDriver> = driver;
                (driver, rx)
            }
            Harness::Codex => {
                // Codex: one `codex app-server` child per session. Its approval
                // policy + sandbox are derived from our permission mode (see
                // `codex::protocol::approval_posture`); the stage policy's
                // allow/deny tool lists have no Codex analog, like OpenCode.
                let bin = state
                    .binaries
                    .require(BinaryKind::Codex)
                    .await
                    .map_err(AppError::from)?;

                let effective = effective_for(state, &project_path).await?;
                let add_dirs: Vec<PathBuf> = effective.backend.thoughts_root().into_iter().collect();
                let permission_mode =
                    permission_mode.unwrap_or(pipeline::policy_for(session.stage).permission_mode);
                launched_mode = Some(permission_mode);

                let path = state.binaries.augmented_path().await;
                let env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];

                // Codex resumes by the thread id IT minted (recorded on `Init`),
                // not by our session id the way Claude does. A resume with no
                // recorded thread id can only start a fresh one — the previous
                // handshake never completed — but the transcript still replays
                // from SQLite, so the UI looks continuous while the model has no
                // memory of it. Say so in the log rather than failing silently.
                let resume_thread = resume.then(|| session.claude_session_id.clone()).flatten();
                if resume && resume_thread.is_none() {
                    log::warn!(
                        "codex session {} has no recorded thread id; starting a new thread \
                         (the model will not have the replayed history)",
                        session.id
                    );
                }

                // Rewind ordinals count every user turn in the session log, so a
                // resumed driver has to start from that count rather than 0 —
                // otherwise `thread/rollback` drops too few turns and still
                // reports the model forgot them. Only the resume path needs it;
                // a fresh thread genuinely starts at zero.
                let prior_user_turns = if resume_thread.is_some() {
                    let sid = session.id.clone();
                    db_task(state.db.clone(), move |db| db.count_user_messages(&sid)).await?
                } else {
                    0
                };

                let (driver, rx) = CodexDriver::spawn(CodexSpawnConfig {
                    codex_bin: bin,
                    cwd: PathBuf::from(&session.cwd),
                    permission_mode,
                    add_dirs,
                    resume: resume_thread,
                    prior_user_turns,
                    model,
                    effort,
                    ephemeral: false,
                    env,
                })
                .await
                .map_err(AppError::from)?;
                let driver: Arc<dyn AgentDriver> = Arc::new(driver);
                (driver, rx)
            }
            Harness::Claude => {
                let claude = state
                    .binaries
                    .require(BinaryKind::Claude)
                    .await
                    .map_err(AppError::from)?;

                // Grant read access to the thoughts root (outside the worktree) so
                // skills can write research/plan docs through the symlinks.
                let effective = effective_for(state, &project_path).await?;
                let add_dirs: Vec<PathBuf> = effective.backend.thoughts_root().into_iter().collect();
                let policy = pipeline::policy_for(session.stage);
                // The stage sets a default permission posture; an explicit user
                // choice in the composer (e.g. "Bypass permissions") overrides just
                // the mode, keeping the stage's allow/deny tool lists as guardrails.
                let permission_mode = permission_mode.unwrap_or(policy.permission_mode);
                launched_mode = Some(permission_mode);

                // The child inherits our env; we override PATH with the resolver's
                // augmented PATH so it finds git/gh/etc, and — for enterprise "API
                // key" auth — inject the Anthropic credentials/routing so the
                // spawned `claude` uses them instead of the user's logged-in session.
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
            }
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

    let forwarder = spawn_forwarder(ForwarderCtx {
        app,
        db: state.db.clone(),
        registry: state.sessions.clone(),
        channel,
        session_id: session.id.clone(),
        workstream_id: session.workstream_id.clone(),
        rx,
        start_seq,
        // A relaunch (resume/restart/log-only rewind) is a NEW process; its
        // boundary drops the dead prior process's background tasks.
        boundary: Boundary::Relaunched,
    });
    state.sessions.set_forwarder(&session.id, forwarder).await;

    Ok(driver)
}

/// A freshly-started session plus the composed first prompt (pre-docs), so the
/// frontend can echo exactly what the agent received. The composition is
/// user-configurable (Settings → Prompts overrides + `{{variable}}`
/// substitution), so the frontend no longer mirrors it.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StartedSession {
    pub session: Session,
    /// The composed first prompt WITHOUT the `Relevant documents:` suffix —
    /// attached docs render as pin cards in the echo, not as raw paths.
    pub prompt: String,
}

/// Resolve the prompt `{{variables}}` for a workstream, computing the git-derived
/// ones (blocking git2 work) only when `template` actually references them.
async fn prompt_vars_for(ws: &Workstream, template: &str) -> pipeline::PromptVars {
    let want_changed = template.contains("{{changed_files}}");
    let want_diff = template.contains("{{diff}}");
    let want_last = template.contains("{{last_commit}}");
    let git_vars = if want_changed || want_diff || want_last {
        let worktree = ws.worktree_path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            hyprlayer_core::gitops::prompt_git_vars(
                std::path::Path::new(&worktree),
                want_changed,
                want_diff,
                want_last,
            )
        })
        .await
        .unwrap_or_default()
    } else {
        Default::default()
    };
    pipeline::PromptVars {
        branch: ws.branch.clone(),
        base_branch: ws
            .base_branch
            .strip_prefix("origin/")
            .unwrap_or(&ws.base_branch)
            .to_string(),
        changed_files: git_vars.changed_files,
        diff: git_vars.diff,
        last_commit: git_vars.last_commit,
    }
}

/// The effective Commit/Ship bodies for this launch: the user's Settings →
/// Prompts override, variable-substituted. Stock behaviour (`None`) when no
/// override exists or the stage isn't an action stage.
async fn action_prompt_bodies(
    state: &AppState,
    stage: Stage,
    ws: &Workstream,
) -> pipeline::ActionPromptBodies {
    let key = match stage {
        Stage::Commit => "commit",
        Stage::Ship => "ship",
        _ => return Default::default(),
    };
    let prompts = state.settings.get().await.prompts;
    // A blank/whitespace-only override reads as unset — a user who cleared the
    // textarea (and navigated away) must still get the stock behaviour, never
    // an empty first turn.
    let Some(template) = prompts
        .builtin_bodies
        .get(key)
        .filter(|t| !t.trim().is_empty())
    else {
        return Default::default();
    };
    let vars = prompt_vars_for(ws, template).await;
    let body = pipeline::substitute_vars(template, &vars);
    match stage {
        Stage::Commit => pipeline::ActionPromptBodies {
            commit: Some(body),
            ship: None,
        },
        _ => pipeline::ActionPromptBodies {
            commit: None,
            ship: Some(body),
        },
    }
}

/// Start a session for a workstream stage. Composes the stage skill prompt,
/// spawns the driver, and streams events back over `on_event`. Returns the
/// persisted [`Session`] row plus the composed prompt (for the transcript echo).
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
    let ws = {
        let id = workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .ok_or_else(|| AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {workstream_id}"),
        remedy: None,
    })?;

    let stage_kind = stage;
    // Ship names the workstream's target branch in its directive; other stages
    // ignore it. So a re-targeted workstream opens its PR against e.g. `develop`.
    // Commit/Ship bodies honor the user's Settings → Prompts overrides.
    let bodies = action_prompt_bodies(&state, stage_kind, &ws).await;
    let prompt = pipeline::compose_prompt(stage_kind, &input, &ws.base_branch, &bodies);
    // The agent additionally receives the attached-doc list; the echo does not
    // (docs render as pin cards in the UI instead of raw paths).
    let wire_prompt = pipeline::append_docs(prompt.clone(), &attached_docs);

    // Persist the session row before spawning, so a crash before init still
    // leaves a record.
    let session = {
        let new = NewSession {
            id: uuid::Uuid::new_v4().to_string(),
            workstream_id: ws.id.clone(),
            stage: stage_kind,
            harness: harness.unwrap_or_default(),
            cwd: ws.worktree_path.clone(),
        };
        db_task(state.db.clone(), move |db| db.insert_session(new)).await?
    };

    // Launch + first turn run in the BACKGROUND so this command returns as soon as
    // the row and prompt exist — the frontend then paints the echoed prompt and the
    // "Connecting to <harness>…" spinner immediately instead of blocking on a slow
    // launch. Codex handshakes (`initialize` + `thread/start`, ~1s) before its first
    // turn; Claude/OpenCode already return fast. Outcomes — including a failure —
    // arrive as events over `on_event`.
    let err_channel = on_event.clone();
    let launch = LaunchParams {
        app: app.clone(),
        session: session.clone(),
        project_path: ws.project_path,
        model,
        effort,
        permission_mode,
        resume: false,
        start_seq: 0,
        channel: on_event,
    };
    tauri::async_runtime::spawn(async move {
        // Re-acquire managed state off the command thread — AppState isn't `Clone`
        // (Mutex-guarded watchers), but the AppHandle lends it back.
        let state = app.state::<AppState>();
        match launch_session(&state, launch).await {
            // Kick off the first turn (the composed skill prompt + attached docs).
            Ok(driver) => {
                if let Err(e) = driver.send_user_turn(
                    &wire_prompt,
                    Some(stage_kind.as_str()),
                    prompt_name.as_deref(),
                ) {
                    emit_start_failure(&err_channel, &AppError::from(e).message);
                }
            }
            // Launch failed after the row was written (missing binary, logged out,
            // handshake timeout): emit a terminal event so "Connecting…" clears.
            Err(e) => emit_start_failure(&err_channel, &e.message),
        }
    });

    Ok(StartedSession { session, prompt })
}

/// Report a background launch failure over the session channel: a visible reason
/// then a terminal `Exited`, so the "Connecting…" spinner clears with the reason
/// instead of pulsing forever. Used only by [`start_stage_session`], whose launch
/// now runs after the command returns.
fn emit_start_failure(channel: &Channel<SessionEvent>, reason: &str) {
    let _ = channel.send(SessionEvent::AssistantText {
        text: format!("⚠ Failed to start the agent: {reason}"),
        parent_id: None,
    });
    let _ = channel.send(SessionEvent::Exited {
        outcome: ExitOutcome::SpawnFailed,
        code: None,
        stderr_tail: String::new(),
    });
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
    permission_mode: Option<PermissionMode>,
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
        let requested = permission_mode;
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
        db_task(state.db.clone(), move |db| db.next_session_seq(&id)).await?
    };
    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.set_session_state(&id, SessionState::Running)
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
            permission_mode,
            resume: true,
            start_seq,
            channel: on_event,
        },
    )
    .await?;

    Ok(session)
}

/// Restart a *live* session into a new permission mode and/or model. Claude
/// binds both its permission mode and its model at launch (a running session
/// can't switch into `bypassPermissions`, nor swap models), so the UI applies
/// such a change by relaunching the process. We do it atomically on the backend
/// — kill the live driver, wait for it to actually exit (the forwarder drops it
/// from the registry on its `Exited`), then re-spawn with `--resume` and the new
/// settings — so the frontend can't race the resume onto the still-dying old
/// process (which would keep the old settings). `model`/`effort` are `None` for a
/// mode-only restart — claude keeps the session's original model/effort when
/// `--model`/`--effort` are omitted on `--resume`; pass them to switch the model
/// mid-conversation. Conversation is preserved; the event log appends after its
/// tail.
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
        db_task(state.db.clone(), move |db| db.next_session_seq(&id)).await?
    };
    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.set_session_state(&id, SessionState::Running)
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
            permission_mode,
            resume: true,
            start_seq,
            channel: on_event,
        },
    )
    .await?;

    Ok(session)
}

/// Rewind a session to the N-th (1-based) user turn, discarding that turn and
/// everything after it. Two paths:
///
/// * **Live rewind (preferred).** If the session is live and its harness supports
///   it, we ask the harness to truncate the model's in-memory conversation to this
///   turn — claude's `rewind_conversation` control request
///   (see [`ClaudeDriver::rewind_to_ordinal`]), Codex's `thread/rollback` — then
///   keep that SAME process alive and hand its event stream to the caller's new
///   channel (see [`live_rewind_handoff`]). The model genuinely forgets the dropped
///   turns. We must NOT kill + resume here: claude's peel does not truncate its
///   on-disk transcript, so a resume would reload the un-rewound history.
/// * **Log-only fallback.** If the model rewind isn't available (session not live,
///   OpenCode, a turn from before the current generation, the first turn, or a
///   harness refusal) we kill + cut + resume: the UI/log truncate but the model
///   keeps its memory. `permission_mode` re-applies the session's posture on this
///   re-spawn (as [`resume_session`]/[`restart_session`] do) — `None` would drop a
///   bypass session back to the stage default and start prompting again.
///
/// `user_turn` is the 1-based ordinal of the clicked user bubble (the N-th
/// `userMessage` event). Returns the existing [`Session`] plus which path ran, so
/// the caller can tell a TRUE rewind (the model forgot) from a log-only prune (it
/// still recalls the dropped turns) instead of assuming success.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RewindResult {
    pub session: Session,
    /// True when the harness's own conversation was truncated on the live process
    /// (the model genuinely forgot the dropped turns); false when we could only
    /// prune the app log (not live / OpenCode / a pre-generation or first turn / a
    /// harness refusal) — the model still recalls them.
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

    // Hold the session's op lock for the whole rewind so a concurrent `send_user_message`
    // can't interleave a new prompt between the peel steps — the model would then
    // receive a turn the cut below deletes from the log.
    let op = state.sessions.op_lock(&session_id).await;
    let _op_guard = op.lock().await;

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

    // Preferred path: a TRUE rewind on the live process — the harness truncates
    // the model's in-memory conversation to this turn and we keep the SAME process
    // alive (a kill + resume would reload its un-truncated transcript). Both
    // harnesses that support it only reach turns from the current live generation
    // (claude needs the turn's uuid, Codex counts the turns it streamed), so any
    // miss — not live, OpenCode, a pre-resume turn, a refusal — is non-fatal and
    // drops to the fallback.
    let live_rewound = if let Some(driver) = state.sessions.get(&session_id).await {
        match driver.rewind_to_ordinal(user_turn as usize).await {
            Ok(()) => true,
            Err(e) => {
                log::info!(
                    "rewind_session {session_id}: model-level rewind unavailable ({e}); \
                     truncating the app log only"
                );
                false
            }
        }
    } else {
        false
    };
    if live_rewound {
        let session = live_rewind_handoff(&state, app, session, cutoff, on_event).await?;
        return Ok(RewindResult {
            session,
            live_rewound: true,
        });
    }

    // Fallback: log-only rewind. Resolve the workstream before any destructive
    // step, so a missing-row error can't strand us with the log already cut.
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
            db.set_session_state(&id, SessionState::Running)
        })
        .await;
    }

    // Continue the log above whatever survives the cut (see `next_session_seq`).
    let start_seq = {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| db.next_session_seq(&id)).await?
    };

    let launched = launch_session(
        &state,
        LaunchParams {
            app,
            session: session.clone(),
            project_path: ws.project_path,
            model: None,
            effort: None,
            // Preserve the session's current posture across the rewind relaunch —
            // otherwise `None` resolves to the stage default and a bypass session
            // starts prompting again (see the command doc).
            permission_mode,
            resume: true,
            start_seq,
            channel: on_event,
        },
    )
    .await;

    if let Err(e) = launched {
        // Roll the cut back so the session is recoverable via a normal resume.
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.restore_session_events(&id, &removed)?;
            db.set_session_state(&id, SessionState::Idle)
        })
        .await;
        return Err(e);
    }

    Ok(RewindResult {
        session,
        live_rewound: false,
    })
}

/// Keep the live process — whose model memory the peel already truncated — and
/// hand its event stream to the caller's new channel. The ordering is load-bearing:
///
/// 1. **Subscribe the new receiver FIRST**, before stopping the old forwarder, so
///    an event the live process emits during the handoff (a final `TurnDone`, a
///    background-task update) is buffered for the replacement rather than dropped
///    into a gap — a `broadcast` send with no receiver is lost.
/// 2. **Stop the old forwarder cooperatively** (signal + join, not abort), so any
///    in-flight `spawn_blocking` DB append finishes BEFORE the cut — an aborted
///    task's write can otherwise land after the cut and re-add a rewound row.
/// 3. Cut the app log to `cutoff`, then spawn a fresh forwarder on the SAME driver
///    at the log's new tail, draining the receiver from step 1.
///
/// The new forwarder emits a `Rewound` boundary: like a resume's `Relaunched` the
/// UI settles at it, but the process is UNCHANGED, so its still-running background
/// tasks can still complete-notify and are kept, not dropped.
async fn live_rewind_handoff(
    state: &AppState,
    app: tauri::AppHandle,
    session: Session,
    cutoff: i64,
    on_event: Channel<SessionEvent>,
) -> CmdResult<Session> {
    let driver = state
        .sessions
        .get(&session.id)
        .await
        .ok_or_else(|| session_not_found(&session.id))?;

    // (1) Subscribe before we stop the old forwarder — no gap where a live event
    // has no receiver and is lost from both the UI and the log.
    let rx = driver.subscribe();

    // (2) Stop the old forwarder cooperatively so it can't keep persisting past the
    // cut AND any in-flight write drains before (3) cuts the tail (which drops the
    // rewound turns and the interrupted turn's aborted tail).
    state.sessions.stop_forwarder(&session.id).await;
    {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| db.cut_session_events_from(&id, cutoff)).await?;
    }
    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| db.set_session_state(&id, SessionState::Running)).await;
    }

    // Continue above whatever survives the cut — robust to any last event the old
    // forwarder flushed before it stopped (a resumed forwarder must never reuse a
    // live seq). Any such event sits at seq >= cutoff and was just removed, so the
    // replacement re-persists it once from `rx`.
    let start_seq = {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| db.next_session_seq(&id)).await?
    };

    // (3) Fresh forwarder on the SAME live driver, wired to the caller's new channel
    // and the receiver subscribed in (1).
    let forwarder = spawn_forwarder(ForwarderCtx {
        app,
        db: state.db.clone(),
        registry: state.sessions.clone(),
        channel: on_event,
        session_id: session.id.clone(),
        workstream_id: session.workstream_id.clone(),
        rx,
        start_seq,
        // Same process — keep its background tasks (see the `Rewound` event doc).
        boundary: Boundary::Rewound,
    });
    state.sessions.set_forwarder(&session.id, forwarder).await;

    Ok(session)
}

/// Send a follow-up turn. `prompt_name` is set when the turn is a custom-prompt
/// run, so the persisted event can tell a card-collapsed prompt body from a
/// message the user typed.
#[tauri::command]
#[specta::specta]
pub async fn send_user_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
    attached_docs: Vec<String>,
    prompt_name: Option<String>,
) -> CmdResult<()> {
    // Serialize against an in-flight rewind on this session (same op lock): a send
    // must land entirely before or after a peel, never interleaved between its
    // steps where the cut would then delete the turn the model just received.
    let op = state.sessions.op_lock(&session_id).await;
    let _op_guard = op.lock().await;

    // Compose the doc list server-side (same `Relevant documents:` format as
    // session start) so the wire format lives only in `pipeline`, not in the
    // frontend. The optimistic bubble still shows just `text` + pin cards.
    let prompt = pipeline::append_docs(text, &attached_docs);
    live_driver(&state, &session_id)
        .await?
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

/// Same for reasoning effort, which Codex also takes per turn. A no-op when the
/// session isn't live, and for Claude/OpenCode (their drivers ignore it).
#[tauri::command]
#[specta::specta]
pub async fn set_session_effort(
    state: State<'_, AppState>,
    session_id: String,
    effort: Option<String>,
) -> CmdResult<()> {
    if let Some(driver) = state.sessions.get(&session_id).await {
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

/// The one-time boundary marker a relaunching forwarder emits before its stream
/// (only when `start_seq > 0`; a fresh start emits nothing). Both settle the UI at
/// the new boundary; they differ only in background-task handling.
#[derive(Clone, Copy)]
enum Boundary {
    /// A NEW process took over (resume/restart/log-only rewind) — drop the dead
    /// prior process's background tasks.
    Relaunched,
    /// A live rewind truncated the SAME process in place — keep its background
    /// tasks (they're still running and can still complete-notify).
    Rewound,
}

impl Boundary {
    fn event(self) -> SessionEvent {
        match self {
            Boundary::Relaunched => SessionEvent::Relaunched {},
            Boundary::Rewound => SessionEvent::Rewound {},
        }
    }
}

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
    /// Which boundary marker to emit before the stream when `start_seq > 0`.
    boundary: Boundary,
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
    State(SessionState),
    TurnDone {
        cost: Option<f64>,
    },
    Exited {
        state: SessionState,
        summary: Option<String>,
    },
    /// The app took the process down itself. The row keeps its pre-exit state so
    /// the session replays resumable; only the orphan ledger needs clearing.
    Detached,
}

fn spawn_forwarder(ctx: ForwarderCtx) -> ForwarderHandle {
    let ForwarderCtx {
        app,
        db,
        registry,
        channel,
        session_id,
        workstream_id,
        mut rx,
        start_seq,
        boundary,
    } = ctx;

    // Cooperative-stop signal: `stop_forwarder` sends (or drops) this so a live
    // rewind can drain the loop before cutting the log (see `ForwarderHandle`).
    let (shutdown, mut shutdown_rx) = oneshot::channel::<()>();

    let join = tauri::async_runtime::spawn(async move {
        let mut seq: i64 = start_seq;

        // A relaunch (resume/restart/rewind) advances `start_seq` past the prior
        // generation's events; emit the boundary marker before the new stream (see
        // the event docs). Skipped on a fresh start.
        if start_seq > 0 {
            let event = boundary.event();
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
            // Cooperative stop: a signalled (or dropped) `shutdown` breaks the loop
            // at the next poll. Writes below the select are awaited to completion
            // per iteration, so a stop can only land BETWEEN events — never leaving
            // a DB append in flight when `stop_forwarder`'s join returns.
            let event = tokio::select! {
                biased;
                _ = &mut shutdown_rx => break,
                recv = rx.recv() => match recv {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("session {session_id} forwarder lagged {n} events");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            };

            // `UserMessage` is persist-only: the UI already showed it optimistically
            // when the user sent it, so forwarding it would double-stamp the turn.
            let is_user_message = matches!(event, SessionEvent::UserMessage { .. });

            // `PartialText` is the streaming live-typing delta; it is ephemeral —
            // the coalesced `AssistantText` block is the durable record — so it is
            // never persisted, only forwarded. Skipping it removes a per-token
            // write storm. Usage pings stay persisted (tiny) so the context meter
            // survives a restart even when a turn was interrupted before its
            // result event landed. A `Detached` exit is skipped for a different
            // reason: the app ended that process for its own bookkeeping (quit,
            // relaunch), so writing it would stamp "Session ended" across every
            // transcript on the next launch.
            let ephemeral = match &event {
                SessionEvent::PartialText { .. } => true,
                SessionEvent::Exited { outcome, .. } => !outcome.is_reportable(),
                _ => false,
            };
            let payload = (!ephemeral)
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
                    Some(SessionState::Running),
                    false,
                ),
                SessionEvent::PermissionAsk { .. } => (
                    RowWrite::State(SessionState::AwaitingPermission),
                    Some(SessionState::AwaitingPermission),
                    false,
                ),
                SessionEvent::QuestionAsk { .. } => (
                    RowWrite::State(SessionState::AwaitingQuestion),
                    Some(SessionState::AwaitingQuestion),
                    false,
                ),
                SessionEvent::TurnDone { cost_usd, .. } => {
                    (RowWrite::TurnDone { cost: *cost_usd }, None, false)
                }
                // Terminal either way — the driver is gone, so the forwarder stops
                // and the registry drops it. A detached exit just says so quietly:
                // no row write, and no lifecycle emit (a relaunch announces its own
                // `Running`, and a quit has nobody left to tell).
                SessionEvent::Exited { outcome, .. } if !outcome.is_reportable() => {
                    (RowWrite::Detached, None, true)
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
                            let _ = db.set_session_state(&sid, SessionState::Running);
                        }
                        RowWrite::Exited { state, summary } => {
                            let _ = db.finish_session(&sid, state, summary.as_deref());
                            let _ = db.clear_orphans_for(&sid);
                        }
                        RowWrite::Detached => {
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

    ForwarderHandle { join, shutdown }
}

fn emit_lifecycle(
    app: &tauri::AppHandle,
    session_id: &str,
    workstream_id: &str,
    state: SessionState,
) {
    let _ = SessionLifecycle {
        session_id: session_id.to_string(),
        workstream_id: workstream_id.to_string(),
        state,
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
