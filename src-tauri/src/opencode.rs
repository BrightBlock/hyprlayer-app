//! The OpenCode harness driver. `opencode serve` runs as one long-lived local
//! server shared by every session ([`OpenCodeManager`]); each session's
//! [`OpenCodeDriver`] filters the global SSE stream by session id and translates
//! OpenCode events into harness-agnostic [`SessionEvent`]s.
//!
//! Wire reference, confirmed against `opencode serve` OpenAPI (v1.17):
//!   - `GET  /global/health`            → `{ healthy, version }`
//!   - `GET  /global/event`             → SSE of `{ directory, project, payload:{ id, type, properties } }`
//!     (the unscoped, all-directories stream — `/event` alone is scoped to the
//!     server's cwd, so it misses sessions created in a worktree directory)
//!   - `POST /session?directory=<cwd>`  → `{ id: "ses_…", … }` (cwd scopes tools)
//!   - `POST /session/{id}/prompt_async`→ `{ parts:[{type:"text",text}], model:{providerID,modelID} }`
//!   - `POST /session/{id}/permissions/{permissionID}` → `{ response: once|always|reject }`
//!   - `POST /session/{id}/abort`
//!   - `DELETE /session/{id}`            → remove a session (used for throwaways)

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::{ExitOutcome, SessionEvent};
use hyprlayer_core::error::Result as CoreResult;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

const EVENT_CAP: usize = 1024;

/// A parsed OpenCode SSE event (`{ id, type, properties }`); heartbeats dropped.
#[derive(Clone)]
struct OcEvent {
    kind: String,
    properties: Value,
}

/// Lazily-started, app-wide `opencode serve` plus its global SSE fan-out.
#[derive(Default)]
pub struct OpenCodeManager {
    inner: Mutex<Option<Arc<OpenCodeServer>>>,
}

impl OpenCodeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The shared server, starting it on first use (or restarting it if the
    /// previous one died). `bin` is the resolved `opencode` path; `path` is the
    /// augmented PATH spawned children inherit; `data_dir` is the app-owned
    /// OpenCode data home (see [`OpenCodeServer::start`]).
    pub async fn ensure(
        &self,
        bin: PathBuf,
        path: OsString,
        data_dir: PathBuf,
    ) -> Result<Arc<OpenCodeServer>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_ref() {
            if s.is_healthy().await {
                return Ok(s.clone());
            }
        }
        let server = Arc::new(OpenCodeServer::start(bin, path, data_dir).await?);
        *guard = Some(server.clone());
        Ok(server)
    }

    /// Drop the cached server so the next [`ensure`](Self::ensure) spawns a fresh
    /// one. Call this whenever provider credentials change (a key added, updated,
    /// or removed): OpenCode reads `auth.json` only once, at `serve` startup —
    /// we inject it via `OPENCODE_AUTH_CONTENT` in [`OpenCodeServer::start`] — and
    /// never re-scans it, so a running server can't see a newly-added provider and
    /// rejects its models with `ProviderModelNotFoundError` until restarted. This
    /// avoids forcing a full app restart for that.
    ///
    /// In-flight sessions are undisturbed: each [`OpenCodeDriver`] holds its own
    /// `Arc<OpenCodeServer>`, so the old process keeps serving them (and is killed
    /// when its last session drops); only newly-started sessions get the
    /// refreshed server that re-reads the updated `auth.json`.
    pub async fn invalidate(&self) {
        *self.inner.lock().await = None;
    }
}

pub struct OpenCodeServer {
    base_url: String,
    client: reqwest::Client,
    events: broadcast::Sender<OcEvent>,
    /// `provider/model` id → context window (tokens), read once from
    /// `GET /config/providers` at startup. This is OpenCode's own resolved limit
    /// for each model (the value it runs the agentic loop against), so it's the
    /// authoritative meter denominator for hosted providers — the UI can't infer
    /// it from the id. Empty if the fetch failed (the UI then falls back).
    model_context: std::collections::HashMap<String, u32>,
    /// Kept alive for the app's lifetime. Supervised because on Windows this is
    /// a `cmd.exe` shim wrapping the real server — killing the pid we hold would
    /// leave the server listening. See `process::spawn_supervised`.
    _child: Mutex<hyprlayer_core::process::SupervisedChild>,
}

