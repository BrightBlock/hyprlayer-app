use rusqlite::types::ValueRef;
use rusqlite::{params, OptionalExtension, Row, ToSql, Transaction};
use serde::{Deserialize, Serialize};

use super::session_event_codec::{decode_raw_event, encode_raw_event};
use super::{now_ms, Db};
use crate::agent::Harness;
use crate::claude::{ExitOutcome, PermissionMode, SessionEvent, SessionState};
use crate::error::{CoreError, Result};
use crate::pipeline::{self, Stage};

use hyprlayer_models::agent_control::AgentCaller;
use hyprlayer_models::session::ClientRef;
pub use hyprlayer_models::session::{LogCursor, Session, SessionLogEntry, SessionOrigin};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub seq: f64,
    pub kind: String,
    pub payload: String,
    pub created_at: f64,
}

impl TryFrom<StoredEvent> for SessionLogEntry {
    type Error = CoreError;

    fn try_from(stored: StoredEvent) -> Result<Self> {
        if !stored.created_at.is_finite() {
            return Err(CoreError::SessionEventDecode {
                detail: format!("seq {} carries a non-finite createdAt", stored.seq),
            });
        }
        Ok(SessionLogEntry {
            seq: stored.seq as i64,
            kind: stored.kind,
            event: decode_raw_event(&stored.payload)?,
            created_at: stored.created_at,
        })
    }
}

pub fn decode_session_log(session_id: &str, rows: Vec<StoredEvent>) -> Vec<SessionLogEntry> {
    rows.into_iter()
        .filter_map(|stored| {
            let seq = stored.seq;
            match SessionLogEntry::try_from(stored) {
                Ok(entry) => Some(entry),
                Err(err) => {
                    tracing::warn!("session {session_id}: skipping event {seq}: {err}");
                    None
                }
            }
        })
        .collect()
}

pub struct NewSession {
    pub id: String,
    pub workstream_id: String,
    pub stage: Stage,
    pub harness: Harness,
    pub cwd: String,
    /// The user's text before `compose_prompt` wrapped it.
    pub first_input: Option<String>,
}

/// id, stage, origin, first_turn and first_input of a start no harness ever ran.
type UnlaunchedRow = (String, String, String, Option<String>, Option<String>);

/// A start's first turn as it was composed, so a failure can record it and a retry resend it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FirstTurnRecord {
    pub text: String,
    pub prompt_name: Option<String>,
    pub client_message_id: String,
}

/// What a session's title is generated from.
pub struct SessionTitleSource {
    pub workstream_id: String,
    pub input: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum FieldOp<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionRowMutation {
    pub provider_session_id: FieldOp<String>,
    pub model: FieldOp<String>,
    pub state: FieldOp<SessionState>,
    pub cost_usd: FieldOp<f64>,
    pub ended_at: FieldOp<f64>,
    pub exit_summary: FieldOp<String>,
    pub clear_orphans: bool,
}

/// Kinds whose commit advances `sessions.rev`. Migration 11 backfilled all but `exited`, which a
/// viewer with no stream would otherwise never be told of.
pub(crate) const REV_KINDS: [&str; 5] = [
    "userMessage",
    "assistantText",
    "toolResult",
    "rewound",
    "exited",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ApplySessionEventOutcome {
    /// `rev` is the session's new rev when this row advanced it.
    Applied {
        rev: Option<u64>,
        created_at: f64,
    },
    AlreadyApplied {
        created_at: f64,
    },
}

impl ApplySessionEventOutcome {
    /// The row's `created_at`, which a client's cursor names it by.
    pub(crate) fn created_at(&self) -> f64 {
        match self {
            Self::Applied { created_at, .. } | Self::AlreadyApplied { created_at } => *created_at,
        }
    }
}

/// One page of raw rows; `last` is the final row read, decodable or not.
pub(crate) struct SessionLogPage {
    pub entries: Vec<SessionLogEntry>,
    pub last: Option<(i64, f64)>,
    pub remaining: u32,
    /// A backward page's first raw row, while rows before it remain.
    pub earlier: Option<(i64, f64)>,
}

/// Where a page starts: the rows after a seq, or the newest rows before one.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PageFrom {
    After(Option<i64>),
    Before(Option<i64>),
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) type SessionEventGateFn = dyn Fn(&str, i64) -> SessionEventGateAction + Send + Sync;

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
pub enum SessionEventGateAction {
    Proceed,
    Delay(std::time::Duration),
    FailRowMutation,
}

struct SetBuilder<'a> {
    session_id: &'a str,
    clauses: Vec<String>,
    values: Vec<Box<dyn ToSql>>,
}

impl<'a> SetBuilder<'a> {
    fn new(session_id: &'a str) -> Self {
        Self {
            session_id,
            clauses: Vec::new(),
            values: Vec::new(),
        }
    }

    fn set<T: ToSql + 'static>(&mut self, column: &str, value: T) {
        self.values.push(Box::new(value));
        self.clauses
            .push(format!("{column} = ?{}", self.values.len()));
    }

    fn clear(&mut self, column: &str) {
        self.clauses.push(format!("{column} = NULL"));
    }

    fn apply(self, tx: &Transaction<'_>) -> rusqlite::Result<()> {
        if self.clauses.is_empty() {
            return Ok(());
        }
        let sql = format!(
            "UPDATE sessions SET {} WHERE id = ?{}",
            self.clauses.join(", "),
            self.values.len() + 1
        );
        let mut params: Vec<&dyn ToSql> = self.values.iter().map(|v| v.as_ref()).collect();
        params.push(&self.session_id);
        tx.execute(&sql, params.as_slice())?;
        Ok(())
    }
}

