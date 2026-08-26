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
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as u32;
        if current < version {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", version)?;
        }
    }
    Ok(())
}
