use rusqlite::Connection;

/// Ordered, append-only migrations applied via PRAGMA user_version.
/// Never edit a shipped step — add a new one.
const MIGRATIONS: &[&str] = &[
    // v1: work items + (forward-declared for phase 3) sessions and the
    // orphan-process ledger.
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
    // v2: "work items" became "workstreams" — rename tables/columns in
    // place; existing rows survive. (v1's SQL text above must never change:
    // fresh installs run v1 then v2 and converge on the same schema.)
    "
    ALTER TABLE work_items RENAME TO workstreams;
    ALTER TABLE work_item_docs RENAME TO workstream_docs;
    ALTER TABLE workstream_docs RENAME COLUMN work_item_id TO workstream_id;
    ALTER TABLE sessions RENAME COLUMN work_item_id TO workstream_id;
    ",
    // v3: the project registry. A project is an explicit, app-owned clone the
    // user added (cloned from a URL or imported from disk) — the source of
    // truth for the project list, replacing the old derive-from-repoMappings
    // approach (which surfaced every worktree's `thoughts init` as a project).
    "
    CREATE TABLE projects (
        path        TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        origin_url  TEXT,
        source      TEXT NOT NULL DEFAULT 'clone',
        created_at  REAL NOT NULL
    );
    ",
    // v4: workstreams can now be created three ways. `worktree_mode` records
    // which, so the archive/delete lifecycle knows whether the branch + dir are
    // app-owned (safe to drop) or the user's. Existing rows are the original
    // behavior: a new branch in a dedicated worktree → 'isolated'.
    //   isolated — new branch <prefix><slug> in a new worktree (the default)
    //   existing — a user-chosen local branch checked out into a new worktree
    //   inplace  — no worktree; the project's own checkout on a chosen branch
    "
    ALTER TABLE workstreams ADD COLUMN worktree_mode TEXT NOT NULL DEFAULT 'isolated';
    ",
    // v5: which harness backs a session. Existing rows predate OpenCode, so they
    // are Claude Code. `claude_session_id` keeps its name but is now the generic
    // harness session id (claude's for Claude, the opencode session id for OpenCode).
    "
    ALTER TABLE sessions ADD COLUMN harness TEXT NOT NULL DEFAULT 'claude';
    ",
    // v6: user-defined project ordering for the sidebar. `position` is the
    // drag-to-reorder rank; `list_projects` sorts by (position, name). Existing
    // rows default to 0 and so keep tiebreaking by name — upgrades preserve
    // today's alphabetical order until the first manual reorder. New projects
    // append at the bottom (position = MAX+1; see `insert_project`).
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
