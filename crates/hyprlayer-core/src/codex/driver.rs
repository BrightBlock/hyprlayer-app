use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::protocol::{
    approval_reply, decode_line, question_reply, CodexPosture, CollabMode, Incoming, Translator,
};
use crate::agent::{declined_message, rewind_unsupported, KillFlag, UsageSoFar};
use crate::claude::events::{ExitOutcome, SessionEvent};
use crate::error::{CoreError, Result};
use hyprlayer_models::session::QuestionAnswers;

const STDERR_RING_LINES: usize = 200;
const EVENT_CHANNEL_CAP: usize = 1024;
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

const REQUEST_USER_INPUT_CONFIG: &str = "tools.experimental_request_user_input.enabled=true";
// Opt-in since codex 0.152; without it `turn/plan/updated` never fires.
const UPDATE_PLAN_CONFIG: &str = "tools.update_plan.enabled=true";
// Personal custom agents are still guarded independently of the stable
// multi-agent runtime in Codex 0.149.x. Set the documented agent registry gate
// so installed persona TOMLs are available in every Hyprlayer Codex session.
const AGENTS_ENABLED_CONFIG: &str = "agents.enabled=true";
// The app owns goals; a native one would continue turns we never sent.
pub(crate) const GOALS_DISABLED_CONFIG: &str = "features.goals=false";
const APP_SERVER_ARGS: [&str; 10] = [
    "app-server",
    "--stdio",
    "-c",
    AGENTS_ENABLED_CONFIG,
    "-c",
    REQUEST_USER_INPUT_CONFIG,
    "-c",
    UPDATE_PLAN_CONFIG,
    "-c",
    GOALS_DISABLED_CONFIG,
];

