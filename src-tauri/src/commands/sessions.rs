use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use hyprlayer_core::agent::{AgentDriver, Harness};
use hyprlayer_core::agents;
use hyprlayer_core::claude::workflow::{self, SessionWorkflows, WorkflowLaunch};
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
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify};

use super::{db_task, effective_for};
use crate::error::{AppError, CmdResult};
use crate::events::SessionLifecycle;
use crate::session_registry::{ForwarderHandle, SessionRegistry, WorkflowChannel};
use crate::settings::opencode_model_is_supported;
use crate::state::AppState;

fn session_not_found(id: &str) -> AppError {
    AppError {
        code: "session_not_found".into(),
        message: format!("No live session with id {id}"),
        remedy: Some("The session may have ended; start a new one.".into()),
    }
}

fn unsupported_opencode_model() -> AppError {
    AppError {
        code: "opencode_provider_unsupported".into(),
        message: "GitHub Copilot models are no longer supported".into(),
        remedy: Some("Choose a model from a supported OpenCode provider.".into()),
    }
}

fn unavailable_opencode_model(model: &str) -> AppError {
    AppError {
        code: "opencode_model_unavailable".into(),
        message: format!("OpenCode does not currently offer `{model}`"),
        remedy: Some(
            "Refresh the model list in Settings → OpenCode and pick a current model.".into(),
        ),
    }
}

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

async fn live_driver(state: &AppState, session_id: &str) -> CmdResult<Arc<dyn AgentDriver>> {
    state
        .sessions
        .get(session_id)
        .await
        .ok_or_else(|| session_not_found(session_id))
}

async fn load_session(state: &AppState, id: &str) -> CmdResult<Session> {
    let owned = id.to_string();
    db_task(state.db.clone(), move |db| db.get_session(&owned))
        .await?
        .ok_or_else(|| session_not_found(id))
}

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

#[derive(Debug, Default, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct StartOpts {
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub prompt_name: Option<String>,
}

struct LaunchParams {
    app: tauri::AppHandle,
    session: Session,
    project_path: String,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    resume: bool,
    start_seq: i64,
    channel: Channel<SessionEvent>,
}

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

    let mut launched_mode: Option<PermissionMode> = None;
    let (driver, rx): (Arc<dyn AgentDriver>, broadcast::Receiver<SessionEvent>) = match session
        .harness
    {
        Harness::Opencode => {
            if model
                .as_deref()
                .is_some_and(|model| !opencode_model_is_supported(model))
            {
                return Err(unsupported_opencode_model());
            }
            let bin = state
                .binaries
                .require(BinaryKind::OpenCode)
                .await
                .map_err(AppError::from)?;
            let path = state.binaries.augmented_path().await;
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
            if let Some(model) = model.as_deref() {
                if server.knows_model(model).await == Some(false) {
                    return Err(unavailable_opencode_model(model));
                }
            }
            let (driver, rx) = crate::opencode::OpenCodeDriver::spawn(server, &session.cwd, model)
                .await
                .map_err(AppError::internal)?;
            let driver: Arc<dyn AgentDriver> = driver;
            (driver, rx)
        }
        Harness::Codex => {
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

            let resume_thread = resume.then(|| session.claude_session_id.clone()).flatten();
            if resume && resume_thread.is_none() {
                log::warn!(
                    "codex session {} has no recorded thread id; starting a new thread \
                         (the model will not have the replayed history)",
                    session.id
                );
            }

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
                skill_roots: hyprlayer_core::skills::skill_roots(Some(std::path::Path::new(
                    &session.cwd,
                ))),
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

            let effective = effective_for(state, &project_path).await?;
            let add_dirs: Vec<PathBuf> = effective.backend.thoughts_root().into_iter().collect();
            let mut policy = pipeline::policy_for(session.stage);
            let settings = state.settings.get().await;
            let library = &settings.agents;
            let drafts: Vec<String> =
                agents::list_agents(Some(std::path::Path::new(&project_path)))
                    .into_iter()
                    .filter(|a| {
                        library
                            .overlays
                            .get(&a.path)
                            .is_some_and(|o| o.status == "draft")
                    })
                    .map(|a| a.name)
                    .collect();
            policy
                .disallowed_tools
                .extend(agents::agent_deny_rules(&drafts));
            let permission_mode = permission_mode.unwrap_or(policy.permission_mode);
            launched_mode = Some(permission_mode);

            let path = state.binaries.augmented_path().await;
            let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
            let claude_cfg = settings.providers.claude;
            env.extend(super::claude_auth_env(&claude_cfg));

            let (driver, rx) = ClaudeDriver::spawn(SpawnConfig {
                claude_bin: claude,
                cwd: PathBuf::from(&session.cwd),
                session_id: session.id.clone(),
                permission_mode,
                allowed_tools: policy.allowed_tools,
                disallowed_tools: policy.disallowed_tools,
                add_dirs,
                resume: resume.then(|| session.id.clone()),
                fork: false,
                max_budget_usd: None,
                model,
                effort,
                chrome: claude_cfg.chrome_enabled,
                agent: pipeline::agent_for(session.stage),
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
        workflow: state.sessions.workflow(&session.id).await,
        start_seq,
        boundary: Boundary::Relaunched,
    });
    state.sessions.set_forwarder(&session.id, forwarder).await;

    Ok(driver)
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StartedSession {
    pub session: Session,
    pub prompt: String,
}

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
    let bodies = action_prompt_bodies(&state, stage_kind, &ws).await;
    let prompt = pipeline::compose_prompt(stage_kind, &input, &ws.base_branch, &bodies);
    let wire_prompt = pipeline::append_docs(prompt.clone(), &attached_docs);

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
        let state = app.state::<AppState>();
        match launch_session(&state, launch).await {
            Ok(driver) => {
                let mut turns = (stage_kind == Stage::Ship).then(|| driver.subscribe());
                if let Err(e) = driver.send_user_turn(
                    &wire_prompt,
                    Some(stage_kind.as_str()),
                    prompt_name.as_deref(),
                ) {
                    emit_start_failure(&err_channel, &AppError::from(e).message);
                } else if let Some(rx) = turns.as_mut() {
                    ship_describe_followup(driver.as_ref(), rx).await;
                }
            }
            Err(e) => emit_start_failure(&err_channel, &e.message),
        }
    });

    Ok(StartedSession { session, prompt })
}

