//! `hyprlayer-server relay`: the server-side half of the SSH transport
//! (ADR-0009, plan Phase 5 §1). Connects to the local socket, spawning `serve`
//! detached first if nothing is listening, then splices the connection to its
//! caller's I/O byte for byte — framing-blind, so the SSH child on the far
//! end needs no protocol knowledge of its own.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::socket;

const CONNECT_RETRIES: u32 = 50;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Connects to the profile's socket and splices `io` to it until either side
/// closes. `io` is the process's own stdin/stdout in production; a test
/// substitutes an in-memory pipe.
pub async fn run<S>(app_config_dir: PathBuf, app_data_dir: PathBuf, mut io: S) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let path = socket::socket_path(&app_data_dir);
    let mut stream = match socket::connect(&path).await {
        Ok(stream) => stream,
        Err(_) => {
            spawn_detached(&app_config_dir, &app_data_dir)?;
            wait_for_socket(&path).await?
        }
    };
    tokio::io::copy_bidirectional(&mut stream, &mut io).await?;
    Ok(())
}

/// A deliberate orphan, exactly like the desktop's own local spawn
/// (`src-tauri/src/supervisor.rs::spawn_detached`): the server must outlive
/// this relay, which itself dies with the SSH connection that started it.
fn spawn_detached(app_config_dir: &Path, app_data_dir: &Path) -> io::Result<()> {
    let bin = std::env::current_exe()?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("serve")
        .env("HYPRLAYER_APP_CONFIG_DIR", app_config_dir)
        .env("HYPRLAYER_APP_DATA_DIR", app_data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hyprlayer_core::process::spawn_orphan(&mut cmd)?;
    Ok(())
}

async fn wait_for_socket(path: &Path) -> io::Result<socket::ClientStream> {
    let mut last_err = None;
    for _ in 0..CONNECT_RETRIES {
        match socket::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("timed out waiting for the server to listen")))
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::sync::Arc;

    use hyprlayer_core::{
        HyprlayerRuntime, NoCredentialLookup, RuntimeEnvironment, RuntimeOptions, RuntimePaths,
    };

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
        let options = RuntimeOptions::new(environment(&root.join("home")), paths)
            .with_credential_lookup(Arc::new(NoCredentialLookup));
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
        let relaying = tokio::spawn(run(app_config_dir, app_data_dir, relay_side));

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
