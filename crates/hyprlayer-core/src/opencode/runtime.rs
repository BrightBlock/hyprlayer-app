//! The shared `opencode serve` (OpenCode 2, `@opencode/cli` 2.4+) each profile runs, and the
//! HTTP and event-stream plumbing its sessions use.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch, Mutex};

pub use crate::opencode::driver::OpenCodeDriver;

use crate::claude::{ExitOutcome, SessionEvent};
use crate::opencode::driver::model_ref;
use crate::opencode::support::opencode_provider_is_supported;
use crate::opencode::translate::{parse_sse_record, OcEvent};

const EVENT_CAP: usize = 1024;
const EXIT_POLL: Duration = Duration::from_millis(250);
const MODEL_WAIT_TRIES: u32 = 40;
/// OpenCode 2's Basic-auth username; only the password can change.
const SERVER_USER: &str = "opencode";
/// The label of the credentials hyprlayer stores; the sync leaves every other one alone.
const KEY_LABEL: &str = "hyprlayer";

/// Name, under the app data dir, of the record `HyprlayerRuntime::open` reaps.
pub const OPENCODE_PID_FILE: &str = "opencode-server.pid";

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
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
    retired: StdMutex<Vec<Weak<OpenCodeServer>>>,
}

impl OpenCodeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn kill_blocking(&self) {
        self.stop_retired();
        if let Ok(guard) = self.inner.try_lock() {
            if let Some(server) = guard.as_ref() {
                server.kill_blocking();
            }
        }
    }

    /// The graceful counterpart to `kill_blocking`, used by the explicit quit
    /// path — it can await the lock instead of skipping a held one.
    pub async fn shutdown(&self) {
        self.stop_retired();
        if let Some(server) = self.inner.lock().await.take() {
            server.kill_blocking();
        }
    }

    fn stop_retired(&self) {
        let retired =
            std::mem::take(&mut *self.retired.lock().unwrap_or_else(PoisonError::into_inner));
        for server in retired.iter().filter_map(Weak::upgrade) {
            server.stop();
        }
    }

    pub async fn ensure(
        &self,
        config: OpenCodeServerConfig,
    ) -> Result<Arc<OpenCodeServer>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_ref() {
            if s.is_ready().await {
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
        let server = OpenCodeServer::start(config).await?;
        *guard = Some(server.clone());
        Ok(server)
    }

    pub async fn current(&self) -> Option<Arc<OpenCodeServer>> {
        self.inner.lock().await.clone()
    }

    /// Sessions keep a retired server until their next idle turn moves them to a new one.
    pub async fn invalidate(&self) {
        let Some(server) = self.inner.lock().await.take() else {
            return;
        };
        server.retired.store(true, Ordering::SeqCst);
        let mut retired = self.retired.lock().unwrap_or_else(PoisonError::into_inner);
        retired.retain(|kept| kept.strong_count() > 0);
        retired.push(Arc::downgrade(&server));
    }
}

#[cfg(any(test, feature = "test-support"))]
impl OpenCodeManager {
    #[doc(hidden)]
    pub async fn install_child_for_test(&self, child: crate::process::SupervisedChild) {
        let (events, _) = broadcast::channel(EVENT_CAP);
        let server = Arc::new(OpenCodeServer {
            base_url: String::new(),
            password: String::new(),
            client: reqwest::Client::new(),
            events,
            exited: watch::channel(None).0,
            model_context: HashMap::new(),
            home: PathBuf::new(),
            tasks: Handle::current(),
            pid_file: PathBuf::new(),
            child: StdMutex::new(child),
            retired: AtomicBool::default(),
            stops: StdMutex::default(),
        });
        server.watch_exit(Arc::default());
        *self.inner.lock().await = Some(server);
    }
}

pub struct OpenCodeServer {
    base_url: String,
    /// Generated per spawn and handed over only through `OPENCODE_SERVER_PASSWORD`; never logged.
    password: String,
    client: reqwest::Client,
    events: broadcast::Sender<OcEvent>,
    exited: watch::Sender<Option<SessionEvent>>,
    model_context: HashMap<String, u32>,
    home: PathBuf,
    // The sync `AgentDriver` methods fire their HTTP calls from whatever thread
    // ends a session — including the app's non-async exit hook — so they need a
    // handle onto the runtime rather than an ambient `tokio::spawn` context.
    tasks: Handle,
    pid_file: PathBuf,
    child: StdMutex<crate::process::SupervisedChild>,
    retired: AtomicBool,
    /// Per session, the stop an ended driver still has in flight; the next driver waits on it.
    stops: StdMutex<HashMap<String, watch::Receiver<bool>>>,
}

