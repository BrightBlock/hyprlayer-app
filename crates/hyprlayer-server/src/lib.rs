//! `hyprlayer-server`: one `HyprlayerRuntime` behind a local socket (ADR-0009).
//! The library is the server; `main.rs` only parses a subcommand.

pub mod client;
pub mod dispatch;
pub mod events;
pub mod logging;
mod relay;
pub mod rpc;
pub mod socket;
pub mod streams;
pub mod transport;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hyprlayer_core::{
    CoreError, HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths,
};
use hyprlayer_proto::frame::Frame;
use hyprlayer_proto::{check_version, Outcome, WireError, PROTOCOL_VERSION};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, Notify};
use tokio::task::{JoinHandle, JoinSet};

use client::{ConnectError, RpcClient};
use dispatch::Server;
use events::Clients;
use logging::RepeatGate;
use rpc::Dispatch;
use streams::StreamRegistry;
use transport::{
    encode_control, label, parse_control, reject_undecodable, Control, FrameReader, FrameWriter,
    OUTBOUND_QUEUE_FRAMES,
};

/// The bundle identifier Tauri derives the desktop's app dirs from; a
/// standalone server must land on the same profile (ADR-0009).
pub const APP_IDENTIFIER: &str = "ai.brightblock.hyprlayer";

const USAGE: &str = "usage: hyprlayer-server [serve|relay|status|stop|logs|--version]";
const STOP_POLLS: u32 = 100;
/// How much of each log file `logs` prints.
const LOGS_TAIL_BYTES: u64 = 64 * 1024;
const SLOW_REQUEST: Duration = Duration::from_secs(5);

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);
static REQUEST_FAILURES: RepeatGate = RepeatGate::new(Duration::from_secs(60));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Serve,
    Relay,
    Status,
    Stop,
    /// Prints the tails of this profile's log files: how a client fetches a
    /// remote host's logs over `ssh`, with or without a running server.
    Logs,
    /// The client provisions a version-keyed server on a remote host and needs
    /// to read back what it installed (plan Phase 7 §3).
    Version,
}

impl Command {
    /// `serve` is the default; the error is the usage text to print.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let command = match args.next().as_deref() {
            None | Some("serve") => Command::Serve,
            Some("relay") => Command::Relay,
            Some("status") => Command::Status,
            Some("stop") => Command::Stop,
            Some("logs") => Command::Logs,
            Some("--version" | "-V") => Command::Version,
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
    match command {
        Command::Serve => serve().await,
        Command::Relay => relay().await,
        Command::Status => status().await,
        Command::Stop => stop().await,
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
    Some(format!("{:x}", Sha256::digest(std::fs::read(path).ok()?)))
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

    let runtime = match HyprlayerRuntime::open(RuntimeOptions::new(environment, paths)).await {
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
    if let Err(e) = runtime.start().await {
        tracing::error!("runtime failed to start: {e}");
    }

    let server = match SocketServer::bind(runtime.clone(), &app_data_dir).await {
        Ok(server) => server,
        Err(e) => {
            let socket = socket::socket_path(&app_data_dir);
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
    if let Err(e) = server.run().await {
        tracing::error!("server loop failed: {e}");
    }
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
            server: Arc::new(Server::new(runtime)),
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
        Ok(Some(requests)) => tracing::info!(
            "conn {conn}: closed after {:?}, {requests} request(s)",
            started.elapsed()
        ),
        Ok(None) => {}
        Err(e) => tracing::info!("conn {conn}: dropped after {:?}: {e}", started.elapsed()),
    }
}

/// `Ok(None)` when the peer never got past the handshake.
async fn serve_connection<S>(
    conn: u64,
    stream: S,
    peer: &str,
    server: Arc<Server>,
    clients: Clients,
    stop: Arc<Notify>,
) -> io::Result<Option<u64>>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);

    match handshake(conn, &mut reader, &mut writer, &stop).await {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(e) => {
            tracing::debug!("conn {conn}: closed during the handshake: {e}");
            return Ok(None);
        }
    }
    tracing::info!("conn {conn}: client connected ({peer})");

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE_FRAMES);
    clients.register(conn, tx.clone());
    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if let Err(e) = writer.write_bytes(&bytes).await {
                tracing::debug!("conn {conn}: write failed: {e}");
                break;
            }
        }
    });

    let streams = StreamRegistry::default();
    let result = read_requests(conn, &mut reader, &server, &tx, &stop, &streams).await;
    // A dropped connection cancels every stream it owned.
    streams.abort_all();
    clients.unregister(conn);
    writer_task.abort();
    result.map(Some)
}

