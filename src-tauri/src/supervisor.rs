//! Finds or spawns the profile's `hyprlayer-server` and reconciles a version
//! skew after an app update (ADR-0009, plan Phase 4 §4-§5). The desktop never
//! opens a runtime itself; this is the only place it starts one.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use hyprlayer_core::CoreError;
use hyprlayer_proto::{WireError, PROTOCOL_VERSION};
use hyprlayer_server::socket;
use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::task::JoinHandle;

use crate::client::{self, ConnectError, RpcClient};
use crate::error::AppError;
use crate::hosts::Host;
use crate::provision;

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
    /// Nothing left to retry. `setup()` opens the window anyway on every arm
    /// except `ALREADY_RUNNING` (plan Phase 1 §5) — `error` is what
    /// `AppState::failed` stores and `connection_status` reports.
    Blocked {
        title: String,
        error: AppError,
    },
}

impl Outcome {
    /// The local-server path: every existing call site's message text moves
    /// into `AppError::server_unavailable` unchanged.
    fn blocked(title: &str, detail: impl Into<String>) -> Self {
        Self::Blocked {
            title: title.to_string(),
            error: AppError::server_unavailable(detail),
        }
    }

    /// The one `Blocked` arm that is not a connect failure: another
    /// Hyprlayer already holds this profile's lock. `setup()` matches on
    /// `error.code == "runtime_in_use"` to keep this one exit (plan Phase 1
    /// §5) — building it any other way makes that match silently not fire.
    fn already_running(app_data_dir: &Path) -> Self {
        Self::Blocked {
            title: ALREADY_RUNNING.to_string(),
            error: AppError::from(CoreError::RuntimeInUse {
                path: app_data_dir.to_path_buf(),
            }),
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

/// Reaches a remote host through `ssh <destination> <version-keyed path> relay`,
/// joining the child's stdio as the connection (ADR-0009, plan Phase 5 §1,
/// Phase 7 §3). A failure here must never fall back to the local server — that
/// would silently connect the app to the wrong profile's data, "the single
/// most damaging mistake available in this plan" (plan, Migration Notes).
pub async fn connect_remote(host: &Host, app_data_dir: &Path) -> Outcome {
    match ssh_relay_connect(host, app_data_dir).await {
        Ok((client, mut child)) => {
            // The relay must live as long as the connection does. It is
            // killed on drop, unlike the local server's deliberate orphan —
            // killing this SSH connection only ends the relay (plan §1), not
            // the persistent daemon on the far end.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Outcome::Connected(client)
        }
        Err(error) => Outcome::Blocked {
            title: CANNOT_START.to_string(),
            error,
        },
    }
}

/// Spawns the relay at the version-keyed path this client installed
/// (`provision::relay_command`) and performs the RPC handshake. The caller
/// owns the returned child's lifetime — `connect_remote` detaches it to live
/// as long as the connection; `probe_remote` kills it the moment the probe is
/// done, since nothing here should outlive a test press.
async fn ssh_relay_connect(
    host: &Host,
    app_data_dir: &Path,
) -> Result<(Arc<RpcClient>, Child), AppError> {
    // Never relay a binary this client has not confirmed is its own: the
    // version in the path cannot tell an unreleased build from the release it
    // descends from, and a stale one is only caught by the handshake, after
    // connecting (plan Phase 7 §3.2).
    if provision::readiness(host, app_data_dir).await?.state != provision::HostServerState::Ready {
        return Err(AppError::remote_cli_missing(&host.label));
    }

    let mut cmd = tokio::process::Command::new("ssh");
    hide_console(&mut cmd);
    cmd.arg(&host.destination)
        .arg(provision::relay_command())
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        AppError::remote_unreachable(
            &host.label,
            format!("Could not run ssh to reach {}: {e}", host.label),
        )
    })?;
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (tail, drained) = drain_stderr(stderr);

    let joined = tokio::io::join(stdout, stdin);
    match RpcClient::connect(joined).await {
        Ok(client) => Ok((client, child)),
        // A version skew is not an ssh failure to classify — the peer
        // answered, just with a refusal. Its own remedy already names the
        // fix, so it is preserved verbatim rather than folded into the
        // remote taxonomy (plan Phase 1 §1: `remote_protocol_mismatch` is
        // deliberately not added).
        Err(ConnectError::Refused(e)) => {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
            let _ = child.kill().await;
            let mut error = AppError::from(e);
            // `WireError::version_mismatch` leaves the newer side's remedy
            // `None` because the local case just evicts and respawns its own
            // server. There is no such move for someone else's machine (plan
            // Phase 1 §8), so the remote path names the actual fix.
            if error.code == "protocol_version_mismatch" && error.remedy.is_none() {
                error.remedy = Some(format!("Update hyprlayer-server on {}.", host.label));
            }
            Err(error)
        }
        Err(_) => {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
            let seen = tail.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let _ = child.start_kill();
            let exit_code = child.wait().await.ok().and_then(|status| status.code());
            Err(classify_ssh_failure(host, exit_code, &seen))
        }
    }
}

/// A parse of another program's prose: `ssh`'s own stderr, in English, is the
/// only signal available for *why* it failed. The fallback arm is
/// `remote_unreachable` with the raw stderr appended, never a silent
/// `internal` (plan Phase 1 §1).
fn classify_ssh_failure(host: &Host, exit_code: Option<i32>, stderr: &str) -> AppError {
    let seen = stderr.trim();
    if exit_code == Some(127)
        || seen.contains("command not found")
        || seen.contains("No such file or directory")
    {
        return AppError::remote_cli_missing(&host.label);
    }
    // "Too many authentication failures" is what an agent holding several keys
    // hits before it ever reaches "Permission denied".
    if seen.contains("Permission denied")
        || seen.contains("Host key verification failed")
        || seen.contains("Too many authentication failures")
    {
        return AppError::remote_auth_failed(&host.label, seen);
    }
    let unreachable = seen.contains("Could not resolve hostname")
        || seen.contains("Connection refused")
        || seen.contains("Connection timed out");
    if unreachable || seen.is_empty() {
        let detail = if seen.is_empty() {
            "ssh closed the connection with no output.".to_string()
        } else {
            seen.to_string()
        };
        return AppError::remote_unreachable(&host.label, detail);
    }
    AppError::remote_unreachable(&host.label, seen)
}

/// Runs a short script on the host and returns its stdout. Separate from
/// `ssh_relay_connect`, which goes straight into relay mode and so cannot
/// carry a second command (plan Phase 7 §2).
pub(crate) async fn ssh_capture(host: &Host, script: &str) -> Result<String, AppError> {
    let output = ssh_run(host, script, Stdio::null()).await?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Streams a local file into a script's stdin on the host — the push half of
/// an install. `ssh` rather than `scp` so the transfer and the rename stay two
/// round trips: an interrupted push cannot reach the rename (plan Phase 7 §3).
pub(crate) async fn ssh_send_file(host: &Host, script: &str, file: &Path) -> Result<(), AppError> {
    let handle = std::fs::File::open(file)
        .map_err(|e| AppError::internal(format!("could not read {}: {e}", file.display())))?;
    ssh_run(host, script, Stdio::from(handle)).await.map(|_| ())
}

async fn ssh_run(host: &Host, script: &str, stdin: Stdio) -> Result<Vec<u8>, AppError> {
    let mut cmd = tokio::process::Command::new("ssh");
    hide_console(&mut cmd);
    let output = cmd
        .arg(&host.destination)
        .arg(script)
        .kill_on_drop(true)
        .stdin(stdin)
        .output()
        .await
        .map_err(|e| {
            AppError::remote_unreachable(
                &host.label,
                format!("Could not run ssh to reach {}: {e}", host.label),
            )
        })?;
    if !output.status.success() {
        return Err(classify_ssh_failure(
            host,
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    Ok(output.stdout)
}

/// `uname -sm` onto one of ADR-0008's supported triples (plan Phase 7 §2).
/// `Darwin x86_64` is deliberately unsupported — no build target covers it.
pub(crate) fn triple_for_uname(host: &Host, uname: &str) -> Result<&'static str, AppError> {
    match uname.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["Linux", "x86_64"] => Ok("x86_64-unknown-linux-gnu"),
        ["Darwin", "arm64"] => Ok("aarch64-apple-darwin"),
        _ => Err(AppError::remote_platform_unsupported(
            &host.label,
            uname.trim(),
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HostProbe {
    pub ok: bool,
    pub server_version: Option<u32>,
    pub error: Option<AppError>,
}

/// A one-shot connection test (plan Phase 1 §3, `test_host_connection`). It
/// cannot reuse `connect_remote`: that function deliberately detaches the ssh
/// child to live as long as the connection, which would leak one relay per
/// test press. This shares the same handshake through `ssh_relay_connect` but
/// keeps the child, so it can drop the client and kill the child
/// unconditionally on both the success and the failure path — the failure
/// path already kills it inside `ssh_relay_connect`.
pub async fn probe_remote(host: &Host, app_data_dir: &Path) -> HostProbe {
    match ssh_relay_connect(host, app_data_dir).await {
        Ok((client, mut child)) => {
            let server_version = client.server_version();
            drop(client);
            let _ = child.kill().await;
            HostProbe {
                ok: true,
                server_version: Some(server_version),
                error: None,
            }
        }
        Err(error) => HostProbe {
            ok: false,
            server_version: None,
            error: Some(error),
        },
    }
}

/// The one dialog that still shows the full message-plus-remedy text as one
/// string — the `runtime_in_use` exit (plan Phase 1 §5).
pub(crate) fn detail_for(err: &AppError) -> String {
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
/// Every other spawn in the tree sets this (`core::process`, `claude::probe`,
/// `claude::driver`); the shell's own `ssh`/`curl` calls did not, and on Windows
/// a GUI process spawning a console program without it flashes a console window
/// each time — several times per connect, since provisioning probes, uploads
/// and installs.
pub(crate) fn hide_console(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

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
                    return Outcome::already_running(app_data_dir);
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
    Outcome::already_running(app_data_dir)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            destination: "nexus".into(),
            label: "nexus".into(),
        }
    }

    #[test]
    fn an_exit_127_or_a_missing_command_is_cli_missing() {
        let err = classify_ssh_failure(
            &host(),
            Some(127),
            "bash: hyprlayer-server: command not found",
        );
        assert_eq!(err.code, "remote_cli_missing");
    }

    #[test]
    fn a_permission_denied_stderr_is_auth_failed() {
        let err = classify_ssh_failure(
            &host(),
            Some(255),
            "jt@nexus: Permission denied (publickey,password).",
        );
        assert_eq!(err.code, "remote_auth_failed");
    }

    #[test]
    fn a_host_key_verification_failure_is_auth_failed() {
        let err = classify_ssh_failure(&host(), Some(255), "Host key verification failed.");
        assert_eq!(err.code, "remote_auth_failed");
    }

    #[test]
    fn too_many_authentication_failures_is_auth_failed_not_unreachable() {
        let err = classify_ssh_failure(
            &host(),
            Some(255),
            "Received disconnect from ::1 port 22:2: Too many authentication failures\n\
             Disconnected from ::1 port 22",
        );
        assert_eq!(err.code, "remote_auth_failed");
    }

    #[test]
    fn an_unresolvable_hostname_is_unreachable() {
        let err = classify_ssh_failure(
            &host(),
            Some(255),
            "ssh: Could not resolve hostname nexus: nodename nor servname provided, or not known",
        );
        assert_eq!(err.code, "remote_unreachable");
    }

    #[test]
    fn a_connection_refused_is_unreachable() {
        let err = classify_ssh_failure(
            &host(),
            Some(255),
            "ssh: connect to host nexus port 22: Connection refused",
        );
        assert_eq!(err.code, "remote_unreachable");
    }

    #[test]
    fn uname_maps_onto_the_supported_triples() {
        assert_eq!(
            triple_for_uname(&host(), "Linux x86_64\n").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            triple_for_uname(&host(), "Darwin arm64\n").unwrap(),
            "aarch64-apple-darwin"
        );
    }

    #[test]
    fn an_unsupported_uname_names_the_platform_it_saw() {
        // Intel macOS is not a build target, so it is unsupported rather than
        // a near miss to fall back from.
        let err = triple_for_uname(&host(), "Darwin x86_64\n").unwrap_err();
        assert_eq!(err.code, "remote_platform_unsupported");
        assert!(err.message.contains("Darwin x86_64"));

        let err = triple_for_uname(&host(), "SunOS i86pc").unwrap_err();
        assert_eq!(err.code, "remote_platform_unsupported");
    }

    #[test]
    fn an_unrecognised_stderr_falls_back_to_unreachable_with_the_raw_text() {
        let err = classify_ssh_failure(
            &host(),
            Some(1),
            "kex_exchange_identification: read: broken pipe",
        );
        assert_eq!(err.code, "remote_unreachable");
        assert!(err
            .message
            .contains("kex_exchange_identification: read: broken pipe"));
    }
}
