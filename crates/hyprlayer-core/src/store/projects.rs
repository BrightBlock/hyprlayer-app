use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::error::Result;

/// How a project entered the registry. `Folder` is the odd one out: a plain
/// directory with no git and no thoughts mapping, so every git, PR, and
/// hyprlayer-CLI surface is skipped for it.
///
/// Tolerance: an unrecognized stored value reads as `Disk` — the in-place,
/// touch-nothing kind — so a mangled row can never make removal delete files
/// the app doesn't own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ProjectSource {
    /// Cloned from a URL into the repos root.
    Clone,
    /// An existing git repo imported in place.
    Disk,
    /// A local directory that isn't a git repo.
    Folder,
}

impl ProjectSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectSource::Clone => "clone",
            ProjectSource::Disk => "disk",
            ProjectSource::Folder => "folder",
        }
    }

    /// Tolerant row-read: see the enum docs for why unknown ⇒ `Disk`.
    pub fn parse(s: &str) -> Self {
        match s {
            "clone" => ProjectSource::Clone,
            "folder" => ProjectSource::Folder,
            _ => ProjectSource::Disk,
        }
    }

    /// Whether the project is backed by git — false only for `Folder`, which
    /// has no branches, worktrees, PRs, or thoughts mapping.
    pub fn is_git(self) -> bool {
        self != ProjectSource::Folder
    }
}

/// A registered project: an app-owned clone the user added deliberately, via
/// cloning a hosted repo, importing one from disk, or opening a plain local
/// folder. This registry — not the thoughts config's `repoMappings` — is the
/// source of truth for the project list, so the per-worktree `thoughts init`
/// entries never surface as projects.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    /// Absolute path of the canonical clone (`~/hyprlayer/repos/<name>`).
    pub path: String,
    pub name: String,
    /// The git source it tracks; None for a local-only repo with no remote.
    pub origin_url: Option<String>,
    pub source: ProjectSource,
    pub created_at: f64,
}

pub struct NewProject {
    pub path: String,
    pub name: String,
    pub origin_url: Option<String>,
    pub source: ProjectSource,
}

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<ProjectRecord> {
    let source: String = row.get("source")?;
    Ok(ProjectRecord {
        path: row.get("path")?,
        name: row.get("name")?,
        origin_url: row.get("origin_url")?,
        source: ProjectSource::parse(&source),
        created_at: row.get("created_at")?,
    })
}

const COLS: &str = "path, name, origin_url, source, created_at";

impl Db {
    /// Register a project. Idempotent on `path`: re-adding the same clone path
    /// refreshes its metadata rather than erroring.
    pub fn insert_project(&self, new: NewProject) -> Result<ProjectRecord> {
        let rec = ProjectRecord {
            path: new.path,
            name: new.name,
            origin_url: new.origin_url,
            source: new.source,
            created_at: now_ms(),
        };
        self.with(|conn| {
            // A new project appends at the bottom of the sidebar (MAX+1); the
            // conflict path leaves `position` (and `created_at`) untouched, so
            // re-registering an existing clone never moves it.
            conn.execute(
                "INSERT INTO projects (path, name, origin_url, source, created_at, position)
                 VALUES (?1,?2,?3,?4,?5, (SELECT COALESCE(MAX(position), -1) + 1 FROM projects))
                 ON CONFLICT(path) DO UPDATE SET
                     name = excluded.name,
                     origin_url = excluded.origin_url,
                     source = excluded.source",
                params![
                    rec.path,
                    rec.name,
                    rec.origin_url,
                    rec.source.as_str(),
                    rec.created_at
                ],
            )?;
            Ok(())
        })?;
        Ok(rec)
    }

