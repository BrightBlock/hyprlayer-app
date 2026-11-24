//! A PTY codex whose TUI runs over its own `codex app-server` (`--remote`), so the bar changes it
//! through `thread/settings/update`, session-only (recipe observed on codex 0.164.0). Unix only:
//! the app-server listens on a unix socket.

#[cfg(unix)]
pub(crate) use unix::{skip_hook_review, AppServerSpec, CodexRemote, MAX_SOCKET_PATH};

#[cfg(unix)]
mod unix {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use hyprlayer_models::{PtyAgentContext, PtyAgentStatus};
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::sync::{mpsc, oneshot, OwnedMutexGuard};
    use tokio::time::Instant;
    use tokio_tungstenite::client_async;
    use tokio_tungstenite::tungstenite::Message;

    use super::super::pty_agents::PtyAgents;
    use super::super::rollout::codex_used_percent;
    use crate::codex::probe::parse_models;
    use crate::error::{CoreError, Result};
    use crate::process::orphan;
    use crate::runtime::terminals::{agent_command_line, default_shell, login_shell_args};
    use crate::terminal::PtySession;

    /// `sun_path` holds 104 bytes on macOS, its NUL included.
    pub(crate) const MAX_SOCKET_PATH: usize = 103;
    /// Short, so a failed app-server falls back to the embedded launch quickly.
    const CONNECT_WAIT: Duration = Duration::from_secs(5);
    /// How long a stopped app-server gets to exit before it is killed: mid-turn it ignores
    /// `SIGTERM` and keeps its thread, which a resume of that thread then can't load.
    const STOP_GRACE: Duration = Duration::from_secs(3);
    const CONNECT_POLL: Duration = Duration::from_millis(100);
    const UPGRADE_WAIT: Duration = Duration::from_secs(5);
    const REQUEST_WAIT: Duration = Duration::from_secs(10);
    const LOADED_WAIT: Duration = Duration::from_secs(60);
    const LOADED_POLL: Duration = Duration::from_secs(1);
    const REVIEW_WAIT: Duration = Duration::from_secs(30);
    /// codex's own update prompt can sit in front of the review until a person answers it.
    const UPDATE_WAIT: Duration = Duration::from_secs(600);
    const REVIEW_POLL: Duration = Duration::from_millis(100);
    const REVIEW_SETTLE: Duration = Duration::from_secs(1);
    const REVIEW_ESCS: u32 = 3;
    /// Its key hints close the bottom pane, a footer row or two at most below them.
    const REVIEW_HINT_ROWS: usize = 3;
    /// The dialog's height above its key hints (observed 0.164.0: 7 rows).
    const REVIEW_ROWS: usize = 12;
    const ESC: &[u8] = b"\x1b";

    /// One app-server, run like the TUI (a login shell, in the worktree) with the agent's identity env.
    pub(crate) struct AppServerSpec {
        pub command: String,
        /// The hook and MCP `-c` pairs: an app-server's hooks and shell tools run under it, not the TUI.
        pub config: Vec<String>,
        pub cwd: PathBuf,
        pub env: Vec<(String, String)>,
        pub socket: PathBuf,
        pub pid_file: PathBuf,
        pub terminal_id: String,
    }

    type Reply = std::result::Result<Value, String>;

    pub(crate) struct CodexRemote {
        terminal_id: String,
        socket: PathBuf,
        pid_file: PathBuf,
        /// Leads the app-server's process group: the wrapper and the native binary both.
        pid: u32,
        stopped: AtomicBool,
        agents: Weak<PtyAgents>,
        outbox: mpsc::UnboundedSender<Message>,
        seq: AtomicU64,
        pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
        state: Mutex<State>,
        /// One change at a time, so a mode's two updates never interleave with another change.
        changing: tokio::sync::Mutex<()>,
    }

    #[derive(Default)]
    struct State {
        thread: Option<String>,
        subscribed: bool,
        subscribing: bool,
        /// `thread/resume` finds no rollout until the thread's first turn has run.
        turned: bool,
        /// A resume can't take the bypass flag, so the bypass pair waits for the loaded thread.
        owe_bypass: bool,
        model: Option<String>,
        effort: Option<String>,
        plan: bool,
        /// Until then the hooks' coarser mode stands in.
        mode_reported: bool,
    }

