pub mod appimage;
pub mod binaries;
pub mod exit;
#[cfg(windows)]
mod job;
pub mod orphan;

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use crate::error::{CoreError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const BOUNDED_READ_CHUNK: usize = 8 * 1024;
const BOUNDED_DRAIN: Duration = Duration::from_millis(500);
const BOUNDED_REAP: Duration = Duration::from_secs(2);
const STDERR_TAIL_CHARS: usize = 2000;
/// Longer arguments are prompts or file bodies; the log gets their length.
const LOGGED_ARG_CHARS: usize = 200;
const LOGGED_STDERR_BYTES: usize = 500;

pub struct Exec {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
    stdin: Option<Vec<u8>>,
    own_process_group: bool,
    timeout: Duration,
}

#[derive(Debug)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl Exec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            stdin: None,
            own_process_group: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Bulk form of `env`, for merging a caller-resolved set (e.g. the
    /// per-attach overlay, ADR-0009 §7) over whatever this builder already has.
    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env
            .extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn own_process_group(mut self) -> Self {
        self.own_process_group = true;
        self
    }

    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    pub async fn run(self) -> Result<ExecOutput> {
        let output = self.run_unchecked().await?;
        if output.exit_code == Some(0) {
            Ok(output)
        } else {
            Err(CoreError::Exec {
                program: program_name(&self.program_display()),
                exit: output.exit_code,
                stderr_tail: tail(&output.stderr, STDERR_TAIL_CHARS),
            })
        }
    }

    pub async fn run_unchecked(&self) -> Result<ExecOutput> {
        let started = Instant::now();
        let ran = self.spawn_and_wait().await;
        let elapsed = started.elapsed();
        match &ran {
            Ok(out) if out.exit_code == Some(0) => {
                tracing::debug!("`{}` succeeded in {elapsed:?}", self.describe());
            }
            Ok(out) => tracing::debug!(
                "`{}` exited {:?} in {elapsed:?}: {}",
                self.describe(),
                out.exit_code,
                crate::redact::url_credentials(crate::redact::tail(
                    out.stderr.trim(),
                    LOGGED_STDERR_BYTES
                ))
            ),
            Err(CoreError::ExecTimeout { seconds, .. }) => {
                tracing::warn!(
                    "`{}` timed out after {seconds}s and was killed",
                    self.describe()
                );
            }
            Err(e) => tracing::debug!("`{}` could not run: {e}", self.describe()),
        }
        ran
    }

    /// stdout and stderr share one ring of `ring_bytes`; a timeout or cancel keeps what was read.
    /// The child leads its own process group, which dies with the run if its future is dropped.
    pub async fn run_bounded(&self, ring_bytes: usize, cancel: &Notify) -> Result<BoundedOutput> {
        // Enabled before the spawn, so a cancel signalled earlier starts nothing and a later one is never missed.
        let cancelled = cancel.notified();
        tokio::pin!(cancelled);
        if cancelled.as_mut().enable() {
            return Ok(BoundedOutput {
                tail: String::new(),
                exit_code: None,
                timed_out: false,
                cancelled: true,
            });
        }

        let mut child = self.spawn_child(true)?;
        let mut group = GroupGuard::new(child.id());
        let mut pipes = Pipes::new(&mut child, ring_bytes);
        let deadline = tokio::time::sleep(self.timeout);
        tokio::pin!(deadline);

        let ended = loop {
            tokio::select! {
                _ = pipes.pump(), if pipes.open() => {}
                status = child.wait() => break Ended::Exited(status),
                _ = &mut deadline => break Ended::TimedOut,
                _ = &mut cancelled => break Ended::Cancelled,
            }
        };
        let timed_out = matches!(ended, Ended::TimedOut);
        let was_cancelled = matches!(ended, Ended::Cancelled);
        let status = match ended {
            Ended::Exited(status) => Some(status.map_err(CoreError::Io)?),
            Ended::TimedOut | Ended::Cancelled => {
                group.kill();
                let _ = child.start_kill();
                tokio::time::timeout(BOUNDED_REAP, child.wait())
                    .await
                    .ok()
                    .and_then(|reaped| reaped.ok())
            }
        };
        // Reaped: the pid may be reused from here, so the guard must not fire on it.
        group.disarm();
        // A process that outlived the child can hold the pipes open, so the drain is bounded.
        let _ = tokio::time::timeout(BOUNDED_DRAIN, async { while pipes.pump().await {} }).await;

        if timed_out {
            tracing::warn!(
                "`{}` timed out after {}s and was killed",
                self.describe(),
                self.timeout.as_secs()
            );
        }
        Ok(BoundedOutput {
            tail: pipes.ring.into_tail(),
            exit_code: status.and_then(|status| status.code()),
            timed_out,
            cancelled: was_cancelled,
        })
    }

    /// Program, cwd and arguments for a log line, with credentials in URLs,
    /// values after key- or token-like flags, and long arguments masked.
    fn describe(&self) -> String {
        let mut out = program_name(&self.program_display());
        let mut secret_next = false;
        for arg in &self.args {
            let arg = arg.to_string_lossy();
            out.push(' ');
            let flag = arg.starts_with('-');
            let lower = arg.to_ascii_lowercase();
            let names_secret = ["key", "token", "secret", "password"]
                .iter()
                .any(|word| lower.contains(word));
            if secret_next && !flag {
                out.push_str("<redacted>");
            } else if let (true, true, Some((name, _))) = (flag, names_secret, arg.split_once('='))
            {
                out.push_str(name);
                out.push_str("=<redacted>");
            } else if arg.chars().count() > LOGGED_ARG_CHARS {
                out.push_str(&format!("<{} chars>", arg.chars().count()));
            } else {
                out.push_str(&crate::redact::url_credentials(&arg));
            }
            secret_next = flag && names_secret && !arg.contains('=');
        }
        if let Some(cwd) = &self.cwd {
            out.push_str(&format!(" (in {})", cwd.display()));
        }
        out
    }

    /// A bounded run takes no stdin and always leads its own process group, which is what it kills.
    fn spawn_child(&self, bounded: bool) -> Result<tokio::process::Child> {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(if self.stdin.is_some() && !bounded {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        appimage::scrub_tokio(&mut cmd);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        #[cfg(unix)]
        if self.own_process_group || bounded {
            cmd.process_group(0);
        }

        cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::BinaryMissing {
                    name: program_name(&self.program_display()),
                }
            } else {
                CoreError::Io(e)
            }
        })
    }

    async fn spawn_and_wait(&self) -> Result<ExecOutput> {
        let mut child = self.spawn_child(false)?;

        let mut stdout_pipe = child.stdout.take().expect("stdout piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");
        let mut stdin_pipe = child.stdin.take();
        let stdin_data = self.stdin.clone();
        #[cfg(unix)]
        let child_pid = child.id();

        let work = async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let write_stdin = async {
                if let Some(mut pipe) = stdin_pipe.take() {
                    if let Some(data) = &stdin_data {
                        pipe.write_all(data).await?;
                    }
                }
                Ok::<(), std::io::Error>(())
            };
            let (out_res, err_res, in_res, status) = tokio::join!(
                stdout_pipe.read_to_end(&mut stdout),
                stderr_pipe.read_to_end(&mut stderr),
                write_stdin,
                child.wait(),
            );
            out_res?;
            err_res?;
            in_res?;
            let status = status?;
            Ok::<ExecOutput, std::io::Error>(ExecOutput {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: status.code(),
            })
        };

        match tokio::time::timeout(self.timeout, work).await {
            Ok(result) => Ok(result?),
            Err(_) => {
                #[cfg(unix)]
                if self.own_process_group {
                    if let Some(pid) = child_pid {
                        unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
                    }
                }
                Err(CoreError::ExecTimeout {
                    program: program_name(&self.program_display()),
                    seconds: self.timeout.as_secs(),
                })
            }
        }
    }

    fn program_display(&self) -> String {
        self.program.display().to_string()
    }
}

