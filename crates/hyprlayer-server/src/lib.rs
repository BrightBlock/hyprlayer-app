//! `hyprlayer-server`: one `HyprlayerRuntime` behind a local socket (ADR-0009).
//! The library is the server; `main.rs` only parses a subcommand.

mod agent_access;
mod agent_plugin;
pub mod client;
mod ctl;
pub mod dispatch;
pub mod events;
pub mod hook;
mod link;
pub mod logging;
mod mcp;
pub mod outbound;
mod relay;
pub mod rpc;
pub mod socket;
mod statusline;
pub mod streams;
pub mod transport;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hyprlayer_core::{
    AgentControlEndpoint, CoreError, HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions,
    RuntimePaths,
};
use hyprlayer_proto::frame::Frame;
use hyprlayer_proto::{
    check_version, Features, Outcome, WireError, PROTOCOL_VERSION, STREAM_WINDOW,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};

use client::{ConnectError, RpcClient};
use dispatch::Server;
use events::Clients;
use hook::HookEngine;
use logging::RepeatGate;
use outbound::Outbound;
use streams::StreamRegistry;
use transport::{
    encode_answer, encode_control, label, parse_control, reject_undecodable, undecodable_id,
    Control, FrameReader, FrameWriter,
};

/// The bundle identifier Tauri derives the desktop's app dirs from; a
/// standalone server must land on the same profile (ADR-0009).
pub const APP_IDENTIFIER: &str = "ai.brightblock.hyprlayer";

const USAGE: &str = "usage: hyprlayer-server [serve|relay|status|stop [--when-idle]|logs|--version|hook <claude|codex> [event]|lead-guard <checkout>|statusline claude|ctl <verb>|mcp [--read-only]]";
const STOP_POLLS: u32 = 100;
/// How much of each log file `logs` prints.
const LOGS_TAIL_BYTES: u64 = 64 * 1024;
const SLOW_REQUEST: Duration = Duration::from_secs(5);

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);
static REQUEST_FAILURES: RepeatGate = RepeatGate::new(Duration::from_secs(60));

tokio::task_local! {
    /// The connection a request arrived on; announce, watch and attribution read it.
    pub(crate) static CONN: u64;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Serve,
    Relay,
    Status,
    Stop {
        when_idle: bool,
    },
    /// Prints the tails of this profile's log files: how a client fetches a
    /// remote host's logs over `ssh`, with or without a running server.
    Logs,
    /// The client provisions a version-keyed server on a remote host and needs
    /// to read back what it installed (plan Phase 7 §3).
    Version,
    /// An engine hook reporting one signal for the agent named by `HYPRLAYER_AGENT_TOKEN`.
    Hook(HookEngine, Option<String>),
    /// A Lead's PreToolUse hook; the main checkout it guards follows.
    LeadGuard(String),
    /// claude's statusLine for a hooked PTY: reports, then runs the user's own.
    Statusline,
    /// An agent's command line into the runtime; the verb and its arguments follow.
    Ctl(Vec<String>),
    /// The `hyprlayer` MCP server over stdio; `--read-only` lists only the read tools.
    Mcp {
        read_only: bool,
    },
}

impl Command {
    /// `serve` is the default; the error is the usage text to print.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let command = match args.next().as_deref() {
            None | Some("serve") => Command::Serve,
            Some("relay") => Command::Relay,
            Some("status") => Command::Status,
            Some("stop") => match args.next().as_deref() {
                None => Command::Stop { when_idle: false },
                Some("--when-idle") => Command::Stop { when_idle: true },
                Some(other) => return Err(format!("unexpected argument `{other}`\n{USAGE}")),
            },
            Some("logs") => Command::Logs,
            Some("--version" | "-V") => Command::Version,
            Some("hook") => {
                let engine = args.next();
                let Some(engine) = engine.as_deref().and_then(HookEngine::parse) else {
                    return Err(format!("hook needs an engine: claude or codex\n{USAGE}"));
                };
                Command::Hook(engine, args.next())
            }
            Some("lead-guard") => match args.next() {
                Some(checkout) => Command::LeadGuard(checkout),
                None => return Err(format!("lead-guard needs the checkout path\n{USAGE}")),
            },
            Some("statusline") => match args.next().as_deref() {
                Some("claude") => Command::Statusline,
                _ => return Err(format!("statusline needs an engine: claude\n{USAGE}")),
            },
            Some("ctl") => return Ok(Command::Ctl(args.collect())),
            Some("mcp") => match args.next().as_deref() {
                None => Command::Mcp { read_only: false },
                Some("--read-only") => Command::Mcp { read_only: true },
                Some(other) => return Err(format!("unexpected argument `{other}`\n{USAGE}")),
            },
            Some(other) => return Err(format!("unknown command `{other}`\n{USAGE}")),
        };
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument `{extra}`\n{USAGE}"));
        }
        Ok(command)
    }
}

