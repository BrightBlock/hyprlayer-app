use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{params, params_from_iter, OptionalExtension};

use super::{now_ms, Db};
use crate::agent::Harness;
use crate::error::Result;
use crate::index::{DocAgent, DocMeta};

const BASELINE_KEY: &str = "read_baseline:";
const AGENT_BACKFILL_KEY: &str = "agent_backfill_rowid";
/// `ensure_not_stale`'s margin: an f64 millisecond round trip, not an edit.
const MTIME_TOLERANCE_MS: f64 = 0.5;

/// Every doc's read state, loaded once per query.
pub struct ReadState {
    /// Root path → when it was first scanned; longest root first.
    baselines: Vec<(String, f64)>,
    rows: HashMap<String, ReadRow>,
}

struct ReadRow {
    read_mtime_ms: f64,
    read_hash: u64,
    read_size: f64,
    unread: bool,
}

impl ReadState {
    /// A newer mtime alone is not news: a sync rebase rewrites mtimes over the
    /// same bytes. No baseline for the doc's root yet means nothing is new.
    pub fn is_unread(&self, meta: &DocMeta, content_hash: u64) -> bool {
        if let Some(row) = self.rows.get(&meta.id) {
            return row.unread
                || (meta.mtime_ms > row.read_mtime_ms + MTIME_TOLERANCE_MS
                    && (content_hash, meta.size_bytes) != (row.read_hash, row.read_size));
        }
        let path = Path::new(&meta.id);
        self.baselines
            .iter()
            .find(|(root, _)| path.starts_with(root))
            .is_some_and(|(_, at)| meta.mtime_ms > at + MTIME_TOLERANCE_MS)
    }
}

impl Db {
    pub fn doc_read_state(&self) -> Result<ReadState> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT key, value FROM library_state")?;
            let mut baselines: Vec<(String, f64)> = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))?
                .filter_map(|row| match row {
                    Ok((key, at)) => key
                        .strip_prefix(BASELINE_KEY)
                        .map(|root| Ok((root.to_string(), at))),
                    Err(e) => Some(Err(e)),
                })
                .collect::<rusqlite::Result<_>>()?;
            baselines.sort_by_key(|(root, _)| std::cmp::Reverse(root.len()));

            let mut stmt = conn.prepare(
                "SELECT doc_id, read_mtime_ms, read_hash, read_size, unread FROM doc_reads",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    let read_hash: i64 = row.get(2)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        ReadRow {
                            read_mtime_ms: row.get(1)?,
                            read_hash: read_hash as u64,
                            read_size: row.get(3)?,
                            unread: row.get(4)?,
                        },
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(ReadState { baselines, rows })
        })
    }

    /// The hash is stored bit-cast to `i64`: SQLite integers are signed.
    pub fn mark_doc_read(
        &self,
        doc_id: &str,
        mtime_ms: f64,
        content_hash: u64,
        size: f64,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO doc_reads (doc_id, read_mtime_ms, read_hash, read_size, unread, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)
                 ON CONFLICT(doc_id) DO UPDATE SET
                     read_mtime_ms = excluded.read_mtime_ms, read_hash = excluded.read_hash,
                     read_size = excluded.read_size, unread = 0, updated_at = excluded.updated_at",
                params![doc_id, mtime_ms, content_hash as i64, size, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn mark_doc_unread(&self, doc_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO doc_reads (doc_id, read_mtime_ms, unread, updated_at)
                 VALUES (?1, 0, 1, ?2)
                 ON CONFLICT(doc_id) DO UPDATE SET unread = 1, updated_at = excluded.updated_at",
                params![doc_id, now_ms()],
            )?;
            Ok(())
        })
    }

    /// The first stamp wins, so a root's corpus reads as seen from its first scan on.
    pub fn ensure_read_baseline(&self, root_path: &str, at_ms: f64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO library_state (key, value) VALUES (?1, ?2)",
                params![format!("{BASELINE_KEY}{root_path}"), at_ms],
            )?;
            Ok(())
        })
    }
}

pub struct DocAgentRow {
    pub doc_id: String,
    pub session_id: String,
    pub harness: Harness,
    pub workstream_id: String,
    pub at_ms: f64,
}

/// A stored `Write`/`Edit`/`MultiEdit` tool use, its paths extracted in SQL.
pub struct ToolWriteRow {
    pub rowid: i64,
    pub session_id: String,
    pub harness: Harness,
    pub cwd: String,
    pub workstream_id: String,
    pub tool_use_id: String,
    pub paths: Vec<String>,
    pub created_at: f64,
}

/// A normalized write tool's `file_path` plus every `edits[].file_path` (codex `MultiEdit`).
pub(crate) fn write_tool_paths(
    file_path: Option<&str>,
    edits: &[serde_json::Value],
) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let edit_paths = edits
        .iter()
        .filter_map(|e| e.get("file_path").and_then(serde_json::Value::as_str));
    for path in file_path.into_iter().chain(edit_paths) {
        if !path.is_empty() && !paths.iter().any(|p| p == path) {
            paths.push(path.to_string());
        }
    }
    paths
}

