//! The long-lived `codex app-server` session driver.
//!
//! Shaped like [`crate::claude::ClaudeDriver`]: one child process per session,
//! spoken to over stdin/stdout, with three background tasks —
//!
//! * **writer** — serializes JSON-RPC lines from every caller onto one stdin.
//! * **stderr** — drained into a small ring buffer, so a crash can report the tail.
//! * **reader + waiter** — stdout → [`SessionEvent`]s (and replies to the
//!   server's approval requests), then `child.wait()` → the final `Exited`.
//!
//! One process per session (rather than the shared-server shape the OpenCode
//! driver needs) buys crash isolation and a real `pid()` for the orphan ledger,
//! and lets `kill` signal the whole process group.
//!
//! Unlike Claude's, [`CodexDriver::spawn`] is async: the app-server needs an
//! `initialize` handshake and a `thread/start` round-trip before a turn can be
//! sent, and the thread id it mints is what `--resume`-equivalent continuation
//! keys on.

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
    approval_posture, approval_reply, decode_line, question_reply, Incoming, Translator,
};
use crate::agent::KillFlag;
use crate::claude::driver::PermissionMode;
use crate::claude::events::{ExitOutcome, SessionEvent};
use crate::error::{CoreError, Result};

const STDERR_RING_LINES: usize = 200;
const EVENT_CHANNEL_CAP: usize = 1024;
/// How long a startup round-trip (`initialize`, `thread/start`) may take before
/// we give up. Generous: a cold app-server also validates auth and loads config.
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Everything needed to launch one Codex session.
pub struct SpawnConfig {
    /// Absolute path to the resolved `codex` binary.
    pub codex_bin: PathBuf,
    /// Working directory — the workstream's worktree.
    pub cwd: PathBuf,
    pub permission_mode: PermissionMode,
    /// Extra writable roots (e.g. the thoughts root outside the worktree).
    pub add_dirs: Vec<PathBuf>,
    /// Continue an existing conversation by Codex thread id.
    pub resume: Option<String>,
    /// User turns this thread already logged before this process took over.
    /// Rewind ordinals are session-global, but the translator only counts what
    /// it streams — so a resume must seed the counter or `thread/rollback` drops
    /// the wrong number of turns while reporting success. 0 for a fresh thread.
    pub prior_user_turns: usize,
    /// Override the model (else Codex's configured default).
    pub model: Option<String>,
    /// Reasoning effort (`minimal|low|medium|high|xhigh`); else Codex's default.
    pub effort: Option<String>,
    /// Don't persist the thread to `~/.codex/sessions` — used by throwaway
    /// one-shot turns (auto-titles) so they don't pollute the user's history.
    pub ephemeral: bool,
    /// Augmented PATH + any extra env for the child.
    pub env: Vec<(String, String)>,
}

/// A handle to a live Codex session. Dropping it does NOT kill the child (the
/// waiter task owns it); call [`CodexDriver::kill`] for a hard stop.
pub struct CodexDriver {
    /// Codex's own thread id — the resume key, persisted as `claude_session_id`.
    thread_id: String,
    /// Serialized writes to the child's stdin.
    stdin_tx: mpsc::UnboundedSender<String>,
    events: broadcast::Sender<SessionEvent>,
    /// Monotonic JSON-RPC request id.
    rpc_seq: AtomicU64,
    /// Model sent with each turn, switchable live (Codex takes a per-turn model).
    model: StdMutex<Option<String>>,
    effort: StdMutex<Option<String>>,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    /// Requests awaiting a reply, keyed by rpc id. The reader resolves them.
    pending: Arc<StdMutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>>,
    /// Shared with the reader task: which approvals/questions are open, plus the
    /// active turn id (for `turn/interrupt`) and the user-turn count (for rollback).
    state: Arc<StdMutex<Translator>>,
    killer: Killer,
}