async fn ship_describe_followup(
    driver: &dyn AgentDriver,
    rx: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
) {
    loop {
        match rx.recv().await {
            Ok(SessionEvent::TurnDone { .. }) => break,
            Ok(SessionEvent::Exited { .. }) | Err(_) => return,
            Ok(_) => {}
        }
    }
    let _ = driver.send_user_turn(pipeline::SHIP_DESCRIBE_PROMPT, None, None);
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
    let session = {
        let id = session_id.clone();
        db_task(state.db.clone(), move |db| db.get_session(&id)).await?
    }
    .ok_or_else(|| session_not_found(&session_id))?;

    if let Some(driver) = state.sessions.get(&session_id).await {
        let requested = permission_mode;
        let running = state.sessions.permission_mode(&session_id).await;
        if requested.is_none() || requested == running {
            tail_live(driver, on_event);
            return Ok(session);
        }
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

    let op = state.sessions.op_lock(&session_id).await;
    let _op_guard = op.lock().await;

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

    let ws = load_workstream(&state, &session.workstream_id).await?;

    kill_and_await_exit(&state, &session_id).await;

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
            permission_mode,
            resume: true,
            start_seq,
            channel: on_event,
        },
    )
    .await;

    if let Err(e) = launched {
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

    let rx = driver.subscribe();

    state.sessions.stop_forwarder(&session.id).await;
    {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| {
            db.cut_session_events_from(&id, cutoff)
        })
        .await?;
    }
    if let Some(w) = state.sessions.workflow(&session.id).await {
        let workflows = w.workflows.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            workflows.blocking_lock().runs.retain_armed_below(cutoff);
        })
        .await;
    }
    {
        let id = session.id.clone();
        let _ = db_task(state.db.clone(), move |db| {
            db.set_session_state(&id, SessionState::Running)
        })
        .await;
    }

    let start_seq = {
        let id = session.id.clone();
        db_task(state.db.clone(), move |db| db.next_session_seq(&id)).await?
    };

    let forwarder = spawn_forwarder(ForwarderCtx {
        app,
        db: state.db.clone(),
        registry: state.sessions.clone(),
        channel: on_event,
        session_id: session.id.clone(),
        workstream_id: session.workstream_id.clone(),
        rx,
        workflow: state.sessions.workflow(&session.id).await,
        start_seq,
        boundary: Boundary::Rewound,
    });
    state.sessions.set_forwarder(&session.id, forwarder).await;

    Ok(session)
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
    let op = state.sessions.op_lock(&session_id).await;
    let _op_guard = op.lock().await;

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
pub async fn set_session_model(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
) -> CmdResult<()> {
    if let Some(model_id) = model.as_deref() {
        if load_session(&state, &session_id).await?.harness == Harness::Opencode {
            if !opencode_model_is_supported(model_id) {
                return Err(unsupported_opencode_model());
            }
            if let Some(server) = state.opencode.current().await {
                if server.knows_model(model_id).await == Some(false) {
                    return Err(unavailable_opencode_model(model_id));
                }
            }
        }
    }
    if let Some(driver) = state.sessions.get(&session_id).await {
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

    if let Some(driver) = state.sessions.get(&session_id).await {
        tail_live(driver, on_event);
    }
    Ok(())
}

fn tail_live(driver: Arc<dyn AgentDriver>, on_event: Channel<SessionEvent>) {
    let mut rx = driver.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let terminal = matches!(event, SessionEvent::Exited { .. });
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

#[derive(Clone, Copy)]
enum Boundary {
    Relaunched,
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
    workflow: Option<WorkflowChannel>,
    start_seq: i64,
    boundary: Boundary,
}

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
    Detached,
}

async fn recv_workflow(workflow: Option<&WorkflowChannel>) -> Option<SessionEvent> {
    match workflow {
        Some(w) => w.rx.lock().await.recv().await,
        None => std::future::pending().await,
    }
}

fn spawn_workflow_reader(
    workflows: Weak<Mutex<SessionWorkflows>>,
    notify: Arc<Notify>,
    tx: mpsc::UnboundedSender<SessionEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            notify.notified().await;
            let Some(handle) = workflows.upgrade() else {
                return;
            };
            let events = match tauri::async_runtime::spawn_blocking(move || {
                handle.blocking_lock().runs.read_tick()
            })
            .await
            {
                Ok(events) => events,
                Err(err) => {
                    log::warn!("workflow reader tick failed: {err}");
                    return;
                }
            };
            for event in events {
                if tx.send(event).is_err() {
                    return;
                }
            }
        }
    })
}

