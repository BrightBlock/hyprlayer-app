use hyprlayer_models::agent::Harness;
use hyprlayer_models::goal::{
    Goal, GoalCheck, GoalCheckOutcome, GoalCheckResult, GoalEndReason, GoalIteration,
    GoalIterationNote, GoalLimits, GoalPauseReason, GoalSource, GoalSpec, GoalStatus, GoalStep,
};
use hyprlayer_models::session::ClientRef;
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row, ToSql};

use super::{now_ms, Db, FieldOp};
use crate::error::{CoreError, Result};

/// A goal with the counters only its runner reads.
pub struct GoalRow {
    pub goal: Goal,
    pub cost_baseline_usd: Option<f64>,
    pub token_baseline: Option<u32>,
    pub stall_count: u32,
    pub check_errors: u32,
    pub turn_failures: u32,
    pub harness_crashes: u32,
    pub restart_resumes: u32,
}

pub struct NewGoal {
    pub session_id: String,
    pub workstream_id: String,
    pub spec: GoalSpec,
    pub author: Option<ClientRef>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GoalStateUpdate {
    pub status: FieldOp<GoalStatus>,
    pub step: FieldOp<GoalStep>,
    pub pause_reason: FieldOp<GoalPauseReason>,
    pub end_reason: FieldOp<GoalEndReason>,
    pub iteration: FieldOp<u32>,
    pub spent_cost_usd: FieldOp<f64>,
    pub spent_tokens: FieldOp<u32>,
    pub cost_baseline_usd: FieldOp<f64>,
    pub token_baseline: FieldOp<u32>,
    pub active_ms: FieldOp<u32>,
    pub stall_count: FieldOp<u32>,
    pub check_errors: FieldOp<u32>,
    pub turn_failures: FieldOp<u32>,
    pub harness_crashes: FieldOp<u32>,
    pub restart_resumes: FieldOp<u32>,
    pub last_outcome: FieldOp<GoalCheckOutcome>,
    pub last_reason: FieldOp<String>,
    pub ended_at: FieldOp<f64>,
}

/// What closing an iteration records; its check results arrive later, in `set_iteration_results`.
pub struct IterationEnd {
    pub turn_status: Option<String>,
    pub cost_usd: Option<f64>,
    pub tokens: Option<u32>,
    pub note: Option<GoalIterationNote>,
}

const GOAL_COLS: &str = "id, session_id, workstream_id, source, source_ref, source_phase, \
                         condition, work_prompt, checks, max_iterations, max_cost_usd, max_tokens, \
                         max_active_secs, status, step, pause_reason, end_reason, iteration, \
                         spent_cost_usd, spent_tokens, cost_baseline_usd, token_baseline, \
                         active_ms, stall_count, check_errors, turn_failures, harness_crashes, \
                         restart_resumes, last_outcome, last_reason, author_client_id, \
                         author_device, started_at, updated_at, ended_at";
const ITERATION_COLS: &str = "goal_id, n, client_message_id, started_at, ended_at, turn_status, \
                              cost_usd, tokens, outcome, reason, results, progress_hash, note";

fn count(row: &Row<'_>, name: &str) -> rusqlite::Result<u32> {
    Ok(row.get::<_, i64>(name)?.clamp(0, u32::MAX as i64) as u32)
}

fn opt_count(row: &Row<'_>, name: &str) -> rusqlite::Result<Option<u32>> {
    Ok(row
        .get::<_, Option<i64>>(name)?
        .map(|n| n.clamp(0, u32::MAX as i64) as u32))
}

/// An unreadable check fails the row: dropping it could let the goal meet without it.
fn row_to_goal(row: &Row<'_>) -> rusqlite::Result<GoalRow> {
    let checks: String = row.get("checks")?;
    let checks: Vec<GoalCheck> = serde_json::from_str(&checks).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            row.as_ref().column_index("checks").unwrap_or_default(),
            Type::Text,
            Box::new(e),
        )
    })?;
    let source: String = row.get("source")?;
    let status: String = row.get("status")?;
    let author_device: Option<String> = row.get("author_device")?;
    let author = row
        .get::<_, Option<String>>("author_client_id")?
        .map(|client_id| ClientRef {
            client_id,
            device: author_device.unwrap_or_default(),
        });
    Ok(GoalRow {
        goal: Goal {
            id: row.get("id")?,
            session_id: row.get("session_id")?,
            workstream_id: row.get("workstream_id")?,
            condition: row.get("condition")?,
            work_prompt: row.get("work_prompt")?,
            source: GoalSource::parse(&source),
            source_ref: row.get("source_ref")?,
            source_phase: row.get("source_phase")?,
            checks,
            limits: GoalLimits {
                max_iterations: count(row, "max_iterations")?,
                max_cost_usd: row.get("max_cost_usd")?,
                max_tokens: opt_count(row, "max_tokens")?,
                max_active_secs: count(row, "max_active_secs")?,
            },
            status: GoalStatus::parse(&status),
            step: row
                .get::<_, Option<String>>("step")?
                .map(|s| GoalStep::parse(&s)),
            pause_reason: row
                .get::<_, Option<String>>("pause_reason")?
                .map(|s| GoalPauseReason::parse(&s)),
            end_reason: row
                .get::<_, Option<String>>("end_reason")?
                .map(|s| GoalEndReason::parse(&s)),
            iteration: count(row, "iteration")?,
            spent_cost_usd: row.get("spent_cost_usd")?,
            spent_tokens: count(row, "spent_tokens")?,
            active_ms: count(row, "active_ms")?,
            last_outcome: row
                .get::<_, Option<String>>("last_outcome")?
                .map(|s| GoalCheckOutcome::parse(&s)),
            last_reason: row.get("last_reason")?,
            author,
            started_at: row.get("started_at")?,
            updated_at: row.get("updated_at")?,
            ended_at: row.get("ended_at")?,
        },
        cost_baseline_usd: row.get("cost_baseline_usd")?,
        token_baseline: opt_count(row, "token_baseline")?,
        stall_count: count(row, "stall_count")?,
        check_errors: count(row, "check_errors")?,
        turn_failures: count(row, "turn_failures")?,
        harness_crashes: count(row, "harness_crashes")?,
        restart_resumes: count(row, "restart_resumes")?,
    })
}