    impl CodexRemote {
        /// Starts the app-server and connects to it before any TUI does, so `thread/started` reaches us.
        pub(crate) async fn start(
            spec: AppServerSpec,
            agents: &Arc<PtyAgents>,
        ) -> Result<Arc<Self>> {
            let _ = std::fs::remove_file(&spec.socket);
            let mut argv = vec![
                "app-server".to_string(),
                "--listen".to_string(),
                listen_url(&spec.socket),
            ];
            argv.extend(spec.config);
            let mut cmd = tokio::process::Command::new(default_shell());
            crate::process::appimage::scrub_tokio(&mut cmd);
            cmd.args(login_shell_args(&agent_command_line(&spec.command, &argv)))
                .current_dir(&spec.cwd)
                .envs(spec.env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .process_group(0);
            // Only the spawn: a stalled connect must not hold up composer codex starts.
            let mut child = {
                let _startup = crate::codex::startup_lock().lock().await;
                cmd.spawn()
                    .map_err(|e| failed(format!("codex app-server: {e}")))?
            };
            let Some(pid) = child.id() else {
                return Err(failed("codex app-server exited at once"));
            };
            orphan::record(&spec.pid_file, pid, &listen_url(&spec.socket));
            if let Some(stderr) = child.stderr.take() {
                let id = spec.terminal_id.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::debug!("terminal {id}: codex app-server: {line}");
                    }
                });
            }
            let ws = match connect(&spec.socket, &mut child).await {
                Ok(ws) => ws,
                Err(e) => {
                    orphan::kill_group(pid);
                    discard(&spec.socket, &spec.pid_file);
                    return Err(e);
                }
            };
            let (mut sink, mut stream) = ws.split();
            let (outbox, mut queue) = mpsc::unbounded_channel::<Message>();
            tokio::spawn(async move {
                while let Some(message) = queue.recv().await {
                    if sink.send(message).await.is_err() {
                        return;
                    }
                }
            });
            let remote = Arc::new(Self {
                terminal_id: spec.terminal_id,
                socket: spec.socket,
                pid_file: spec.pid_file,
                pid,
                stopped: AtomicBool::new(false),
                agents: Arc::downgrade(agents),
                outbox,
                seq: AtomicU64::new(1),
                pending: Mutex::default(),
                state: Mutex::default(),
                changing: tokio::sync::Mutex::default(),
            });
            let reader = Arc::downgrade(&remote);
            tokio::spawn(async move {
                while let Some(Ok(message)) = stream.next().await {
                    let Some(remote) = reader.upgrade() else {
                        return;
                    };
                    match message {
                        Message::Text(text) => remote.receive(text.as_str()),
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                if let Some(remote) = reader.upgrade() {
                    remote.fail_pending();
                }
            });
            let watcher = Arc::downgrade(&remote);
            let recorded = remote.pid_file.clone();
            tokio::spawn(async move {
                let status = child.wait().await;
                // Only once it has exited: a record of one still running lets the next server reap it.
                orphan::clear(&recorded);
                if let Some(remote) = watcher.upgrade() {
                    remote.exited(status);
                }
            });
            let ready = async {
                remote
                    .request("initialize", crate::codex::driver::initialize_params())
                    .await
                    .map_err(failed)?;
                remote.notify("initialized")
            };
            if let Err(e) = ready.await {
                remote.stop();
                return Err(e);
            }
            Ok(remote)
        }

        /// The TUI's `--remote` argument.
        pub(crate) fn url(&self) -> String {
            listen_url(&self.socket)
        }

        /// SIGTERM to the whole group, which also removes the socket; we remove it too in case it can't.
        pub(crate) fn stop(&self) {
            if self.stopped.swap(true, Ordering::SeqCst) {
                return;
            }
            orphan::kill_group(self.pid);
            let _ = std::fs::remove_file(&self.socket);
            let pid = self.pid;
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    tokio::time::sleep(STOP_GRACE).await;
                    if orphan::alive(pid) {
                        orphan::kill_group_now(pid);
                    }
                });
            }
        }

        pub(crate) fn running(&self) -> bool {
            orphan::alive(self.pid)
        }

        pub(crate) fn kill_now(&self) {
            orphan::kill_group_now(self.pid);
        }

        pub(crate) fn mode_reported(&self) -> bool {
            self.lock().mode_reported
        }

        pub(crate) fn alive(&self) -> bool {
            !self.stopped.load(Ordering::SeqCst)
        }

        /// A TUI that reconnects to a fresh app-server loses its hooks, so its PTY ends instead.
        fn exited(&self, status: std::io::Result<std::process::ExitStatus>) {
            if self.stopped.swap(true, Ordering::SeqCst) {
                return;
            }
            tracing::warn!(
                "terminal {}: its codex app-server exited ({status:?}); ending the TUI",
                self.terminal_id
            );
            orphan::kill_group(self.pid);
            discard(&self.socket, &self.pid_file);
            if let Some(agents) = self.agents.upgrade() {
                agents.end_terminal(&self.terminal_id);
            }
        }

        /// A resume launch: its thread has a rollout already, and bypass waits for the TUI to load it.
        pub(crate) fn resume(self: &Arc<Self>, thread: Option<String>, bypass: bool) {
            {
                let mut state = self.lock();
                state.turned = true;
                state.owe_bypass = bypass;
            }
            if let Some(thread) = thread {
                self.adopt_thread(&thread);
            }
        }

        /// The TUI's thread, from `thread/started` or its `SessionStart` hook; subscribes once it can.
        pub(crate) fn adopt_thread(self: &Arc<Self>, thread: &str) {
            {
                let mut state = self.lock();
                if state.thread.as_deref() == Some(thread) {
                    return;
                }
                state.thread = Some(thread.to_string());
                state.subscribed = false;
            }
            self.subscribe();
        }

        /// Waits for the TUI to load the thread, so our resume never preempts its `-m`/effort.
        fn subscribe(self: &Arc<Self>) {
            {
                let mut state = self.lock();
                if state.subscribing || state.subscribed || state.thread.is_none() {
                    return;
                }
                state.subscribing = true;
            }
            let remote = self.clone();
            tokio::spawn(async move {
                remote.subscribe_loaded().await;
                remote.lock().subscribing = false;
            });
        }

        async fn subscribe_loaded(&self) {
            let deadline = Instant::now() + LOADED_WAIT;
            loop {
                let Some(thread) = self.lock().thread.clone() else {
                    return;
                };
                if self.loaded(&thread).await {
                    if std::mem::take(&mut self.lock().owe_bypass) {
                        if let Err(e) = self.update(&thread, bypass_pair()).await {
                            tracing::warn!(
                                "terminal {}: resumed without bypass: {e}",
                                self.terminal_id
                            );
                        }
                    }
                    if !self.lock().turned {
                        return;
                    }
                    let params = json!({ "threadId": thread, "excludeTurns": true });
                    match self.request("thread/resume", params).await {
                        Ok(result) => {
                            let mut state = self.lock();
                            if state.thread.as_deref() != Some(thread.as_str()) {
                                return;
                            }
                            state.subscribed = true;
                            drop(state);
                            self.fold(&result);
                            return;
                        }
                        Err(e) => tracing::debug!(
                            "terminal {}: not subscribed to its codex thread yet: {e}",
                            self.terminal_id
                        ),
                    }
                }
                if !self.alive() || Instant::now() >= deadline {
                    return;
                }
                tokio::time::sleep(LOADED_POLL).await;
            }
        }

        async fn loaded(&self, thread: &str) -> bool {
            self.request("thread/loaded/list", json!({}))
                .await
                .ok()
                .and_then(|r| r.get("data").and_then(Value::as_array).cloned())
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(thread)))
        }

        fn receive(self: &Arc<Self>, text: &str) {
            let Ok(message) = serde_json::from_str::<Value>(text) else {
                return;
            };
            let method = message.get("method").and_then(Value::as_str);
            match (message.get("id"), method) {
                (Some(id), None) => {
                    let reply = match message.get("error") {
                        Some(error) => Err(error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("an error")
                            .to_string()),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let waiter = id.as_u64().and_then(|id| lock(&self.pending).remove(&id));
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(reply);
                    }
                }
                // Approvals fan out to every client; the TUI answers them, so a request is never ours to.
                (Some(_), Some(_)) => {}
                (None, Some(method)) => {
                    let params = message.get("params").unwrap_or(&Value::Null);
                    self.notified(method, params);
                }
                (None, None) => {}
            }
        }

        fn notified(self: &Arc<Self>, method: &str, params: &Value) {
            if method == "thread/started" {
                let thread = params.get("thread").unwrap_or(&Value::Null);
                // The TUI's title thread is ephemeral; a subagent's has a parent.
                let own = thread.get("ephemeral").and_then(Value::as_bool) == Some(false)
                    && thread.get("parentThreadId").is_none_or(Value::is_null);
                if let (true, Some(id)) = (own, thread.get("id").and_then(Value::as_str)) {
                    self.fold(thread);
                    self.adopt_thread(id);
                }
                return;
            }
            let ours = params
                .get("threadId")
                .and_then(Value::as_str)
                .is_some_and(|id| self.lock().thread.as_deref() == Some(id));
            if !ours {
                return;
            }
            match method {
                "thread/status/changed" => {
                    match params.pointer("/status/type").and_then(Value::as_str) {
                        Some("active") => {
                            self.lock().turned = true;
                            self.subscribe();
                        }
                        Some("idle") if self.lock().turned => self.subscribe(),
                        _ => {}
                    }
                }
                "thread/settings/updated" => {
                    if let Some(settings) = params.get("threadSettings") {
                        self.fold(settings);
                    }
                }
                "thread/tokenUsage/updated" => {
                    let usage = params.get("tokenUsage").unwrap_or(&Value::Null);
                    let tokens = usage.pointer("/last/totalTokens").and_then(Value::as_u64);
                    let window = usage.get("modelContextWindow").and_then(Value::as_u64);
                    if let (Some(tokens), Some(window)) = (tokens, window) {
                        self.publish(|status| {
                            let tokens = u32::try_from(tokens).unwrap_or(u32::MAX);
                            let window = u32::try_from(window).unwrap_or(u32::MAX);
                            status.context = Some(PtyAgentContext {
                                tokens,
                                window,
                                percent: Some(codex_used_percent(tokens, window)),
                            });
                        });
                    }
                }
                _ => {}
            }
        }

        /// Settings as codex reported them: `thread/started`'s thread, a resume result, or `threadSettings`.
        fn fold(&self, settings: &Value) {
            let text = |keys: &[&str]| {
                keys.iter()
                    .find_map(|k| settings.get(*k).and_then(Value::as_str))
                    .map(str::to_string)
            };
            let model = text(&["model"]);
            let effort = text(&["effort", "reasoningEffort"]);
            let plan = settings
                .pointer("/collaborationMode/mode")
                .and_then(Value::as_str)
                .map(|mode| mode == "plan");
            let approval = settings.get("approvalPolicy").and_then(Value::as_str);
            let sandbox = settings
                .get("sandboxPolicy")
                .or_else(|| settings.get("sandbox"))
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str);
            let mode = match (plan, approval, sandbox) {
                (Some(true), ..) => Some(Some("plan")),
                (_, Some(approval), Some(sandbox)) => Some(mode_key(approval, sandbox)),
                _ => None,
            };
            {
                let mut state = self.lock();
                if model.is_some() {
                    state.model.clone_from(&model);
                }
                if effort.is_some() {
                    state.effort.clone_from(&effort);
                }
                if let Some(plan) = plan {
                    state.plan = plan;
                }
                state.mode_reported |= mode.is_some();
            }
            self.publish(|status| {
                if model.is_some() {
                    status.model = model;
                }
                if effort.is_some() {
                    status.effort = effort;
                }
                if let Some(mode) = mode {
                    status.permission_mode = mode.map(str::to_string);
                }
            });
        }

        fn publish(&self, change: impl FnOnce(&mut PtyAgentStatus)) {
            if let Some(agents) = self.agents.upgrade() {
                agents.fold_remote_status(&self.terminal_id, change);
            }
        }

        pub(crate) async fn set_model(&self, model: &str) -> Result<()> {
            let _one = self.changing.lock().await;
            let thread = self.thread()?;
            if !self.models().await?.iter().any(|m| m.id == model) {
                return Err(failed(format!("codex's model list has no {model}")));
            }
            self.change(&thread, json!({ "model": model }), |state, status| {
                state.model = Some(model.to_string());
                status.model = Some(model.to_string());
            })
            .await
        }

        pub(crate) async fn set_effort(&self, effort: &str) -> Result<()> {
            let _one = self.changing.lock().await;
            let thread = self.thread()?;
            let model = self
                .lock()
                .model
                .clone()
                .ok_or_else(|| failed("codex hasn't reported its model yet"))?;
            let known = self
                .models()
                .await?
                .into_iter()
                .find(|m| m.id == model)
                .is_some_and(|m| m.supported_efforts.iter().any(|e| e == effort));
            if !known {
                return Err(failed(format!("{model} has no {effort} effort")));
            }
            self.change(&thread, json!({ "effort": effort }), |state, status| {
                state.effort = Some(effort.to_string());
                status.effort = Some(effort.to_string());
            })
            .await
        }

        /// The bar's mode keys as codex's composer lowers them; `auto` has no codex counterpart.
        pub(crate) async fn set_permission_mode(&self, key: &str) -> Result<()> {
            let permissions = match key {
                "default" => json!({ "permissions": ":workspace", "approvalPolicy": "on-request" }),
                "acceptEdits" => json!({ "permissions": ":workspace", "approvalPolicy": "never" }),
                "plan" => json!({ "permissions": ":read-only", "approvalPolicy": "on-request" }),
                "bypassPermissions" => bypass_pair(),
                _ => return Err(failed(format!("codex has no {key} permission mode"))),
            };
            let _one = self.changing.lock().await;
            let thread = self.thread()?;
            let want_plan = key == "plan";
            if self.lock().plan != want_plan {
                let (model, effort) = {
                    let state = self.lock();
                    (state.model.clone(), state.effort.clone())
                };
                let model = model.ok_or_else(|| failed("codex hasn't reported its model yet"))?;
                // Its settings overwrite the thread's model and effort, so they go back as they are.
                let settings = json!({
                    "model": model,
                    "reasoning_effort": effort,
                    "developer_instructions": null,
                });
                let mode = if want_plan { "plan" } else { "default" };
                let mode = json!({ "collaborationMode": { "mode": mode, "settings": settings } });
                self.change(&thread, mode, |state, _| state.plan = want_plan)
                    .await?;
            }
            self.change(&thread, permissions, |state, status| {
                state.mode_reported = true;
                status.permission_mode = Some(key.to_string());
            })
            .await
        }

        /// Before the subscribe no `thread/settings/updated` comes back, so an accepted change folds here.
        async fn change(
            &self,
            thread: &str,
            params: Value,
            accepted: impl FnOnce(&mut State, &mut PtyAgentStatus),
        ) -> Result<()> {
            self.update(thread, params).await.map_err(failed)?;
            let mut state = self.lock();
            if state.subscribed {
                return Ok(());
            }
            let mut status = PtyAgentStatus::default();
            accepted(&mut state, &mut status);
            drop(state);
            self.publish(|now| {
                if status.model.is_some() {
                    now.model = status.model;
                }
                if status.effort.is_some() {
                    now.effort = status.effort;
                }
                if status.permission_mode.is_some() {
                    now.permission_mode = status.permission_mode;
                }
            });
            Ok(())
        }

        async fn update(&self, thread: &str, mut params: Value) -> std::result::Result<(), String> {
            params["threadId"] = json!(thread);
            self.request("thread/settings/update", params)
                .await
                .map(|_| ())
                .map_err(|e| format!("codex refused the change: {e}"))
        }

        async fn models(&self) -> Result<Vec<crate::codex::CodexModel>> {
            let list = self
                .request("model/list", json!({}))
                .await
                .map_err(|e| failed(format!("codex's model list is unreadable: {e}")))?;
            Ok(parse_models(&list))
        }

        fn thread(&self) -> Result<String> {
            if !self.alive() {
                return Err(failed("that agent is no longer running"));
            }
            self.lock()
                .thread
                .clone()
                .ok_or_else(|| failed("codex hasn't started its thread yet"))
        }

        async fn request(&self, method: &str, params: Value) -> std::result::Result<Value, String> {
            let id = self.seq.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            lock(&self.pending).insert(id, tx);
            let message = json!({ "id": id, "method": method, "params": params });
            if self
                .outbox
                .send(Message::text(message.to_string()))
                .is_err()
            {
                lock(&self.pending).remove(&id);
                return Err("the app-server connection is closed".into());
            }
            match tokio::time::timeout(REQUEST_WAIT, rx).await {
                Ok(Ok(reply)) => reply,
                Ok(Err(_)) => Err("the app-server connection closed".into()),
                Err(_) => {
                    lock(&self.pending).remove(&id);
                    Err(format!("{method} got no answer"))
                }
            }
        }

        fn notify(&self, method: &str) -> Result<()> {
            let message = json!({ "method": method, "params": {} });
            self.outbox
                .send(Message::text(message.to_string()))
                .map_err(|_| failed("the app-server connection is closed"))
        }

        fn fail_pending(&self) {
            lock(&self.pending).clear();
        }

        fn lock(&self) -> MutexGuard<'_, State> {
            lock(&self.state)
        }
    }

    /// Esc on a resumed TUI's "Hooks need review": Enter or a digit there trusts the hooks in
    /// `config.toml`, so only Esc is sent, and only while the dialog is on screen. Holds the
    /// terminal's input lock so an agent's first prompt waits behind it.
    pub(crate) async fn skip_hook_review(pty: Arc<PtySession>, _input: OwnedMutexGuard<()>) {
        let screen = pty.screen();
        let started = Instant::now();
        let mut deadline = started + REVIEW_WAIT;
        // Two equal reads, or a second of it: an Esc during the dialog's first paint can be lost.
        let mut last: Option<Vec<String>> = None;
        let mut shown_at: Option<Instant> = None;
        loop {
            let screen_rows = screen.rows();
            // Never answered here: skipping or taking an update is the person's call.
            if screen_rows.as_deref().is_some_and(update_prompt) {
                deadline = (Instant::now() + REVIEW_WAIT).min(started + UPDATE_WAIT);
            }
            let rows = screen_rows.filter(|rows| hook_review(rows));
            if rows.is_none() {
                shown_at = None;
            } else if rows == last || shown_at.is_some_and(|at| at.elapsed() >= REVIEW_SETTLE) {
                break;
            } else {
                shown_at.get_or_insert_with(Instant::now);
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    "a resumed codex never showed its hook review; its hooks may not run"
                );
                return;
            }
            last = rows;
            tokio::time::sleep(REVIEW_POLL).await;
        }
        for attempt in 1..=REVIEW_ESCS {
            if let Err(e) = pty.write_input(ESC) {
                tracing::warn!("could not dismiss a resumed codex's hook review: {e}");
                return;
            }
            tracing::info!("sent Esc {attempt} of {REVIEW_ESCS} to a resumed codex's hook review");
            tokio::time::sleep(REVIEW_SETTLE).await;
            // Another Esc only while the dialog still shows: past it, one could interrupt a turn.
            if !screen.rows().is_some_and(|rows| hook_review(&rows)) {
                tracing::info!("dismissed a resumed codex's hook review");
                return;
            }
        }
        tracing::warn!(
            "a resumed codex's hook review is still on screen after {REVIEW_ESCS} Escs; its tab needs an Esc by hand"
        );
    }

    /// codex's "Update available" prompt, which a launch shows before anything else.
    fn update_prompt(rows: &[String]) -> bool {
        let shown: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|r| !r.trim().is_empty())
            .collect();
        shown
            .iter()
            .rev()
            .take(REVIEW_HINT_ROWS)
            .any(|r| r.contains("esc skip"))
            && shown
                .iter()
                .rev()
                .take(REVIEW_ROWS)
                .any(|r| r.contains("Update available") || r.contains("Update now"))
    }

    /// The dialog in codex's bottom pane, where it replaces the composer: a resumed conversation
    /// that quotes it sits above a composer.
    fn hook_review(rows: &[String]) -> bool {
        let shown: Vec<&str> = rows
            .iter()
            .map(String::as_str)
            .filter(|r| !r.trim().is_empty())
            .collect();
        let Some(hints) = shown
            .iter()
            .rev()
            .take(REVIEW_HINT_ROWS)
            .position(|r| r.contains("esc skip"))
            .map(|from_end| shown.len() - 1 - from_end)
        else {
            return false;
        };
        let top = hints.saturating_sub(REVIEW_ROWS);
        let Some(title) = shown[top..hints]
            .iter()
            .rposition(|r| r.trim() == "Hooks need review")
            .map(|at| top + at)
        else {
            return false;
        };
        !shown[title..].iter().any(|r| composer(r))
    }

    /// `› ` then anything but a numbered choice: the dialog's own rows are `› 1. Review hooks`.
    fn composer(row: &str) -> bool {
        row.trim_start()
            .strip_prefix('›')
            .is_some_and(|rest| !rest.trim_start().starts_with(|c: char| c.is_ascii_digit()))
    }

    /// codex's YOLO: the `:danger-full-access` profile alone leaves approvals on.
    fn bypass_pair() -> Value {
        json!({ "approvalPolicy": "never", "sandboxPolicy": { "type": "dangerFullAccess" } })
    }

    /// The bar's key for a sandbox and approval pair, the reverse of `set_permission_mode`.
    fn mode_key(approval: &str, sandbox: &str) -> Option<&'static str> {
        match (sandbox, approval) {
            ("dangerFullAccess", "never") => Some("bypassPermissions"),
            ("workspaceWrite", "never") => Some("acceptEdits"),
            ("workspaceWrite", "on-request" | "untrusted" | "on-failure") => Some("default"),
            _ => None,
        }
    }

    fn listen_url(socket: &Path) -> String {
        format!("unix://{}", socket.display())
    }

    fn discard(socket: &Path, pid_file: &Path) {
        let _ = std::fs::remove_file(socket);
        orphan::clear(pid_file);
    }

    type Socket = tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>;

    /// Polls until the socket takes a connection, then upgrades it; fails if the app-server dies first.
    async fn connect(socket: &Path, child: &mut tokio::process::Child) -> Result<Socket> {
        let deadline = Instant::now() + CONNECT_WAIT;
        let stream = loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(failed(format!(
                    "codex app-server exited at startup ({status})"
                )));
            }
            if let Ok(stream) = tokio::net::UnixStream::connect(socket).await {
                break stream;
            }
            if Instant::now() >= deadline {
                return Err(failed("codex app-server never opened its socket"));
            }
            tokio::time::sleep(CONNECT_POLL).await;
        };
        match tokio::time::timeout(UPGRADE_WAIT, client_async("ws://localhost/", stream)).await {
            Ok(Ok((ws, _))) => Ok(ws),
            Ok(Err(e)) => Err(failed(format!(
                "codex app-server refused the WebSocket upgrade: {e}"
            ))),
            Err(_) => Err(failed(
                "codex app-server never answered the WebSocket upgrade",
            )),
        }
    }

    fn failed(detail: impl Into<String>) -> CoreError {
        CoreError::PtyControlFailed {
            detail: detail.into(),
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Never built off Unix: codex stays embedded there, so no value exists.
#[cfg(not(unix))]
pub(crate) enum CodexRemote {}

#[cfg(not(unix))]
impl CodexRemote {
    pub(crate) fn url(&self) -> String {
        match *self {}
    }

    pub(crate) fn mode_reported(&self) -> bool {
        match *self {}
    }

    pub(crate) fn stop(&self) {
        match *self {}
    }

    pub(crate) fn running(&self) -> bool {
        match *self {}
    }

    pub(crate) fn kill_now(&self) {
        match *self {}
    }

    pub(crate) fn adopt_thread(self: &std::sync::Arc<Self>, _thread: &str) {
        match **self {}
    }

    pub(crate) async fn set_model(&self, _model: &str) -> crate::error::Result<()> {
        match *self {}
    }

    pub(crate) async fn set_effort(&self, _effort: &str) -> crate::error::Result<()> {
        match *self {}
    }

    pub(crate) async fn set_permission_mode(&self, _key: &str) -> crate::error::Result<()> {
        match *self {}
    }
}
