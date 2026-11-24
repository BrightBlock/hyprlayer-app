use hyprlayer_models::{AgentCaller, PermissionMode, PtyLaunchSession};
use rusqlite::{params, OptionalExtension, Row};

use super::Db;
use crate::error::Result;

const COLUMNS: &str = "terminal_id, workstream_id, engine_id, launch, spawned_by, depth, mode, \
                       provider_session_id, title, working, resumes";

/// A server-launched PTY agent as the next server needs it to resume one in place.
#[derive(Debug, Clone, PartialEq)]
pub struct PtyAgentRow {
    pub terminal_id: String,
    pub workstream_id: String,
    pub engine_id: String,
    /// The launch's settings; never a session id, a resume or a first prompt.
    pub launch: PtyLaunchSession,
    pub spawned_by: Option<AgentCaller>,
    pub depth: u32,
    /// The mode its identity was minted at, for one an agent spawned.
    pub mode: Option<PermissionMode>,
    pub provider_session_id: Option<String>,
    pub title: Option<String>,
    /// Mid-turn when last heard from, or resumed and still owed its nudge.
    pub working: bool,
    /// Restart resumes since it last finished a turn.
    pub resumes: u32,
}

/// What a live agent's change writes; `None` leaves the stored value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PtyAgentSync {
    pub provider_session_id: Option<String>,
    pub title: Option<String>,
    pub working: bool,
    pub turn_done: bool,
}

fn row(r: &Row) -> rusqlite::Result<PtyAgentRow> {
    let launch: String = r.get(3)?;
    let spawned_by: Option<String> = r.get(4)?;
    let mode: Option<String> = r.get(6)?;
    Ok(PtyAgentRow {
        terminal_id: r.get(0)?,
        workstream_id: r.get(1)?,
        engine_id: r.get(2)?,
        launch: serde_json::from_str(&launch).unwrap_or_default(),
        spawned_by: spawned_by.and_then(|s| serde_json::from_str(&s).ok()),
        depth: r.get(5)?,
        mode: mode.as_deref().and_then(PermissionMode::try_parse),
        provider_session_id: r.get(7)?,
        title: r.get(8)?,
        working: r.get(9)?,
        resumes: r.get(10)?,
    })
}

impl Db {
    /// A launch, or a resume of the same terminal, which keeps its turn state and resume count.
    pub fn upsert_pty_agent(&self, agent: &PtyAgentRow) -> Result<()> {
        let launch = serde_json::to_string(&PtyLaunchSession {
            session_id: None,
            resume: false,
            first_prompt: None,
            ..agent.launch.clone()
        })
        .unwrap_or_default();
        let spawned_by = agent
            .spawned_by
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        self.with(|conn| {
            conn.execute(
                "INSERT INTO pty_agents (terminal_id, workstream_id, engine_id, launch, spawned_by,
                                         depth, mode, provider_session_id, title, working, resumes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(terminal_id) DO UPDATE SET
                    launch = excluded.launch,
                    provider_session_id = COALESCE(excluded.provider_session_id, provider_session_id)",
                params![
                    agent.terminal_id,
                    agent.workstream_id,
                    agent.engine_id,
                    launch,
                    spawned_by,
                    agent.depth,
                    agent.mode.map(PermissionMode::as_str),
                    agent.provider_session_id,
                    agent.title,
                    agent.working,
                    agent.resumes,
                ],
            )?;
            Ok(())
        })
    }