/// `kill_on_drop` signals only the server; its process group also holds whatever it spawned.
impl Drop for OpenCodeServer {
    fn drop(&mut self) {
        self.child
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .kill_blocking();
    }
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

fn generate_password() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl OpenCodeServer {
    async fn start(config: OpenCodeServerConfig) -> Result<Arc<Self>, String> {
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
        let password = generate_password();
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
            // OpenCode prefers `OPENCODE_PASSWORD`; one inherited from the user's shell would
            // replace the password this server is reached with.
            .env_remove("OPENCODE_PASSWORD")
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
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
        let tasks = Handle::current();
        let (events, _) = broadcast::channel(EVENT_CAP);
        let (exited, _) = watch::channel(None);
        let mut server = Self {
            base_url,
            password,
            client,
            events,
            exited,
            model_context: HashMap::new(),
            home,
            tasks,
            pid_file,
            child: StdMutex::new(child),
            retired: AtomicBool::default(),
            stops: StdMutex::default(),
        };

        let mut ready = false;
        let mut last_check = String::from("no answer");
        for _ in 0..60 {
            // A crashed server and a slow one look alike to the readiness check.
            let polled = server
                .child
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .try_wait();
            if let Ok(Some(status)) = polled {
                let said = stderr_text(&stderr);
                tracing::error!("opencode serve exited during startup ({status}): {said}");
                return Err(format!(
                    "`opencode serve` exited during startup ({status}): {said}"
                ));
            }
            match server.info().await {
                Ok(()) => {
                    ready = true;
                    break;
                }
                Err(e) => last_check = e,
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !ready {
            tracing::error!(
                "opencode serve on port {port} was not ready after 15s ({last_check}); it said: {}",
                stderr_text(&stderr)
            );
            return Err("`opencode serve` did not become ready in time".into());
        }

        server.sync_credentials(&auth_file).await;
        server.model_context = server.fetch_model_context().await;
        if server.model_context.is_empty() {
            tracing::warn!("opencode serve on port {port} listed no models in time");
        }
        spawn_sse(&server);
        let server = Arc::new(server);
        server.watch_exit(stderr);
        Ok(server)
    }

    /// A request to the server's API, authenticated.
    pub(crate) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}/api{path}", self.base_url))
            .basic_auth(SERVER_USER, Some(&self.password))
    }

    pub(crate) fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.tasks.spawn(task)
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<OcEvent> {
        self.events.subscribe()
    }

    pub(crate) fn exit_watch(&self) -> watch::Receiver<Option<SessionEvent>> {
        self.exited.subscribe()
    }

    pub(crate) fn home(&self) -> &Path {
        &self.home
    }

    pub(crate) fn retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
    }

    pub(crate) fn model_context(&self, model: &str) -> Option<u32> {
        self.model_context.get(model).copied()
    }

    pub(crate) fn stopping(&self, session: &str, done: watch::Receiver<bool>) {
        let mut stops = self.stops.lock().unwrap_or_else(PoisonError::into_inner);
        stops.retain(|_, stop| !*stop.borrow());
        stops.insert(session.to_string(), done);
    }

    pub(crate) fn stop_in_flight(&self, session: &str) -> Option<watch::Receiver<bool>> {
        let stops = self.stops.lock().unwrap_or_else(PoisonError::into_inner);
        stops.get(session).cloned()
    }

