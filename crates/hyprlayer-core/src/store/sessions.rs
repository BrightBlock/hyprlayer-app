//! Persistence for agent sessions, their event logs (for attach/replay), and
//! the orphan-PID ledger used to reap processes a crash left behind.

use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workstream_id: String,
    pub stage: String,
    /// claude's own session id (set on the init event; drives `--resume`).
    pub claude_session_id: Option<String>,
    pub cwd: String,
    /// starting | running | awaiting_permission | awaiting_question | completed |
    /// interrupted | crashed | killed | spawn_failed
    pub state: String,
    pub model: Option<String>,
    /// Cumulative cost reported by the latest `result` event.
    pub cost_usd: Option<f64>,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub exit_summary: Option<String>,
}

/// One persisted event, for replay into a freshly-attached UI Channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    /// Monotonic per-session sequence (f64 for IPC; small integers in practice).
    pub seq: f64,
    pub kind: String,
    /// The full `SessionEvent` JSON.
    pub payload: String,
    pub created_at: f64,
}

pub struct NewSession {
    pub id: String,
    pub workstream_id: String,
    pub stage: String,
    pub cwd: String,
}

const COLS: &str = "id, workstream_id, stage, claude_session_id, cwd, state, model, \
                    cost_usd, started_at, ended_at, exit_summary";

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get("id")?,
        workstream_id: row.get("workstream_id")?,
        stage: row.get("stage")?,
        claude_session_id: row.get("claude_session_id")?,
        cwd: row.get("cwd")?,
        state: row.get("state")?,
        model: row.get("model")?,
        cost_usd: row.get("cost_usd")?,
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        exit_summary: row.get("exit_summary")?,
    })
}

