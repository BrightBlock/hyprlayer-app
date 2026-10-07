//! Every `ssh` this app runs: one command shape, one connection per host shared
//! across runs unless the user's config sets its own `ControlPath`, and the
//! window answering whatever ssh asks. Windows keeps plain `ssh`: its OpenSSH
//! has no working `ControlMaster`.

mod askpass;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::hosts::Host;
use crate::supervisor::hide_console;

pub(crate) use askpass::{Ended, SignIn};
pub use askpass::{SshPrompt, SshPromptKind};

/// ssh's default is the OS TCP timeout, over a minute for an asleep host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a shared connection outlives its last session.
const CONTROL_PERSIST: &str = "ControlPersist=10m";
/// Ends a shared connection over a dead link in ~45s rather than TCP's hours.
const SERVER_ALIVE: [&str; 2] = ["ServerAliveInterval=15", "ServerAliveCountMax=3"];
/// For every question in one ssh run together: sshd's default 120s
/// `LoginGraceTime` covers the whole login, counted from the TCP accept.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(95);
/// `sun_path` holds 103 bytes, and ssh binds at the path plus 17 random bytes.
const CONTROL_PATH_MAX: usize = 103 - 17;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SshSetup {
    pub program: PathBuf,
    /// The askpass helper: this app's own binary.
    pub askpass: Option<PathBuf>,
    pub socket_dir: PathBuf,
    /// Where shared connections live; `None` never shares.
    pub control_dir: Option<PathBuf>,
    pub prompt_timeout: Duration,
}

impl SshSetup {
    /// `HYPRLAYER_SSH` picks the program, as `GIT_SSH` does for git.
    pub fn for_app(app_data_dir: &Path) -> Self {
        Self {
            program: std::env::var_os("HYPRLAYER_SSH")
                .filter(|p| !p.is_empty())
                .map_or_else(|| PathBuf::from("ssh"), PathBuf::from),
            askpass: std::env::current_exe().ok(),
            socket_dir: app_data_dir.to_path_buf(),
            control_dir: control_dir(app_data_dir),
            prompt_timeout: PROMPT_TIMEOUT,
        }
    }
}

/// Short and private: macOS's per-user `TMPDIR`, else `XDG_RUNTIME_DIR`.
fn control_dir(app_data_dir: &Path) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        return None;
    } else if cfg!(target_os = "macos") {
        std::env::temp_dir()
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map_or_else(|| app_data_dir.to_path_buf(), PathBuf::from)
    };
    Some(base.join("hyprlayer-ssh"))
}

/// Narrowing to 0700 fails unless we own it, which is the check that matters
/// for a directory that could be under a shared `/tmp`.
fn prepare(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ready = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)));
        if let Err(e) = ready {
            log::warn!("not sharing ssh connections: {}: {e}", dir.display());
            return false;
        }
        true
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        false
    }
}

/// How this app runs `ssh`; cheap to clone. The app's own handle may ask the
/// user; `batch()` never does.
#[derive(Clone)]
pub struct Ssh {
    shared: Arc<Shared>,
    ask: bool,
}

struct Shared {
    program: PathBuf,
    control_dir: Option<PathBuf>,
    askpass: Option<askpass::Askpass>,
}

/// A command up to the destination; the caller adds the remote command.
pub(crate) struct SshCommand {
    pub cmd: tokio::process::Command,
    pub sign_in: Option<SignIn>,
    /// `BatchMode`: a login it could not finish needs the user.
    pub batch: bool,
}

impl Ssh {
    pub fn new(setup: SshSetup) -> Self {
        let control_dir = setup.control_dir.filter(|dir| prepare(dir));
        let askpass = setup.askpass.filter(|_| cfg!(unix)).map(|helper| {
            askpass::Askpass::new(
                helper,
                setup.socket_dir.join("askpass.sock"),
                setup.prompt_timeout,
            )
        });
        Self {
            shared: Arc::new(Shared {
                program: setup.program,
                control_dir,
                askpass,
            }),
            ask: true,
        }
    }

