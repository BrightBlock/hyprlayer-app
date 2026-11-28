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
use crate::opencode::config::{
    OpencodeSignInAttempt, OpencodeSignInMethod, OpencodeSignInStatus, SignInRecords,
};
use crate::opencode::driver::model_ref;
use crate::opencode::support::opencode_provider_is_supported;
use crate::opencode::translate::{parse_sse_record, OcEvent};

const EVENT_CAP: usize = 1024;
const EXIT_POLL: Duration = Duration::from_millis(250);
const MODEL_WAIT_TRIES: u32 = 40;
/// OpenCode 2's Basic-auth username; only the password can change.
const SERVER_USER: &str = "opencode";
/// The labels of the credentials hyprlayer stores; the sync leaves every other one alone.
const KEY_LABEL: &str = "hyprlayer";
const SIGN_IN_LABEL: &str = "hyprlayer-sign-in";
const SIGN_IN_FOLLOW_EVERY: Duration = Duration::from_secs(2);
/// A bound for an attempt that never reports an end; the pending record outlives it.
const SIGN_IN_FOLLOW_LIMIT: Duration = Duration::from_secs(30 * 60);

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
    pub sign_ins: SignInRecords,
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
            sign_ins,
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

        server.sync_credentials(&auth_file, &sign_ins).await;
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

    /// The `data` of an authenticated `GET /api{path}`.
    pub async fn get_data(&self, path: &str) -> Result<Value, String> {
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

    /// The server's credentials made to match the app's records: an API key from `auth.json` is
    /// added or replaced and one the user removed is deleted; a sign-in made here stays while
    /// its provider is signed in or has a sign-in pending, and a pending one it finds is
    /// recorded as signed in. A pending sign-in that left no credential is dropped: an attempt
    /// still live on a retired server is recorded by its follower. OpenCode 2 cannot take a v1 sign-in (it records no sign-in
    /// method), so such an entry is skipped and named in the log. Only credentials hyprlayer
    /// labelled are touched.
    async fn sync_credentials(&self, auth_file: &Path, sign_ins: &SignInRecords) {
        let wanted = match crate::opencode::config::api_keys(auth_file) {
            Ok(keys) => keys,
            Err(e) => {
                tracing::warn!("opencode credentials not synced: {e}");
                return;
            }
        };
        let (signed_in, pending) = (sign_ins.signed_in(), sign_ins.pending());
        for provider in wanted.unusable.iter().filter(|p| !signed_in.contains(p)) {
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
        let mut found = HashSet::new();
        for credential in &existing {
            let field = |key: &str| {
                credential
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            };
            let (id, integration, label) = (field("id"), field("integrationID"), field("label"));
            let key = credential.pointer("/value/key").and_then(Value::as_str);
            let keep = match label {
                KEY_LABEL => {
                    let matches = wanted
                        .keys
                        .get(integration)
                        .is_some_and(|k| Some(k.as_str()) == key);
                    matches && kept.insert(integration.to_string())
                }
                SIGN_IN_LABEL if pending.iter().any(|p| p == integration) => {
                    if let Err(e) = sign_ins.completed(integration) {
                        tracing::warn!("opencode sign-in for {integration} not recorded: {e}");
                    }
                    found.insert(integration.to_string());
                    true
                }
                SIGN_IN_LABEL => signed_in.iter().any(|p| p == integration),
                _ => true,
            };
            if !keep {
                self.delete_credential(id, integration).await;
            }
        }
        for provider in pending.iter().filter(|p| !found.contains(*p)) {
            if let Err(e) = sign_ins.abandoned(provider) {
                tracing::warn!("opencode sign-in for {provider} not cleared: {e}");
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

    async fn delete_credential(&self, id: &str, integration: &str) {
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

    /// Deletes the sign-ins made here for `provider`, as removing it in Settings does.
    pub async fn remove_sign_in(&self, provider: &str) {
        let Ok(Value::Array(existing)) = self.get_data("/credential").await else {
            return;
        };
        for credential in existing {
            let field = |key: &str| {
                credential
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            };
            if field("integrationID") == provider && field("label") == SIGN_IN_LABEL {
                self.delete_credential(field("id"), provider).await;
            }
        }
    }

    /// The OAuth sign-ins OpenCode offers for supported providers, leaving out a method whose
    /// form asks for a value it gives no default for.
    pub async fn sign_in_methods(&self) -> Result<Vec<OpencodeSignInMethod>, String> {
        let integrations = self.get_data("/integration").await?;
        let mut out = Vec::new();
        for integration in integrations.as_array().into_iter().flatten() {
            let provider = integration
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if provider.is_empty() || !opencode_provider_is_supported(provider) {
                continue;
            }
            for method in integration
                .get("methods")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let answerable = method
                    .get("form")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .all(|field| field_default(field).is_some() || !field_required(field));
                if method.get("type").and_then(Value::as_str) != Some("oauth") || !answerable {
                    continue;
                }
                let (Some(id), label) = (
                    method.get("id").and_then(Value::as_str),
                    method
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or(provider),
                ) else {
                    continue;
                };
                out.push(OpencodeSignInMethod {
                    provider: provider.to_string(),
                    method_id: id.to_string(),
                    label: label.to_string(),
                });
            }
        }
        Ok(out)
    }

    pub async fn begin_sign_in(
        &self,
        provider: &str,
        method_id: &str,
    ) -> Result<OpencodeSignInAttempt, String> {
        let methods = self.get_data(&format!("/integration/{provider}")).await?;
        let form = methods
            .get("methods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|m| m.get("id").and_then(Value::as_str) == Some(method_id))
            .and_then(|m| m.get("form"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let answer: serde_json::Map<String, Value> = form
            .iter()
            .filter_map(|field| {
                Some((
                    field.get("key")?.as_str()?.to_string(),
                    field_default(field)?,
                ))
            })
            .collect();
        let mut body = json!({ "methodID": method_id, "label": SIGN_IN_LABEL });
        if !answer.is_empty() {
            body["answer"] = Value::Object(answer);
        }
        let mut started: Value = self
            .request(
                reqwest::Method::POST,
                &format!("/integration/{provider}/connect/oauth"),
            )
            .json(&body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.without_url().to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let attempt = started
            .get_mut("data")
            .map(Value::take)
            .unwrap_or(Value::Null);
        let text = |key: &str| {
            attempt
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let attempt_id = text("attemptID");
        if attempt_id.is_empty() {
            return Err("OpenCode started no sign-in".into());
        }
        Ok(OpencodeSignInAttempt {
            provider: provider.to_string(),
            attempt_id,
            url: text("url"),
            instructions: text("instructions"),
            needs_code: text("mode") == "code",
            accepts_paste: attempt.get("paste").and_then(Value::as_bool) == Some(true),
        })
    }

    pub async fn sign_in_status(
        &self,
        provider: &str,
        attempt: &str,
    ) -> Result<OpencodeSignInStatus, String> {
        let path = format!("/integration/{provider}/connect/oauth/{attempt}");
        let status = self.get_data(&path).await?;
        Ok(match status.get("status").and_then(Value::as_str) {
            Some("complete") => OpencodeSignInStatus::Complete,
            Some("pending") => OpencodeSignInStatus::Pending,
            other => OpencodeSignInStatus::Failed {
                message: status
                    .pointer("/error/message")
                    .or_else(|| status.get("message"))
                    .and_then(Value::as_str)
                    .map_or_else(
                        || format!("the sign-in ended: {}", other.unwrap_or("unknown")),
                        str::to_string,
                    ),
            },
        })
    }

    /// A code, or the address a browser that could not reach the server's loopback landed on.
    pub async fn complete_sign_in(
        &self,
        provider: &str,
        attempt: &str,
        code: &str,
    ) -> Result<(), String> {
        let path = format!("/integration/{provider}/connect/oauth/{attempt}/complete");
        self.request(reqwest::Method::POST, &path)
            .json(&json!({ "code": code }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map(|_| ())
            .map_err(|e| e.without_url().to_string())
    }

    /// Reads the attempt until it ends and records the outcome, so a sign-in the browser
    /// finishes while no client polls still counts. It holds the server: a retired one stays up
    /// for the attempt, whose loopback it serves.
    pub fn follow_sign_in(
        self: &Arc<Self>,
        provider: String,
        attempt: String,
        sign_ins: SignInRecords,
    ) -> tokio::task::JoinHandle<()> {
        let server = self.clone();
        self.spawn(async move {
            let deadline = tokio::time::Instant::now() + SIGN_IN_FOLLOW_LIMIT;
            let recorded = loop {
                match server.sign_in_status(&provider, &attempt).await {
                    Ok(OpencodeSignInStatus::Complete) => break sign_ins.completed(&provider),
                    Ok(OpencodeSignInStatus::Failed { .. }) => break sign_ins.abandoned(&provider),
                    // A server that stopped answering leaves the pending record to the next sync.
                    Err(_) => return,
                    Ok(OpencodeSignInStatus::Pending) => {}
                }
                if tokio::time::Instant::now() >= deadline {
                    return;
                }
                tokio::time::sleep(SIGN_IN_FOLLOW_EVERY).await;
            };
            if let Err(e) = recorded {
                tracing::warn!("opencode sign-in for {provider} not recorded: {e}");
            }
        })
    }

    pub async fn cancel_sign_in(&self, provider: &str, attempt: &str) {
        let path = format!("/integration/{provider}/connect/oauth/{attempt}");
        let _ = self.request(reqwest::Method::DELETE, &path).send().await;
    }

    /// OpenCode answers `/api/info` before it has loaded its models (2.4.1: about 300 ms
    /// later), so this waits a bounded while for a non-empty list.
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

fn field_required(field: &Value) -> bool {
    field.get("required").and_then(Value::as_bool) == Some(true)
}

/// The answer a form field gives itself: its default, or the only option a required one has.
fn field_default(field: &Value) -> Option<Value> {
    if let Some(default) = field.get("default").filter(|d| !d.is_null()) {
        return Some(default.clone());
    }
    let options = field.get("options").and_then(Value::as_array)?;
    match options.as_slice() {
        [only] => only.get("value").cloned(),
        _ => None,
    }
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

    /// The credential and sign-in routes of OpenCode 2's API, over one store that outlives any
    /// one `OpenCodeServer`, as its data dir outlives a restart.
    #[derive(Default)]
    struct FakeApi {
        credentials: Vec<Value>,
        attempts: HashMap<String, &'static str>,
        status_reads: u32,
        deleted: Vec<String>,
    }

    type Fake = Arc<StdMutex<FakeApi>>;

    async fn serve_fake(api: Fake) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(answer_fake(stream, api.clone()));
            }
        });
        url
    }

    async fn answer_fake(mut stream: tokio::net::TcpStream, api: Fake) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
            }
        }
        let head = String::from_utf8_lossy(&raw).to_string();
        let mut words = head.split_whitespace();
        let (method, path) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let segments: Vec<&str> = path.trim_start_matches("/api/").split('/').collect();
        let data = {
            let mut api = api.lock().unwrap();
            match (method, segments.as_slice()) {
                ("GET", ["credential"]) => Some(json!(api.credentials)),
                ("DELETE", ["credential", id]) => {
                    api.credentials.retain(|c| c["id"] != *id);
                    api.deleted.push(id.to_string());
                    Some(Value::Null)
                }
                ("GET", ["integration", _, "connect", "oauth", attempt]) => {
                    api.status_reads += 1;
                    api.attempts
                        .get(*attempt)
                        .map(|status| json!({ "status": status }))
                }
                _ => None,
            }
        };
        let (status, body) = match data {
            Some(data) => ("200 OK", json!({ "data": data }).to_string()),
            None => (
                "404 Not Found",
                json!({ "_tag": "NotFoundError" }).to_string(),
            ),
        };
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(reply.as_bytes()).await;
    }

    #[cfg(unix)]
    fn server_on(base_url: &str) -> Arc<OpenCodeServer> {
        let mut placeholder = Command::new("sleep");
        placeholder.arg("60");
        let (events, _) = broadcast::channel(EVENT_CAP);
        Arc::new(OpenCodeServer {
            base_url: base_url.to_string(),
            password: generate_password(),
            client: reqwest::Client::new(),
            events,
            exited: watch::channel(None).0,
            model_context: HashMap::new(),
            home: PathBuf::new(),
            tasks: Handle::current(),
            pid_file: PathBuf::new(),
            child: StdMutex::new(crate::process::spawn_supervised(&mut placeholder).unwrap()),
            retired: AtomicBool::default(),
            stops: StdMutex::default(),
        })
    }

    fn sign_in_credential(id: &str, provider: &str) -> Value {
        json!({ "id": id, "integrationID": provider, "label": SIGN_IN_LABEL,
                "value": { "type": "oauth", "methodID": "device" } })
    }

    fn records(dir: &Path) -> (PathBuf, SignInRecords) {
        let auth = dir.join("auth.json");
        std::fs::write(&auth, "{}").unwrap();
        let sign_ins = SignInRecords {
            signed_in: dir.join("sign-ins.json"),
            pending: dir.join("sign-ins-pending.json"),
        };
        (auth, sign_ins)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_sign_in_finished_while_nobody_polls_survives_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let (auth, sign_ins) = records(dir.path());
        let api = Fake::default();
        api.lock()
            .unwrap()
            .attempts
            .insert("con_1".into(), "pending");
        let url = serve_fake(api.clone()).await;

        sign_ins.begun("openai").unwrap();
        let follower =
            server_on(&url).follow_sign_in("openai".into(), "con_1".into(), sign_ins.clone());
        while api.lock().unwrap().status_reads == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(sign_ins.signed_in().is_empty());
        {
            let mut api = api.lock().unwrap();
            api.attempts.insert("con_1".into(), "complete");
            api.credentials.push(sign_in_credential("crd_1", "openai"));
        }
        tokio::time::timeout(Duration::from_secs(10), follower)
            .await
            .expect("the follower reads the attempt's end")
            .unwrap();
        assert_eq!(sign_ins.signed_in(), ["openai"]);
        assert!(sign_ins.pending().is_empty());

        server_on(&url).sync_credentials(&auth, &sign_ins).await;
        let api = api.lock().unwrap();
        assert!(api.deleted.is_empty(), "deleted {:?}", api.deleted);
        assert_eq!(api.credentials[0]["id"], "crd_1");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_before_the_attempt_is_read_keeps_and_records_its_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let (auth, sign_ins) = records(dir.path());
        let api = Fake::default();
        {
            let mut api = api.lock().unwrap();
            api.credentials.push(sign_in_credential("crd_1", "openai"));
            api.credentials.push(sign_in_credential("crd_2", "xai"));
        }
        let url = serve_fake(api.clone()).await;
        sign_ins.begun("openai").unwrap();
        sign_ins.begun("poe").unwrap();

        server_on(&url).sync_credentials(&auth, &sign_ins).await;
        assert_eq!(sign_ins.signed_in(), ["openai"]);
        assert!(sign_ins.pending().is_empty());
        let api = api.lock().unwrap();
        assert_eq!(
            api.deleted,
            ["crd_2"],
            "a sign-in nobody began or kept goes"
        );
    }
}
