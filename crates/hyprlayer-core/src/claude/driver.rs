//! The long-lived `claude` session driver.
//!
//! Unlike one-shot [`crate::process::Exec`], a session is a long-running child
//! we talk to over stdin/stdout for its whole life. [`ClaudeDriver::spawn`]
//! starts the process and three background tasks:
//!
//! * **reader** — `claude` stdout NDJSON → [`SessionEvent`]s, broadcast to
//!   subscribers (the UI Channel and the persistence writer both attach).
//! * **stderr** — drained into a small ring buffer, so a crash can report the
//!   tail (e.g. a `/login` hint).
//! * **waiter** — `child.wait()` → classify the exit → final `Exited` event.
//!
//! Turns and control responses are written through an mpsc queue so writes from
//! different callers serialize onto one stdin without interleaving.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::events::{ExitOutcome, SessionEvent};
use super::protocol::{self, Block, ParsedLine};
use crate::error::{CoreError, Result};

/// Per-stage permission posture (maps to claude's `--permission-mode`). Also
/// the wire type for a user-chosen override (serialized as the CLI's own
/// camelCase flag strings, which the frontend already speaks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    BypassPermissions,
}

impl PermissionMode {
    fn as_flag(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Plan => "plan",
            PermissionMode::BypassPermissions => "bypassPermissions",
        }
    }

    /// Parse a `--permission-mode` value (the same strings claude's CLI accepts),
    /// or `None` for anything unrecognized. Used to apply a user-chosen override
    /// on top of the stage's default posture; an unknown value falls back to the
    /// stage default at the call site.
    pub fn from_flag(s: &str) -> Option<PermissionMode> {
        match s {
            "default" => Some(PermissionMode::Default),
            "acceptEdits" => Some(PermissionMode::AcceptEdits),
            "plan" => Some(PermissionMode::Plan),
            "bypassPermissions" => Some(PermissionMode::BypassPermissions),
            _ => None,
        }
    }
}

/// Everything needed to launch one session.
pub struct SpawnConfig {
    /// Absolute path to the resolved `claude` binary.
    pub claude_bin: PathBuf,
    /// Working directory — the workstream's worktree.
    pub cwd: PathBuf,
    /// App-generated session id (so resume bookkeeping survives a crash before
    /// the init event lands).
    pub session_id: String,
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    /// Extra read/write roots (e.g. the thoughts root outside the worktree).
    pub add_dirs: Vec<PathBuf>,
    /// `--resume <claude_session_id>` to continue; cwd must match the original.
    pub resume: Option<String>,
    /// Branch a resumed session instead of continuing it.
    pub fork: bool,
    /// Cost circuit-breaker.
    pub max_budget_usd: Option<f64>,
    /// Override the model (else claude's default).
    pub model: Option<String>,
    /// Reasoning effort level (`low|medium|high|xhigh|max`); else claude's default.
    pub effort: Option<String>,
    /// Enable the "Claude in Chrome" browser-control feature (`--chrome`).
    pub chrome: bool,
    /// Augmented PATH + any extra env for the child (e.g. `ANTHROPIC_API_KEY`
    /// for enterprise API-key auth).
    pub env: Vec<(String, String)>,
}

const STDERR_RING_LINES: usize = 200;
const EVENT_CHANNEL_CAP: usize = 1024;

/// The result of one `rewind_conversation` control request. `rewound` is claude's
/// own success flag; `error` carries its refusal reason ("stale target", "turn
/// running", "no preceding assistant", …) when it declined.
struct RewindOutcome {
    rewound: bool,
    error: Option<String>,
}

