use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE work_items (
        id            TEXT PRIMARY KEY,
        project_path  TEXT NOT NULL,
        title         TEXT NOT NULL,
        slug          TEXT NOT NULL,
        branch        TEXT NOT NULL,
        worktree_path TEXT NOT NULL,
        base_branch   TEXT NOT NULL,
        stage         TEXT NOT NULL DEFAULT 'general',
        status        TEXT NOT NULL DEFAULT 'active',
        pr_number     INTEGER,
        created_at    REAL NOT NULL,
        updated_at    REAL NOT NULL,
        archived_at   REAL
    );
    CREATE INDEX idx_work_items_project ON work_items(project_path, status);

    CREATE TABLE work_item_docs (
        work_item_id  TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
        doc_path      TEXT NOT NULL,
        role          TEXT NOT NULL DEFAULT 'other',
        PRIMARY KEY (work_item_id, doc_path)
    );

    CREATE TABLE sessions (
        id                TEXT PRIMARY KEY,
        work_item_id      TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
        stage             TEXT NOT NULL,
        claude_session_id TEXT,
        cwd               TEXT NOT NULL,
        state             TEXT NOT NULL,
        model             TEXT,
        cost_usd          REAL,
        started_at        REAL NOT NULL,
        ended_at          REAL,
        exit_summary      TEXT
    );
    CREATE INDEX idx_sessions_work_item ON sessions(work_item_id);

    CREATE TABLE session_events (
        session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        seq        INTEGER NOT NULL,
        kind       TEXT NOT NULL,
        payload    TEXT NOT NULL,
        created_at REAL NOT NULL,
        PRIMARY KEY (session_id, seq)
    );

    CREATE TABLE orphan_pids (
        pid        INTEGER NOT NULL,
        session_id TEXT NOT NULL,
        spawned_at REAL NOT NULL,
        PRIMARY KEY (pid, session_id)
    );
    ",
    "
    ALTER TABLE work_items RENAME TO workstreams;
    ALTER TABLE work_item_docs RENAME TO workstream_docs;
    ALTER TABLE workstream_docs RENAME COLUMN work_item_id TO workstream_id;
    ALTER TABLE sessions RENAME COLUMN work_item_id TO workstream_id;
    ",
    "
    CREATE TABLE projects (
        path        TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        origin_url  TEXT,
        source      TEXT NOT NULL DEFAULT 'clone',
        created_at  REAL NOT NULL
    );
    ",
    "
    ALTER TABLE workstreams ADD COLUMN worktree_mode TEXT NOT NULL DEFAULT 'isolated';
    ",
    "
    ALTER TABLE sessions ADD COLUMN harness TEXT NOT NULL DEFAULT 'claude';
    ",
    "
    ALTER TABLE projects ADD COLUMN position REAL NOT NULL DEFAULT 0;
    ",
    "
    CREATE TABLE workstream_archives (
        workstream_id  TEXT PRIMARY KEY REFERENCES workstreams(id) ON DELETE CASCADE,
        archived_at    REAL NOT NULL,
        purge_after    REAL NOT NULL,
        kept_at        REAL,
        size_bytes     REAL,
        uncommitted    INTEGER,
        unpushed       INTEGER,
        scanned_at     REAL,
        pr_number      INTEGER,
        pr_state       TEXT,
        pr_checked_at  REAL
    );
    ",
    "
    ALTER TABLE sessions ADD COLUMN title TEXT;
    ",
    "
    ALTER TABLE sessions ADD COLUMN first_input TEXT;
    ",
    "
    ALTER TABLE sessions ADD COLUMN archived_at REAL;
    ",
    "
    ALTER TABLE sessions ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE sessions ADD COLUMN reset_rev INTEGER NOT NULL DEFAULT 0;
    UPDATE sessions SET rev = (
        SELECT COUNT(*) FROM session_events e
         WHERE e.session_id = sessions.id
           AND e.kind IN ('userMessage', 'assistantText', 'toolResult', 'rewound')
    );
    ",
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let latest = MIGRATIONS.len() as u32;
    if current > latest {
        tracing::warn!(
            "database schema {current} is newer than this build's {latest}: a newer Hyprlayer wrote it"
        );
    }
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as u32;
        if current < version {
            tracing::info!("migrating the database schema to {version}");
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", version)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;

    /// Schema v10, the state migration 11 backfills from.
    fn v10_schema() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..10] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 10).unwrap();
        conn
    }

    fn seed_workstream(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO workstreams (id, project_path, title, slug, branch, worktree_path,
                                       base_branch, created_at, updated_at)
             VALUES (?1, '/p', 't', 't', 'b', '/wt', 'main', 0, 0)",
            params![id],
        )
        .unwrap();
    }

    fn seed_session(conn: &Connection, id: &str, workstream_id: &str) {
        conn.execute(
            "INSERT INTO sessions (id, workstream_id, stage, cwd, state, started_at)
             VALUES (?1, ?2, 'general', '/wt', 'idle', 0)",
            params![id, workstream_id],
        )
        .unwrap();
    }

    fn seed_events(conn: &Connection, session_id: &str, kinds: &[&str]) {
        for (seq, kind) in kinds.iter().enumerate() {
            conn.execute(
                "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                 VALUES (?1, ?2, ?3, '{}', 0)",
                params![session_id, seq as i64, kind],
            )
            .unwrap();
        }
    }

    fn rev_of(conn: &Connection, id: &str) -> i64 {
        conn.query_row("SELECT rev FROM sessions WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn bookkeeping_kinds_are_not_counted() {
        let conn = v10_schema();
        seed_workstream(&conn, "w1");
        seed_session(&conn, "s1", "w1");
        seed_events(
            &conn,
            "s1",
            &["init", "usage", "turnDone", "toolUse", "thinking"],
        );
        migrate(&conn).unwrap();
        assert_eq!(rev_of(&conn, "s1"), 0);
    }

    #[test]
    fn rewound_rows_are_counted() {
        let conn = v10_schema();
        seed_workstream(&conn, "w1");
        seed_session(&conn, "s1", "w1");
        // toolUse is bookkeeping; userMessage and rewound both advance rev.
        seed_events(&conn, "s1", &["userMessage", "toolUse", "rewound"]);
        migrate(&conn).unwrap();
        assert_eq!(rev_of(&conn, "s1"), 2);
    }

    #[test]
    fn a_session_with_no_events_backfills_to_zero() {
        let conn = v10_schema();
        seed_workstream(&conn, "w1");
        seed_session(&conn, "s1", "w1");
        migrate(&conn).unwrap();
        assert_eq!(rev_of(&conn, "s1"), 0);
    }

    #[test]
    fn another_sessions_rows_do_not_leak_into_the_count() {
        let conn = v10_schema();
        seed_workstream(&conn, "w1");
        seed_session(&conn, "s1", "w1");
        seed_session(&conn, "s2", "w1");
        seed_events(&conn, "s1", &["userMessage"]);
        seed_events(
            &conn,
            "s2",
            &["userMessage", "assistantText", "userMessage"],
        );
        migrate(&conn).unwrap();
        assert_eq!(rev_of(&conn, "s1"), 1);
        assert_eq!(rev_of(&conn, "s2"), 3);
    }

    #[test]
    fn reset_rev_backfills_to_zero() {
        let conn = v10_schema();
        seed_workstream(&conn, "w1");
        seed_session(&conn, "s1", "w1");
        seed_events(&conn, "s1", &["userMessage", "rewound", "userMessage"]);
        migrate(&conn).unwrap();
        let reset_rev: i64 = conn
            .query_row("SELECT reset_rev FROM sessions WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(reset_rev, 0);
    }

    #[test]
    fn user_version_reaches_11() {
        let conn = v10_schema();
        migrate(&conn).unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 11);
    }

    /// This phase's manual check: the backfill against a real profile copy, never a runtime
    /// (which would sweep expired archives and purge real worktree paths on open).
    #[test]
    #[ignore = "needs HYPRLAYER_MIGRATE_COPY"]
    fn backfills_a_copied_profile() {
        let Ok(path) = std::env::var("HYPRLAYER_MIGRATE_COPY") else {
            return;
        };
        let conn = Connection::open(&path).unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 10, "expected a v10 profile copy at {path}");

        migrate(&conn).unwrap();

        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 11);

        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.rev, s.reset_rev,
                        (SELECT COUNT(*) FROM session_events e
                          WHERE e.session_id = s.id
                            AND e.kind IN ('userMessage','assistantText','toolResult','rewound'))
                   FROM sessions s",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .unwrap();

        let mut checked = 0;
        for row in rows {
            let (id, rev, reset_rev, expected) = row.unwrap();
            assert_eq!(
                rev, expected,
                "session {id}: rev does not match its four-kind count"
            );
            assert_eq!(reset_rev, 0, "session {id}: reset_rev should backfill to 0");
            checked += 1;
        }
        println!("checked {checked} session(s) in the copied profile");
    }
}
