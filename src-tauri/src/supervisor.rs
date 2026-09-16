//! Finds or spawns the profile's `hyprlayer-server` and reconciles a version
//! skew after an app update (ADR-0009, plan Phase 4 §4-§5). The desktop never
//! opens a runtime itself; this is the only place it starts one.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use hyprlayer_proto::{WireError, PROTOCOL_VERSION};
use hyprlayer_server::socket;
use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::task::JoinHandle;

use crate::client::{self, ConnectError, RpcClient};
use crate::error::AppError;

const CONNECT_RETRIES: u32 = 10;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);
/// A server we just spawned gets far longer than a reconnect does: cold start
/// opens SQLite and runs migrations, which on a debug build or a large profile
/// takes many seconds. At 10 retries this timed out and the app died with a
/// dialog, leaving the started server orphaned and holding the profile lock,
/// so the next launch failed differently.
const SPAWN_LISTEN_RETRIES: u32 = 300;
const SPAWN_RETRIES: u32 = 5;
const SPAWN_RETRY_DELAY: Duration = Duration::from_secs(1);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

const CANNOT_START: &str = "Hyprlayer cannot start";
const ALREADY_RUNNING: &str = "Hyprlayer is already running";

/// The remedy for a mismatch this app update itself cannot resolve (plan
/// Phase 4 §5 — do not paraphrase, a prior draft's wording here was wrong).
const STOP_THE_SERVER_REMEDY: &str = "Stop the running Hyprlayer server (`hyprlayer-server stop`, \
     or `systemctl --user stop hyprlayer-server`) and reopen the app.";

/// `tauri dev` never rebuilds the sibling server binary, so a dev build names
/// the fix; a release build ships a matching pair and needs no such hint.
fn dev_rebuild_hint() -> &'static str {
    if cfg!(debug_assertions) {
        " In a dev build, rebuild it first: `cargo build -p hyprlayer-server`."
    } else {
        ""
    }
}

/// For a server that accepted the handshake and then dropped the first
/// request: same protocol version, different build.
pub fn incompatible_server_detail() -> String {
    format!(
        "The Hyprlayer server closed the connection as soon as the app spoke to it, \
         which usually means it is running a build this app cannot talk to.\n\n{STOP_THE_SERVER_REMEDY}{}",
        dev_rebuild_hint()
    )
}

pub enum Outcome {
    Connected(Arc<RpcClient>),
    /// Nothing left to retry — show this and exit.
    Blocked {
        title: String,
        detail: String,
    },
}

impl Outcome {
    fn blocked(title: &str, detail: impl Into<String>) -> Self {
        Self::Blocked {
            title: title.to_string(),
            detail: detail.into(),
        }
    }
}

pub async fn connect_or_spawn(app_config_dir: &Path, app_data_dir: &Path) -> Outcome {
    match client::connect(app_data_dir).await {
        Ok(client) => return Outcome::Connected(client),
        Err(ConnectError::Refused(e)) if e.code == "protocol_version_mismatch" => {
            return if e.remedy.is_some() {
                // We are older than the running server — no self-upgrade path.
                Outcome::blocked(CANNOT_START, detail_for(&AppError::from(e)))
            } else {
                // We are newer: this is the normal path for every release
                // after Phase 4. Evict the old server, then start our own.
                evict_and_respawn(app_config_dir, app_data_dir).await
            };
        }
        Err(ConnectError::Refused(e)) => {
            return Outcome::blocked(CANNOT_START, detail_for(&AppError::from(e)));
        }
        // ENOENT / ECONNREFUSED — no server listening. Fall through to spawn.
        // Any other IO error (permissions, a non-server listener already on
        // the path) is not something spawning again can fix.
        Err(ConnectError::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => {
            return Outcome::blocked(
                CANNOT_START,
                format!("Could not connect to the Hyprlayer server: {e}"),
            );
        }
    }

    spawn_and_connect(app_config_dir, app_data_dir).await
}

/// Reaches a remote host through `ssh <destination> hyprlayer-server relay`,
/// joining the child's stdio as the connection (ADR-0009, plan Phase 5 §1). A
/// failure here must never fall back to the local server — that would
/// silently connect the app to the wrong profile's data, "the single most
/// damaging mistake available in this plan" (plan, Migration Notes).
pub async fn connect_remote(host: &crate::hosts::Host) -> Outcome {
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.arg(&host.destination)
        .arg("hyprlayer-server")
        .arg("relay")
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Outcome::blocked(
                CANNOT_START,
                format!("Could not run ssh to reach {}: {e}", host.label),
            );
        }
    };
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (tail, drained) = drain_stderr(stderr);

    let joined = tokio::io::join(stdout, stdin);
    match RpcClient::connect(joined).await {
        Ok(client) => {
            // The relay must live as long as the connection does. It is
            // killed on drop, unlike the local server's deliberate orphan —
            // killing this SSH connection only ends the relay (plan §1), not
            // the persistent daemon on the far end.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Outcome::Connected(client)
        }
        Err(e) => {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
            let seen = tail.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let _ = child.kill().await;
            // A refusal carries a remedy the message alone drops — a version
            // skew's is the only instruction that resolves it, and the remote
            // path must show it as the local one does.
            let reason = match e {
                ConnectError::Refused(e) => detail_for(&AppError::from(e)),
                e => e.to_string(),
            };
            let detail = if seen.trim().is_empty() {
                format!(
                    "Could not reach the Hyprlayer server on {}: {reason}",
                    host.label
                )
            } else {
                format!(
                    "Could not reach the Hyprlayer server on {}: {reason}\n\n{seen}",
                    host.label
                )
            };
            Outcome::blocked(CANNOT_START, detail)
        }
    }
}