pub(crate) fn initialize_params() -> Value {
    json!({
        "clientInfo": {
            "name": "hyprlayer",
            "title": "Hyprlayer",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": { "experimentalApi": true },
    })
}

pub struct SpawnConfig {
    pub codex_bin: PathBuf,
    pub cwd: PathBuf,
    pub posture: CodexPosture,
    pub add_dirs: Vec<PathBuf>,
    pub skill_roots: Vec<PathBuf>,
    pub resume: Option<String>,
    pub prior_user_turns: usize,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub ephemeral: bool,
    pub env: Vec<(String, String)>,
    pub developer_instructions: Option<String>,
    /// Per-spawn `-c` pairs after the fixed app-server args.
    pub extra_args: Vec<String>,
}

type PendingReplies =
    Arc<StdMutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>>;

pub struct CodexDriver {
    thread_id: String,
    stdin_tx: mpsc::UnboundedSender<String>,
    events: broadcast::Sender<SessionEvent>,
    rpc_seq: AtomicU64,
    model: StdMutex<Option<String>>,
    effort: StdMutex<Option<String>>,
    collab_mode: StdMutex<Option<CollabMode>>,
    thread_model: StdMutex<Option<String>>,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    pending: PendingReplies,
    state: Arc<StdMutex<Translator>>,
    killer: Killer,
    paginated: bool,
}

impl CodexDriver {
    pub async fn spawn(config: SpawnConfig) -> Result<(Self, broadcast::Receiver<SessionEvent>)> {
        let startup = crate::codex::startup_lock().lock().await;

        let mut cmd = Command::new(&config.codex_bin);
        crate::process::appimage::scrub_tokio(&mut cmd);
        cmd.args(APP_SERVER_ARGS)
            .args(&config.extra_args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = crate::process::spawn_supervised(&mut cmd).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::BinaryMissing {
                    name: "codex".into(),
                }
            } else {
                CoreError::Io(e)
            }
        })?;

        let pid = child.id();
        let stdin = child.take_stdin().expect("stdin piped");
        let stdout = child.take_stdout().expect("stdout piped");
        let stderr = child.take_stderr().expect("stderr piped");

        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAP);
        let primary_rx = events_tx.subscribe();
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        let stderr_ring = Arc::new(StdMutex::new(Vec::<String>::new()));
        let kill_flag = Arc::new(KillFlag::default());
        let pending = Arc::new(StdMutex::new(HashMap::new()));
        let state = Arc::new(StdMutex::new(Translator::resumed(config.prior_user_turns)));

        tokio::spawn(write_loop(stdin, stdin_rx));
        tokio::spawn(stderr_loop(stderr, stderr_ring.clone()));
        tokio::spawn(read_and_wait(
            child,
            stdout,
            events_tx.clone(),
            stderr_ring.clone(),
            kill_flag.clone(),
            pending.clone(),
            state.clone(),
            stdin_tx.downgrade(),
        ));

        let driver = Self {
            thread_id: String::new(),
            stdin_tx,
            events: events_tx,
            rpc_seq: AtomicU64::new(1),
            model: StdMutex::new(config.model.clone()),
            effort: StdMutex::new(config.effort.clone()),
            collab_mode: StdMutex::new(None),
            thread_model: StdMutex::new(None),
            stderr_ring,
            kill_flag,
            pending,
            state,
            killer: Killer { pid },
            paginated: false,
        };

        let started = driver.handshake(&config).await;
        drop(startup);
        let (thread_id, model, paginated) = match started {
            Ok(v) => v,
            Err(e) => {
                driver.kill();
                return Err(e);
            }
        };

        let driver = Self {
            thread_id,
            thread_model: StdMutex::new(model.clone()),
            paginated,
            ..driver
        };

        let _ = driver.events.send(SessionEvent::Init {
            provider_session_id: driver.thread_id.clone(),
            model,
            mcp_servers: None,
        });

        Ok((driver, primary_rx))
    }

    async fn handshake(&self, config: &SpawnConfig) -> Result<(String, Option<String>, bool)> {
        self.request("initialize", initialize_params()).await?;
        self.notify("initialized", json!({}))?;

        if !config.skill_roots.is_empty() {
            let roots: Vec<String> = config
                .skill_roots
                .iter()
                .map(|r| r.to_string_lossy().into_owned())
                .collect();
            let _ = self
                .request("skills/extraRoots/set", json!({ "extraRoots": roots }))
                .await;
        }

        let (method, params) = thread_params(config);

        let rpc_id = self.next_rpc_id();
        self.state
            .lock()
            .map_err(|_| CoreError::OutputParse {
                program: "codex".into(),
                detail: "protocol state was poisoned during thread handshake".into(),
            })?
            .expect_root_thread_from(rpc_id);
        let result = self.request_with_id(rpc_id, method, params).await?;
        let thread_id = result
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::OutputParse {
                program: "codex".into(),
                detail: format!("{method} returned no thread id"),
            })?
            .to_string();
        let model = result
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| config.model.clone());
        let paginated = is_paginated(&result);
        if paginated {
            let turn_ids = self
                .thread_turn_ids(&thread_id, &result, config.prior_user_turns)
                .await;
            if let Ok(mut s) = self.state.lock() {
                s.turn_ids = turn_ids;
            }
        }
        Ok((thread_id, model, paginated))
    }

    /// The thread's turn ids, oldest first, or `None` when they do not match the
    /// session's user turns one for one.
    async fn thread_turn_ids(
        &self,
        thread_id: &str,
        response: &Value,
        prior_user_turns: usize,
    ) -> Option<Vec<String>> {
        let window = turn_ids_in(response.get("thread").and_then(|t| t.get("turns")));
        let mut seen: HashSet<String> = window.iter().cloned().collect();
        let mut older = Vec::new();
        let mut cursor = response
            .get("turnsBackwardsCursor")
            .and_then(Value::as_str)
            .map(str::to_string);
        while let Some(current) = cursor.take() {
            let params = json!({
                "threadId": thread_id,
                "cursor": current,
                "sortDirection": "desc",
                "itemsView": "notLoaded",
            });
            let page = match self.request("thread/turns/list", params).await {
                Ok(page) => page,
                Err(e) => {
                    tracing::warn!("codex thread {thread_id}: its turns could not be listed: {e}");
                    return None;
                }
            };
            older.extend(
                turn_ids_in(page.get("data"))
                    .into_iter()
                    .filter(|id| seen.insert(id.clone())),
            );
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|next| *next != current)
                .map(str::to_string);
        }
        older.reverse();
        older.extend(window);
        if older.len() != prior_user_turns {
            tracing::info!(
                "codex thread {thread_id} holds {} turns but the session logged {prior_user_turns}; \
                 a rewind will not reach the model",
                older.len()
            );
            return None;
        }
        Some(older)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Mid-turn, the message steers the running turn, as codex's own Enter
    /// does; its echo waits for codex to commit it at the next step.
    pub fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
        client_message_id: Option<&str>,
    ) -> Result<()> {
        let echo = SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
            client_message_id: client_message_id.map(str::to_string),
            author: None,
        };
        let running = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.active_turn.clone().or_else(|| s.requested_turn.clone()));
        if let (Some(turn), Some(client)) = (running, client_message_id) {
            return self.steer(&turn, client, text, echo);
        }
        let _ = self.events.send(echo);

        let model = self.model.lock().ok().and_then(|m| m.clone());
        let effort = self.effort.lock().ok().and_then(|e| e.clone());
        let collab_mode = self.collab_mode.lock().ok().and_then(|mut m| m.take());
        let thread_model = self.thread_model.lock().ok().and_then(|m| m.clone());
        let params = turn_params(
            &self.thread_id,
            text,
            model.as_deref(),
            effort.as_deref(),
            collab_mode,
            thread_model.as_deref(),
        );

        let rx = self.request_channel("turn/start", params)?;
        let events = self.events.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            match rx.await {
                Ok(Err(e)) => {
                    let _ = events.send(SessionEvent::AssistantText {
                        text: format!("⚠ Codex request failed: {e}"),
                        parent_id: None,
                        injected: false,
                        auth_required: false,
                    });
                    let _ = events.send(SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                        turn_status: Some("failed".into()),
                        thread_tokens: None,
                    });
                }
                Ok(Ok(result)) => {
                    let id = result
                        .get("turn")
                        .and_then(|t| t.get("id"))
                        .and_then(Value::as_str);
                    if let (Some(id), Ok(mut s)) = (id, state.lock()) {
                        s.requested_turn = Some(id.to_string());
                    }
                }
                Err(_) => {}
            }
        });
        Ok(())
    }

    fn steer(&self, turn: &str, client: &str, text: &str, echo: SessionEvent) -> Result<()> {
        if let Ok(mut s) = self.state.lock() {
            s.steer_sent(client.to_string(), echo);
        }
        let params = json!({
            "threadId": self.thread_id,
            "expectedTurnId": turn,
            "clientUserMessageId": client,
            "input": text_input(text),
        });
        let rx = match self.request_channel("turn/steer", params) {
            Ok(rx) => rx,
            Err(e) => {
                if let Ok(mut s) = self.state.lock() {
                    s.take_steer(client);
                }
                return Err(e);
            }
        };
        let events = self.events.clone();
        let state = self.state.clone();
        let client = client.to_string();
        tokio::spawn(async move {
            // The turn ended first, isn't steerable, or this codex predates steering.
            if let Ok(Err(e)) = rx.await {
                tracing::debug!("codex refused a steer: {e}");
                let echo = state.lock().ok().and_then(|mut s| s.take_steer(&client));
                if let Some(declined) = echo.as_ref().and_then(declined_message) {
                    let _ = events.send(declined);
                }
            }
        });
        Ok(())
    }

    pub fn set_model(&self, model: Option<String>) {
        if let Ok(mut m) = self.model.lock() {
            *m = model;
        }
    }

    pub fn set_effort(&self, effort: Option<String>) {
        if let Ok(mut e) = self.effort.lock() {
            *e = effort;
        }
    }

    pub fn set_collab_mode(&self, mode: Option<CollabMode>) {
        if let Ok(mut m) = self.collab_mode.lock() {
            *m = mode;
        }
    }

    pub fn observed_collab_mode(&self) -> Option<String> {
        self.state.lock().ok().and_then(|s| s.collab_mode.clone())
    }

    pub fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        let entry = self
            .state
            .lock()
            .ok()
            .and_then(|mut s| s.pending_approvals.remove(request_id));
        let Some((rpc_id, kind)) = entry else {
            return Ok(());
        };
        let _ = self.events.send(SessionEvent::PermissionResolved {
            request_id: request_id.to_string(),
            allow,
        });
        self.reply(rpc_id, approval_reply(&kind, allow, message))
    }

    pub fn pending_requests(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|s| {
                s.pending_approvals
                    .keys()
                    .chain(s.pending_questions.keys())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> Result<()> {
        let pending = self
            .state
            .lock()
            .ok()
            .and_then(|mut s| s.pending_questions.remove(request_id));
        let Some(pending) = pending else {
            return Ok(());
        };
        let _ = self.events.send(SessionEvent::QuestionAnswered {
            request_id: request_id.to_string(),
        });
        let body = question_reply(&pending, &json!(answers));
        self.reply(pending.rpc_id.clone(), body)
    }

    pub fn interrupt(&self) -> Result<()> {
        let turn_id = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.active_turn.clone().or_else(|| s.requested_turn.clone()));
        let Some(turn_id) = turn_id else {
            return Ok(());
        };
        drop(self.request_channel(
            "turn/interrupt",
            json!({ "threadId": self.thread_id, "turnId": turn_id }),
        )?);
        Ok(())
    }

    pub async fn rewind_to_ordinal(&self, ordinal: usize) -> Result<()> {
        let (method, params) = {
            let state = self.state.lock().map_err(|_| rewind_unsupported())?;
            rewind_request(&self.thread_id, self.paginated, ordinal, &state)?
        };
        match self.request(method, params).await {
            // codex 0.156 removed thread/rollback, which leaves legacy threads
            // with no model-level rewind.
            Err(CoreError::OutputParse { detail, .. })
                if detail.contains("unknown variant `thread/rollback`") =>
            {
                return Err(rewind_unsupported());
            }
            result => result?,
        };
        if let Ok(mut s) = self.state.lock() {
            s.rewound_to(ordinal);
        }
        Ok(())
    }

    pub async fn skills(&self) -> Result<Vec<String>> {
        let result = self.request("skills/list", json!({})).await?;
        Ok(result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("skills").and_then(Value::as_array))
            .flatten()
            .filter_map(|skill| skill.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect())
    }

    pub fn kill(&self) {
        self.kill_flag.mark_killed();
        self.killer.kill();
    }

    pub fn kill_detached(&self) {
        self.kill_flag.mark_detached();
        self.killer.kill();
    }

    pub fn pid(&self) -> Option<u32> {
        self.killer.pid
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// The thread's total, which a resumed driver already knows before its first turn.
    pub fn usage_so_far(&self) -> UsageSoFar {
        UsageSoFar {
            cost_usd: None,
            thread_tokens: self.state.lock().ok().and_then(|s| s.thread_tokens),
        }
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr_ring
            .lock()
            .map(|r| r.join("\n"))
            .unwrap_or_default()
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_rpc_id();
        self.request_with_id(id, method, params).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn request_for_test(&self, method: &str, params: Value) -> Result<Value> {
        self.request(method, params).await
    }

    async fn request_with_id(&self, id: u64, method: &str, params: Value) -> Result<Value> {
        let rx = self.open_request(id, method, params)?;
        let replied = tokio::time::timeout(STARTUP_TIMEOUT, rx).await;
        let failed = |detail: String| CoreError::OutputParse {
            program: "codex".into(),
            detail,
        };
        match replied {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(e))) => Err(failed(format!("{method}: {e}"))),
            Ok(Err(_)) => {
                let tail = self.stderr_tail();
                Err(failed(if tail.is_empty() {
                    format!("{method}: codex app-server exited")
                } else {
                    format!("{method}: codex app-server exited: {tail}")
                }))
            }
            Err(_) => Err(failed(format!("{method}: timed out"))),
        }
    }

    fn next_rpc_id(&self) -> u64 {
        self.rpc_seq.fetch_add(1, Ordering::SeqCst)
    }

    fn request_channel(
        &self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<std::result::Result<Value, String>>> {
        self.open_request(self.next_rpc_id(), method, params)
    }

    fn open_request(
        &self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<std::result::Result<Value, String>>> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(id, tx);
        }
        self.write(json!({ "id": id, "method": method, "params": params }))?;
        Ok(rx)
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(json!({ "method": method, "params": params }))
    }

    fn reply(&self, id: Value, result: Value) -> Result<()> {
        self.write(json!({ "id": id, "result": result }))
    }

    fn write(&self, value: Value) -> Result<()> {
        let mut line = value.to_string();
        line.push('\n');
        self.stdin_tx
            .send(line)
            .map_err(|_| CoreError::OutputParse {
                program: "codex".into(),
                detail: "session stdin is closed".into(),
            })
    }
}