async fn arm_workflow_run(
    w: &WorkflowChannel,
    launch: WorkflowLaunch,
    workflow_tool_use_id: String,
    armed_seq: i64,
    seen_uses: HashSet<String>,
    seen_results: HashSet<String>,
) {
    let Some(workflows_dir) = launch.run_dir.parent().map(|p| p.to_path_buf()) else {
        log::warn!(
            "workflow run dir {} has no parent; not arming",
            launch.run_dir.display()
        );
        return;
    };

    let workflows = w.workflows.clone();
    let armed = tauri::async_runtime::spawn_blocking(move || {
        let mut guard = workflows.blocking_lock();
        guard.runs.register_seeded(
            launch,
            workflow_tool_use_id,
            armed_seq,
            seen_uses,
            seen_results,
        );
        guard.arm(&workflows_dir, Box::new(|| {}))
    })
    .await;
    let (notify, newly_armed) = match armed {
        Ok(Ok(armed)) => armed,
        Ok(Err(err)) => {
            log::warn!("could not arm the workflow watch: {err}");
            return;
        }
        Err(err) => {
            log::warn!("workflow arming task failed: {err}");
            return;
        }
    };

    if newly_armed {
        let reader =
            spawn_workflow_reader(Arc::downgrade(&w.workflows), notify.clone(), w.tx.clone());
        let workflows = w.workflows.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            workflows.blocking_lock().attach_reader(reader);
        })
        .await;
    }
    notify.notify_one();
}

