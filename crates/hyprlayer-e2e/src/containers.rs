//! The container tier: a Linux host running sshd, reached the way the desktop
//! reaches a remote host, `ssh <host> "<version-keyed server>" relay`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use hyprlayer_core::models::RuntimeEvent;
use hyprlayer_server::client::RpcClient;
use tokio::process::Child;
use tokio::sync::mpsc::Receiver;

use crate::sandbox::Sandbox;

const IMAGE: &str = "hyprlayer-e2e/remote-host";
/// Bookworm on both sides, so the binary it builds runs on the host's glibc.
const BUILDER: &str = "rust:1-bookworm";
/// The remote host's base; `containers/remote-host/Dockerfile` takes it as `BASE`.
const HOST_BASE: &str = "debian:bookworm-slim";
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

/// The host's architecture.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Whatever Docker runs natively; `aarch64` on Apple silicon, which the
    /// desktop has no server for.
    Native,
    /// The triple the desktop provisions. Emulated on Apple silicon.
    Amd64,
}

impl Platform {
    fn docker(self) -> &'static str {
        match self {
            Platform::Native => native_platform(),
            Platform::Amd64 => "linux/amd64",
        }
    }

    /// A private tag per platform: `docker run --platform` would re-point the shared base tag.
    fn builder(self) -> &'static str {
        match self {
            Platform::Native => "hyprlayer-e2e/builder:native",
            Platform::Amd64 => "hyprlayer-e2e/builder:amd64",
        }
    }

    /// The remote host's base, privately tagged for the same reason.
    fn host_base(self) -> &'static str {
        match self {
            Platform::Native => "hyprlayer-e2e/host-base:native",
            Platform::Amd64 => "hyprlayer-e2e/host-base:amd64",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Platform::Native => "",
            Platform::Amd64 => "-amd64",
        }
    }
}

pub struct HostOptions {
    pub build: Build,
    pub platform: Platform,
    /// `e2e` may also log in with this password, as `password` or as PAM's
    /// keyboard-interactive (how macOS's sshd asks).
    pub password: Option<&'static str>,
    /// The server at its version-keyed path, as on a provisioned host.
    pub install: bool,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            build: Build::Release,
            platform: Platform::Native,
            password: None,
            install: true,
        }
    }
}

/// A running host container, removed on drop.
pub struct RemoteHost {
    name: String,
    ssh_config: PathBuf,
    home: PathBuf,
    logs: PathBuf,
    port: String,
    identity: PathBuf,
    binary: PathBuf,
    relays: AtomicU32,
}

/// One SSH connection carrying the relay, and the client speaking over it.
pub struct Relay {
    pub client: Arc<RpcClient>,
    pub events: Receiver<RuntimeEvent>,
    child: Child,
}

impl RemoteHost {
    pub fn start(sandbox: &Sandbox) -> Self {
        Self::start_with(sandbox, Build::Release)
    }

    pub fn start_with(sandbox: &Sandbox, build: Build) -> Self {
        Self::start_opts(
            sandbox,
            HostOptions {
                build,
                ..HostOptions::default()
            },
        )
    }

    pub fn start_opts(sandbox: &Sandbox, options: HostOptions) -> Self {
        // Under nextest, unbounded container tests exhaust the Docker VM.
        if std::env::var_os("NEXTEST").is_some()
            && std::env::var("NEXTEST_TEST_GROUP").as_deref() != Ok("docker")
        {
            panic!(
                "container tests must be in nextest's `docker` test group; \
                 add this binary to its filter in .config/nextest.toml"
            );
        }
        let lock = build_lock();
        let platform = options.platform.docker();
        ensure_tag(BUILDER, options.platform.builder(), options.platform);
        ensure_tag(HOST_BASE, options.platform.host_base(), options.platform);
        let binary = linux_server(options.build, options.platform);
        let image = match options.platform {
            Platform::Native => IMAGE.to_string(),
            Platform::Amd64 => format!("{IMAGE}:amd64"),
        };
        let dockerfile = dockerfile_dir();
        let base = format!("BASE={}", options.platform.host_base());
        docker(&[
            "build",
            "--quiet",
            "--platform",
            platform,
            "--build-arg",
            &base,
            "--tag",
            &image,
            &dockerfile,
        ]);
        drop(lock);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or_default();
        let name = format!("hyprlayer-e2e-{}-{nanos}", std::process::id());
        let mut run_args = vec![
            "run",
            "--detach",
            "--rm",
            "--label",
            "hyprlayer-e2e=1",
            "--name",
            &name,
            "--publish",
            "127.0.0.1::22",
            "--platform",
            platform,
            &image,
        ];
        // sshd's command line outranks the image's `PasswordAuthentication no`.
        let with_password = options.password.map(|password| {
            format!(
                "echo 'e2e:{password}' | chpasswd && exec /usr/sbin/sshd -D -e \
                 -o PasswordAuthentication=yes -o KbdInteractiveAuthentication=yes"
            )
        });
        if let Some(script) = &with_password {
            run_args.extend(["sh", "-c", script]);
        }
        docker(&run_args);
        let mut started = Started(Some(name.clone()));
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

        started.0 = None;
        let host = Self {
            name,
            ssh_config,
            home: sandbox.home.clone(),
            logs: sandbox.logs.clone(),
            port,
            identity: key.clone(),
            binary,
            relays: AtomicU32::new(0),
        };
        let public = std::fs::read(key.with_extension("pub")).expect("read the public key");
        run(Command::new("docker")
            .args(["exec", "--interactive", "--user", "e2e", &host.name, "sh", "-c"])
            .arg("mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat > ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys")
            .stdin_bytes(&public));
        if options.install {
            docker(&[
                "cp",
                &host.binary.display().to_string(),
                &format!("{}:/tmp/hyprlayer-server", host.name),
            ]);
            host.exec(&format!(
                "install -D -m 755 /tmp/hyprlayer-server \"$HOME/.hyprlayer/server/{}/hyprlayer-server\"",
                hyprlayer_server::VERSION
            ));
        }
        host.wait_for_sshd();
        host
    }