fn apply_row_mutation(
    tx: &Transaction<'_>,
    session_id: &str,
    mutation: &SessionRowMutation,
) -> rusqlite::Result<()> {
    let mut builder = SetBuilder::new(session_id);
    match &mutation.provider_session_id {
        FieldOp::Unchanged => {}
        FieldOp::Set(v) => builder.set("claude_session_id", v.clone()),
        FieldOp::Clear => builder.clear("claude_session_id"),
    }
    match &mutation.model {
        FieldOp::Unchanged => {}
        FieldOp::Set(v) => builder.set("model", v.clone()),
        FieldOp::Clear => builder.clear("model"),
    }
    match &mutation.state {
        FieldOp::Unchanged => {}
        FieldOp::Set(v) => builder.set("state", v.as_str()),
        FieldOp::Clear => builder.clear("state"),
    }
    match &mutation.cost_usd {
        FieldOp::Unchanged => {}
        FieldOp::Set(v) => builder.set("cost_usd", *v),
        FieldOp::Clear => builder.clear("cost_usd"),
    }
    match &mutation.ended_at {
        FieldOp::Unchanged => {}
        FieldOp::Set(v) => builder.set("ended_at", *v),
        FieldOp::Clear => builder.clear("ended_at"),
    }
    match &mutation.exit_summary {
        FieldOp::Unchanged => {}
        FieldOp::Set(v) => builder.set("exit_summary", v.clone()),
        FieldOp::Clear => builder.clear("exit_summary"),
    }
    builder.apply(tx)?;
    if mutation.clear_orphans {
        tx.execute(
            "DELETE FROM orphan_pids WHERE session_id = ?1",
            params![session_id],
        )?;
    }
    Ok(())
}

const COLS: &str = "id, workstream_id, stage, harness, claude_session_id, cwd, state, model, \
                    cost_usd, started_at, ended_at, exit_summary, title, archived_at, \
                    origin, parent_session_id, effort, requested_permission_mode, \
                    (SELECT e.kind FROM session_events e
                      WHERE e.session_id = sessions.id AND e.kind IN ('userMessage', 'turnDone')
                      ORDER BY e.seq DESC LIMIT 1) AS last_turn_kind, \
                    (SELECT e.created_at FROM session_events e WHERE e.session_id = sessions.id
                      ORDER BY e.seq DESC LIMIT 1) AS last_active_at, \
                    (SELECT json_object('id', g.id, 'status', g.status, 'step', g.step,
                                        'pauseReason', g.pause_reason, 'iteration', g.iteration,
                                        'maxIterations', g.max_iterations)
                       FROM goals g
                      WHERE g.session_id = sessions.id AND g.status IN ('running', 'paused')
                      ORDER BY g.started_at DESC LIMIT 1) AS goal_brief";

const UNANSWERED_WAKES: &str = "CASE WHEN EXISTS (SELECT 1 FROM session_events e
                                     WHERE e.session_id = sessions.id AND e.kind = 'turnDone'
                                       AND e.seq > sessions.restart_wake_seq)
                          THEN 0 ELSE sessions.restart_wakes END";

/// `TurnDone` persists `Running`, so a live idle harness is told apart by its log.
fn is_working(state: SessionState, last_turn_kind: Option<&str>) -> bool {
    match state {
        SessionState::Starting
        | SessionState::AwaitingPermission
        | SessionState::AwaitingQuestion => true,
        SessionState::Running => last_turn_kind == Some("userMessage"),
        _ => false,
    }
}

const LEAD_ORIGIN: &str = "lead";

pub struct LeadLink {
    pub id: String,
    pub workstream_id: String,
    pub lead: bool,
    pub spawn_parent: Option<String>,
    pub started_at: f64,
}

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    let state = SessionState::parse(&row.get::<_, String>("state")?);
    let origin: String = row.get("origin")?;
    let last_turn_kind: Option<String> = row.get("last_turn_kind")?;
    let goal_brief: Option<String> = row.get("goal_brief")?;
    Ok(Session {
        id: row.get("id")?,
        workstream_id: row.get("workstream_id")?,
        stage: Stage::parse(&row.get::<_, String>("stage")?),
        harness: Harness::parse(&row.get::<_, String>("harness")?),
        provider_session_id: row.get("claude_session_id")?,
        cwd: row.get("cwd")?,
        state,
        model: row.get("model")?,
        cost_usd: row.get("cost_usd")?,
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        exit_summary: row.get("exit_summary")?,
        title: row.get("title")?,
        preview: None,
        archived_at: row.get("archived_at")?,
        working: is_working(state, last_turn_kind.as_deref()),
        last_active_at: row.get("last_active_at")?,
        goal: goal_brief
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        origin: SessionOrigin::parse(&origin),
        parent_session_id: row.get("parent_session_id")?,
        effort: row.get("effort")?,
        requested_permission_mode: row
            .get::<_, Option<String>>("requested_permission_mode")?
            .and_then(|m| PermissionMode::try_parse(&m)),
        // A Lead is a user's session; `origin = 'lead'` parses as `User` on an older server too.
        lead: origin == LEAD_ORIGIN,
    })
}

const PREVIEW_CHARS: usize = 120;

/// Correlated subquery over `sessions`: its first `userMessage` payload.
const FIRST_PROMPT: &str = "(SELECT payload FROM session_events e
                             WHERE e.session_id = sessions.id AND e.kind = 'userMessage'
                             ORDER BY e.seq LIMIT 1) AS first_prompt";

/// The user's own first text: `first_input`, else recovered from the first
/// turn for rows that predate it.
fn user_input(row: &Row<'_>) -> rusqlite::Result<Option<String>> {
    if let Some(input) = row.get::<_, Option<String>>("first_input")? {
        return Ok(Some(input));
    }
    // Strict: `Stage::parse` reads a newer server's stage as General.
    let Some(stage) = Stage::try_parse(&row.get::<_, String>("stage")?) else {
        return Ok(None);
    };
    let payload: Option<String> = row.get("first_prompt")?;
    Ok(payload.as_deref().and_then(|payload| {
        let SessionEvent::UserMessage { text, .. } = decode_raw_event(payload).ok()? else {
            return None;
        };
        pipeline::recover_input(stage, &text)
    }))
}

/// Flattens text to one line, capped at `PREVIEW_CHARS`.
fn preview_of(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = flat.chars().take(PREVIEW_CHARS).collect();
    if cut.len() < flat.len() {
        cut.push('…');
    }
    (!cut.is_empty()).then_some(cut)
}

