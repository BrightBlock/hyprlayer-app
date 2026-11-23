use hyprlayer_models::{LeadQueueItem, LeadQueueState};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{now_ms, Db};
use crate::error::Result;

const COLUMNS: &str =
    "id, project_path, title, summary, state, workstream_id, position, created_at";

fn item(row: &Row) -> rusqlite::Result<LeadQueueItem> {
    Ok(LeadQueueItem {
        id: row.get(0)?,
        project_path: row.get(1)?,
        title: row.get(2)?,
        summary: row.get(3)?,
        state: LeadQueueState::parse(&row.get::<_, String>(4)?),
        workstream_id: row.get(5)?,
        position: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn get(conn: &Connection, project_path: &str, id: &str) -> rusqlite::Result<Option<LeadQueueItem>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM lead_queue WHERE project_path = ?1 AND id = ?2"),
        params![project_path, id],
        item,
    )
    .optional()
}

impl Db {
    pub fn lead_queue(&self, project_path: &str) -> Result<Vec<LeadQueueItem>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM lead_queue
                 WHERE project_path = ?1 AND state != 'done'
                 ORDER BY position, created_at"
            ))?;
            let rows = stmt.query_map(params![project_path], item)?;
            rows.collect()
        })
    }

    pub fn add_lead_queue_item(
        &self,
        project_path: &str,
        title: &str,
        summary: &str,
    ) -> Result<LeadQueueItem> {
        self.with(|conn| {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO lead_queue (id, project_path, title, summary, position, created_at)
                 VALUES (?1, ?2, ?3, ?4,
                         (SELECT COALESCE(MAX(position) + 1, 0) FROM lead_queue WHERE project_path = ?2),
                         ?5)",
                params![id, project_path, title, summary, now_ms()],
            )?;
            Ok(get(conn, project_path, &id)?.expect("the row just inserted"))
        })
    }

    pub fn set_lead_queue_state(
        &self,
        project_path: &str,
        id: &str,
        state: LeadQueueState,
        workstream_id: Option<&str>,
    ) -> Result<Option<LeadQueueItem>> {
        self.with(|conn| {
            conn.execute(
                "UPDATE lead_queue SET state = ?3, workstream_id = COALESCE(?4, workstream_id)
                 WHERE project_path = ?1 AND id = ?2",
                params![project_path, id, state.as_str(), workstream_id],
            )?;
            get(conn, project_path, id)
        })
    }

    pub fn lead_queue_item(&self, project_path: &str, id: &str) -> Result<Option<LeadQueueItem>> {
        self.with(|conn| get(conn, project_path, id))
    }

    pub fn remove_lead_queue_item(&self, project_path: &str, id: &str) -> Result<bool> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM lead_queue WHERE project_path = ?1 AND id = ?2",
                params![project_path, id],
            )
            .map(|n| n > 0)
        })
    }

    pub fn reorder_lead_queue(&self, project_path: &str, ids: &[String]) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let rest: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT id FROM lead_queue WHERE project_path = ?1 ORDER BY position, created_at",
                )?;
                let all = stmt.query_map(params![project_path], |r| r.get::<_, String>(0))?;
                all.collect::<rusqlite::Result<Vec<_>>>()?
                    .into_iter()
                    .filter(|id| !ids.contains(id))
                    .collect()
            };
            for (position, id) in ids.iter().chain(&rest).enumerate() {
                tx.execute(
                    "UPDATE lead_queue SET position = ?3 WHERE project_path = ?1 AND id = ?2",
                    params![project_path, id, position as u32],
                )?;
            }
            tx.commit()
        })
    }
}
