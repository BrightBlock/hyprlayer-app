//! The tokio runtime on its own thread: it owns the `RpcClient` and feeds entities over a channel.
use std::path::PathBuf;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_core::models::TerminalEvent;
use hyprlayer_proto::Features;
use hyprlayer_server::client::RpcClient;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// The hosted server's pid: `cx.quit()` exits without dropping the child, so quit kills it by pid.
pub static HOSTED_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub enum Msg {
    Bytes(Vec<u8>),
    Resize(u16, u16),
    Status(String),
    Tick(u64),
}

pub enum Source {
    /// Attach to a running server's workstream.
    Attach { socket: PathBuf, workstream: String },
    /// Spawn a server on a throwaway profile (unix only) and open a terminal in a seeded workstream.
    Host,
    /// No server: canned ANSI, for machines that only prove the window.
    Synthetic,
}

pub fn spawn(source: Source) -> UnboundedReceiver<Msg> {
    let (tx, rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("tokio-feed".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let ticker = tx.clone();
                tokio::spawn(async move {
                    let mut n = 0u64;
                    loop {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        n += 1;
                        if ticker.send(Msg::Tick(n)).is_err() {
                            return;
                        }
                    }
                });
                let result = match source {
                    Source::Synthetic => synthetic(&tx).await,
                    Source::Attach { socket, workstream } => {
                        attach(&tx, &socket, &workstream).await
                    }
                    #[cfg(unix)]
                    Source::Host => host::run(&tx).await,
                    #[cfg(not(unix))]
                    Source::Host => Err(anyhow::anyhow!("hosted mode is unix-only")),
                };
                if let Err(e) = result {
                    eprintln!("feed ended: {e:#}");
                    let _ = tx.send(Msg::Status(format!("feed ended: {e:#}")));
                    let _ = synthetic(&tx).await;
                }
                std::future::pending::<()>().await;
            });
        })
        .expect("spawn feed thread");
    rx
}

const DEMO: &str = "\x1b[1;32mhyprlayer\x1b[0m gpui spike  \x1b[2m(synthetic feed)\x1b[0m\r\n\
\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m \x1b[33myellow\x1b[0m \x1b[34mblue\x1b[0m \x1b[7minverse\x1b[0m \x1b[48;5;24m bg24 \x1b[0m\r\n\
┌──────────────┐ box drawing\r\n│ ╭──╮ ▀▄█ ░▒▓ │ block elements\r\n└──────────────┘ wide: 日本語 \r\n$ ";

async fn synthetic(tx: &UnboundedSender<Msg>) -> anyhow::Result<()> {
    tx.send(Msg::Bytes(DEMO.as_bytes().to_vec()))?;
    Ok(())
}

async fn attach(
    tx: &UnboundedSender<Msg>,
    socket: &std::path::Path,
    workstream: &str,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    let stream = tokio::net::UnixStream::connect(socket).await?;
    #[cfg(not(unix))]
    let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(socket)?;
    let client = RpcClient::connect_announcing(stream, Features::ALL)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tx.send(Msg::Status(format!(
        "connected, server {} (protocol {})",
        client.server_semver().unwrap_or("?"),
        client.server_version()
    )))?;
    pump(tx, client, workstream).await
}