fn detail_for(err: &AppError) -> String {
    match &err.remedy {
        Some(remedy) => format!("{}\n\n{remedy}", err.message),
        None => err.message.clone(),
    }
}

/// The report a fatal `open` leaves behind (`hyprlayer_server::serve` writes
/// it, and only a successful open deletes it). Read after this launch's own
/// spawn failed, and only if written since that spawn: reading it *before*
/// spawning wedged every later launch on a stale report, because the server
/// that would have cleared it was never started.
fn read_runtime_fatal(app_data_dir: &Path, since: SystemTime) -> Option<AppError> {
    let path = app_data_dir.join(".runtime-fatal.json");
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if modified < since {
        return None;
    }
    let wire: WireError = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(AppError::from(wire))
}

/// Resolves the sibling `hyprlayer-server` binary next to this executable.
/// Every install shape puts it there: cargo/`tauri dev` target dirs, the
/// bundled sidecar, `/usr/bin` from the packages, and the dev-install scripts.
fn server_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "hyprlayer-server.exe"
    } else {
        "hyprlayer-server"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Spawns the server as a deliberate orphan — the opposite of every other
/// child this codebase spawns (`process/mod.rs`'s `Exec`/`spawn_supervised`
/// both set `kill_on_drop(true)` on purpose; this one must outlive us).
fn spawn_detached(
    bin: &Path,
    app_config_dir: &Path,
    app_data_dir: &Path,
) -> std::io::Result<Child> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.env("HYPRLAYER_APP_CONFIG_DIR", app_config_dir)
        .env("HYPRLAYER_APP_DATA_DIR", app_data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    hyprlayer_core::process::spawn_orphan(&mut cmd)
}

/// Drains a spawned child's stderr into a shared buffer, both to recognise a
/// `runtime_in_use` exit and so a *surviving* server never blocks writing to
/// a full, unread pipe once this task is its only reader. The buffer is only
/// filled at EOF, so a caller that saw the child exit must await the handle
/// before reading it.
fn drain_stderr(mut stderr: tokio::process::ChildStderr) -> (Arc<Mutex<String>>, JoinHandle<()>) {
    let tail = Arc::new(Mutex::new(String::new()));
    let sink = tail.clone();
    let drained = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        *sink.lock().unwrap_or_else(PoisonError::into_inner) =
            String::from_utf8_lossy(&buf).into_owned();
    });
    (tail, drained)
}

enum SpawnWait {
    Connected(Arc<RpcClient>),
    /// The binary we just started speaks another protocol version.
    Refused(WireError),
    ExitedEarly,
    TimedOut,
}

async fn wait_for_spawn(child: &mut Child, app_data_dir: &Path) -> SpawnWait {
    for _ in 0..SPAWN_LISTEN_RETRIES {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return SpawnWait::ExitedEarly;
        }
        match client::connect(app_data_dir).await {
            Ok(client) => return SpawnWait::Connected(client),
            Err(ConnectError::Refused(e)) => return SpawnWait::Refused(e),
            Err(_) => {}
        }
        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
    }
    if matches!(child.try_wait(), Ok(Some(_))) {
        return SpawnWait::ExitedEarly;
    }
    SpawnWait::TimedOut
}

