//! The server's own log files. A daemon's stderr is a dead pipe once the app
//! that spawned it quits, and `/dev/null` under the SSH relay, so every event
//! the runtime emits goes to a daily file instead.

use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

/// The desktop passes its own log dir, so a local server logs beside it.
pub const LOG_DIR_ENV: &str = "HYPRLAYER_APP_LOG_DIR";
/// An `EnvFilter` directive, e.g. `debug` or `info,hyprlayer_core=debug`.
pub const LEVEL_ENV: &str = "HYPRLAYER_LOG";
/// What a spawned server writes outside tracing: panics that outrun the
/// hook, aborts, and the startup error the supervisor reads back. Named outside
/// both daily prefixes, which the appender prunes by without checking a date.
pub const STDERR_FILE: &str = "hyprlayer-stderr.log";

const DAYS_KEPT: usize = 7;
const STDERR_FILE_CAP: u64 = 5 * 1024 * 1024;

#[derive(Clone, Copy)]
pub enum Process {
    Serve,
    Relay,
}

impl Process {
    fn file_prefix(self) -> &'static str {
        match self {
            Process::Serve => "hyprlayer-server",
            Process::Relay => "hyprlayer-relay",
        }
    }
}

/// Tauri's `app_log_dir` rule for the profile `app_data_dir` names, so a
/// server started without the desktop still logs where the desktop would.
pub fn log_dir(app_data_dir: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os(LOG_DIR_ENV) {
        return PathBuf::from(dir);
    }
    let profile = app_data_dir.file_name().map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let platform = dirs::home_dir().map(|home| home.join("Library/Logs"));
    #[cfg(not(target_os = "macos"))]
    let platform = dirs::data_local_dir();
    match (platform, profile) {
        #[cfg(target_os = "macos")]
        (Some(base), Some(profile)) => base.join(profile),
        #[cfg(not(target_os = "macos"))]
        (Some(base), Some(profile)) => base.join(profile).join("logs"),
        _ => app_data_dir.join("logs"),
    }
}

/// Installs the global subscriber. Hold the guard until exit: dropping it is
/// what flushes the last lines, which `process::exit` would otherwise skip.
pub fn init(process: Process, dir: &Path) -> Option<WorkerGuard> {
    let filter = std::env::var(LEVEL_ENV)
        .ok()
        .and_then(|directive| EnvFilter::try_new(directive).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));

    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(process.file_prefix())
        .filename_suffix("log")
        .max_log_files(DAYS_KEPT)
        .build(dir);
    let (file_layer, guard) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer().with_writer(writer).with_ansi(false).boxed();
            (Some(layer), Some(guard))
        }
        Err(e) => {
            eprintln!("cannot log to {}: {e}", dir.display());
            (None, None)
        }
    };

    // A terminal or journald reads stderr; a spawned daemon's stderr is a file
    // the supervisor inspects, and tracing lines would crowd out its one line.
    let stderr = std::io::stderr();
    let stderr_layer =
        (stderr.is_terminal() || std::env::var_os("JOURNAL_STREAM").is_some()).then(|| {
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(stderr.is_terminal())
                .boxed()
        });

    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    if installed.is_ok() {
        install_panic_hook();
    }
    guard
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("{info}\n{backtrace}");
        previous(info);
    }));
}

/// Opens the spawned server's stderr file for appending, rotating it once it
/// passes a few MiB. Returns the handle and the offset this spawn starts at.
pub fn stderr_file(dir: &Path) -> std::io::Result<(File, u64)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(STDERR_FILE);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > STDERR_FILE_CAP) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let start = file.metadata()?.len();
    Ok((file, start))
}