    /// The published sshd port on 127.0.0.1.
    pub fn port(&self) -> &str {
        &self.port
    }

    /// The private key `e2e` accepts.
    pub fn identity(&self) -> &Path {
        &self.identity
    }

    /// The server built for this host's platform.
    pub fn server_binary(&self) -> &Path {
        &self.binary
    }

    /// Everything sshd has logged, logins included.
    pub fn sshd_log(&self) -> String {
        let output = Command::new("docker")
            .args(["logs", &self.name])
            .output()
            .expect("run docker logs");
        String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr)
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

/// Removes a container whose setup panicked before `RemoteHost`, whose `Drop` takes over.
struct Started(Option<String>);

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(name) = self.0.take() {
            let _ = Command::new("docker")
                .args(["rm", "--force", &name])
                .output();
        }
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

/// `hyprlayer-server` for the host's platform, built from this tree in the
/// platform's builder with its target and registry cached in named volumes,
/// one set per platform. The caller holds `build_lock`.
fn linux_server(build: Build, platform: Platform) -> PathBuf {
    static BUILT: Mutex<Vec<(Build, Platform, PathBuf)>> = Mutex::new(Vec::new());
    let mut built = BUILT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((.., path)) = built.iter().find(|(b, p, _)| *b == build && *p == platform) {
        return path.clone();
    }
    let (flag, profile) = match build {
        Build::Release => ("--release", "release"),
        Build::Debug => ("", "debug"),
    };
    let suffix = platform.suffix();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/hyprlayer-e2e sits two levels under the workspace");
    let out = crate::journey::target_dir()
        .join("e2e")
        .join(format!("linux{suffix}"))
        .join(profile);
    std::fs::create_dir_all(&out).expect("create the linux output dir");
    let src = format!("{}:/src:ro", workspace.display());
    let cargo = format!("hyprlayer-e2e-cargo{suffix}:/usr/local/cargo/registry");
    let target = format!("hyprlayer-e2e-target{suffix}:/target");
    let out_volume = format!("{}:/out", out.display());
    // Copied then renamed, so a concurrent `docker cp` never reads half a binary.
    let script = format!(
        "cargo build {flag} --locked --quiet --package hyprlayer-server --bin hyprlayer-server \
         && cp /target/{profile}/hyprlayer-server /out/.hyprlayer-server.tmp \
         && mv -f /out/.hyprlayer-server.tmp /out/hyprlayer-server"
    );
    let mut args = vec!["run", "--rm", "--platform", platform.docker()];
    args.extend([
        "--volume",
        &src,
        "--volume",
        &cargo,
        "--volume",
        &target,
        "--volume",
        &out_volume,
        "--env",
        "CARGO_TARGET_DIR=/target",
        "--env",
        "CARGO_PROFILE_RELEASE_LTO=off",
        "--env",
        "CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16",
        "--workdir",
        "/src",
        platform.builder(),
        "sh",
        "-c",
        &script,
    ]);
    docker(&args);
    let path = out.join("hyprlayer-server");
    built.push((build, platform, path.clone()));
    path
}

/// Builders share target volumes and the Docker VM's memory: one at a time.
fn build_lock() -> std::fs::File {
    let dir = crate::journey::target_dir().join("e2e");
    std::fs::create_dir_all(&dir).expect("create the e2e output dir");
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".linux-build.lock"))
        .expect("open the build lock");
    file.lock().expect("lock the linux build");
    file
}

/// Tags `base` privately as `tag` for the platform, once; later runs find it.
fn ensure_tag(base: &str, tag: &str, platform: Platform) {
    let present = Command::new("docker")
        .args(["image", "inspect", tag])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !present {
        run(Command::new("docker")
            .args([
                "build",
                "--quiet",
                "--platform",
                platform.docker(),
                "--tag",
                tag,
                "-",
            ])
            .stdin_bytes(format!("FROM {base}\n").as_bytes()));
    }
}

/// Docker's own platform, passed explicitly so a native run can't resolve to another variant.
fn native_platform() -> &'static str {
    static NATIVE: OnceLock<String> = OnceLock::new();
    NATIVE.get_or_init(|| docker(&["version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"]))
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