/// A handle to a live session. Dropping it does NOT kill the child (the waiter
/// task owns it); call [`ClaudeDriver::kill`] for a hard stop.
pub struct ClaudeDriver {
    session_id: String,
    /// Serialized writes to the child's stdin (turns + control responses).
    stdin_tx: mpsc::UnboundedSender<String>,
    /// New subscribers clone this to receive the event stream.
    events: broadcast::Sender<SessionEvent>,
    /// Monotonic counter for app-initiated control requests.
    control_seq: std::sync::atomic::AtomicU64,
    /// Last 200 stderr lines (for crash diagnostics).
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    /// Set when `kill` is called, so the waiter classifies the exit as an
    /// app-initiated `Killed` rather than an unexpected `Crashed`.
    killed: Arc<AtomicBool>,
    /// Tool inputs from open `can_use_tool` requests, keyed by request id —
    /// echoed back as `updatedInput` when allowing (claude's schema requires it).
    pending_permissions: Arc<StdMutex<HashMap<String, Value>>>,
    /// Control requests awaiting a `control_response`, keyed by request id (just
    /// `rewind_conversation` today). The reader resolves the oneshot on reply.
    pending_controls: Arc<StdMutex<HashMap<String, oneshot::Sender<Value>>>>,
    /// Best-effort hard-kill hook (pid / process group).
    killer: Killer,
}

