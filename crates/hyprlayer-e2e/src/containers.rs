//! The container tier: a Linux host running sshd, reached the way the desktop
//! reaches a remote host, `ssh <host> "<version-keyed server>" relay`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use hyprlayer_core::models::RuntimeEvent;
use hyprlayer_server::client::RpcClient;
use tokio::process::Child;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::sandbox::Sandbox;

const IMAGE: &str = "hyprlayer-e2e/remote-host";
/// Bookworm on both sides, so the binary it builds runs on the host's glibc.
const BUILDER: &str = "rust:1-bookworm";
const ALIAS: &str = "e2e-host";
const SSHD_READY: Duration = Duration::from_secs(30);

/// Which build of the server the host runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// As shipped, minus LTO for rebuild time.
    Release,
    /// Hashes its own ~300 MB binary before binding, so it starts slowly.
    Debug,
}

/// A running host container, removed on drop.
pub struct RemoteHost {
    name: String,
    ssh_config: PathBuf,
    home: PathBuf,
    logs: PathBuf,
    relays: AtomicU32,
}

/// One SSH connection carrying the relay, and the client speaking over it.
pub struct Relay {
    pub client: Arc<RpcClient>,
    pub events: UnboundedReceiver<RuntimeEvent>,
    child: Child,
}

impl RemoteHost {
    pub fn start(sandbox: &Sandbox) -> Self {
        Self::start_with(sandbox, Build::Release)
    }

    pub fn start_with(sandbox: &Sandbox, build: Build) -> Self {
        let binary = linux_server(build);
        docker(&["build", "--quiet", "--tag", IMAGE, &dockerfile_dir()]);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or_default();
        let name = format!("hyprlayer-e2e-{}-{nanos}", std::process::id());
        docker(&[
            "run",
            "--detach",
            "--rm",
            "--label",
            "hyprlayer-e2e=1",
            "--name",
            &name,
            "--publish",
            "127.0.0.1::22",
            IMAGE,
        ]);
        let port = docker(&["port", &name, "22/tcp"])
            .lines()
            .next()
            .and_then(|line| line.rsplit(':').next())
            .expect("a published sshd port")
            .to_string();

        let ssh = sandbox.root.join("ssh");
        std::fs::create_dir_all(&ssh).expect("create the ssh dir");
        let key = ssh.join("id_ed25519");
        run(Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-q", "-C", "hyprlayer-e2e", "-f"])
            .arg(&key));
        let ssh_config = ssh.join("config");
        // `IdentityAgent none` and `-F`: the user's agent and ~/.ssh/config
        // (a 1Password agent would prompt) never take part.
        std::fs::write(
            &ssh_config,
            format!(
                "Host {ALIAS}\n  HostName 127.0.0.1\n  Port {port}\n  User e2e\n  \
                 IdentityFile {key}\n  IdentitiesOnly yes\n  IdentityAgent none\n  \
                 StrictHostKeyChecking no\n  UserKnownHostsFile {known}\n  BatchMode yes\n  \
                 LogLevel ERROR\n  ConnectTimeout 10\n",
                key = key.display(),
                known = ssh.join("known_hosts").display(),
            ),
        )
        .expect("write the ssh config");

        let host = Self {
            name,
            ssh_config,
            home: sandbox.home.clone(),
            logs: sandbox.logs.clone(),
            relays: AtomicU32::new(0),
        };
        let public = std::fs::read(key.with_extension("pub")).expect("read the public key");
        run(Command::new("docker")
            .args(["exec", "--interactive", "--user", "e2e", &host.name, "sh", "-c"])
            .arg("mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat > ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys")
            .stdin_bytes(&public));
        docker(&[
            "cp",
            &binary.display().to_string(),
            &format!("{}:/tmp/hyprlayer-server", host.name),
        ]);
        host.exec(&format!(
            "install -D -m 755 /tmp/hyprlayer-server \"$HOME/.hyprlayer/server/{}/hyprlayer-server\"",
            hyprlayer_server::VERSION
        ));
        host.wait_for_sshd();
        host
    }