impl Db {
    /// True when the row is new or its time span widened.
    pub fn record_doc_agent(&self, row: &DocAgentRow) -> Result<bool> {
        self.with(|conn| {
            let changed = conn.execute(
                "INSERT INTO doc_agents (doc_id, session_id, harness, workstream_id, first_at, last_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(doc_id, session_id) DO UPDATE SET
                     first_at = min(first_at, excluded.first_at),
                     last_at = max(last_at, excluded.last_at)
                 WHERE excluded.first_at < first_at OR excluded.last_at > last_at",
                params![
                    row.doc_id,
                    row.session_id,
                    row.harness.as_str(),
                    row.workstream_id,
                    row.at_ms
                ],
            )?;
            Ok(changed > 0)
        })
    }

    /// The newest attribution per doc; an archived or missing workstream has no project.
    pub fn doc_agents(&self) -> Result<HashMap<String, DocAgent>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT a.doc_id, a.harness, a.session_id, a.workstream_id, a.last_at, w.project_path
                   FROM doc_agents a
                   LEFT JOIN workstreams w ON w.id = a.workstream_id AND w.status = 'active'
                  ORDER BY a.doc_id, a.last_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    DocAgent {
                        harness: Harness::parse(&row.get::<_, String>(1)?),
                        session_id: row.get(2)?,
                        workstream_id: row.get(3)?,
                        at_ms: row.get(4)?,
                        project_path: row.get(5)?,
                    },
                ))
            })?;
            let mut newest = HashMap::new();
            for row in rows {
                let (doc_id, agent) = row?;
                newest.entry(doc_id).or_insert(agent);
            }
            Ok(newest)
        })
    }

    /// One backfill window, `(after_rowid, upto_rowid]`. Never selects a tool's
    /// raw input: a `Write` carries the whole file.
    pub fn write_tool_uses_between(
        &self,
        after_rowid: i64,
        upto_rowid: i64,
    ) -> Result<Vec<ToolWriteRow>> {
        self.with(|conn| {
            // One malformed input would otherwise fail this window on every start.
            let mut stmt = conn.prepare(
                "SELECT rowid, session_id, harness, cwd, workstream_id, tool_use_id,
                        CASE WHEN json_valid(input) THEN json_extract(input, '$.file_path') END,
                        CASE WHEN json_valid(input) THEN json_extract(input, '$.edits') END,
                        created_at
                   FROM (SELECT e.rowid AS rowid, e.session_id, s.harness, s.cwd, s.workstream_id,
                                json_extract(e.payload, '$.id') AS tool_use_id,
                                json_extract(e.payload, '$.input') AS input, e.created_at
                           FROM session_events e
                           JOIN sessions s ON s.id = e.session_id
                          WHERE e.rowid > ?1 AND e.rowid <= ?2 AND e.kind = 'toolUse'
                            AND json_extract(e.payload, '$.name') IN ('Write', 'Edit', 'MultiEdit'))",
            )?;
            let rows = stmt.query_map(params![after_rowid, upto_rowid], |row| {
                let file_path: Option<String> = row.get(6)?;
                let edits: Vec<serde_json::Value> = row
                    .get::<_, Option<String>>(7)?
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default();
                Ok(ToolWriteRow {
                    rowid: row.get(0)?,
                    session_id: row.get(1)?,
                    harness: Harness::parse(&row.get::<_, String>(2)?),
                    cwd: row.get(3)?,
                    workstream_id: row.get(4)?,
                    tool_use_id: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    paths: write_tool_paths(file_path.as_deref(), &edits),
                    created_at: row.get(8)?,
                })
            })?;
            rows.collect()
        })
    }

    /// `(session_id, tool_use_id)` of successful results; a result always follows its use.
    pub fn ok_tool_result_ids(
        &self,
        session_ids: &[String],
        after_rowid: i64,
    ) -> Result<HashSet<(String, String)>> {
        if session_ids.is_empty() {
            return Ok(HashSet::new());
        }
        self.with(|conn| {
            let placeholders = vec!["?"; session_ids.len()].join(", ");
            let mut stmt = conn.prepare(&format!(
                "SELECT session_id, json_extract(payload, '$.toolUseId')
                   FROM session_events
                  WHERE rowid > ? AND kind = 'toolResult'
                    AND json_extract(payload, '$.isError') = 0
                    AND session_id IN ({placeholders})"
            ))?;
            let args = std::iter::once(rusqlite::types::Value::Integer(after_rowid)).chain(
                session_ids
                    .iter()
                    .map(|id| rusqlite::types::Value::Text(id.clone())),
            );
            let rows = stmt.query_map(params_from_iter(args), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            let mut ok = HashSet::new();
            for row in rows {
                if let (session_id, Some(tool_use_id)) = row? {
                    ok.insert((session_id, tool_use_id));
                }
            }
            Ok(ok)
        })
    }

    pub fn max_session_event_rowid(&self) -> Result<i64> {
        self.with(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(rowid), 0) FROM session_events",
                [],
                |row| row.get(0),
            )
        })
    }

    pub fn agent_backfill_mark(&self) -> Result<i64> {
        self.with(|conn| {
            let mark: Option<f64> = conn
                .query_row(
                    "SELECT value FROM library_state WHERE key = ?1",
                    params![AGENT_BACKFILL_KEY],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(mark.map_or(0, |m| m as i64))
        })
    }

    pub fn set_agent_backfill_mark(&self, rowid: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO library_state (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![AGENT_BACKFILL_KEY, rowid as f64],
            )?;
            Ok(())
        })
    }
}