    pub fn sync_pty_agent(&self, terminal_id: &str, sync: &PtyAgentSync) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE pty_agents SET
                    provider_session_id = COALESCE(?2, provider_session_id),
                    title = COALESCE(?3, title),
                    working = ?4,
                    resumes = CASE WHEN ?5 THEN 0 ELSE resumes END
                 WHERE terminal_id = ?1",
                params![
                    terminal_id,
                    sync.provider_session_id,
                    sync.title,
                    sync.working,
                    sync.turn_done,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_pty_agent(&self, terminal_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM pty_agents WHERE terminal_id = ?1",
                params![terminal_id],
            )?;
            Ok(())
        })
    }

    pub fn pty_agent(&self, terminal_id: &str) -> Result<Option<PtyAgentRow>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM pty_agents WHERE terminal_id = ?1"),
                params![terminal_id],
                row,
            )
            .optional()
        })
    }

    /// Shallowest first, so a spawner resumes before the agents it spawned.
    pub fn pty_agents(&self) -> Result<Vec<PtyAgentRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM pty_agents ORDER BY depth, terminal_id"
            ))?;
            let rows = stmt.query_map([], row)?;
            rows.collect()
        })
    }

    /// The agent holding a conversation in a workstream, so a second launch resuming it finds it.
    pub fn pty_agent_holding(
        &self,
        workstream_id: &str,
        engine_id: &str,
        provider_session_id: &str,
    ) -> Result<Option<PtyAgentRow>> {
        self.with(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM pty_agents
                     WHERE workstream_id = ?1 AND engine_id = ?2 AND provider_session_id = ?3
                     ORDER BY terminal_id LIMIT 1"
                ),
                params![workstream_id, engine_id, provider_session_id],
                row,
            )
            .optional()
        })
    }

    /// Counted before the launch, so a server that dies during it still counts the try.
    pub fn record_pty_resume(&self, terminal_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE pty_agents SET resumes = resumes + 1 WHERE terminal_id = ?1",
                params![terminal_id],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Db {
        let db = Db::open_in_memory().unwrap();
        // No workstream row: the cascade is the schema's concern, not these.
        db.with(|conn| conn.execute_batch("PRAGMA foreign_keys = OFF;"))
            .unwrap();
        db
    }

    fn agent(id: &str) -> PtyAgentRow {
        PtyAgentRow {
            terminal_id: id.into(),
            workstream_id: "ws".into(),
            engine_id: "claude-code".into(),
            launch: PtyLaunchSession {
                session_id: Some("ignored".into()),
                resume: true,
                bypass: true,
                model: Some("haiku".into()),
                ..Default::default()
            },
            spawned_by: Some(AgentCaller::Session {
                session_id: "lead".into(),
            }),
            depth: 1,
            mode: Some(PermissionMode::BypassPermissions),
            provider_session_id: Some("p1".into()),
            title: None,
            working: false,
            resumes: 0,
        }
    }

    #[test]
    fn a_row_keeps_its_launch_but_never_a_resume_or_session_id() {
        let db = seeded();
        db.upsert_pty_agent(&agent("t1")).unwrap();
        let stored = db.pty_agent("t1").unwrap().unwrap();
        assert_eq!(stored.launch.session_id, None);
        assert!(!stored.launch.resume);
        assert!(stored.launch.bypass);
        assert_eq!(stored.launch.model.as_deref(), Some("haiku"));
        assert_eq!(stored.mode, Some(PermissionMode::BypassPermissions));
        assert_eq!(stored.spawned_by, agent("t1").spawned_by);
    }

    #[test]
    fn a_resume_keeps_the_turn_state_and_count_a_launch_would_reset() {
        let db = seeded();
        db.upsert_pty_agent(&agent("t1")).unwrap();
        db.sync_pty_agent(
            "t1",
            &PtyAgentSync {
                working: true,
                ..Default::default()
            },
        )
        .unwrap();
        db.record_pty_resume("t1").unwrap();
        db.upsert_pty_agent(&agent("t1")).unwrap();
        let stored = db.pty_agent("t1").unwrap().unwrap();
        assert!(stored.working);
        assert_eq!(stored.resumes, 1);
    }

    #[test]
    fn a_finished_turn_clears_the_resume_count() {
        let db = seeded();
        db.upsert_pty_agent(&agent("t1")).unwrap();
        db.record_pty_resume("t1").unwrap();
        db.record_pty_resume("t1").unwrap();
        db.sync_pty_agent(
            "t1",
            &PtyAgentSync {
                title: Some("Fix it".into()),
                turn_done: true,
                ..Default::default()
            },
        )
        .unwrap();
        let stored = db.pty_agent("t1").unwrap().unwrap();
        assert_eq!(stored.resumes, 0);
        assert!(!stored.working);
        assert_eq!(stored.title.as_deref(), Some("Fix it"));
        assert_eq!(stored.provider_session_id.as_deref(), Some("p1"));
    }

    #[test]
    fn the_holder_of_a_conversation_is_found_by_workstream_engine_and_id() {
        let db = seeded();
        db.upsert_pty_agent(&agent("t1")).unwrap();
        assert!(db
            .pty_agent_holding("ws", "claude-code", "p1")
            .unwrap()
            .is_some());
        assert!(db.pty_agent_holding("ws", "codex", "p1").unwrap().is_none());
        db.delete_pty_agent("t1").unwrap();
        assert!(db.pty_agent("t1").unwrap().is_none());
    }
}
