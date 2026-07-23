//! Subprocess execution for one-shot external commands.
//!
//! Long-lived children (claude sessions) get their own driver; everything
//! else goes through [`Exec`]: absolute binary path, explicit cwd, args as
//! vectors (never a shell), captured output, timeout, typed errors.

pub mod binaries;
#[cfg(windows)]
mod job;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{CoreError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const STDERR_TAIL_CHARS: usize = 2000;

pub struct Exec {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
    /// Bytes to feed the child on stdin (then close it). `None` = no stdin
    /// (`/dev/null`). Use this for sensitive input that must not appear in argv,
    /// which is world-readable via process listing.
    stdin: Option<Vec<u8>>,
    /// Put the child in its own process group so a timeout SIGKILLs the whole
    /// group (the command plus anything it spawned), not just the direct child.
    /// Unix-only; a no-op on Windows.
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

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Run the child in a fresh process group so a timeout kills the whole group
    /// (e.g. a script that backgrounded a server), not just the direct child.
    /// Unix-only; a no-op on Windows.
    pub fn own_process_group(mut self) -> Self {
        self.own_process_group = true;
        self
    }

    /// Feed `data` to the child on stdin, then close it (EOF). Prefer this over
    /// [`Exec::arg`] for secrets or user content — argv is visible to any local
    /// process listing, stdin is not.
    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    /// Run to completion, capturing output. Fails on non-zero exit.
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

    /// Run to completion, capturing output regardless of exit status.
    pub async fn run_unchecked(&self) -> Result<ExecOutput> {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(if self.stdin.is_some() {
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
        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        #[cfg(unix)]
        if self.own_process_group {
            // New group with pgid == child pid, so a timeout can killpg the whole
            // tree (see the timeout branch). `process_group` is inherent on tokio's
            // Command on Unix.
            cmd.process_group(0);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::BinaryMissing {
                    name: program_name(&self.program_display()),
                }
            } else {
                CoreError::Io(e)
            }
        })?;

        let mut stdout_pipe = child.stdout.take().expect("stdout piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");
        // Take the stdin pipe (present only when `self.stdin` is set); writing it
        // lives inside the `join!` below so a child that blocks on stdout can't
        // deadlock us, and the pipe is dropped afterwards to signal EOF.
        let mut stdin_pipe = child.stdin.take();
        let stdin_data = self.stdin.clone();
        // Captured before `work` borrows `child`, so the timeout branch can kill
        // the child's whole process group.
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
                    // `pipe` drops here → child sees EOF on stdin.
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
                // `kill_on_drop` only kills the direct child; when it's in its own
                // group, SIGKILL the whole group so spawned children die too.
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

/// A long-lived child plus whatever the OS needs to guarantee its whole tree
/// goes with it. Hold it for as long as the child should live; drop it to tear
/// the child down.
pub struct SupervisedChild {
    child: tokio::process::Child,
    /// Dropping this closes the job handle, which is what kills the tree.
    #[cfg(windows)]
    _job: Option<job::JobObject>,
}

impl SupervisedChild {
    /// The pid we spawned. On Windows this may be a `cmd.exe` wrapper rather
    /// than the program itself — do not kill it directly, drop `self`.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }
}

/// Spawn a long-lived child so that killing us kills everything it started.
///
/// Windows needs a job object: a resolved CLI is often an npm `.cmd` shim, which
/// `std::process::Command` runs through `cmd.exe`, so the pid we hold is the
/// wrapper and `kill_on_drop` alone leaves the real program running (see
/// [`job`]). The job also covers the routes where destructors never run —
/// `process::exit`, a panic, `TerminateProcess` — because the kernel closes our
/// handle regardless.
///
/// Unix needs nothing extra: a script bin is exec'd in place rather than wrapped,
/// so the pid we hold *is* the program and `kill_on_drop` reaches it.
///
/// A job that cannot be created or joined is logged and skipped rather than
/// failing the spawn: degrading to the old behaviour beats refusing to start.
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

    /// With `own_process_group()`, a timeout must SIGKILL the whole group — a
    /// child the command backgrounded must not outlive it.
    #[tokio::test]
    async fn timeout_kills_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        // Background a long sleep, record its pid, then hang so the timeout fires.
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
