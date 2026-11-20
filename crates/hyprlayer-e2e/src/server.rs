use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use hyprlayer_server::client::RpcClient;
use tokio::process::{Child, Command};

use crate::sandbox::Sandbox;

/// A cold start opens SQLite, runs migrations and probes every binary.
const STARTUP: Duration = Duration::from_secs(60);
const SHUTDOWN: Duration = Duration::from_secs(30);

/// A `hyprlayer-server serve` process on a sandbox's profile, killed on drop.
pub struct Server {
    child: Child,
    socket: PathBuf,
    stderr: PathBuf,
    /// The connection that proved the server is listening. Kept, because a
    /// dropped `RpcClient` leaves its connection open until the server closes it.
    client: Option<Arc<RpcClient>>,
}

/// How a server ended.
pub struct Stopped {
    pub status: ExitStatus,
    pub still_listening: bool,
}

impl Server {
    pub async fn start(sandbox: &Sandbox) -> Self {
        let stderr = sandbox.logs.join("server-stderr.log");
        let child = Command::new(binary())
            .arg("serve")
            .env_clear()
            .envs(sandbox.server_env())
            .current_dir(&sandbox.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&stderr).expect("create the stderr log"))
            .kill_on_drop(true)
            .spawn()
            .expect("spawn hyprlayer-server");
        let mut server = Self {
            child,
            socket: sandbox.socket(),
            stderr,
            client: None,
        };
        server.wait_until_listening().await;
        server
    }

    async fn wait_until_listening(&mut self) {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("poll the server") {
                panic!(
                    "hyprlayer-server exited before listening ({status}): {}",
                    std::fs::read_to_string(&self.stderr).unwrap_or_default()
                );
            }
            if let Ok(client) = RpcClient::connect_socket(&self.socket).await {
                self.client = Some(client);
                return;
            }
            assert!(
                started.elapsed() < STARTUP,
                "hyprlayer-server did not listen on {} within {STARTUP:?}",
                self.socket.display()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn pid(&self) -> u32 {
        self.child
            .id()
            .expect("a listening server has not been reaped")
    }

    /// The connection the server was first reached on.
    pub fn client(&self) -> Arc<RpcClient> {
        self.client
            .clone()
            .expect("a listening server has a client")
    }

    /// An additional, independent connection.
    pub async fn connect(&self) -> Arc<RpcClient> {
        RpcClient::connect_socket(&self.socket)
            .await
            .expect("connect to hyprlayer-server")
    }

    /// How the server ended, if it exits on its own within `within`.
    pub async fn exited(&mut self, within: Duration) -> Option<ExitStatus> {
        tokio::time::timeout(within, self.child.wait())
            .await
            .ok()
            .map(|status| status.expect("wait for hyprlayer-server"))
    }

    /// Stops the server the way `hyprlayer-server stop` does.
    pub async fn shutdown(mut self, client: &RpcClient) -> Stopped {
        client.request_shutdown().await.expect("request a shutdown");
        let status = tokio::time::timeout(SHUTDOWN, self.child.wait())
            .await
            .expect("hyprlayer-server exits after a shutdown request")
            .expect("wait for hyprlayer-server");
        let still_listening = RpcClient::connect_socket(&self.socket).await.is_ok();
        Stopped {
            status,
            still_listening,
        }
    }
}

/// The binary every `Server` runs; agents reach it as `HYPRLAYER_CTL`.
pub fn server_binary() -> &'static Path {
    binary()
}

/// `HYPRLAYER_E2E_SERVER_BIN` when set (prebuilt, so parallel tests don't queue on
/// cargo's lock); else built from the current tree, as `tauri dev` never rebuilds it.
fn binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        if let Some(path) = std::env::var_os("HYPRLAYER_E2E_SERVER_BIN").map(PathBuf::from) {
            assert!(
                path.is_file(),
                "HYPRLAYER_E2E_SERVER_BIN={} is missing; run \
                 `node scripts/build-server-sidecar.mjs --debug` or unset it",
                path.display()
            );
            return path;
        }
        match std::env::var("NEXTEST_RUN_ID") {
            Ok(run) => run_copy(&run),
            Err(_) => build(),
        }
    })
}

/// `target/<profile>/hyprlayer-server`, built from the current tree.
fn build() -> PathBuf {
    let exe = std::env::current_exe().expect("locate the test executable");
    let profile = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps/<test>");
    let mut cargo = std::process::Command::new(env!("CARGO"));
    cargo.args([
        "build",
        "--quiet",
        "--package",
        "hyprlayer-server",
        "--bin",
        "hyprlayer-server",
    ]);
    if profile.file_name() == Some(OsStr::new("release")) {
        cargo.arg("--release");
    }
    let status = cargo.status().expect("run cargo build");
    assert!(status.success(), "building hyprlayer-server failed");
    profile.join(format!("hyprlayer-server{}", std::env::consts::EXE_SUFFIX))
}

/// One build per nextest run, copied to a path no build writes: every cargo build,
/// even a no-op, re-links `target/<profile>/hyprlayer-server` while sibling tests spawn it.
fn run_copy(run: &str) -> PathBuf {
    let dir = crate::journey::target_dir().join("e2e").join("server");
    std::fs::create_dir_all(&dir).expect("create the server copy dir");
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".lock"))
        .expect("open the server copy lock");
    lock.lock().expect("lock the server copy");
    let copy = dir.join(format!("hyprlayer-server{}", std::env::consts::EXE_SUFFIX));
    let marker = dir.join("run");
    if std::fs::read_to_string(&marker).ok().as_deref() != Some(run) {
        let tmp = dir.join(".hyprlayer-server.tmp");
        std::fs::copy(build(), &tmp).expect("copy the server");
        // Renamed over, so a previous run's process still spawning it never finds it missing.
        std::fs::rename(&tmp, &copy).expect("publish the server copy");
        std::fs::write(&marker, run).expect("record the run");
    }
    copy
}
