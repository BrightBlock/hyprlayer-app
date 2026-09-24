use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, Mutex};

use crate::agent::{AgentDriver, KillFlag};
use crate::claude::{ExitOutcome, SessionEvent};
use crate::error::Result as CoreResult;
use crate::models::session::QuestionAnswers;
use crate::opencode::support::opencode_provider_is_supported;
use crate::store::session_event_codec::questions_from_value;

const EVENT_CAP: usize = 1024;

/// Name, under the app data dir, of the record `HyprlayerRuntime::open` reaps.
pub const OPENCODE_PID_FILE: &str = "opencode-server.pid";

fn supported_auth_content(content: &str) -> Option<String> {
    let Value::Object(mut auth) = serde_json::from_str::<Value>(content).ok()? else {
        return None;
    };
    auth.retain(|provider, _| opencode_provider_is_supported(provider));
    serde_json::to_string(&Value::Object(auth)).ok()
}

#[derive(Clone)]
struct OcEvent {
    kind: String,
    properties: Value,
}

pub struct OpenCodeServerConfig {
    pub bin: PathBuf,
    pub path: OsString,
    pub data_dir: PathBuf,
    pub home: PathBuf,
    pub auth_file: PathBuf,
    pub pid_file: PathBuf,
    /// The attaching client's env overlay, baked in at spawn. First attach wins:
    /// this server is cached for the profile's lifetime (ADR-0009).
    pub overlay: Vec<(String, String)>,
}

#[derive(Default)]
pub struct OpenCodeManager {
    inner: Mutex<Option<Arc<OpenCodeServer>>>,
}

impl OpenCodeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn kill_blocking(&self) {
        if let Ok(guard) = self.inner.try_lock() {
            if let Some(server) = guard.as_ref() {
                server.kill_blocking();
            }
        }
    }

    /// The graceful counterpart to `kill_blocking`, used by the explicit quit
    /// path — it can await the lock instead of skipping a held one.
    pub async fn shutdown(&self) {
        if let Some(server) = self.inner.lock().await.take() {
            server.kill_blocking();
        }
    }

    pub async fn ensure(
        &self,
        config: OpenCodeServerConfig,
    ) -> Result<Arc<OpenCodeServer>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_ref() {
            if s.is_healthy().await {
                return Ok(s.clone());
            }
        }
        if let Some(dead) = guard.take() {
            tracing::warn!(
                "opencode serve at {} stopped answering; replacing it",
                dead.base_url
            );
            dead.kill_blocking();
        }
        let server = Arc::new(OpenCodeServer::start(config).await?);
        *guard = Some(server.clone());
        Ok(server)
    }

    pub async fn current(&self) -> Option<Arc<OpenCodeServer>> {
        self.inner.lock().await.clone()
    }

    pub async fn invalidate(&self) {
        *self.inner.lock().await = None;
    }
}

#[cfg(any(test, feature = "test-support"))]
impl OpenCodeManager {
    #[doc(hidden)]
    pub async fn install_child_for_test(&self, child: crate::process::SupervisedChild) {
        let (events, _) = broadcast::channel(EVENT_CAP);
        *self.inner.lock().await = Some(Arc::new(OpenCodeServer {
            base_url: String::new(),
            client: reqwest::Client::new(),
            events,
            model_context: std::collections::HashMap::new(),
            home: PathBuf::new(),
            tasks: Handle::current(),
            pid_file: PathBuf::new(),
            _child: Mutex::new(child),
        }));
    }
}

pub struct OpenCodeServer {
    base_url: String,
    client: reqwest::Client,
    events: broadcast::Sender<OcEvent>,
    model_context: std::collections::HashMap<String, u32>,
    home: PathBuf,
    // The sync `AgentDriver` methods fire their HTTP calls from whatever thread
    // ends a session — including the app's non-async exit hook — so they need a
    // handle onto the runtime rather than an ambient `tokio::spawn` context.
    tasks: Handle,
    pid_file: PathBuf,
    _child: Mutex<crate::process::SupervisedChild>,
}

