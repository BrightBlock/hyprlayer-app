//! The client's log file, `hyprlayer-native.log` beside the Tauri app's in the profile's log dir:
//! rotated at 5 MB with five files kept, UTC timestamps to the millisecond, as `logging.rs` writes.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use log::{LevelFilter, Log, Metadata, Record};

const FILE: &str = "hyprlayer-native";
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const FILES_KEPT: usize = 5;

struct Rotating {
    dir: PathBuf,
    file: Option<File>,
    written: u64,
}

impl Rotating {
    fn path(&self, n: usize) -> PathBuf {
        match n {
            0 => self.dir.join(format!("{FILE}.log")),
            n => self.dir.join(format!("{FILE}.{n}.log")),
        }
    }

    fn open(&mut self) -> Option<&mut File> {
        if self.file.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path(0))
                .ok()?;
            self.written = file.metadata().map(|m| m.len()).unwrap_or(0);
            self.file = Some(file);
        }
        self.file.as_mut()
    }

    fn rotate(&mut self) {
        self.file = None;
        for n in (0..FILES_KEPT - 1).rev() {
            let _ = std::fs::rename(self.path(n), self.path(n + 1));
        }
    }

    fn write(&mut self, line: &str) {
        if self.written + line.len() as u64 > MAX_FILE_BYTES {
            self.rotate();
        }
        if let Some(file) = self.open() {
            if file.write_all(line.as_bytes()).is_ok() {
                self.written += line.len() as u64;
            }
        }
    }
}

struct Logger {
    level: LevelFilter,
    out: Mutex<Rotating>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // zbus logs every D-Bus handshake (notify-rust) at info.
        let level = if metadata.target().starts_with("zbus") {
            self.level.min(LevelFilter::Warn)
        } else {
            self.level
        };
        metadata.level() <= level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let stamp = time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        );
        let now = time::OffsetDateTime::now_utc()
            .format(&stamp)
            .unwrap_or_default();
        let line = format!(
            "{now} {:<5} {}: {}\n",
            record.level(),
            record.target(),
            record.args()
        );
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        self.out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(&line);
    }

    fn flush(&self) {
        if let Some(file) = self
            .out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .file
            .as_mut()
        {
            let _ = file.flush();
        }
    }
}

/// The leading level of `HYPRLAYER_LOG`, which a server this client spawns also reads.
fn level() -> LevelFilter {
    std::env::var(hyprlayer_server::logging::LEVEL_ENV)
        .ok()
        .and_then(|directive| directive.split(',').next()?.trim().parse().ok())
        .unwrap_or(LevelFilter::Info)
}

pub fn init(dir: &Path) {
    let level = level();
    let logger = Logger {
        level,
        out: Mutex::new(Rotating {
            dir: dir.to_path_buf(),
            file: None,
            written: 0,
        }),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("{info}\n{}", std::backtrace::Backtrace::force_capture());
        previous(info);
    }));
}