#[derive(Debug)]
pub struct BoundedOutput {
    pub tail: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
}

enum Ended {
    Exited(std::io::Result<std::process::ExitStatus>),
    TimedOut,
    Cancelled,
}

/// Kills the child's whole process group when dropped while armed: `kill_on_drop` reaches only the child.
/// Elsewhere than unix there is no group, and only the child is killed.
struct GroupGuard {
    #[cfg(unix)]
    pgid: Option<u32>,
}

impl GroupGuard {
    fn new(pid: Option<u32>) -> Self {
        #[cfg(unix)]
        {
            Self { pgid: pid }
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            Self {}
        }
    }

    fn kill(&self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) };
        }
    }

    fn disarm(&mut self) {
        #[cfg(unix)]
        {
            self.pgid = None;
        }
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The last `cap` bytes written, whichever pipe they came from.
struct Ring {
    bytes: VecDeque<u8>,
    cap: usize,
    wrapped: bool,
}

impl Ring {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk);
        if self.bytes.len() > self.cap {
            self.bytes.drain(..self.bytes.len() - self.cap);
            self.wrapped = true;
        }
    }

    fn into_tail(mut self) -> String {
        let bytes = self.bytes.make_contiguous();
        // The cut may land inside a character; start at the next whole one.
        let skip = if self.wrapped {
            bytes.iter().take_while(|b| **b & 0xC0 == 0x80).count()
        } else {
            0
        };
        String::from_utf8_lossy(&bytes[skip..]).into_owned()
    }
}

/// A child's stdout and stderr, read into one ring until each closes.
struct Pipes {
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    out_chunk: Vec<u8>,
    err_chunk: Vec<u8>,
    ring: Ring,
}

