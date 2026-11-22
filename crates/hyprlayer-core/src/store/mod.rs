mod goals;
mod library;
mod migrations;
mod projects;
mod reviews;
pub mod session_event_codec;
mod sessions;
mod workstreams;

pub use goals::*;
pub use library::*;
pub use projects::*;
pub use reviews::*;
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
        // sqlite_stat1 for ad-hoc joins on DB copies; the app's own PK lookups don't need it.
        conn.execute_batch("PRAGMA optimize;")?;
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

/// `(year, month, day)` of a UTC epoch day: Howard Hinnant's civil_from_days, era from 0000-03-01.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (era * 400 + yoe + i64::from(month <= 2), month, day)
}

// Howard Hinnant's days_from_civil, the inverse of civil_from_days.
pub fn epoch_ms_of_utc(s: &str) -> Option<f64> {
    let (date, time) = s.strip_suffix('Z')?.split_once('T')?;
    let num = |p: Option<&str>| p?.parse::<i64>().ok();
    let mut d = date.split('-');
    let (y, m, day) = (num(d.next())?, num(d.next())?, num(d.next())?);
    let mut t = time.split(':');
    let (h, min, sec) = (num(t.next())?, num(t.next())?, num(t.next())?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || d.next().is_some() {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3_600 + min * 60 + sec) * 1_000) as f64)
}

/// `YYYY-MM-DD` in UTC.
pub fn utc_date(epoch_secs: u64) -> String {
    let (year, month, day) = civil_from_days((epoch_secs / 86_400) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}
