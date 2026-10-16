//! Finds or spawns the profile's `hyprlayer-server` and reconciles a version
//! skew after an app update (ADR-0009, plan Phase 4 §4-§5). The desktop never
//! opens a runtime itself; this is the only place it starts one.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use hyprlayer_core::CoreError;
use hyprlayer_core::{Session, SessionState};
use hyprlayer_proto::{check_version, WireError, PROTOCOL_VERSION};
use hyprlayer_server::logging::{self as server_logging, RepeatGate};
use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};
use hyprlayer_server::{socket, APP_IDENTIFIER};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::task::JoinHandle;

use crate::client::{self, ConnectError, RpcClient};
use crate::error::AppError;
use crate::hosts::{self, Host, Target};
use crate::provision;
use crate::server_version::{self, Fit, MIN_SERVER_VERSION, SERVER_VERSION};
use crate::ssh::{self, Ended, Ssh, SshCommand};

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
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// For a host that accepts the connection and then stalls; the handshake alone
/// allows 60s. `setup()` blocks on this, so it is how long a launch can hang.
/// Time spent waiting on the user to sign in does not count.
const REMOTE_CONNECT_DEADLINE: Duration = Duration::from_secs(15);

/// The reconnect loop retries every 30s forever; one report per window.
static CONNECT_FAILURES: RepeatGate = RepeatGate::new(Duration::from_secs(600));

const CANNOT_START: &str = "Hyprlayer cannot start";
const ALREADY_RUNNING: &str = "Hyprlayer is already running";

/// The remedy for a mismatch this app update itself cannot resolve (plan
/// Phase 4 §5 — do not paraphrase, a prior draft's wording here was wrong).
const STOP_THE_SERVER_REMEDY: &str = "Stop the running Hyprlayer server (`hyprlayer-server stop`, \
     or `systemctl --user stop hyprlayer-server`) and reopen the app.";

fn step_aside_failed() -> Outcome {
    Outcome::Blocked {
        title: CANNOT_START.to_string(),
        error: AppError::server_unavailable_with(
            "The running Hyprlayer server wouldn't step aside for this app.",
            STOP_THE_SERVER_REMEDY,
        ),
    }
}

/// For a server that accepted the handshake and then dropped the first
/// request: same protocol version, different build.
pub fn incompatible_server_error() -> AppError {
    let remedy = if cfg!(debug_assertions) {
        "Rebuild it with `cargo build -p hyprlayer-server`, stop it with `hyprlayer-server stop`, and reopen the app."
    } else {
        STOP_THE_SERVER_REMEDY
    };
    AppError::server_unavailable_with(
        "The Hyprlayer server on this machine is a build this app can't use.",
        remedy,
    )
}

pub enum Outcome {
    /// `outdated`: the server is not the one this app ships, so a restart
    /// would replace it. Optional while it meets `MIN_SERVER_VERSION`.
    Connected {
        client: Arc<RpcClient>,
        outdated: bool,
    },
    /// Nothing left to retry. `setup()` opens the window anyway (plan Phase 1
    /// §5): `error` is what `AppState::failed` stores and `connection_status` reports.
    Blocked { title: String, error: AppError },
}

impl Outcome {
    fn connected(client: Arc<RpcClient>) -> Self {
        Self::Connected {
            client,
            outdated: false,
        }
    }

    /// The local-server path: every existing call site's message text moves
    /// into `AppError::server_unavailable` unchanged.
    fn blocked(title: &str, detail: impl Into<String>) -> Self {
        Self::Blocked {
            title: title.to_string(),
            error: AppError::server_unavailable(detail),
        }
    }

    /// The one `Blocked` arm that is not a connect failure: another
    /// Hyprlayer already holds this profile's lock. Built from
    /// `CoreError::RuntimeInUse`, not `server_unavailable`, so the recovery
    /// screen gets its code and remedy.
    fn already_running(app_data_dir: &Path) -> Self {
        Self::Blocked {
            title: ALREADY_RUNNING.to_string(),
            error: AppError::from(CoreError::RuntimeInUse {
                path: app_data_dir.to_path_buf(),
            }),
        }
    }
}

