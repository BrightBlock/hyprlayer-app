use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::{now_ms, Db};
use crate::error::Result;

/// A registered project: an app-owned clone the user added deliberately, via
/// cloning a hosted repo or importing one from disk. This registry — not the
/// thoughts config's `repoMappings` — is the source of truth for the project
/// list, so the per-worktree `thoughts init` entries never surface as projects.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    /// Absolute path of the canonical clone (`~/hyprlayer/repos/<name>`).
    pub path: String,
    pub name: String,
    /// The git source it tracks; None for a local-only repo with no remote.
    pub origin_url: Option<String>,
    /// `clone` (from a URL) | `disk` (imported from a local repo).
    pub source: String,
    pub created_at: f64,
}

pub struct NewProject {
    pub path: String,
    pub name: String,
    pub origin_url: Option<String>,
    pub source: String,
}

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<ProjectRecord> {
    Ok(ProjectRecord {
        path: row.get("path")?,
        name: row.get("name")?,
        origin_url: row.get("origin_url")?,
        source: row.get("source")?,
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
            conn.execute(
                "INSERT INTO projects (path, name, origin_url, source, created_at)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(path) DO UPDATE SET
                     name = excluded.name,
                     origin_url = excluded.origin_url,
                     source = excluded.source",
                params![
                    rec.path,
                    rec.name,
                    rec.origin_url,
                    rec.source,
                    rec.created_at
                ],
            )?;
            Ok(())
        })?;
        Ok(rec)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRecord>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM projects ORDER BY name"))?;
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

    fn add(db: &Db, name: &str, source: &str) -> ProjectRecord {
        db.insert_project(NewProject {
            path: format!("/repos/{name}"),
            name: name.into(),
            origin_url: Some(format!("https://github.com/acme/{name}.git")),
            source: source.into(),
        })
        .unwrap()
    }

    #[test]
    fn project_registry_round_trip() {
        let db = Db::open_in_memory().unwrap();
        add(&db, "beta", "clone");
        add(&db, "alpha", "disk");

        // Listed alphabetically by name.
        let listed = db.list_projects().unwrap();
        assert_eq!(
            listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(listed[0].source, "disk");

        // Idempotent upsert: re-adding refreshes, doesn't duplicate.
        db.insert_project(NewProject {
            path: "/repos/alpha".into(),
            name: "alpha".into(),
            origin_url: None,
            source: "clone".into(),
        })
        .unwrap();
        let got = db.get_project_record("/repos/alpha").unwrap().unwrap();
        assert_eq!(got.origin_url, None);
        assert_eq!(got.source, "clone");
        assert_eq!(db.list_projects().unwrap().len(), 2);

        db.delete_project("/repos/alpha").unwrap();
        assert_eq!(db.list_projects().unwrap().len(), 1);
        assert!(db.get_project_record("/repos/alpha").unwrap().is_none());
    }
}
