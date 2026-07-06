//! App-local persistence (SQLite via rusqlite, bundled): project registry,
//! workstreams, and session records. Documents, comments, and git state are
//! always derived from their sources of truth, never stored here.

mod migrations;
mod projects;
mod sessions;
mod workstreams;

pub use projects::*;
pub use sessions::*;
pub use workstreams::*;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::Result;

/// Connection handle. All access is synchronous behind a Mutex — traffic is
/// tiny (single user, single window); callers wrap calls in spawn_blocking.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Under WAL, NORMAL is the standard crash-safe setting: it drops the
        // per-commit fsync (only a power-loss window remains, not a process-crash
        // one), which matters because the session forwarder appends an event row
        // per block during a live turn. A short busy_timeout lets a reader wait
        // out a writer instead of erroring under the single shared connection.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5_000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        Ok(f(&conn)?)
    }
}

/// Milliseconds since the unix epoch as f64 (the IPC-safe number type).
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}