pub async fn connect(
    target: &Target,
    app_config_dir: &Path,
    app_data_dir: &Path,
    app_log_dir: &Path,
    ssh: &Ssh,
) -> Outcome {
    let started = Instant::now();
    let outcome = match target {
        Target::Own => match connect_or_spawn(app_config_dir, app_data_dir, app_log_dir).await {
            Outcome::Connected { client, .. }
                if server_version::fit(client.server_semver()) == Fit::TooOld =>
            {
                log::warn!(
                    "replacing the running server {}: this app needs {MIN_SERVER_VERSION}",
                    server_version::describe(client.server_semver())
                );
                replace_own_server(client, app_config_dir, app_data_dir, app_log_dir).await
            }
            Outcome::Connected { client, .. } => {
                let expected = provision::sha256_file(&server_binary());
                let fit = server_version::exact_fit(client.server_semver());
                let outdated = is_outdated(fit, &client, expected.as_deref());
                Outcome::Connected { client, outdated }
            }
            blocked => blocked,
        },
        Target::LocalProfile(dir) => connect_local_profile(dir, &target.name()).await,
        Target::Remote(host) => {
            // Dropping the attempt kills its ssh children (`kill_on_drop`).
            // A dead link's shared connection ends itself (`ServerAliveInterval`);
            // closing it here would also end a healthy one that a slow start timed out.
            ssh.within(
                REMOTE_CONNECT_DEADLINE,
                connect_remote(host, app_data_dir, ssh),
            )
            .await
            .unwrap_or_else(|| Outcome::Blocked {
                title: CANNOT_START.to_string(),
                error: AppError::remote_unreachable(
                    &host.label,
                    format!(
                        "it did not respond within {}s.",
                        REMOTE_CONNECT_DEADLINE.as_secs()
                    ),
                ),
            })
        }
    };
    log_outcome(target, started.elapsed(), &outcome);
    outcome
}

fn log_outcome(target: &Target, elapsed: Duration, outcome: &Outcome) {
    match outcome {
        Outcome::Connected { client, outdated } => log::info!(
            "connected to {} in {elapsed:?}: server {}, protocol {}, build {}{}",
            target.name(),
            server_version::describe(client.server_semver()),
            client.server_version(),
            client
                .server_build()
                .map_or("unknown", |build| &build[..build.len().min(12)]),
            if *outdated {
                ", not the server this app ships"
            } else {
                ""
            }
        ),
        Outcome::Blocked { title, error } => {
            let key = format!("{}:{}", error.code, error.message);
            let Some(held) = CONNECT_FAILURES.admit(&key) else {
                return;
            };
            let repeats = if held > 0 {
                format!(" ({held} more attempts failed the same way)")
            } else {
                String::new()
            };
            log::error!(
                "could not connect to {} ({title}): {}: {}{repeats}",
                target.name(),
                error.code,
                hyprlayer_core::redact::url_credentials(&error.message)
            );
        }
    }
}

/// Decided by version, so an app release that leaves the server alone asks
/// for no restart. Bytes are compared only in debug builds, where `cargo
/// build` changes the server without changing its version.
fn is_outdated(fit: Fit, client: &RpcClient, expected_build: Option<&str>) -> bool {
    match fit {
        Fit::TooOld | Fit::Older | Fit::OtherBuild => true,
        Fit::Current => {
            cfg!(debug_assertions)
                && expected_build.is_some_and(|expected| client.server_build() != Some(expected))
        }
    }
}