impl Db {
    pub fn insert_session(&self, new: NewSession) -> Result<Session> {
        let session = Session {
            id: new.id,
            workstream_id: new.workstream_id,
            stage: new.stage,
            harness: new.harness,
            provider_session_id: None,
            cwd: new.cwd,
            state: SessionState::Starting,
            model: None,
            cost_usd: None,
            started_at: now_ms(),
            ended_at: None,
            exit_summary: None,
            title: None,
            preview: None,
            archived_at: None,
            working: true,
            last_active_at: None,
            goal: None,
            origin: SessionOrigin::User,
            parent_session_id: None,
            effort: None,
            requested_permission_mode: None,
            lead: false,
        };
        self.with(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, workstream_id, stage, harness, claude_session_id, cwd,
                                       state, model, cost_usd, started_at, ended_at, exit_summary,
                                       first_input)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    session.id,
                    session.workstream_id,
                    session.stage.as_str(),
                    session.harness.as_str(),
                    session.provider_session_id,
                    session.cwd,
                    session.state.as_str(),
                    session.model,
                    session.cost_usd,
                    session.started_at,
                    session.ended_at,
                    session.exit_summary,
                    new.first_input,
                ],
            )?;
            Ok(())
        })?;
        Ok(session)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {COLS} FROM sessions WHERE id = ?1"),
                params![id],
                row_to_session,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    /// A session with no recoverable user text keeps a `None` preview and the
    /// caller falls back to the stage label.
    pub fn list_sessions(&self, workstream_id: &str) -> Result<Vec<Session>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COLS}, first_input, {FIRST_PROMPT}
                   FROM sessions WHERE workstream_id = ?1 ORDER BY started_at"
            ))?;
            let rows = stmt.query_map(params![workstream_id], |row| {
                let mut session = row_to_session(row)?;
                session.preview = user_input(row)?.as_deref().and_then(preview_of);
                Ok(session)
            })?;
            rows.collect()
        })
    }

    pub fn lead_links(&self, project_path: &str) -> Result<Vec<LeadLink>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.workstream_id, s.origin, s.parent_session_id, s.started_at
                   FROM sessions s JOIN workstreams w ON w.id = s.workstream_id
                  WHERE w.project_path = ?1 AND w.status = 'active' AND s.archived_at IS NULL",
            )?;
            let rows = stmt.query_map(params![project_path], |row| {
                let origin: String = row.get("origin")?;
                let parent: Option<String> = row.get("parent_session_id")?;
                Ok(LeadLink {
                    id: row.get("id")?,
                    workstream_id: row.get("workstream_id")?,
                    lead: origin == LEAD_ORIGIN,
                    spawn_parent: parent
                        .filter(|_| SessionOrigin::parse(&origin) == SessionOrigin::Agent),
                    started_at: row.get("started_at")?,
                })
            })?;
            rows.collect()
        })
    }

    /// `None` once the session is titled or has no user text to title it from.
    pub fn session_title_source(&self, id: &str) -> Result<Option<SessionTitleSource>> {
        self.with(|conn| {
            conn.query_row(
                &format!(
                    "SELECT workstream_id, stage, first_input, {FIRST_PROMPT}
                       FROM sessions WHERE id = ?1 AND title IS NULL"
                ),
                params![id],
                |row| {
                    let Some(input) = user_input(row)? else {
                        return Ok(None);
                    };
                    Ok(Some(SessionTitleSource {
                        workstream_id: row.get("workstream_id")?,
                        input,
                    }))
                },
            )
            .optional()
            .map(Option::flatten)
        })
    }

    pub fn set_session_state(&self, id: &str, state: SessionState) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET state = ?2 WHERE id = ?1",
                params![id, state.as_str()],
            )?;
            Ok(())
        })
    }

    pub fn set_session_model(&self, id: &str, model: Option<&str>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET model = ?2 WHERE id = ?1",
                params![id, model],
            )?;
            Ok(())
        })
    }

    /// The model and effort belong to the old harness, so they go with it.
    pub fn set_session_harness(
        &self,
        id: &str,
        harness: Harness,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET harness = ?2, model = ?3, effort = ?4 WHERE id = ?1",
                params![id, harness.as_str(), model, effort],
            )?;
            Ok(())
        })
    }

    pub fn set_session_requested_permission_mode(
        &self,
        id: &str,
        mode: Option<PermissionMode>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET requested_permission_mode = ?2 WHERE id = ?1",
                params![id, mode.map(PermissionMode::as_str)],
            )?;
            Ok(())
        })
    }

    /// A mode the harness reports switching to. `false`, and nothing written, when the session
    /// asks for Auto and the report is only Auto's resolution, the stage's own mode.
    pub fn record_reported_permission_mode(&self, id: &str, mode: PermissionMode) -> Result<bool> {
        self.with(|conn| {
            let row: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT stage, requested_permission_mode FROM sessions WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((stage, requested)) = row else {
                return Ok(true);
            };
            let auto = requested
                .as_deref()
                .and_then(PermissionMode::try_parse)
                .is_none();
            if auto && mode == pipeline::policy_for(Stage::parse(&stage)).permission_mode {
                return Ok(false);
            }
            conn.execute(
                "UPDATE sessions SET requested_permission_mode = ?2 WHERE id = ?1",
                params![id, mode.as_str()],
            )?;
            Ok(true)
        })
    }

    pub fn session_first_input(&self, id: &str) -> Result<Option<String>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT first_input FROM sessions WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
        })
    }

    pub fn session_effort(&self, id: &str) -> Result<Option<String>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT effort FROM sessions WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
    }

    pub fn set_session_effort(&self, id: &str, effort: Option<&str>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET effort = ?2 WHERE id = ?1",
                params![id, effort],
            )?;
            Ok(())
        })
    }

    pub fn set_session_lineage(
        &self,
        id: &str,
        origin: SessionOrigin,
        parent_session_id: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET origin = ?2, parent_session_id = ?3 WHERE id = ?1",
                params![id, origin.as_str(), parent_session_id],
            )?;
            Ok(())
        })
    }

    pub fn mark_session_lead(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET origin = ?2 WHERE id = ?1",
                params![id, LEAD_ORIGIN],
            )?;
            Ok(())
        })
    }

    pub fn set_session_title(&self, id: &str, title: Option<&str>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET title = ?2 WHERE id = ?1",
                params![id, title],
            )?;
            Ok(())
        })
    }

    /// Never overwrites a title, so a rename that lands first wins.
    pub fn set_session_title_if_unset(&self, id: &str, title: &str) -> Result<bool> {
        self.with(|conn| {
            let changed = conn.execute(
                "UPDATE sessions SET title = ?2 WHERE id = ?1 AND title IS NULL",
                params![id, title],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn set_session_archived(&self, id: &str, archived: bool) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET archived_at = CASE WHEN ?2 THEN COALESCE(archived_at, ?3) END
                  WHERE id = ?1",
                params![id, archived, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM session_events WHERE session_id = ?1",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM held_messages WHERE session_id = ?1",
                params![id],
            )?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
            tx.commit()
        })
    }

    /// A row whose harness the app detached for good: idle like a settled one, with an end time.
    pub fn settle_session(&self, id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET state = ?2, ended_at = ?3 WHERE id = ?1",
                params![id, SessionState::Idle.as_str(), now_ms()],
            )?;
            Ok(())
        })
    }

    /// Rows a previous server left live; their harnesses died with it. Queues a wake for each mid-turn one and counts them.
    pub fn settle_stale_sessions(&self) -> Result<usize> {
        let stale = [
            SessionState::Starting.as_str(),
            SessionState::Running.as_str(),
            SessionState::AwaitingPermission.as_str(),
            SessionState::AwaitingQuestion.as_str(),
        ];
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mid_turn = tx
                .prepare(&format!(
                    "SELECT {COLS} FROM sessions WHERE state IN (?1, ?2, ?3, ?4)"
                ))?
                .query_map(stale, row_to_session)?
                .filter_map(|row| row.map(|s| s.working.then_some(s.id)).transpose())
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for id in &mid_turn {
                tx.execute(
                    "INSERT INTO pending_wakes (session_id, mid_turn) VALUES (?1, 1)
                     ON CONFLICT(session_id) DO UPDATE SET mid_turn = 1",
                    params![id],
                )?;
            }
            tx.execute(
                "UPDATE sessions SET state = ?1 WHERE state IN (?2, ?3, ?4, ?5)",
                params![
                    SessionState::Idle.as_str(),
                    stale[0],
                    stale[1],
                    stale[2],
                    stale[3]
                ],
            )?;
            tx.commit()?;
            Ok(mid_turn.len())
        })
    }

    /// A start the previous run never launched ends failed, keeping its first turn, as a failed launch does.
    pub fn fail_unlaunched_sessions(&self, reason: &str) -> Result<usize> {
        let unlaunched: Vec<UnlaunchedRow> = self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, stage, origin, first_turn, first_input FROM sessions s WHERE state = ?1
                     AND NOT EXISTS (SELECT 1 FROM session_events e WHERE e.session_id = s.id)",
            )?;
            let rows = stmt.query_map(params![SessionState::Starting.as_str()], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
            rows.collect()
        })?;
        for (id, stage, origin, first_turn, input) in &unlaunched {
            // A goal's or an agent's start is discarded when it fails, as `start_detached` and goals do.
            if SessionOrigin::parse(origin) != SessionOrigin::User {
                self.delete_session(id)?;
                continue;
            }
            // Rows from before `first_turn` existed keep only the raw input.
            let turn = first_turn
                .as_deref()
                .and_then(|json| serde_json::from_str::<FirstTurnRecord>(json).ok())
                .or_else(|| {
                    input.clone().map(|text| FirstTurnRecord {
                        text,
                        prompt_name: None,
                        client_message_id: uuid::Uuid::new_v4().to_string(),
                    })
                });
            let prompt = turn.map(|turn| SessionEvent::UserMessage {
                text: turn.text,
                stage: Some(stage.clone()),
                prompt_name: turn.prompt_name,
                client_message_id: Some(turn.client_message_id),
                author: None,
            });
            self.append_failed_start(id, prompt, reason)?;
        }
        Ok(unlaunched.len())
    }

    /// The prompt and a `spawn_failed` exit after the session's last row, with the row ended to match.
    pub fn append_failed_start(
        &self,
        id: &str,
        prompt: Option<SessionEvent>,
        reason: &str,
    ) -> Result<()> {
        let exit = SessionEvent::Exited {
            outcome: ExitOutcome::SpawnFailed,
            code: None,
            stderr_tail: reason.to_string(),
        };
        let failed = SessionRowMutation {
            state: FieldOp::Set(SessionState::SpawnFailed),
            ended_at: FieldOp::Set(now_ms()),
            exit_summary: FieldOp::Set(reason.to_string()),
            ..Default::default()
        };
        let rows = prompt
            .into_iter()
            .map(|e| (e, SessionRowMutation::default()))
            .chain([(exit, failed)]);
        let first = self.next_session_seq(id)?;
        for (offset, (event, mutation)) in rows.enumerate() {
            let payload = encode_raw_event(&event)?;
            self.apply_session_event(id, first + offset as i64, event.kind(), &payload, mutation)?;
        }
        Ok(())
    }

    pub fn set_session_first_turn(&self, id: &str, turn: &FirstTurnRecord) -> Result<()> {
        let json =
            serde_json::to_string(turn).map_err(|e| CoreError::Io(std::io::Error::other(e)))?;
        self.with(|conn| {
            conn.execute(
                "UPDATE sessions SET first_turn = ?2 WHERE id = ?1",
                params![id, json],
            )?;
            Ok(())
        })
    }

    pub fn session_first_turn(&self, id: &str) -> Result<Option<FirstTurnRecord>> {
        let json: Option<String> = self.with(|conn| {
            conn.query_row(
                "SELECT first_turn FROM sessions WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
        })?;
        Ok(json.and_then(|json| serde_json::from_str(&json).ok()))
    }

    pub fn append_session_event(
        &self,
        session_id: &str,
        seq: i64,
        kind: &str,
        payload: &str,
    ) -> Result<()> {
        self.with(|conn| {
            let mut stmt = conn.prepare_cached(
                "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                 VALUES (?1,?2,?3,?4,?5)",
            )?;
            stmt.execute(params![session_id, seq, kind, payload, now_ms()])?;
            Ok(())
        })
    }

    pub fn session_log(&self, session_id: &str) -> Result<Vec<SessionLogEntry>> {
        Ok(decode_session_log(
            session_id,
            self.session_events(session_id)?,
        ))
    }

    pub(crate) fn session_log_after(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
    ) -> Result<Vec<SessionLogEntry>> {
        let rows = self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT seq, kind, payload, created_at FROM session_events
                 WHERE session_id = ?1 AND seq > ?2 ORDER BY seq",
            )?;
            let rows = stmt.query_map(params![session_id, after_seq.unwrap_or(-1)], |row| {
                Ok(StoredEvent {
                    seq: row.get::<_, i64>("seq")? as f64,
                    kind: row.get("kind")?,
                    payload: row.get("payload")?,
                    created_at: row.get("created_at")?,
                })
            })?;
            rows.collect()
        })?;
        Ok(decode_session_log(session_id, rows))
    }

    pub(crate) fn session_event_created_at(
        &self,
        session_id: &str,
        seq: i64,
    ) -> Result<Option<f64>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT created_at FROM session_events WHERE session_id = ?1 AND seq = ?2",
                params![session_id, seq],
                |row| row.get(0),
            )
            .optional()
        })
    }

    pub fn next_session_seq(&self, session_id: &str) -> Result<i64> {
        self.with(|conn| {
            let max: Option<i64> = conn.query_row(
                "SELECT MAX(seq) FROM session_events WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(max.map(|m| m + 1).unwrap_or(0))
        })
    }

    /// `(rev, reset_rev)`, or `(0, 0)` when the session has no row.
    pub(crate) fn session_rev(&self, session_id: &str) -> Result<(u64, u64)> {
        self.with(|conn| {
            let row = conn
                .query_row(
                    "SELECT rev, reset_rev FROM sessions WHERE id = ?1",
                    params![session_id],
                    |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
                )
                .optional()?;
            Ok(row.unwrap_or((0, 0)))
        })
    }

    /// `(rev, reset_rev, working)` for a session with no running actor; `None` without a row.
    pub(crate) fn session_rev_state(&self, session_id: &str) -> Result<Option<(u64, u64, bool)>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT rev, reset_rev, {COLS} FROM sessions WHERE id = ?1"),
                params![session_id],
                |row| {
                    let state = SessionState::parse(&row.get::<_, String>("state")?);
                    let last_turn_kind: Option<String> = row.get("last_turn_kind")?;
                    Ok((
                        row.get::<_, i64>("rev")? as u64,
                        row.get::<_, i64>("reset_rev")? as u64,
                        is_working(state, last_turn_kind.as_deref()),
                    ))
                },
            )
            .optional()
        })
    }

    /// Bumps `rev` and sets `reset_rev` to match, for a rewind boundary that
    /// failed to persist or a restore. Both right-hand `rev`s read the old value.
    pub(crate) fn bump_session_reset(&self, session_id: &str) -> Result<u64> {
        self.with(|conn| {
            let rev = conn
                .query_row(
                    "UPDATE sessions SET rev = rev + 1, reset_rev = rev + 1 WHERE id = ?1 RETURNING rev",
                    params![session_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            Ok(rev.unwrap_or(0) as u64)
        })
    }

    /// A page of raw rows in seq order, decoded as `session_log_after` does; a backward page
    /// holds the newest rows before its seq. It stops before the row that would pass
    /// `max_bytes` of payload, but always holds one.
    pub(crate) fn session_log_page(
        &self,
        session_id: &str,
        from: PageFrom,
        limit: u32,
        max_bytes: usize,
    ) -> Result<SessionLogPage> {
        let (sql, bound) = match from {
            PageFrom::After(seq) => (
                "SELECT seq, kind, payload, created_at FROM session_events
                 WHERE session_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
                seq.unwrap_or(-1),
            ),
            PageFrom::Before(seq) => (
                "SELECT seq, kind, payload, created_at FROM session_events
                 WHERE session_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3",
                seq.unwrap_or(i64::MAX),
            ),
        };
        let mut rows: Vec<StoredEvent> = self.with(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let mut rows = stmt.query(params![session_id, bound, limit])?;
            let mut page = Vec::new();
            let mut bytes = 0;
            while let Some(row) = rows.next()? {
                let size = match row.get_ref("payload")? {
                    ValueRef::Text(payload) | ValueRef::Blob(payload) => payload.len(),
                    _ => 0,
                };
                if !page.is_empty() && bytes + size > max_bytes {
                    break;
                }
                bytes += size;
                page.push(StoredEvent {
                    seq: row.get::<_, i64>("seq")? as f64,
                    kind: row.get("kind")?,
                    payload: row.get("payload")?,
                    created_at: row.get("created_at")?,
                });
            }
            Ok(page)
        })?;
        if let PageFrom::Before(_) = from {
            rows.reverse();
        }

        let last = rows.last().map(|r| (r.seq as i64, r.created_at));
        let after_last = match (last, from) {
            (Some((seq, _)), _) => seq,
            (None, PageFrom::After(seq)) => seq.unwrap_or(-1),
            (None, PageFrom::Before(seq)) => seq.map_or(-1, |seq| seq - 1),
        };
        let first = rows.first().map(|r| (r.seq as i64, r.created_at));
        let (remaining, earlier) = self.with(|conn| {
            let remaining = conn.query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1 AND seq > ?2",
                params![session_id, after_last],
                |row| row.get::<_, i64>(0),
            )?;
            let earlier = match (from, first) {
                (PageFrom::Before(_), Some((seq, created_at))) => conn
                    .query_row(
                        "SELECT 1 FROM session_events WHERE session_id = ?1 AND seq < ?2 LIMIT 1",
                        params![session_id, seq],
                        |_| Ok((seq, created_at)),
                    )
                    .optional()?,
                _ => None,
            };
            Ok((remaining, earlier))
        })?;

        Ok(SessionLogPage {
            entries: decode_session_log(session_id, rows),
            last,
            earlier,
            remaining: remaining as u32,
        })
    }

    /// The ask rows naming one of `request_ids`, oldest first, with their payload bytes.
    pub(crate) fn session_open_asks(
        &self,
        session_id: &str,
        request_ids: &[String],
    ) -> Result<(Vec<SessionLogEntry>, usize)> {
        if request_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let rows: Vec<StoredEvent> = self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT seq, kind, payload, created_at FROM session_events
                 WHERE session_id = ?1 AND kind IN ('permissionAsk', 'questionAsk') ORDER BY seq",
            )?;
            let rows = stmt.query_map(params![session_id], |row| {
                Ok(StoredEvent {
                    seq: row.get::<_, i64>("seq")? as f64,
                    kind: row.get("kind")?,
                    payload: row.get("payload")?,
                    created_at: row.get("created_at")?,
                })
            })?;
            rows.collect()
        })?;
        let asks: Vec<SessionLogEntry> = decode_session_log(session_id, rows)
            .into_iter()
            .filter(|entry| match &entry.event {
                SessionEvent::PermissionAsk { request_id, .. }
                | SessionEvent::QuestionAsk { request_id, .. } => request_ids.contains(request_id),
                _ => false,
            })
            .collect();
        let bytes = asks
            .iter()
            .map(|ask| encode_raw_event(&ask.event).map_or(0, |payload| payload.len()))
            .sum();
        Ok((asks, bytes))
    }

    pub fn session_events(&self, session_id: &str) -> Result<Vec<StoredEvent>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT seq, kind, payload, created_at FROM session_events
                 WHERE session_id = ?1 ORDER BY seq",
            )?;
            let rows = stmt.query_map(params![session_id], |row| {
                Ok(StoredEvent {
                    seq: row.get::<_, i64>("seq")? as f64,
                    kind: row.get("kind")?,
                    payload: row.get("payload")?,
                    created_at: row.get("created_at")?,
                })
            })?;
            rows.collect()
        })
    }

    pub fn nth_user_message_seq(&self, session_id: &str, n: usize) -> Result<Option<i64>> {
        if n == 0 {
            return Ok(None);
        }
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT seq FROM session_events
                 WHERE session_id = ?1 AND kind = 'userMessage'
                 ORDER BY seq
                 LIMIT 1 OFFSET ?2",
            )?;
            let mut rows = stmt.query(params![session_id, (n - 1) as i64])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn count_user_messages(&self, session_id: &str) -> Result<usize> {
        self.with(|conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_events
                 WHERE session_id = ?1 AND kind = 'userMessage'",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(n.max(0) as usize)
        })
    }

    pub fn cut_session_events_from(
        &self,
        session_id: &str,
        cutoff: i64,
    ) -> Result<Vec<StoredEvent>> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let removed = {
                let mut stmt = tx.prepare(
                    "SELECT seq, kind, payload, created_at FROM session_events
                     WHERE session_id = ?1 AND seq >= ?2 ORDER BY seq",
                )?;
                let rows = stmt.query_map(params![session_id, cutoff], |row| {
                    Ok(StoredEvent {
                        seq: row.get::<_, i64>("seq")? as f64,
                        kind: row.get("kind")?,
                        payload: row.get("payload")?,
                        created_at: row.get("created_at")?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            tx.execute(
                "DELETE FROM session_events WHERE session_id = ?1 AND seq >= ?2",
                params![session_id, cutoff],
            )?;
            tx.commit()?;
            Ok(removed)
        })
    }

    pub fn restore_session_events(&self, session_id: &str, events: &[StoredEvent]) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut insert = tx.prepare(
                    "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for e in events {
                    insert.execute(params![
                        session_id,
                        e.seq as i64,
                        e.kind,
                        e.payload,
                        e.created_at
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn record_orphan(&self, pid: i64, session_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO orphan_pids (pid, session_id, spawned_at)
                 VALUES (?1,?2,?3)",
                params![pid, session_id, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn clear_orphans_for(&self, session_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM orphan_pids WHERE session_id = ?1",
                params![session_id],
            )?;
            Ok(())
        })
    }

    /// Pid, session id and when the pid was recorded, in ms.
    pub fn list_orphans(&self) -> Result<Vec<(i64, String, f64)>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT pid, session_id, spawned_at FROM orphan_pids")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.collect()
        })
    }

    pub fn queue_orphan_wakes(&self) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "INSERT OR IGNORE INTO pending_wakes (session_id, mid_turn)
                 SELECT DISTINCT session_id, 0 FROM orphan_pids",
                [],
            )?;
            tx.execute("DELETE FROM orphan_pids", [])?;
            tx.commit()
        })
    }

    pub fn pending_wakes(&self) -> Result<Vec<(String, bool)>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT session_id, mid_turn FROM pending_wakes ORDER BY session_id")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
    }

    pub fn unanswered_restart_wakes(&self, session_id: &str) -> Result<u32> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {UNANSWERED_WAKES} FROM sessions WHERE id = ?1"),
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::unwrap_or_default)
        })
    }

    pub fn record_restart_wake(&self, session_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                &format!(
                    "UPDATE sessions
                        SET restart_wakes = {UNANSWERED_WAKES} + 1,
                            restart_wake_seq = (SELECT COALESCE(MAX(seq), -1) FROM session_events
                                                 WHERE session_id = sessions.id)
                      WHERE id = ?1"
                ),
                params![session_id],
            )?;
            Ok(())
        })
    }

    pub fn delete_pending_wake(&self, session_id: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM pending_wakes WHERE session_id = ?1",
                params![session_id],
            )?;
            Ok(())
        })
    }

    pub fn hold_message(&self, held: &HeldMessage) -> Result<()> {
        let sender = held
            .sender
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| CoreError::Io(std::io::Error::other(e)))?;
        self.with(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO held_messages
                 (session_id, nonce, text, prompt_name, author_id, author_device, sender, project_path, resent)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    held.session_id,
                    held.nonce,
                    held.text,
                    held.prompt_name,
                    held.author.as_ref().map(|a| &a.client_id),
                    held.author.as_ref().map(|a| &a.device),
                    sender,
                    held.project_path,
                    held.resent,
                ],
            )?;
            Ok(())
        })
    }

    pub fn release_held(&self, session_id: &str, nonce: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM held_messages WHERE session_id = ?1 AND nonce = ?2",
                params![session_id, nonce],
            )?;
            Ok(())
        })
    }

    /// Removes and returns a session's held messages, oldest first; with `told_only`, only those
    /// whose sender can be told.
    pub fn take_held(&self, session_id: &str, told_only: bool) -> Result<Vec<HeldMessage>> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let held: Vec<HeldMessage> = held_rows(&tx, session_id)?
                .into_iter()
                .filter(|held| !told_only || held.sender.is_some())
                .collect();
            tx.execute(
                "DELETE FROM held_messages WHERE session_id = ?1 AND (?2 = 0 OR sender IS NOT NULL)",
                params![session_id, told_only],
            )?;
            tx.commit()?;
            Ok(held)
        })
    }

    /// For a relaunch: the messages to send again, now marked resent, and those already
    /// resent once, now removed.
    pub fn relaunch_held(&self, session_id: &str) -> Result<(Vec<HeldMessage>, Vec<HeldMessage>)> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let (lost, resend): (Vec<_>, Vec<_>) = held_rows(&tx, session_id)?
                .into_iter()
                .partition(|held| held.resent);
            tx.execute(
                "DELETE FROM held_messages WHERE session_id = ?1 AND resent = 1",
                params![session_id],
            )?;
            tx.execute(
                "UPDATE held_messages SET resent = 1 WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.commit()?;
            Ok((resend, lost))
        })
    }

    pub fn held_session_ids(&self) -> Result<Vec<String>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT DISTINCT session_id FROM held_messages")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect()
        })
    }

    pub(crate) fn apply_session_event(
        &self,
        session_id: &str,
        seq: i64,
        kind: &str,
        raw_payload: &str,
        mutation: SessionRowMutation,
    ) -> Result<ApplySessionEventOutcome> {
        self.with_core(|conn| {
            let tx = conn.unchecked_transaction()?;

            let existing: Option<(String, String, f64)> = tx
                .query_row(
                    "SELECT kind, payload, created_at FROM session_events
                     WHERE session_id = ?1 AND seq = ?2",
                    params![session_id, seq],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;

            let outcome = match existing {
                None => {
                    let created_at = now_ms();
                    tx.execute(
                        "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
                         VALUES (?1,?2,?3,?4,?5)",
                        params![session_id, seq, kind, raw_payload, created_at],
                    )?;
                    let rev = if REV_KINDS.contains(&kind) {
                        tx.query_row(
                            // A rewind boundary is also the rev every reader reloads from.
                            "UPDATE sessions SET rev = rev + 1,
                                    reset_rev = CASE WHEN ?2 = 'rewound' THEN rev + 1 ELSE reset_rev END
                              WHERE id = ?1 RETURNING rev",
                            params![session_id, kind],
                            |row| row.get::<_, i64>(0),
                        )
                        .optional()?
                        .map(|rev| rev as u64)
                    } else {
                        None
                    };
                    ApplySessionEventOutcome::Applied { rev, created_at }
                }
                Some((existing_kind, existing_payload, created_at)) => {
                    if existing_kind == kind && existing_payload == raw_payload {
                        ApplySessionEventOutcome::AlreadyApplied { created_at }
                    } else {
                        return Err(CoreError::SessionSequenceConflict {
                            session_id: session_id.to_string(),
                            seq,
                        });
                    }
                }
            };

            #[cfg(any(test, feature = "test-support"))]
            if let Some(action) = self.session_event_gate_check(session_id, seq) {
                match action {
                    SessionEventGateAction::Proceed => {}
                    SessionEventGateAction::Delay(delay) => std::thread::sleep(delay),
                    SessionEventGateAction::FailRowMutation => {
                        return Err(CoreError::Db(rusqlite::Error::InvalidParameterName(
                            "session_event_gate: forced row-mutation failure".into(),
                        )));
                    }
                }
            }

            apply_row_mutation(&tx, session_id, &mutation)?;

            tx.commit()?;
            Ok(outcome)
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Db {
    fn session_event_gate_check(
        &self,
        session_id: &str,
        seq: i64,
    ) -> Option<SessionEventGateAction> {
        self.session_event_gate
            .lock()
            .expect("session event gate mutex poisoned")
            .as_ref()
            .map(|hook| hook(session_id, seq))
    }

    pub fn set_session_event_gate(
        &self,
        hook: impl Fn(&str, i64) -> SessionEventGateAction + Send + Sync + 'static,
    ) {
        *self
            .session_event_gate
            .lock()
            .expect("session event gate mutex poisoned") = Some(std::sync::Arc::new(hook));
    }

    pub fn clear_session_event_gate(&self) {
        *self
            .session_event_gate
            .lock()
            .expect("session event gate mutex poisoned") = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::workstreams::WorktreeMode;

    fn seed_dialog(db: &Db, session_id: &str) {
        db.insert_workstream(crate::store::NewWorkstream {
            project_path: "/p".into(),
            title: "t".into(),
            slug: format!("t-{session_id}"),
            branch: format!("hyprlayer/t-{session_id}"),
            worktree_path: "/wt".into(),
            base_branch: "main".into(),
            worktree_mode: WorktreeMode::Isolated,
        })
        .unwrap();
        let ws = db.list_active_workstreams().unwrap().pop().unwrap();
        db.insert_session(NewSession {
            id: session_id.into(),
            workstream_id: ws.id,
            stage: Stage::General,
            harness: Harness::Claude,
            cwd: "/wt".into(),
            first_input: None,
        })
        .unwrap();
        for (seq, kind) in [
            (0, "init"),
            (1, "userMessage"),
            (2, "assistantText"),
            (3, "userMessage"),
            (4, "assistantText"),
            (5, "userMessage"),
            (6, "assistantText"),
        ] {
            db.append_session_event(session_id, seq, kind, &format!(r#"{{"kind":"{kind}"}}"#))
                .unwrap();
        }
    }

    #[test]
    fn nth_user_message_seq_finds_each_user_turn() {
        let db = Db::open_in_memory().unwrap();
        seed_dialog(&db, "s1");
        assert_eq!(db.nth_user_message_seq("s1", 1).unwrap(), Some(1));
        assert_eq!(db.nth_user_message_seq("s1", 2).unwrap(), Some(3));
        assert_eq!(db.nth_user_message_seq("s1", 3).unwrap(), Some(5));
        assert_eq!(db.nth_user_message_seq("s1", 4).unwrap(), None);
    }

    #[test]
    fn cut_drops_everything_from_cutoff_then_restores_verbatim() {
        let db = Db::open_in_memory().unwrap();
        seed_dialog(&db, "s1");
        let kinds = |db: &Db| -> Vec<(i64, String)> {
            db.session_events("s1")
                .unwrap()
                .into_iter()
                .map(|e| (e.seq as i64, e.kind))
                .collect()
        };
        let removed = db.cut_session_events_from("s1", 3).unwrap();
        assert_eq!(removed.len(), 4);
        assert_eq!(removed[0].seq as i64, 3);
        assert_eq!(
            kinds(&db),
            vec![
                (0, "init".into()),
                (1, "userMessage".into()),
                (2, "assistantText".into()),
            ],
        );
        db.restore_session_events("s1", &removed).unwrap();
        assert_eq!(
            kinds(&db),
            vec![
                (0, "init".into()),
                (1, "userMessage".into()),
                (2, "assistantText".into()),
                (3, "userMessage".into()),
                (4, "assistantText".into()),
                (5, "userMessage".into()),
                (6, "assistantText".into()),
            ],
        );
    }

    #[test]
    fn preview_is_the_users_own_text_for_any_parseable_stage() {
        let db = Db::open_in_memory().unwrap();
        seed_dialog(&db, "anchor");
        let ws = db.list_active_workstreams().unwrap().pop().unwrap().id;
        let cases = [
            ("general", Stage::General, None, "the first prompt"),
            (
                "plan",
                Stage::Plan,
                Some("add dark mode"),
                "/create_plan add dark mode",
            ),
            (
                "legacy-plan",
                Stage::Plan,
                None,
                "/create_plan add dark mode",
            ),
            ("commit", Stage::Commit, None, "Review the working tree"),
            ("future", Stage::General, None, "the first prompt"),
        ];
        for (id, stage, first_input, prompt) in cases {
            db.insert_session(NewSession {
                id: id.into(),
                workstream_id: ws.clone(),
                stage,
                harness: Harness::Claude,
                cwd: "/wt".into(),
                first_input: first_input.map(str::to_string),
            })
            .unwrap();
            let payload = serde_json::json!({"kind": "userMessage", "text": prompt, "stage": null});
            db.append_session_event(id, 0, "userMessage", &payload.to_string())
                .unwrap();
        }
        // A stage from a newer server that this build cannot parse.
        db.with(|conn| {
            conn.execute(
                "UPDATE sessions SET stage = 'review' WHERE id = 'future'",
                [],
            )
        })
        .unwrap();

        let sessions = db.list_sessions(&ws).unwrap();
        let preview = |id: &str| {
            sessions
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.preview.clone())
        };
        assert_eq!(preview("general").as_deref(), Some("the first prompt"));
        assert_eq!(preview("plan").as_deref(), Some("add dark mode"));
        assert_eq!(preview("legacy-plan").as_deref(), Some("add dark mode"));
        assert_eq!(preview("commit"), None);
        assert_eq!(preview("future"), None);

        assert!(db.set_session_title_if_unset("plan", "Dark Mode").unwrap());
        assert!(db.session_title_source("plan").unwrap().is_none());
        assert!(!db.set_session_title_if_unset("plan", "Other").unwrap());
        let source = db.session_title_source("legacy-plan").unwrap().unwrap();
        assert_eq!(source.input, "add dark mode");
    }

    #[test]
    fn orphan_ledger_records_and_clears() {
        let db = Db::open_in_memory().unwrap();
        db.record_orphan(1234, "s1").unwrap();
        db.record_orphan(5678, "s2").unwrap();
        assert_eq!(db.list_orphans().unwrap().len(), 2);
        db.clear_orphans_for("s1").unwrap();
        let left = db.list_orphans().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].0, left[0].1.as_str()), (5678, "s2"));
        assert!(left[0].2 > 0.0);
        db.record_orphan(9012, "s2").unwrap();
        db.record_orphan(3456, "s3").unwrap();
        db.queue_orphan_wakes().unwrap();
        assert!(db.list_orphans().unwrap().is_empty());
        assert_eq!(
            db.pending_wakes().unwrap(),
            [("s2".to_string(), false), ("s3".to_string(), false)]
        );
        db.delete_pending_wake("s2").unwrap();
        assert_eq!(db.pending_wakes().unwrap(), [("s3".to_string(), false)]);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeldMessage {
    pub session_id: String,
    pub nonce: String,
    pub text: String,
    pub prompt_name: Option<String>,
    pub author: Option<ClientRef>,
    /// The agent to tell if the message is never read; `None` when no one would read it.
    pub sender: Option<AgentCaller>,
    pub project_path: Option<String>,
    pub resent: bool,
}

fn held_rows(conn: &rusqlite::Connection, session_id: &str) -> rusqlite::Result<Vec<HeldMessage>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, nonce, text, prompt_name, author_id, author_device, sender, project_path, resent
         FROM held_messages WHERE session_id = ?1 ORDER BY rowid",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        let sender: Option<String> = row.get(6)?;
        let sender = sender
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        let author_id: Option<String> = row.get(4)?;
        let author_device: Option<String> = row.get(5)?;
        Ok(HeldMessage {
            session_id: row.get(0)?,
            nonce: row.get(1)?,
            text: row.get(2)?,
            prompt_name: row.get(3)?,
            author: author_id
                .zip(author_device)
                .map(|(client_id, device)| ClientRef { client_id, device }),
            sender,
            project_path: row.get(7)?,
            resent: row.get(8)?,
        })
    })?;
    rows.collect()
}