fn thread_params(config: &SpawnConfig) -> (&'static str, Value) {
    let CodexPosture {
        approval_policy,
        sandbox,
        network_access,
    } = config.posture;
    let mut params = json!({
        "cwd": config.cwd.to_string_lossy(),
        "approvalPolicy": approval_policy,
        "sandbox": sandbox,
    });
    if let Some(model) = &config.model {
        params["model"] = json!(model);
    }
    if let Some(instructions) = &config.developer_instructions {
        params["developerInstructions"] = json!(instructions);
    }
    if sandbox == "workspace-write" {
        let roots: Vec<String> = config
            .add_dirs
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect();
        params["config"] = json!({
            "sandbox_workspace_write": {
                "network_access": network_access,
                "writable_roots": roots,
            }
        });
    }
    match &config.resume {
        Some(thread_id) => {
            params["threadId"] = json!(thread_id);
            ("thread/resume", params)
        }
        None => {
            if config.ephemeral {
                params["ephemeral"] = json!(true);
            }
            ("thread/start", params)
        }
    }
}

pub fn config_sets_developer_instructions(config_toml: &str) -> bool {
    toml::from_str::<toml::Table>(config_toml).is_ok_and(|config| {
        config
            .get("developer_instructions")
            .is_some_and(toml::Value::is_str)
    })
}

