use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::error::Result;
use crate::pipeline::Stage;

/// How a workstream is backed: `isolated` (app-created branch + dedicated
/// worktree), `existing` (user's local branch in a dedicated worktree),
/// `inplace` (no worktree — the project's own checkout), or `folder` (a
/// [`ProjectSource::Folder`](super::ProjectSource) project — the project
/// directory itself, with no git at all). Governs whether the branch +
/// directory are app-owned and safe to drop on archive/delete.
///
/// Tolerance: an unrecognized stored value reads (via [`WorktreeMode::parse`])
/// as `Inplace` — the touch-nothing mode — so a mangled row can never make
/// archive/cleanup delete a branch or checkout the app doesn't own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum WorktreeMode {
    Isolated,
    Existing,
    Inplace,
    Folder,
}

impl WorktreeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WorktreeMode::Isolated => "isolated",
            WorktreeMode::Existing => "existing",
            WorktreeMode::Inplace => "inplace",
            WorktreeMode::Folder => "folder",
        }
    }

    /// Tolerant row-read: see the enum docs for why unknown ⇒ `Inplace`.
    pub fn parse(s: &str) -> Self {
        match s {
            "isolated" => WorktreeMode::Isolated,
            "existing" => WorktreeMode::Existing,
            "folder" => WorktreeMode::Folder,
            _ => WorktreeMode::Inplace,
        }
    }
}

/// Whether a workstream is live or archived. Unknown stored values read as
/// `Active` via [`WorkstreamStatus::parse`] (they can only come from a newer
/// schema; showing the row beats hiding it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum WorkstreamStatus {
    Active,
    Archived,
}

impl WorkstreamStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkstreamStatus::Active => "active",
            WorkstreamStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "archived" => WorkstreamStatus::Archived,
            _ => WorkstreamStatus::Active,
        }
    }
}

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
    pub worktree_mode: WorktreeMode,
    pub stage: Stage,
    pub status: WorkstreamStatus,
    pub pr_number: Option<u32>,
    pub created_at: f64,
    pub updated_at: f64,
    pub archived_at: Option<f64>,
}

impl Workstream {
    /// The argument to pass `gh pr view`/`gh pr merge` for this workstream's PR:
    /// a manually attached PR number when one is set, otherwise the branch name
    /// (gh then resolves the branch's own open PR — the default behavior).
    pub fn pr_ref(&self) -> String {
        match self.pr_number {
            Some(n) => n.to_string(),
            None => self.branch.clone(),
        }
    }

    /// `inplace` workstreams share the project checkout AND adopt one of its
    /// branches — the case the one-per-project guard exists for.
    pub fn is_inplace(&self) -> bool {
        self.worktree_mode == WorktreeMode::Inplace
    }

    /// No dedicated worktree — `worktree_path` IS the user's own project
    /// directory. The archive/restore/delete paths special-case these so they
    /// never move, remove, or re-link it.
    pub fn shares_project_dir(&self) -> bool {
        matches!(
            self.worktree_mode,
            WorktreeMode::Inplace | WorktreeMode::Folder
        )
    }

    /// A `folder` project's workstream: no git, no branch, no thoughts mapping.
    /// Every git/PR/thoughts surface is skipped for these.
    pub fn is_folder(&self) -> bool {
        self.worktree_mode == WorktreeMode::Folder
    }

    /// True only when the app created the branch (`isolated`). `existing` and
    /// `inplace` workstreams adopt a branch the user already had, and `folder`
    /// has none, so the app must never delete it on archive/cleanup.
    pub fn app_owns_branch(&self) -> bool {
        self.worktree_mode == WorktreeMode::Isolated
    }

    /// Modes whose branch is the user's, not the app's: the checkout can be
    /// switched out from under us and the row should follow. `isolated` stays
    /// pinned to the branch the app created; `folder` has no branch at all.
    pub fn follows_head(&self) -> bool {
        self.status == WorkstreamStatus::Active
            && matches!(
                self.worktree_mode,
                WorktreeMode::Inplace | WorktreeMode::Existing
            )
    }

    /// The branch this row should adopt given the checkout's live HEAD, or
    /// `None` when nothing changes. `head` is `None` for a detached or unborn
    /// HEAD, which must never overwrite a good stored value.
    pub fn branch_to_adopt<'a>(&self, head: Option<&'a str>) -> Option<&'a str> {
        let head = head?;
        (self.follows_head() && !head.is_empty() && head != self.branch).then_some(head)
    }
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
    pub worktree_mode: WorktreeMode,
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<Workstream> {
    // The enum columns stay TEXT in sqlite; parse tolerantly so rows written by
    // any past (or future) schema keep loading.
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
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        archived_at: row.get("archived_at")?,
    })
}