async fn pump(
    tx: &UnboundedSender<Msg>,
    client: std::sync::Arc<RpcClient>,
    workstream: &str,
) -> anyhow::Result<()> {
    let (terminal, stream_id, mut events) = client
        .terminal_open_stream(workstream.to_string(), 100, 24)
        .await
        .map_err(|e| anyhow::anyhow!("terminal_open: {e:?}"))?;
    tx.send(Msg::Status(format!("live terminal {terminal}")))?;
    let _ = client
        .terminal_write(
            terminal.clone(),
            B64.encode(
                "echo LIVE_TERMINAL_OK; ls -G /; printf '\\033[1;35mbold magenta\\033[0m\\n'\n",
            ),
        )
        .await;
    while let Some(event) = events.recv().await {
        match event {
            TerminalEvent::Resized { cols, rows, .. } => tx.send(Msg::Resize(cols, rows))?,
            TerminalEvent::Output { data, frame, .. } => {
                tx.send(Msg::Bytes(B64.decode(&data).unwrap_or_default()))?;
                if let Some(seq) = frame {
                    let _ = client.terminal_frame_ack(stream_id, seq).await;
                }
            }
            TerminalEvent::Clipboard { .. } => {}
            TerminalEvent::Exited { code } => {
                tx.send(Msg::Status(format!("terminal exited: {code:?}")))?;
                break;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
mod host {
    use super::*;
    use hyprlayer_core::runtime::projects::ThoughtsSource;
    use hyprlayer_core::WorktreeMode;
    use hyprlayer_server::rpc::Request;
    use std::process::Stdio;

    fn server_bin() -> anyhow::Result<PathBuf> {
        if let Some(p) = std::env::var_os("HYPRLAYER_SPIKE_SERVER") {
            return Ok(p.into());
        }
        let exe = std::env::current_exe()?;
        let sibling = exe.with_file_name("hyprlayer-server");
        anyhow::ensure!(
            sibling.exists(),
            "no hyprlayer-server next to {}",
            exe.display()
        );
        Ok(sibling)
    }

    fn tagged<T: serde::Serialize>(v: &T, command: &str) -> serde_json::Value {
        serde_json::to_value(v)
            .ok()
            .and_then(|mut v| v.get_mut(command).map(serde_json::Value::take))
            .unwrap_or_default()
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> anyhow::Result<()> {
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "spike")
            .env("GIT_AUTHOR_EMAIL", "spike@example.com")
            .env("GIT_COMMITTER_NAME", "spike")
            .env("GIT_COMMITTER_EMAIL", "spike@example.com")
            .status()?;
        anyhow::ensure!(st.success(), "git {args:?} failed");
        Ok(())
    }

    pub async fn run(tx: &UnboundedSender<Msg>) -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("gpui-spike-{}", std::process::id()));
        let (home, cfg, data, logs, run) = (
            root.join("home"),
            root.join("config"),
            root.join("data"),
            root.join("logs"),
            root.join("run"),
        );
        for d in [&home, &cfg, &data, &logs, &run] {
            std::fs::create_dir_all(d)?;
        }
        let repo = root.join("demo");
        std::fs::create_dir_all(&repo)?;
        git(&repo, &["init", "-q", "-b", "main"])?;
        std::fs::write(repo.join("README.md"), "# demo\n")?;
        git(&repo, &["add", "."])?;
        git(&repo, &["commit", "-q", "-m", "init"])?;
        let socket = if cfg!(target_os = "linux") {
            run.join("hyprlayer").join(format!(
                "{}.sock",
                hyprlayer_server::socket::profile_hash(&data)
            ))
        } else {
            data.join("hyprlayer.sock")
        };
        let child = tokio::process::Command::new(server_bin()?)
            .arg("serve")
            .env_clear()
            .env("HOME", &home)
            .env("USER", "spike")
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin")
            .env("SHELL", "/bin/sh")
            .env("XDG_RUNTIME_DIR", &run)
            .env("HYPRLAYER_APP_CONFIG_DIR", &cfg)
            .env("HYPRLAYER_APP_DATA_DIR", &data)
            .env("HYPRLAYER_APP_LOG_DIR", &logs)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        HOSTED_PID.store(
            child.id().unwrap_or(0),
            std::sync::atomic::Ordering::Relaxed,
        );
        let _child = child;
        tx.send(Msg::Status(
            "starting hosted server on a throwaway profile".into(),
        ))?;
        let client = loop {
            match RpcClient::connect_socket(&socket).await {
                Ok(_) => break (),
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        };
        let _ = client;
        let stream = tokio::net::UnixStream::connect(&socket).await?;
        let client = RpcClient::connect_announcing(stream, Features::ALL)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let probe = client
            .call(Request::ProbeDefaultThoughts {})
            .await
            .map_err(|e| anyhow::anyhow!("probe: {e:?}"))?;
        let probe = tagged(&probe, "probe_default_thoughts");
        let path = probe
            .pointer("/default_path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        client
            .call(Request::InitThoughtsRepo {
                user: "spike".into(),
                source: ThoughtsSource::Create { path },
            })
            .await
            .map_err(|e| anyhow::anyhow!("init_thoughts_repo: {e:?}"))?;
        let project = client
            .call(Request::ImportProject {
                path: repo.display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("import_project: {e:?}"))?;
        let project = tagged(&project, "import_project");
        let project_path = project
            .pointer("/path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let view = client
            .call(Request::CreateWorkstream {
                project_path,
                title: "spike".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("create_workstream: {e:?}"))?;
        let view = tagged(&view, "create_workstream");
        let id = view
            .pointer("/item/id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        super::pump(tx, client, &id).await
    }
}