impl CodexDriver {
    /// Launch a session: spawn the app-server, shake hands, and start (or resume)
    /// a thread. The returned receiver is subscribed before the reader task can
    /// broadcast, so it cannot miss an event to a startup race.
    pub async fn spawn(config: SpawnConfig) -> Result<(Self, broadcast::Receiver<SessionEvent>)> {
        // Only one app-server may be STARTING at a time; concurrent starts
        // interfere and one loses. Dropped once the handshake returns, so the
        // session's own lifetime never blocks a probe. See `codex::startup_lock`.
        let startup = crate::codex::startup_lock().lock().await;

        let mut cmd = Command::new(&config.codex_bin);
        cmd.args(["app-server", "--stdio"])
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        // Unix: own process group so `kill` can signal the whole tree (codex
        // spawns tool subprocesses). Windows uses taskkill /T instead.
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Supervised: on Windows the job object kills the app-server (and anything
        // it spawned) even when our destructors never run — a `process::exit` or a
        // hard kill would otherwise orphan it.
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
        // Subscribe BEFORE the reader task exists: the caller's receiver can't
        // miss the `Init` event to a startup race.
        let primary_rx = events_tx.subscribe();
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        let stderr_ring = Arc::new(StdMutex::new(Vec::<String>::new()));
        let kill_flag = Arc::new(KillFlag::default());
        let pending = Arc::new(StdMutex::new(HashMap::new()));
        let state = Arc::new(StdMutex::new(Translator::resumed(config.prior_user_turns)));

        tokio::spawn(write_loop(stdin, stdin_rx));
        tokio::spawn(stderr_loop(stderr, stderr_ring.clone()));
        // The reader gets a WEAK sender: it needs to write auto-replies to server
        // requests, but holding a strong one would keep the write channel (and so
        // the child's stdin) open forever, so dropping the driver could never make
        // codex see EOF and exit.
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
            stderr_ring,
            kill_flag,
            pending,
            state,
            killer: Killer { pid },
        };

        // The handshake is fallible in ways a bare spawn isn't (auth missing,
        // an incompatible app-server). Fold the stderr tail into the error so
        // the UI can show `codex login` hints rather than a bare timeout.
        let started = driver.handshake(&config).await;
        // Startup is done either way — let the next app-server begin. (The error
        // path drops it too; this just makes the success path explicit.)
        drop(startup);
        let (thread_id, model) = match started {
            Ok(v) => v,
            Err(e) => {
                driver.kill();
                return Err(e);
            }
        };

        let driver = Self { thread_id, ..driver };

        // Mirrors the Claude driver's `init` line: records the harness session id
        // (used for resume) and flips the session row to running.
        let _ = driver.events.send(SessionEvent::Init {
            claude_session_id: driver.thread_id.clone(),
            model,
        });

        Ok((driver, primary_rx))
    }