/// Runs one subcommand to completion and returns the process exit code.
pub async fn run(command: Command) -> i32 {
    // A speed bump, not a boundary: refused before any profile dir or socket is touched.
    if matches!(command, Command::Stop { .. } | Command::Relay)
        && std::env::var_os("HYPRLAYER_AGENT_TOKEN").is_some_and(|t| !t.is_empty())
    {
        eprintln!("hyprlayer-server: this command is not available inside a hyprlayer agent");
        return 2;
    }
    match command {
        Command::Serve => serve().await,
        Command::Relay => relay().await,
        Command::Status => status().await,
        Command::Stop { when_idle } => {
            if when_idle {
                wait_until_idle().await;
            }
            stop().await
        }
        Command::Logs => {
            let (_, data_dir) = app_dirs();
            print!(
                "{}",
                logging::tail_logs(&logging::log_dir(&data_dir), LOGS_TAIL_BYTES)
            );
            0
        }
        Command::Version => {
            println!("{VERSION}");
            0
        }
        Command::Hook(engine, event) => hook::run_hook(engine, event).await,
        Command::LeadGuard(checkout) => hook::run_lead_guard(&checkout),
        Command::Statusline => statusline::run_statusline().await,
        Command::Ctl(args) => ctl::run(args).await,
        Command::Mcp { read_only } => mcp::run(read_only).await,
    }
}

/// The profile dirs the desktop resolves for `APP_IDENTIFIER`;
/// `HYPRLAYER_APP_CONFIG_DIR` / `HYPRLAYER_APP_DATA_DIR` override them.
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let from_env = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let config = from_env("HYPRLAYER_APP_CONFIG_DIR").unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_IDENTIFIER)
    });
    let data = from_env("HYPRLAYER_APP_DATA_DIR").unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_IDENTIFIER)
    });
    (config, data)
}

/// A server not started by the desktop (systemd, launchd, by hand) has no
/// stderr the desktop can read, so a fatal `open` writes its `AppError`-shaped
/// report here instead. Deleted on the next successful `open`, so a stale
/// report from a since-fixed profile can never mislead a later launch.
fn runtime_fatal_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(".runtime-fatal.json")
}

fn write_runtime_fatal(path: &Path, error: WireError) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_vec_pretty(&error) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(path, bytes) {
                tracing::error!("could not write {}: {e}", path.display());
            }
        }
        Err(e) => tracing::error!("could not encode the fatal-error report: {e}"),
    }
}

/// This server's release version, independent of the desktop's.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Set once by `serve`, before the socket binds, so a binary replaced on disk
/// afterwards cannot change what this process reports.
static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn executable_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    // Streamed: the binary whole is a buffer the allocator would keep for the server's life.
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path).ok()?, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

async fn serve() -> i32 {
    let (config_dir, data_dir) = app_dirs();
    let log_dir = logging::log_dir(&data_dir);
    let _log = logging::init(logging::Process::Serve, &log_dir);
    if let Some(build) = std::env::current_exe()
        .ok()
        .and_then(|exe| executable_sha256(&exe))
    {
        let _ = BUILD.set(build);
    }
    tracing::info!(
        "hyprlayer-server {} starting: pid {}, protocol {PROTOCOL_VERSION}, build {}, {}/{}, \
         config {}, data {}, logs {}",
        VERSION,
        std::process::id(),
        BUILD
            .get()
            .map_or("unknown", |build| &build[..build.len().min(12)]),
        std::env::consts::OS,
        std::env::consts::ARCH,
        config_dir.display(),
        data_dir.display(),
        log_dir.display(),
    );
    let environment = RuntimeEnvironment::detect();
    let paths = RuntimePaths::detect(&environment, config_dir, data_dir);
    let app_data_dir = paths.app_data_dir.clone();
    let fatal_path = runtime_fatal_path(&app_data_dir);

    let mut options = RuntimeOptions::new(environment, paths);
    match std::env::current_exe() {
        Ok(ctl) => {
            options = options.with_agent_control(AgentControlEndpoint {
                ctl,
                socket: socket::bind_path(&app_data_dir),
                plugin_dir: agent_plugin::install(&app_data_dir),
                #[cfg(unix)]
                runtime_dir: socket::runtime_dir(&app_data_dir),
                #[cfg(not(unix))]
                runtime_dir: None,
            });
        }
        Err(e) => tracing::warn!("agents get no control endpoint: cannot resolve this binary: {e}"),
    }

    let runtime = match HyprlayerRuntime::open(options).await {
        Ok(runtime) => runtime,
        Err(e) => {
            // Only the two the desktop's migration-ledger dialog already
            // handles are "fatal" here — `RuntimeInUse` is the supervisor's
            // spawn-retry case (§4), not a persisted, dialog-worthy failure.
            let fatal = matches!(
                e,
                CoreError::RuntimeMigrationLedgerInvalid { .. }
                    | CoreError::RuntimeMigrationLedgerTooNew { .. }
            );
            tracing::error!("runtime failed to open: {}: {e}", e.code());
            // The supervisor reads this line back to recognise `runtime_in_use`.
            eprintln!("{}: {e}", e.code());
            if fatal {
                write_runtime_fatal(&fatal_path, dispatch::wire_error(e));
            }
            return 1;
        }
    };
    let _ = std::fs::remove_file(&fatal_path);
    agent_plugin::prune(&app_data_dir);
    if let Err(e) = runtime.start().await {
        tracing::error!("runtime failed to start: {e}");
    }

    let server = match SocketServer::bind(runtime.clone(), &app_data_dir).await {
        Ok(server) => server,
        Err(e) => {
            let socket = socket::bind_path(&app_data_dir);
            tracing::error!("cannot bind {}: {e}", socket.display());
            eprintln!("cannot bind {}: {e}", socket.display());
            runtime.shutdown().await;
            return 1;
        }
    };
    tracing::info!("listening on {}", server.socket_path().display());

    let started = Instant::now();
    let stop = server.stop_handle();
    tokio::spawn(async move {
        let signal = wait_for_signal().await;
        tracing::info!("received {signal}; stopping");
        stop.stop();
    });
    let restart_watch = tokio::spawn(stop_when_unattended_and_missing_binaries(
        runtime.clone(),
        server.clients(),
        server.stop_handle(),
    ));
    if let Err(e) = server.run().await {
        tracing::error!("server loop failed: {e}");
    }
    restart_watch.abort();
    let report = runtime.shutdown().await;
    let failures = if report.task_failures.is_empty() {
        String::new()
    } else {
        format!("; task failures: {}", report.task_failures.join("; "))
    };
    tracing::info!(
        "stopped after {:?} up; shutdown took {:?} and forced {} task(s){failures}",
        started.elapsed(),
        report.elapsed,
        report.forced_process_count,
    );
    0
}

