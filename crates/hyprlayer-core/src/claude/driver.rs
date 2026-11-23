use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::events::{ExitOutcome, SessionEvent};
use super::protocol::{self, Block, ParsedLine};
use crate::agent::{declined_message, KillFlag, UsageSoFar};
use crate::error::{CoreError, Result};
use crate::store::session_event_codec::questions_from_value;
use hyprlayer_models::session::{QuestionAnswers, QuestionSubject};

pub use hyprlayer_models::agent::PermissionMode;

pub(crate) fn permission_mode_flag(mode: PermissionMode) -> &'static str {
    mode.as_str()
}

pub struct SpawnConfig {
    pub claude_bin: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub add_dirs: Vec<PathBuf>,
    pub resume: Option<String>,
    pub fork: bool,
    pub max_budget_usd: Option<f64>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub chrome: bool,
    pub agent: Option<crate::pipeline::SessionAgent>,
    pub append_system_prompt: Option<String>,
    pub env: Vec<(String, String)>,
    pub claude_config_dir: Option<PathBuf>,
    /// `--mcp-config` JSON; variadic, which is safe here because `-p` stream-json takes no positional.
    pub mcp_config: Option<String>,
    pub plugin_dir: Option<PathBuf>,
}

const STDERR_RING_LINES: usize = 200;
const EVENT_CHANNEL_CAP: usize = 1024;
const REWIND_TIMEOUT: Duration = Duration::from_secs(20);
const REMOTE_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

struct RewindOutcome {
    rewound: bool,
    error: Option<String>,
}

/// A message written mid-turn waits in claude's queue until its next step, so
/// its echo waits for the `command_lifecycle` frame that says claude took it in.
#[derive(Default)]
struct Inbox {
    busy: bool,
    /// Claude has sent a lifecycle frame for one of our messages.
    lifecycle: bool,
    /// Wire uuid and held echo, oldest first.
    waiting: Vec<(String, SessionEvent)>,
    /// Our messages claude has yet to settle; their frames are ours to swallow.
    own: HashSet<String>,
    /// Being withdrawn: claude's `cancelled` for these is the user's, not a drop.
    cancelling: HashSet<String>,
}

impl Inbox {
    /// Nothing waiting either: claude starts a waiting message right after a `result`.
    fn idle(&self) -> bool {
        !self.busy && self.waiting.is_empty()
    }

    fn take(&mut self, wire: &str) -> Option<SessionEvent> {
        let at = self.waiting.iter().position(|(w, _)| w == wire)?;
        Some(self.waiting.remove(at).1)
    }

    fn wire_for(&self, client_message_id: &str) -> Option<String> {
        self.waiting.iter().find_map(|(wire, echo)| {
            matches!(echo, SessionEvent::UserMessage { client_message_id: Some(id), .. } if id == client_message_id)
                .then(|| wire.clone())
        })
    }

    /// `None` for another command's frame; otherwise what our own frame emits.
    fn frame(&mut self, wire: &str, state: &str) -> Option<Option<SessionEvent>> {
        if !self.own.contains(wire) {
            return None;
        }
        self.lifecycle = true;
        Some(match state {
            "queued" => None,
            "started" => {
                self.busy = true;
                self.take(wire)
            }
            _ => {
                self.own.remove(wire);
                let withdrawn = self.cancelling.remove(wire);
                self.take(wire)
                    .filter(|_| !withdrawn)
                    .and_then(|echo| declined_message(&echo))
            }
        })
    }
}

fn lock(inbox: &StdMutex<Inbox>) -> MutexGuard<'_, Inbox> {
    inbox.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct ClaudeDriver {
    session_id: String,
    stdin_tx: mpsc::UnboundedSender<String>,
    events: broadcast::Sender<SessionEvent>,
    control_seq: std::sync::atomic::AtomicU64,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    pending_permissions: Arc<StdMutex<HashMap<String, Value>>>,
    pending_controls: Arc<StdMutex<HashMap<String, oneshot::Sender<Value>>>>,
    inbox: Arc<StdMutex<Inbox>>,
    /// The process's cumulative `total_cost_usd` as of its last `result`.
    cost_so_far: Arc<StdMutex<Option<f64>>>,
    budget_cap: Option<f64>,
    killer: Killer,
    claude_config_dir: Option<PathBuf>,
    cwd: PathBuf,
}