impl ClaudeDriver {
    /// Spawn a session. Returns the driver plus a receiver that is guaranteed to
    /// observe every event (it's subscribed before the reader task can
    /// broadcast, so the `init` event can't be missed). Additional subscribers
    /// (re-attach) use [`ClaudeDriver::subscribe`] and replay from SQLite for
    /// anything before they joined.
    pub fn spawn(config: SpawnConfig) -> Result<(Self, broadcast::Receiver<SessionEvent>)> {
        let mut cmd = Command::new(&config.claude_bin);
        cmd.arg("-p")
            .args(["--output-format", "stream-json"])
            .args(["--input-format", "stream-json"])
            .arg("--verbose")
            .arg("--include-partial-messages")
            .args(["--permission-prompt-tool", "stdio"])
            .args(["--permission-mode", config.permission_mode.as_flag()]);

        // Fresh sessions pin our generated id; resuming continues an existing
        // conversation by id (claude rejects --session-id together with --resume).
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
                // "ultracode" is our composer preset, not a CLI --effort value
                // (the CLI rejects `--effort ultracode`). It means xhigh effort
                // plus standing dynamic-workflow orchestration, which the CLI
                // enables via the `ultracode` settings key. The --effort flag wins
                // over the settings-implied xhigh, so xhigh is passed explicitly;
                // enableWorkflows pins the workflows feature on. --settings layers
                // over the user's settings, leaving all other keys untouched.
                cmd.args(["--effort", "xhigh"]).args([
                    "--settings",
                    r#"{"ultracode":true,"enableWorkflows":true}"#,
                ]);
            } else {
                cmd.args(["--effort", effort]);
            }
        }
        if config.chrome {
            cmd.arg("--chrome");
        }

        cmd.current_dir(&config.cwd)
            .env("CLAUDE_CODE_ENTRYPOINT", "hyprlayer-app")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        // Unix: own process group so we can signal the whole tree (claude spawns
        // tool subprocesses). Windows job-object isolation is a follow-up.
        #[cfg(unix)]
        cmd.process_group(0);

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::BinaryMissing {
                    name: "claude".into(),
                }
            } else {
                CoreError::Io(e)
            }
        })?;

        let pid = child.id();
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAP);
        // Subscribe BEFORE the reader task is spawned: the returned receiver
        // can't miss the init event to a startup race.
        let primary_rx = events_tx.subscribe();
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        let stderr_ring = Arc::new(StdMutex::new(Vec::<String>::new()));
        let killed = Arc::new(AtomicBool::new(false));
        let pending_permissions = Arc::new(StdMutex::new(HashMap::<String, Value>::new()));
        let pending_controls =
            Arc::new(StdMutex::new(HashMap::<String, oneshot::Sender<Value>>::new()));

        // Writer task: drain the queue onto stdin, one line per message.
        tokio::spawn(write_loop(stdin, stdin_rx));

        // Stderr task: keep the last N lines.
        tokio::spawn(stderr_loop(stderr, stderr_ring.clone()));

        // Reader + waiter: parse stdout into events, then classify the exit.
        tokio::spawn(read_and_wait(
            child,
            stdout,
            events_tx.clone(),
            stderr_ring.clone(),
            killed.clone(),
            pending_permissions.clone(),
            pending_controls.clone(),
        ));

        Ok((
            Self {
                session_id: config.session_id,
                stdin_tx,
                events: events_tx,
                control_seq: std::sync::atomic::AtomicU64::new(0),
                stderr_ring,
                killed,
                pending_permissions,
                pending_controls,
                killer: Killer { pid },
            },
            primary_rx,
        ))
    }

    /// Subscribe to the event stream. Late subscribers miss earlier events —
    /// the persistence layer attaches immediately and replays from SQLite.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Send a user turn (the first prompt, or a follow-up). Stage-skill
    /// invocations are just prompt text (e.g. `"/research_codebase …"`). `stage`
    /// is the stage label for the opening prompt, else `None` for a follow-up.
    ///
    /// Emits a [`SessionEvent::UserMessage`] first so the turn is persisted and
    /// replays after a restart — claude never echoes the human prompt on stdout,
    /// so without this the user's side of the conversation is lost on reload.
    /// `prompt_name` tags the turn as a custom-prompt run so the replayed
    /// transcript collapses the body into a card instead of a wall of text.
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
        self.write_line(protocol::user_turn_json(&self.session_id, text))
    }

    /// Answer a `PermissionAsk`. On allow, claude's schema requires the tool's
    /// `updatedInput` back — we echo the original input we cached from the
    /// `can_use_tool` request (unchanged); on deny it requires a message.
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
        self.write_line(protocol::permission_response_json(
            request_id,
            allow,
            updated_input,
            message,
        ))
    }

    /// Answer an `AskUserQuestion` prompt. claude's AskUserQuestion executor
    /// reads the user's choices from `updatedInput.answers` (a map of question
    /// text → selected label(s), comma-joined for multi-select), so we merge
    /// `answers` into the cached tool input and allow. An empty `answers` map is
    /// claude's "the user didn't answer" signal (it then asks again in prose).
    pub fn answer_question(&self, request_id: &str, answers: Value) -> Result<()> {
        let mut input = self
            .pending_permissions
            .lock()
            .ok()
            .and_then(|mut map| map.remove(request_id))
            .unwrap_or_else(|| json!({}));
        if let Value::Object(map) = &mut input {
            map.insert("answers".to_string(), answers);
        }
        self.write_line(protocol::permission_response_json(
            request_id,
            true,
            Some(input),
            None,
        ))
    }

    /// Graceful interrupt via the control protocol (portable; the process keeps
    /// its pid). Falls back to a hard kill if claude doesn't ack.
    pub fn interrupt(&self) -> Result<()> {
        let id = self.next_control_id();
        self.write_line(protocol::control_request_json(&id, "interrupt", json!({})))
    }

    /// Switch permission posture mid-session (e.g. plan → acceptEdits).
    pub fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        let id = self.next_control_id();
        self.write_line(protocol::control_request_json(
            &id,
            "set_permission_mode",
            json!({ "mode": mode.as_flag() }),
        ))
    }

    /// Rewind claude's OWN conversation to the 1-based `ordinal` human turn so that
    /// turn and everything after it leave the model's memory. claude's
    /// `rewind_conversation` only accepts the *latest* human turn as a target, so we
    /// peel from the tail down to the target; each step is the current last turn, so
    /// none is "stale".
    ///
    /// The target uuids come from claude's own on-disk transcript — the SAME record
    /// it reloads on `--resume` — so `ordinal` maps directly onto index `ordinal - 1`
    /// and works across resumes (an app restart, model/permission switch, or a prior
    /// log-only rewind), not just the current generation. Any miss — the transcript
    /// isn't found/readable, the first turn (no preceding assistant to land on), a
    /// running turn that won't interrupt, or a claude refusal — leaves the
    /// conversation untouched and lets the caller fall back to a log-only rewind.
    pub async fn rewind_to_ordinal(&self, ordinal: usize) -> Result<()> {
        let uuids = self.transcript_user_uuids().await;
        // Resolve which uuid to peel down to (pure; unit-tested). An unreachable
        // target leaves the conversation untouched and drops the caller to a
        // log-only rewind.
        let start = rewind_local_start(ordinal, uuids.len()).map_err(|detail| {
            CoreError::OutputParse {
                program: "claude".into(),
                detail,
            }
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

    /// This session's human-turn uuids, in order, read from claude's transcript
    /// (`~/.claude/projects/*/<session_id>.jsonl`). Runs the blocking file read off
    /// the async runtime. Empty on any miss (not found, unreadable, unparseable), so
    /// [`Self::rewind_to_ordinal`] falls back to a log-only rewind rather than error.
    async fn transcript_user_uuids(&self) -> Vec<String> {
        let session_id = self.session_id.clone();
        tokio::task::spawn_blocking(move || {
            transcript_path(&session_id)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|c| parse_transcript_user_uuids(&c))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }

    /// Send one `rewind_conversation` control request and await its reply. The
    /// reader resolves the oneshot keyed by our request id (see [`read_and_wait`]).
    async fn rewind_conversation(
        &self,
        target_uuid: &str,
        interrupt_if_running: bool,
    ) -> Result<RewindOutcome> {
        let id = self.next_control_id();
        let (tx, rx) = oneshot::channel::<Value>();
        if let Ok(mut map) = self.pending_controls.lock() {
            map.insert(id.clone(), tx);
        }
        // Reclaim the pending slot if the write fails (stdin closed) — otherwise the
        // sender lingers forever, since no reply will ever arrive to remove it.
        if let Err(e) = self.write_line(protocol::control_request_json(
            &id,
            "rewind_conversation",
            json!({
                "target_message_uuid": target_uuid,
                "interrupt_if_running": interrupt_if_running,
                "dry_run": false,
            }),
        )) {
            if let Ok(mut map) = self.pending_controls.lock() {
                map.remove(&id);
            }
            return Err(e);
        }
        let payload = match tokio::time::timeout(std::time::Duration::from_secs(20), rx).await {
            Ok(Ok(v)) => v,
            // Timed out or the reader dropped the sender (process exited): reclaim
            // the slot and report so the caller falls back.
            _ => {
                if let Ok(mut map) = self.pending_controls.lock() {
                    map.remove(&id);
                }
                return Err(CoreError::OutputParse {
                    program: "claude".into(),
                    detail: "rewind_conversation got no reply".into(),
                });
            }
        };
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

    /// Hard stop: kill the process (group on Unix). The waiter task observes the
    /// exit and emits the terminal event (classified `Killed`).
    pub fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        self.killer.kill();
    }

    /// The child's OS process id, if it spawned (recorded in the orphan ledger).
    pub fn pid(&self) -> Option<u32> {
        self.killer.pid
    }

    /// Snapshot of the captured stderr tail (crash diagnostics).
    pub fn stderr_tail(&self) -> String {
        self.stderr_ring
            .lock()
            .map(|r| r.join("\n"))
            .unwrap_or_default()
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
            break; // child closed stdin; nothing more we can do
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
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    events: broadcast::Sender<SessionEvent>,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    killed: Arc<AtomicBool>,
    pending_permissions: Arc<StdMutex<HashMap<String, Value>>>,
    pending_controls: Arc<StdMutex<HashMap<String, oneshot::Sender<Value>>>>,
) {
    let mut saw_result = false;
    let mut saw_error_result = false;
    let mut lines = BufReader::new(stdout).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let parsed = protocol::parse_line(&line);
                match &parsed {
                    ParsedLine::Result { is_error, .. } => {
                        saw_result = true;
                        saw_error_result |= *is_error;
                    }
                    // Stash the tool input so an allow can echo it as updatedInput.
                    ParsedLine::CanUseTool {
                        request_id, input, ..
                    } => {
                        if let Ok(mut map) = pending_permissions.lock() {
                            map.insert(request_id.clone(), input.clone());
                        }
                    }
                    // Defensive: suppress any replayed user echo (we no longer run
                    // with `--replay-user-messages`, but a stray echo is a
                    // re-emission of a prompt we sent, not new content — never render
                    // it as text).
                    ParsedLine::User {
                        is_replay: true, ..
                    } => continue,
                    // A reply to one of our control requests: hand it to the waiter
                    // keyed by request id (if any), then drop it — not a transcript
                    // event. Unmatched acks (e.g. interrupt) simply have no waiter.
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
            Ok(None) => break, // EOF: stdout closed, process is winding down
            Err(_) => break,   // read error: treat as EOF
        }
    }

    let status = child.wait().await;
    let code = status.as_ref().ok().and_then(|s| s.code());
    let stderr_tail = stderr_ring.lock().map(|r| r.join("\n")).unwrap_or_default();

    // Classify: an app-initiated kill takes precedence; otherwise a clean
    // terminal result + exit 0 is Completed and anything else is a crash.
    let outcome = if killed.load(Ordering::SeqCst) {
        ExitOutcome::Killed
    } else {
        match (&status, saw_result, saw_error_result) {
            (Ok(s), true, false) if s.success() => ExitOutcome::Completed,
            (Ok(_), _, _) => ExitOutcome::Crashed,
            (Err(_), _, _) => ExitOutcome::SpawnFailed,
        }
    };

    let _ = events.send(SessionEvent::Exited {
        outcome,
        code,
        stderr_tail,
    });
}

/// Translate one parsed stdout line into zero or more session events.
fn parsed_to_events(parsed: ParsedLine) -> Vec<SessionEvent> {
    match parsed {
        ParsedLine::Init {
            claude_session_id,
            model,
            ..
        } => vec![SessionEvent::Init {
            claude_session_id: claude_session_id.unwrap_or_default(),
            model,
        }],
        ParsedLine::PartialText { text, parent_id } => {
            vec![SessionEvent::PartialText { text, parent_id }]
        }
        ParsedLine::Assistant {
            blocks,
            context_tokens,
            parent_id,
        } => {
            // A live usage ping (main thread only — subagent usage is separate
            // and shouldn't move the session's context meter), then the blocks.
            context_tokens
                .filter(|_| parent_id.is_none())
                .map(|t| SessionEvent::Usage { context_tokens: t })
                .into_iter()
                .chain(
                    blocks
                        .into_iter()
                        .filter_map(|b| block_to_event(b, parent_id.as_deref())),
                )
                .collect()
        }
        ParsedLine::User {
            blocks, parent_id, ..
        } => blocks
            .into_iter()
            .filter_map(|b| block_to_event(b, parent_id.as_deref()))
            .collect(),
        ParsedLine::CanUseTool {
            request_id,
            tool_name,
            input,
        } => {
            // AskUserQuestion isn't a yes/no gate — surface its questions so the
            // UI can render them interactively (answered via `answer_question`).
            // Fall back to a normal permission card if `questions` is missing or
            // not a non-empty array (a malformed shape we can't render as a prompt).
            let questions = (tool_name == "AskUserQuestion")
                .then(|| input.get("questions"))
                .flatten()
                .filter(|q| q.as_array().is_some_and(|a| !a.is_empty()));
            match questions {
                Some(questions) => vec![SessionEvent::QuestionAsk {
                    request_id,
                    questions: questions.to_string(),
                }],
                None => vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name,
                    input: input.to_string(),
                }],
            }
        }
        ParsedLine::Result {
            total_cost_usd,
            num_turns,
            context_tokens,
            ..
        } => vec![SessionEvent::TurnDone {
            cost_usd: total_cost_usd,
            num_turns,
            context_tokens,
        }],
        ParsedLine::CompactBoundary {
            trigger,
            pre_tokens,
        } => {
            vec![SessionEvent::CompactBoundary {
                trigger,
                pre_tokens,
            }]
        }
        ParsedLine::TaskNotification {
            task_id,
            tool_use_id,
            status,
        } => vec![SessionEvent::BackgroundTaskUpdate {
            task_id,
            tool_use_id,
            status,
        }],
        ParsedLine::Unknown { raw } => vec![SessionEvent::ProtocolError {
            detail: "unrecognized stream-json line".into(),
            raw,
        }],
        // Replies to our control requests are routed by the reader (keyed by
        // request id), never surfaced as transcript events.
        ParsedLine::ControlResponse { .. } => Vec::new(),
        ParsedLine::System { .. } => Vec::new(),
    }
}

fn block_to_event(block: Block, parent_id: Option<&str>) -> Option<SessionEvent> {
    let parent_id = parent_id.map(str::to_string);
    match block {
        Block::Text { text } => Some(SessionEvent::AssistantText { text, parent_id }),
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

/// Map a 1-based human-turn `ordinal` onto the index in the transcript's `len`
/// human-turn uuids to begin peeling from (`ordinal - 1`). Pure so the ordinal
/// arithmetic — the part that silently corrupted a resumed session — is
/// exhaustively unit-tested without a live claude.
///
/// `Err` (target unreachable) is not fatal: the caller falls back to a log-only
/// rewind. Unreachable when the target is the first turn (no preceding assistant to
/// land on) or past the turns the transcript holds (`>= len`).
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

/// claude's config root — `$CLAUDE_CONFIG_DIR` when set (claude honors it), else
/// `~/.claude`. Session transcripts live under `<root>/projects/`.
fn claude_config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

/// Locate `<config>/projects/*/<session_id>.jsonl` — claude keys each project's
/// transcript dir off a (lossy) escaping of the cwd, so rather than replicate that
/// we scan the project dirs for the file named by the (unique) session id.
fn transcript_path(session_id: &str) -> Option<PathBuf> {
    let projects = claude_config_dir()?.join("projects");
    let filename = format!("{session_id}.jsonl");
    for entry in std::fs::read_dir(&projects).ok()?.flatten() {
        let candidate = entry.path().join(&filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The uuids of the genuine human turns in a session transcript (`jsonl`), in
/// order — the same turns claude's own rewind targets. Mirrors claude's filter:
/// a `type:"user"` line that isn't a tool-result echo, meta / injected / compaction
/// line, subagent sidechain, or non-human origin. Pure + unit-tested.
fn parse_transcript_user_uuids(jsonl: &str) -> Vec<String> {
    jsonl
        .lines()
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line.trim()).ok()?;
            if v.get("type").and_then(Value::as_str) != Some("user") {
                return None;
            }
            let flag = |k| v.get(k).and_then(Value::as_bool).unwrap_or(false);
            if flag("isMeta")
                || flag("isCompactSummary")
                || flag("isVisibleInTranscriptOnly")
                || flag("isSidechain")
            {
                return None;
            }
            // A user line whose content is a tool_result array is a tool return.
            let content = v.get("message").and_then(|m| m.get("content"));
            let is_tool_result = content
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|b| b.get("type"))
                .and_then(Value::as_str)
                == Some("tool_result");
            if is_tool_result {
                return None;
            }
            // `origin.kind`, when present, must be human.
            if let Some(kind) = v.get("origin").and_then(|o| o.get("kind")).and_then(Value::as_str) {
                if kind != "human" {
                    return None;
                }
            }
            v.get("uuid").and_then(Value::as_str).map(str::to_string)
        })
        .collect()
}

/// Best-effort process-tree killer. On Unix we signal the process group we put
/// the child in; elsewhere we fall back to the pid (kill_on_drop covers the
/// common case where the driver is dropped).
struct Killer {
    pid: Option<u32>,
}

impl Killer {
    fn kill(&self) {
        let Some(pid) = self.pid else { return };
        #[cfg(unix)]
        {
            // Negative pid = the process group. SIGTERM first; the OS reaps.
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

    /// The index is `ordinal - 1`, so peeling from it keeps turns `1..ordinal` and
    /// drops `ordinal..`. Because the uuids come from the transcript (all turns,
    /// every generation), a turn from before a resume maps directly — the fix for
    /// the "predates this live generation" fall-through.
    #[test]
    fn rewind_maps_ordinal_to_index() {
        assert_eq!(rewind_local_start(2, 5), Ok(1));
        assert_eq!(rewind_local_start(5, 5), Ok(4));
        // A pre-resume turn is now reachable (transcript holds every turn's uuid).
        assert_eq!(rewind_local_start(3, 7), Ok(2));
        // The first turn has no preceding assistant — unreachable.
        assert!(rewind_local_start(1, 5).is_err());
        // Beyond what the transcript holds — unreachable.
        assert!(rewind_local_start(6, 5).is_err());
    }

    /// An empty transcript (not found / unreadable) reaches no turn — every ordinal
    /// falls back to a log-only rewind.
    #[test]
    fn rewind_empty_transcript_reaches_nothing() {
        for ordinal in 1..=8 {
            assert!(rewind_local_start(ordinal, 0).is_err());
        }
    }

    /// The transcript parser keeps genuine human turns in order and drops the rest:
    /// tool-result echoes, meta / compaction / injected lines, subagent sidechains,
    /// and non-human origins — mirroring what claude's own rewind will accept.
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

    /// Blank/garbage lines are skipped, not fatal (tolerant parse).
    #[test]
    fn parse_transcript_tolerates_junk_lines() {
        let jsonl = "\nnot json\n{}\n{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(parse_transcript_user_uuids(jsonl), vec!["u1"]);
    }

    /// AskUserQuestion is surfaced as an interactive `QuestionAsk` (carrying the
    /// questions JSON), not a yes/no `PermissionAsk`.
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
        });
        match events.as_slice() {
            [SessionEvent::QuestionAsk {
                request_id,
                questions: q,
            }] => {
                assert_eq!(request_id, "req-1");
                // The serialized questions round-trip back to the same value.
                assert_eq!(serde_json::from_str::<Value>(q).unwrap(), questions);
            }
            other => panic!("expected one QuestionAsk, got {other:?}"),
        }
    }

    /// Every other tool keeps the normal Allow/Deny permission gate.
    #[test]
    fn other_tools_remain_permission_asks() {
        let events = parsed_to_events(ParsedLine::CanUseTool {
            request_id: "req-2".into(),
            tool_name: "Bash".into(),
            input: json!({ "command": "ls" }),
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::PermissionAsk { tool_name, .. }] if tool_name == "Bash"
        ));
    }

    /// A background-task completion notification becomes a `BackgroundTaskUpdate`
    /// session event (the UI drops the finished shell from its live indicator),
    /// carrying the shell id, the spawning Bash tool-use id, and the status.
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

    /// Defensive: an AskUserQuestion whose `questions` is missing OR present but
    /// malformed (null, non-array, or empty) falls back to a permission card
    /// rather than emitting an unanswerable/blank question prompt.
    #[test]
    fn ask_user_question_with_bad_questions_falls_back_to_permission() {
        for bad in [
            json!({}), // missing entirely
            json!({ "questions": null }),
            json!({ "questions": "nope" }), // not an array
            json!({ "questions": [] }),     // empty array
        ] {
            let events = parsed_to_events(ParsedLine::CanUseTool {
                request_id: "req-3".into(),
                tool_name: "AskUserQuestion".into(),
                input: bad.clone(),
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