/// The desktop leaves its server running when it quits, so one started before
/// a required tool was installed would never find it. Once the last client
/// leaves with nothing running here and a required binary still missing, stop:
/// the next launch spawns a server that looks again with a fresh environment.
async fn stop_when_unattended_and_missing_binaries(
    runtime: Arc<HyprlayerRuntime>,
    clients: Clients,
    stop: StopHandle,
) {
    loop {
        clients.wait_unattended().await;
        let missing = runtime.missing_required_binaries_when_idle().await;
        // The probe can take seconds; a client that announced meanwhile keeps us.
        if missing.is_empty() || !clients.is_unattended() {
            tracing::debug!("the last client left; staying up");
            continue;
        }
        let names: Vec<&str> = missing.iter().map(|kind| kind.command_name()).collect();
        tracing::info!(
            "stopping: the last client left and {} is still missing; the next launch \
             starts a server that looks again",
            names.join(", ")
        );
        stop.stop();
        return;
    }
}

/// The desktop's remote transport: `ssh <host> hyprlayer-server relay`, with
/// this process's stdin/stdout as the connection (ADR-0009, plan Phase 5 §1).
async fn relay() -> i32 {
    let (config_dir, data_dir) = app_dirs();
    let log_dir = logging::log_dir(&data_dir);
    let _log = logging::init(logging::Process::Relay, &log_dir);
    tracing::info!("relay {} starting: pid {}", VERSION, std::process::id());
    let io = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    match relay::run(config_dir, data_dir, &log_dir, io).await {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("relay failed: {e}");
            // Reaches the desktop as ssh's stderr.
            eprintln!("relay failed: {e}");
            1
        }
    }
}

async fn wait_for_signal() -> &'static str {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(e) => {
                    tracing::warn!("cannot listen for SIGTERM: {e}");
                    let _ = tokio::signal::ctrl_c().await;
                    return "ctrl-c";
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "ctrl-c",
            _ = term.recv() => "SIGTERM",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "ctrl-c"
    }
}

async fn status() -> i32 {
    let (_, data_dir) = app_dirs();
    let path = socket::socket_path(&data_dir);
    match RpcClient::connect_socket(&path).await {
        Ok(client) => {
            println!(
                "listening on {} (protocol version {})",
                path.display(),
                client.server_version()
            );
            println!("logs in {}", logging::log_dir(&data_dir).display());
            0
        }
        Err(ConnectError::Refused(e)) => {
            println!("{} refused the handshake: {}", path.display(), e.message);
            1
        }
        Err(e) => {
            println!("no server at {} ({e})", path.display());
            1
        }
    }
}

const IDLE_POLL: Duration = Duration::from_secs(15);
const IDLE_SETTLE: Duration = Duration::from_secs(120);

// ponytail: the app's serverAutoUpdate signals minus keystrokes, which only the app
// sees; a command running in a plain terminal with no agent in it reads as idle.
async fn wait_until_idle() {
    use hyprlayer_models::{PtyAgentState, SessionState};
    let (_, data_dir) = app_dirs();
    let path = socket::socket_path(&data_dir);
    let mut idle_since: Option<Instant> = None;
    loop {
        let idle = match RpcClient::connect_socket(&path).await {
            Ok(client) => {
                let sessions = client.list_active_sessions().await;
                let ptys = client.list_pty_agents(None).await;
                let scripts = client.list_running_scripts().await;
                match (sessions, ptys, scripts) {
                    (Ok(sessions), Ok(ptys), Ok(scripts)) => {
                        scripts.is_empty()
                            && !sessions.iter().any(|a| {
                                a.session.working
                                    || matches!(
                                        a.session.state,
                                        SessionState::Starting
                                            | SessionState::AwaitingPermission
                                            | SessionState::AwaitingQuestion
                                    )
                            })
                            && !ptys.iter().any(|p| {
                                matches!(
                                    p.state,
                                    PtyAgentState::Starting
                                        | PtyAgentState::Working
                                        | PtyAgentState::AwaitingPermission
                                        | PtyAgentState::AwaitingQuestion
                                )
                            })
                    }
                    _ => false,
                }
            }
            Err(_) => return,
        };
        if !idle {
            idle_since = None;
        } else if idle_since.get_or_insert_with(Instant::now).elapsed() >= IDLE_SETTLE {
            return;
        }
        tokio::time::sleep(IDLE_POLL).await;
    }
}