impl Db {
    pub fn insert_session(&self, new: NewSession) -> Result<Session> {
        let session = Session {
            id: new.id,
            workstream_id: new.workstream_id,
            stage: new.stage,
            claude_session_id: None,
            cwd: new.cwd,
            state: "starting".into(),
            model: None,
            cost_usd: None,
            started_at: now_ms(),
            ended_at: None,
            exit_summary: None,
        };
        self.with(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, workstream_id, stage, claude_session_id, cwd, state,
                                       model, cost_usd, started_at, ended_at, exit_summary)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    session.id,
                    session.workstream_id,
                    session.stage,
                    session.claude_session_id,
                    session.cwd,
                    session.state,
                    session.model,
                    session.cost_usd,
                    session.started_at,
                    session.ended_at,
                    session.exit_summary,
                ],
            )?;
            Ok(())
        })?;
        Ok(session)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {COLS} FROM sessions WHERE id = ?1"),
                params![id],
                row_to_session,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    pub fn list_sessions(&self, workstream_id: &str) -> Result<Vec<Session>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM sessions WHERE workstream_id = ?1 ORDER BY started_at"
            ))?;
            let rows = stmt.query_map(params![workstream_id], row_to_session)?;
            rows.collect()
        })
    }

    /// Record claude's session id + model from the init event.
    pub fn mark_session_init(
        &self,
        id: &str,
        claude_session_id: &str,
        model: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET claude_session_id = ?2, model = ?3, state = 'running'
                 WHERE id = ?1",
                params![id, claude_session_id, model],
            )?;
            Ok(())
        })
    }

    pub fn set_session_state(&self, id: &str, state: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET state = ?2 WHERE id = ?1",
                params![id, state],
            )?;
            Ok(())
        })
    }

    pub fn set_session_cost(&self, id: &str, cost_usd: f64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET cost_usd = ?2 WHERE id = ?1",
                params![id, cost_usd],
            )?;
            Ok(())
        })
    }

    /// Terminal update: state + ended_at + a short exit summary.
    pub fn finish_session(&self, id: &str, state: &str, exit_summary: Option<&str>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET state = ?2, ended_at = ?3, exit_summary = ?4 WHERE id = ?1",
                params![id, state, now_ms(), exit_summary],
            )?;
            Ok(())
        })
    }

    pub fn append_session_event(
        &self,
        session_id: &str,
        seq: i64,
        kind: &str,
        payload: &str,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![session_id, seq, kind, payload, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn session_events(&self, session_id: &str) -> Result<Vec<StoredEvent>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT seq, kind, payload, created_at FROM session_events
                 WHERE session_id = ?1 ORDER BY seq",
            )?;
            let rows = stmt.query_map(params![session_id], |row| {
                Ok(StoredEvent {
                    seq: row.get::<_, i64>("seq")? as f64,
                    kind: row.get("kind")?,
                    payload: row.get("payload")?,
                    created_at: row.get("created_at")?,
                })
            })?;
            rows.collect()
        })
    }

    /* ----------------------------- Orphan PIDs ---------------------------- */

    pub fn record_orphan(&self, pid: i64, session_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO orphan_pids (pid, session_id, spawned_at)
                 VALUES (?1,?2,?3)",
                params![pid, session_id, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn clear_orphans_for(&self, session_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM orphan_pids WHERE session_id = ?1",
                params![session_id],
            )?;
            Ok(())
        })
    }

    /// All recorded orphan (pid, session_id) pairs — read at startup to reap.
    pub fn list_orphans(&self) -> Result<Vec<(i64, String)>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT pid, session_id FROM orphan_pids")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
    }

    pub fn clear_all_orphans(&self) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM orphan_pids", [])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lifecycle_persists_and_replays() {
        let db = Db::open_in_memory().unwrap();
        // A session needs a parent workstream row (FK).
        db.insert_workstream(crate::store::NewWorkstream {
            project_path: "/p".into(),
            title: "t".into(),
            slug: "t-1".into(),
            branch: "hyprlayer/t-1".into(),
            worktree_path: "/wt".into(),
            base_branch: "main".into(),
        })
        .unwrap();
        let ws = db.list_active_workstreams().unwrap().pop().unwrap();

        let session = db
            .insert_session(NewSession {
                id: "s1".into(),
                workstream_id: ws.id.clone(),
                stage: "research".into(),
                cwd: "/wt".into(),
            })
            .unwrap();
        assert_eq!(session.state, "starting");

        db.mark_session_init("s1", "claude-abc", Some("claude-opus-4-8"))
            .unwrap();
        db.append_session_event("s1", 0, "init", r#"{"kind":"init"}"#)
            .unwrap();
        db.append_session_event(
            "s1",
            1,
            "assistantText",
            r#"{"kind":"assistantText","text":"hi"}"#,
        )
        .unwrap();
        db.set_session_cost("s1", 0.05).unwrap();
        db.finish_session("s1", "completed", Some("done")).unwrap();

        let reloaded = db.get_session("s1").unwrap().unwrap();
        assert_eq!(reloaded.claude_session_id.as_deref(), Some("claude-abc"));
        assert_eq!(reloaded.state, "completed");
        assert_eq!(reloaded.cost_usd, Some(0.05));
        assert!(reloaded.ended_at.is_some());

        let events = db.session_events("s1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 0.0);
        assert_eq!(events[1].kind, "assistantText");

        assert_eq!(db.list_sessions(&ws.id).unwrap().len(), 1);
    }

    #[test]
    fn orphan_ledger_records_and_clears() {
        let db = Db::open_in_memory().unwrap();
        db.record_orphan(1234, "s1").unwrap();
        db.record_orphan(5678, "s2").unwrap();
        assert_eq!(db.list_orphans().unwrap().len(), 2);
        db.clear_orphans_for("s1").unwrap();
        assert_eq!(db.list_orphans().unwrap(), vec![(5678, "s2".to_string())]);
        db.clear_all_orphans().unwrap();
        assert!(db.list_orphans().unwrap().is_empty());
    }
}