const COLS: &str = "id, project_path, title, slug, branch, worktree_path, base_branch, \
                    worktree_mode, stage, status, pr_number, created_at, updated_at, archived_at";

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
            worktree_mode: new.worktree_mode,
            stage: Stage::General,
            status: WorkstreamStatus::Active,
            pr_number: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        self.with(|conn| {
            conn.execute(
                "INSERT INTO workstreams (id, project_path, title, slug, branch, worktree_path,
                                         base_branch, worktree_mode, stage, status, pr_number,
                                         created_at, updated_at, archived_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
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

    pub fn update_workstream_stage(&self, id: &str, stage: Stage) -> Result<()> {
        self.touch(id, "stage", stage.as_str())
    }

    pub fn update_workstream_title(&self, id: &str, title: &str) -> Result<()> {
        self.touch(id, "title", title)
    }

    /// Re-target the workstream's merge base. The next status recompute measures
    /// ahead/behind against it; a future Ship opens its PR there.
    pub fn update_workstream_base_branch(&self, id: &str, base: &str) -> Result<()> {
        self.touch(id, "base_branch", base)
    }

    /// Adopt the branch the checkout is actually on. Only ever called for rows
    /// whose branch the app does not own (see [`Workstream::follows_head`]).
    ///
    /// Clears `pr_number`: an attached PR belongs to the branch we just left, and
    /// [`Workstream::pr_ref`] prefers it over the branch — leaving it would point
    /// Merge at the wrong PR. Deliberately not a `touch`: a checkout done out of
    /// band isn't workstream activity, and stamping `updated_at` would reorder
    /// the board under the user.
    pub fn update_workstream_branch(&self, id: &str, branch: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE workstreams SET branch = ?2, pr_number = NULL WHERE id = ?1",
                params![id, branch],
            )?;
            Ok(())
        })
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
        // The fixture creates isolated workstreams: app owns the branch, has a
        // dedicated worktree.
        assert_eq!(listed[0].worktree_mode, WorktreeMode::Isolated);
        assert!(listed[0].app_owns_branch());
        assert!(!listed[0].is_inplace());

        db.update_workstream_stage(&a.id, Stage::Research).unwrap();
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
        // Shares the user's directory, so archive/restore must never move it...
        assert!(got.shares_project_dir());
        // ...but it is NOT `inplace`: that's the one-per-project branch-adopting
        // mode, and a folder project can hold as many workstreams as it likes.
        assert!(!got.is_inplace());
        assert!(!got.app_owns_branch());

        // A second one in the same folder is fine.
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

        // A value from a newer schema still falls back to the touch-nothing mode.
        assert_eq!(WorktreeMode::parse("submodule"), WorktreeMode::Inplace);
        assert_eq!(WorktreeMode::parse("folder"), WorktreeMode::Folder);
    }

    #[test]
    fn pr_ref_prefers_attached_number_then_branch() {
        let db = Db::open_in_memory().unwrap();
        let a = item(&db, "auth");
        // No attached PR → gh resolves the branch's own PR.
        assert_eq!(a.pr_ref(), a.branch);

        db.set_workstream_pr(&a.id, Some(42)).unwrap();
        let attached = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(attached.pr_ref(), "42");

        // Detaching reverts to branch-based resolution.
        db.set_workstream_pr(&a.id, None).unwrap();
        let detached = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(detached.pr_ref(), detached.branch);
    }

    /// A row in `mode`, on branch `on`, used by the branch-follow truth table.
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

        // The modes that adopt a branch the user already owns follow it.
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

        // The app owns the isolated branch, so archive may delete it — never
        // let HEAD repoint the row at a branch the user made.
        assert!(!isolated.follows_head());
        assert_eq!(isolated.branch_to_adopt(Some("feature/x")), None);
        assert!(!folder.follows_head());
        assert_eq!(folder.branch_to_adopt(Some("feature/x")), None);
    }

    #[test]
    fn branch_adoption_ignores_no_ops_and_headless_checkouts() {
        let db = Db::open_in_memory().unwrap();
        let a = row(&db, "here", WorktreeMode::Inplace, "main");

        // Already there.
        assert_eq!(a.branch_to_adopt(Some("main")), None);
        // Detached or unborn HEAD: keep the last good branch.
        assert_eq!(a.branch_to_adopt(None), None);
        assert_eq!(a.branch_to_adopt(Some("")), None);

        // An archived row is a historical record; the checkout moved on without it.
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
        // The whole point: gh now resolves the new branch's PR, not main's.
        assert_eq!(got.pr_ref(), "feature/x");
        // Base is deliberately untouched — it's the user-settable merge target.
        assert_eq!(got.base_branch, "origin/main");
        assert!(got.updated_at >= got.created_at);
    }

    /// An attached PR outranks the branch in `pr_ref()`, so adoption has to drop
    /// it — otherwise Merge acts on the PR of the branch we just left.
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

    /// Out-of-band checkouts aren't workstream activity: the board sorts columns
    /// by `updated_at`, so stamping it would jump the card to the top for a
    /// change the user made in a terminal.
    #[test]
    fn adopting_a_branch_leaves_updated_at_alone() {
        let db = Db::open_in_memory().unwrap();
        let a = row(&db, "here", WorktreeMode::Inplace, "main");
        let before = db.get_workstream(&a.id).unwrap().unwrap().updated_at;

        db.update_workstream_branch(&a.id, "feature/x").unwrap();
        let got = db.get_workstream(&a.id).unwrap().unwrap();
        assert_eq!(got.updated_at, before);

        // A real edit still stamps it.
        db.update_workstream_title(&a.id, "renamed").unwrap();
        assert!(db.get_workstream(&a.id).unwrap().unwrap().updated_at >= before);
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
        // Untouched siblings stay untouched.
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