async fn stop() -> i32 {
    let (_, data_dir) = app_dirs();
    let path = socket::socket_path(&data_dir);
    let client = match RpcClient::connect_socket(&path).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("no server at {} ({e})", path.display());
            return 1;
        }
    };
    if let Err(e) = client.request_shutdown().await {
        eprintln!("{}", e.message);
        return 1;
    }
    for _ in 0..STOP_POLLS {
        if socket::connect(&path).await.is_err() {
            return 0;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!("server did not stop within {}s", STOP_POLLS / 10);
    1
}

/// Signals a running `SocketServer` to stop accepting and drop its connections.
#[derive(Clone)]
pub struct StopHandle(Arc<Notify>);

impl StopHandle {
    pub fn stop(&self) {
        self.0.notify_one();
    }
}

/// A bound socket serving one runtime. Bind, then `run` until stopped; the
/// runtime's own shutdown is the caller's, so the desktop can host a server
/// against a runtime it also uses directly.
pub struct SocketServer {
    listener: socket::Listener,
    server: Arc<Server>,
    clients: Clients,
    forwarder: JoinHandle<()>,
    stop: Arc<Notify>,
}

impl SocketServer {
    /// Binds the profile's socket and starts forwarding runtime events. Call
    /// from inside a Tokio runtime.
    pub async fn bind(runtime: Arc<HyprlayerRuntime>, app_data_dir: &Path) -> io::Result<Self> {
        let listener = socket::Listener::bind(app_data_dir)?;
        let clients = Clients::default();
        let forwarder = events::spawn_forwarder(&runtime, clients.clone());
        Ok(Self {
            listener,
            server: Arc::new(Server::new(runtime, clients.clone())),
            clients,
            forwarder,
            stop: Arc::new(Notify::new()),
        })
    }

    pub fn socket_path(&self) -> &Path {
        self.listener.path()
    }

    pub fn stop_handle(&self) -> StopHandle {
        StopHandle(self.stop.clone())
    }

    pub fn clients(&self) -> Clients {
        self.clients.clone()
    }

    /// Accepts until stopped, then aborts every connection. Each connection
    /// gets a reader and a writer task; each request its own task.
    pub async fn run(mut self) -> io::Result<()> {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    Ok(stream) => {
                        let peer = socket::peer(&stream);
                        connections.spawn(handle_connection(
                            stream,
                            peer,
                            self.server.clone(),
                            self.clients.clone(),
                            self.stop.clone(),
                        ));
                        while connections.try_join_next().is_some() {}
                    }
                    Err(e) => {
                        tracing::warn!("accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                _ = self.stop.notified() => break,
            }
        }
        tracing::info!("closing {} connection(s)", connections.len());
        connections.shutdown().await;
        self.forwarder.abort();
        Ok(())
    }
}

async fn handle_connection<S>(
    stream: S,
    peer: String,
    server: Arc<Server>,
    clients: Clients,
    stop: Arc<Notify>,
) where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    match serve_connection(conn, stream, &peer, server, clients, stop).await {
        Ok(Some((requests, true))) => tracing::info!(
            "conn {conn}: closed after {:?}, {requests} request(s)",
            started.elapsed()
        ),
        Ok(Some((requests, false))) => tracing::debug!(
            "conn {conn}: closed after {:?}, {requests} request(s)",
            started.elapsed()
        ),
        Ok(None) => {}
        Err(e) => tracing::info!("conn {conn}: dropped after {:?}: {e}", started.elapsed()),
    }
}

/// `Ok(None)` when the peer never got past the handshake; else the request count, and whether it was logged at info.
async fn serve_connection<S>(
    conn: u64,
    stream: S,
    peer: &str,
    server: Arc<Server>,
    clients: Clients,
    stop: Arc<Notify>,
) -> io::Result<Option<(u64, bool)>>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);

    let features = match handshake(conn, &mut reader, &mut writer, &stop).await {
        Ok(Some(features)) => features,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::debug!("conn {conn}: closed during the handshake: {e}");
            return Ok(None);
        }
    };
    tracing::debug!("conn {conn}: client connected ({peer})");

    let out = Outbound::new(features, features.link_credit);
    clients.register(conn, out.clone());
    streams::open_connection(&server, conn);
    let writer_task = tokio::spawn({
        let out = out.clone();
        async move {
            if let Err(e) = outbound::write(out, writer).await {
                tracing::debug!("conn {conn}: write failed: {e}");
            }
        }
    });
    let streams = StreamRegistry::default();
    let _ending = ConnectionGuard {
        out: out.clone(),
        writer: writer_task.abort_handle(),
        streams: streams.clone(),
    };

    let result = read_requests(conn, peer, &mut reader, &server, &out, &stop, &streams).await;
    streams::fail_abandoned_launches(&server, conn).await;
    let left = clients.unregister(conn);
    clients.send_viewers(&left);
    server.runtime().terminals().forget_viewer(conn);
    // Read from who is left, not from `unregister`: a send may already have pruned this one.
    let _ = server.sync_worktree_watch().await;
    result.map(Some)
}

/// Ends a connection's writer and streams however its task ends, an abort at
/// shutdown included, so the socket closes and the client sees the disconnect.
struct ConnectionGuard {
    out: Outbound,
    writer: tokio::task::AbortHandle,
    streams: StreamRegistry,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.streams.abort_all();
        self.out.close();
        self.writer.abort();
    }
}