    /// `initialize` → `initialized` → `thread/start`|`thread/resume`.
    /// Returns `(thread_id, model)`.
    async fn handshake(&self, config: &SpawnConfig) -> Result<(String, Option<String>)> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "hyprlayer",
                    "title": "Hyprlayer",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )
        .await?;
        self.notify("initialized", json!({}))?;

        let (approval_policy, sandbox) = approval_posture(config.permission_mode);
        let mut params = json!({
            "cwd": config.cwd.to_string_lossy(),
            "approvalPolicy": approval_policy,
            "sandbox": sandbox,
        });
        if let Some(model) = &config.model {
            params["model"] = json!(model);
        }
        // Configure the workspace-write sandbox: enable network so a *permitted*
        // command can actually reach it — Codex blocks network here by default, and
        // approving the command (untrusted) doesn't lift that, so without this an
        // allowed `curl` still fails to resolve. Also grant write access to roots
        // outside the worktree (the thoughts tree) so skills can write their docs.
        if sandbox == "workspace-write" {
            let roots: Vec<String> = config
                .add_dirs
                .iter()
                .map(|d| d.to_string_lossy().into_owned())
                .collect();
            params["config"] = json!({
                "sandbox_workspace_write": {
                    "network_access": true,
                    "writable_roots": roots,
                }
            });
        }

        let (method, params) = match &config.resume {
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
        };

        let result = self.request(method, params).await?;
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

    /// Send a user turn. Emits [`SessionEvent::UserMessage`] first so the turn is
    /// persisted and replays after a restart — Codex never echoes the human
    /// prompt back to us in a form we can distinguish from a replay.
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

        let mut params = json!({
            "threadId": self.thread_id,
            "input": [{ "type": "text", "text": text, "text_elements": [] }],
        });
        if let Some(model) = self.model.lock().ok().and_then(|m| m.clone()) {
            params["model"] = json!(model);
        }
        if let Some(effort) = self.effort.lock().ok().and_then(|e| e.clone()) {
            params["effort"] = json!(effort);
        }

        // Fire and forget, but surface a rejected turn: `turn/start` answers
        // before any content streams, so a failure (bad model, expired auth,
        // a busy thread) would otherwise leave the UI pinned on "running" with
        // nothing to show.
        let rx = self.request_channel("turn/start", params)?;
        let events = self.events.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            match rx.await {
                Ok(Err(e)) => {
                    let _ = events.send(SessionEvent::AssistantText {
                        text: format!("⚠ Codex request failed: {e}"),
                        parent_id: None,
                    });
                    let _ = events.send(SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                    });
                }
                // Record the turn id from the REPLY, which lands before the
                // `turn/started` notification. Without this, Stop during that
                // window finds no turn to interrupt, reports success, and the
                // frontend escalates to killing the whole session.
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

    /// Switch the model for subsequent turns. Codex takes a model per turn, so
    /// this applies live — no relaunch (unlike Claude, which binds at spawn).
    pub fn set_model(&self, model: Option<String>) {
        if let Ok(mut m) = self.model.lock() {
            *m = model;
        }
    }

    /// Same for reasoning effort, which rides on the same `turn/start`.
    pub fn set_effort(&self, effort: Option<String>) {
        if let Ok(mut e) = self.effort.lock() {
            *e = effort;
        }
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
            // Already answered, or the turn was interrupted out from under it.
            return Ok(());
        };
        self.reply(rpc_id, approval_reply(&kind, allow, message))
    }

    pub fn answer_question(&self, request_id: &str, answers: Value) -> Result<()> {
        let pending = self
            .state
            .lock()
            .ok()
            .and_then(|mut s| s.pending_questions.remove(request_id));
        let Some(pending) = pending else {
            return Ok(());
        };
        let body = question_reply(&pending, &answers);
        self.reply(pending.rpc_id.clone(), body)
    }

    /// Graceful interrupt of the current turn (the session stays alive). A no-op
    /// between turns — there is nothing to address.
    pub fn interrupt(&self) -> Result<()> {
        let turn_id = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.active_turn.clone().or_else(|| s.requested_turn.clone()));
        let Some(turn_id) = turn_id else {
            return Ok(());
        };
        let _ = self.request_channel(
            "turn/interrupt",
            json!({ "threadId": self.thread_id, "turnId": turn_id }),
        )?;
        Ok(())
    }

    /// Truncate the model's conversation to the 1-based `ordinal` human turn,
    /// dropping that turn and everything after it.
    ///
    /// Codex's `thread/rollback` drops N turns from the END, so the count is
    /// derived from how many user turns this thread has seen — which is why a
    /// resumed driver seeds its counter with [`SpawnConfig::prior_user_turns`].
    /// Without that seeding the count is short by however many turns predate this
    /// process, and the rollback silently drops the wrong ones. It only rewinds
    /// conversation history — local file changes the agent already made stay put,
    /// which matches the app's log-only fallback semantics.
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

    pub fn kill(&self) {
        self.kill_flag.mark_killed();
        self.killer.kill();
    }

    /// Same teardown, but for the app's own bookkeeping (quit, relaunch) — the
    /// exit is classified `Detached` and leaves the transcript untouched.
    pub fn kill_detached(&self) {
        self.kill_flag.mark_detached();
        self.killer.kill();
    }

    pub fn pid(&self) -> Option<u32> {
        self.killer.pid
    }

    /// Codex's own thread id — the resume key.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Snapshot of the captured stderr tail (crash diagnostics).
    pub fn stderr_tail(&self) -> String {
        self.stderr_ring
            .lock()
            .map(|r| r.join("\n"))
            .unwrap_or_default()
    }

    /// Send a request and await its reply.
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let rx = self.request_channel(method, params)?;
        let replied = tokio::time::timeout(STARTUP_TIMEOUT, rx).await;
        let failed = |detail: String| CoreError::OutputParse {
            program: "codex".into(),
            detail,
        };
        match replied {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(e))) => Err(failed(format!("{method}: {e}"))),
            // The reader dropped the sender: the process died mid-request.
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

    /// Send a request, returning the channel its reply will arrive on.
    fn request_channel(
        &self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<std::result::Result<Value, String>>> {
        let id = self.rpc_seq.fetch_add(1, Ordering::SeqCst);
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

/// How many turns `thread/rollback` must drop to land on the 1-based `ordinal`
/// user turn, given `total` turns seen. Rewinding to turn N keeps `1..N` and
/// drops `N..`, so that's `total - ordinal + 1`.
///
/// Split out so the arithmetic (and its two unreachable cases) is testable
/// without a live app-server.
fn rollback_count(ordinal: usize, total: usize) -> Result<u64> {
    let unsupported = |detail: &str| CoreError::OutputParse {
        program: "codex".into(),
        detail: detail.to_string(),
    };
    if ordinal == 0 {
        return Err(unsupported("turn ordinals are 1-based"));
    }
    if ordinal > total {
        // A turn from before this process took over the thread: the counter only
        // saw the turns it streamed. The caller falls back to a log-only cut.
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
    mut child: crate::process::SupervisedChild,
    stdout: tokio::process::ChildStdout,
    events: broadcast::Sender<SessionEvent>,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    pending: Arc<StdMutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>>,
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
                match incoming {
                    Incoming::Response { id, result } => {
                        let waiter = pending.lock().ok().and_then(|mut m| m.remove(&id));
                        if let Some(tx) = waiter {
                            let _ = tx.send(result);
                        }
                    }
                    Incoming::Notification { method, params } => {
                        if method == "thread/closed" {
                            clean_exit = true;
                        }
                        let out = match state.lock() {
                            Ok(mut s) => s.notification(&method, &params),
                            Err(_) => Vec::new(),
                        };
                        for event in out {
                            let _ = events.send(event);
                        }
                    }
                    Incoming::ServerRequest { id, method, params } => {
                        let out = match state.lock() {
                            Ok(mut s) => s.server_request(&id, &method, &params),
                            Err(_) => Vec::new(),
                        };
                        if out.is_empty() {
                            // A request we don't model (MCP elicitation, dynamic
                            // tool calls, attestation). Leaving it unanswered
                            // wedges the turn, so decline explicitly.
                            auto_decline(&stdin_tx, &id, &method);
                        }
                        for event in out {
                            let _ = events.send(event);
                        }
                    }
                }
            }
            Ok(None) => break, // EOF: stdout closed, process is winding down
            Err(_) => break,   // read error: treat as EOF
        }
    }

    // Anything still waiting on a reply will never get one.
    if let Ok(mut map) = pending.lock() {
        for (_, tx) in map.drain() {
            let _ = tx.send(Err("codex app-server exited".into()));
        }
    }

    let status = child.wait().await;
    let code = status.as_ref().ok().and_then(|s| s.code());
    let stderr_tail = stderr_ring.lock().map(|r| r.join("\n")).unwrap_or_default();

    // An app-initiated kill takes precedence (and carries its reason); otherwise
    // a clean exit code is Completed and anything else is read from the wait
    // status. Unlike Claude there's no terminal `result` line to key on — the
    // app-server just exits when its stdin closes.
    let outcome = kill_flag.outcome().unwrap_or_else(|| match &status {
        Ok(s) if s.success() || clean_exit => ExitOutcome::Completed,
        Ok(s) => ExitOutcome::from_status(s),
        Err(_) => ExitOutcome::SpawnFailed,
    });

    let _ = events.send(SessionEvent::Exited {
        outcome,
        code,
        stderr_tail,
    });
}