/// History only, so an unreadable results column reads as no results.
fn row_to_iteration(row: &Row<'_>) -> rusqlite::Result<GoalIteration> {
    let results: String = row.get("results")?;
    Ok(GoalIteration {
        goal_id: row.get("goal_id")?,
        n: count(row, "n")?,
        client_message_id: row.get("client_message_id")?,
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        turn_status: row.get("turn_status")?,
        cost_usd: row.get("cost_usd")?,
        tokens: opt_count(row, "tokens")?,
        outcome: row
            .get::<_, Option<String>>("outcome")?
            .map(|s| GoalCheckOutcome::parse(&s)),
        reason: row.get("reason")?,
        results: serde_json::from_str::<Vec<GoalCheckResult>>(&results).unwrap_or_default(),
        progress_hash: row.get("progress_hash")?,
        note: row
            .get::<_, Option<String>>("note")?
            .and_then(|s| GoalIterationNote::try_parse(&s)),
    })
}

fn json_column<T: serde::Serialize + ?Sized>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// The one unique constraint a goal write can trip: ids are minted fresh and never rewritten.
fn is_live_goal_conflict(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(e, _)
        if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
}

fn state_write_error(conn: &Connection, goal_id: &str, err: rusqlite::Error) -> CoreError {
    if !is_live_goal_conflict(&err) {
        return err.into();
    }
    let session_id = conn
        .query_row(
            "SELECT session_id FROM goals WHERE id = ?1",
            params![goal_id],
            |row| row.get(0),
        )
        .unwrap_or_default();
    CoreError::GoalActive { session_id }
}

#[derive(Default)]
struct Sets {
    clauses: Vec<String>,
    values: Vec<Box<dyn ToSql>>,
}

impl Sets {
    fn set(&mut self, column: &str, value: impl ToSql + 'static) {
        self.values.push(Box::new(value));
        self.clauses
            .push(format!("{column} = ?{}", self.values.len()));
    }

    fn clear(&mut self, column: &str) {
        self.clauses.push(format!("{column} = NULL"));
    }

    fn put<T, V: ToSql + 'static>(
        &mut self,
        column: &str,
        op: &FieldOp<T>,
        value: impl Fn(&T) -> V,
    ) {
        match op {
            FieldOp::Unchanged => {}
            FieldOp::Set(v) => self.set(column, value(v)),
            FieldOp::Clear => self.clear(column),
        }
    }
}

/// Bumps the goal's `updated_at`: its iterations changing is the goal changing.
fn touch(conn: &Connection, goal_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE goals SET updated_at = ?2 WHERE id = ?1",
        params![goal_id, now_ms()],
    )?;
    Ok(())
}