/// Once the peer's hello is accepted, the features it announced.
async fn handshake<R, W>(
    conn: u64,
    reader: &mut FrameReader<R>,
    writer: &mut FrameWriter<W>,
    stop: &Notify,
) -> io::Result<Option<Features>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: BUILD.get().cloned(),
            version: Some(VERSION.to_string()),
            features: Features::ALL.names(),
            stream_window: Some(STREAM_WINDOW),
        })
        .await?;
    match reader.next_control().await? {
        Some(Control::Hello {
            protocol_version,
            features,
            ..
        }) => {
            let Err(error) = check_version(protocol_version) else {
                return Ok(Some(Features::parse(&features)));
            };
            tracing::warn!(
                "conn {conn}: refused a client speaking protocol {protocol_version}; \
                 this server speaks {PROTOCOL_VERSION}"
            );
            let refused = writer.write_control(&Control::Refused { error }).await;
            // A version mismatch normally ends the connection here. The one
            // exception (Phase 4 §5): a client strictly newer than us is
            // mid-upgrade and gets one more read, to ask us to step aside —
            // an equal-or-older client's `Shutdown` here is ignored, or two
            // mismatched clients could evict each other in a loop.
            if protocol_version > PROTOCOL_VERSION {
                if let Ok(Some(Control::Shutdown)) = reader.next_control().await {
                    tracing::warn!(
                        "conn {conn}: stepping aside for a newer client; every session and \
                         terminal on this server ends"
                    );
                    stop.notify_one();
                }
            }
            refused.map(|()| None)
        }
        Some(other) => {
            tracing::warn!("conn {conn}: expected hello, got {}", label(&other));
            let error = WireError::protocol(format!("expected hello, got {}", label(&other)));
            writer.write_control(&Control::Refused { error }).await?;
            Ok(None)
        }
        None => Ok(None),
    }
}

/// A hook or statusLine face: one short connection per agent event, too many to log at info.
fn is_face_report(command: &str) -> bool {
    matches!(
        command,
        "report_agent_signal" | "report_agent_status" | "model_switch_pending"
    )
}