/// Refuse a server request we don't implement, in the shape that request
/// expects. Codex blocks the turn until every request is answered, so silence
/// is the one thing we must not do.
fn auto_decline(stdin_tx: &mpsc::WeakUnboundedSender<String>, id: &Value, method: &str) {
    let result = match method {
        "mcpServer/elicitation/request" => json!({ "action": "decline" }),
        "execCommandApproval" | "applyPatchApproval" => json!({
            "decision": { "denied": { "rejection": "unsupported by this client" } }
        }),
        _ => json!({}),
    };
    let line = format!("{}\n", json!({ "id": id, "result": result }));
    // `None` once the driver is gone — the session is being torn down, so there's
    // no turn left to unblock.
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

    /// Rewinding to turn N keeps `1..N` and drops `N..`, so a 5-turn thread
    /// rewound to turn 2 drops 4.
    #[test]
    fn rollback_count_maps_ordinal_to_turns_dropped() {
        assert_eq!(rollback_count(2, 5).ok(), Some(4));
        // The most recent turn drops just itself.
        assert_eq!(rollback_count(5, 5).ok(), Some(1));
        // Rewinding to the first turn clears the whole conversation.
        assert_eq!(rollback_count(1, 5).ok(), Some(5));
    }

    /// The resume case: 2 turns before the restart, 5 streamed after. Rewinding
    /// to app-turn 3 must drop 5 turns (3..=7), not 3 — the bug seeding fixes.
    /// A translator that counted only this generation reports total=5 and would
    /// drop the LAST 3, leaving the model remembering turns the log deleted while
    /// the command still claims `live_rewound`.
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
        // Without seeding the same rewind silently under-drops.
        assert_eq!(rollback_count(3, 5).ok(), Some(3));
    }

    #[test]
    fn rollback_rejects_unreachable_turns() {
        // Ordinals are 1-based; 0 is a caller bug.
        assert!(rollback_count(0, 5).is_err());
        // A turn this process never streamed (it predates the live generation) —
        // the caller falls back to a log-only cut rather than mis-rolling back.
        assert!(rollback_count(6, 5).is_err());
        assert!(rollback_count(1, 0).is_err());
    }

    #[test]
    fn unmodelled_server_requests_are_declined_not_ignored() {
        // Codex blocks the turn until every server request is answered, so an
        // unhandled one must still get a reply.
        let (tx, mut rx) = mpsc::unbounded_channel();
        auto_decline(&tx.downgrade(), &json!(9), "mcpServer/elicitation/request");
        let line = rx.try_recv().expect("a reply was written");
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], json!(9));
        assert_eq!(v["result"]["action"], json!("decline"));

        auto_decline(&tx.downgrade(), &json!(10), "applyPatchApproval");
        let line = rx.try_recv().expect("a reply was written");
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["result"]["decision"]["denied"]["rejection"], json!("unsupported by this client"));
    }
}