async fn recover_workflow_runs(db: &Arc<Db>, session_id: &str, w: &WorkflowChannel) {
    let sid = session_id.to_string();
    let handle = db.clone();
    let events =
        match tauri::async_runtime::spawn_blocking(move || handle.session_events(&sid)).await {
            Ok(Ok(events)) => events,
            Ok(Err(err)) => {
                log::warn!("session {session_id}: could not scan the log for workflow runs: {err}");
                return;
            }
            Err(err) => {
                log::warn!("session {session_id}: workflow recovery scan failed: {err}");
                return;
            }
        };

    let mut workflow_tool_ids: HashSet<String> = HashSet::new();
    let mut seen_uses: HashSet<String> = HashSet::new();
    let mut seen_results: HashSet<String> = HashSet::new();
    let mut ended: HashSet<String> = HashSet::new();
    let mut launches: Vec<(WorkflowLaunch, String, i64)> = Vec::new();

    for stored in &events {
        let Ok(event) = serde_json::from_str::<SessionEvent>(&stored.payload) else {
            continue;
        };
        match event {
            SessionEvent::ToolUse { id, name, .. } => {
                if name == "Workflow" {
                    workflow_tool_ids.insert(id.clone());
                }
                seen_uses.insert(id);
            }
            SessionEvent::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                if workflow_tool_ids.contains(&tool_use_id) {
                    if let Some(launch) = workflow::workflow_launch_from_result(&content) {
                        launches.push((launch, tool_use_id.clone(), stored.seq as i64));
                    }
                }
                seen_results.insert(tool_use_id);
            }
            SessionEvent::BackgroundTaskUpdate {
                tool_use_id,
                status,
                ..
            } => {
                if let Some(id) = tool_use_id.filter(|_| status != "running") {
                    ended.insert(id);
                }
            }
            _ => {}
        }
    }

    for (launch, tool_use_id, armed_seq) in launches {
        if ended.contains(&tool_use_id) {
            continue;
        }
        arm_workflow_run(
            w,
            launch,
            tool_use_id,
            armed_seq,
            seen_uses.clone(),
            seen_results.clone(),
        )
        .await;
    }
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
        workflow,
        start_seq,
        boundary,
    } = ctx;

    let (shutdown, mut shutdown_rx) = oneshot::channel::<()>();

    let join = tauri::async_runtime::spawn(async move {
        let mut seq: i64 = start_seq;

        if start_seq > 0 {
            let event = boundary.event();
            let kind = event.kind();
            if let Ok(payload) = serde_json::to_string(&event) {
                let _ = channel.send(event);
                let cur_seq = seq;
                let db = db.clone();
                let sid = session_id.clone();
                let persisted = tauri::async_runtime::spawn_blocking(move || {
                    db.append_session_event(&sid, cur_seq, kind, &payload)
                        .is_ok()
                })
                .await
                .unwrap_or(false);
                if persisted {
                    seq += 1;
                }
            }
        }

        if start_seq > 0 && matches!(boundary, Boundary::Relaunched) {
            if let Some(w) = &workflow {
                recover_workflow_runs(&db, &session_id, w).await;
            }
        }

        loop {
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
                Some(event) = recv_workflow(workflow.as_ref()) => event,
            };

            let is_user_message = matches!(event, SessionEvent::UserMessage { .. });

            let ephemeral = match &event {
                SessionEvent::PartialText { .. } => true,
                SessionEvent::Exited { outcome, .. } => !outcome.is_reportable(),
                _ => false,
            };
            let payload = (!ephemeral)
                .then(|| serde_json::to_string(&event).ok())
                .flatten();
            let kind = event.kind();

            let launched_workflow = match &event {
                SessionEvent::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => workflow::workflow_launch_from_result(content)
                    .map(|launch| (launch, tool_use_id.clone(), seq)),
                _ => None,
            };

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

            if !is_user_message {
                let _ = channel.send(event);
            }

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

            if let (Some(w), Some((launch, tool_use_id, armed_seq))) =
                (&workflow, launched_workflow)
            {
                arm_workflow_run(
                    w,
                    launch,
                    tool_use_id,
                    armed_seq,
                    HashSet::new(),
                    HashSet::new(),
                )
                .await;
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_dropped_session_reaps_its_workflow_watch_and_reader() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workflows = Arc::new(Mutex::new(SessionWorkflows::new()));
        let (tx, rx) = mpsc::unbounded_channel::<SessionEvent>();

        let notify = {
            let mut guard = workflows.lock().await;
            let (notify, armed) = guard
                .arm(&tmp.path().join("subagents/workflows"), Box::new(|| {}))
                .expect("arming creates the directory it watches");
            assert!(armed, "the session's watch must really be armed");
            let reader =
                spawn_workflow_reader(Arc::downgrade(&workflows), notify.clone(), tx.clone());
            guard.attach_reader(reader);
            notify
        };

        let weak = Arc::downgrade(&workflows);
        drop(workflows);
        drop(tx);
        drop(rx);

        assert!(
            weak.upgrade().is_none(),
            "the reader task is still holding the session's workflows alive, so the debouncer and the task itself can never be stopped"
        );

        for _ in 0..200 {
            if Arc::strong_count(&notify) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            Arc::strong_count(&notify),
            1,
            "the reader task outlived the watch that was supposed to abort it"
        );
    }
}