impl Db {
    /// A second live goal on the session trips the unique index and reads as `GoalActive`.
    pub fn insert_goal(&self, new: NewGoal) -> Result<GoalRow> {
        let now = now_ms();
        let spec = new.spec;
        let goal = Goal {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: new.session_id,
            workstream_id: new.workstream_id,
            condition: spec.condition,
            work_prompt: spec.work_prompt,
            source: spec.source,
            source_ref: spec.source_ref,
            source_phase: spec.source_phase,
            checks: spec.checks,
            limits: spec.limits,
            status: GoalStatus::Running,
            step: Some(GoalStep::Working),
            pause_reason: None,
            end_reason: None,
            iteration: 0,
            spent_cost_usd: 0.0,
            spent_tokens: 0,
            active_ms: 0,
            last_outcome: None,
            last_reason: None,
            author: new.author,
            started_at: now,
            updated_at: now,
            ended_at: None,
        };
        self.with_core(|conn| {
            let checks = json_column(&goal.checks)?;
            conn.execute(
                "INSERT INTO goals (id, session_id, workstream_id, source, source_ref, source_phase,
                                    condition, work_prompt, checks, max_iterations, max_cost_usd,
                                    max_tokens, max_active_secs, status, step, author_client_id,
                                    author_device, started_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18)",
                params![
                    goal.id,
                    goal.session_id,
                    goal.workstream_id,
                    goal.source.as_str(),
                    goal.source_ref,
                    goal.source_phase,
                    goal.condition,
                    goal.work_prompt,
                    checks,
                    goal.limits.max_iterations,
                    goal.limits.max_cost_usd,
                    goal.limits.max_tokens,
                    goal.limits.max_active_secs,
                    goal.status.as_str(),
                    goal.step.map(GoalStep::as_str),
                    goal.author.as_ref().map(|a| a.client_id.as_str()),
                    goal.author.as_ref().map(|a| a.device.as_str()),
                    goal.started_at,
                ],
            )
            .map_err(|e| {
                if is_live_goal_conflict(&e) {
                    CoreError::GoalActive {
                        session_id: goal.session_id.clone(),
                    }
                } else {
                    e.into()
                }
            })?;
            Ok(())
        })?;
        Ok(GoalRow {
            goal,
            cost_baseline_usd: None,
            token_baseline: None,
            stall_count: 0,
            check_errors: 0,
            turn_failures: 0,
            harness_crashes: 0,
            restart_resumes: 0,
        })
    }

    pub fn goal(&self, id: &str) -> Result<Option<GoalRow>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {GOAL_COLS} FROM goals WHERE id = ?1"),
                params![id],
                row_to_goal,
            )
            .optional()
        })
    }

    pub fn live_goal_for_session(&self, session_id: &str) -> Result<Option<GoalRow>> {
        self.with(|conn| live_goal(conn, session_id))
    }

    /// Newest first; `active_only` keeps running and paused goals.
    pub fn list_goals(&self, workstream_id: Option<&str>, active_only: bool) -> Result<Vec<Goal>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {GOAL_COLS} FROM goals
                  WHERE (?1 IS NULL OR workstream_id = ?1)
                    AND (?2 = 0 OR status IN ('running', 'paused'))
                  ORDER BY started_at DESC, rowid DESC"
            ))?;
            let rows = stmt.query_map(params![workstream_id, active_only], |row| {
                row_to_goal(row).map(|r| r.goal)
            })?;
            rows.collect()
        })
    }

    pub fn goals_with_status(&self, status: GoalStatus) -> Result<Vec<GoalRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {GOAL_COLS} FROM goals WHERE status = ?1 ORDER BY started_at, rowid"
            ))?;
            let rows = stmt.query_map(params![status.as_str()], row_to_goal)?;
            rows.collect()
        })
    }

    /// Writing `running` over a session's other live goal is `GoalActive`. `None`: no such goal.
    pub(crate) fn update_goal_state(
        &self,
        id: &str,
        update: GoalStateUpdate,
    ) -> Result<Option<GoalRow>> {
        let mut sets = Sets::default();
        sets.put("status", &update.status, |v| v.as_str());
        sets.put("step", &update.step, |v| v.as_str());
        sets.put("pause_reason", &update.pause_reason, |v| v.as_str());
        sets.put("end_reason", &update.end_reason, |v| v.as_str());
        sets.put("iteration", &update.iteration, |v| i64::from(*v));
        sets.put("spent_cost_usd", &update.spent_cost_usd, |v| *v);
        sets.put("spent_tokens", &update.spent_tokens, |v| i64::from(*v));
        sets.put("cost_baseline_usd", &update.cost_baseline_usd, |v| *v);
        sets.put("token_baseline", &update.token_baseline, |v| i64::from(*v));
        sets.put("active_ms", &update.active_ms, |v| i64::from(*v));
        sets.put("stall_count", &update.stall_count, |v| i64::from(*v));
        sets.put("check_errors", &update.check_errors, |v| i64::from(*v));
        sets.put("turn_failures", &update.turn_failures, |v| i64::from(*v));
        sets.put("harness_crashes", &update.harness_crashes, |v| {
            i64::from(*v)
        });
        sets.put("restart_resumes", &update.restart_resumes, |v| {
            i64::from(*v)
        });
        sets.put("last_outcome", &update.last_outcome, |v| v.as_str());
        sets.put("last_reason", &update.last_reason, |v| v.clone());
        sets.put("ended_at", &update.ended_at, |v| *v);
        sets.set("updated_at", now_ms());
        let sql = format!(
            "UPDATE goals SET {} WHERE id = ?{}",
            sets.clauses.join(", "),
            sets.values.len() + 1
        );
        self.with_core(|conn| {
            let mut args: Vec<&dyn ToSql> = sets.values.iter().map(|v| v.as_ref()).collect();
            args.push(&id);
            let changed = conn
                .execute(&sql, args.as_slice())
                .map_err(|e| state_write_error(conn, id, e))?;
            if changed == 0 {
                return Ok(None);
            }
            Ok(conn
                .query_row(
                    &format!("SELECT {GOAL_COLS} FROM goals WHERE id = ?1"),
                    params![id],
                    row_to_goal,
                )
                .optional()?)
        })
    }

    /// Writes only what an edit can change; the goal's source stays.
    pub fn update_goal_spec(
        &self,
        id: &str,
        condition: &str,
        work_prompt: &str,
        checks: &[GoalCheck],
        limits: &GoalLimits,
    ) -> Result<Option<GoalRow>> {
        self.with(|conn| {
            let changed = conn.execute(
                "UPDATE goals SET condition = ?2, work_prompt = ?3, checks = ?4, max_iterations = ?5,
                                  max_cost_usd = ?6, max_tokens = ?7, max_active_secs = ?8,
                                  updated_at = ?9
                  WHERE id = ?1",
                params![
                    id,
                    condition,
                    work_prompt,
                    json_column(checks)?,
                    limits.max_iterations,
                    limits.max_cost_usd,
                    limits.max_tokens,
                    limits.max_active_secs,
                    now_ms(),
                ],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            conn.query_row(
                &format!("SELECT {GOAL_COLS} FROM goals WHERE id = ?1"),
                params![id],
                row_to_goal,
            )
            .optional()
        })
    }

    /// The sessions of running and paused goals, with each one's harness and harness session id.
    pub fn live_goal_sessions(&self) -> Result<Vec<(String, Harness, Option<String>)>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.harness, s.claude_session_id FROM goals g
                   JOIN sessions s ON s.id = g.session_id
                  WHERE g.status IN ('running', 'paused')",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Harness::parse(&row.get::<_, String>(1)?),
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect()
        })
    }

    pub fn insert_iteration(&self, goal_id: &str, n: u32, client_message_id: &str) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO goal_iterations (goal_id, n, client_message_id, started_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![goal_id, n, client_message_id, now_ms()],
            )?;
            touch(&tx, goal_id)?;
            tx.commit()
        })
    }

    pub fn finish_iteration(&self, goal_id: &str, n: u32, end: &IterationEnd) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE goal_iterations
                    SET ended_at = ?3, turn_status = ?4, cost_usd = ?5, tokens = ?6, note = ?7
                  WHERE goal_id = ?1 AND n = ?2",
                params![
                    goal_id,
                    n,
                    now_ms(),
                    end.turn_status,
                    end.cost_usd,
                    end.tokens,
                    end.note.map(GoalIterationNote::as_str),
                ],
            )?;
            touch(&tx, goal_id)?;
            tx.commit()
        })
    }

    pub fn set_iteration_results(
        &self,
        goal_id: &str,
        n: u32,
        outcome: GoalCheckOutcome,
        reason: Option<&str>,
        results: &[GoalCheckResult],
        progress_hash: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE goal_iterations
                    SET outcome = ?3, reason = ?4, results = ?5, progress_hash = ?6
                  WHERE goal_id = ?1 AND n = ?2",
                params![
                    goal_id,
                    n,
                    outcome.as_str(),
                    reason,
                    json_column(results)?,
                    progress_hash
                ],
            )?;
            touch(&tx, goal_id)?;
            tx.commit()
        })
    }

    /// For a send that never left: the iteration it would have been did not happen.
    pub fn delete_iteration(&self, goal_id: &str, n: u32) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM goal_iterations WHERE goal_id = ?1 AND n = ?2",
                params![goal_id, n],
            )?;
            touch(&tx, goal_id)?;
            tx.commit()
        })
    }

    pub fn goal_iterations(&self, goal_id: &str) -> Result<Vec<GoalIteration>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ITERATION_COLS} FROM goal_iterations WHERE goal_id = ?1 ORDER BY n"
            ))?;
            let rows = stmt.query_map(params![goal_id], row_to_iteration)?;
            rows.collect()
        })
    }
}

fn live_goal(conn: &Connection, session_id: &str) -> rusqlite::Result<Option<GoalRow>> {
    conn.query_row(
        &format!(
            "SELECT {GOAL_COLS} FROM goals
              WHERE session_id = ?1 AND status IN ('running', 'paused')
              ORDER BY started_at DESC LIMIT 1"
        ),
        params![session_id],
        row_to_goal,
    )
    .optional()
}