async fn connect_local_profile(data_dir: &Path, name: &str) -> Outcome {
    let mut error = match client::connect(data_dir).await {
        Ok(client) if server_version::fit(client.server_semver()) == Fit::TooOld => {
            AppError::server_unavailable(format!(
                "The Hyprlayer server for {name} is version {}; this app needs {MIN_SERVER_VERSION} or newer.",
                server_version::describe(client.server_semver())
            ))
        }
        // Not flagged outdated: this app does not restart another profile's server.
        Ok(client) => return Outcome::connected(client),
        Err(ConnectError::Refused(e)) => AppError::from(e),
        Err(e) => AppError::server_unavailable(format!(
            "Could not reach the Hyprlayer server for {name}: {e}"
        )),
    };
    if error.remedy.is_none() {
        error.remedy = Some(
            "This app does not start or replace another profile's server. Start it where it came \
             from, or choose Use this machine."
                .into(),
        );
    }
    Outcome::Blocked {
        title: CANNOT_START.to_string(),
        error,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalServer {
    pub data_dir: String,
    pub name: String,
    pub server_version: u32,
    /// Set when this app cannot talk to it.
    pub error: Option<AppError>,
}

/// Other profiles' servers listening on this machine. Only siblings named for
/// `APP_IDENTIFIER` are tried: macOS prompts before a process looks inside
/// some other apps' data directories.
pub async fn list_local_servers(app_data_dir: &Path) -> Vec<LocalServer> {
    let Some(entries) = app_data_dir
        .parent()
        .and_then(|root| std::fs::read_dir(root).ok())
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for dir in entries.flatten().map(|entry| entry.path()) {
        let in_family = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| {
                name == APP_IDENTIFIER
                    || name
                        .strip_prefix(APP_IDENTIFIER)
                        .is_some_and(|rest| rest.starts_with('.'))
            });
        if !in_family || dir == app_data_dir {
            continue;
        }
        let probe =
            tokio::time::timeout(PROBE_TIMEOUT, peer_version(&socket::socket_path(&dir))).await;
        if let Ok(Some(version)) = probe {
            found.push(LocalServer {
                data_dir: dir.display().to_string(),
                name: hosts::server_name(&dir),
                server_version: version,
                error: check_version(version).err().map(AppError::from),
            });
        }
    }
    found.sort_by(|a, b| a.data_dir.cmp(&b.data_dir));
    found
}

/// Raw frames rather than `RpcClient`, whose reader task keeps the socket open
/// after the client is dropped — a probe would leave an idle connection behind
/// on every server it looked at.
async fn peer_version(socket_path: &Path) -> Option<u32> {
    let (rd, wr) = tokio::io::split(socket::connect(socket_path).await.ok()?);
    FrameWriter::new(wr)
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: None,
            version: None,
        })
        .await
        .ok()?;
    match FrameReader::new(rd).next_control().await {
        Ok(Some(Control::Hello {
            protocol_version, ..
        })) => Some(protocol_version),
        _ => None,
    }
}