    async fn info(&self) -> Result<(), String> {
        let resp = self
            .request(reqwest::Method::GET, "/info")
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("answered {}", resp.status()))
        }
    }

    async fn is_ready(&self) -> bool {
        self.info().await.is_ok()
    }

    async fn get_data(&self, path: &str) -> Result<Value, String> {
        let mut body: Value = self
            .request(reqwest::Method::GET, path)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok(body.get_mut("data").map(Value::take).unwrap_or(Value::Null))
    }

    /// The server's API-key credentials made to match the app's `auth.json`: a key is added or
    /// replaced, one whose entry the user removed is deleted. OpenCode 2 cannot take a v1
    /// sign-in (it records no sign-in method), so such an entry is skipped and named in the log.
    /// Only credentials hyprlayer labelled are touched.
    async fn sync_credentials(&self, auth_file: &Path) {
        let wanted = match crate::opencode::config::api_keys(auth_file) {
            Ok(keys) => keys,
            Err(e) => {
                tracing::warn!("opencode credentials not synced: {e}");
                return;
            }
        };
        for provider in &wanted.unusable {
            tracing::warn!(
                "opencode provider {provider} has a sign-in OpenCode 2 cannot use; its sessions run without it"
            );
        }
        let existing = match self.get_data("/credential").await {
            Ok(Value::Array(existing)) => existing,
            Ok(_) => Vec::new(),
            Err(e) => {
                tracing::warn!("opencode credentials not synced: {e}");
                return;
            }
        };
        let mut kept = HashSet::new();
        for credential in &existing {
            let field = |key: &str| {
                credential
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            };
            let (id, integration, label) = (field("id"), field("integrationID"), field("label"));
            if label != KEY_LABEL {
                continue;
            }
            let key = credential.pointer("/value/key").and_then(Value::as_str);
            let matches = wanted
                .keys
                .get(integration)
                .is_some_and(|k| Some(k.as_str()) == key);
            if matches && kept.insert(integration.to_string()) {
                continue;
            }
            let removed = self
                .request(reqwest::Method::DELETE, &format!("/credential/{id}"))
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(e) = removed {
                tracing::warn!(
                    "opencode credential for {integration} not removed: {}",
                    e.without_url()
                );
            }
        }
        for (provider, key) in wanted.keys {
            if kept.contains(&provider) || !opencode_provider_is_supported(&provider) {
                continue;
            }
            let body = json!({
                "integrationID": provider,
                "label": KEY_LABEL,
                "value": { "type": "key", "key": key },
            });
            let added = self
                .request(reqwest::Method::POST, "/credential")
                .json(&body)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(e) = added {
                tracing::warn!(
                    "opencode credential for {provider} not stored: {}",
                    e.without_url()
                );
            }
        }
    }

    async fn fetch_model_context(&self) -> HashMap<String, u32> {
        for _ in 0..MODEL_WAIT_TRIES {
            match self.get_data("/model").await {
                Ok(models) if models.as_array().is_some_and(|m| !m.is_empty()) => {
                    return parse_model_context(&models);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("opencode /api/model read failed: {e}"),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        HashMap::new()
    }

    fn watch_exit(self: &Arc<Self>, stderr: Arc<StdMutex<VecDeque<String>>>) {
        let weak = Arc::downgrade(self);
        self.tasks.spawn(async move {
            loop {
                tokio::time::sleep(EXIT_POLL).await;
                let Some(server) = weak.upgrade() else {
                    return;
                };
                let polled = server
                    .child
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .try_wait();
                let Ok(Some(status)) = polled else { continue };
                let said = stderr_text(&stderr);
                let detail = format!("`opencode serve` exited ({status}): {said}");
                if server.end_sessions(ExitOutcome::Crashed, status.code(), detail) {
                    tracing::error!("opencode serve exited ({status}): {said}");
                }
                return;
            }
        });
    }

    fn end_sessions(&self, outcome: ExitOutcome, code: Option<i32>, stderr_tail: String) -> bool {
        self.exited.send_if_modified(|exited| {
            let first = exited.is_none();
            if first {
                *exited = Some(SessionEvent::Exited {
                    outcome,
                    code,
                    stderr_tail,
                });
            }
            first
        })
    }

    fn kill_blocking(&self) {
        self.stop();
        crate::process::orphan::clear(&self.pid_file);
    }

    /// Leaves the pid record alone: it names the newest server, which a retired one is not.
    fn stop(&self) {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        // Killing a live server detaches its sessions, as a server stop does; one that already
        // exited crashed, and `watch_exit` says so.
        if let Ok(None) = child.try_wait() {
            self.end_sessions(ExitOutcome::Detached, None, String::new());
        }
        child.kill_blocking();
    }

    /// `None` when the server lists no models at all: it has not loaded them yet.
    pub async fn knows_model(&self, model: &str) -> Option<bool> {
        let (provider, model_id) = model.split_once('/')?;
        let registry = self.provider_models().await.filter(|r| !r.is_empty())?;
        Some(
            registry
                .get(provider)
                .is_some_and(|models| models.iter().any(|(id, _)| id == model_id)),
        )
    }

    pub async fn provider_models(&self) -> Option<HashMap<String, Vec<(String, String)>>> {
        let models = self.get_data("/model").await.ok()?;
        Some(parse_provider_model_names(&models))
    }

    pub(crate) async fn create_session(
        &self,
        cwd: &str,
        model: Option<&(String, String)>,
    ) -> Result<String, String> {
        let mut body = json!({ "title": "Hyprlayer session", "location": { "directory": cwd } });
        if let Some(model) = model {
            body["model"] = model_ref(model);
        }
        let created: Value = self
            .request(reqwest::Method::POST, "/session")
            .json(&body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        created
            .pointer("/data/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "OpenCode session create returned no id".into())
    }

    /// The client error OpenCode answers once it lost `id`, as a reset of its data dir does.
    pub(crate) async fn missing_session(
        &self,
        id: &str,
    ) -> Result<Option<reqwest::StatusCode>, String> {
        let status = self
            .request(reqwest::Method::GET, &format!("/session/{id}"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .status();
        if status.is_client_error() {
            return Ok(Some(status));
        }
        if !status.is_success() {
            return Err(format!("OpenCode answered {status} for session {id}"));
        }
        Ok(None)
    }

    /// The `field` of each item a location-scoped list route returns, for the session's cwd.
    pub(crate) async fn names(
        &self,
        route: &str,
        field: &str,
        cwd: &str,
    ) -> Result<HashSet<String>, String> {
        let body: Value = self
            .request(reqwest::Method::GET, route)
            .query(&[("location[directory]", cwd)])
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let entries = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("OpenCode {route} returned no list"))?;
        Ok(entries
            .iter()
            .filter_map(|entry| entry.get(field).and_then(Value::as_str))
            .map(str::to_string)
            .collect())
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

/// Reads `/api/event` until the server exits, reconnecting after a gap; OpenCode starts each
/// stream with `server.connected`, which tells a session to read back what it missed.
fn spawn_sse(server: &OpenCodeServer) {
    let request = server
        .client
        .get(format!("{}/api/event", server.base_url))
        .basic_auth(SERVER_USER, Some(&server.password));
    let base_url = server.base_url.clone();
    let tx = server.events.clone();
    let mut exited = server.exited.subscribe();
    let stream = async move {
        // One warning per outage: this retries every half second until the server exits.
        let mut connected = true;
        loop {
            let response = request
                .try_clone()
                .expect("a bodiless GET clones")
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
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
    };
    server.tasks.spawn(async move {
        tokio::select! {
            () = stream => {}
            _ = exited.wait_for(Option::is_some) => {}
        }
    });
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("allocate a local port: {e}"))
}

fn model_key(model: &Value) -> Option<String> {
    let provider = model.get("providerID").and_then(Value::as_str)?;
    let id = model.get("id").and_then(Value::as_str)?;
    Some(format!("{provider}/{id}"))
}

/// `GET /api/model`'s context limits, by `provider/model`.
fn parse_model_context(models: &Value) -> HashMap<String, u32> {
    models
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let context = model.pointer("/limit/context").and_then(Value::as_u64)?;
            let key = model_key(model).filter(|_| context > 0)?;
            Some((key, context.min(u32::MAX as u64) as u32))
        })
        .collect()
}

/// `GET /api/model`'s models grouped by provider, as `(id, name)`.
fn parse_provider_model_names(models: &Value) -> HashMap<String, Vec<(String, String)>> {
    let mut out: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for model in models.as_array().into_iter().flatten() {
        let (Some(provider), Some(id)) = (
            model.get("providerID").and_then(Value::as_str),
            model.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
        out.entry(provider.to_string())
            .or_default()
            .push((id.to_string(), name.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_list_indexes_context_and_names_by_provider() {
        let models = json!([
            { "id": "big-pickle", "providerID": "opencode", "name": "Big Pickle", "limit": { "context": 200000, "output": 1 } },
            { "id": "no-limit", "providerID": "openrouter", "name": "No Limit" },
            { "id": "zero", "providerID": "openrouter", "limit": { "context": 0, "output": 1 } },
        ]);
        let context = parse_model_context(&models);
        assert_eq!(context.get("opencode/big-pickle"), Some(&200_000));
        assert_eq!(context.len(), 1);
        let names = parse_provider_model_names(&models);
        assert_eq!(
            names["opencode"],
            [("big-pickle".to_string(), "Big Pickle".to_string())]
        );
        assert_eq!(names["openrouter"].len(), 2);
        assert!(parse_model_context(&json!("garbage")).is_empty());
    }

    #[test]
    fn each_spawn_gets_its_own_long_password() {
        let (a, b) = (generate_password(), generate_password());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }
}
