use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Workstream {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub slug: String,
    pub branch: String,
    pub worktree_path: String,
    pub base_branch: String,
    /// general | research | plan | implement | validate | ship
    pub stage: String,
    /// active | archived
    pub status: String,
    pub pr_number: Option<u32>,
    pub created_at: f64,
    pub updated_at: f64,
    pub archived_at: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamDoc {
    pub workstream_id: String,
    pub doc_path: String,
    /// research | plan | other
    pub role: String,
}

pub struct NewWorkstream {
    pub project_path: String,
    pub title: String,
    pub slug: String,
    pub branch: String,
    pub worktree_path: String,
    pub base_branch: String,
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<Workstream> {
    Ok(Workstream {
        id: row.get("id")?,
        project_path: row.get("project_path")?,
        title: row.get("title")?,
        slug: row.get("slug")?,
        branch: row.get("branch")?,
        worktree_path: row.get("worktree_path")?,
        base_branch: row.get("base_branch")?,
        stage: row.get("stage")?,
        status: row.get("status")?,
        pr_number: row.get("pr_number")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
    })
}

const COLS: &str = "id, project_path, title, slug, branch, worktree_path, base_branch, \
                    stage, status, pr_number, created_at, updated_at, archived_at";

impl Db {
    pub fn insert_workstream(&self, new: NewWorkstream) -> Result<Workstream> {
        let now = now_ms();
        let item = Workstream {
            id: uuid::Uuid::new_v4().to_string(),
            project_path: new.project_path,
            title: new.title,
            slug: new.slug,
            branch: new.branch,
            worktree_path: new.worktree_path,
            base_branch: new.base_branch,
            stage: "general".into(),
            status: "active".into(),
            pr_number: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        self.with(|conn| {
            conn.execute(
                "INSERT INTO workstreams (id, project_path, title, slug, branch, worktree_path,
                                         base_branch, stage, status, pr_number, created_at,
                                         updated_at, archived_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    item.id,
                    item.project_path,
                    item.title,
                    item.slug,
                    item.branch,
                    item.worktree_path,
                    item.base_branch,
                    item.stage,
                    item.status,
                    item.pr_number,
                    item.created_at,
                    item.updated_at,
                    item.archived_at,
                ],
            )?;
            Ok(())
        })?;
        Ok(item)
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

    /// Worktree paths of EVERY row, archived included — the durable record of
    /// which repo mappings are workstream infrastructure rather than projects.
    pub fn list_worktree_paths(&self) -> Result<Vec<String>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT worktree_path FROM workstreams")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect()
        })
    }

    /// All non-archived items across projects (startup reconciliation).
    pub fn list_active_workstreams(&self) -> Result<Vec<Workstream>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM workstreams WHERE status = 'active' ORDER BY created_at"
            ))?;
            let rows = stmt.query_map([], row_to_item)?;
            rows.collect()
        })
    }

    /// All archived items across projects, newest-first — feeds the archive
    /// history view.
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

    pub fn update_workstream_stage(&self, id: &str, stage: &str) -> Result<()> {
        self.touch(id, "stage", stage)
    }

    pub fn update_workstream_title(&self, id: &str, title: &str) -> Result<()> {
        self.touch(id, "title", title)
    }

    pub fn set_workstream_pr(&self, id: &str, pr_number: Option<u32>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET pr_number = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, pr_number, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn archive_workstream(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET status = 'archived', archived_at = ?2, updated_at = ?2
                 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            Ok(())
        })
    }

    /// Flip an archived row back to active and clear `archived_at`. The
    /// matching worktree restore is the caller's job.
    pub fn unarchive_workstream(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET status = 'active', archived_at = NULL, updated_at = ?2
                 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            Ok(())
        })
    }

    /// Permanent delete. ON DELETE CASCADE on workstream_docs sweeps the
    /// attached doc rows. The matching filesystem purge is the caller's job.
    pub fn delete_workstream(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM workstreams WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn touch(&self, id: &str, column: &str, value: &str) -> Result<()> {
        // column is a compile-time constant from the callers above.
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

    pub fn detach_workstream_doc(&self, workstream_id: &str, doc_path: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM workstream_docs WHERE workstream_id = ?1 AND doc_path = ?2",
                params![workstream_id, doc_path],
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
        assert_eq!(listed[0].stage, "general");

        db.update_workstream_stage(&a.id, "research").unwrap();
        db.update_workstream_title(&a.id, "Auth revamp").unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.stage, "research");
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
    fn archive_unarchive_round_trip_clears_archived_at() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        let b = item(&db, "billing");

        db.archive_workstream(&a.id).unwrap();
        let archived = db.list_archived_workstreams().unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, a.id);
        assert_eq!(archived[0].status, "archived");
        assert!(archived[0].archived_at.is_some());

        db.unarchive_workstream(&a.id).unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.status, "active");
        assert!(got.archived_at.is_none());
        assert!(db.list_archived_workstreams().unwrap().is_empty());
        // Untouched siblings stay untouched.
        assert_eq!(db.get_workstream(&b.id).unwrap().unwrap().status, "active");
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
    fn docs_attach_detach_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        db.attach_workstream_doc(&a.id, "/t/r.md", "research")
            .unwrap();
        db.attach_workstream_doc(&a.id, "/t/p.md", "plan").unwrap();
        // upsert role
        db.attach_workstream_doc(&a.id, "/t/r.md", "other").unwrap();

        let docs = db.workstream_docs(&a.id).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(
            docs.iter().find(|d| d.doc_path == "/t/r.md").unwrap().role,
            "other"
        );

        db.detach_workstream_doc(&a.id, "/t/p.md").unwrap();
        assert_eq!(db.workstream_docs(&a.id).unwrap().len(), 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let db = Db::open(&path).unwrap();
            item(&db, "x");
        }
        // Re-open: migrations must not re-run / data must survive.
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_workstreams("/repo", false).unwrap().len(), 1);
    }
}