impl ClaudeDriver {
    pub fn spawn(config: SpawnConfig) -> Result<(Self, broadcast::Receiver<SessionEvent>)> {
        let mut cmd = Command::new(&config.claude_bin);
        cmd.arg("-p")
            .args(["--output-format", "stream-json"])
            .args(["--input-format", "stream-json"])
            .arg("--verbose")
            .arg("--include-partial-messages")
            .args(["--permission-prompt-tool", "stdio"])
            // Without it, a live switch to bypass is refused with `bypass_not_launched`.
            .arg("--allow-dangerously-skip-permissions")
            .args([
                "--permission-mode",
                permission_mode_flag(config.permission_mode),
            ]);

        if let Some(resume) = &config.resume {
            cmd.args(["--resume", resume]);
            if config.fork {
                cmd.arg("--fork-session");
            }
        } else {
            cmd.args(["--session-id", &config.session_id]);
        }
        if !config.allowed_tools.is_empty() {
            cmd.args(["--allowedTools", &config.allowed_tools.join(" ")]);
        }
        if !config.disallowed_tools.is_empty() {
            cmd.args(["--disallowedTools", &config.disallowed_tools.join(" ")]);
        }
        if let Some(mcp) = &config.mcp_config {
            cmd.args(["--mcp-config", mcp]);
        }
        if let Some(dir) = &config.plugin_dir {
            cmd.arg("--plugin-dir").arg(dir);
        }
        for dir in &config.add_dirs {
            cmd.arg("--add-dir").arg(dir);
        }
        if let Some(budget) = config.max_budget_usd {
            cmd.args(["--max-budget-usd", &budget.to_string()]);
        }
        if let Some(model) = &config.model {
            cmd.args(["--model", model]);
        }
        if let Some(effort) = &config.effort {
            if effort == "ultracode" {
                cmd.args(["--effort", "xhigh"])
                    .args(["--settings", r#"{"ultracode":true,"enableWorkflows":true}"#]);
            } else {
                cmd.args(["--effort", effort]);
            }
        }
        if config.chrome {
            cmd.arg("--chrome");
        }
        if let Some(agent) = &config.agent {
            let mut def = serde_json::json!({
                "description": agent.description,
                "prompt": agent.prompt,
            });
            if !agent.disallowed_tools.is_empty() {
                def["disallowedTools"] = agent.disallowed_tools.into();
            }
            if let Some(hooks) = &agent.hooks {
                def["hooks"] = hooks.clone();
            }
            let defs = serde_json::json!({ agent.name: def });
            cmd.args(["--agents", &defs.to_string()])
                .args(["--agent", agent.name]);
        }
        if let Some(prompt) = &config.append_system_prompt {
            cmd.args(["--append-system-prompt", prompt]);
        }

        crate::process::appimage::scrub_tokio(&mut cmd);
        cmd.current_dir(&config.cwd)
            .env("CLAUDE_CODE_ENTRYPOINT", "hyprlayer-app")
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
                    name: "claude".into(),
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
        let pending_permissions = Arc::new(StdMutex::new(HashMap::<String, Value>::new()));
        let pending_controls = Arc::new(StdMutex::new(
            HashMap::<String, oneshot::Sender<Value>>::new(),
        ));
        let inbox = Arc::new(StdMutex::new(Inbox::default()));
        let cost_so_far = Arc::new(StdMutex::new(None));

        tokio::spawn(write_loop(stdin, stdin_rx));

        tokio::spawn(stderr_loop(stderr, stderr_ring.clone()));

        tokio::spawn(read_and_wait(
            child,
            stdout,
            events_tx.clone(),
            stderr_ring.clone(),
            kill_flag.clone(),
            pending_permissions.clone(),
            pending_controls.clone(),
            inbox.clone(),
            cost_so_far.clone(),
            stdin_tx.downgrade(),
        ));

        Ok((
            Self {
                session_id: config.session_id,
                stdin_tx,
                events: events_tx,
                control_seq: std::sync::atomic::AtomicU64::new(0),
                stderr_ring,
                kill_flag,
                pending_permissions,
                pending_controls,
                inbox,
                cost_so_far,
                budget_cap: config.max_budget_usd,
                killer: Killer { pid },
                claude_config_dir: config.claude_config_dir,
                cwd: config.cwd,
            },
            primary_rx,
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Mid-turn, claude takes the message in at its next step, as its own CLI
    /// does; the echo is emitted then, so it lands where the model read it.
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
        // Minted here: a client's id need not be the uuid claude expects.
        let wire = uuid::Uuid::new_v4().to_string();
        let line = protocol::user_turn_json(&self.session_id, text, &wire);
        let mut inbox = lock(&self.inbox);
        inbox.own.insert(wire.clone());
        // Without lifecycle frames nothing would say when claude took it in: it still
        // waits in claude's queue, but echoes now.
        if !inbox.idle() && inbox.lifecycle {
            inbox.waiting.push((wire.clone(), echo));
        } else {
            inbox.busy = true;
            let _ = self.events.send(echo);
        }
        let written = self.write_line(line);
        if written.is_err() {
            inbox.own.remove(&wire);
            inbox.take(&wire);
        }
        written
    }

    /// Withdraws a message still waiting for claude's next step; false once claude took it in.
    pub async fn cancel_queued_message(&self, client_message_id: &str) -> Result<bool> {
        let wire = {
            let mut inbox = lock(&self.inbox);
            let Some(wire) = inbox.wire_for(client_message_id) else {
                return Ok(false);
            };
            inbox.cancelling.insert(wire.clone());
            wire
        };
        let reply = self
            .control(
                "cancel_async_message",
                json!({ "message_uuid": wire }),
                CANCEL_TIMEOUT,
            )
            .await;
        let cancelled = reply
            .as_ref()
            .ok()
            .and_then(|payload| payload.get("cancelled"))
            .and_then(Value::as_bool)
            == Some(true);
        let mut inbox = lock(&self.inbox);
        if cancelled {
            inbox.take(&wire);
        } else {
            inbox.cancelling.remove(&wire);
        }
        reply.map(|_| cancelled)
    }

    pub fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        let updated_input = self
            .pending_permissions
            .lock()
            .ok()
            .and_then(|mut map| map.remove(request_id));
        if updated_input.is_some() {
            let _ = self.events.send(SessionEvent::PermissionResolved {
                request_id: request_id.to_string(),
                allow,
            });
        }
        self.write_line(protocol::permission_response_json(
            request_id,
            allow,
            updated_input,
            message,
        ))
    }

    pub fn pending_requests(&self) -> Vec<String> {
        self.pending_permissions
            .lock()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> Result<()> {
        let pending = self
            .pending_permissions
            .lock()
            .ok()
            .and_then(|mut map| map.remove(request_id));
        if pending.is_some() {
            let _ = self.events.send(SessionEvent::QuestionAnswered {
                request_id: request_id.to_string(),
            });
        }
        let mut input = pending.unwrap_or_else(|| json!({}));
        if let Value::Object(map) = &mut input {
            map.insert("answers".to_string(), json!(answers));
        }
        self.write_line(protocol::permission_response_json(
            request_id,
            true,
            Some(input),
            None,
        ))
    }

    pub fn interrupt(&self) -> Result<()> {
        let id = self.next_control_id();
        self.write_line(protocol::control_request_json(&id, "interrupt", json!({})))
    }

    pub fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        let id = self.next_control_id();
        self.write_line(protocol::control_request_json(
            &id,
            "set_permission_mode",
            json!({ "mode": permission_mode_flag(mode) }),
        ))
    }

    pub async fn set_remote_control(&self, enabled: bool, name: &str) -> Result<Option<String>> {
        let payload = self
            .control(
                "remote_control",
                json!({ "enabled": enabled, "name": name }),
                REMOTE_CONTROL_TIMEOUT,
            )
            .await?;
        if let Some(error) = payload.get("error").and_then(Value::as_str) {
            return Err(CoreError::OutputParse {
                program: "claude".into(),
                detail: format!("Remote Control: {error}"),
            });
        }
        let url = payload
            .get("session_url")
            .and_then(Value::as_str)
            .map(str::to_string);
        if enabled && url.is_none() {
            return Err(CoreError::OutputParse {
                program: "claude".into(),
                detail: "Remote Control started without a session URL".into(),
            });
        }
        Ok(url)
    }

    pub async fn rewind_to_ordinal(&self, ordinal: usize) -> Result<()> {
        let uuids = self.transcript_user_uuids().await;
        let start =
            rewind_local_start(ordinal, uuids.len()).map_err(|detail| CoreError::OutputParse {
                program: "claude".into(),
                detail,
            })?;
        for uuid in uuids[start..].iter().rev() {
            let outcome = self.rewind_conversation(uuid, true).await?;
            if !outcome.rewound {
                return Err(CoreError::OutputParse {
                    program: "claude".into(),
                    detail: format!(
                        "claude refused rewind_conversation: {}",
                        outcome.error.as_deref().unwrap_or("unknown")
                    ),
                });
            }
        }
        Ok(())
    }

    async fn transcript_user_uuids(&self) -> Vec<String> {
        let session_id = self.session_id.clone();
        let claude_config_dir = self.claude_config_dir.clone();
        let cwd = self.cwd.clone();
        tokio::task::spawn_blocking(move || {
            transcript_path(&session_id, claude_config_dir.as_deref(), Some(&cwd))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|c| parse_transcript_user_uuids(&c))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }

    async fn rewind_conversation(
        &self,
        target_uuid: &str,
        interrupt_if_running: bool,
    ) -> Result<RewindOutcome> {
        let payload = self
            .control(
                "rewind_conversation",
                json!({
                    "target_message_uuid": target_uuid,
                    "interrupt_if_running": interrupt_if_running,
                    "dry_run": false,
                }),
                REWIND_TIMEOUT,
            )
            .await?;
        Ok(RewindOutcome {
            rewound: payload
                .get("rewound")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error: payload
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
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

    pub fn usage_so_far(&self) -> UsageSoFar {
        UsageSoFar {
            cost_usd: self.cost_so_far.lock().ok().and_then(|cost| *cost),
            thread_tokens: None,
        }
    }

    pub fn budget_cap(&self) -> Option<f64> {
        self.budget_cap
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr_ring
            .lock()
            .map(|r| r.join("\n"))
            .unwrap_or_default()
    }

    /// A control request whose reply carries a payload.
    async fn control(&self, subtype: &str, extra: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_control_id();
        let (tx, rx) = oneshot::channel::<Value>();
        if let Ok(mut map) = self.pending_controls.lock() {
            map.insert(id.clone(), tx);
        }
        let forget = || {
            if let Ok(mut map) = self.pending_controls.lock() {
                map.remove(&id);
            }
        };
        if let Err(e) = self.write_line(protocol::control_request_json(&id, subtype, extra)) {
            forget();
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(payload)) => Ok(payload),
            _ => {
                forget();
                Err(CoreError::OutputParse {
                    program: "claude".into(),
                    detail: format!("{subtype} got no reply"),
                })
            }
        }
    }

    fn write_line(&self, mut line: String) -> Result<()> {
        line.push('\n');
        self.stdin_tx
            .send(line)
            .map_err(|_| CoreError::OutputParse {
                program: "claude".into(),
                detail: "session stdin is closed".into(),
            })
    }

    fn next_control_id(&self) -> String {
        let n = self
            .control_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("app-{}-{n}", self.session_id)
    }
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
    pending_permissions: Arc<StdMutex<HashMap<String, Value>>>,
    pending_controls: Arc<StdMutex<HashMap<String, oneshot::Sender<Value>>>>,
    inbox: Arc<StdMutex<Inbox>>,
    cost_so_far: Arc<StdMutex<Option<f64>>>,
    stdin: mpsc::WeakUnboundedSender<String>,
) {
    let mut saw_result = false;
    let mut saw_error_result = false;
    let mut model: Option<String> = None;
    let mut sent_window: Option<u32> = None;
    let mut lines = BufReader::new(stdout).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let parsed = protocol::parse_line(&line);
                match &parsed {
                    ParsedLine::Init { model: m, .. } => {
                        model = m.clone();
                        // Every turn opens with one, including those claude starts itself.
                        lock(&inbox).busy = true;
                    }
                    ParsedLine::CommandLifecycle {
                        command_uuid,
                        state,
                    } => {
                        let ours = lock(&inbox).frame(command_uuid, state);
                        if let Some(emit) = ours {
                            if let Some(event) = emit {
                                let _ = events.send(event);
                            }
                            continue;
                        }
                    }
                    ParsedLine::Result {
                        is_error,
                        context_windows,
                        total_cost_usd,
                        ..
                    } => {
                        lock(&inbox).busy = false;
                        saw_result = true;
                        saw_error_result |= *is_error;
                        if let (Some(cost), Ok(mut so_far)) = (total_cost_usd, cost_so_far.lock()) {
                            *so_far = Some(*cost);
                        }
                        if let Some(tokens) = context_windows
                            .iter()
                            .find(|(m, _)| Some(m) == model.as_ref())
                            .map(|(_, w)| *w)
                            .filter(|w| Some(*w) != sent_window)
                        {
                            sent_window = Some(tokens);
                            let _ = events.send(SessionEvent::ContextWindow { tokens });
                        }
                    }
                    ParsedLine::CanUseTool {
                        request_id, input, ..
                    } => {
                        if let Ok(mut map) = pending_permissions.lock() {
                            map.insert(request_id.clone(), input.clone());
                        }
                    }
                    ParsedLine::Elicitation { request_id } => {
                        if let Some(stdin) = stdin.upgrade() {
                            let _ =
                                stdin.send(protocol::elicitation_decline_json(request_id) + "\n");
                        }
                        continue;
                    }
                    ParsedLine::ControlCancel { request_id } => {
                        if let Ok(mut map) = pending_permissions.lock() {
                            map.remove(request_id);
                        }
                        continue;
                    }
                    ParsedLine::User {
                        is_replay: true, ..
                    } => continue,
                    ParsedLine::ControlResponse {
                        request_id,
                        payload,
                    } => {
                        if let Ok(mut map) = pending_controls.lock() {
                            if let Some(tx) = map.remove(request_id) {
                                let _ = tx.send(payload.clone());
                            }
                        }
                        continue;
                    }
                    _ => {}
                }
                for event in parsed_to_events(parsed) {
                    let _ = events.send(event);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    let status = child.wait().await;
    let code = status.as_ref().ok().and_then(|s| s.code());
    let stderr_tail = stderr_ring.lock().map(|r| r.join("\n")).unwrap_or_default();

    let outcome =
        kill_flag
            .outcome()
            .unwrap_or_else(|| match (&status, saw_result, saw_error_result) {
                (Ok(s), true, false) if s.success() => ExitOutcome::Completed,
                (Ok(s), _, _) => crate::process::exit::outcome_from_status(s),
                (Err(_), _, _) => ExitOutcome::SpawnFailed,
            });

    let _ = events.send(SessionEvent::Exited {
        outcome,
        code,
        stderr_tail,
    });
}

pub(crate) fn parsed_to_events(parsed: ParsedLine) -> Vec<SessionEvent> {
    match parsed {
        ParsedLine::Init {
            claude_session_id,
            model,
            mcp_servers,
            ..
        } => vec![SessionEvent::Init {
            provider_session_id: claude_session_id.unwrap_or_default(),
            model,
            mcp_servers: Some(mcp_servers),
        }],
        ParsedLine::PartialText { text, parent_id } => {
            vec![SessionEvent::PartialText { text, parent_id }]
        }
        ParsedLine::Assistant {
            blocks,
            context_tokens,
            parent_id,
            error,
        } => {
            let auth_failed =
                parent_id.is_none() && error.as_deref() == Some("authentication_failed");
            context_tokens
                .filter(|_| parent_id.is_none())
                .map(|t| SessionEvent::Usage {
                    context_tokens: t,
                    thread_tokens: None,
                })
                .into_iter()
                .chain(
                    blocks
                        .into_iter()
                        .filter_map(|b| block_to_event(b, parent_id.as_deref())),
                )
                .map(|event| match event {
                    SessionEvent::AssistantText {
                        text,
                        parent_id,
                        injected,
                        ..
                    } if auth_failed => SessionEvent::AssistantText {
                        text,
                        parent_id,
                        injected,
                        auth_required: true,
                    },
                    other => other,
                })
                .collect()
        }
        ParsedLine::User {
            blocks,
            parent_id,
            is_synthetic,
            ..
        } => blocks
            .into_iter()
            .filter_map(|b| block_to_event(b, parent_id.as_deref()))
            .map(|event| match event {
                SessionEvent::AssistantText {
                    text, parent_id, ..
                } if is_synthetic => SessionEvent::AssistantText {
                    text,
                    parent_id,
                    injected: true,
                    auth_required: false,
                },
                other => other,
            })
            .collect(),
        ParsedLine::CanUseTool {
            request_id,
            tool_name,
            input,
            tool_use_id,
        } => {
            let questions = (tool_name == "AskUserQuestion")
                .then(|| input.get("questions"))
                .flatten()
                .filter(|q| q.as_array().is_some_and(|a| !a.is_empty()));
            match questions {
                Some(questions) => vec![SessionEvent::QuestionAsk {
                    request_id,
                    questions: questions_from_value(Some(questions)),
                    subject: question_subject(&input),
                }],
                None => vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name,
                    input: input.to_string(),
                    tool_use_id,
                }],
            }
        }
        ParsedLine::Result {
            is_error,
            subtype,
            terminal_reason,
            total_cost_usd,
            num_turns,
            context_tokens,
            ..
        } => vec![SessionEvent::TurnDone {
            cost_usd: total_cost_usd,
            num_turns,
            context_tokens,
            turn_status: Some(
                claude_turn_status(is_error, subtype.as_deref(), terminal_reason.as_deref()).into(),
            ),
            thread_tokens: None,
        }],
        // A manual /compact's result carries no usage, so the boundary is the
        // only report of the compacted context.
        ParsedLine::CompactBoundary {
            trigger,
            pre_tokens,
            post_tokens,
        } => std::iter::once(SessionEvent::CompactBoundary {
            trigger,
            pre_tokens,
        })
        .chain(post_tokens.map(|t| SessionEvent::Usage {
            context_tokens: t,
            thread_tokens: None,
        }))
        .collect(),
        ParsedLine::TaskNotification {
            task_id,
            tool_use_id,
            status,
        } => vec![SessionEvent::BackgroundTaskUpdate {
            task_id,
            tool_use_id,
            status,
        }],
        ParsedLine::CommandLifecycle {
            command_uuid,
            state,
        } => vec![SessionEvent::CommandLifecycle {
            command_uuid,
            state,
        }],
        ParsedLine::Unknown { raw } => vec![SessionEvent::ProtocolError {
            detail: "unrecognized stream-json line".into(),
            raw,
        }],
        ParsedLine::PermissionMode { mode } => vec![SessionEvent::PermissionModeChanged { mode }],
        ParsedLine::ControlResponse { .. }
        | ParsedLine::ControlCancel { .. }
        | ParsedLine::Elicitation { .. } => Vec::new(),
        ParsedLine::System { .. } => Vec::new(),
    }
}

// Models send `metadata` as an object or as a JSON string of one.
fn question_subject(input: &Value) -> Option<QuestionSubject> {
    let decoded: Value;
    let metadata = match input.get("metadata")? {
        Value::String(raw) => {
            decoded = serde_json::from_str(raw).ok()?;
            &decoded
        }
        other => other,
    };
    let (kind, id) = metadata.get("source")?.as_str()?.split_once(':')?;
    let workstream_id = id.trim();
    (kind.trim() == "workstream" && !workstream_id.is_empty()).then(|| QuestionSubject {
        workstream_id: workstream_id.to_string(),
        archive: false,
    })
}

fn claude_turn_status(
    is_error: bool,
    subtype: Option<&str>,
    terminal: Option<&str>,
) -> &'static str {
    match (subtype, terminal) {
        // A stop mid-tool ends on `aborted_tools`, with `is_error` set.
        (_, Some("aborted_streaming" | "aborted_tools")) => "interrupted",
        (Some("error_max_budget_usd"), _) => "budget_exhausted",
        _ if is_error => "failed",
        _ => "completed",
    }
}

pub(crate) fn block_to_event(block: Block, parent_id: Option<&str>) -> Option<SessionEvent> {
    let parent_id = parent_id.map(str::to_string);
    match block {
        Block::Text { text } => Some(SessionEvent::AssistantText {
            text,
            parent_id,
            injected: false,
            auth_required: false,
        }),
        Block::Thinking { text } => Some(SessionEvent::Thinking { text, parent_id }),
        Block::ToolUse { id, name, input } => Some(SessionEvent::ToolUse {
            id,
            name,
            input: input.to_string(),
            parent_id,
        }),
        Block::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(SessionEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
            parent_id,
        }),
        Block::Other { .. } => None,
    }
}

fn rewind_local_start(ordinal: usize, len: usize) -> std::result::Result<usize, String> {
    if ordinal < 2 {
        return Err("rewind_conversation cannot target the first user turn".into());
    }
    let idx = ordinal - 1;
    if idx >= len {
        return Err(format!(
            "no uuid for user turn {ordinal} (transcript has {len} turns)"
        ));
    }
    Ok(idx)
}

/// Prefers the copy under `cwd`'s project key: claude appends there, and a
/// restore to a new path leaves a stale copy under the old key.
pub(super) fn transcript_path(
    session_id: &str,
    claude_config_dir: Option<&Path>,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let projects = claude_config_dir?.join("projects");
    let filename = format!("{session_id}.jsonl");
    if let Some(cwd) = cwd {
        let keyed = keyed_transcript(&projects, cwd, &filename);
        if keyed.is_file() {
            return Some(keyed);
        }
    }
    for entry in std::fs::read_dir(&projects).ok()?.flatten() {
        let candidate = entry.path().join(&filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn keyed_transcript(projects: &Path, cwd: &Path, filename: &str) -> PathBuf {
    projects
        .join(super::memory::project_key(&super::memory::canonical(cwd)))
        .join(filename)
}

/// Copies a session's transcript under `cwd`'s project key, where
/// `claude --resume` looks for it; returns whether anything was copied.
pub fn relocate_transcript(
    claude_config_dir: &Path,
    session_id: &str,
    cwd: &Path,
) -> std::io::Result<bool> {
    let projects = claude_config_dir.join("projects");
    let dest = keyed_transcript(&projects, cwd, &format!("{session_id}.jsonl"));
    if dest.exists() {
        return Ok(false);
    }
    let Some(source) = transcript_path(session_id, Some(claude_config_dir), None) else {
        return Ok(false);
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, &dest)?;
    Ok(true)
}

/// The uuids of the app's user turns, in order: what a rewind ordinal indexes.
fn parse_transcript_user_uuids(jsonl: &str) -> Vec<String> {
    jsonl
        .lines()
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line.trim()).ok()?;
            let flag = |k| v.get(k).and_then(Value::as_bool).unwrap_or(false);
            if flag("isMeta")
                || flag("isCompactSummary")
                || flag("isVisibleInTranscriptOnly")
                || flag("isSidechain")
            {
                return None;
            }
            let human = |origin: Option<&Value>| {
                origin
                    .and_then(|o| o.get("kind"))
                    .and_then(Value::as_str)
                    .is_none_or(|kind| kind == "human")
            };
            match v.get("type").and_then(Value::as_str)? {
                "user" => {
                    let content = v.get("message").and_then(|m| m.get("content"));
                    let first = content.and_then(Value::as_array).and_then(|a| a.first());
                    let block = |key| first.and_then(|b| b.get(key)).and_then(Value::as_str);
                    let text = content.and_then(Value::as_str).or_else(|| block("text"));
                    // The interrupt marker reaches the app as assistant text, not a user turn.
                    let marker = text.is_some_and(|t| t.starts_with(INTERRUPT_MARKER));
                    if block("type") == Some("tool_result") || marker || !human(v.get("origin")) {
                        return None;
                    }
                }
                // A message sent mid-turn joins the turn as an attachment, not a user line.
                "attachment" => {
                    let queued = v.get("attachment")?;
                    let field = |key| queued.get(key).and_then(Value::as_str);
                    if field("type") != Some("queued_command")
                        || field("commandMode") != Some("prompt")
                        || !human(queued.get("origin"))
                    {
                        return None;
                    }
                }
                _ => return None,
            }
            v.get("uuid").and_then(Value::as_str).map(str::to_string)
        })
        .collect()
}

const INTERRUPT_MARKER: &str = "[Request interrupted by user";

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
    use serde_json::json;

    #[test]
    fn rewind_maps_ordinal_to_index() {
        assert_eq!(rewind_local_start(2, 5), Ok(1));
        assert_eq!(rewind_local_start(5, 5), Ok(4));
        assert_eq!(rewind_local_start(3, 7), Ok(2));
        assert!(rewind_local_start(1, 5).is_err());
        assert!(rewind_local_start(6, 5).is_err());
    }

    #[test]
    fn rewind_empty_transcript_reaches_nothing() {
        for ordinal in 1..=8 {
            assert!(rewind_local_start(ordinal, 0).is_err());
        }
    }

    #[test]
    fn parse_transcript_keeps_only_human_turns() {
        let jsonl = [
            r#"{"type":"system","subtype":"init","uuid":"sys"}"#,
            r#"{"type":"user","uuid":"u1","message":{"role":"user","content":"first turn"}}"#,
            r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"user","uuid":"tr","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#,
            r#"{"type":"user","uuid":"u2","message":{"role":"user","content":"second turn"}}"#,
            r#"{"type":"user","uuid":"meta","isMeta":true,"message":{"content":"injected"}}"#,
            r#"{"type":"user","uuid":"side","isSidechain":true,"message":{"content":"subagent"}}"#,
            r#"{"type":"user","uuid":"nonhuman","origin":{"kind":"agent"},"message":{"content":"x"}}"#,
            r#"{"type":"user","uuid":"u3","message":{"role":"user","content":"third turn"}}"#,
        ]
        .join("\n");
        assert_eq!(parse_transcript_user_uuids(&jsonl), vec!["u1", "u2", "u3"]);
    }

    #[test]
    fn parse_transcript_tolerates_junk_lines() {
        let jsonl = "\nnot json\n{}\n{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(parse_transcript_user_uuids(jsonl), vec!["u1"]);
    }

    #[test]
    fn ask_user_question_becomes_a_question_ask() {
        let questions = json!([{
            "question": "Tabs or spaces?",
            "header": "Indentation",
            "multiSelect": false,
            "options": [
                { "label": "Tabs", "description": "Tab characters." },
                { "label": "Spaces", "description": "Space characters." }
            ]
        }]);
        let events = parsed_to_events(ParsedLine::CanUseTool {
            request_id: "req-1".into(),
            tool_name: "AskUserQuestion".into(),
            input: json!({ "questions": questions }),
            tool_use_id: None,
        });
        match events.as_slice() {
            [SessionEvent::QuestionAsk {
                request_id,
                questions: q,
                subject: None,
            }] => {
                assert_eq!(request_id, "req-1");
                assert_eq!(serde_json::to_value(q).unwrap(), questions);
            }
            other => panic!("expected one QuestionAsk, got {other:?}"),
        }
    }

    #[test]
    fn ask_user_question_metadata_source_names_its_subject() {
        let subject = |id: &str| {
            Some(QuestionSubject {
                workstream_id: id.into(),
                archive: false,
            })
        };
        for (metadata, expected) in [
            (json!({ "source": "workstream:ws-461" }), subject("ws-461")),
            (json!(r#"{"source":"workstream:ws-7"}"#), subject("ws-7")),
            (json!({ "source": "workstream: ws-2 " }), subject("ws-2")),
            (json!({ "source": "archive:ws-461" }), None),
            (json!({ "source": "workstream:" }), None),
            (json!({ "source": "merge:ws-461" }), None),
            (json!({ "source": "ws-461" }), None),
            (json!({ "source": 461 }), None),
            (json!({}), None),
            (json!("not json"), None),
            (Value::Null, None),
        ] {
            let events = parsed_to_events(ParsedLine::CanUseTool {
                request_id: "req-1".into(),
                tool_name: "AskUserQuestion".into(),
                input: json!({
                    "questions": [{ "question": "Archive it?", "options": [] }],
                    "metadata": metadata,
                }),
                tool_use_id: None,
            });
            match events.as_slice() {
                [SessionEvent::QuestionAsk { subject, .. }] => {
                    assert_eq!(subject, &expected, "metadata {metadata}")
                }
                other => panic!("expected one QuestionAsk, got {other:?}"),
            }
        }
    }

    #[test]
    fn synthetic_user_text_is_flagged_injected() {
        let events = parsed_to_events(ParsedLine::User {
            blocks: vec![Block::Text {
                text: "Approach this as the design lead".into(),
            }],
            parent_id: None,
            uuid: None,
            is_replay: false,
            is_synthetic: true,
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::AssistantText { injected: true, .. }]
        ));
    }

    #[test]
    fn ordinary_user_text_is_not_injected() {
        let events = parsed_to_events(ParsedLine::User {
            blocks: vec![Block::Text {
                text: "plain".into(),
            }],
            parent_id: None,
            uuid: None,
            is_replay: false,
            is_synthetic: false,
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::AssistantText {
                injected: false,
                auth_required: false,
                ..
            }]
        ));
    }

    #[test]
    fn other_tools_remain_permission_asks() {
        let events = parsed_to_events(ParsedLine::CanUseTool {
            request_id: "req-2".into(),
            tool_name: "Bash".into(),
            input: json!({ "command": "ls" }),
            tool_use_id: None,
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::PermissionAsk { tool_name, .. }] if tool_name == "Bash"
        ));
    }

    #[test]
    fn task_notification_becomes_a_background_task_update() {
        let events = parsed_to_events(ParsedLine::TaskNotification {
            task_id: "bxmc1fof8".into(),
            tool_use_id: Some("toolu_01HgG".into()),
            status: "completed".into(),
        });
        match events.as_slice() {
            [SessionEvent::BackgroundTaskUpdate {
                task_id,
                tool_use_id,
                status,
            }] => {
                assert_eq!(task_id, "bxmc1fof8");
                assert_eq!(tool_use_id.as_deref(), Some("toolu_01HgG"));
                assert_eq!(status, "completed");
            }
            other => panic!("expected one BackgroundTaskUpdate, got {other:?}"),
        }
    }

    #[test]
    fn ask_user_question_with_bad_questions_falls_back_to_permission() {
        for bad in [
            json!({}),
            json!({ "questions": null }),
            json!({ "questions": "nope" }),
            json!({ "questions": [] }),
        ] {
            let events = parsed_to_events(ParsedLine::CanUseTool {
                request_id: "req-3".into(),
                tool_name: "AskUserQuestion".into(),
                input: bad.clone(),
                tool_use_id: None,
            });
            assert!(
                matches!(
                    events.as_slice(),
                    [SessionEvent::PermissionAsk { tool_name, .. }] if tool_name == "AskUserQuestion"
                ),
                "expected PermissionAsk fallback for input {bad}"
            );
        }
    }
}
