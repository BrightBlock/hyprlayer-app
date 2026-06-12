use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
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
pub struct WorkItemDoc {
    pub work_item_id: String,
    pub doc_path: String,
    /// research | plan | other
    pub role: String,
}

pub struct NewWorkItem {
    pub project_path: String,
    pub title: String,
    pub slug: String,
    pub branch: String,
    pub worktree_path: String,
    pub base_branch: String,
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<WorkItem> {
    Ok(WorkItem {
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
    pub fn insert_work_item(&self, new: NewWorkItem) -> Result<WorkItem> {
        let now = now_ms();
        let item = WorkItem {
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
                "INSERT INTO work_items (id, project_path, title, slug, branch, worktree_path,
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

    pub fn get_work_item(&self, id: &str) -> Result<Option<WorkItem>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {COLS} FROM work_items WHERE id = ?1"),
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

    pub fn list_work_items(
        &self,
        project_path: &str,
        include_archived: bool,
    ) -> Result<Vec<WorkItem>> {
        self.with(|conn| {
            let sql = if include_archived {
                format!("SELECT {COLS} FROM work_items WHERE project_path = ?1 ORDER BY created_at")
            } else {
                format!(
                    "SELECT {COLS} FROM work_items WHERE project_path = ?1 AND status = 'active'
                     ORDER BY created_at"
                )
            };
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![project_path], row_to_item)?;
            rows.collect()
        })
    }

    /// All non-archived items across projects (startup reconciliation).
    pub fn list_active_work_items(&self) -> Result<Vec<WorkItem>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM work_items WHERE status = 'active' ORDER BY created_at"
            ))?;
            let rows = stmt.query_map([], row_to_item)?;
            rows.collect()
        })
    }

    pub fn update_work_item_stage(&self, id: &str, stage: &str) -> Result<()> {
        self.touch(id, "stage", stage)
    }

    pub fn update_work_item_title(&self, id: &str, title: &str) -> Result<()> {
        self.touch(id, "title", title)
    }

    pub fn set_work_item_pr(&self, id: &str, pr_number: Option<u32>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE work_items SET pr_number = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, pr_number, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn archive_work_item(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE work_items SET status = 'archived', archived_at = ?2, updated_at = ?2
                 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            Ok(())
        })
    }

    fn touch(&self, id: &str, column: &str, value: &str) -> Result<()> {
        // column is a compile-time constant from the callers above.
        self.with(|conn| {
            conn.execute(
                &format!("UPDATE work_items SET {column} = ?2, updated_at = ?3 WHERE id = ?1"),
                params![id, value, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn attach_work_item_doc(
        &self,
        work_item_id: &str,
        doc_path: &str,
        role: &str,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO work_item_docs (work_item_id, doc_path, role) VALUES (?1,?2,?3)
                 ON CONFLICT(work_item_id, doc_path) DO UPDATE SET role = excluded.role",
                params![work_item_id, doc_path, role],
            )?;
            Ok(())
        })
    }

    pub fn detach_work_item_doc(&self, work_item_id: &str, doc_path: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM work_item_docs WHERE work_item_id = ?1 AND doc_path = ?2",
                params![work_item_id, doc_path],
            )?;
            Ok(())
        })
    }

    pub fn work_item_docs(&self, work_item_id: &str) -> Result<Vec<WorkItemDoc>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT work_item_id, doc_path, role FROM work_item_docs
                 WHERE work_item_id = ?1 ORDER BY doc_path",
            )?;
            let rows = stmt.query_map(params![work_item_id], |row| {
                Ok(WorkItemDoc {
                    work_item_id: row.get(0)?,
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

    fn item(db: &Db, title: &str) -> WorkItem {
        db.insert_work_item(NewWorkItem {
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
    fn work_item_crud_round_trip() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        let b = item(&db, "billing");

        let listed = db.list_work_items("/repo", false).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[0].stage, "general");

        db.update_work_item_stage(&a.id, "research").unwrap();
        db.update_work_item_title(&a.id, "Auth revamp").unwrap();
        let got = db.get_work_item(&a.id).unwrap().unwrap();
        assert_eq!(got.stage, "research");
        assert_eq!(got.title, "Auth revamp");
        assert!(got.updated_at >= got.created_at);

        db.archive_work_item(&b.id).unwrap();
        assert_eq!(db.list_work_items("/repo", false).unwrap().len(), 1);
        assert_eq!(db.list_work_items("/repo", true).unwrap().len(), 2);
        assert!(db
            .get_work_item(&b.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some());

        assert!(db.get_work_item("nope").unwrap().is_none());
    }

    #[test]
    fn docs_attach_detach_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        db.attach_work_item_doc(&a.id, "/t/r.md", "research")
            .unwrap();
        db.attach_work_item_doc(&a.id, "/t/p.md", "plan").unwrap();
        // upsert role
        db.attach_work_item_doc(&a.id, "/t/r.md", "other").unwrap();

        let docs = db.work_item_docs(&a.id).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(
            docs.iter().find(|d| d.doc_path == "/t/r.md").unwrap().role,
            "other"
        );

        db.detach_work_item_doc(&a.id, "/t/p.md").unwrap();
        assert_eq!(db.work_item_docs(&a.id).unwrap().len(), 1);
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
        assert_eq!(db.list_work_items("/repo", false).unwrap().len(), 1);
    }
}