/// The string the reaper matches against the live process's command line.
///
/// Deliberately the binary's file name rather than `bin` itself: `bin` may be a
/// version-manager shim that execs a different path, and the process then reports
/// the path it exec'd into, so a record built from `bin` would never match. The
/// args carry the port, which is what keeps the match specific to this server.
fn orphan_program(bin: &Path, args: &[String]) -> String {
    let name = bin
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("opencode");
    format!("{name} {}", args.join(" "))
}

impl OpenCodeServer {
    async fn start(config: OpenCodeServerConfig) -> Result<Self, String> {
        let OpenCodeServerConfig {
            bin,
            path,
            data_dir,
            home,
            auth_file,
            pid_file,
            overlay,
        } = config;
        let port = free_port()?;
        let base_url = format!("http://127.0.0.1:{port}");
        let _ = std::fs::create_dir_all(&data_dir);

        let serve_args = [
            "serve".to_string(),
            "--hostname".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            port.to_string(),
        ];

        let mut cmd = Command::new(&bin);
        crate::process::appimage::scrub_tokio(&mut cmd);
        cmd.args(&serve_args)
            .current_dir(&home)
            .env("PATH", &path)
            .env("XDG_DATA_HOME", &data_dir)
            .envs(overlay)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(content) = std::fs::read_to_string(&auth_file)
            .ok()
            .filter(|c| !c.trim().is_empty())
        {
            let filtered = supported_auth_content(&content).unwrap_or_else(|| {
                tracing::warn!(
                    "opencode auth.json did not parse as a JSON object; passing it through unfiltered"
                );
                content
            });
            cmd.env("OPENCODE_AUTH_CONTENT", filtered);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = crate::process::spawn_supervised(&mut cmd)
            .map_err(|e| format!("spawn `opencode serve`: {e}"))?;
        tracing::info!(
            "started {} serve as pid {:?} on port {port}",
            bin.display(),
            child.id()
        );
        let stderr = capture_stderr(child.take_stderr());

        if let Some(pid) = child.id() {
            crate::process::orphan::record(&pid_file, pid, &orphan_program(&bin, &serve_args));
        }

        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| e.to_string())?;

        let mut healthy = false;
        let mut last_check = String::from("no answer");
        for _ in 0..60 {
            // A crashed server and a slow one look alike to the health check.
            if let Ok(Some(status)) = child.try_wait() {
                let said = stderr_text(&stderr);
                tracing::error!("opencode serve exited during startup ({status}): {said}");
                return Err(format!(
                    "`opencode serve` exited during startup ({status}): {said}"
                ));
            }
            match client
                .get(format!("{base_url}/global/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(resp) => match resp.json::<Value>().await {
                    Ok(v) if v.get("healthy").and_then(Value::as_bool) == Some(true) => {
                        healthy = true;
                        break;
                    }
                    Ok(v) => last_check = format!("reported {v}"),
                    Err(e) => last_check = e.to_string(),
                },
                Err(e) => last_check = e.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !healthy {
            tracing::error!(
                "opencode serve on port {port} was not healthy after 15s ({last_check}); it said: {}",
                stderr_text(&stderr)
            );
            return Err("`opencode serve` did not become healthy in time".into());
        }

        let model_context = fetch_model_context(&client, &base_url).await;
        let tasks = Handle::current();

        let (events, _) = broadcast::channel(EVENT_CAP);
        spawn_sse(&tasks, base_url.clone(), client.clone(), events.clone());

        Ok(Self {
            base_url,
            client,
            events,
            model_context,
            home,
            tasks,
            pid_file,
            _child: Mutex::new(child),
        })
    }

    fn kill_blocking(&self) {
        if let Ok(child) = self._child.try_lock() {
            child.kill_blocking();
        }
        crate::process::orphan::clear(&self.pid_file);
    }

    fn context_window(&self, model_id: &str) -> Option<u32> {
        self.model_context.get(model_id).copied()
    }

    fn home(&self) -> &Path {
        &self.home
    }

    pub async fn knows_model(&self, model: &str) -> Option<bool> {
        let (provider, model_id) = model.split_once('/')?;
        let registry = self.provider_models().await?;
        Some(
            registry
                .get(provider)
                .is_some_and(|models| models.iter().any(|(id, _)| id == model_id)),
        )
    }

    pub async fn provider_models(
        &self,
    ) -> Option<std::collections::HashMap<String, Vec<(String, String)>>> {
        let body = self
            .client
            .get(format!("{}/config/providers", self.base_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()?;
        parse_provider_model_names(&body)
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

const STDERR_LINES_KEPT: usize = 40;

/// Keeps opencode's last stderr lines for the error when it fails to start;
/// every line also goes to the debug log.
fn capture_stderr(stderr: Option<tokio::process::ChildStderr>) -> Arc<StdMutex<VecDeque<String>>> {
    let kept = Arc::new(StdMutex::new(VecDeque::new()));
    if let Some(stderr) = stderr {
        let sink = kept.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = crate::redact::url_credentials(&line);
                tracing::debug!("opencode serve: {line}");
                let mut kept = sink.lock().unwrap_or_else(PoisonError::into_inner);
                if kept.len() == STDERR_LINES_KEPT {
                    kept.pop_front();
                }
                kept.push_back(line);
            }
        });
    }
    kept
}

fn stderr_text(kept: &StdMutex<VecDeque<String>>) -> String {
    let kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
    if kept.is_empty() {
        "nothing on stderr".into()
    } else {
        kept.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

fn spawn_sse(
    tasks: &Handle,
    base_url: String,
    client: reqwest::Client,
    tx: broadcast::Sender<OcEvent>,
) {
    tasks.spawn(async move {
        // One warning per outage: this retries every half second, forever.
        let mut connected = true;
        loop {
            let response = client.get(format!("{base_url}/global/event")).send().await;
            match &response {
                Ok(_) if !connected => {
                    tracing::info!("opencode event stream at {base_url} is back");
                    connected = true;
                }
                Err(e) if connected => {
                    tracing::warn!("opencode event stream at {base_url} failed; retrying: {e}");
                    connected = false;
                }
                _ => {}
            }
            if let Ok(mut resp) = response {
                let mut buf = String::new();
                while let Ok(Some(bytes)) = resp.chunk().await {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(idx) = buf.find("\n\n") {
                        let record: String = buf.drain(..idx + 2).collect();
                        if let Some(ev) = parse_sse_record(&record) {
                            let _ = tx.send(ev);
                        }
                    }
                }
            }
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
                tracing::warn!("opencode /config/providers read failed: {e}");
                return std::collections::HashMap::new();
            }
        },
        Err(e) => {
            tracing::warn!("opencode /config/providers fetch failed: {e}");
            return std::collections::HashMap::new();
        }
    };
    parse_model_context(&body)
}

async fn fetch_command_names(
    client: &reqwest::Client,
    base_url: &str,
    cwd: &str,
) -> Result<std::collections::HashSet<String>, String> {
    let value: Value = client
        .get(format!("{base_url}/command"))
        .query(&[("directory", cwd)])
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let entries = value
        .as_array()
        .ok_or_else(|| "OpenCode /command returned a non-array response".to_string())?;
    Ok(entries
        .iter()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect())
}

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
        tracing::warn!("opencode /config/providers parse failed");
        return out;
    };
    for p in parsed.providers {
        for (model_id, m) in p.models {
            if let Some(ctx) = m.limit.and_then(|l| l.context).filter(|c| *c > 0) {
                out.insert(
                    format!("{}/{model_id}", p.id),
                    ctx.min(u32::MAX as u64) as u32,
                );
            }
        }
    }
    out
}

fn parse_provider_model_names(
    body: &str,
) -> Option<std::collections::HashMap<String, Vec<(String, String)>>> {
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
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
    }

    let parsed = serde_json::from_str::<Resp>(body).ok()?;
    Some(
        parsed
            .providers
            .into_iter()
            .map(|p| {
                let models = p
                    .models
                    .into_iter()
                    .map(|(key, m)| {
                        let id = m.id.unwrap_or(key);
                        let name = m.name.unwrap_or_else(|| id.clone());
                        (id, name)
                    })
                    .collect();
                (p.id, models)
            })
            .collect(),
    )
}

pub struct OpenCodeDriver {
    server: Arc<OpenCodeServer>,
    session_id: String,
    cwd: String,
    skills: std::collections::HashSet<String>,
    commands: std::collections::HashSet<String>,
    model: std::sync::Mutex<Option<(String, String)>>,
    events: broadcast::Sender<SessionEvent>,
    kill_flag: Arc<KillFlag>,
    filter_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pending_questions: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
}

impl OpenCodeDriver {
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
        // OpenCode's HTTP prompt endpoint does not parse slash commands, so typed
        // invocations route through /session/{id}/command. The routable set is the
        // server's own command registry (its builtins, commands, and skills alike);
        // the local skill index only decides which invocations carry the
        // cross-harness routing preamble.
        let skill_scan = tokio::task::spawn_blocking({
            let cwd = cwd.to_string();
            let home = server.home().to_path_buf();
            move || {
                crate::skills::list_skills(Some(std::path::Path::new(&cwd)), Some(&home))
                    .into_iter()
                    .map(|skill| skill.name)
                    .collect::<std::collections::HashSet<String>>()
            }
        });
        let (skills, commands) = tokio::join!(
            skill_scan,
            fetch_command_names(&server.client, &server.base_url, cwd)
        );
        let skills = skills.unwrap_or_default();
        let commands = match commands {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(
                    "opencode command discovery failed; explicit invocations fall back to plain prompts: {error}"
                );
                std::collections::HashSet::new()
            }
        };

        let (events, _) = broadcast::channel(EVENT_CAP);
        let primary_rx = events.subscribe();

        let _ = events.send(SessionEvent::Init {
            provider_session_id: session_id.clone(),
            model: model.as_ref().map(|(p, m)| format!("{p}/{m}")),
        });

        if let Some(tokens) = model.as_ref().and_then(|(p, m)| {
            let id = format!("{p}/{m}");
            crate::opencode::config::opencode_model_context_window(&id)
                .or_else(|| server.context_window(&id))
        }) {
            let _ = events.send(SessionEvent::ContextWindow { tokens });
        }

        let mut sse_rx = server.events.subscribe();
        let tx = events.clone();
        let sid = session_id.clone();
        let pending_questions: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>> =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let pq = pending_questions.clone();
        let task = server.tasks.spawn(async move {
            let mut xlate = Translator::default();
            loop {
                match sse_rx.recv().await {
                    Ok(ev) => {
                        if event_session_id(&ev).as_deref() != Some(sid.as_str()) {
                            continue;
                        }
                        if ev.kind == "question.asked" {
                            if let Some(qid) = ev.properties.get("id").and_then(Value::as_str) {
                                if let Ok(mut m) = pq.lock() {
                                    m.insert(
                                        qid.to_string(),
                                        ev.properties
                                            .get("questions")
                                            .cloned()
                                            .unwrap_or(json!([])),
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
            cwd: cwd.to_string(),
            skills,
            commands,
            model: std::sync::Mutex::new(model),
            events,
            kill_flag: Arc::new(KillFlag::default()),
            filter_task: Mutex::new(Some(task)),
            pending_questions,
        });
        Ok((driver, primary_rx))
    }

    fn end(&self, outcome: ExitOutcome) {
        self.post_async(format!("/session/{}/abort", self.session_id), json!({}));
        if let Ok(mut guard) = self.filter_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        let _ = self.events.send(SessionEvent::Exited {
            outcome,
            code: None,
            stderr_tail: String::new(),
        });
    }

    fn post_async(&self, path: String, body: Value) {
        let client = self.server.client.clone();
        let url = format!("{}{}", self.server.base_url, path);
        self.server.tasks.spawn(async move {
            match client.post(&url).json(&body).send().await {
                Ok(resp) if !resp.status().is_success() => {
                    tracing::warn!("opencode POST {url} returned {}", resp.status());
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("opencode POST {url} failed: {e}"),
            }
        });
    }

    fn prompt_async(&self, body: Value) {
        self.request_async(
            format!(
                "{}/session/{}/prompt_async",
                self.server.base_url, self.session_id
            ),
            body,
            None,
        );
    }

    fn command_async(&self, command: &str, arguments: &str, original_text: &str) {
        let model = self.model.lock().ok().and_then(|model| model.clone());
        let arguments = if self.skills.contains(command) {
            routed_skill_arguments(skill_base_for_model(model.as_ref()), arguments)
        } else {
            arguments.to_string()
        };
        let mut body = json!({ "command": command, "arguments": arguments });
        let mut fallback = json!({ "parts": [{ "type": "text", "text": original_text }] });
        if let Some((provider, model)) = &model {
            body["model"] = json!(format!("{provider}/{model}"));
            fallback["model"] = json!({ "providerID": provider, "modelID": model });
        }

        let client = self.server.client.clone();
        let events = self.events.clone();
        let command_url = format!(
            "{}/session/{}/command",
            self.server.base_url, self.session_id
        );
        let prompt_url = format!(
            "{}/session/{}/prompt_async",
            self.server.base_url, self.session_id
        );
        let directory = self.cwd.clone();
        self.server.tasks.spawn(async move {
            let sent = client
                .post(command_url)
                .query(&[("directory", directory)])
                .json(&body)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            let Err(command_error) = sent else { return };
            tracing::warn!(
                "opencode command failed; delivering the turn as a plain prompt: {command_error}"
            );
            let delivered = client
                .post(prompt_url)
                .json(&fallback)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(e) = delivered {
                tracing::warn!("opencode request failed: {e}");
                let _ = events.send(SessionEvent::AssistantText {
                    text: format!("⚠ OpenCode request failed: {e}"),
                    parent_id: None,
                    injected: false,
                });
                let _ = events.send(SessionEvent::TurnDone {
                    cost_usd: None,
                    num_turns: None,
                    context_tokens: None,
                    turn_status: None,
                });
            }
        });
    }

    fn request_async(&self, url: String, body: Value, directory: Option<String>) {
        let client = self.server.client.clone();
        let events = self.events.clone();
        self.server.tasks.spawn(async move {
            let mut request = client.post(url).json(&body);
            if let Some(directory) = directory {
                request = request.query(&[("directory", directory)]);
            }
            let result = request
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(e) = result {
                tracing::warn!("opencode request failed: {e}");
                let _ = events.send(SessionEvent::AssistantText {
                    text: format!("⚠ OpenCode request failed: {e}"),
                    parent_id: None,
                    injected: false,
                });
                let _ = events.send(SessionEvent::TurnDone {
                    cost_usd: None,
                    num_turns: None,
                    context_tokens: None,
                    turn_status: None,
                });
            }
        });
    }

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
            tracing::warn!("opencode session delete failed: {e}");
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
        let _ = self.events.send(SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
        });
        if let Some(invocation) = leading_skill_invocation(text, &self.commands, prompt_name) {
            self.command_async(invocation.name, invocation.arguments, text);
            return Ok(());
        }
        let mut body = json!({ "parts": [{ "type": "text", "text": text }] });
        if let Some((provider, model)) = self.model.lock().ok().and_then(|m| m.clone()) {
            body["model"] = json!({ "providerID": provider, "modelID": model });
        }
        self.prompt_async(body);
        Ok(())
    }

    fn set_model(&self, model: Option<String>) {
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

    fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> CoreResult<()> {
        let questions = self
            .pending_questions
            .lock()
            .ok()
            .and_then(|mut m| m.remove(request_id));
        if answers.is_empty() {
            self.post_async(format!("/question/{request_id}/reject"), json!({}));
            return Ok(());
        }
        let ordered: Vec<Vec<String>> = questions
            .as_ref()
            .and_then(Value::as_array)
            .map(|qs| {
                qs.iter()
                    .map(|q| {
                        let text = q
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        answers
                            .get(text)
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
        if self.kill_flag.mark_killed() {
            self.end(ExitOutcome::Killed);
        }
    }

    fn kill_detached(&self) {
        if self.kill_flag.mark_detached() {
            self.end(ExitOutcome::Detached);
        }
    }

    fn pid(&self) -> Option<u32> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillBase {
    Claude,
    Codex,
}

fn skill_base_for_model(model: Option<&(String, String)>) -> SkillBase {
    let Some((provider, model)) = model else {
        return SkillBase::Codex;
    };
    let id = format!("{provider}/{model}").to_ascii_lowercase();
    if provider.eq_ignore_ascii_case("anthropic") || id.contains("claude") {
        SkillBase::Claude
    } else {
        SkillBase::Codex
    }
}

struct SkillInvocation<'a> {
    name: &'a str,
    arguments: &'a str,
}

fn leading_skill_invocation<'a>(
    text: &'a str,
    known: &std::collections::HashSet<String>,
    prompt_name: Option<&str>,
) -> Option<SkillInvocation<'a>> {
    // Custom prompts have already been expanded by the app. Their bodies are
    // ordinary user text even if the first line happens to resemble a skill.
    if prompt_name.is_some() {
        return None;
    }
    let text = text.trim_start();
    let rest = text.strip_prefix('/').or_else(|| text.strip_prefix('$'))?;
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() || !known.contains(name) {
        return None;
    }
    Some(SkillInvocation {
        name,
        arguments: rest[name_end..].trim_start(),
    })
}

fn routed_skill_arguments(base: SkillBase, arguments: &str) -> String {
    let (platform, agent_source) = match base {
        SkillBase::Claude => ("Claude", "the matching .claude/agents/<name>.md persona"),
        SkillBase::Codex => (
            "Codex",
            "the matching .codex/agents/<name>.toml persona, or ~/.claude/agents/<name>.md when no TOML exists",
        ),
    };
    let routing = format!(
        "<hyprlayer-skill-routing>\n\
         Base platform: {platform}. Where this skill distinguishes harness behavior, follow its \
         {platform} path. OpenCode is only the execution harness: if it cannot load a named \
         {platform} agent directly, read {agent_source} and carry those instructions into a \
         generic subagent.\n\
         </hyprlayer-skill-routing>"
    );
    if arguments.is_empty() {
        routing
    } else {
        format!("{routing}\n\n{arguments}")
    }
}

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

#[derive(Default)]
struct Translator {
    roles: std::collections::HashMap<String, String>,
    text_len: std::collections::HashMap<String, usize>,
    text_done: std::collections::HashSet<String>,
    delta_parts: std::collections::HashSet<String>,
    part_types: std::collections::HashMap<String, String>,
    tool_started: std::collections::HashSet<String>,
    tool_done: std::collections::HashSet<String>,
    cost_total: f64,
    context_tokens: Option<u32>,
    planning: bool,
}

impl Translator {
    fn handle(&mut self, ev: &OcEvent) -> Vec<SessionEvent> {
        let p = &ev.properties;
        match ev.kind.as_str() {
            "message.updated" => {
                let Some(info) = p.get("info") else {
                    return Vec::new();
                };
                if let (Some(id), Some(role)) = (
                    info.get("id").and_then(Value::as_str),
                    info.get("role").and_then(Value::as_str),
                ) {
                    self.roles.insert(id.to_string(), role.to_string());
                }
                // Every message names the agent answering it; `plan` is OpenCode's plan mode.
                let planning = match info.get("agent").and_then(Value::as_str) {
                    Some(agent) => agent == "plan",
                    None => return Vec::new(),
                };
                if planning == self.planning {
                    return Vec::new();
                }
                self.planning = planning;
                vec![SessionEvent::PlanModeChanged { active: planning }]
            }
            "message.part.delta" => {
                let mid = p
                    .get("messageID")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if self.roles.get(mid).map(String::as_str) != Some("assistant") {
                    return Vec::new();
                }
                let pid = p.get("partID").and_then(Value::as_str).unwrap_or_default();
                if !pid.is_empty() {
                    self.delta_parts.insert(pid.to_string());
                }
                if self.part_types.get(pid).map(String::as_str) == Some("reasoning") {
                    return Vec::new();
                }
                let field = p.get("field").and_then(Value::as_str).unwrap_or("text");
                let delta = p.get("delta").and_then(Value::as_str).unwrap_or_default();
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
            "question.asked" => vec![SessionEvent::QuestionAsk {
                request_id: str_field(p, "id"),
                questions: questions_from_value(p.get("questions")),
            }],
            "session.idle" => vec![SessionEvent::TurnDone {
                cost_usd: Some(self.cost_total),
                num_turns: None,
                context_tokens: self.context_tokens,
                turn_status: None,
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
                if name == "MessageAbortedError" || msg == "Aborted" {
                    return vec![SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                        turn_status: None,
                    }];
                }
                vec![
                    SessionEvent::AssistantText {
                        text: format!("⚠ OpenCode error: {msg}"),
                        parent_id: None,
                        injected: false,
                    },
                    SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                        turn_status: None,
                    },
                ]
            }
            _ => Vec::new(),
        }
    }

    fn handle_part(&mut self, part: &Value) -> Vec<SessionEvent> {
        let pid = part
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let ptype = part.get("type").and_then(Value::as_str).unwrap_or_default();
        if !pid.is_empty() {
            self.part_types.insert(pid.clone(), ptype.to_string());
        }
        let mid = part
            .get("messageID")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.roles.get(mid).map(String::as_str) != Some("assistant") {
            return Vec::new();
        }
        let mut out = Vec::new();

        match ptype {
            "text" => {
                let full = part.get("text").and_then(Value::as_str).unwrap_or_default();
                if !self.delta_parts.contains(&pid) {
                    let emitted = self.text_len.get(&pid).copied().unwrap_or(0);
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
                        injected: false,
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
                let call_id = part
                    .get("callID")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tool = part
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if tool == "question" {
                    return out;
                }
                let state = part.get("state").cloned().unwrap_or(Value::Null);
                let status = state
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
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
                    let total = f("total");
                    (if total > 0.0 {
                        total
                    } else {
                        f("input") + f("output")
                    }) as u32
                });
                self.cost_total += cost.unwrap_or(0.0);
                if let Some(ctx) = ctx {
                    self.context_tokens = Some(ctx);
                    out.push(SessionEvent::Usage {
                        context_tokens: ctx,
                    });
                }
            }
            _ => {}
        }
        out
    }
}

fn str_field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

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
        _ => return (tool.to_string(), input),
    };
    (name.to_string(), input)
}

fn part_ended(part: &Value) -> bool {
    part.get("time").and_then(|t| t.get("end")).is_some()
}

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
    fn auth_content_keeps_credentials_for_supported_providers_only() {
        let filtered = supported_auth_content(
            r#"{
                "openai": {"type":"api","key":"supported"},
                "GitHub-Copilot": {"type":"api","key":"retired"}
            }"#,
        )
        .unwrap();
        let Value::Object(auth) = serde_json::from_str(&filtered).unwrap() else {
            panic!("filtered auth content should be an object");
        };
        assert!(auth.contains_key("openai"));
        assert!(!auth
            .keys()
            .any(|provider| provider.eq_ignore_ascii_case("github-copilot")));
    }

    #[test]
    fn normalize_file_tools_to_claude_vocabulary() {
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
        assert_eq!(
            input,
            json!({ "file_path": "/w/a.txt", "old_string": "a", "new_string": "b" })
        );
    }

    #[test]
    fn parse_model_context_indexes_by_provider_slash_model() {
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
    fn opencode_routes_only_claude_family_models_to_the_claude_base() {
        for id in [
            "anthropic/claude-sonnet-4-6",
            "openrouter/anthropic/claude-opus-4.1",
            "amazon-bedrock/us.anthropic.claude-4",
            "vertex/claude-sonnet-4-5",
        ] {
            let model = parse_model(id).unwrap();
            assert_eq!(
                skill_base_for_model(Some(&model)),
                SkillBase::Claude,
                "{id}"
            );
        }
        for id in [
            "openai/gpt-5.4",
            "google/gemini-3-pro",
            "ollama/qwen3-coder",
            "deepseek/deepseek-v4",
        ] {
            let model = parse_model(id).unwrap();
            assert_eq!(skill_base_for_model(Some(&model)), SkillBase::Codex, "{id}");
        }
        assert_eq!(skill_base_for_model(None), SkillBase::Codex);
    }

    #[test]
    fn explicit_known_skills_route_with_either_platform_sigil() {
        let known = std::collections::HashSet::from(["research_codebase".to_string()]);

        let slash = leading_skill_invocation(
            "  /research_codebase focus here\n\nRelevant documents:\n- plan.md",
            &known,
            None,
        )
        .unwrap();
        assert_eq!(slash.name, "research_codebase");
        assert_eq!(
            slash.arguments,
            "focus here\n\nRelevant documents:\n- plan.md"
        );

        let dollar = leading_skill_invocation("$research_codebase", &known, None).unwrap();
        assert_eq!(dollar.name, "research_codebase");
        assert_eq!(dollar.arguments, "");
    }

    #[test]
    fn free_text_and_expanded_custom_prompts_are_not_reinterpreted_as_skills() {
        let known = std::collections::HashSet::from(["research_codebase".to_string()]);

        assert!(leading_skill_invocation("discuss /research_codebase", &known, None).is_none());
        assert!(leading_skill_invocation("/not-installed normal text", &known, None).is_none());
        assert!(leading_skill_invocation(
            "/research_codebase is just the custom prompt's first line",
            &known,
            Some("My prompt"),
        )
        .is_none());
    }

    #[test]
    fn routed_skill_arguments_select_the_platform_persona_tree() {
        let claude = routed_skill_arguments(SkillBase::Claude, "focus on sessions");
        assert!(claude.contains("Base platform: Claude"));
        assert!(claude.contains(".claude/agents/<name>.md"));
        assert!(claude.ends_with("\n\nfocus on sessions"));

        let codex = routed_skill_arguments(SkillBase::Codex, "");
        assert!(codex.contains("Base platform: Codex"));
        assert!(codex.contains(".codex/agents/<name>.toml"));
        assert!(codex.ends_with("</hyprlayer-skill-routing>"));
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
        let (name, input) = normalize_tool("grep", json!({ "pattern": "DONE", "path": "/w" }));
        assert_eq!(name, "Grep");
        assert_eq!(input, json!({ "pattern": "DONE", "path": "/w" }));

        let (name, _) = normalize_tool("bash", json!({ "command": "echo hi" }));
        assert_eq!(name, "Bash");
    }

    fn oc(kind: &str, properties: Value) -> OcEvent {
        OcEvent {
            kind: kind.to_string(),
            properties,
        }
    }

    #[test]
    fn a_multi_step_turn_emits_exactly_one_turn_done_carrying_summed_usage() {
        let mut x = Translator::default();
        x.handle(&oc(
            "message.updated",
            json!({ "info": { "id": "m1", "role": "assistant" } }),
        ));

        let mut events = Vec::new();
        for (id, cost, total) in [("s1", 0.25, 100), ("s2", 0.75, 250)] {
            events.extend(x.handle(&oc(
                "message.part.updated",
                json!({ "part": {
                    "id": id, "type": "step-finish", "messageID": "m1",
                    "cost": cost, "tokens": { "total": total },
                } }),
            )));
        }
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SessionEvent::TurnDone { .. })),
            "step-finish is a per-step boundary and must not end the turn: {events:?}"
        );

        events.extend(x.handle(&oc("session.idle", json!({}))));
        let done: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SessionEvent::TurnDone { .. }))
            .collect();
        assert_eq!(done.len(), 1, "expected one TurnDone, got {events:?}");
        match done[0] {
            SessionEvent::TurnDone {
                cost_usd,
                context_tokens,
                ..
            } => {
                assert_eq!(*cost_usd, Some(1.0));
                assert_eq!(*context_tokens, Some(250));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn reasoning_announced_before_role_is_not_leaked_as_text() {
        let mut x = Translator::default();

        let out = x.handle(&oc(
            "message.part.updated",
            json!({ "part": { "id": "r1", "type": "reasoning", "messageID": "m1" } }),
        ));
        assert!(out.is_empty());

        x.handle(&oc(
            "message.updated",
            json!({ "info": { "id": "m1", "role": "assistant" } }),
        ));

        let out = x.handle(&oc(
            "message.part.delta",
            json!({ "messageID": "m1", "partID": "r1", "field": "text", "delta": "secret plan" }),
        ));
        assert!(
            out.is_empty(),
            "reasoning delta leaked as a live event: {out:?}"
        );

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
        let rec = "data: {\"directory\":\"/w\",\"payload\":{\"type\":\"message.updated\",\"properties\":{\"x\":1}}}";
        let ev = parse_sse_record(rec).expect("payload event parses");
        assert_eq!(ev.kind, "message.updated");
        assert_eq!(ev.properties, json!({ "x": 1 }));

        let flat = "data: {\"type\":\"session.idle\",\"properties\":{}}";
        assert_eq!(parse_sse_record(flat).unwrap().kind, "session.idle");

        assert!(parse_sse_record("data: {\"type\":\"server.heartbeat\"}").is_none());
    }
}