/// `Ok(true)` once the peer's hello is accepted.
async fn handshake<R, W>(
    conn: u64,
    reader: &mut FrameReader<R>,
    writer: &mut FrameWriter<W>,
    stop: &Notify,
) -> io::Result<bool>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: BUILD.get().cloned(),
            version: Some(VERSION.to_string()),
        })
        .await?;
    match reader.next_control().await? {
        Some(Control::Hello {
            protocol_version, ..
        }) => {
            let Err(error) = check_version(protocol_version) else {
                return Ok(true);
            };
            tracing::warn!(
                "conn {conn}: refused a client speaking protocol {protocol_version}; \
                 this server speaks {PROTOCOL_VERSION}"
            );
            writer.write_control(&Control::Refused { error }).await?;
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
            Ok(false)
        }
        Some(other) => {
            tracing::warn!("conn {conn}: expected hello, got {}", label(&other));
            let error = WireError::protocol(format!("expected hello, got {}", label(&other)));
            writer.write_control(&Control::Refused { error }).await?;
            Ok(false)
        }
        None => Ok(false),
    }
}

/// Returns how many requests the connection made.
async fn read_requests<R: AsyncRead + Unpin>(
    conn: u64,
    reader: &mut FrameReader<R>,
    server: &Arc<Server>,
    tx: &mpsc::Sender<Vec<u8>>,
    stop: &Arc<Notify>,
    streams: &StreamRegistry,
) -> io::Result<u64> {
    let mut requests = 0u64;
    while let Some(frame) = reader.next_frame().await? {
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
            Err(e) => match reject_undecodable(&line) {
                Some((command, reply)) => {
                    tracing::warn!("conn {conn}: refused {command}: this build cannot decode it");
                    let bytes = encode_control(&reply).expect("an error outcome always serializes");
                    let _ = tx.send(bytes).await;
                    continue;
                }
                None => return Err(e),
            },
        };
        match control {
            Control::Request { id, body } => {
                requests += 1;
                let server = server.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let command = body.command();
                    let started = Instant::now();
                    // Its own task, so a panicking handler still gets an answer
                    // back to the caller instead of leaving it waiting.
                    let outcome: Outcome<_> =
                        match tokio::spawn(async move { server.dispatch(body).await }).await {
                            Ok(result) => result.into(),
                            Err(e) if e.is_panic() => {
                                tracing::error!("conn {conn}: {command} panicked: {e}");
                                Outcome::Err(crashed(command))
                            }
                            Err(_) => return,
                        };
                    log_outcome(conn, command, started.elapsed(), &outcome);
                    let bytes =
                        encode_control(&Control::Response { id, outcome }).unwrap_or_else(|e| {
                            tracing::error!("{command}: result is not serializable: {e}");
                            let outcome = Outcome::Err(WireError::internal(format!(
                                "{command}: result is not serializable: {e}"
                            )));
                            encode_control(&Control::Response { id, outcome })
                                .expect("an error outcome always serializes")
                        });
                    if tx.send(bytes).await.is_err() {
                        tracing::debug!("conn {conn}: {command} finished after the client left");
                    }
                });
            }
            Control::StreamRequest { id, body } => {
                requests += 1;
                let server = server.clone();
                let tx = tx.clone();
                let streams = streams.clone();
                tokio::spawn(async move {
                    let command = body.command();
                    let started = Instant::now();
                    let preparing = server.clone();
                    let prepared = match tokio::spawn(async move {
                        streams::prepare_stream(&preparing, body).await
                    })
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
                            // The response must reach the wire before a byte of
                            // the stream it names does, or a fast source could
                            // race a data frame ahead of the id that explains it.
                            if tx.send(bytes).await.is_err() {
                                return;
                            }
                            if let (Some(stream_id), Some(source)) = (stream_id, source) {
                                streams::spawn_stream(
                                    server.runtime(),
                                    stream_id,
                                    source,
                                    tx,
                                    streams,
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
                            let _ = tx.send(bytes).await;
                        }
                    }
                });
            }
            Control::Cancel { stream_id } => {
                tracing::debug!("conn {conn}: cancelled stream {stream_id}");
                streams.abort(stream_id)
            }
            Control::Shutdown => {
                tracing::info!("conn {conn}: asked the server to stop");
                stop.notify_one();
                return Ok(requests);
            }
            other => tracing::warn!(
                "conn {conn}: unexpected {} frame from client",
                label(&other)
            ),
        }
    }
    Ok(requests)
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

    use base64::Engine as _;
    use hyprlayer_core::terminal::TerminalEvent;

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
        assert_eq!(parse(&["stop"]), Ok(Command::Stop));
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

    /// Builds and resolves the sibling `hyprlayer-server` binary from this
    /// test binary's own path: `target/<profile>/deps/hyprlayer_server-<hash>`
    /// for a lib unit test, two parents up to `target/<profile>/`.
    /// `CARGO_BIN_EXE_...` is only set for integration tests and benchmarks,
    /// and `cargo test` builds a package's binaries only for those, so the
    /// binary is built here — cargo has released its build lock by the time
    /// tests run, and an up-to-date build is a fingerprint check.
    fn server_binary() -> PathBuf {
        let exe = std::env::current_exe().expect("current test exe");
        let profile_dir = exe
            .parent()
            .and_then(Path::parent)
            .expect("target/<profile>/deps -> target/<profile>");
        let mut build = std::process::Command::new(env!("CARGO"));
        build.args(["build", "-q", "-p", "hyprlayer-server"]);
        if profile_dir.file_name().and_then(|n| n.to_str()) == Some("release") {
            build.arg("--release");
        }
        let status = build
            .status()
            .expect("run cargo build for the server binary");
        assert!(status.success(), "cargo build -p hyprlayer-server failed");
        profile_dir.join("hyprlayer-server")
    }

    #[tokio::test]
    async fn a_second_server_process_on_the_same_profile_exits_with_runtime_in_use() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let runtime = open_runtime_with(root.path(), environment(&home)).await;

        let bin = server_binary();
        let output = tokio::process::Command::new(&bin)
            .env("HOME", &home)
            .env("HYPRLAYER_APP_CONFIG_DIR", root.path().join("app-config"))
            .env("HYPRLAYER_APP_DATA_DIR", root.path().join("app-data"))
            .kill_on_drop(true)
            .output()
            .await
            .expect("spawn a second hyprlayer-server");

        assert!(
            !output.status.success(),
            "a second server on the same profile must not start"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("runtime_in_use"),
            "expected runtime_in_use on stderr, got: {stderr}"
        );

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
    async fn an_attach_supplied_env_overlay_reaches_a_spawned_child() {
        let root = tempfile::tempdir().unwrap();
        // No SSH_AUTH_SOCK anywhere in the runtime's own environment.
        let runtime = open_runtime(root.path()).await;
        let server = SocketServer::bind(runtime.clone(), &root.path().join("app-data"))
            .await
            .expect("bind");
        let socket_path = server.socket_path().to_path_buf();
        let stop = server.stop_handle();
        let serving = tokio::spawn(server.run());

        // A real socket: the overlay now drops an SSH_AUTH_SOCK that names nothing here.
        let agent_sock = root.path().join("a.sock");
        let _agent = std::os::unix::net::UnixListener::bind(&agent_sock).expect("bind agent.sock");

        let client = RpcClient::connect_socket(&socket_path)
            .await
            .expect("connect");
        client
            .set_env_overlay(BTreeMap::from([(
                "SSH_AUTH_SOCK".to_string(),
                agent_sock.display().to_string(),
            )]))
            .await
            .expect("set the env overlay");

        let (_, _, mut events) = client
            .spawn_shell_terminal_stream("printf \"$SSH_AUTH_SOCK\"".into(), 80, 24)
            .await
            .expect("open a shell terminal");

        let mut seen = String::new();
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = events.recv().await {
                if let TerminalEvent::Output { data } = event {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .unwrap_or_default();
                    seen.push_str(&String::from_utf8_lossy(&decoded));
                    if seen.contains(&agent_sock.display().to_string()) {
                        return true;
                    }
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(
            found,
            "expected the overlay's SSH_AUTH_SOCK in the child's environment, got {seen:?}"
        );

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