/// Returns how many requests the connection made, and whether it announced itself at info.
async fn read_requests<R: AsyncRead + Unpin>(
    conn: u64,
    peer: &str,
    reader: &mut FrameReader<R>,
    server: &Arc<Server>,
    tx: &Outbound,
    stop: &Arc<Notify>,
    streams: &StreamRegistry,
) -> io::Result<(u64, bool)> {
    let mut requests = 0u64;
    let mut loud = false;
    let mut announce = |command: &str| {
        if !loud && !is_face_report(command) {
            loud = true;
            tracing::info!("conn {conn}: client connected ({peer})");
        }
    };
    // Reports what it read to a client that windows its own writes, uploads included.
    let mut reports = tx
        .link_credit()
        .then(|| outbound::ReadReports::new(tx.clone(), reader.consumed()));
    let mut heartbeat = tx.heartbeat().then(|| outbound::Heartbeat::new(tx.clone()));
    // Never waits on the writer, so credit, cancels and `received` always land.
    loop {
        let Some(frame) = outbound::next_frame(reader, &mut reports, &mut heartbeat).await? else {
            break;
        };
        let line = match frame {
            Frame::Control(line) => line,
            Frame::Data(_) => {
                tracing::debug!(
                    "conn {conn}: data frame ignored: the data plane only carries stream events"
                );
                continue;
            }
        };
        let control = match parse_control(&line) {
            Ok(control) => control,
            // Answered rather than dropped: a client newer than this server
            // would otherwise lose every in-flight call and reconnect.
            // Never fatal: one unreadable frame must not end the sessions this
            // connection carries.
            Err(e) => {
                let reply = match reject_undecodable(&line) {
                    Some((command, reply)) => {
                        tracing::warn!(
                            "conn {conn}: refused {command}: this build cannot decode it"
                        );
                        Some(reply)
                    }
                    None => {
                        tracing::warn!("conn {conn}: skipped an unreadable control frame: {e}");
                        match undecodable_id(&line) {
                            Some((kind, id)) if kind == "request" => Some(Control::Response {
                                id,
                                outcome: Outcome::Err(WireError::unreadable_request()),
                            }),
                            Some((kind, id)) if kind == "stream_request" => {
                                Some(Control::StreamResponse {
                                    id,
                                    outcome: Outcome::Err(WireError::unreadable_request()),
                                })
                            }
                            _ => None,
                        }
                    }
                };
                if let Some(reply) = reply {
                    let bytes = encode_control(&reply).expect("an error outcome always serializes");
                    tx.push_frame(bytes);
                }
                continue;
            }
        };
        match control {
            Control::Request { id, body } => {
                requests += 1;
                announce(body.command());
                // Before the read loop can see this connection close and fail the start.
                if let crate::rpc::Request::LaunchAndConverse { session_id } = &body {
                    streams::claim_pending_launch(server, session_id);
                }
                let server = server.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _slot = if body.takes_no_slot() {
                        None
                    } else {
                        tx.request_slot(body.answers_in_bulk()).await
                    };
                    let command = body.command();
                    let started = Instant::now();
                    // Its own task, so a panicking handler still gets an answer
                    // back to the caller instead of leaving it waiting.
                    let outcome: Outcome<_> =
                        // A task-local scope does not survive a further spawn, so it wraps the inner one.
                        match tokio::spawn(CONN.scope(conn, async move { server.answer(body).await }))
                            .await
                        {
                            Ok(result) => result.into(),
                            Err(e) if e.is_panic() => {
                                tracing::error!("conn {conn}: {command} panicked: {e}");
                                Outcome::Err(crashed(command))
                            }
                            Err(_) => return,
                        };
                    log_outcome(conn, command, started.elapsed(), &outcome);
                    let body = encode_answer(id, outcome).unwrap_or_else(|e| {
                        tracing::error!("{command}: result is not serializable: {e}");
                        let outcome = Outcome::Err(WireError::internal(format!(
                            "{command}: result is not serializable: {e}"
                        )));
                        encode_control(&Control::Response { id, outcome })
                            .expect("an error outcome always serializes")
                            .into()
                    });
                    if !tx.push_frame_written(body).await {
                        tracing::debug!("conn {conn}: {command} finished after the client left");
                    }
                });
            }
            Control::StreamRequest { id, body } => {
                requests += 1;
                announce(body.command());
                let server = server.clone();
                let tx = tx.clone();
                let streams = streams.clone();
                tokio::spawn(async move {
                    // Held until the response is written; the stream runs on without it.
                    let _slot = tx.request_slot(false).await;
                    let command = body.command();
                    let started = Instant::now();
                    let preparing = server.clone();
                    let prepared = match tokio::spawn(CONN.scope(conn, async move {
                        streams::prepare_stream(&preparing, body).await
                    }))
                    .await
                    {
                        Ok(prepared) => prepared,
                        Err(e) if e.is_panic() => {
                            tracing::error!("conn {conn}: {command} panicked: {e}");
                            Err(crashed(command))
                        }
                        Err(_) => return,
                    };
                    match prepared {
                        Ok((response, source)) => {
                            let stream_id = response.stream_id();
                            let spawned_terminal = match &response {
                                streams::StreamResponse::Terminal { terminal_id, .. }
                                    if matches!(
                                        command,
                                        "spawn_cli_terminal" | "spawn_shell_terminal"
                                    ) =>
                                {
                                    Some(terminal_id.clone())
                                }
                                _ => None,
                            };
                            tracing::debug!(
                                "conn {conn}: {command} opened stream {stream_id:?} in {:?}",
                                started.elapsed()
                            );
                            let bytes = encode_control(&Control::StreamResponse {
                                id,
                                outcome: Outcome::Ok(response),
                            })
                            .unwrap_or_else(|e| {
                                tracing::error!("stream response is not serializable: {e}");
                                encode_control(&Control::StreamResponse {
                                    id,
                                    outcome: Outcome::Err(WireError::internal(format!(
                                        "stream response is not serializable: {e}"
                                    ))),
                                })
                                .expect("an error outcome always serializes")
                            });
                            // Held before the client can see the id, so its cancel can't arrive first.
                            if let Some(stream_id) = stream_id {
                                streams.reserve(stream_id);
                            }
                            // The response must reach the wire before a byte of
                            // the stream it names does, or a fast source could
                            // race a data frame ahead of the id that explains it.
                            if !tx.push_frame_written(bytes).await {
                                if let Some(stream_id) = stream_id {
                                    streams.remove(stream_id);
                                }
                                // Nothing lists a workstream-less terminal, so no one else could close it.
                                if let Some(terminal_id) = spawned_terminal {
                                    let close =
                                        hyprlayer_core::runtime::terminals::CloseTerminalRequest {
                                            terminal_id,
                                        };
                                    if let Err(e) = server.runtime().terminals().close(close).await
                                    {
                                        tracing::warn!(
                                            "conn {conn}: closing an unclaimed {command}: {e}"
                                        );
                                    }
                                }
                                return;
                            }
                            if let (Some(stream_id), Some(source)) = (stream_id, source) {
                                let rows = source
                                    .rows_of()
                                    .map(|session| server.clients().stream_rows(conn, session));
                                streams::spawn_stream(
                                    server.runtime(),
                                    stream_id,
                                    source,
                                    tx,
                                    streams,
                                    rows,
                                );
                            }
                        }
                        Err(error) => {
                            log_failure(conn, command, &error, started.elapsed());
                            let bytes = encode_control(&Control::StreamResponse {
                                id,
                                outcome: Outcome::Err(error),
                            })
                            .expect("a WireError outcome always serializes");
                            tx.push_frame(bytes);
                        }
                    }
                });
            }
            Control::Cancel { stream_id } => {
                tracing::debug!("conn {conn}: cancelled stream {stream_id}");
                streams.abort(stream_id)
            }
            Control::Credit { stream_id, bytes } => streams.grant(stream_id, bytes),
            Control::Received { bytes, read_us } => {
                tx.received(bytes, read_us.map(Duration::from_micros))
            }
            Control::Shutdown if server.is_agent(conn) => {
                tracing::warn!("conn {conn}: refused an agent connection's shutdown");
            }
            Control::Shutdown => {
                tracing::info!("conn {conn}: asked the server to stop");
                stop.notify_one();
                return Ok((requests, loud));
            }
            other => tracing::warn!(
                "conn {conn}: unexpected {} frame from client",
                label(&other)
            ),
        }
    }
    Ok((requests, loud))
}

fn crashed(command: &str) -> WireError {
    WireError::internal(format!(
        "the server hit an internal error handling {command}; its log has the details"
    ))
}

fn log_outcome<T>(conn: u64, command: &str, elapsed: Duration, outcome: &Outcome<T>) {
    match outcome {
        Outcome::Err(error) => log_failure(conn, command, error, elapsed),
        Outcome::Ok(_) if elapsed >= SLOW_REQUEST => {
            tracing::info!("conn {conn}: {command} took {elapsed:?}")
        }
        Outcome::Ok(_) => tracing::trace!("conn {conn}: {command} in {elapsed:?}"),
    }
}