/// Spawns the server and waits for it to start listening, retrying the
/// *spawn* — not the connect — on a `runtime_in_use` exit: a pre-Phase-4
/// desktop may still hold the lock in-process with no socket yet to reach.
async fn spawn_and_connect(app_config_dir: &Path, app_data_dir: &Path) -> Outcome {
    let bin = server_binary();
    #[cfg(debug_assertions)]
    warn_if_server_predates_sources(&bin);
    // Two seconds of slack for coarse filesystem timestamps.
    let spawned_at = SystemTime::now() - Duration::from_secs(2);
    for attempt in 0..SPAWN_RETRIES {
        let mut child = match spawn_detached(&bin, app_config_dir, app_data_dir) {
            Ok(child) => child,
            Err(e) => {
                return Outcome::blocked(
                    CANNOT_START,
                    format!(
                        "Could not start the Hyprlayer server ({}): {e}",
                        bin.display()
                    ),
                );
            }
        };
        let stderr = child.stderr.take().expect("stderr is piped");
        let (tail, drained) = drain_stderr(stderr);

        match wait_for_spawn(&mut child, app_data_dir).await {
            SpawnWait::Connected(client) => return Outcome::Connected(client),
            SpawnWait::Refused(e) => {
                return Outcome::blocked(
                    CANNOT_START,
                    format!(
                        "The Hyprlayer server this app started ({}) is not the build it expects: {}\n\n\
                         Reinstall Hyprlayer so the app and its server match, then reopen the app.{}",
                        bin.display(),
                        e.message,
                        dev_rebuild_hint()
                    ),
                );
            }
            SpawnWait::ExitedEarly => {
                // The exit is observed before the drain task has necessarily
                // stored what the child wrote; an empty read here would miss
                // `runtime_in_use` and skip the spawn retry.
                let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
                let seen = tail.lock().unwrap_or_else(PoisonError::into_inner).clone();
                if seen.contains("runtime_in_use") {
                    if attempt + 1 < SPAWN_RETRIES {
                        tokio::time::sleep(SPAWN_RETRY_DELAY).await;
                        continue;
                    }
                    return Outcome::blocked(
                        ALREADY_RUNNING,
                        "Another Hyprlayer window or process is already using this profile. \
                         Quit it, then relaunch.",
                    );
                }
                if let Some(fatal) = read_runtime_fatal(app_data_dir, spawned_at) {
                    return Outcome::blocked(CANNOT_START, detail_for(&fatal));
                }
                return Outcome::blocked(
                    CANNOT_START,
                    format!("The Hyprlayer server exited unexpectedly.\n\n{seen}"),
                );
            }
            SpawnWait::TimedOut => {
                return Outcome::blocked(
                    CANNOT_START,
                    "The Hyprlayer server did not start listening in time.",
                );
            }
        }
    }
    Outcome::blocked(
        ALREADY_RUNNING,
        "Another Hyprlayer window or process is already using this profile. Quit it, then relaunch.",
    )
}

/// Dev builds spawn the sibling binary whatever its vintage, so say so when it
/// is older than the newest source it is built from — the usual cause of a dead
/// client after startup. Compared against sources, not the app binary: `tauri
/// dev` rebuilds the app far more often than the server, so a current server
/// "predates" the app on almost every launch.
#[cfg(debug_assertions)]
fn warn_if_server_predates_sources(bin: &Path) {
    let Some(built) = std::fs::metadata(bin).and_then(|m| m.modified()).ok() else {
        return;
    };
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates");
    let newest = [
        "hyprlayer-server",
        "hyprlayer-core",
        "hyprlayer-proto",
        "hyprlayer-models",
    ]
    .iter()
    .filter_map(|name| newest_mtime(&crates.join(name).join("src")))
    .max();
    if matches!(newest, Some(source) if source > built) {
        log::warn!(
            "{} is older than its sources; run `cargo build -p hyprlayer-server` and \
             `hyprlayer-server stop`, then relaunch",
            bin.display()
        );
    }
}

#[cfg(debug_assertions)]
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let candidate = if path.is_dir() {
            newest_mtime(&path)
        } else {
            entry.metadata().and_then(|m| m.modified()).ok()
        };
        if candidate > newest {
            newest = candidate;
        }
    }
    newest
}

/// Sends our hello, takes the peer's, then `Shutdown` — the one frame a
/// refused connection still accepts, and only from a strictly newer peer
/// (`serve_connection`'s post-`Refused` read).
async fn request_shutdown_of_refusing_server(socket_path: &Path) -> std::io::Result<()> {
    let stream = socket::connect(socket_path).await?;
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
        })
        .await?;
    let _ = reader.next_control().await?;
    writer.write_control(&Control::Shutdown).await
}

async fn wait_for_socket_gone(socket_path: &Path, attempts: u32, delay: Duration) -> bool {
    for _ in 0..attempts {
        if socket::connect(socket_path).await.is_err() {
            return true;
        }
        tokio::time::sleep(delay).await;
    }
    false
}

/// The normal path for every release from here on: shut down the older
/// server, wait for its lock to free, spawn our own matching binary. Ends
/// every live session on the profile — quitting is free, updating is not.
async fn evict_and_respawn(app_config_dir: &Path, app_data_dir: &Path) -> Outcome {
    let socket_path = socket::socket_path(app_data_dir);
    if let Err(e) = request_shutdown_of_refusing_server(&socket_path).await {
        return Outcome::blocked(
            CANNOT_START,
            format!("Could not ask the running Hyprlayer server to step aside: {e}\n\n{STOP_THE_SERVER_REMEDY}"),
        );
    }

    if !wait_for_socket_gone(&socket_path, CONNECT_RETRIES, CONNECT_RETRY_DELAY).await {
        return Outcome::blocked(CANNOT_START, STOP_THE_SERVER_REMEDY);
    }

    spawn_and_connect(app_config_dir, app_data_dir).await
}
