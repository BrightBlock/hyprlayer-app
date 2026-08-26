use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::agent::Harness;
use crate::claude::SessionState;
use crate::error::Result;
use crate::pipeline::Stage;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workstream_id: String,
    pub stage: Stage,
    pub harness: Harness,
    pub claude_session_id: Option<String>,
    pub cwd: String,
    pub state: SessionState,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub exit_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub seq: f64,
    pub kind: String,
    pub payload: String,
    pub created_at: f64,
}

pub struct NewSession {
    pub id: String,
    pub workstream_id: String,
    pub stage: Stage,
    pub harness: Harness,
    pub cwd: String,
}

const COLS: &str = "id, workstream_id, stage, harness, claude_session_id, cwd, state, model, \
                    cost_usd, started_at, ended_at, exit_summary";

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get("id")?,
        workstream_id: row.get("workstream_id")?,
        stage: Stage::parse(&row.get::<_, String>("stage")?),
        harness: Harness::parse(&row.get::<_, String>("harness")?),
        claude_session_id: row.get("claude_session_id")?,
        cwd: row.get("cwd")?,
        state: SessionState::parse(&row.get::<_, String>("state")?),
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
            harness: new.harness,
            claude_session_id: None,
            cwd: new.cwd,
            state: SessionState::Starting,
            model: None,
            cost_usd: None,
            started_at: now_ms(),
            ended_at: None,
            exit_summary: None,
        };
        self.with(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, workstream_id, stage, harness, claude_session_id, cwd,
                                       state, model, cost_usd, started_at, ended_at, exit_summary)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    session.id,
                    session.workstream_id,
                    session.stage.as_str(),
                    session.harness.as_str(),
                    session.claude_session_id,
                    session.cwd,
                    session.state.as_str(),
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

    pub fn set_session_state(&self, id: &str, state: SessionState) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET state = ?2 WHERE id = ?1",
                params![id, state.as_str()],
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

    pub fn finish_session(
        &self,
        id: &str,
        state: SessionState,
        exit_summary: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET state = ?2, ended_at = ?3, exit_summary = ?4 WHERE id = ?1",
                params![id, state.as_str(), now_ms(), exit_summary],
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
            let mut stmt = conn.prepare_cached(
                "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                 VALUES (?1,?2,?3,?4,?5)",
            )?;
            stmt.execute(params![session_id, seq, kind, payload, now_ms()])?;
            Ok(())
        })
    }

    pub fn next_session_seq(&self, session_id: &str) -> Result<i64> {
        self.with(|conn| {
            let max: Option<i64> = conn.query_row(
                "SELECT MAX(seq) FROM session_events WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(max.map(|m| m + 1).unwrap_or(0))
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

    pub fn nth_user_message_seq(&self, session_id: &str, n: usize) -> Result<Option<i64>> {
        if n == 0 {
            return Ok(None);
        }
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT seq FROM session_events
                 WHERE session_id = ?1 AND kind = 'userMessage'
                 ORDER BY seq
                 LIMIT 1 OFFSET ?2",
            )?;
            let mut rows = stmt.query(params![session_id, (n - 1) as i64])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn count_user_messages(&self, session_id: &str) -> Result<usize> {
        self.with(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_events
                 WHERE session_id = ?1 AND kind = 'userMessage'",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(n.max(0) as usize)
        })
    }

    pub fn cut_session_events_from(
        &self,
        session_id: &str,
        cutoff: i64,
    ) -> Result<Vec<StoredEvent>> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let removed = {
                let mut stmt = tx.prepare(
                    "SELECT seq, kind, payload, created_at FROM session_events
                     WHERE session_id = ?1 AND seq >= ?2 ORDER BY seq",
                )?;
                let rows = stmt.query_map(params![session_id, cutoff], |row| {
                    Ok(StoredEvent {
                        seq: row.get::<_, i64>("seq")? as f64,
                        kind: row.get("kind")?,
                        payload: row.get("payload")?,
                        created_at: row.get("created_at")?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            tx.execute(
                "DELETE FROM session_events WHERE session_id = ?1 AND seq >= ?2",
                params![session_id, cutoff],
            )?;
            tx.commit()?;
            Ok(removed)
        })
    }

    pub fn restore_session_events(&self, session_id: &str, events: &[StoredEvent]) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut insert = tx.prepare(
                    "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for e in events {
                    insert.execute(params![
                        session_id,
                        e.seq as i64,
                        e.kind,
                        e.payload,
                        e.created_at
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

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
    use crate::store::workstreams::WorktreeMode;

    #[test]
    fn session_lifecycle_persists_and_replays() {
        let db = Db::open_in_memory().unwrap();
        db.insert_workstream(crate::store::NewWorkstream {
            project_path: "/p".into(),
            title: "t".into(),
            slug: "t-1".into(),
            branch: "hyprlayer/t-1".into(),
            worktree_path: "/wt".into(),
            base_branch: "main".into(),
            worktree_mode: WorktreeMode::Isolated,
        })
        .unwrap();
        let ws = db.list_active_workstreams().unwrap().pop().unwrap();

        let session = db
            .insert_session(NewSession {
                id: "s1".into(),
                workstream_id: ws.id.clone(),
                stage: Stage::Research,
                harness: Harness::Claude,
                cwd: "/wt".into(),
            })
            .unwrap();
        assert_eq!(session.state, SessionState::Starting);

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
        db.finish_session("s1", SessionState::Completed, Some("done"))
            .unwrap();

        let reloaded = db.get_session("s1").unwrap().unwrap();
        assert_eq!(reloaded.claude_session_id.as_deref(), Some("claude-abc"));
        assert_eq!(reloaded.state, SessionState::Completed);
        assert_eq!(reloaded.cost_usd, Some(0.05));
        assert!(reloaded.ended_at.is_some());

        let events = db.session_events("s1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 0.0);
        assert_eq!(events[1].kind, "assistantText");

        assert_eq!(db.list_sessions(&ws.id).unwrap().len(), 1);
    }

    fn seed_dialog(db: &Db, session_id: &str) {
        db.insert_workstream(crate::store::NewWorkstream {
            project_path: "/p".into(),
            title: "t".into(),
            slug: format!("t-{session_id}"),
            branch: format!("hyprlayer/t-{session_id}"),
            worktree_path: "/wt".into(),
            base_branch: "main".into(),
            worktree_mode: WorktreeMode::Isolated,
        })
        .unwrap();
        let ws = db.list_active_workstreams().unwrap().pop().unwrap();
        db.insert_session(NewSession {
            id: session_id.into(),
            workstream_id: ws.id,
            stage: Stage::General,
            harness: Harness::Claude,
            cwd: "/wt".into(),
        })
        .unwrap();
        for (seq, kind) in [
            (0, "init"),
            (1, "userMessage"),
            (2, "assistantText"),
            (3, "userMessage"),
            (4, "assistantText"),
            (5, "userMessage"),
            (6, "assistantText"),
        ] {
            db.append_session_event(session_id, seq, kind, &format!(r#"{{"kind":"{kind}"}}"#))
                .unwrap();
        }
    }

    #[test]
    fn nth_user_message_seq_finds_each_user_turn() {
        let db = Db::open_in_memory().unwrap();
        seed_dialog(&db, "s1");
        assert_eq!(db.nth_user_message_seq("s1", 1).unwrap(), Some(1));
        assert_eq!(db.nth_user_message_seq("s1", 2).unwrap(), Some(3));
        assert_eq!(db.nth_user_message_seq("s1", 3).unwrap(), Some(5));
        assert_eq!(db.nth_user_message_seq("s1", 4).unwrap(), None);
    }

    #[test]
    fn cut_drops_everything_from_cutoff_then_restores_verbatim() {
        let db = Db::open_in_memory().unwrap();
        seed_dialog(&db, "s1");
        let kinds = |db: &Db| -> Vec<(i64, String)> {
            db.session_events("s1")
                .unwrap()
                .into_iter()
                .map(|e| (e.seq as i64, e.kind))
                .collect()
        };
        let removed = db.cut_session_events_from("s1", 3).unwrap();
        assert_eq!(removed.len(), 4);
        assert_eq!(removed[0].seq as i64, 3);
        assert_eq!(
            kinds(&db),
            vec![
                (0, "init".into()),
                (1, "userMessage".into()),
                (2, "assistantText".into()),
            ],
        );
        db.restore_session_events("s1", &removed).unwrap();
        assert_eq!(
            kinds(&db),
            vec![
                (0, "init".into()),
                (1, "userMessage".into()),
                (2, "assistantText".into()),
                (3, "userMessage".into()),
                (4, "assistantText".into()),
                (5, "userMessage".into()),
                (6, "assistantText".into()),
            ],
        );
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