fn turn_params(
    thread_id: &str,
    text: &str,
    model: Option<&str>,
    effort: Option<&str>,
    collab_mode: Option<CollabMode>,
    thread_model: Option<&str>,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "input": text_input(text),
    });
    if let Some(model) = model {
        params["model"] = json!(model);
    }
    if let Some(effort) = effort {
        params["effort"] = json!(effort);
    }
    if let (Some(mode), Some(m)) = (collab_mode, model.or(thread_model)) {
        params["collaborationMode"] = json!({
            "mode": mode.as_str(),
            "settings": { "model": m },
        });
    }
    params
}

fn text_input(text: &str) -> Value {
    json!([{ "type": "text", "text": text, "text_elements": [] }])
}

// A schema default: a thread without the field is legacy.
fn is_paginated(response: &Value) -> bool {
    response
        .get("thread")
        .and_then(|t| t.get("historyMode"))
        .and_then(Value::as_str)
        == Some("paginated")
}

fn turn_ids_in(turns: Option<&Value>) -> Vec<String> {
    turns
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|turn| turn.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

// Paginated threads reject thread/rollback, so they revert by turn id.
fn rewind_request(
    thread_id: &str,
    paginated: bool,
    ordinal: usize,
    state: &Translator,
) -> Result<(&'static str, Value)> {
    let turn = state
        .turn_of(ordinal)
        .ok_or_else(|| CoreError::OutputParse {
            program: "codex".into(),
            detail: "that message joined a running turn, and codex reverts whole turns".into(),
        })?;
    if !paginated {
        let num_turns = rollback_count(turn, state.turns())?;
        return Ok((
            "thread/rollback",
            json!({ "threadId": thread_id, "numTurns": num_turns }),
        ));
    }
    let ids = state
        .turn_ids
        .as_deref()
        .ok_or_else(|| CoreError::OutputParse {
            program: "codex".into(),
            detail: "this thread's turns do not match the session's user turns".into(),
        })?;
    let before = &ids[turn_index(turn, ids.len())?];
    Ok((
        "thread/revert",
        json!({ "threadId": thread_id, "beforeTurnId": before }),
    ))
}

fn rollback_count(ordinal: usize, total: usize) -> Result<u64> {
    Ok((total - turn_index(ordinal, total)?) as u64)
}

fn turn_index(ordinal: usize, total: usize) -> Result<usize> {
    let unsupported = |detail: &str| CoreError::OutputParse {
        program: "codex".into(),
        detail: detail.to_string(),
    };
    if ordinal == 0 {
        return Err(unsupported("turn ordinals are 1-based"));
    }
    if ordinal > total {
        return Err(unsupported(
            "that turn predates this session's live generation",
        ));
    }
    Ok(ordinal - 1)
}

async fn write_loop(
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(line) = rx.recv().await {
        if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
}

async fn stderr_loop(stderr: tokio::process::ChildStderr, ring: Arc<StdMutex<Vec<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(mut ring) = ring.lock() {
            ring.push(line);
            let overflow = ring.len().saturating_sub(STDERR_RING_LINES);
            if overflow > 0 {
                ring.drain(0..overflow);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_and_wait(
    mut child: crate::process::SupervisedChild,
    stdout: tokio::process::ChildStdout,
    events: broadcast::Sender<SessionEvent>,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    pending: PendingReplies,
    state: Arc<StdMutex<Translator>>,
    stdin_tx: mpsc::WeakUnboundedSender<String>,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut clean_exit = false;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let Some(incoming) = decode_line(&line) else {
                    // Recorded like claude's unrecognised lines, so the store
                    // shows what codex versions actually send.
                    if !line.trim().is_empty() {
                        tracing::debug!(
                            "codex sent a line that is not JSON-RPC ({} bytes)",
                            line.len()
                        );
                        let _ = events.send(SessionEvent::ProtocolError {
                            detail: "codex sent output the app could not read".into(),
                            raw: line.chars().take(2000).collect(),
                        });
                    }
                    continue;
                };
                clean_exit |= route_incoming(incoming, &events, &pending, &state, &stdin_tx);
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("reading codex output failed: {e}");
                break;
            }
        }
    }

    if let Ok(mut map) = pending.lock() {
        for (_, tx) in map.drain() {
            let _ = tx.send(Err("codex app-server exited".into()));
        }
    }

    let status = child.wait().await;
    let code = status.as_ref().ok().and_then(|s| s.code());
    let stderr_tail = stderr_ring.lock().map(|r| r.join("\n")).unwrap_or_default();

    let outcome = kill_flag.outcome().unwrap_or_else(|| match &status {
        Ok(s) if s.success() || clean_exit => ExitOutcome::Completed,
        Ok(s) => crate::process::exit::outcome_from_status(s),
        Err(_) => ExitOutcome::SpawnFailed,
    });

    let _ = events.send(SessionEvent::Exited {
        outcome,
        code,
        stderr_tail,
    });
}

fn route_incoming(
    incoming: Incoming,
    events: &broadcast::Sender<SessionEvent>,
    pending: &PendingReplies,
    state: &Arc<StdMutex<Translator>>,
    stdin_tx: &mpsc::WeakUnboundedSender<String>,
) -> bool {
    match incoming {
        Incoming::Response { id, result } => {
            if let Ok(mut s) = state.lock() {
                s.observe_response(id, &result);
            }
            let waiter = pending.lock().ok().and_then(|mut m| m.remove(&id));
            if let Some(tx) = waiter {
                let _ = tx.send(result);
            }
            false
        }
        Incoming::Notification { method, params } => {
            let (closes_root, out) = match state.lock() {
                Ok(mut s) => (
                    method == "thread/closed" && !s.is_child_thread(&params),
                    s.notification(&method, &params),
                ),
                Err(_) => (method == "thread/closed", Vec::new()),
            };
            for event in out {
                let _ = events.send(event);
            }
            closes_root
        }
        Incoming::ServerRequest { id, method, params } => {
            // Child-thread requests must remain routable: their agent cannot
            // continue until an approval or user-input response is delivered.
            let out = match state.lock() {
                Ok(mut s) => s.server_request(&id, &method, &params),
                Err(_) => Vec::new(),
            };
            if out.is_empty() {
                auto_decline(stdin_tx, &id, &method);
            }
            for event in out {
                let _ = events.send(event);
            }
            false
        }
    }
}

fn auto_decline(stdin_tx: &mpsc::WeakUnboundedSender<String>, id: &Value, method: &str) {
    tracing::warn!(
        "codex asked for {method} (request {id}), which the app does not handle; declined"
    );
    let result = match method {
        "mcpServer/elicitation/request" => json!({ "action": "decline" }),
        "item/tool/requestUserInput" => json!({ "answers": {} }),
        "execCommandApproval" | "applyPatchApproval" => json!({
            "decision": { "denied": { "rejection": "unsupported by this client" } }
        }),
        _ => json!({}),
    };
    let line = format!("{}\n", json!({ "id": id, "result": result }));
    if let Some(tx) = stdin_tx.upgrade() {
        let _ = tx.send(line);
    }
}

struct Killer {
    pid: Option<u32>,
}

impl Killer {
    fn kill(&self) {
        let Some(pid) = self.pid else { return };
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000)
                .output();
        }
    }
}