pub async fn connect_or_spawn(
    app_config_dir: &Path,
    app_data_dir: &Path,
    app_log_dir: &Path,
) -> Outcome {
    match client::connect(app_data_dir).await {
        Ok(client) => return Outcome::connected(client),
        Err(ConnectError::Refused(e)) if e.code == "protocol_version_mismatch" => {
            return if e.remedy.is_some() {
                // We are older than the running server — no self-upgrade path.
                Outcome::blocked(CANNOT_START, detail_for(&AppError::from(e)))
            } else {
                // We are newer: this is the normal path for every release
                // after Phase 4. Evict the old server, then start our own.
                log::warn!("replacing the running server: {}", e.message);
                evict_and_respawn(app_config_dir, app_data_dir, app_log_dir).await
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
            ) =>
        {
            log::info!("no server is listening ({e}); starting one");
        }
        Err(e) => {
            return Outcome::blocked(
                CANNOT_START,
                format!("Could not connect to the Hyprlayer server: {e}"),
            );
        }
    }

    spawn_and_connect(app_config_dir, app_data_dir, app_log_dir).await
}

/// Reaches a remote host through `ssh <destination> <version-keyed path> relay`,
/// joining the child's stdio as the connection (ADR-0009, plan Phase 5 §1,
/// Phase 7 §3). A failure here must never fall back to the local server — that
/// would silently connect the app to the wrong profile's data, "the single
/// most damaging mistake available in this plan" (plan, Migration Notes).
pub async fn connect_remote(host: &Host, app_data_dir: &Path, ssh: &Ssh) -> Outcome {
    match ssh_relay_connect(host, app_data_dir, false, ssh).await {
        Ok((client, relay, expected)) => {
            // The relay attaches to whatever daemon is already running there.
            let fit = server_version::exact_fit(client.server_semver());
            let outdated = is_outdated(fit, &client, expected.as_deref());
            // The relay must live as long as the connection does. It is
            // killed on drop, unlike the local server's deliberate orphan —
            // killing this SSH connection only ends the relay (plan §1), not
            // the persistent daemon on the far end.
            tokio::spawn(relay.watch(host_name(host)));
            Outcome::Connected { client, outdated }
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
/// The `ssh … relay` child and what it has written to stderr, which is where
/// both ssh and the remote relay say why a connection ended.
struct Relay {
    child: Child,
    stderr: Arc<Mutex<String>>,
    drained: JoinHandle<()>,
}

impl Relay {
    async fn watch(mut self, host: String) {
        let status = self.child.wait().await;
        let _ = tokio::time::timeout(DRAIN_TIMEOUT, self.drained).await;
        let said = self
            .stderr
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let status = match status {
            Ok(status) => status.to_string(),
            Err(e) => format!("unknown status: {e}"),
        };
        log::warn!(
            "the ssh relay to {host} exited ({status}): {}",
            hyprlayer_core::redact::tail(said.trim(), 2000)
        );
    }
}

fn host_name(host: &Host) -> String {
    Target::Remote(host.clone()).name()
}

/// `allow_too_old` keeps a server below `MIN_SERVER_VERSION`, for reading what runs on it.
async fn ssh_relay_connect(
    host: &Host,
    app_data_dir: &Path,
    allow_too_old: bool,
    ssh: &Ssh,
) -> Result<(Arc<RpcClient>, Relay, Option<String>), AppError> {
    // Never relay a binary this client has not confirmed is its own: the
    // version in the path cannot tell an unreleased build from the release it
    // descends from, and a stale one is only caught by the handshake, after
    // connecting (plan Phase 7 §3.2).
    let (provisioning, host_sha) = provision::inspect(host, app_data_dir, ssh).await?;
    let Some(version) = provisioning.usable_version.clone() else {
        let key = format!("{}:{:?}", host.destination, provisioning.state);
        if CONNECT_FAILURES.admit(&key).is_some() {
            log::warn!(
                "{} has no usable hyprlayer-server {}: {:?}; installed: [{}]",
                host_name(host),
                provisioning.version,
                provisioning.state,
                provisioning.installed_versions.join(", ")
            );
        }
        return Err(AppError::remote_cli_missing(&host.label));
    };
    // The host's hash is of this client's version directory, so it says
    // nothing about an older one the relay falls back to.
    let host_sha = host_sha.filter(|_| version == SERVER_VERSION);

    let SshCommand {
        mut cmd,
        mut sign_in,
        batch,
    } = ssh.command(host).await;
    cmd.arg(provision::relay_command(&version))
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
    let handshake = match ssh::unless_cancelled(&mut sign_in, RpcClient::connect(joined)).await {
        Ok(handshake) => handshake,
        Err(ended) => {
            let _ = child.start_kill();
            return Err(sign_in_ended(host, ended));
        }
    };
    match handshake {
        Ok(client)
            if !allow_too_old && server_version::fit(client.server_semver()) == Fit::TooOld =>
        {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
            Err(AppError::remote_server_outdated(
                &host.label,
                &server_version::describe(client.server_semver()),
                MIN_SERVER_VERSION,
            ))
        }
        Ok(client) => Ok((
            client,
            Relay {
                child,
                stderr: tail,
                drained,
            },
            host_sha,
        )),
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
        // Something answered, but not with a hello: output from the host's
        // shell startup files, or nothing within the handshake timeout.
        Err(ConnectError::Protocol(detail)) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
            let seen = tail.lock().unwrap_or_else(PoisonError::into_inner).clone();
            log::warn!(
                "the handshake with {} over ssh failed: {detail}; ssh said: {}",
                host_name(host),
                seen.trim()
            );
            Err(AppError::remote_command_failed(
                &host.label,
                format!(
                    "{detail}. Anything the host's shell startup files print to stdout \
                     breaks the connection."
                ),
            ))
        }
        Err(e) => {
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
            let seen = tail.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let _ = child.start_kill();
            let exit_code = child.wait().await.ok().and_then(|status| status.code());
            log::warn!(
                "the handshake with {} over ssh failed ({e}), ssh exit {exit_code:?}: {}",
                host_name(host),
                seen.trim()
            );
            Err(classify(host, RemoteCall::Server, exit_code, &seen, batch))
        }
    }
}

fn sign_in_ended(host: &Host, ended: Ended) -> AppError {
    AppError::remote_sign_in_cancelled(&host.label, ended == Ended::TimedOut)
}

/// A run that could not ask turns a login it could not finish into a request
/// to sign in; "too many authentication failures" is not one a prompt fixes.
fn classify(
    host: &Host,
    call: RemoteCall,
    exit_code: Option<i32>,
    stderr: &str,
    batch: bool,
) -> AppError {
    let error = classify_ssh_failure(host, call, exit_code, stderr);
    if batch
        && error.code == "remote_auth_failed"
        && !stderr.contains("Too many authentication failures")
    {
        return AppError::remote_sign_in_required(&host.label);
    }
    error
}

/// What was on the far end of the pipe. ssh reports its own failures as 255 and
/// otherwise hands back the remote command's status, so the same stderr means
/// opposite things either side of that line: `Permission denied` from ssh is a
/// credential, from a script we wrote it is a file mode.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RemoteCall {
    /// The server binary. A shell that cannot find it is the one failure
    /// provisioning can answer.
    Server,
    /// A script this client wrote; its stderr is that script's, never ssh's.
    Script,
}

/// A parse of another program's prose: `ssh`'s own stderr, in English, is the
/// only signal available for *why* it failed. The fallback arm keeps the raw
/// stderr, never a silent `internal` (plan Phase 1 §1).
fn classify_ssh_failure(
    host: &Host,
    call: RemoteCall,
    exit_code: Option<i32>,
    stderr: &str,
) -> AppError {
    let seen = stderr.trim();
    // 255, or no status at all, is ssh failing to deliver the command; only
    // then is the stderr ssh's own to read.
    if exit_code == Some(255) || exit_code.is_none() {
        // Never offered for trust: ssh refuses it outright, and so does this app.
        if let Some((key_host, known_hosts)) = changed_host_key(seen) {
            return AppError::remote_host_key_changed(
                &host.label,
                &key_host,
                known_hosts.as_deref(),
            );
        }
        // "Too many authentication failures" is what an agent holding several
        // keys hits before it ever reaches "Permission denied".
        if seen.contains("Permission denied")
            || seen.contains("Host key verification failed")
            || seen.contains("Too many authentication failures")
        {
            return AppError::remote_auth_failed(&host.label, seen);
        }
        let detail = if seen.is_empty() {
            "ssh closed the connection with no output.".to_string()
        } else {
            seen.to_string()
        };
        return AppError::remote_unreachable(&host.label, detail);
    }
    // The command ran. Only when it *was* the server does a missing path mean
    // the server is missing — a script hitting one is just a broken script,
    // and reporting that as `remote_cli_missing` would offer to install a
    // server that is already there.
    if call == RemoteCall::Server
        && (exit_code == Some(127)
            || seen.contains("command not found")
            || seen.contains("No such file or directory"))
    {
        return AppError::remote_cli_missing(&host.label);
    }
    let detail = if seen.is_empty() {
        match exit_code {
            Some(code) => format!("it exited {code} with no output."),
            None => "it exited with no output.".to_string(),
        }
    } else {
        seen.to_string()
    };
    AppError::remote_command_failed(&host.label, detail)
}

/// `ssh` names the host the way `known_hosts` does: `[127.0.0.1]:2222`, or
/// `nexus.local` once an alias resolves. OpenSSH 10 drops the key type that
/// older releases put first.
fn changed_host_key(stderr: &str) -> Option<(String, Option<String>)> {
    let line = stderr
        .lines()
        .find(|l| l.contains("has changed and you have requested strict checking"))?;
    let key_host = line
        .split_once("key for ")
        .and_then(|(_, rest)| rest.split_once(" has changed"))
        .map(|(host, _)| host.trim().to_string())?;
    // "Offending ED25519 key in /home/me/.ssh/known_hosts:12"
    let known_hosts = stderr.lines().find_map(|l| {
        let (_, file) = l.strip_prefix("Offending ")?.split_once(" key in ")?;
        file.trim()
            .rsplit_once(':')
            .map(|(path, _)| path.to_string())
    });
    Some((key_host, known_hosts))
}

/// Runs a short script on the host and returns its stdout. Separate from
/// `ssh_relay_connect`, which goes straight into relay mode and so cannot
/// carry a second command (plan Phase 7 §2).
pub(crate) async fn ssh_capture(host: &Host, script: &str, ssh: &Ssh) -> Result<String, AppError> {
    let output = ssh_run(host, script, Stdio::null(), ssh).await?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Streams a local file into a script's stdin on the host — the push half of
/// an install. `ssh` rather than `scp` so the transfer and the rename stay two
/// round trips: an interrupted push cannot reach the rename (plan Phase 7 §3).
pub(crate) async fn ssh_send_file(
    host: &Host,
    script: &str,
    file: &Path,
    ssh: &Ssh,
) -> Result<(), AppError> {
    let handle = std::fs::File::open(file)
        .map_err(|e| AppError::internal(format!("could not read {}: {e}", file.display())))?;
    ssh_run(host, script, Stdio::from(handle), ssh)
        .await
        .map(|_| ())
}

async fn ssh_run(host: &Host, script: &str, stdin: Stdio, ssh: &Ssh) -> Result<Vec<u8>, AppError> {
    let SshCommand {
        mut cmd,
        mut sign_in,
        batch,
    } = ssh.command(host).await;
    let child = cmd
        .arg(script)
        .kill_on_drop(true)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            AppError::remote_unreachable(
                &host.label,
                format!("Could not run ssh to reach {}: {e}", host.label),
            )
        })?;
    let output = ssh::unless_cancelled(&mut sign_in, child.wait_with_output())
        .await
        .map_err(|ended| sign_in_ended(host, ended))?
        .map_err(|e| {
            AppError::remote_unreachable(
                &host.label,
                format!("Could not run ssh to reach {}: {e}", host.label),
            )
        })?;
    if !output.status.success() {
        return Err(classify(
            host,
            RemoteCall::Script,
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
            batch,
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
    /// The server's release version; `None` from a server up to 0.8.2.
    pub server_release: Option<String>,
    pub error: Option<AppError>,
}

/// A one-shot connection test (plan Phase 1 §3, `test_host_connection`). It
/// cannot reuse `connect_remote`: that function deliberately detaches the ssh
/// child to live as long as the connection, which would leak one relay per
/// test press. This shares the same handshake through `ssh_relay_connect` but
/// keeps the child, so it can drop the client and kill the child
/// unconditionally on both the success and the failure path — the failure
/// path already kills it inside `ssh_relay_connect`.
pub async fn probe_remote(host: &Host, app_data_dir: &Path, ssh: &Ssh) -> HostProbe {
    match ssh_relay_connect(host, app_data_dir, false, ssh).await {
        Ok((client, mut relay, _)) => {
            let server_version = client.server_version();
            let server_release = client.server_semver().map(str::to_string);
            drop(client);
            let _ = relay.child.kill().await;
            HostProbe {
                ok: true,
                server_version: Some(server_version),
                server_release,
                error: None,
            }
        }
        Err(error) => HostProbe {
            ok: false,
            server_version: None,
            server_release: None,
            error: Some(error),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HostActivity {
    pub server_release: Option<String>,
    pub sessions: Vec<Session>,
    pub running_scripts: Vec<String>,
}

/// What runs on a host whose server this app refuses, so the frontend can
/// update it unattended once it is idle. Closes the relay like `probe_remote`.
pub async fn outdated_host_activity(
    host: &Host,
    app_data_dir: &Path,
    ssh: &Ssh,
) -> Result<HostActivity, AppError> {
    let read = async {
        let (client, mut relay, _) = ssh_relay_connect(host, app_data_dir, true, ssh).await?;
        let server_release = client.server_semver().map(str::to_string);
        let read = async {
            let active = client.list_active_sessions().await?;
            Ok::<_, WireError>((active, client.list_running_scripts().await?))
        }
        .await;
        drop(client);
        let _ = relay.child.kill().await;
        let (active, running_scripts) = read.map_err(AppError::from)?;
        let guess_working = !reports_working(server_release.as_deref());
        let sessions = active
            .into_iter()
            .map(|mut active| {
                if guess_working {
                    active.session.working = active.session.state == SessionState::Running;
                }
                active.session
            })
            .collect();
        Ok(HostActivity {
            server_release,
            sessions,
            running_scripts,
        })
    };
    tokio::time::timeout(REMOTE_CONNECT_DEADLINE, read)
        .await
        .unwrap_or_else(|_| {
            Err(AppError::remote_unreachable(
                &host.label,
                format!(
                    "it did not respond within {}s.",
                    REMOTE_CONNECT_DEADLINE.as_secs()
                ),
            ))
        })
}

/// Servers before 0.8.5 send no `working`, so their idle live harness is
/// indistinguishable from one mid-turn.
fn reports_working(semver: Option<&str>) -> bool {
    semver
        .and_then(server_version::parse)
        .is_some_and(|v| v >= semver::Version::new(0, 8, 5))
}

/// Message and remedy as one string, for a `server_unavailable` detail (which
/// carries no remedy of its own) and the launch log.
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

/// Where a spawned server's stderr file is, and where this spawn starts in it.
struct StderrFile {
    dir: PathBuf,
    start: u64,
}

impl StderrFile {
    fn written(&self) -> String {
        server_logging::stderr_since(&self.dir, self.start)
    }
}

/// Spawns the server as a deliberate orphan — the opposite of every other
/// child this codebase spawns (`process/mod.rs`'s `Exec`/`spawn_supervised`
/// both set `kill_on_drop(true)` on purpose; this one must outlive us).
/// Its stderr goes to a file, not a pipe: a pipe's reader dies with this app,
/// and the server outlives it.
fn spawn_detached(
    bin: &Path,
    app_config_dir: &Path,
    app_data_dir: &Path,
    app_log_dir: &Path,
) -> std::io::Result<(Child, Option<StderrFile>)> {
    let stderr = [app_log_dir, app_data_dir].into_iter().find_map(|dir| {
        match server_logging::stderr_file(dir) {
            Ok((file, start)) => Some((file, dir.to_path_buf(), start)),
            Err(e) => {
                log::warn!("cannot keep the server's stderr in {}: {e}", dir.display());
                None
            }
        }
    });
    let mut cmd = tokio::process::Command::new(bin);
    cmd.env("HYPRLAYER_APP_CONFIG_DIR", app_config_dir)
        .env("HYPRLAYER_APP_DATA_DIR", app_data_dir)
        .env(server_logging::LOG_DIR_ENV, app_log_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    let written = match stderr {
        Some((file, dir, start)) => {
            cmd.stderr(Stdio::from(file));
            Some(StderrFile { dir, start })
        }
        None => {
            cmd.stderr(Stdio::null());
            None
        }
    };
    let child = hyprlayer_core::process::spawn_orphan(&mut cmd)?;
    Ok((child, written))
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
    ExitedEarly(std::process::ExitStatus),
    /// Carries the last connect error.
    TimedOut(String),
}

async fn wait_for_spawn(child: &mut Child, app_data_dir: &Path) -> SpawnWait {
    let mut last_error = String::new();
    for _ in 0..SPAWN_LISTEN_RETRIES {
        if let Ok(Some(status)) = child.try_wait() {
            return SpawnWait::ExitedEarly(status);
        }
        match client::connect(app_data_dir).await {
            Ok(client) => return SpawnWait::Connected(client),
            Err(ConnectError::Refused(e)) => return SpawnWait::Refused(e),
            Err(e) => last_error = e.to_string(),
        }
        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
    }
    if let Ok(Some(status)) = child.try_wait() {
        return SpawnWait::ExitedEarly(status);
    }
    SpawnWait::TimedOut(last_error)
}

/// Spawns the server and waits for it to start listening, retrying the
/// *spawn* — not the connect — on a `runtime_in_use` exit: a pre-Phase-4
/// desktop may still hold the lock in-process with no socket yet to reach.
async fn spawn_and_connect(
    app_config_dir: &Path,
    app_data_dir: &Path,
    app_log_dir: &Path,
) -> Outcome {
    let bin = server_binary();
    #[cfg(debug_assertions)]
    warn_if_server_predates_sources(&bin);
    // Two seconds of slack for coarse filesystem timestamps.
    let spawned_at = SystemTime::now() - Duration::from_secs(2);
    for attempt in 0..SPAWN_RETRIES {
        let (mut child, stderr) =
            match spawn_detached(&bin, app_config_dir, app_data_dir, app_log_dir) {
                Ok(spawned) => spawned,
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
        log::info!("started {} as pid {:?}", bin.display(), child.id());

        match wait_for_spawn(&mut child, app_data_dir).await {
            SpawnWait::Connected(client) => return Outcome::connected(client),
            SpawnWait::Refused(e) => {
                log::warn!(
                    "{} is not the build this app expects: {}",
                    bin.display(),
                    e.message
                );
                let remedy = if cfg!(debug_assertions) {
                    "Rebuild it with `cargo build -p hyprlayer-server` and reopen the app."
                } else {
                    "Reinstall Hyprlayer and reopen the app."
                };
                return Outcome::Blocked {
                    title: CANNOT_START.to_string(),
                    error: AppError::server_unavailable_with(
                        "The server this app started isn't the build it expects.",
                        remedy,
                    ),
                };
            }
            SpawnWait::ExitedEarly(status) => {
                let seen = stderr.as_ref().map(StderrFile::written).unwrap_or_default();
                log::warn!(
                    "the server exited during startup ({status}): {}",
                    seen.trim()
                );
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
            SpawnWait::TimedOut(last_error) => {
                log::error!(
                    "the server (pid {:?}) did not start listening in time; last connect \
                     error: {last_error}",
                    child.id()
                );
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
            build: None,
            version: None,
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

/// Same protocol, below `MIN_SERVER_VERSION`: this app cannot use it, so it
/// is replaced the way a protocol mismatch is, ending its sessions.
async fn replace_own_server(
    client: Arc<RpcClient>,
    app_config_dir: &Path,
    app_data_dir: &Path,
    app_log_dir: &Path,
) -> Outcome {
    let socket_path = socket::socket_path(app_data_dir);
    if let Err(e) = client.request_shutdown().await {
        log::warn!("the running server would not step aside: {}", e.message);
        return step_aside_failed();
    }
    drop(client);
    if !wait_for_socket_gone(&socket_path, CONNECT_RETRIES, CONNECT_RETRY_DELAY).await {
        log::error!("the old server did not stop");
        return Outcome::blocked(CANNOT_START, STOP_THE_SERVER_REMEDY);
    }
    spawn_and_connect(app_config_dir, app_data_dir, app_log_dir).await
}

/// The normal path for every release from here on: shut down the older
/// server, wait for its lock to free, spawn our own matching binary. Ends
/// every live session on the profile — quitting is free, updating is not.
async fn evict_and_respawn(
    app_config_dir: &Path,
    app_data_dir: &Path,
    app_log_dir: &Path,
) -> Outcome {
    let socket_path = socket::socket_path(app_data_dir);
    log::warn!("asking the running server to stop; its sessions and terminals end");
    if let Err(e) = request_shutdown_of_refusing_server(&socket_path).await {
        log::warn!("the running server would not step aside: {e}");
        return step_aside_failed();
    }

    if !wait_for_socket_gone(&socket_path, CONNECT_RETRIES, CONNECT_RETRY_DELAY).await {
        log::error!("the old server did not stop");
        return Outcome::blocked(CANNOT_START, STOP_THE_SERVER_REMEDY);
    }

    spawn_and_connect(app_config_dir, app_data_dir, app_log_dir).await
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
    fn a_remote_file_mode_is_not_read_as_an_ssh_credential() {
        // The install script's own mkdir, not ssh's auth: reporting this as
        // `remote_auth_failed` sent the user to their SSH keys for a chmod.
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Script,
            Some(1),
            "mkdir: cannot create directory '/home/jt/.hyprlayer/server': Permission denied",
        );
        assert_eq!(err.code, "remote_command_failed");
        assert!(err.message.contains("mkdir: cannot create directory"));
    }

    #[test]
    fn a_script_that_cannot_find_a_path_is_not_a_missing_server() {
        // `remote_cli_missing` is what offers to install a server, so a script
        // tripping over a path must never raise it — the server is already
        // there, and the offer would loop.
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Script,
            Some(1),
            "cat: /home/jt/.hyprlayer/server/0.7.5.tmp/hyprlayer-server: No such file or directory",
        );
        assert_eq!(err.code, "remote_command_failed");
    }

    #[test]
    fn ssh_itself_still_owns_the_255_stderr_whatever_ran() {
        // The same words, from ssh rather than the remote command.
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Script,
            Some(255),
            "jt@nexus: Permission denied (publickey).",
        );
        assert_eq!(err.code, "remote_auth_failed");
    }

    #[test]
    fn a_remote_failure_with_no_output_still_names_its_status() {
        let err = classify_ssh_failure(&host(), RemoteCall::Script, Some(3), "   ");
        assert_eq!(err.code, "remote_command_failed");
        assert!(err.message.contains("exited 3"));
    }

    #[test]
    fn an_exit_127_or_a_missing_command_is_cli_missing() {
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Server,
            Some(127),
            "bash: hyprlayer-server: command not found",
        );
        assert_eq!(err.code, "remote_cli_missing");
    }

    #[test]
    fn a_permission_denied_stderr_is_auth_failed() {
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Server,
            Some(255),
            "jt@nexus: Permission denied (publickey,password).",
        );
        assert_eq!(err.code, "remote_auth_failed");
    }

    #[test]
    fn a_host_key_verification_failure_is_auth_failed() {
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Server,
            Some(255),
            "Host key verification failed.",
        );
        assert_eq!(err.code, "remote_auth_failed");
    }

    #[test]
    fn too_many_authentication_failures_is_auth_failed_not_unreachable() {
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Server,
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
            RemoteCall::Server,
            Some(255),
            "ssh: Could not resolve hostname nexus: nodename nor servname provided, or not known",
        );
        assert_eq!(err.code, "remote_unreachable");
    }

    #[test]
    fn a_connection_refused_is_unreachable() {
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Server,
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
        // 255 because this is ssh's own diagnostic; it reports its own failures
        // with that status and the remote command's status otherwise.
        let err = classify_ssh_failure(
            &host(),
            RemoteCall::Server,
            Some(255),
            "kex_exchange_identification: read: broken pipe",
        );
        assert_eq!(err.code, "remote_unreachable");
        assert!(err
            .message
            .contains("kex_exchange_identification: read: broken pipe"));
    }

    #[test]
    fn a_killed_relay_with_no_status_is_still_read_as_ssh_failing() {
        // `probe_remote` kills the child, so the status is gone; that is ssh
        // not delivering the command, not a remote program's exit.
        let err = classify_ssh_failure(&host(), RemoteCall::Server, None, "");
        assert_eq!(err.code, "remote_unreachable");
    }
}