impl OpenCodeServer {
    /// Spawn `opencode serve` against an **app-owned data home** (`XDG_DATA_HOME`
    /// = `data_dir`) so OpenCode's session/message database is created and
    /// migrated by us, never sharing the user's global `~/.local/share/opencode`
    /// (which can carry a stale/incompatible schema from another OpenCode build).
    /// Provider credentials still come from the user's real `auth.json`, injected
    /// via `OPENCODE_AUTH_CONTENT` — OpenCode (1.17) reads auth only from
    /// `$XDG_DATA_HOME/opencode/auth.json` or this env var; it ignores
    /// `OPENCODE_AUTH_JSON`. So keys configured in the OpenCode settings tab keep
    /// working even though the data home is app-owned.
    async fn start(bin: PathBuf, path: OsString, data_dir: PathBuf) -> Result<Self, String> {
        let port = free_port()?;
        let base_url = format!("http://127.0.0.1:{port}");
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let _ = std::fs::create_dir_all(&data_dir);

        let mut cmd = Command::new(&bin);
        cmd.args(["serve", "--hostname", "127.0.0.1", "--port", &port.to_string()])
            .current_dir(&home)
            .env("PATH", &path)
            .env("XDG_DATA_HOME", &data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Keep using the user's real provider credentials (written by the
        // OpenCode settings tab) even though the data home is app-owned. Inject
        // the auth.json *contents* (the path-based OPENCODE_AUTH_JSON is ignored).
        if let Some(content) = hyprlayer_core::opencode::config::auth_json_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .filter(|c| !c.trim().is_empty())
        {
            cmd.env("OPENCODE_AUTH_CONTENT", content);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            // tokio's Command has an inherent creation_flags (no CommandExt needed).
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = hyprlayer_core::process::spawn_supervised(&mut cmd)
            .map_err(|e| format!("spawn `opencode serve`: {e}"))?;

        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| e.to_string())?;

        // Wait for the server to come up (≈15s budget).
        let mut healthy = false;
        for _ in 0..60 {
            if let Ok(resp) = client
                .get(format!("{base_url}/global/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                if let Ok(v) = resp.json::<Value>().await {
                    if v.get("healthy").and_then(Value::as_bool) == Some(true) {
                        healthy = true;
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !healthy {
            return Err("`opencode serve` did not become healthy in time".into());
        }

        // Read each model's context window once, now that the server is up. Best
        // effort: a failure just leaves the map empty (the UI falls back). The
        // server is shared + recycled on auth changes, so this stays in step with
        // the configured providers.
        let model_context = fetch_model_context(&client, &base_url).await;

        let (events, _) = broadcast::channel(EVENT_CAP);
        spawn_sse(base_url.clone(), client.clone(), events.clone());

        Ok(Self {
            base_url,
            client,
            events,
            model_context,
            _child: Mutex::new(child),
        })
    }

    /// The context window (tokens) OpenCode runs `model_id` (`provider/model`)
    /// with, when known — the meter's denominator. `None` falls back to the UI's
    /// own table.
    fn context_window(&self, model_id: &str) -> Option<u32> {
        self.model_context.get(model_id).copied()
    }

    async fn is_healthy(&self) -> bool {
        self.client
            .get(format!("{}/global/health", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// Consume the global SSE stream, re-broadcasting parsed events. Reconnects if
/// the stream drops (e.g. a transient hiccup); ends only when the process exits.
///
/// Subscribes to `/global/event` (all directories), NOT `/event` (scoped to the
/// server's cwd): sessions are created against their worktree directory, so the
/// cwd-scoped stream would never see their `message.*`/`session.*` events and the
/// chat would show only the initial `init`.
fn spawn_sse(base_url: String, client: reqwest::Client, tx: broadcast::Sender<OcEvent>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Ok(mut resp) = client.get(format!("{base_url}/global/event")).send().await {
                let mut buf = String::new();
                while let Ok(Some(bytes)) = resp.chunk().await {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    // SSE records are separated by a blank line.
                    while let Some(idx) = buf.find("\n\n") {
                        let record: String = buf.drain(..idx + 2).collect();
                        if let Some(ev) = parse_sse_record(&record) {
                            let _ = tx.send(ev);
                        }
                    }
                }
            }
            // Server gone or stream dropped — back off, then retry.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

fn parse_sse_record(record: &str) -> Option<OcEvent> {
    let mut data = String::new();
    for line in record.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(&data).ok()?;
    // `/global/event` wraps the real event under `payload` (alongside top-level
    // `directory`/`project`); fall back to `v` itself so a flat `/event`-shaped
    // record (no `payload`) still parses.
    let event = v.get("payload").unwrap_or(&v);
    let kind = event.get("type")?.as_str()?.to_string();
    if kind == "server.heartbeat" {
        return None;
    }
    Some(OcEvent {
        kind,
        properties: event.get("properties").cloned().unwrap_or(Value::Null),
    })
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("allocate a local port: {e}"))
}

/// Read each configured model's context window from `GET /config/providers` and
/// index it by `provider/model` id (the same id the composer sends). OpenCode
/// resolves these limits itself (models.dev + any provider config), so this is
/// the authoritative denominator for hosted providers. Best effort — a transport
/// or parse failure yields an empty map and the UI falls back to its own table.
async fn fetch_model_context(
    client: &reqwest::Client,
    base_url: &str,
) -> std::collections::HashMap<String, u32> {
    let body = match client
        .get(format!("{base_url}/config/providers"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(r) => match r.text().await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("opencode /config/providers read failed: {e}");
                return std::collections::HashMap::new();
            }
        },
        Err(e) => {
            log::warn!("opencode /config/providers fetch failed: {e}");
            return std::collections::HashMap::new();
        }
    };
    parse_model_context(&body)
}

/// Pure parse of `GET /config/providers` → `provider/model` id → context window.
/// Tolerant: unknown fields ignored, models without a positive `limit.context`
/// skipped. Split out from the HTTP so it's unit-testable against a captured body.
fn parse_model_context(body: &str) -> std::collections::HashMap<String, u32> {
    #[derive(serde::Deserialize)]
    struct Resp {
        #[serde(default)]
        providers: Vec<ProviderEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ProviderEntry {
        id: String,
        #[serde(default)]
        models: std::collections::HashMap<String, ModelEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        #[serde(default)]
        limit: Option<LimitEntry>,
    }
    #[derive(serde::Deserialize)]
    struct LimitEntry {
        #[serde(default)]
        context: Option<u64>,
    }

    let mut out = std::collections::HashMap::new();
    let Ok(parsed) = serde_json::from_str::<Resp>(body) else {
        log::warn!("opencode /config/providers parse failed");
        return out;
    };
    for p in parsed.providers {
        for (model_id, m) in p.models {
            if let Some(ctx) = m.limit.and_then(|l| l.context).filter(|c| *c > 0) {
                out.insert(format!("{}/{model_id}", p.id), ctx.min(u32::MAX as u64) as u32);
            }
        }
    }
    out
}

/// A single OpenCode session.
pub struct OpenCodeDriver {
    server: Arc<OpenCodeServer>,
    session_id: String,
    /// `(providerID, modelID)` to send with each prompt, if a model was chosen.
    /// Behind a `Mutex` so it can be switched live mid-session (OpenCode takes a
    /// model per prompt) via [`AgentDriver::set_model`].
    model: std::sync::Mutex<Option<(String, String)>>,
    events: broadcast::Sender<SessionEvent>,
    killed: Arc<AtomicBool>,
    filter_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Pending `question.asked` payloads (questionID → its `questions` array).
    /// The UI answers with a `{question text → label}` map; OpenCode wants
    /// positional `{answers: [[label,…],…]}`, so we remember each question to map
    /// back. Recorded by the SSE filter task, drained by `answer_question`.
    pending_questions: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
}

impl OpenCodeDriver {
    /// Create an OpenCode session on the shared server and start translating its
    /// slice of the global event stream. The first prompt is sent separately via
    /// [`AgentDriver::send_user_turn`], mirroring the Claude launch flow.
    ///
    /// `cwd` is the workstream checkout the session runs in. The shared server is
    /// started from `$HOME`, so without this every session (and its tools) would
    /// default to the home directory; `?directory=` scopes the session to the
    /// worktree at create time and OpenCode persists it for the session's prompts.
    pub async fn spawn(
        server: Arc<OpenCodeServer>,
        cwd: &str,
        model: Option<String>,
    ) -> Result<(Arc<Self>, broadcast::Receiver<SessionEvent>), String> {
        let created: Value = server
            .client
            .post(format!("{}/session", server.base_url))
            .query(&[("directory", cwd)])
            .json(&json!({ "title": "Hyprlayer session" }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let session_id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or("OpenCode session create returned no id")?
            .to_string();

        let model = model.as_deref().and_then(parse_model);

        let (events, _) = broadcast::channel(EVENT_CAP);
        let primary_rx = events.subscribe();

        // Init first, so the forwarder records the harness session id (used for
        // resume) and flips the row to running.
        let _ = events.send(SessionEvent::Init {
            claude_session_id: session_id.clone(),
            model: model.as_ref().map(|(p, m)| format!("{p}/{m}")),
        });

        // Tell the UI this model's real context window — the meter's denominator —
        // so it doesn't fall back to the Claude 1M default. Ollama models run with
        // a fixed `num_ctx` we set ourselves (authoritative, and the UI can't infer
        // it); every other provider uses the limit OpenCode resolved at startup.
        // `None` (model unknown) keeps the UI's own per-model table.
        if let Some(tokens) = model.as_ref().and_then(|(p, m)| {
            let id = format!("{p}/{m}");
            hyprlayer_core::opencode::config::opencode_model_context_window(&id)
                .or_else(|| server.context_window(&id))
        }) {
            let _ = events.send(SessionEvent::ContextWindow { tokens });
        }

        // Filter the global stream by this session and translate. The translator
        // is stateful (tracks message roles + per-part streaming progress), so it
        // lives for the life of the task.
        let mut sse_rx = server.events.subscribe();
        let tx = events.clone();
        let sid = session_id.clone();
        let pending_questions: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>> =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let pq = pending_questions.clone();
        let task = tauri::async_runtime::spawn(async move {
            let mut xlate = Translator::default();
            loop {
                match sse_rx.recv().await {
                    Ok(ev) => {
                        if event_session_id(&ev).as_deref() != Some(sid.as_str()) {
                            continue;
                        }
                        // Remember a question's payload so the user's answers can be
                        // mapped back into OpenCode's positional reply format.
                        if ev.kind == "question.asked" {
                            if let Some(qid) = ev.properties.get("id").and_then(Value::as_str) {
                                if let Ok(mut m) = pq.lock() {
                                    m.insert(
                                        qid.to_string(),
                                        ev.properties.get("questions").cloned().unwrap_or(json!([])),
                                    );
                                }
                            }
                        }
                        for se in xlate.handle(&ev) {
                            let _ = tx.send(se);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let driver = Arc::new(Self {
            server,
            session_id,
            model: std::sync::Mutex::new(model),
            events,
            killed: Arc::new(AtomicBool::new(false)),
            filter_task: Mutex::new(Some(task)),
            pending_questions,
        });
        Ok((driver, primary_rx))
    }

    /// Fire an HTTP POST without blocking the (sync) trait method; failures land
    /// in the log (and, for prompts, surface as an absent response on the stream).
    fn post_async(&self, path: String, body: Value) {
        let client = self.server.client.clone();
        let url = format!("{}{}", self.server.base_url, path);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = client.post(url).json(&body).send().await {
                log::warn!("opencode POST failed: {e}");
            }
        });
    }

    /// Send a prompt. Unlike [`post_async`], a transport error or non-2xx
    /// response is surfaced on the session stream and the turn is ended — a
    /// failed prompt (dead server, bad auth, unknown model, 5xx) otherwise leaves
    /// the UI stuck "running" with no visible failure, since `prompt_async`
    /// returns before the turn produces any `session.*` events.
    fn prompt_async(&self, body: Value) {
        let client = self.server.client.clone();
        let url = format!(
            "{}/session/{}/prompt_async",
            self.server.base_url, self.session_id
        );
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            let result = client
                .post(url)
                .json(&body)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(e) = result {
                log::warn!("opencode prompt failed: {e}");
                let _ = events.send(SessionEvent::AssistantText {
                    text: format!("⚠ OpenCode request failed: {e}"),
                    parent_id: None,
                });
                let _ = events.send(SessionEvent::TurnDone {
                    cost_usd: None,
                    num_turns: None,
                    context_tokens: None,
                });
            }
        });
    }

    /// Remove this session from the server's store. Throwaway sessions (e.g.
    /// one-shot title generation) call this so they don't accumulate; interactive
    /// sessions keep theirs for resume and only [`AgentDriver::kill`]. Best-effort
    /// — a failure just leaves the record behind, so the caller can ignore it.
    pub async fn delete_session(&self) {
        let url = format!("{}/session/{}", self.server.base_url, self.session_id);
        if let Err(e) = self
            .server
            .client
            .delete(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            log::warn!("opencode session delete failed: {e}");
        }
    }
}

impl AgentDriver for OpenCodeDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> CoreResult<()> {
        // Persist/replay the human turn the same way the Claude driver does.
        let _ = self.events.send(SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
        });
        let mut body = json!({ "parts": [{ "type": "text", "text": text }] });
        // Read the (possibly live-switched) model under the lock.
        if let Some((provider, model)) = self.model.lock().ok().and_then(|m| m.clone()) {
            body["model"] = json!({ "providerID": provider, "modelID": model });
        }
        self.prompt_async(body);
        Ok(())
    }

    fn set_model(&self, model: Option<String>) {
        // OpenCode honors a per-prompt model, so switching mid-session just swaps
        // what the next turn sends. `provider/model` ids only; anything that
        // doesn't split into both clears the override (falls back to the default).
        if let Ok(mut m) = self.model.lock() {
            *m = model.as_deref().and_then(parse_model);
        }
    }

    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        _message: Option<&str>,
    ) -> CoreResult<()> {
        let response = if allow { "once" } else { "reject" };
        self.post_async(
            format!("/session/{}/permissions/{}", self.session_id, request_id),
            json!({ "response": response }),
        );
        Ok(())
    }

    fn answer_question(&self, request_id: &str, answers: Value) -> CoreResult<()> {
        // The UI sends `{ "<question text>": "<label>" }` (multi-select labels are
        // comma-joined); OpenCode wants positional `{ answers: [[label,…],…] }` in
        // question order. Map back through the remembered question payload. An empty
        // map is the UI's "dismiss" → reject the question instead of replying.
        let questions = self
            .pending_questions
            .lock()
            .ok()
            .and_then(|mut m| m.remove(request_id));
        let empty = answers.as_object().map(|o| o.is_empty()).unwrap_or(true);
        if empty {
            self.post_async(format!("/question/{request_id}/reject"), json!({}));
            return Ok(());
        }
        let ordered: Vec<Vec<String>> = questions
            .as_ref()
            .and_then(Value::as_array)
            .map(|qs| {
                qs.iter()
                    .map(|q| {
                        let text = q.get("question").and_then(Value::as_str).unwrap_or_default();
                        answers
                            .get(text)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(|s| s.split(", ").map(str::to_string).collect())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.post_async(
            format!("/question/{request_id}/reply"),
            json!({ "answers": ordered }),
        );
        Ok(())
    }

    fn interrupt(&self) -> CoreResult<()> {
        self.post_async(format!("/session/{}/abort", self.session_id), json!({}));
        Ok(())
    }

    fn kill(&self) {
        if self.killed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.post_async(format!("/session/{}/abort", self.session_id), json!({}));
        if let Ok(mut guard) = self.filter_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        // The shared server stays up; this session is done.
        let _ = self.events.send(SessionEvent::Exited {
            outcome: ExitOutcome::Killed,
            code: None,
            stderr_tail: String::new(),
        });
    }

    fn pid(&self) -> Option<u32> {
        None // backed by the shared server, not a per-session process
    }
}

/// Split a `"provider/model"` id into `(providerID, modelID)`.
fn parse_model(id: &str) -> Option<(String, String)> {
    id.split_once('/')
        .map(|(p, m)| (p.to_string(), m.to_string()))
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

fn event_session_id(ev: &OcEvent) -> Option<String> {
    ev.properties
        .get("sessionID")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Translates OpenCode's event stream into [`SessionEvent`]s. Stateful, because
/// OpenCode streams content as repeated `message.part.updated` events that grow a
/// part in place (text accrues; tools transition pending→running→completed), and
/// a part's role is only known from the owning message. One translator per
/// session lives for the session's lifetime.
///
/// OpenCode's content model (confirmed against the v1.17 OpenAPI + a live trace):
///   - `message.updated`      → the message (carries `role`: user|assistant)
///   - `message.part.updated` → a `Part` (text | reasoning | tool | step-finish …)
///   - `session.idle`         → the turn finished
///   - `session.error`        → the turn failed (surface the message)
#[derive(Default)]
struct Translator {
    /// messageID → role ("user" | "assistant").
    roles: std::collections::HashMap<String, String>,
    /// text/reasoning part id → bytes already streamed as PartialText (used for
    /// the legacy grow-in-place path on builds without `message.part.delta`).
    text_len: std::collections::HashMap<String, usize>,
    /// text/reasoning part id → final block already emitted.
    text_done: std::collections::HashSet<String>,
    /// part ids that streamed via `message.part.delta` — their growing
    /// `message.part.updated` text must NOT be re-emitted as PartialText.
    delta_parts: std::collections::HashSet<String>,
    /// part id → part type ("text" | "reasoning" | "tool" | …), learned from
    /// `message.part.updated`. A part is announced before its deltas stream, so
    /// this lets the delta handler tell a reasoning part's text (which carries
    /// `field:"text"` too) from real assistant text.
    part_types: std::collections::HashMap<String, String>,
    /// tool callID → ToolUse already emitted.
    tool_started: std::collections::HashSet<String>,
    /// tool callID → ToolResult already emitted.
    tool_done: std::collections::HashSet<String>,
}

impl Translator {
    fn handle(&mut self, ev: &OcEvent) -> Vec<SessionEvent> {
        let p = &ev.properties;
        match ev.kind.as_str() {
            "message.updated" => {
                if let Some(info) = p.get("info") {
                    if let (Some(id), Some(role)) = (
                        info.get("id").and_then(Value::as_str),
                        info.get("role").and_then(Value::as_str),
                    ) {
                        self.roles.insert(id.to_string(), role.to_string());
                    }
                }
                Vec::new()
            }
            // Live streaming on newer builds: incremental text/reasoning deltas.
            "message.part.delta" => {
                let mid = p.get("messageID").and_then(Value::as_str).unwrap_or_default();
                if self.roles.get(mid).map(String::as_str) != Some("assistant") {
                    return Vec::new();
                }
                let pid = p.get("partID").and_then(Value::as_str).unwrap_or_default();
                if !pid.is_empty() {
                    self.delta_parts.insert(pid.to_string());
                }
                // Reasoning parts stream their text via deltas too (also `field:
                // "text"`), but should surface only as a Thinking block when the
                // part completes — never as live assistant text. The owning part's
                // type was announced before these deltas.
                if self.part_types.get(pid).map(String::as_str) == Some("reasoning") {
                    return Vec::new();
                }
                let field = p.get("field").and_then(Value::as_str).unwrap_or("text");
                let delta = p.get("delta").and_then(Value::as_str).unwrap_or_default();
                // Only text streams live; reasoning is surfaced as a Thinking block
                // when its part completes (no partial-thinking event exists).
                if field == "text" && !delta.is_empty() {
                    vec![SessionEvent::PartialText {
                        text: delta.to_string(),
                        parent_id: None,
                    }]
                } else {
                    Vec::new()
                }
            }
            "message.part.updated" => self.handle_part(p.get("part").unwrap_or(&Value::Null)),
            "permission.asked" => vec![SessionEvent::PermissionAsk {
                request_id: str_field(p, "id"),
                tool_name: {
                    let t = str_field(p, "permission");
                    if t.is_empty() {
                        "tool".to_string()
                    } else {
                        t
                    }
                },
                input: json!({
                    "patterns": p.get("patterns").cloned().unwrap_or(json!([])),
                    "metadata": p.get("metadata").cloned().unwrap_or(json!({})),
                })
                .to_string(),
            }],
            // The model called the `question` tool to ask the user something.
            // OpenCode's `questions` array (question/header/options[label,desc])
            // matches our QuestionAsk shape, so surface it interactively — the
            // matching `question` tool part is suppressed in `handle_part`.
            "question.asked" => vec![SessionEvent::QuestionAsk {
                request_id: str_field(p, "id"),
                questions: p.get("questions").cloned().unwrap_or(json!([])).to_string(),
            }],
            "session.idle" => vec![SessionEvent::TurnDone {
                cost_usd: None,
                num_turns: None,
                context_tokens: None,
            }],
            "session.error" => {
                let err = p.get("error");
                let name = err
                    .and_then(|e| e.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let msg = err
                    .and_then(|e| e.get("data"))
                    .and_then(|d| d.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("OpenCode reported an error")
                    .to_string();
                // A user-initiated stop aborts the turn — that's not an error, so
                // end the turn cleanly (the partial output stays) rather than
                // surfacing a scary banner.
                if name == "MessageAbortedError" || msg == "Aborted" {
                    return vec![SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                    }];
                }
                vec![
                    SessionEvent::AssistantText {
                        text: format!("⚠ OpenCode error: {msg}"),
                        parent_id: None,
                    },
                    SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                    },
                ]
            }
            _ => Vec::new(),
        }
    }

    fn handle_part(&mut self, part: &Value) -> Vec<SessionEvent> {
        let pid = part.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let ptype = part.get("type").and_then(Value::as_str).unwrap_or_default();
        // Remember this part's type *before* the role gate. A reasoning part can be
        // announced (this `message.part.updated`) before its owning message's role
        // is known; if we skipped it here, its later `field:"text"` deltas — which
        // the delta handler suppresses only when the part is known to be reasoning
        // — would be misclassified as visible assistant text, leaking the thought
        // into prose. Recording it unconditionally keeps that suppression reliable
        // regardless of message/part event ordering. (Harmless for user parts: the
        // delta handler gates on role before ever consulting this map.)
        if !pid.is_empty() {
            self.part_types.insert(pid.clone(), ptype.to_string());
        }
        // Only assistant output: the user's echoed prompt part is already shown
        // (we emit UserMessage in send_user_turn). Roles arrive via message.updated
        // before the message's parts, so an unknown role means "not assistant yet".
        let mid = part.get("messageID").and_then(Value::as_str).unwrap_or_default();
        if self.roles.get(mid).map(String::as_str) != Some("assistant") {
            return Vec::new();
        }
        let mut out = Vec::new();

        match ptype {
            "text" => {
                let full = part.get("text").and_then(Value::as_str).unwrap_or_default();
                // Legacy path (no `message.part.delta`): stream the newly-grown
                // suffix. Skip when deltas already streamed this part live.
                if !self.delta_parts.contains(&pid) {
                    let emitted = self.text_len.get(&pid).copied().unwrap_or(0);
                    // `emitted` is the byte length of a previous identical prefix,
                    // so it's always a valid UTF-8 boundary in `full`.
                    if full.len() > emitted {
                        out.push(SessionEvent::PartialText {
                            text: full[emitted..].to_string(),
                            parent_id: None,
                        });
                        self.text_len.insert(pid.clone(), full.len());
                    }
                }
                if part_ended(part) && self.text_done.insert(pid) {
                    out.push(SessionEvent::AssistantText {
                        text: full.to_string(),
                        parent_id: None,
                    });
                }
            }
            "reasoning" => {
                if part_ended(part) && self.text_done.insert(pid) {
                    let full = part.get("text").and_then(Value::as_str).unwrap_or_default();
                    out.push(SessionEvent::Thinking {
                        text: full.to_string(),
                        parent_id: None,
                    });
                }
            }
            "tool" => {
                let call_id = part.get("callID").and_then(Value::as_str).unwrap_or_default().to_string();
                let tool = part.get("tool").and_then(Value::as_str).unwrap_or_default().to_string();
                // The `question` tool is surfaced interactively via `question.asked`
                // (→ QuestionAsk). Don't also render it as a generic tool card, which
                // would sit "working" forever while waiting for the answer.
                if tool == "question" {
                    return out;
                }
                let state = part.get("state").cloned().unwrap_or(Value::Null);
                let status = state.get("status").and_then(Value::as_str).unwrap_or_default();
                if !call_id.is_empty()
                    && status != "pending"
                    && self.tool_started.insert(call_id.clone())
                {
                    let (name, input) =
                        normalize_tool(&tool, state.get("input").cloned().unwrap_or(json!({})));
                    out.push(SessionEvent::ToolUse {
                        id: call_id.clone(),
                        name,
                        input: input.to_string(),
                        parent_id: None,
                    });
                }
                match status {
                    "completed" if self.tool_done.insert(call_id.clone()) => {
                        out.push(SessionEvent::ToolResult {
                            tool_use_id: call_id,
                            content: value_to_text(state.get("output")),
                            is_error: false,
                            parent_id: None,
                        });
                    }
                    "error" if self.tool_done.insert(call_id.clone()) => {
                        out.push(SessionEvent::ToolResult {
                            tool_use_id: call_id,
                            content: value_to_text(state.get("error")),
                            is_error: true,
                            parent_id: None,
                        });
                    }
                    _ => {}
                }
            }
            "step-finish" => {
                let cost = part.get("cost").and_then(Value::as_f64);
                let ctx = part.get("tokens").map(|t| {
                    let f = |k: &str| t.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                    // OpenCode's `total` already sums input+output+reasoning; fall
                    // back to input+output on older shapes that omit it.
                    let total = f("total");
                    (if total > 0.0 { total } else { f("input") + f("output") }) as u32
                });
                out.push(SessionEvent::TurnDone {
                    cost_usd: cost,
                    num_turns: None,
                    context_tokens: ctx,
                });
            }
            _ => {}
        }
        out
    }
}

fn str_field(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Map an OpenCode tool call onto the Claude-Code tool vocabulary the transcript
/// UI is built around. OpenCode emits lowercase names (`read`, `edit`) with
/// camelCase input keys (`filePath`, `oldString`); the frontend keys every
/// display path — standalone per-file chips, edit-diff previews, agent-doc
/// detection, argument summaries — on Claude's PascalCase names and snake_case
/// keys. Normalizing here (the harness-agnostic driver boundary) makes all of
/// that work for OpenCode unchanged; without it, read/write/edit fall through to
/// the generic path and collapse into a "N tool calls" group instead of showing
/// as their own file rows. Unknown tools pass through with input untouched.
fn normalize_tool(tool: &str, mut input: Value) -> (String, Value) {
    fn rename(input: &mut Value, from: &str, to: &str) {
        if let Some(obj) = input.as_object_mut() {
            if let Some(v) = obj.remove(from) {
                obj.insert(to.to_string(), v);
            }
        }
    }
    let name = match tool {
        "read" => {
            rename(&mut input, "filePath", "file_path");
            "Read"
        }
        "write" => {
            rename(&mut input, "filePath", "file_path");
            "Write"
        }
        "edit" => {
            rename(&mut input, "filePath", "file_path");
            rename(&mut input, "oldString", "old_string");
            rename(&mut input, "newString", "new_string");
            "Edit"
        }
        "bash" => "Bash",
        "grep" => "Grep",
        "glob" => "Glob",
        "webfetch" => "WebFetch",
        "task" => {
            rename(&mut input, "subagentType", "subagent_type");
            "Task"
        }
        "todowrite" => "TodoWrite",
        // Everything else (list, patch, custom MCP tools, …) keeps OpenCode's
        // name and input — it renders via the generic summary fallback.
        _ => return (tool.to_string(), input),
    };
    (name.to_string(), input)
}

/// A streaming part is complete once its `time.end` is set.
fn part_ended(part: &Value) -> bool {
    part.get("time").and_then(|t| t.get("end")).is_some()
}

/// Best-effort string form of a tool's output/error value.
fn value_to_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_file_tools_to_claude_vocabulary() {
        // Input shapes captured from a live `opencode serve` (v1.17.10) trace.
        let (name, input) = normalize_tool("read", json!({ "filePath": "/w/a.txt" }));
        assert_eq!(name, "Read");
        assert_eq!(input, json!({ "file_path": "/w/a.txt" }));

        let (name, input) =
            normalize_tool("write", json!({ "filePath": "/w/b.txt", "content": "x" }));
        assert_eq!(name, "Write");
        assert_eq!(input, json!({ "file_path": "/w/b.txt", "content": "x" }));

        let (name, input) = normalize_tool(
            "edit",
            json!({ "filePath": "/w/a.txt", "oldString": "a", "newString": "b" }),
        );
        assert_eq!(name, "Edit");
        // old_string/new_string are what the UI's edit-diff preview reads.
        assert_eq!(
            input,
            json!({ "file_path": "/w/a.txt", "old_string": "a", "new_string": "b" })
        );
    }

    #[test]
    fn parse_model_context_indexes_by_provider_slash_model() {
        // Shape captured from `GET /config/providers` (opencode 1.17.10): models
        // are keyed by an id that can itself contain slashes; each carries
        // `limit.context`. The map key must be `provider/<model id>` so it matches
        // the `provider/model` id the composer sends.
        let body = r#"{
            "providers": [
                {
                    "id": "fireworks-ai",
                    "name": "Fireworks AI",
                    "models": {
                        "accounts/fireworks/routers/kimi-k2p6-turbo": {
                            "id": "accounts/fireworks/routers/kimi-k2p6-turbo",
                            "limit": { "context": 262000, "output": 262000 }
                        }
                    }
                },
                {
                    "id": "openrouter",
                    "models": {
                        "no-limit-model": { "id": "no-limit-model" },
                        "zero-ctx-model": { "id": "zero-ctx-model", "limit": { "context": 0 } }
                    }
                }
            ]
        }"#;
        let map = parse_model_context(body);
        assert_eq!(
            map.get("fireworks-ai/accounts/fireworks/routers/kimi-k2p6-turbo"),
            Some(&262_000)
        );
        // Models without a positive context limit are skipped (no bogus denominator).
        assert!(!map.contains_key("openrouter/no-limit-model"));
        assert!(!map.contains_key("openrouter/zero-ctx-model"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_model_context_tolerates_garbage() {
        assert!(parse_model_context("not json").is_empty());
        assert!(parse_model_context(r#"{"providers":[]}"#).is_empty());
    }

    #[test]
    fn normalize_passes_through_unknown_tools() {
        let original = json!({ "anything": 1 });
        let (name, input) = normalize_tool("some_mcp_tool", original.clone());
        assert_eq!(name, "some_mcp_tool");
        assert_eq!(input, original);
    }

    #[test]
    fn normalize_renames_non_file_tools_without_touching_input() {
        // grep/glob/bash already share Claude's key names — only the name maps.
        let (name, input) = normalize_tool("grep", json!({ "pattern": "DONE", "path": "/w" }));
        assert_eq!(name, "Grep");
        assert_eq!(input, json!({ "pattern": "DONE", "path": "/w" }));

        let (name, _) = normalize_tool("bash", json!({ "command": "echo hi" }));
        assert_eq!(name, "Bash");
    }

    fn oc(kind: &str, properties: Value) -> OcEvent {
        OcEvent { kind: kind.to_string(), properties }
    }

    #[test]
    fn reasoning_announced_before_role_is_not_leaked_as_text() {
        // Regression: a reasoning part can be announced before its message's role
        // is known. Its text streams as `field:"text"` deltas under the reasoning
        // part id, which must be suppressed and surfaced only as a Thinking block —
        // never as visible assistant text. This works only if the part's type is
        // recorded even while the role is still unknown.
        let mut x = Translator::default();

        // 1. Reasoning part announced BEFORE the owning message's role arrives.
        let out = x.handle(&oc(
            "message.part.updated",
            json!({ "part": { "id": "r1", "type": "reasoning", "messageID": "m1" } }),
        ));
        assert!(out.is_empty());

        // 2. Now the role lands.
        x.handle(&oc("message.updated", json!({ "info": { "id": "m1", "role": "assistant" } })));

        // 3. Reasoning streams as field:"text" deltas — must be suppressed.
        let out = x.handle(&oc(
            "message.part.delta",
            json!({ "messageID": "m1", "partID": "r1", "field": "text", "delta": "secret plan" }),
        ));
        assert!(
            out.is_empty(),
            "reasoning delta leaked as a live event: {out:?}"
        );

        // 4. The reasoning part completes → exactly one Thinking block, no text.
        let out = x.handle(&oc(
            "message.part.updated",
            json!({ "part": { "id": "r1", "type": "reasoning", "messageID": "m1", "text": "secret plan", "time": { "end": 1 } } }),
        ));
        assert_eq!(out.len(), 1);
        match &out[0] {
            SessionEvent::Thinking { text, .. } => assert_eq!(text, "secret plan"),
            other => panic!("expected Thinking, got {other:?}"),
        }
        assert!(
            !out.iter().any(|e| matches!(
                e,
                SessionEvent::PartialText { .. } | SessionEvent::AssistantText { .. }
            )),
            "reasoning surfaced as assistant text"
        );
    }

    #[test]
    fn global_event_record_unwraps_payload() {
        // `/global/event` wraps the real event under `payload`.
        let rec = "data: {\"directory\":\"/w\",\"payload\":{\"type\":\"message.updated\",\"properties\":{\"x\":1}}}";
        let ev = parse_sse_record(rec).expect("payload event parses");
        assert_eq!(ev.kind, "message.updated");
        assert_eq!(ev.properties, json!({ "x": 1 }));

        // A flat `/event`-shaped record (no payload) still parses.
        let flat = "data: {\"type\":\"session.idle\",\"properties\":{}}";
        assert_eq!(parse_sse_record(flat).unwrap().kind, "session.idle");

        // Heartbeats are dropped.
        assert!(parse_sse_record("data: {\"type\":\"server.heartbeat\"}").is_none());
    }
}