/// What the spawned server wrote to its stderr file since `start`.
pub fn stderr_since(dir: &Path, start: u64) -> String {
    let mut text = String::new();
    if let Ok(mut file) = File::open(dir.join(STDERR_FILE)) {
        if file.seek(SeekFrom::Start(start)).is_ok() {
            let mut bytes = Vec::new();
            if file.read_to_end(&mut bytes).is_ok() {
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }
    text
}

/// The newest server log, the newest relay log, and the stderr file, each cut
/// to its last `max` bytes, as one text block for a support report.
pub fn tail_logs(dir: &Path, max: u64) -> String {
    let mut out = format!("log directory: {}\n", dir.display());
    let sources = [
        newest(dir, Process::Serve.file_prefix()),
        newest(dir, Process::Relay.file_prefix()),
        Some(dir.join(STDERR_FILE)).filter(|p| p.exists()),
    ];
    for path in sources.into_iter().flatten() {
        out.push_str(&format!("\n== {} ==\n", path.display()));
        out.push_str(&tail_file(&path, max));
    }
    out
}

/// Daily files are `<prefix>.<YYYY-MM-DD>.log`, so the name orders them.
fn newest(dir: &Path, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('.'))
                .and_then(|rest| rest.strip_suffix(".log"))
                .is_some_and(|date| {
                    !date.is_empty() && date.chars().all(|c| c.is_ascii_digit() || c == '-')
                })
        })
        .max()
        .map(|name| dir.join(name))
}

/// The last `max` bytes of `path`, or nothing if it cannot be read.
pub fn tail_file(path: &Path, max: u64) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(max)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Lets a key's first occurrence through, then at most one per `window` with
/// the count held back in between: the UI retries failing calls on a timer.
pub struct RepeatGate {
    window: Duration,
    seen: Mutex<Vec<(String, Instant, u32)>>,
}

impl RepeatGate {
    pub const fn new(window: Duration) -> Self {
        Self {
            window,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// `Some(held_back)` when this occurrence should be logged.
    pub fn admit(&self, key: &str) -> Option<u32> {
        let now = Instant::now();
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        match seen.iter_mut().find(|(k, ..)| k == key) {
            Some((_, last, held)) if now.duration_since(*last) < self.window => {
                *held += 1;
                None
            }
            Some((_, last, held)) => {
                *last = now;
                Some(std::mem::take(held))
            }
            None => {
                seen.push((key.to_string(), now, 0));
                Some(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stderr_file_is_outside_every_daily_prune() {
        for process in [Process::Serve, Process::Relay] {
            assert!(!STDERR_FILE.starts_with(process.file_prefix()));
        }
    }

    #[test]
    fn repeat_gate_admits_once_per_window_and_counts_the_rest() {
        let gate = RepeatGate::new(Duration::from_millis(50));
        assert_eq!(gate.admit("list_workstreams:internal"), Some(0));
        assert_eq!(gate.admit("list_workstreams:internal"), None);
        assert_eq!(gate.admit("list_workstreams:internal"), None);
        assert_eq!(gate.admit("git_status:internal"), Some(0));
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(gate.admit("list_workstreams:internal"), Some(2));
    }

    #[test]
    fn stderr_since_reads_only_what_this_spawn_appended() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STDERR_FILE), "an earlier run\n").unwrap();
        let (mut file, start) = stderr_file(dir.path()).unwrap();
        std::io::Write::write_all(&mut file, b"runtime_in_use: busy\n").unwrap();
        assert_eq!(stderr_since(dir.path(), start), "runtime_in_use: busy\n");
    }

    #[test]
    fn tail_logs_picks_the_newest_file_of_each_kind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hyprlayer-server.2026-09-19.log"), "new\n").unwrap();
        std::fs::write(dir.path().join("hyprlayer-server.2026-09-18.log"), "old\n").unwrap();
        std::fs::write(dir.path().join(STDERR_FILE), "stderr\n").unwrap();
        std::fs::write(dir.path().join("hyprlayer-relay.2026-09-19.log"), "relay\n").unwrap();
        std::fs::write(dir.path().join("Hyprlayer.log"), "desktop\n").unwrap();

        let text = tail_logs(dir.path(), 1024);
        assert!(text.contains("new\n") && !text.contains("old\n"), "{text}");
        assert!(
            text.contains("relay\n") && text.contains("stderr\n"),
            "{text}"
        );
        assert_eq!(text.matches("new\n").count(), 1, "{text}");
        assert!(!text.contains("desktop"), "{text}");
    }
}
