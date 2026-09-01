use rusqlite::{params, Row};

use super::{now_ms, Db};
use crate::error::Result;

pub use hyprlayer_models::project::{ProjectRecord, ProjectSource};

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
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS} FROM projects ORDER BY position, name"
            ))?;
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

        let listed = db.list_projects().unwrap();
        assert_eq!(
            listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["beta", "alpha"]
        );
        assert_eq!(listed[0].source, ProjectSource::Clone);

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
        assert_eq!(names(&db), ["beta", "alpha"]);

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

        assert_eq!(ProjectSource::parse("worktree"), ProjectSource::Disk);
        assert!(ProjectSource::parse("worktree").is_git());
    }

    #[test]
    fn reorder_projects_sets_explicit_order() {
        let db = Db::open_in_memory().unwrap();
        add(&db, "one", ProjectSource::Clone);
        add(&db, "two", ProjectSource::Clone);
        add(&db, "three", ProjectSource::Clone);
        assert_eq!(names(&db), ["one", "two", "three"]);

        db.reorder_projects(&[
            "/repos/three".into(),
            "/repos/one".into(),
            "/repos/two".into(),
        ])
        .unwrap();
        assert_eq!(names(&db), ["three", "one", "two"]);

        add(&db, "four", ProjectSource::Clone);
        assert_eq!(names(&db), ["three", "one", "two", "four"]);

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