    /// Persist a user-defined project order (drag-to-reorder in the sidebar).
    /// `ordered_paths` is the full project list in its new top-to-bottom order;
    /// each row's `position` is set to its index. Runs in one transaction, so the
    /// list never renders half-reordered. Idempotent and self-healing: unknown
    /// paths are no-ops, and any drift is corrected by the next reorder.
    pub fn reorder_projects(&self, ordered_paths: &[String]) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            for (i, path) in ordered_paths.iter().enumerate() {
                tx.execute(
                    "UPDATE projects SET position = ?1 WHERE path = ?2",
                    params![i as f64, path],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRecord>> {
        self.with(|conn| {
            // `position` is the drag-to-reorder rank; `name` tiebreaks so rows
            // still at the default 0 (never reordered) stay alphabetical.
            let mut stmt =
                conn.prepare(&format!("SELECT {COLS} FROM projects ORDER BY position, name"))?;
            let rows = stmt.query_map([], row_to_project)?;
            rows.collect()
        })
    }

    pub fn get_project_record(&self, path: &str) -> Result<Option<ProjectRecord>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {COLS} FROM projects WHERE path = ?1"),
                params![path],
                row_to_project,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    pub fn delete_project(&self, path: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM projects WHERE path = ?1", params![path])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(db: &Db, name: &str, source: ProjectSource) -> ProjectRecord {
        db.insert_project(NewProject {
            path: format!("/repos/{name}"),
            name: name.into(),
            origin_url: Some(format!("https://github.com/acme/{name}.git")),
            source,
        })
        .unwrap()
    }

    fn names(db: &Db) -> Vec<String> {
        db.list_projects()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    #[test]
    fn project_registry_round_trip() {
        let db = Db::open_in_memory().unwrap();
        add(&db, "beta", ProjectSource::Clone);
        add(&db, "alpha", ProjectSource::Disk);

        // New projects append at the bottom (position = MAX+1), so the list keeps
        // insertion order rather than sorting alphabetically.
        let listed = db.list_projects().unwrap();
        assert_eq!(
            listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["beta", "alpha"]
        );
        assert_eq!(listed[0].source, ProjectSource::Clone);

        // Idempotent upsert: re-adding refreshes, doesn't duplicate or reorder.
        db.insert_project(NewProject {
            path: "/repos/alpha".into(),
            name: "alpha".into(),
            origin_url: None,
            source: ProjectSource::Clone,
        })
        .unwrap();
        let got = db.get_project_record("/repos/alpha").unwrap().unwrap();
        assert_eq!(got.origin_url, None);
        assert_eq!(got.source, ProjectSource::Clone);
        assert_eq!(db.list_projects().unwrap().len(), 2);
        assert_eq!(names(&db), ["beta", "alpha"]); // position preserved on conflict

        db.delete_project("/repos/alpha").unwrap();
        assert_eq!(db.list_projects().unwrap().len(), 1);
        assert!(db.get_project_record("/repos/alpha").unwrap().is_none());
    }

    #[test]
    fn folder_projects_round_trip_and_unknown_sources_read_as_disk() {
        let db = Db::open_in_memory().unwrap();
        db.insert_project(NewProject {
            path: "/notes".into(),
            name: "notes".into(),
            origin_url: None,
            source: ProjectSource::Folder,
        })
        .unwrap();
        let got = db.get_project_record("/notes").unwrap().unwrap();
        assert_eq!(got.source, ProjectSource::Folder);
        assert!(!got.source.is_git());

        // A value from a newer schema must never read as `Folder` — that would
        // silently strip a real repo's git surfaces.
        assert_eq!(ProjectSource::parse("worktree"), ProjectSource::Disk);
        assert!(ProjectSource::parse("worktree").is_git());
    }

    #[test]
    fn reorder_projects_sets_explicit_order() {
        let db = Db::open_in_memory().unwrap();
        add(&db, "one", ProjectSource::Clone); // position 0
        add(&db, "two", ProjectSource::Clone); // position 1
        add(&db, "three", ProjectSource::Clone); // position 2
        assert_eq!(names(&db), ["one", "two", "three"]);

        // Drag "three" to the top.
        db.reorder_projects(&[
            "/repos/three".into(),
            "/repos/one".into(),
            "/repos/two".into(),
        ])
        .unwrap();
        assert_eq!(names(&db), ["three", "one", "two"]);

        // A project added after a manual reorder appends at the bottom.
        add(&db, "four", ProjectSource::Clone);
        assert_eq!(names(&db), ["three", "one", "two", "four"]);

        // An unknown path is a tolerated no-op; the rest of the order still applies.
        db.reorder_projects(&[
            "/repos/missing".into(),
            "/repos/one".into(),
            "/repos/two".into(),
            "/repos/three".into(),
            "/repos/four".into(),
        ])
        .unwrap();
        assert_eq!(names(&db), ["one", "two", "three", "four"]);
    }
}
