use std::collections::HashMap;
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
use crate::agent::KillFlag;
use crate::claude::events::{ExitOutcome, SessionEvent};
use crate::error::{CoreError, Result};
use hyprlayer_models::session::QuestionAnswers;

const STDERR_RING_LINES: usize = 200;
const EVENT_CHANNEL_CAP: usize = 1024;
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

const REQUEST_USER_INPUT_CONFIG: &str = "tools.experimental_request_user_input.enabled=true";
// Personal custom agents are still guarded independently of the stable
// multi-agent runtime in Codex 0.149.x. Set the documented agent registry gate
// so installed persona TOMLs are available in every Hyprlayer Codex session.
const AGENTS_ENABLED_CONFIG: &str = "agents.enabled=true";
const APP_SERVER_ARGS: [&str; 6] = [
    "app-server",
    "--stdio",
    "-c",
    AGENTS_ENABLED_CONFIG,
    "-c",
    REQUEST_USER_INPUT_CONFIG,
];

fn initialize_params() -> Value {
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
}

impl CodexDriver {
    pub async fn spawn(config: SpawnConfig) -> Result<(Self, broadcast::Receiver<SessionEvent>)> {
        let startup = crate::codex::startup_lock().lock().await;

        let mut cmd = Command::new(&config.codex_bin);
        crate::process::appimage::scrub_tokio(&mut cmd);
        cmd.args(APP_SERVER_ARGS)
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
        };

        let started = driver.handshake(&config).await;
        drop(startup);
        let (thread_id, model) = match started {
            Ok(v) => v,
            Err(e) => {
                driver.kill();
                return Err(e);
            }
        };

        let driver = Self {
            thread_id,
            thread_model: StdMutex::new(model.clone()),
            ..driver
        };

        let _ = driver.events.send(SessionEvent::Init {
            provider_session_id: driver.thread_id.clone(),
            model,
        });

        Ok((driver, primary_rx))
    }

    async fn handshake(&self, config: &SpawnConfig) -> Result<(String, Option<String>)> {
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
        Ok((thread_id, model))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> Result<()> {
        let _ = self.events.send(SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
        });

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
                    });
                    let _ = events.send(SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                        turn_status: Some("failed".into()),
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
        self.reply(rpc_id, approval_reply(&kind, allow, message))
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
        let total = self.state.lock().map(|s| s.user_turns).unwrap_or(0);
        let num_turns = rollback_count(ordinal, total)?;
        self.request(
            "thread/rollback",
            json!({ "threadId": self.thread_id, "numTurns": num_turns }),
        )
        .await?;
        if let Ok(mut s) = self.state.lock() {
            s.user_turns = ordinal.saturating_sub(1);
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

// The oldest installed codex whose generated app-server schema was checked for
// developerInstructions (0.152.0 and 0.154.0); 0.149.x-0.151.x are unverified.
const DEVELOPER_INSTRUCTIONS_SINCE: [u64; 3] = [0, 152, 0];

pub fn accepts_developer_instructions(version: Option<&str>) -> bool {
    version
        .and_then(crate::process::binaries::parse_semver)
        .is_some_and(|v| v >= DEVELOPER_INSTRUCTIONS_SINCE)
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
        "input": [{ "type": "text", "text": text, "text_elements": [] }],
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

fn rollback_count(ordinal: usize, total: usize) -> Result<u64> {
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
    Ok((total - ordinal + 1) as u64)
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
                    continue;
                };
                clean_exit |= route_incoming(incoming, &events, &pending, &state, &stdin_tx);
            }
            Ok(None) => break,
            Err(_) => break,
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
    fn app_server_args_enable_the_user_input_tool() {
        assert_eq!(
            APP_SERVER_ARGS,
            [
                "app-server",
                "--stdio",
                "-c",
                "agents.enabled=true",
                "-c",
                "tools.experimental_request_user_input.enabled=true"
            ]
        );
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
    fn accepts_developer_instructions_from_0_152() {
        for version in ["0.152.0", "0.154.0"] {
            assert!(accepts_developer_instructions(Some(version)), "{version}");
        }
        for version in [Some("0.151.9"), Some("0.149.1"), None, Some("garbage")] {
            assert!(!accepts_developer_instructions(version), "{version:?}");
        }
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
}
