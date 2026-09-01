mod migrations;
mod projects;
pub mod session_event_codec;
mod sessions;
mod workstreams;

pub use projects::*;
pub use sessions::*;
pub use workstreams::*;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::Result;

pub struct Db {
    conn: Mutex<Connection>,
    #[cfg(any(test, feature = "test-support"))]
    session_event_gate: Mutex<Option<std::sync::Arc<sessions::SessionEventGateFn>>>,
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
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5_000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            #[cfg(any(test, feature = "test-support"))]
            session_event_gate: Mutex::new(None),
        })
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        Ok(f(&conn)?)
    }

    pub(crate) fn with_core<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        f(&conn)
    }
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}