/// Command name, code and message only: arguments carry keys, keystrokes and
/// prompts.
fn log_failure(conn: u64, command: &str, error: &WireError, elapsed: Duration) {
    let Some(held) = REQUEST_FAILURES.admit(&format!("{command}:{}", error.code)) else {
        return;
    };
    let repeats = if held > 0 {
        format!(" ({held} more since the last report)")
    } else {
        String::new()
    };
    tracing::warn!(
        "conn {conn}: {command} failed after {elapsed:?}: {}: {}{repeats}",
        error.code,
        hyprlayer_core::redact::url_credentials(&error.message)
    );
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use hyprlayer_core::terminal::{PtySession, SpawnTerminal};

    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        Command::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn serve_is_the_default_and_extra_arguments_are_usage_errors() {
        assert_eq!(parse(&[]), Ok(Command::Serve));
        assert_eq!(parse(&["serve"]), Ok(Command::Serve));
        assert_eq!(parse(&["relay"]), Ok(Command::Relay));
        assert_eq!(parse(&["status"]), Ok(Command::Status));
        assert_eq!(parse(&["stop"]), Ok(Command::Stop { when_idle: false }));
        assert_eq!(
            parse(&["stop", "--when-idle"]),
            Ok(Command::Stop { when_idle: true })
        );
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
        assert!(parse(&["restart"]).unwrap_err().contains(USAGE));
        assert!(parse(&["stop", "now"]).unwrap_err().contains(USAGE));
    }

    // The same hermetic shape as crates/hyprlayer-core/tests/headless_runtime.rs:
    // an unspawnable SHELL keeps the login-shell probe off the developer's rc files.
    fn environment(home: &Path) -> RuntimeEnvironment {
        let mut process_env = BTreeMap::new();
        process_env.insert(
            OsString::from("SHELL"),
            OsString::from(home.join("no-such-shell")),
        );
        RuntimeEnvironment {
            home_dir: home.to_path_buf(),
            process_cwd: home.to_path_buf(),
            process_env,
            env_overlay: Default::default(),
        }
    }

    fn paths(root: &Path) -> RuntimePaths {
        RuntimePaths {
            hyprlayer_config_file: root.join("config/hyprlayer.json"),
            app_config_dir: root.join("app-config"),
            app_data_dir: root.join("app-data"),
            default_worktree_root: root.join("worktrees"),
            default_repos_root: root.join("repos"),
            opencode_config_file: root.join("opencode/opencode.json"),
            opencode_auth_file: root.join("opencode/auth.json"),
            opencode_data_dir: root.join("opencode/data"),
            claude_config_dir: root.join("claude"),
            claude_managed_settings_files: Vec::new(),
            codex_home: root.join("codex"),
            agent_roots: Vec::new(),
            download_dir: root.join("downloads"),
        }
    }

    fn write_inputs(paths: &RuntimePaths) {
        for dir in [
            paths.hyprlayer_config_file.parent().unwrap(),
            &paths.app_config_dir,
            &paths.app_data_dir,
            paths.opencode_config_file.parent().unwrap(),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&paths.hyprlayer_config_file, r#"{"version": 4}"#).unwrap();
        std::fs::write(paths.app_config_dir.join("settings.json"), "{}").unwrap();
        std::fs::write(&paths.opencode_config_file, "{}").unwrap();
        std::fs::write(&paths.opencode_auth_file, "{}").unwrap();
    }

    async fn open_runtime_with(
        root: &Path,
        environment: RuntimeEnvironment,
    ) -> Arc<HyprlayerRuntime> {
        let paths = paths(root);
        write_inputs(&paths);
        let options = RuntimeOptions::new(environment, paths);
        let runtime = HyprlayerRuntime::open(options).await.expect("open");
        runtime.start().await.expect("start");
        runtime
    }

    async fn open_runtime(root: &Path) -> Arc<HyprlayerRuntime> {
        open_runtime_with(root, environment(&root.join("home"))).await
    }

    /// A stream on a terminal that prints nothing: its forwarder parks reading the ring.
    fn parked_stream(
        runtime: &Arc<HyprlayerRuntime>,
        out: &Outbound,
        streams: &StreamRegistry,
        stream_id: u64,
    ) -> (Arc<PtySession>, Option<tokio::task::AbortHandle>) {
        let pty = PtySession::spawn(SpawnTerminal {
            shell: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 60".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "ws".into(),
        })
        .expect("spawn pty");
        let task = streams::spawn_stream(
            runtime.clone(),
            stream_id,
            streams::StreamSource::Terminal(pty.subscribe()),
            out.clone(),
            streams.clone(),
            None,
        );
        (pty, task)
    }

    #[tokio::test]
    async fn an_aborted_connection_ends_the_streams_it_owned() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let out = Outbound::new(Features::ALL, false);
        let streams = StreamRegistry::default();
        let (pty, task) = parked_stream(&runtime, &out, &streams, 1);
        let task = task.expect("the stream started");
        // Parked on its source, where a closed queue alone never reaches it.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let writer = tokio::spawn(std::future::pending::<()>());
        drop(ConnectionGuard {
            out,
            writer: writer.abort_handle(),
            streams,
        });
        let ended = tokio::time::timeout(Duration::from_secs(2), async {
            while !task.is_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(ended.is_ok(), "the stream outlived its connection");
        pty.kill();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn a_cancel_that_beats_the_stream_open_still_cancels_it() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let out = Outbound::new(Features::ALL, false);
        let streams = StreamRegistry::default();
        streams.reserve(7);
        streams.abort(7);
        let (pty, task) = parked_stream(&runtime, &out, &streams, 7);
        assert!(
            task.is_none(),
            "a stream cancelled before it opened still ran"
        );
        pty.kill();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn a_strictly_newer_client_can_still_shut_down_a_refusing_server() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let server = SocketServer::bind(runtime.clone(), &root.path().join("app-data"))
            .await
            .expect("bind");
        let socket_path = server.socket_path().to_path_buf();
        let serving = tokio::spawn(server.run());

        let stream = socket::connect(&socket_path).await.expect("connect");
        let (rd, wr) = tokio::io::split(stream);
        let mut reader = FrameReader::new(rd);
        let mut writer = FrameWriter::new(wr);

        writer
            .write_control(&Control::Hello {
                protocol_version: PROTOCOL_VERSION + 1,
                build: None,
                version: None,
                features: Vec::new(),
                stream_window: None,
            })
            .await
            .unwrap();
        match reader.next_control().await.unwrap() {
            Some(Control::Hello { .. }) => {}
            other => panic!("expected the server's own hello first, got {other:?}"),
        }
        match reader.next_control().await.unwrap() {
            Some(Control::Refused { error }) => {
                assert_eq!(error.code, "protocol_version_mismatch");
            }
            other => panic!("expected a version refusal, got {other:?}"),
        }
        writer.write_control(&Control::Shutdown).await.unwrap();

        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .expect("the server did not stop for a strictly newer client's shutdown")
            .unwrap()
            .unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn an_older_clients_shutdown_after_refusal_is_ignored() {
        // Equal never reaches this branch at all — `check_version` only
        // refuses on inequality — so the reachable "not strictly newer" case
        // is an older client; its `Shutdown` in that state must be ignored,
        // or two mismatched clients could evict each other in a loop.
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let server = SocketServer::bind(runtime.clone(), &root.path().join("app-data"))
            .await
            .expect("bind");
        let socket_path = server.socket_path().to_path_buf();
        let stop = server.stop_handle();
        let serving = tokio::spawn(server.run());

        let stream = socket::connect(&socket_path).await.expect("connect");
        let (rd, wr) = tokio::io::split(stream);
        let mut reader = FrameReader::new(rd);
        let mut writer = FrameWriter::new(wr);

        writer
            .write_control(&Control::Hello {
                protocol_version: PROTOCOL_VERSION.saturating_sub(1),
                build: None,
                version: None,
                features: Vec::new(),
                stream_window: None,
            })
            .await
            .unwrap();
        let _ = reader.next_control().await; // the server's own hello
        let _ = reader.next_control().await; // the refusal
                                             // The server never reads a second frame from a client that is not
                                             // strictly newer, so the connection may already be closed on its
                                             // side — the point under test is that it's ignored either way, not
                                             // that this write itself succeeds.
        let _ = writer.write_control(&Control::Shutdown).await;

        // No shutdown landed: a fresh connection still gets a normal hello.
        let second = socket::connect(&socket_path).await.expect("connect again");
        let (rd2, wr2) = tokio::io::split(second);
        let mut reader2 = FrameReader::new(rd2);
        let mut writer2 = FrameWriter::new(wr2);
        writer2
            .write_control(&Control::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: None,
                version: None,
                features: Vec::new(),
                stream_window: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            reader2.next_control().await.unwrap(),
            Some(Control::Hello { .. })
        ));

        stop.stop();
        serving.await.unwrap().unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn opencode_credentials_and_default_model_land_in_the_servers_files() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let server = SocketServer::bind(runtime.clone(), &root.path().join("app-data"))
            .await
            .expect("bind");
        let socket_path = server.socket_path().to_path_buf();
        let stop = server.stop_handle();
        let serving = tokio::spawn(server.run());

        let client = RpcClient::connect_socket(&socket_path)
            .await
            .expect("connect");
        let server_paths = paths(root.path());
        let read_json = |path: &Path| -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };

        let listed = client
            .opencode_set_provider_key("anthropic".into(), "sk-ant-test".into())
            .await
            .expect("set a key over the socket");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].provider, "anthropic");
        assert_eq!(
            read_json(&server_paths.opencode_auth_file)["anthropic"]["key"],
            "sk-ant-test"
        );
        let relisted = client
            .opencode_auth_list()
            .await
            .expect("list over the socket");
        assert_eq!(relisted.len(), 1);
        assert_eq!(relisted[0].provider, "anthropic");

        let refused = client
            .opencode_set_provider_key("github-copilot".into(), "ghu-test".into())
            .await
            .unwrap_err();
        assert_eq!(refused.code, "opencode_provider_unsupported");
        assert_eq!(
            refused.message,
            "OpenCode provider `github-copilot` is no longer supported"
        );
        assert_eq!(
            refused.remedy.as_deref(),
            Some("Remove its saved credential from the provider list.")
        );

        client
            .set_opencode_default_model(Some("anthropic/claude-sonnet-4-5".into()))
            .await
            .expect("set the default model over the socket");
        assert_eq!(
            read_json(&server_paths.opencode_config_file)["model"],
            "anthropic/claude-sonnet-4-5"
        );
        assert_eq!(
            client
                .get_app_settings()
                .await
                .unwrap()
                .providers
                .opencode
                .default_model
                .as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );

        let remaining = client
            .opencode_remove_provider("anthropic".into())
            .await
            .expect("remove over the socket");
        assert!(remaining.is_empty());
        assert!(read_json(&server_paths.opencode_auth_file)
            .get("anthropic")
            .is_none());

        stop.stop();
        serving.await.unwrap().unwrap();
        runtime.shutdown().await;
    }
}
