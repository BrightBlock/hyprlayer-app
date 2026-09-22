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

/// Built from the current tree once per test process: `tauri dev` never
/// rebuilds the server, so a stale binary is the easy mistake.
fn binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
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
    })
}
