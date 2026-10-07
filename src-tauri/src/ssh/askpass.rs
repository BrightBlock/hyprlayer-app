//! ssh's questions, answered in the window. ssh runs this app's own binary as
//! `SSH_ASKPASS`; `helper` sends the question to the broker over a socket and
//! prints whatever the user answered.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, watch, OnceCell};

use crate::hosts::Host;

const SOCKET_ENV: &str = "HYPRLAYER_ASKPASS_SOCKET";
const TOKEN_ENV: &str = "HYPRLAYER_ASKPASS_TOKEN";
/// ssh's longest question, a host-key banner, is a few hundred bytes.
const MAX_REQUEST: u64 = 64 * 1024;
const REQUEST_READ: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SshPromptKind {
    /// First contact: trust this host's key?
    HostKey,
    /// A password, passphrase, PIN or one-time code.
    Secret,
    /// A yes/no question (`SSH_ASKPASS_PROMPT=confirm`).
    Confirm,
    /// Nothing to answer, such as "touch your security key".
    Notice,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SshPrompt {
    pub id: String,
    pub host: Host,
    pub kind: SshPromptKind,
    /// ssh's own words.
    pub message: String,
    pub key_type: Option<String>,
    pub fingerprint: Option<String>,
    /// The host a key prompt is about, as ssh names it. A jump host's is not `host`.
    pub key_host: Option<String>,
    /// ssh asked this again: the last answer was refused.
    pub retry: bool,
}

/// Why a sign-in stopped before ssh finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    Cancelled,
    TimedOut,
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: String,
    hint: Option<String>,
    /// The ssh that asked: a `ProxyJump` hop is a second ssh sharing the token.
    #[serde(default)]
    parent: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct Response {
    answer: Option<String>,
}

/// The askpass run of this binary, when ssh started it; `main` exits with its code.
pub fn helper() -> Option<i32> {
    let socket = std::env::var_os(SOCKET_ENV)?;
    let request = Request {
        token: std::env::var(TOKEN_ENV).unwrap_or_default(),
        prompt: std::env::args_os()
            .nth(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .unwrap_or_default(),
        hint: std::env::var("SSH_ASKPASS_PROMPT").ok(),
        parent: parent_pid(),
    };
    let Some(answer) = ask(Path::new(&socket), &request) else {
        return Some(1);
    };
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let printed = writeln!(out, "{answer}").and_then(|()| out.flush());
    Some(if printed.is_ok() { 0 } else { 1 })
}

#[cfg(unix)]
fn parent_pid() -> Option<u32> {
    Some(std::os::unix::process::parent_id())
}

#[cfg(not(unix))]
fn parent_pid() -> Option<u32> {
    None
}

#[cfg(unix)]
fn ask(socket: &Path, request: &Request) -> Option<String> {
    use std::io::{BufRead, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
    writeln!(stream, "{}", serde_json::to_string(request).ok()?).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str::<Response>(&line).ok()?.answer
}

#[cfg(not(unix))]
fn ask(_: &Path, _: &Request) -> Option<String> {
    None
}

/// Holds every question ssh is waiting on. One per app; cheap to clone.
#[derive(Clone)]
pub(crate) struct Askpass(Arc<Broker>);

struct Broker {
    helper: PathBuf,
    socket: PathBuf,
    timeout: Duration,
    listening: OnceCell<Result<(), String>>,
    state: Mutex<State>,
    prompts: watch::Sender<Vec<SshPrompt>>,
}

#[derive(Default)]
struct State {
    /// By token: one per ssh run that may ask.
    runs: HashMap<String, Run>,
    /// By prompt id: the run it belongs to and where its answer goes.
    waiting: HashMap<String, (String, oneshot::Sender<String>)>,
    clock: Clock,
}

struct Run {
    host: Host,
    /// By asking ssh: a jump hop's identical "user@host's password" is not a retry.
    asked: Vec<(Option<u32>, String)>,
    /// sshd's login grace period covers every question in one login, not each.
    deadline: tokio::time::Instant,
    ended: watch::Sender<Option<Ended>>,
}

/// How long the window has had a question up, so deadlines can stand still meanwhile.
#[derive(Default)]
struct Clock {
    total: Duration,
    since: Option<Instant>,
}

impl Clock {
    fn read(&self) -> Duration {
        self.total + self.since.map_or(Duration::ZERO, |since| since.elapsed())
    }
}

/// One ssh run's standing with the broker; dropping it withdraws its questions.
pub(crate) struct SignIn {
    broker: Arc<Broker>,
    token: String,
    ended: watch::Receiver<Option<Ended>>,
}

impl SignIn {
    pub(crate) fn env(&self) -> [(&'static str, OsString); 4] {
        [
            ("SSH_ASKPASS", self.broker.helper.clone().into()),
            // Even with a terminal attached, as under `tauri dev`.
            ("SSH_ASKPASS_REQUIRE", "force".into()),
            (SOCKET_ENV, self.broker.socket.clone().into()),
            (TOKEN_ENV, self.token.clone().into()),
        ]
    }

    /// Resolves once the user cancels one of this run's questions or it times out.
    pub(crate) async fn ended(&mut self) -> Ended {
        loop {
            if let Some(ended) = *self.ended.borrow_and_update() {
                return ended;
            }
            if self.ended.changed().await.is_err() {
                return std::future::pending().await;
            }
        }
    }
}

impl Drop for SignIn {
    fn drop(&mut self) {
        self.broker.state().runs.remove(&self.token);
    }
}

impl Askpass {
    pub(crate) fn new(helper: PathBuf, socket: PathBuf, timeout: Duration) -> Self {
        Self(Arc::new(Broker {
            helper,
            socket,
            timeout,
            listening: OnceCell::new(),
            state: Mutex::new(State::default()),
            prompts: watch::Sender::new(Vec::new()),
        }))
    }

    /// Registers an ssh run that may ask about `host`.
    pub(crate) async fn sign_in(&self, host: &Host) -> Result<SignIn, String> {
        // Moved or replaced since launch: ssh would hear nothing and send an empty password.
        if !self.0.helper.is_file() {
            return Err(format!("{} is gone", self.0.helper.display()));
        }
        self.0
            .listening
            .get_or_init(|| listen(self.0.clone()))
            .await
            .clone()?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let (ended, rx) = watch::channel(None);
        self.0.state().runs.insert(
            token.clone(),
            Run {
                host: host.clone(),
                asked: Vec::new(),
                deadline: tokio::time::Instant::now() + self.0.timeout,
                ended,
            },
        );
        Ok(SignIn {
            broker: self.0.clone(),
            token,
            ended: rx,
        })
    }

    pub(crate) fn prompts(&self) -> watch::Receiver<Vec<SshPrompt>> {
        self.0.prompts.subscribe()
    }

    /// `None` cancels: the run is stopped rather than told "no", which ssh
    /// would answer by sending an empty password and asking again.
    pub(crate) fn answer(&self, id: &str, answer: Option<String>) -> bool {
        let mut state = self.0.state();
        let Some((token, reply)) = state.waiting.remove(id) else {
            return false;
        };
        match answer {
            Some(text) => {
                let _ = reply.send(text);
            }
            None => {
                if let Some(run) = state.runs.get(&token) {
                    log::info!("signing in to {} was cancelled", run.host.destination);
                    let _ = run.ended.send(Some(Ended::Cancelled));
                }
            }
        }
        self.0.remove_prompt(&mut state, id);
        true
    }

    /// Time any question has been on screen, since the app started.
    pub(crate) fn waited(&self) -> Duration {
        self.0.state().clock.read()
    }
}

impl Broker {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(&self, request: Request) -> Option<Opened> {
        let mut state = self.state();
        let run = state.runs.get_mut(&request.token)?;
        if run.ended.borrow().is_some() {
            return None;
        }
        let asked = (request.parent, request.prompt.clone());
        let retry = run.asked.contains(&asked);
        run.asked.push(asked);
        let Classified {
            kind,
            key_type,
            fingerprint,
            key_host,
        } = classify(&request.prompt, request.hint.as_deref());
        let prompt = SshPrompt {
            id: uuid::Uuid::new_v4().simple().to_string(),
            host: run.host.clone(),
            kind,
            message: request.prompt,
            key_type,
            fingerprint,
            key_host,
            retry,
        };
        let deadline = run.deadline;
        let ended = run.ended.subscribe();
        log::info!(
            "ssh asks about {}: {kind:?}{}",
            prompt.host.destination,
            if retry { ", again" } else { "" }
        );
        let (reply, answer) = oneshot::channel();
        state
            .waiting
            .insert(prompt.id.clone(), (request.token, reply));
        if state.clock.since.is_none() {
            state.clock.since = Some(Instant::now());
        }
        self.prompts.send_modify(|list| list.push(prompt.clone()));
        Some(Opened {
            id: prompt.id,
            answer,
            deadline,
            ended,
        })
    }

    fn remove_prompt(&self, state: &mut State, id: &str) {
        state.waiting.remove(id);
        let mut empty = false;
        self.prompts.send_if_modified(|list| {
            let before = list.len();
            list.retain(|p| p.id != id);
            empty = list.is_empty();
            list.len() != before
        });
        if empty {
            if let Some(since) = state.clock.since.take() {
                state.clock.total += since.elapsed();
            }
        }
    }

    fn withdraw(&self, id: &str) {
        let mut state = self.state();
        self.remove_prompt(&mut state, id);
    }

    fn expire(&self, id: &str) {
        let mut state = self.state();
        if let Some((token, _)) = state.waiting.get(id) {
            if let Some(run) = state.runs.get(token) {
                log::warn!(
                    "nobody answered ssh's question about {}; stopping it",
                    run.host.destination
                );
                let _ = run.ended.send(Some(Ended::TimedOut));
            }
        }
        self.remove_prompt(&mut state, id);
    }
}

struct Opened {
    id: String,
    answer: oneshot::Receiver<String>,
    deadline: tokio::time::Instant,
    ended: watch::Receiver<Option<Ended>>,
}

struct Classified {
    kind: SshPromptKind,
    key_type: Option<String>,
    fingerprint: Option<String>,
    key_host: Option<String>,
}

impl Classified {
    fn plain(kind: SshPromptKind) -> Self {
        Self {
            kind,
            key_type: None,
            fingerprint: None,
            key_host: None,
        }
    }
}

/// The kind of question, and for a host key, whose key and what it is.
fn classify(message: &str, hint: Option<&str>) -> Classified {
    match hint {
        Some("none") => return Classified::plain(SshPromptKind::Notice),
        Some("confirm") => return Classified::plain(SshPromptKind::Confirm),
        _ => {}
    }
    if !message.contains("continue connecting (yes/no") {
        return Classified::plain(SshPromptKind::Secret);
    }
    // "ED25519 key fingerprint is: SHA256:…" (OpenSSH 10), "… is SHA256:….", older.
    let line = message.lines().find(|l| l.contains("key fingerprint is"));
    let key_type = line
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string);
    let fingerprint = line
        .and_then(|l| l.split("key fingerprint is").nth(1))
        .map(|rest| {
            rest.trim_start_matches(':')
                .trim()
                .trim_end_matches('.')
                .to_string()
        })
        .filter(|f| !f.is_empty());
    // "The authenticity of host 'nexus (100.64.0.5)' can't be established."
    // Ends at " (", not a quote: a `HostKeyAlias` may hold one.
    let key_host = message
        .split_once("authenticity of host '")
        .and_then(|(_, rest)| rest.split_once(" ("))
        .map(|(named, _)| named.to_string())
        .filter(|h| !h.is_empty() && !h.contains('\n'));
    Classified {
        kind: SshPromptKind::HostKey,
        key_type,
        fingerprint,
        key_host,
    }
}

#[cfg(unix)]
async fn listen(broker: Arc<Broker>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let socket = broker.socket.clone();
    let bound = (|| {
        if let Some(dir) = socket.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Single-instance: a socket here is a previous run's.
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        Ok::<_, std::io::Error>(listener)
    })();
    let listener = bound.map_err(|e| {
        let why = format!(
            "could not listen for ssh prompts at {}: {e}",
            socket.display()
        );
        log::warn!("{why}");
        why
    })?;
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(serve(broker.clone(), stream));
                }
                // Out of descriptors, say. Giving up would leave every later
                // helper hanging on a socket nobody serves.
                Err(e) => {
                    log::warn!("the ssh prompt socket could not accept: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });
    Ok(())
}

#[cfg(not(unix))]
async fn listen(_: Arc<Broker>) -> Result<(), String> {
    Err("ssh prompts need a Unix socket".into())
}

/// One helper connection: its question, then the answer or nothing at all.
#[cfg(unix)]
async fn serve(broker: Arc<Broker>, stream: tokio::net::UnixStream) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();
    let read = tokio::time::timeout(
        REQUEST_READ,
        (&mut reader).take(MAX_REQUEST).read_line(&mut line),
    )
    .await;
    if !matches!(read, Ok(Ok(n)) if n > 0) {
        return;
    }
    let Ok(request) = serde_json::from_str::<Request>(&line) else {
        return;
    };
    let Some(Opened {
        id,
        answer,
        deadline,
        mut ended,
    }) = broker.open(request)
    else {
        let _ = reply(&mut write, None).await;
        return;
    };
    let mut probe = [0u8; 1];
    let answered = tokio::select! {
        answer = answer => answer.ok(),
        // ssh killed its helper (a notice that is done) or died itself.
        _ = reader.read(&mut probe) => {
            broker.withdraw(&id);
            return;
        }
        () = run_over(&mut ended) => {
            broker.withdraw(&id);
            return;
        }
        () = tokio::time::sleep_until(deadline) => {
            broker.expire(&id);
            None
        }
    };
    match answered {
        Some(text) => {
            let _ = reply(&mut write, Some(text)).await;
        }
        // Answering now would let ssh send an empty password before it is
        // killed, so the helper waits for the run to be gone.
        None => {
            tokio::select! {
                () = run_over(&mut ended) => {}
                _ = reader.read(&mut probe) => {}
            }
        }
    }
}

#[cfg(unix)]
async fn reply(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    answer: Option<String>,
) -> std::io::Result<()> {
    let mut line = serde_json::to_string(&Response { answer }).map_err(std::io::Error::other)?;
    line.push('\n');
    write.write_all(line.as_bytes()).await?;
    write.shutdown().await
}

/// Until the run's `SignIn` is dropped.
async fn run_over(ended: &mut watch::Receiver<Option<Ended>>) {
    while ended.changed().await.is_ok() {}
}
