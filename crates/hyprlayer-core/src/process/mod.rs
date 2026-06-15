//! Subprocess execution for one-shot external commands.
//!
//! Long-lived children (claude sessions) get their own driver; everything
//! else goes through [`Exec`]: absolute binary path, explicit cwd, args as
//! vectors (never a shell), captured output, timeout, typed errors.

pub mod binaries;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;

use crate::error::{CoreError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const STDERR_TAIL_CHARS: usize = 2000;

pub struct Exec {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
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
            .stdin(Stdio::null())
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

        let work = async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let (out_res, err_res, status) = tokio::join!(
                stdout_pipe.read_to_end(&mut stdout),
                stderr_pipe.read_to_end(&mut stderr),
                child.wait(),
            );
            out_res?;
            err_res?;
            let status = status?;
            Ok::<ExecOutput, std::io::Error>(ExecOutput {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: status.code(),
            })
        };

        match tokio::time::timeout(self.timeout, work).await {
            Ok(result) => Ok(result?),
            Err(_) => Err(CoreError::ExecTimeout {
                program: program_name(&self.program_display()),
                seconds: self.timeout.as_secs(),
            }),
        }
    }

    fn program_display(&self) -> String {
        self.program.display().to_string()
    }
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