    fn wait_for_sshd(&self) {
        let started = Instant::now();
        while !self.ssh().arg("true").status().is_ok_and(|s| s.success()) {
            assert!(
                started.elapsed() < SSHD_READY,
                "sshd in {} never accepted the key",
                self.name
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn ssh(&self) -> Command {
        let mut ssh = Command::new("ssh");
        ssh.arg("-F")
            .arg(&self.ssh_config)
            .arg(ALIAS)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        ssh
    }

    /// What the desktop runs: the relay under the version-keyed install path.
    pub async fn relay(&self) -> Relay {
        self.try_relay()
            .await
            .unwrap_or_else(|e| panic!("relay failed: {e}"))
    }

    /// A relay whose failure is the scenario's to judge: the handshake error
    /// plus whatever the relay said on SSH's stderr.
    pub async fn try_relay(&self) -> Result<Relay, String> {
        let n = self.relays.fetch_add(1, Ordering::SeqCst) + 1;
        let stderr_log = self.relay_log(n);
        let stderr = std::fs::File::create(&stderr_log).expect("create the ssh log");
        let mut child = tokio::process::Command::from(self.ssh())
            .arg(format!(
                "\"$HOME/.hyprlayer/server/{}/hyprlayer-server\" relay",
                hyprlayer_server::VERSION
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn ssh");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        match RpcClient::connect(tokio::io::join(stdout, stdin)).await {
            Ok(client) => {
                let events = client.take_events().expect("a fresh client's events");
                Ok(Relay {
                    client,
                    events,
                    child,
                })
            }
            Err(e) => {
                let _ = child.wait().await;
                let said = std::fs::read_to_string(&stderr_log).unwrap_or_default();
                Err(format!("{e}: {}", said.trim()))
            }
        }
    }

    /// What the `n`th relay's SSH session wrote to stderr.
    pub fn relay_stderr(&self, n: u32) -> String {
        std::fs::read_to_string(self.relay_log(n)).unwrap_or_default()
    }

    fn relay_log(&self, n: u32) -> PathBuf {
        self.logs.join(format!("ssh-relay-{n}.log"))
    }

    /// Runs `script` with `sh` as the host's user, from its home.
    pub fn exec(&self, script: &str) -> String {
        docker(&[
            "exec",
            "--user",
            "e2e",
            "--workdir",
            "/home/e2e",
            &self.name,
            "sh",
            "-c",
            script,
        ])
    }

    /// Pids whose command line matches `pattern`.
    pub fn pids(&self, pattern: &str) -> Vec<String> {
        let output = Command::new("docker")
            .args(["exec", &self.name, "pgrep", "-f", pattern])
            .output()
            .expect("run pgrep in the host");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// SIGKILLs every process matching `pattern`, if any. Not through `sh -c`,
    /// whose own command line would match and die with them.
    pub fn kill(&self, pattern: &str) {
        let _ = Command::new("docker")
            .args(["exec", &self.name, "pkill", "-KILL", "-f", pattern])
            .output();
    }

    /// Copies the host server's own log tails next to the local logs.
    pub fn save_server_logs(&self) {
        let tails = self.exec(&format!(
            "\"$HOME/.hyprlayer/server/{}/hyprlayer-server\" logs",
            hyprlayer_server::VERSION
        ));
        let _ = std::fs::write(self.logs.join("remote-server.log"), tails);
    }
}

impl Drop for RemoteHost {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", &self.name])
            .output();
    }
}

impl Relay {
    /// Ends the SSH connection the way a closed laptop lid would.
    pub async fn hang_up(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

/// `hyprlayer-server` for the host's platform, built from this tree inside
/// `BUILDER` with its target and registry cached in named volumes.
fn linux_server(build: Build) -> &'static Path {
    static RELEASE: OnceLock<PathBuf> = OnceLock::new();
    static DEBUG: OnceLock<PathBuf> = OnceLock::new();
    let (cell, flag, profile) = match build {
        Build::Release => (&RELEASE, "--release", "release"),
        Build::Debug => (&DEBUG, "", "debug"),
    };
    cell.get_or_init(|| {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crates/hyprlayer-e2e sits two levels under the workspace");
        let out = crate::journey::target_dir()
            .join("e2e")
            .join("linux")
            .join(profile);
        std::fs::create_dir_all(&out).expect("create the linux output dir");
        docker(&[
            "run",
            "--rm",
            "--volume",
            &format!("{}:/src:ro", workspace.display()),
            "--volume",
            "hyprlayer-e2e-cargo:/usr/local/cargo/registry",
            "--volume",
            "hyprlayer-e2e-target:/target",
            "--volume",
            &format!("{}:/out", out.display()),
            "--env",
            "CARGO_TARGET_DIR=/target",
            "--env",
            "CARGO_PROFILE_RELEASE_LTO=off",
            "--env",
            "CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16",
            "--workdir",
            "/src",
            BUILDER,
            "sh",
            "-c",
            &format!(
                "cargo build {flag} --locked --quiet --package hyprlayer-server --bin hyprlayer-server \
                 && cp /target/{profile}/hyprlayer-server /out/"
            ),
        ]);
        out.join("hyprlayer-server")
    })
}

fn dockerfile_dir() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("containers")
        .join("remote-host")
        .display()
        .to_string()
}

fn docker(args: &[&str]) -> String {
    run(Command::new("docker").args(args))
}

fn run(command: &mut Command) -> String {
    let output = command.output().expect("spawn a command");
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

trait StdinBytes {
    fn stdin_bytes(&mut self, bytes: &[u8]) -> &mut Self;
}

impl StdinBytes for Command {
    /// Feeds `bytes` to the child's stdin through a temp file, which keeps
    /// `run` a plain `output()`.
    fn stdin_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        let mut file = tempfile::tempfile().expect("a temp file for stdin");
        std::io::Write::write_all(&mut file, bytes).expect("write stdin");
        std::io::Seek::rewind(&mut file).expect("rewind stdin");
        self.stdin(file)
    }
}