impl Pipes {
    fn new(child: &mut tokio::process::Child, ring_bytes: usize) -> Self {
        Self {
            stdout: Some(child.stdout.take().expect("stdout piped")),
            stderr: Some(child.stderr.take().expect("stderr piped")),
            out_chunk: vec![0; BOUNDED_READ_CHUNK],
            err_chunk: vec![0; BOUNDED_READ_CHUNK],
            ring: Ring {
                bytes: VecDeque::new(),
                cap: ring_bytes,
                wrapped: false,
            },
        }
    }

    fn open(&self) -> bool {
        self.stdout.is_some() || self.stderr.is_some()
    }

    /// Takes the next chunk from whichever pipe has one; false once both are closed.
    async fn pump(&mut self) -> bool {
        if !self.open() {
            return false;
        }
        tokio::select! {
            read = read_chunk(&mut self.stdout, &mut self.out_chunk) => match read {
                Ok(n) if n > 0 => self.ring.push(&self.out_chunk[..n]),
                _ => self.stdout = None,
            },
            read = read_chunk(&mut self.stderr, &mut self.err_chunk) => match read {
                Ok(n) if n > 0 => self.ring.push(&self.err_chunk[..n]),
                _ => self.stderr = None,
            },
        }
        true
    }
}

/// Waits forever on a closed pipe, so `select!` only ever hears from the open ones.
async fn read_chunk<R: AsyncRead + Unpin>(
    pipe: &mut Option<R>,
    chunk: &mut [u8],
) -> std::io::Result<usize> {
    match pipe {
        Some(pipe) => pipe.read(chunk).await,
        None => std::future::pending().await,
    }
}

pub struct SupervisedChild {
    child: tokio::process::Child,
    #[cfg(windows)]
    _job: Option<job::JobObject>,
}

impl SupervisedChild {
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    pub fn kill_blocking(&self) {
        let Some(pid) = self.child.id() else { return };
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        #[cfg(windows)]
        let _ = pid;
    }
}

/// The mirror of `spawn_supervised`: a child that must *outlive* us.
///
/// `DETACHED_PROCESS` leaves the console but not an inherited job object, so a
/// kill-on-close job — OpenSSH makes one per session — takes the "orphan" down
/// with its parent. `CREATE_BREAKAWAY_FROM_JOB` leaves the job too, and a job
/// that forbids breakaway fails the spawn outright, hence the retry.
pub fn spawn_orphan(cmd: &mut tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    #[cfg(unix)]
    {
        cmd.process_group(0);
        cmd.spawn()
    }

    #[cfg(windows)]
    {
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const ERROR_ACCESS_DENIED: i32 = 5;

        cmd.creation_flags(DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB);
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                tracing::warn!("job forbids breakaway; the server will not outlive this process");
                cmd.creation_flags(DETACHED_PROCESS);
                cmd.spawn()
            }
            other => other,
        }
    }
}

pub fn spawn_supervised(cmd: &mut tokio::process::Command) -> std::io::Result<SupervisedChild> {
    cmd.kill_on_drop(true);

    #[cfg(windows)]
    {
        let job = match job::JobObject::kill_on_close() {
            Ok(job) => Some(job),
            Err(e) => {
                tracing::warn!("job object unavailable, child tree may outlive us: {e}");
                None
            }
        };
        let child = cmd.spawn()?;
        if let Some(j) = &job {
            if let Err(e) = j.adopt(&child) {
                tracing::warn!("could not join child to job, tree may outlive us: {e}");
            }
        }
        Ok(SupervisedChild { child, _job: job })
    }

    #[cfg(not(windows))]
    Ok(SupervisedChild {
        child: cmd.spawn()?,
    })
}

fn program_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

fn tail(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let skip = trimmed.chars().count() - max_chars;
        trimmed.chars().skip(skip).collect()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_logged_command_line_masks_secrets_and_long_arguments() {
        let prompt = "x".repeat(LOGGED_ARG_CHARS + 1);
        let line = Exec::new("/usr/bin/git")
            .args([
                "clone",
                "https://me:ghp_abc@github.com/a/b.git",
                "--depth=1",
            ])
            .args(["--api-key", "sk-live", "--token=abc", "-p"])
            .arg(&prompt)
            .cwd("/work")
            .describe();
        assert_eq!(
            line,
            format!(
                "git clone https://***@github.com/a/b.git --depth=1 --api-key <redacted> \
                 --token=<redacted> -p <{} chars> (in /work)",
                LOGGED_ARG_CHARS + 1
            )
        );
    }

    #[tokio::test]
    async fn timeout_kills_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("sleep 60 & echo $! > {}; sleep 60", pidfile.display());

        let res = Exec::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .own_process_group()
            .timeout(Duration::from_millis(500))
            .run_unchecked()
            .await;
        assert!(
            matches!(res, Err(CoreError::ExecTimeout { .. })),
            "expected timeout, got {res:?}"
        );

        let pid: i32 = std::fs::read_to_string(&pidfile)
            .expect("pidfile written")
            .trim()
            .parse()
            .expect("pid parses");
        let mut gone = false;
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(gone, "backgrounded child {pid} survived the timeout");
    }
}