    /// This handle, never asking: for launch, when no window can answer yet,
    /// and for reads nobody pressed a button for.
    pub fn batch(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            ask: false,
        }
    }

    /// What ssh is waiting on the user for, oldest first.
    pub fn prompts(&self) -> watch::Receiver<Vec<SshPrompt>> {
        match &self.shared.askpass {
            Some(askpass) => askpass.prompts(),
            None => watch::channel(Vec::new()).1,
        }
    }

    /// `None` cancels; `false` when no such question is waiting.
    pub fn answer(&self, id: &str, answer: Option<String>) -> bool {
        self.shared
            .askpass
            .as_ref()
            .is_some_and(|askpass| askpass.answer(id, answer))
    }

    pub(crate) async fn command(&self, host: &Host) -> SshCommand {
        let resolved = self.resolve(host).await;
        let mut cmd = tokio::process::Command::new(&self.shared.program);
        hide_console(&mut cmd);
        // Through a proxy it also bounds the wait for the far end's banner,
        // which a jump host's own sign-in holds up.
        let proxied = resolved.as_ref().is_some_and(|r| r.proxied);
        let can_ask = self.ask && self.shared.askpass.is_some();
        let connect_timeout = if proxied && can_ask {
            PROMPT_TIMEOUT
        } else {
            CONNECT_TIMEOUT
        };
        cmd.arg("-o")
            .arg(format!("ConnectTimeout={}", connect_timeout.as_secs()));
        if let Some(path) = self.control_path(resolved.as_ref()) {
            cmd.args(["-o", "ControlMaster=auto", "-o"])
                .arg(control_path_option(&path))
                .args(["-o", CONTROL_PERSIST])
                .args(["-o", SERVER_ALIVE[0], "-o", SERVER_ALIVE[1]]);
        }
        let mut sign_in = None;
        // Whether or not the helper exists: a run that may not ask must not.
        let batch = cfg!(unix) && !self.ask;
        if batch {
            cmd.args(["-o", "BatchMode=yes"]);
        } else if let Some(askpass) = self.shared.askpass.as_ref().filter(|_| self.ask) {
            match askpass.sign_in(host).await {
                Ok(registered) => {
                    cmd.envs(registered.env());
                    sign_in = Some(registered);
                }
                // Nothing can answer, so fail rather than ask a terminal nobody watches.
                Err(e) => {
                    log::warn!("ssh to {} cannot ask for anything: {e}", host.destination);
                    cmd.args(["-o", "BatchMode=yes"]);
                }
            }
        }
        cmd.arg(&host.destination);
        SshCommand {
            cmd,
            sign_in,
            batch,
        }
    }

    /// Like `tokio::time::timeout`, except the clock stops while the window is
    /// waiting on the user.
    pub(crate) async fn within<F: Future>(&self, limit: Duration, fut: F) -> Option<F::Output> {
        let Some(askpass) = self.shared.askpass.as_ref().filter(|_| self.ask) else {
            return tokio::time::timeout(limit, fut).await.ok();
        };
        tokio::pin!(fut);
        let start = tokio::time::Instant::now();
        let base = askpass.waited();
        let deadline = || start + limit + askpass.waited().saturating_sub(base);
        loop {
            tokio::select! {
                out = &mut fut => return Some(out),
                () = tokio::time::sleep_until(deadline()) => {
                    let mut prompts = askpass.prompts();
                    if !prompts.borrow().is_empty() {
                        tokio::select! {
                            out = &mut fut => return Some(out),
                            _ = prompts.wait_for(Vec::is_empty) => {}
                        }
                    } else if tokio::time::Instant::now() >= deadline() {
                        return None;
                    }
                }
            }
        }
    }

    /// This app's socket for `host`, unless the user's config multiplexes it.
    fn control_path(&self, resolved: Option<&Resolved>) -> Option<PathBuf> {
        let dir = self.shared.control_dir.as_ref()?;
        let resolved = resolved?;
        if resolved.theirs {
            return None;
        }
        let path = dir.join(fnv(&resolved.key));
        if path.as_os_str().len() > CONTROL_PATH_MAX {
            log::warn!(
                "not sharing ssh connections: {} is too long",
                path.display()
            );
            return None;
        }
        Some(path)
    }

    /// `ssh -G`: the config as ssh will apply it, without connecting.
    async fn resolve(&self, host: &Host) -> Option<Resolved> {
        let mut cmd = tokio::process::Command::new(&self.shared.program);
        hide_console(&mut cmd);
        cmd.arg("-G")
            .arg(&host.destination)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(RESOLVE_TIMEOUT, cmd.output())
            .await
            .ok()?
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let set = |name: &str| {
            text.lines().any(|line| {
                line.split_once(' ')
                    .is_some_and(|(key, value)| key == name && value.trim() != "none")
            })
        };
        let theirs = set("controlpath");
        let proxied = set("proxyjump") || set("proxycommand");
        // All of it, not `%C`'s inputs: two aliases that differ in host keys,
        // identities or auth must not ride one login. The user never opted in.
        Some(Resolved {
            theirs,
            proxied,
            key: text,
        })
    }
}

struct Resolved {
    theirs: bool,
    proxied: bool,
    key: String,
}

/// Quoted for spaces; `%` doubled because ssh expands tokens in the path.
fn control_path_option(path: &Path) -> String {
    format!(
        "ControlPath=\"{}\"",
        path.display().to_string().replace('%', "%%")
    )
}

/// FNV-1a, which a 104-byte socket path has room for and `DefaultHasher`,
/// unstable across builds, is not.
fn fnv(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")[..12].to_string()
}

/// Runs `fut` unless the user cancels one of this run's questions first.
/// Dropping `fut` must kill its ssh (`kill_on_drop`) before `sign_in` goes:
/// the helper is held until then so ssh cannot send an empty password.
pub(crate) async fn unless_cancelled<F: Future>(
    sign_in: &mut Option<SignIn>,
    fut: F,
) -> Result<F::Output, Ended> {
    let Some(registered) = sign_in.as_mut() else {
        return Ok(fut.await);
    };
    tokio::select! {
        out = fut => Ok(out),
        ended = registered.ended() => Err(ended),
    }
}

/// The askpass run of this binary, when ssh started it; `main` exits with its code.
pub fn askpass_helper() -> Option<i32> {
    askpass::helper()
}
