use rusqlite::{params, Row};

use super::{now_ms, Db};
use crate::error::Result;
use crate::pipeline::{pipeline_rank, Stage};
use crate::worktree::ArchiveScan;

pub use hyprlayer_models::workstream::{
    ObservedPr, PrState, Workstream, WorkstreamDoc, WorkstreamStatus, WorktreeMode,
};

pub struct NewWorkstream {
    pub project_path: String,
    pub title: String,
    pub slug: String,
    pub branch: String,
    pub worktree_path: String,
    pub base_branch: String,
    pub worktree_mode: WorktreeMode,
}

fn observed_pr(number: Option<u32>, state: Option<String>) -> Option<ObservedPr> {
    let state = PrState::parse(state.as_deref()?)?;
    Some(ObservedPr {
        number: number?,
        state,
    })
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<Workstream> {
    let worktree_mode: String = row.get("worktree_mode")?;
    let stage: String = row.get("stage")?;
    let status: String = row.get("status")?;
    Ok(Workstream {
        id: row.get("id")?,
        project_path: row.get("project_path")?,
        title: row.get("title")?,
        slug: row.get("slug")?,
        branch: row.get("branch")?,
        worktree_path: row.get("worktree_path")?,
        base_branch: row.get("base_branch")?,
        worktree_mode: WorktreeMode::parse(&worktree_mode),
        stage: Stage::parse(&stage),
        status: WorkstreamStatus::parse(&status),
        pr_number: row.get("pr_number")?,
        observed_pr: observed_pr(
            row.get("observed_pr_number")?,
            row.get("observed_pr_state")?,
        ),
        observed_ref: row.get("observed_ref")?,
        observed_pr_head: row.get("observed_pr_head")?,
        observed_pr_tip: row.get("observed_pr_tip")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
        parent_workstream_id: row.get("parent_workstream_id")?,
    })
}

const COLS: &str = "id, project_path, title, slug, branch, worktree_path, base_branch, \
                    worktree_mode, stage, status, pr_number, observed_pr_number, observed_pr_state, \
                    observed_ref, observed_pr_head, observed_pr_tip, \
                    created_at, updated_at, archived_at, parent_workstream_id";

pub struct ClaudeWorktree {
    pub workstream: Workstream,
    pub created_head: Option<String>,
}

pub struct NewClaudeWorktree {
    pub workstream_id: String,
    pub project_path: String,
    pub name: String,
    pub harness_session_id: Option<String>,
    pub agent_id: Option<String>,
    pub created_head: Option<String>,
}

const DAY_MS: f64 = 86_400_000.0;
/// An archive is purged this long after it was archived, unless kept.
pub const ARCHIVE_RETENTION_MS: f64 = 30.0 * DAY_MS;
/// The least notice any purge gets: the length of the "purges in" warning.
pub const PURGE_WARNING_MS: f64 = 7.0 * DAY_MS;

/// `pr_state` is `none`, `open`, `merged` or `closed`; NULL until a lookup succeeds.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveRecord {
    pub purge_after: f64,
    pub kept_at: Option<f64>,
    pub size_bytes: Option<f64>,
    pub uncommitted: Option<u32>,
    pub unpushed: Option<u32>,
    pub scanned_at: Option<f64>,
    pub pr_number: Option<u32>,
    pub pr_state: Option<String>,
    pub pr_checked_at: Option<f64>,
}

const RECORD_COLS: &str = "a.purge_after, a.kept_at, a.size_bytes, a.uncommitted, a.unpushed, \
                           a.scanned_at, a.pr_number AS archive_pr_number, a.pr_state, \
                           a.pr_checked_at";

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<ArchiveRecord> {
    Ok(ArchiveRecord {
        purge_after: row.get("purge_after")?,
        kept_at: row.get("kept_at")?,
        size_bytes: row.get("size_bytes")?,
        uncommitted: row.get("uncommitted")?,
        unpushed: row.get("unpushed")?,
        scanned_at: row.get("scanned_at")?,
        pr_number: row.get("archive_pr_number")?,
        pr_state: row.get("pr_state")?,
        pr_checked_at: row.get("pr_checked_at")?,
    })
}

