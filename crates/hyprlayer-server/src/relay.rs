//! `hyprlayer-server relay`: the server-side half of the SSH transport
//! (ADR-0009, plan Phase 5 §1). Connects to the local socket, spawning `serve`
//! detached first if nothing is listening, then splices the connection to its
//! caller's I/O byte for byte — framing-blind, so the SSH child on the far
//! end needs no protocol knowledge of its own.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};

use crate::{logging, socket};

/// As long as the desktop's own spawn waits (`supervisor.rs`): a cold start
/// runs migrations, and a debug build hashes its own binary first.
const SPAWN_LISTEN_RETRIES: u32 = 300;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Connects to the profile's socket and splices `io` to it until either side
/// closes. `io` is the process's own stdin/stdout in production; a test
/// substitutes an in-memory pipe.
pub async fn run<S>(
    app_config_dir: PathBuf,
    app_data_dir: PathBuf,
    log_dir: &Path,
    io: S,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let path = socket::socket_path(&app_data_dir);
    let stream = match socket::connect(&path).await {
        Ok(stream) => {
            tracing::info!("relaying to the server at {}", path.display());
            stream
        }
        Err(e) => {
            tracing::info!("no server at {} ({e}); starting one", path.display());
            let (mut child, stderr_start) =
                spawn_detached(&app_config_dir, &app_data_dir, log_dir)?;
            let said = || {
                stderr_start
                    .map(|start| logging::stderr_since(log_dir, start))
                    .unwrap_or_default()
            };
            match wait_for_spawn(&mut child, &path, said).await {
                Ok(stream) => stream,
                // What the server said on its way out is the reason; the
                // wait only says it never listened.
                Err(failure) => {
                    return Err(io::Error::other(match said().trim() {
                        "" => failure,
                        said => format!("{failure}; the server said: {said}"),
                    }));
                }
            }
        }
    };
    let started = Instant::now();
    let (mut from_server, mut to_server) = tokio::io::split(stream);
    let (mut from_client, mut to_client) = tokio::io::split(io);
    // Either side hanging up ends the relay. Waiting for both, as
    // `copy_bidirectional` does, leaves a client connected to a dead server
    // until it next writes.
    let (closer, copied) = tokio::select! {
        bytes = tokio::io::copy(&mut from_server, &mut to_client) => ("the server", bytes),
        bytes = tokio::io::copy(&mut from_client, &mut to_server) => ("the client", bytes),
    };
    match &copied {
        Ok(bytes) => tracing::info!(
            "relay ended after {:?}: {closer} closed, having sent {bytes} bytes",
            started.elapsed()
        ),
        Err(e) => tracing::warn!("relay ended after {:?}: {e}", started.elapsed()),
    }
    copied.map(|_| ())
}

/// A deliberate orphan, exactly like the desktop's own local spawn
/// (`src-tauri/src/supervisor.rs::spawn_detached`): the server must outlive
/// this relay, which itself dies with the SSH connection that started it.
/// Returns the child, for its exit status, and where this spawn's stderr
/// starts in the shared stderr file.
fn spawn_detached(
    app_config_dir: &Path,
    app_data_dir: &Path,
    log_dir: &Path,
) -> io::Result<(tokio::process::Child, Option<u64>)> {
    let bin = std::env::current_exe()?;
    let (stderr, start) = match logging::stderr_file(log_dir) {
        Ok((file, start)) => (Stdio::from(file), Some(start)),
        Err(e) => {
            tracing::warn!("the server's stderr will be discarded: {e}");
            (Stdio::null(), None)
        }
    };
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("serve")
        .env("HYPRLAYER_APP_CONFIG_DIR", app_config_dir)
        .env("HYPRLAYER_APP_DATA_DIR", app_data_dir)
        .env(logging::LOG_DIR_ENV, log_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    let child = hyprlayer_core::process::spawn_orphan(&mut cmd)?;
    tracing::info!("started {} serve as pid {:?}", bin.display(), child.id());
    Ok((child, start))
}

/// Waits for the spawned server to listen. An exit fails at once, unless it
/// lost the profile lock to a server another relay started at the same
/// moment, which will be listening shortly.
async fn wait_for_spawn(
    child: &mut tokio::process::Child,
    path: &Path,
    said: impl Fn() -> String,
) -> Result<socket::ClientStream, String> {
    let mut lost_the_race = false;
    let mut last_err = None;
    for _ in 0..SPAWN_LISTEN_RETRIES {
        match socket::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
        if !lost_the_race {
            if let Ok(Some(status)) = child.try_wait() {
                if !said().contains("runtime_in_use") {
                    return Err(format!("the server exited during startup ({status})"));
                }
                lost_the_race = true;
            }
        }
        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
    }
    let waited = CONNECT_RETRY_DELAY * SPAWN_LISTEN_RETRIES;
    Err(format!(
        "the server did not listen at {} within {}s{}",
        path.display(),
        waited.as_secs(),
        last_err.map(|e| format!(" ({e})")).unwrap_or_default()
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::sync::Arc;

    use hyprlayer_core::{HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths};

    use super::*;
    use crate::client::RpcClient;
    use crate::SocketServer;

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

    async fn open_runtime(root: &Path) -> Arc<HyprlayerRuntime> {
        let paths = paths(root);
        write_inputs(&paths);
        let options = RuntimeOptions::new(environment(&root.join("home")), paths);
        let runtime = HyprlayerRuntime::open(options).await.expect("open");
        runtime.start().await.expect("start");
        runtime
    }

    /// Stands in for `ssh <host> hyprlayer-server relay`'s joined stdio with an
    /// in-memory pipe pair, and proves a request issued on the far end reaches
    /// real dispatch and its response comes back — the byte-splice mechanics,
    /// not the SSH transport itself.
    #[tokio::test]
    async fn a_relay_over_a_pipe_pair_round_trips_a_request() {
        let root = tempfile::tempdir().unwrap();
        let runtime = open_runtime(root.path()).await;
        let app_data_dir = root.path().join("app-data");
        let app_config_dir = root.path().join("app-config");
        let server = SocketServer::bind(runtime.clone(), &app_data_dir)
            .await
            .expect("bind");
        let stop = server.stop_handle();
        let serving = tokio::spawn(server.run());

        // relay_side plays the SSH child's joined stdio; client_side is what
        // an RpcClient on the desktop side of that SSH connection would hold.
        let (relay_side, client_side) = tokio::io::duplex(64 * 1024);
        let log_dir = root.path().join("logs");
        let relaying =
            tokio::spawn(
                async move { run(app_config_dir, app_data_dir, &log_dir, relay_side).await },
            );

        let client = RpcClient::connect(client_side)
            .await
            .expect("connect through the relay");
        let folder = root.path().join("projects/notes");
        std::fs::create_dir_all(&folder).unwrap();
        let imported = client
            .import_local_folder(folder.display().to_string(), None)
            .await
            .expect("import over the relay");
        let direct = runtime
            .projects()
            .find(&imported.path)
            .await
            .expect("registry read")
            .expect("the imported project is registered");
        assert_eq!(imported.path, direct.path);
        assert_eq!(imported.mapped_name, direct.mapped_name);

        drop(client);
        relaying.abort();
        stop.stop();
        serving.await.unwrap().unwrap();
        runtime.shutdown().await;
    }
}
