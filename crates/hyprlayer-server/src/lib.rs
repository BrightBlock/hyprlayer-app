//! `hyprlayer-server`: one `HyprlayerRuntime` behind a local socket (ADR-0009).
//! The library is the server; `main.rs` only parses a subcommand.

pub mod client;
pub mod dispatch;
pub mod events;
mod relay;
pub mod rpc;
pub mod socket;
pub mod streams;
pub mod transport;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
use rpc::Dispatch;
use streams::StreamRegistry;
use transport::{
    encode_control, label, parse_control, Control, FrameReader, FrameWriter, OUTBOUND_QUEUE_FRAMES,
};

/// The bundle identifier Tauri derives the desktop's app dirs from; a
/// standalone server must land on the same profile (ADR-0009).
pub const APP_IDENTIFIER: &str = "ai.brightblock.hyprlayer";

const USAGE: &str = "usage: hyprlayer-server [serve|relay|status|stop|--version]";
const STOP_POLLS: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Serve,
    Relay,
    Status,
    Stop,
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
        Command::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
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
                eprintln!("could not write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("could not encode the fatal-error report: {e}"),
    }
}

/// Set once by `serve`, before the socket binds, so a binary replaced on disk
/// afterwards cannot change what this process reports.
static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn executable_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    Some(format!("{:x}", Sha256::digest(std::fs::read(path).ok()?)))
}

async fn serve() -> i32 {
    if let Some(build) = std::env::current_exe()
        .ok()
        .and_then(|exe| executable_sha256(&exe))
    {
        let _ = BUILD.set(build);
    }
    let environment = RuntimeEnvironment::detect();
    let (config_dir, data_dir) = app_dirs();
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
            eprintln!("{}: {e}", e.code());
            if fatal {
                write_runtime_fatal(&fatal_path, dispatch::wire_error(e));
            }
            return 1;
        }
    };
    let _ = std::fs::remove_file(&fatal_path);
    if let Err(e) = runtime.start().await {
        eprintln!("runtime failed to start: {e}");
    }

    let server = match SocketServer::bind(runtime.clone(), &app_data_dir).await {
        Ok(server) => server,
        Err(e) => {
            eprintln!(
                "cannot bind {}: {e}",
                socket::socket_path(&app_data_dir).display()
            );
            runtime.shutdown().await;
            return 1;
        }
    };
    eprintln!(
        "hyprlayer-server listening on {}",
        server.socket_path().display()
    );

    let stop = server.stop_handle();
    tokio::spawn(async move {
        wait_for_signal().await;
        stop.stop();
    });
    if let Err(e) = server.run().await {
        eprintln!("server loop failed: {e}");
    }
    runtime.shutdown().await;
    0
}

/// The desktop's remote transport: `ssh <host> hyprlayer-server relay`, with
/// this process's stdin/stdout as the connection (ADR-0009, plan Phase 5 §1).
async fn relay() -> i32 {
    let (config_dir, data_dir) = app_dirs();
    let io = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    match relay::run(config_dir, data_dir, io).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("relay failed: {e}");
            1
        }
    }
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(e) => {
                    eprintln!("cannot listen for SIGTERM: {e}");
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
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
                        connections.spawn(handle_connection(
                            stream,
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
        connections.shutdown().await;
        self.forwarder.abort();
        Ok(())
    }
}

async fn handle_connection<S>(stream: S, server: Arc<Server>, clients: Clients, stop: Arc<Notify>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    if let Err(e) = serve_connection(stream, server, clients, stop).await {
        tracing::debug!("connection ended: {e}");
    }
}

async fn serve_connection<S>(
    stream: S,
    server: Arc<Server>,
    clients: Clients,
    stop: Arc<Notify>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);

    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: BUILD.get().cloned(),
        })
        .await?;
    match reader.next_control().await? {
        Some(Control::Hello {
            protocol_version, ..
        }) => {
            if let Err(error) = check_version(protocol_version) {
                writer.write_control(&Control::Refused { error }).await?;
                // A version mismatch normally ends the connection here. The one
                // exception (Phase 4 §5): a client strictly newer than us is
                // mid-upgrade and gets one more read, to ask us to step aside —
                // an equal-or-older client's `Shutdown` here is ignored, or two
                // mismatched clients could evict each other in a loop.
                if protocol_version > PROTOCOL_VERSION {
                    if let Ok(Some(Control::Shutdown)) = reader.next_control().await {
                        stop.notify_one();
                    }
                }
                return Ok(());
            }
        }
        Some(other) => {
            let error = WireError::protocol(format!("expected hello, got {}", label(&other)));
            writer.write_control(&Control::Refused { error }).await?;
            return Ok(());
        }
        None => return Ok(()),
    }

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE_FRAMES);
    let client_id = clients.register(tx.clone());
    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if writer.write_bytes(&bytes).await.is_err() {
                break;
            }
        }
    });

    let streams = StreamRegistry::default();
    let result = read_requests(&mut reader, &server, &tx, &stop, &streams).await;
    // A dropped connection cancels every stream it owned.
    streams.abort_all();
    clients.unregister(client_id);
    writer_task.abort();
    result
}

async fn read_requests<R: AsyncRead + Unpin>(
    reader: &mut FrameReader<R>,
    server: &Arc<Server>,
    tx: &mpsc::Sender<Vec<u8>>,
    stop: &Arc<Notify>,
    streams: &StreamRegistry,
) -> io::Result<()> {
    while let Some(frame) = reader.next_frame().await? {
        let line = match frame {
            Frame::Control(line) => line,
            Frame::Data(_) => {
                tracing::debug!("data frame ignored: the data plane only carries stream events");
                continue;
            }
        };
        match parse_control(&line)? {
            Control::Request { id, body } => {
                let server = server.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let command = body.command();
                    let outcome: Outcome<_> = server.dispatch(body).await.into();
                    let bytes =
                        encode_control(&Control::Response { id, outcome }).unwrap_or_else(|e| {
                            tracing::error!("{command}: result is not serializable: {e}");
                            let outcome = Outcome::Err(WireError::internal(format!(
                                "{command}: result is not serializable: {e}"
                            )));
                            encode_control(&Control::Response { id, outcome })
                                .expect("an error outcome always serializes")
                        });
                    let _ = tx.send(bytes).await;
                });
            }
            Control::StreamRequest { id, body } => {
                let server = server.clone();
                let tx = tx.clone();
                let streams = streams.clone();
                tokio::spawn(async move {
                    match streams::prepare_stream(&server, body).await {
                        Ok((response, source)) => {
                            let stream_id = response.stream_id();
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
            Control::Cancel { stream_id } => streams.abort(stream_id),
            Control::Shutdown => {
                stop.notify_one();
                return Ok(());
            }
            other => tracing::warn!("unexpected {} frame from client", label(&other)),
        }
    }
    Ok(())
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

        let client = RpcClient::connect_socket(&socket_path)
            .await
            .expect("connect");
        client
            .set_env_overlay(BTreeMap::from([(
                "SSH_AUTH_SOCK".to_string(),
                "/tmp/probe".to_string(),
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
                    if seen.contains("/tmp/probe") {
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
}