#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::PermissionMode;

    #[test]
    fn rollback_count_maps_ordinal_to_turns_dropped() {
        assert_eq!(rollback_count(2, 5).ok(), Some(4));
        assert_eq!(rollback_count(5, 5).ok(), Some(1));
        assert_eq!(rollback_count(1, 5).ok(), Some(5));
    }

    #[test]
    fn rollback_uses_session_global_totals_after_a_resume() {
        let mut t = Translator::resumed(2);
        for _ in 0..5 {
            t.notification(
                "item/completed",
                &json!({ "item": { "type": "userMessage", "id": "u", "content": [] } }),
            );
        }
        assert_eq!(t.user_turns, 7, "seeded count plus streamed turns");
        assert_eq!(rollback_count(3, t.user_turns).ok(), Some(5));
        assert_eq!(rollback_count(3, 5).ok(), Some(3));
    }

    #[test]
    fn rollback_rejects_unreachable_turns() {
        assert!(rollback_count(0, 5).is_err());
        assert!(rollback_count(6, 5).is_err());
        assert!(rollback_count(1, 0).is_err());
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn tracking(turn_ids: &[&str]) -> Translator {
        let mut t = Translator::resumed(turn_ids.len());
        t.turn_ids = Some(ids(turn_ids));
        t
    }

    #[test]
    fn only_a_paginated_history_mode_reverts() {
        assert!(is_paginated(
            &json!({ "thread": { "id": "t1", "historyMode": "paginated" } })
        ));
        assert!(!is_paginated(
            &json!({ "thread": { "id": "t1", "historyMode": "legacy" } })
        ));
        assert!(!is_paginated(&json!({ "thread": { "id": "t1" } })));
    }

    #[test]
    fn a_paginated_rewind_reverts_before_the_ordinals_turn() {
        let t = tracking(&["a", "b", "c"]);
        for (ordinal, before) in [(1, "a"), (2, "b"), (3, "c")] {
            let (method, params) = rewind_request("t1", true, ordinal, &t).unwrap();
            assert_eq!(method, "thread/revert");
            assert_eq!(params, json!({ "threadId": "t1", "beforeTurnId": before }));
        }
        assert!(rewind_request("t1", true, 0, &t).is_err());
        assert!(rewind_request("t1", true, 4, &t).is_err());
    }

    #[test]
    fn a_legacy_rewind_still_rolls_back_by_count() {
        let (method, params) = rewind_request("t1", false, 2, &tracking(&["a", "b", "c"])).unwrap();
        assert_eq!(method, "thread/rollback");
        assert_eq!(params, json!({ "threadId": "t1", "numTurns": 2 }));

        let (method, params) = rewind_request("t1", false, 2, &Translator::resumed(5)).unwrap();
        assert_eq!(method, "thread/rollback");
        assert_eq!(params, json!({ "threadId": "t1", "numTurns": 4 }));
    }

    #[test]
    fn a_paginated_thread_without_aligned_turn_ids_does_not_revert() {
        assert!(rewind_request("t1", true, 1, &Translator::resumed(3)).is_err());
    }

    fn fake_driver(paginated: bool, state: Translator) -> (CodexDriver, FakeCodex) {
        let (stdin_tx, requests) = mpsc::unbounded_channel();
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let driver = CodexDriver {
            thread_id: "t1".into(),
            stdin_tx,
            events: broadcast::channel(8).0,
            rpc_seq: AtomicU64::new(1),
            model: StdMutex::new(None),
            effort: StdMutex::new(None),
            collab_mode: StdMutex::new(None),
            thread_model: StdMutex::new(None),
            stderr_ring: Arc::new(StdMutex::new(Vec::new())),
            kill_flag: Arc::new(KillFlag::default()),
            pending: pending.clone(),
            state: Arc::new(StdMutex::new(state)),
            killer: Killer { pid: None },
            paginated,
        };
        (driver, FakeCodex { requests, pending })
    }

    struct FakeCodex {
        requests: mpsc::UnboundedReceiver<String>,
        pending: PendingReplies,
    }

    impl FakeCodex {
        async fn answer(
            &mut self,
            method: &str,
            reply: std::result::Result<Value, String>,
        ) -> Value {
            let line = self.requests.recv().await.expect("a request");
            let request: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(request["method"], json!(method));
            let id = request["id"].as_u64().unwrap();
            let waiter = self.pending.lock().unwrap().remove(&id).unwrap();
            waiter.send(reply).unwrap();
            request["params"].clone()
        }
    }

    #[tokio::test]
    async fn a_revert_forgets_the_dropped_turns() {
        let (driver, mut codex) = fake_driver(true, tracking(&["a", "b", "c"]));
        let (rewound, params) = tokio::join!(
            driver.rewind_to_ordinal(2),
            codex.answer("thread/revert", Ok(json!({ "thread": { "id": "t1" } })))
        );
        rewound.unwrap();
        assert_eq!(params["beforeTurnId"], json!("b"));
        let state = driver.state.lock().unwrap();
        assert_eq!(state.turn_ids, Some(ids(&["a"])));
        assert_eq!(state.user_turns, 1);
    }

    #[tokio::test]
    async fn a_refused_revert_keeps_the_turns() {
        let (driver, mut codex) = fake_driver(true, tracking(&["a", "b"]));
        let (rewound, _) = tokio::join!(
            driver.rewind_to_ordinal(1),
            codex.answer("thread/revert", Err("thread not found: t1".into()))
        );
        assert!(rewound.is_err());
        assert_eq!(
            driver.state.lock().unwrap().turn_ids,
            Some(ids(&["a", "b"]))
        );
    }

    #[tokio::test]
    async fn a_removed_rollback_reads_as_an_unsupported_rewind() {
        let (driver, mut codex) = fake_driver(false, Translator::resumed(3));
        // codex 0.156.0-alpha.7's reply, abridged.
        let removed = "Invalid request: unknown variant `thread/rollback`, expected one of \
                       `initialize`, `thread/start`";
        let (rewound, params) = tokio::join!(
            driver.rewind_to_ordinal(1),
            codex.answer("thread/rollback", Err(removed.into()))
        );
        assert_eq!(params["numTurns"], json!(3));
        match rewound {
            Err(CoreError::OutputParse { detail, .. }) => {
                assert_eq!(
                    detail,
                    "conversation rewind is not supported by this harness"
                )
            }
            other => panic!("expected the unsupported-rewind error, got {other:?}"),
        }
        assert_eq!(driver.state.lock().unwrap().user_turns, 3);
    }

    #[tokio::test]
    async fn a_resume_seeds_turn_ids_from_its_window_then_older_pages() {
        let (driver, mut codex) = fake_driver(true, Translator::default());
        let resumed = json!({
            "thread": { "id": "t1", "historyMode": "paginated", "turns": [{ "id": "c" }, { "id": "d" }] },
            "turnsBackwardsCursor": "at-d",
        });
        let pages = async {
            let first = codex
                .answer(
                    "thread/turns/list",
                    Ok(json!({ "data": [{ "id": "d" }, { "id": "c" }, { "id": "b" }], "nextCursor": "at-b" })),
                )
                .await;
            assert_eq!(first["cursor"], json!("at-d"));
            assert_eq!(first["sortDirection"], json!("desc"));
            let second = codex
                .answer(
                    "thread/turns/list",
                    Ok(json!({ "data": [{ "id": "a" }], "nextCursor": null })),
                )
                .await;
            assert_eq!(second["cursor"], json!("at-b"));
        };
        let (seeded, ()) = tokio::join!(driver.thread_turn_ids("t1", &resumed, 4), pages);
        assert_eq!(seeded, Some(ids(&["a", "b", "c", "d"])));
    }

    #[tokio::test]
    async fn a_resumed_thread_that_disagrees_with_the_log_keeps_no_turn_ids() {
        let (driver, _codex) = fake_driver(true, Translator::default());
        let resumed = json!({ "thread": { "id": "t1", "turns": [{ "id": "a" }, { "id": "b" }] } });
        assert_eq!(driver.thread_turn_ids("t1", &resumed, 3).await, None);
        assert_eq!(
            driver.thread_turn_ids("t1", &resumed, 2).await,
            Some(ids(&["a", "b"]))
        );
    }

    #[test]
    fn unmodelled_server_requests_are_declined_not_ignored() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        auto_decline(&tx.downgrade(), &json!(9), "mcpServer/elicitation/request");
        let line = rx.try_recv().expect("a reply was written");
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], json!(9));
        assert_eq!(v["result"]["action"], json!("decline"));

        auto_decline(&tx.downgrade(), &json!(10), "applyPatchApproval");
        let line = rx.try_recv().expect("a reply was written");
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(
            v["result"]["decision"]["denied"]["rejection"],
            json!("unsupported by this client")
        );
    }

    #[test]
    fn an_mcp_tool_approval_is_asked_and_answered() {
        fn written(codex: &mut FakeCodex) -> Option<Value> {
            let line = codex.requests.try_recv().ok()?;
            Some(serde_json::from_str(line.trim()).unwrap())
        }
        let (driver, mut codex) = fake_driver(true, Translator::default());
        let mut events = driver.subscribe();
        let feed = |line: &str| {
            route_incoming(
                decode_line(line).expect("a JSON-RPC line"),
                &driver.events,
                &driver.pending,
                &driver.state,
                &driver.stdin_tx.downgrade(),
            );
        };
        let mut asked = || {
            std::iter::from_fn(|| events.try_recv().ok()).find_map(|e| match e {
                SessionEvent::PermissionAsk {
                    request_id,
                    tool_name,
                    input,
                    ..
                } => Some((request_id, tool_name, input)),
                _ => None,
            })
        };
        let tool_approval = |rpc: u64, tool: &str, params: &str| {
            format!(
                r#"{{"method":"mcpServer/elicitation/request","id":{rpc},"params":{{"threadId":"th","turnId":"t1","serverName":"probe","environmentId":null,"mode":"form","_meta":{{"codex_approval_kind":"mcp_tool_call","persist":["session","always"],"tool_name":"{tool}","tool_params":{params},"tool_params_display":[]}},"message":"Allow the probe MCP server to run tool \"{tool}\"?","requestedSchema":{{"type":"object","properties":{{}}}}}}}}"#
            )
        };

        feed(&tool_approval(0, "wipe_cache", r#"{"scope":"all"}"#));
        let (request_id, tool_name, input) = asked().expect("the approval is asked");
        assert_eq!(tool_name, "mcp__probe__wipe_cache");
        assert_eq!(input, r#"{"scope":"all"}"#);
        assert_eq!(written(&mut codex), None, "the ask waits for the user");
        assert_eq!(driver.pending_requests(), vec![request_id.clone()]);
        driver.respond_permission(&request_id, true, None).unwrap();
        assert_eq!(
            written(&mut codex),
            Some(json!({ "id": 0, "result": { "action": "accept", "content": {} } }))
        );

        feed(&tool_approval(1, "plain_tool", "{}"));
        let (request_id, tool_name, _) = asked().expect("the approval is asked");
        assert_eq!(tool_name, "mcp__probe__plain_tool");
        driver.respond_permission(&request_id, false, None).unwrap();
        assert_eq!(
            written(&mut codex),
            Some(json!({ "id": 1, "result": { "action": "decline" } }))
        );

        feed(
            r#"{"method":"mcpServer/elicitation/request","id":2,"params":{"threadId":"th","turnId":"t1","serverName":"probe","environmentId":null,"mode":"form","_meta":null,"message":"Deploy to production?","requestedSchema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}}}"#,
        );
        assert_eq!(asked(), None);
        assert_eq!(
            written(&mut codex),
            Some(json!({ "id": 2, "result": { "action": "decline" } }))
        );
        assert!(driver.pending_requests().is_empty());
    }

    #[test]
    fn app_server_args_enable_the_user_input_and_plan_tools() {
        let configs: Vec<&str> = APP_SERVER_ARGS
            .windows(2)
            .filter(|w| w[0] == "-c")
            .map(|w| w[1])
            .collect();
        for flag in [
            "tools.experimental_request_user_input.enabled=true",
            "tools.update_plan.enabled=true",
        ] {
            assert!(configs.contains(&flag), "{flag} missing: {configs:?}");
        }
    }

    #[test]
    fn initialize_advertises_the_experimental_api() {
        let params = initialize_params();
        assert_eq!(params["capabilities"]["experimentalApi"], json!(true));
        assert_eq!(params["clientInfo"]["name"], json!("hyprlayer"));
    }

    fn spawn_config(
        mode: PermissionMode,
        resume: Option<&str>,
        developer_instructions: Option<&str>,
    ) -> SpawnConfig {
        SpawnConfig {
            codex_bin: PathBuf::from("codex"),
            cwd: PathBuf::from("/w"),
            posture: CodexPosture::from_permission_mode(mode),
            add_dirs: vec![PathBuf::from("/t")],
            skill_roots: Vec::new(),
            resume: resume.map(str::to_string),
            prior_user_turns: 0,
            model: None,
            effort: None,
            ephemeral: false,
            env: Vec::new(),
            developer_instructions: developer_instructions.map(str::to_string),
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn thread_params_start_carries_developer_instructions() {
        let (method, params) =
            thread_params(&spawn_config(PermissionMode::Default, None, Some("notes")));
        assert_eq!(method, "thread/start");
        assert_eq!(params["developerInstructions"], json!("notes"));
        assert!(params.get("threadId").is_none());
    }

    #[test]
    fn thread_params_resume_carries_developer_instructions() {
        let config = spawn_config(PermissionMode::Default, Some("t1"), Some("notes"));
        let (method, params) = thread_params(&config);
        assert_eq!(method, "thread/resume");
        assert_eq!(params["threadId"], json!("t1"));
        assert_eq!(params["developerInstructions"], json!("notes"));
    }

    #[test]
    fn thread_params_omit_absent_developer_instructions() {
        for resume in [None, Some("t1")] {
            let (_, params) = thread_params(&spawn_config(PermissionMode::Default, resume, None));
            assert!(params.get("developerInstructions").is_none());
        }
    }

    #[test]
    fn thread_params_keep_the_sandbox_shape() {
        let (_, params) = thread_params(&spawn_config(PermissionMode::Default, None, None));
        assert_eq!(
            params["config"]["sandbox_workspace_write"]["writable_roots"],
            json!(["/t"])
        );

        let (_, params) = thread_params(&spawn_config(PermissionMode::Plan, None, None));
        assert!(params.get("config").is_none());

        let start = SpawnConfig {
            ephemeral: true,
            ..spawn_config(PermissionMode::Plan, None, None)
        };
        assert_eq!(thread_params(&start).1["ephemeral"], json!(true));
        let resume = SpawnConfig {
            ephemeral: true,
            ..spawn_config(PermissionMode::Plan, Some("t1"), None)
        };
        assert!(thread_params(&resume).1.get("ephemeral").is_none());
    }

    #[test]
    fn config_sets_developer_instructions_for_a_top_level_string() {
        let trust = "[projects.\"/w\"]\ntrust_level = \"trusted\"\n";
        assert!(config_sets_developer_instructions(&format!(
            "developer_instructions = \"Be terse.\"\n{trust}"
        )));
        assert!(!config_sets_developer_instructions(trust));
    }

    #[test]
    fn a_turn_without_a_collab_mode_is_unchanged() {
        let params = turn_params("t1", "hi", Some("gpt-5"), Some("high"), None, None);
        assert_eq!(params["threadId"], json!("t1"));
        assert_eq!(params["input"][0]["text"], json!("hi"));
        assert_eq!(params["model"], json!("gpt-5"));
        assert_eq!(params["effort"], json!("high"));
        assert!(params.get("collaborationMode").is_none());
    }

    #[test]
    fn collab_mode_falls_back_to_the_thread_model() {
        let params = turn_params(
            "t1",
            "hi",
            None,
            None,
            Some(CollabMode::Plan),
            Some("gpt-5.6-sol"),
        );
        assert_eq!(
            params["collaborationMode"]["settings"]["model"],
            json!("gpt-5.6-sol")
        );
        assert_eq!(params["collaborationMode"]["mode"], json!("plan"));
        assert!(params.get("model").is_none());
    }

    #[test]
    fn a_collab_mode_without_a_model_is_dropped() {
        let params = turn_params("t1", "hi", None, None, Some(CollabMode::Plan), None);
        assert!(params.get("collaborationMode").is_none());
    }

    #[tokio::test]
    async fn routing_filters_child_notifications_only() {
        let (events, mut event_rx) = broadcast::channel(8);
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let state = Arc::new(StdMutex::new(Translator::default()));
        state.lock().unwrap().expect_root_thread_from(42);
        let (stdin_tx, _stdin_rx) = mpsc::unbounded_channel();

        let (reply_tx, reply_rx) = oneshot::channel();
        pending.lock().unwrap().insert(42, reply_tx);
        assert!(!route_incoming(
            Incoming::Response {
                id: 42,
                result: Ok(json!({ "thread": { "id": "root-thread" } })),
            },
            &events,
            &pending,
            &state,
            &stdin_tx.downgrade(),
        ));
        assert_eq!(
            reply_rx.await.unwrap().unwrap()["thread"]["id"],
            json!("root-thread"),
            "handshake responses must bypass notification filtering"
        );

        assert!(!route_incoming(
            Incoming::Notification {
                method: "turn/completed".into(),
                params: json!({ "threadId": "child-thread", "turn": {
                    "id": "child-turn", "status": "completed"
                } }),
            },
            &events,
            &pending,
            &state,
            &stdin_tx.downgrade(),
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        assert!(!route_incoming(
            Incoming::ServerRequest {
                id: json!(51),
                method: "item/commandExecution/requestApproval".into(),
                params: json!({ "threadId": "child-thread", "turnId": "child-turn",
                    "itemId": "child-command", "command": "touch child-output", "cwd": "/w"
                }),
            },
            &events,
            &pending,
            &state,
            &stdin_tx.downgrade(),
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(SessionEvent::PermissionAsk { request_id, .. }) if request_id == "child-thread:child-command"
        ));

        assert!(route_incoming(
            Incoming::Notification {
                method: "thread/closed".into(),
                params: json!({ "threadId": "root-thread" }),
            },
            &events,
            &pending,
            &state,
            &stdin_tx.downgrade(),
        ));
        assert!(!route_incoming(
            Incoming::Notification {
                method: "thread/closed".into(),
                params: json!({ "threadId": "child-thread" }),
            },
            &events,
            &pending,
            &state,
            &stdin_tx.downgrade(),
        ));
    }

    #[tokio::test]
    async fn routing_empty_answers_child_questions_and_surfaces_the_root_question() {
        let (events, mut event_rx) = broadcast::channel(8);
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let state = Arc::new(StdMutex::new(Translator::default()));
        state
            .lock()
            .unwrap()
            .set_root_thread_id("root-thread".into());
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel();
        let question = |thread_id: &str, item_id: &str| {
            json!({
                "threadId": thread_id,
                "turnId": format!("{thread_id}-turn"),
                "itemId": item_id,
                "questions": [{
                    "id": format!("{item_id}-choice"),
                    "header": "Choice",
                    "question": "Continue?",
                    "options": []
                }]
            })
        };

        for (rpc_id, thread_id, item_id) in [
            (61, "child-one", "child-question-one"),
            (62, "child-two", "child-question-two"),
        ] {
            assert!(!route_incoming(
                Incoming::ServerRequest {
                    id: json!(rpc_id),
                    method: "item/tool/requestUserInput".into(),
                    params: question(thread_id, item_id),
                },
                &events,
                &pending,
                &state,
                &stdin_tx.downgrade(),
            ));
            let line = stdin_rx.try_recv().expect("child question was answered");
            let reply: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(reply["id"], json!(rpc_id));
            assert_eq!(reply["result"], json!({ "answers": {} }));
        }
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(state.lock().unwrap().pending_questions.is_empty());

        assert!(!route_incoming(
            Incoming::ServerRequest {
                id: json!(63),
                method: "item/tool/requestUserInput".into(),
                params: question("root-thread", "root-question"),
            },
            &events,
            &pending,
            &state,
            &stdin_tx.downgrade(),
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(SessionEvent::QuestionAsk { request_id, .. }) if request_id == "root-thread:root-question"
        ));
        assert!(matches!(
            stdin_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(state
            .lock()
            .unwrap()
            .pending_questions
            .contains_key("root-thread:root-question"));
    }

    #[test]
    fn an_ask_codex_settles_itself_closes_once() {
        let mut state = Translator::default();
        state.set_root_thread_id("th".into());
        let (driver, _codex) = fake_driver(true, state);
        let mut events = driver.subscribe();
        let feed = |line: &str| {
            route_incoming(
                decode_line(line).expect("a JSON-RPC line"),
                &driver.events,
                &driver.pending,
                &driver.state,
                &driver.stdin_tx.downgrade(),
            );
        };
        let mut settled = || {
            std::iter::from_fn(|| events.try_recv().ok())
                .filter(|e| {
                    matches!(
                        e,
                        SessionEvent::PermissionResolved { .. }
                            | SessionEvent::QuestionAnswered { .. }
                    )
                })
                .collect::<Vec<_>>()
        };
        let interrupted = |turn: &str| {
            format!(
                r#"{{"method":"turn/completed","params":{{"threadId":"th","turn":{{"id":"{turn}","status":"interrupted","error":null}}}}}}"#
            )
        };
        let resolved = |rpc: u64| {
            format!(
                r#"{{"method":"serverRequest/resolved","params":{{"threadId":"th","requestId":{rpc}}}}}"#
            )
        };

        feed(
            r#"{"method":"item/commandExecution/requestApproval","id":0,"params":{"kind":"command","threadId":"th","turnId":"t1","itemId":"exec-1","command":"/usr/bin/bash -lc 'touch probe-one.txt'","cwd":"/w"}}"#,
        );
        feed(&interrupted("t1"));
        feed(&resolved(0));
        assert!(driver.pending_requests().is_empty());
        assert_eq!(
            settled(),
            [SessionEvent::PermissionResolved {
                request_id: "th:exec-1".into(),
                allow: false,
            }]
        );

        feed(
            r#"{"method":"item/tool/requestUserInput","id":1,"params":{"threadId":"th","turnId":"t2","itemId":"call-1","questions":[{"id":"color_choice","header":"Color","question":"Which color do you choose?","isOther":true,"isSecret":false,"options":[{"label":"Red","description":"Choose red."},{"label":"Blue","description":"Choose blue."}]}],"isBlocking":true,"autoResolutionMs":null}}"#,
        );
        feed(&interrupted("t2"));
        feed(&resolved(1));
        assert!(driver.pending_requests().is_empty());
        assert_eq!(
            settled(),
            [SessionEvent::QuestionAnswered {
                request_id: "th:call-1".into(),
            }]
        );

        feed(
            r#"{"method":"item/commandExecution/requestApproval","id":2,"params":{"kind":"command","threadId":"th","turnId":"t3","itemId":"exec-2","command":"ls","cwd":"/w"}}"#,
        );
        driver.respond_permission("th:exec-2", true, None).unwrap();
        feed(&resolved(2));
        assert_eq!(
            settled(),
            [SessionEvent::PermissionResolved {
                request_id: "th:exec-2".into(),
                allow: true,
            }]
        );
    }
}