fn qualified_cols() -> String {
    COLS.split(", ")
        .map(|col| format!("w.{col}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Db {
    pub fn insert_workstream(&self, new: NewWorkstream) -> Result<Workstream> {
        self.insert_child_workstream(new, None)
    }

    pub fn insert_child_workstream(
        &self,
        new: NewWorkstream,
        parent_workstream_id: Option<String>,
    ) -> Result<Workstream> {
        let now = now_ms();
        let item = Workstream {
            id: uuid::Uuid::new_v4().to_string(),
            project_path: new.project_path,
            title: new.title,
            slug: new.slug,
            branch: new.branch,
            worktree_path: new.worktree_path,
            base_branch: new.base_branch,
            worktree_mode: new.worktree_mode,
            stage: Stage::General,
            status: WorkstreamStatus::Active,
            pr_number: None,
            observed_pr: None,
            observed_ref: None,
            observed_pr_head: None,
            observed_pr_tip: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            parent_workstream_id,
        };
        self.with(|conn| {
            conn.execute(
                "INSERT INTO workstreams (id, project_path, title, slug, branch, worktree_path,
                                         base_branch, worktree_mode, stage, status, pr_number,
                                         created_at, updated_at, archived_at, parent_workstream_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                params![
                    item.id,
                    item.project_path,
                    item.title,
                    item.slug,
                    item.branch,
                    item.worktree_path,
                    item.base_branch,
                    item.worktree_mode.as_str(),
                    item.stage.as_str(),
                    item.status.as_str(),
                    item.pr_number,
                    item.created_at,
                    item.updated_at,
                    item.archived_at,
                    item.parent_workstream_id,
                ],
            )?;
            Ok(())
        })?;
        Ok(item)
    }

    /// Any status: an archived row still owns its slug's archive dir.
    pub fn workstream_slug_taken(&self, project_path: &str, slug: &str) -> Result<bool> {
        self.with(|conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM workstreams WHERE project_path = ?1 AND slug = ?2)",
                params![project_path, slug],
                |row| row.get(0),
            )
        })
    }

    pub fn insert_claude_worktree(&self, row: NewClaudeWorktree) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO claude_worktrees (workstream_id, project_path, name,
                                               harness_session_id, agent_id, created_head)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    row.workstream_id,
                    row.project_path,
                    row.name,
                    row.harness_session_id,
                    row.agent_id,
                    row.created_head,
                ],
            )?;
            Ok(())
        })
    }

    /// Archive keeps the row so a restore finds it again. A restored archive and a
    /// later create of the same name can both be active; the newer create wins.
    pub fn active_claude_worktree(
        &self,
        project_path: &str,
        name: &str,
    ) -> Result<Option<ClaudeWorktree>> {
        self.claude_worktree_where(project_path, "c.name", name)
    }

    pub fn active_claude_worktree_at(
        &self,
        project_path: &str,
        worktree_path: &str,
    ) -> Result<Option<ClaudeWorktree>> {
        self.claude_worktree_where(project_path, "w.worktree_path", worktree_path)
    }

    fn claude_worktree_where(
        &self,
        project_path: &str,
        column: &str,
        value: &str,
    ) -> Result<Option<ClaudeWorktree>> {
        self.with(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {}, c.created_head FROM claude_worktrees c
                     JOIN workstreams w ON w.id = c.workstream_id
                     WHERE c.project_path = ?1 AND {column} = ?2 AND w.status = 'active'
                     ORDER BY c.rowid DESC",
                    qualified_cols()
                ),
                params![project_path, value],
                |row| {
                    Ok(ClaudeWorktree {
                        workstream: row_to_item(row)?,
                        created_head: row.get("created_head")?,
                    })
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    pub fn get_workstream(&self, id: &str) -> Result<Option<Workstream>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {COLS} FROM workstreams WHERE id = ?1"),
                params![id],
                row_to_item,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    pub fn list_workstreams(
        &self,
        project_path: &str,
        include_archived: bool,
    ) -> Result<Vec<Workstream>> {
        self.with(|conn| {
            let sql = if include_archived {
                format!(
                    "SELECT {COLS} FROM workstreams WHERE project_path = ?1 ORDER BY created_at"
                )
            } else {
                format!(
                    "SELECT {COLS} FROM workstreams WHERE project_path = ?1 AND status = 'active'
                     ORDER BY created_at"
                )
            };
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![project_path], row_to_item)?;
            rows.collect()
        })
    }

    pub fn list_worktree_paths(&self) -> Result<Vec<String>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT worktree_path FROM workstreams")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect()
        })
    }

    pub fn list_active_workstreams(&self) -> Result<Vec<Workstream>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM workstreams WHERE status = 'active' ORDER BY created_at"
            ))?;
            let rows = stmt.query_map([], row_to_item)?;
            rows.collect()
        })
    }

    pub fn list_archived_workstreams(&self) -> Result<Vec<Workstream>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM workstreams WHERE status = 'archived'
                 ORDER BY archived_at DESC, created_at DESC"
            ))?;
            let rows = stmt.query_map([], row_to_item)?;
            rows.collect()
        })
    }

    /// Forward only, in one transaction so concurrent writers can't undo each other. `current_ref`
    /// is the ref PRs are looked up by now: a worktree that moved off the observed one starts its
    /// stage over from this pick, before the watcher's next poll can reset it.
    pub fn advance_workstream_stage(
        &self,
        id: &str,
        stage: Stage,
        current_ref: Option<&str>,
    ) -> Result<()> {
        let Some(to) = pipeline_rank(stage) else {
            return Ok(());
        };
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let (from, seen): (String, Option<String>) = tx.query_row(
                "SELECT stage, observed_ref FROM workstreams WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let moved_to = current_ref.filter(|now| seen.as_deref().is_some_and(|s| s != *now));
            if let Some(now) = moved_to {
                tx.execute(
                    "UPDATE workstreams SET stage = ?2, observed_ref = ?3, observed_pr_number = NULL,
                        observed_pr_state = NULL, observed_pr_head = NULL, observed_pr_tip = NULL,
                        updated_at = ?4
                     WHERE id = ?1",
                    params![id, stage.as_str(), now, now_ms()],
                )?;
            } else if pipeline_rank(Stage::parse(&from)).is_none_or(|rank| rank < to) {
                tx.execute(
                    "UPDATE workstreams SET stage = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, stage.as_str(), now_ms()],
                )?;
            }
            tx.commit()
        })
    }

    pub fn update_workstream_title(&self, id: &str, title: &str) -> Result<()> {
        self.touch(id, "title", title)
    }

    pub fn update_workstream_base_branch(&self, id: &str, base: &str) -> Result<()> {
        self.touch(id, "base_branch", base)
    }

    pub fn update_workstream_branch(&self, id: &str, branch: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET branch = ?2, pr_number = NULL WHERE id = ?1",
                params![id, branch],
            )?;
            Ok(())
        })
    }

    /// An in-app rename is the same work, so unlike a move it keeps the observation's place.
    pub fn rename_workstream_branch(&self, id: &str, branch: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET branch = ?2, pr_number = NULL,
                    observed_ref = CASE WHEN observed_ref IS NULL THEN NULL ELSE ?2 END
                 WHERE id = ?1",
                params![id, branch],
            )?;
            Ok(())
        })
    }

    pub fn set_workstream_pr(&self, id: &str, pr_number: Option<u32>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET pr_number = ?2, observed_pr_number = NULL,
                    observed_pr_state = NULL, observed_ref = NULL, observed_pr_head = NULL,
                    observed_pr_tip = NULL, updated_at = ?3 WHERE id = ?1",
                params![id, pr_number, now_ms()],
            )?;
            Ok(())
        })
    }

    /// Returns whether the row changed. Moving to a ref with no merged PR starts the stage over;
    /// `updated_at` otherwise stays, since a background poll isn't activity on the workstream.
    pub fn record_pr_observation(
        &self,
        id: &str,
        observed: Option<ObservedPr>,
        pr_ref: &str,
        head: Option<&str>,
        tip: Option<&str>,
    ) -> Result<bool> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let (number, state, seen, same_marks): (
                Option<u32>,
                Option<String>,
                Option<String>,
                bool,
            ) = tx.query_row(
                "SELECT observed_pr_number, observed_pr_state, observed_ref,
                    observed_pr_head IS ?2 AND observed_pr_tip IS ?3
                 FROM workstreams WHERE id = ?1",
                params![id, head, tip],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            let moved = seen.as_deref().is_some_and(|s| s != pr_ref);
            let changed = moved || observed_pr(number, state) != observed;
            if !changed && seen.is_some() && same_marks {
                return Ok(false);
            }
            if moved && observed.is_none_or(|now| now.state != PrState::Merged) {
                tx.execute(
                    "UPDATE workstreams SET stage = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, Stage::General.as_str(), now_ms()],
                )?;
            }
            tx.execute(
                "UPDATE workstreams SET observed_pr_number = ?2, observed_pr_state = ?3,
                    observed_ref = ?4, observed_pr_head = ?5, observed_pr_tip = ?6 WHERE id = ?1",
                params![
                    id,
                    observed.map(|o| o.number),
                    observed.map(|o| o.state.as_str()),
                    pr_ref,
                    head,
                    tip
                ],
            )?;
            tx.commit()?;
            Ok(changed)
        })
    }

    /// Starts a fresh retention record: anything kept or cached from an earlier
    /// archive of the same row is reset.
    pub fn archive_workstream(&self, id: &str) -> Result<()> {
        let now = now_ms();
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE workstreams SET status = 'archived', archived_at = ?2, updated_at = ?2
                 WHERE id = ?1",
                params![id, now],
            )?;
            tx.execute(
                "INSERT INTO workstream_archives (workstream_id, archived_at, purge_after)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(workstream_id) DO UPDATE SET
                    archived_at = excluded.archived_at, purge_after = excluded.purge_after,
                    kept_at = NULL, size_bytes = NULL, uncommitted = NULL, unpushed = NULL,
                    scanned_at = NULL, pr_number = NULL, pr_state = NULL, pr_checked_at = NULL",
                params![id, now, now + ARCHIVE_RETENTION_MS],
            )?;
            tx.commit()
        })
    }

    pub fn unarchive_workstream(&self, id: &str) -> Result<()> {
        self.unarchive_workstream_at(id, None)
    }

    /// `relocate` is `(from, to)`: the worktree moved, so its path and the cwd of
    /// every session that ran there move with it.
    pub fn unarchive_workstream_at(&self, id: &str, relocate: Option<(&str, &str)>) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE workstreams SET status = 'active', archived_at = NULL, updated_at = ?2
                 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            if let Some((from, to)) = relocate {
                tx.execute(
                    "UPDATE workstreams SET worktree_path = ?2 WHERE id = ?1",
                    params![id, to],
                )?;
                tx.execute(
                    "UPDATE sessions SET cwd = ?3 WHERE workstream_id = ?1 AND cwd = ?2",
                    params![id, from, to],
                )?;
            }
            // What the archive learned is newer than the row; `none` may only mean a failed lookup.
            tx.execute(
                "UPDATE workstreams SET
                    observed_pr_number = a.pr_number, observed_pr_state = a.pr_state,
                    observed_pr_head = CASE WHEN workstreams.observed_pr_number IS a.pr_number
                        THEN workstreams.observed_pr_head END,
                    observed_pr_tip = CASE WHEN workstreams.observed_pr_number IS a.pr_number
                        THEN workstreams.observed_pr_tip END
                 FROM workstream_archives a
                 WHERE workstreams.id = ?1 AND a.workstream_id = ?1 AND a.pr_number IS NOT NULL
                    AND a.pr_state IN ('open', 'merged', 'closed')",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM workstream_archives WHERE workstream_id = ?1",
                params![id],
            )?;
            tx.commit()
        })
    }

    /// Drops records whose row is no longer that archive (an older server
    /// restored or re-archived it) and creates missing ones. A rebuilt deadline
    /// is never nearer than the warning period.
    pub fn ensure_archive_records(&self, now: f64) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM workstream_archives WHERE NOT EXISTS (
                    SELECT 1 FROM workstreams w
                    WHERE w.id = workstream_archives.workstream_id AND w.status = 'archived'
                      AND COALESCE(w.archived_at, w.updated_at) = workstream_archives.archived_at)",
                [],
            )?;
            tx.execute(
                "INSERT INTO workstream_archives (workstream_id, archived_at, purge_after)
                 SELECT w.id, COALESCE(w.archived_at, w.updated_at),
                        MAX(COALESCE(w.archived_at, w.updated_at) + ?1, ?2 + ?3)
                 FROM workstreams w
                 WHERE w.status = 'archived' AND NOT EXISTS (
                    SELECT 1 FROM workstream_archives a WHERE a.workstream_id = w.id)",
                params![ARCHIVE_RETENTION_MS, now, PURGE_WARNING_MS],
            )?;
            tx.commit()
        })
    }

    pub fn archived_with_records(&self) -> Result<Vec<(Workstream, ArchiveRecord)>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {RECORD_COLS} FROM workstreams w
                 JOIN workstream_archives a ON a.workstream_id = w.id
                 WHERE w.status = 'archived'
                 ORDER BY w.archived_at DESC, w.created_at DESC",
                qualified_cols()
            ))?;
            let rows = stmt.query_map([], |row| Ok((row_to_item(row)?, row_to_record(row)?)))?;
            rows.collect()
        })
    }

    pub fn archive_record(&self, id: &str) -> Result<Option<ArchiveRecord>> {
        self.with(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {RECORD_COLS} FROM workstream_archives a WHERE a.workstream_id = ?1"
                ),
                params![id],
                row_to_record,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    pub fn due_archive_ids(&self, now: f64) -> Result<Vec<String>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT a.workstream_id FROM workstream_archives a
                 JOIN workstreams w ON w.id = a.workstream_id
                 WHERE w.status = 'archived' AND a.kept_at IS NULL AND a.purge_after <= ?1
                 ORDER BY a.purge_after",
            )?;
            let rows = stmt.query_map(params![now], |row| row.get(0))?;
            rows.collect()
        })
    }

    /// Returns whether the archive was newly kept.
    pub fn keep_archive(&self, id: &str, now: f64) -> Result<bool> {
        self.with(|conn| {
            let changed = conn.execute(
                "UPDATE workstream_archives SET kept_at = ?2
                 WHERE workstream_id = ?1 AND kept_at IS NULL",
                params![id, now],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn cache_archive_scan(&self, id: &str, scan: &ArchiveScan, now: f64) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstream_archives
                 SET size_bytes = ?2, uncommitted = ?3, unpushed = ?4, scanned_at = ?5
                 WHERE workstream_id = ?1",
                params![id, scan.size_bytes, scan.uncommitted, scan.unpushed, now],
            )?;
            Ok(())
        })
    }

    /// `state` of `None` records a lookup that failed: only the time is kept, so
    /// the next one waits out the backoff.
    pub fn cache_archive_pr(
        &self,
        id: &str,
        found: Option<(Option<u32>, &str)>,
        now: f64,
    ) -> Result<()> {
        self.with(|conn| {
            match found {
                Some((number, state)) => conn.execute(
                    "UPDATE workstream_archives
                     SET pr_number = ?2, pr_state = ?3, pr_checked_at = ?4
                     WHERE workstream_id = ?1",
                    params![id, number, state, now],
                )?,
                None => conn.execute(
                    "UPDATE workstream_archives SET pr_checked_at = ?2 WHERE workstream_id = ?1",
                    params![id, now],
                )?,
            };
            Ok(())
        })
    }

    pub fn delete_workstream(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM workstreams WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn touch(&self, id: &str, column: &str, value: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                &format!("UPDATE workstreams SET {column} = ?2, updated_at = ?3 WHERE id = ?1"),
                params![id, value, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn attach_workstream_doc(
        &self,
        workstream_id: &str,
        doc_path: &str,
        role: &str,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO workstream_docs (workstream_id, doc_path, role) VALUES (?1,?2,?3)
                 ON CONFLICT(workstream_id, doc_path) DO UPDATE SET role = excluded.role",
                params![workstream_id, doc_path, role],
            )?;
            Ok(())
        })
    }

    pub fn workstream_docs(&self, workstream_id: &str) -> Result<Vec<WorkstreamDoc>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT workstream_id, doc_path, role FROM workstream_docs
                 WHERE workstream_id = ?1 ORDER BY doc_path",
            )?;
            let rows = stmt.query_map(params![workstream_id], |row| {
                Ok(WorkstreamDoc {
                    workstream_id: row.get(0)?,
                    doc_path: row.get(1)?,
                    role: row.get(2)?,
                })
            })?;
            rows.collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(db: &Db, title: &str) -> Workstream {
        db.insert_workstream(NewWorkstream {
            project_path: "/repo".into(),
            title: title.into(),
            slug: format!("{title}-abcd"),
            branch: format!("hyprlayer/{title}-abcd"),
            worktree_path: format!("/wt/{title}"),
            base_branch: "origin/main".into(),
            worktree_mode: WorktreeMode::Isolated,
        })
        .unwrap()
    }

    #[test]
    fn workstream_crud_round_trip() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        let b = item(&db, "billing");

        let listed = db.list_workstreams("/repo", false).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[0].stage, Stage::General);
        assert_eq!(listed[0].worktree_mode, WorktreeMode::Isolated);
        assert!(listed[0].app_owns_branch());
        assert!(!listed[0].is_inplace());

        db.advance_workstream_stage(&a.id, Stage::Research, None)
            .unwrap();
        db.update_workstream_title(&a.id, "Auth revamp").unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.stage, Stage::Research);
        assert_eq!(got.title, "Auth revamp");
        assert!(got.updated_at >= got.created_at);

        db.archive_workstream(&b.id).unwrap();
        assert_eq!(db.list_workstreams("/repo", false).unwrap().len(), 1);
        assert_eq!(db.list_workstreams("/repo", true).unwrap().len(), 2);
        assert!(db
            .get_workstream(&b.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some());

        assert!(db.get_workstream("nope").unwrap().is_none());
    }

    #[test]
    fn folder_workstreams_own_nothing_and_never_look_inplace() {
        let db = Db::open_in_memory().unwrap();
        let row = db
            .insert_workstream(NewWorkstream {
                project_path: "/notes".into(),
                title: "reading".into(),
                slug: "reading-abcd".into(),
                branch: String::new(),
                worktree_path: "/notes".into(),
                base_branch: String::new(),
                worktree_mode: WorktreeMode::Folder,
            })
            .unwrap();
        let got = db.get_workstream(&row.id).unwrap().unwrap();

        assert_eq!(got.worktree_mode, WorktreeMode::Folder);
        assert!(got.is_folder());
        assert!(!got.app_owns_worktree());
        assert!(!got.is_inplace());
        assert!(!got.app_owns_branch());

        db.insert_workstream(NewWorkstream {
            project_path: "/notes".into(),
            title: "drafting".into(),
            slug: "drafting-efgh".into(),
            branch: String::new(),
            worktree_path: "/notes".into(),
            base_branch: String::new(),
            worktree_mode: WorktreeMode::Folder,
        })
        .unwrap();
        assert_eq!(db.list_workstreams("/notes", false).unwrap().len(), 2);

        assert_eq!(WorktreeMode::parse("submodule"), WorktreeMode::Inplace);
        assert_eq!(WorktreeMode::parse("folder"), WorktreeMode::Folder);
    }

    #[test]
    fn pr_ref_prefers_attached_number_then_branch() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        assert_eq!(a.pr_ref(), a.branch);

        db.set_workstream_pr(&a.id, Some(42)).unwrap();
        let attached = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(attached.pr_ref(), "42");

        db.set_workstream_pr(&a.id, None).unwrap();
        let detached = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(detached.pr_ref(), detached.branch);
    }

    fn row(db: &Db, title: &str, mode: WorktreeMode, on: &str) -> Workstream {
        db.insert_workstream(NewWorkstream {
            project_path: "/repo".into(),
            title: title.into(),
            slug: format!("{title}-abcd"),
            branch: on.into(),
            worktree_path: "/repo".into(),
            base_branch: "origin/main".into(),
            worktree_mode: mode,
        })
        .unwrap()
    }

    #[test]
    fn only_user_owned_branch_modes_follow_head() {
        let db = Db::open_in_memory().unwrap();
        let inplace = row(&db, "here", WorktreeMode::Inplace, "main");
        let existing = row(&db, "adopted", WorktreeMode::Existing, "main");
        let isolated = row(&db, "own", WorktreeMode::Isolated, "main");
        let folder = row(&db, "notes", WorktreeMode::Folder, "");

        assert!(inplace.follows_head());
        assert!(existing.follows_head());
        assert_eq!(
            inplace.branch_to_adopt(Some("feature/x")),
            Some("feature/x")
        );
        assert_eq!(
            existing.branch_to_adopt(Some("feature/x")),
            Some("feature/x")
        );

        assert!(!isolated.follows_head());
        assert_eq!(isolated.branch_to_adopt(Some("feature/x")), None);
        assert!(!folder.follows_head());
        assert_eq!(folder.branch_to_adopt(Some("feature/x")), None);
    }

    #[test]
    fn branch_adoption_ignores_no_ops_and_headless_checkouts() {
        let db = Db::open_in_memory().unwrap();
        let a = row(&db, "here", WorktreeMode::Inplace, "main");

        assert_eq!(a.branch_to_adopt(Some("main")), None);
        assert_eq!(a.branch_to_adopt(None), None);
        assert_eq!(a.branch_to_adopt(Some("")), None);

        db.archive_workstream(&a.id).unwrap();
        let archived = db.get_workstream(&a.id).unwrap().unwrap();
        assert!(!archived.follows_head());
        assert_eq!(archived.branch_to_adopt(Some("feature/x")), None);
    }

    #[test]
    fn update_workstream_branch_persists_and_repoints_pr_ref() {
        let db = Db::open_in_memory().unwrap();
        let a = row(&db, "here", WorktreeMode::Inplace, "main");

        db.update_workstream_branch(&a.id, "feature/x").unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.branch, "feature/x");
        assert_eq!(got.pr_ref(), "feature/x");
        assert_eq!(got.base_branch, "origin/main");
        assert!(got.updated_at >= got.created_at);
    }

    #[test]
    fn adopting_a_branch_detaches_the_previous_branchs_pr() {
        let db = Db::open_in_memory().unwrap();
        let a = row(&db, "here", WorktreeMode::Inplace, "main");
        db.set_workstream_pr(&a.id, Some(42)).unwrap();

        db.update_workstream_branch(&a.id, "feature/x").unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.pr_number, None);
        assert_eq!(got.pr_ref(), "feature/x");
    }

    #[test]
    fn adopting_a_branch_leaves_updated_at_alone() {
        let db = Db::open_in_memory().unwrap();
        let a = row(&db, "here", WorktreeMode::Inplace, "main");
        let before = db.get_workstream(&a.id).unwrap().unwrap().updated_at;

        db.update_workstream_branch(&a.id, "feature/x").unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.updated_at, before);

        db.update_workstream_title(&a.id, "renamed").unwrap();
        assert!(db.get_workstream(&a.id).unwrap().unwrap().updated_at >= before);
    }

    #[test]
    fn the_observed_pr_head_is_written_read_back_and_cleared() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        let merged = Some(ObservedPr {
            number: 7,
            state: PrState::Merged,
        });
        let marks = |db: &Db| {
            let row = db.get_workstream(&a.id).unwrap().unwrap();
            (row.observed_pr_head, row.observed_pr_tip)
        };
        let some = |h: &str, t: &str| (Some(h.to_string()), Some(t.to_string()));

        assert!(db
            .record_pr_observation(&a.id, merged, &a.branch, Some("h1"), Some("t1"))
            .unwrap());
        assert_eq!(marks(&db), some("h1", "t1"));
        assert!(!db
            .record_pr_observation(&a.id, merged, &a.branch, Some("h2"), Some("t1"))
            .unwrap());
        assert_eq!(marks(&db), some("h2", "t1"));
        assert!(!db
            .record_pr_observation(&a.id, merged, &a.branch, Some("h2"), Some("t2"))
            .unwrap());
        assert_eq!(marks(&db), some("h2", "t2"));

        db.set_workstream_pr(&a.id, Some(9)).unwrap();
        assert_eq!(marks(&db), (None, None));
    }

    #[test]
    fn a_restore_keeps_the_head_only_for_the_pr_it_was_recorded_for() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        let restored_head = |archived_pr: u32| {
            let observed = Some(ObservedPr {
                number: 7,
                state: PrState::Merged,
            });
            db.record_pr_observation(&a.id, observed, &a.branch, Some("h7"), Some("t7"))
                .unwrap();
            db.archive_workstream(&a.id).unwrap();
            db.cache_archive_pr(&a.id, Some((Some(archived_pr), "merged")), now_ms())
                .unwrap();
            db.unarchive_workstream(&a.id).unwrap();
            let got = db.get_workstream(&a.id).unwrap().unwrap();
            assert_eq!(got.observed_pr.map(|o| o.number), Some(archived_pr));
            (got.observed_pr_head, got.observed_pr_tip)
        };
        assert_eq!(
            restored_head(7),
            (Some("h7".to_string()), Some("t7".to_string()))
        );
        assert_eq!(restored_head(8), (None, None));
    }

    #[test]
    fn archive_unarchive_round_trip_clears_archived_at() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        let b = item(&db, "billing");

        db.archive_workstream(&a.id).unwrap();
        let archived = db.list_archived_workstreams().unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, a.id);
        assert_eq!(archived[0].status, WorkstreamStatus::Archived);
        assert!(archived[0].archived_at.is_some());

        db.unarchive_workstream(&a.id).unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.status, WorkstreamStatus::Active);
        assert!(got.archived_at.is_none());
        assert!(db.list_archived_workstreams().unwrap().is_empty());
        assert_eq!(
            db.get_workstream(&b.id).unwrap().unwrap().status,
            WorkstreamStatus::Active
        );
    }

    #[test]
    fn delete_workstream_cascades_docs() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        db.attach_workstream_doc(&a.id, "/t/r.md", "research")
            .unwrap();
        assert_eq!(db.workstream_docs(&a.id).unwrap().len(), 1);

        db.delete_workstream(&a.id).unwrap();
        assert!(db.get_workstream(&a.id).unwrap().is_none());
        assert!(db.workstream_docs(&a.id).unwrap().is_empty());
    }

    #[test]
    fn attaching_a_doc_again_updates_its_role() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        db.attach_workstream_doc(&a.id, "/t/r.md", "research")
            .unwrap();
        db.attach_workstream_doc(&a.id, "/t/p.md", "plan").unwrap();
        db.attach_workstream_doc(&a.id, "/t/r.md", "other").unwrap();

        let docs = db.workstream_docs(&a.id).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(
            docs.iter().find(|d| d.doc_path == "/t/r.md").unwrap().role,
            "other"
        );
    }
}
