use std::collections::HashMap;

use hyprlayer_models::review::{
    ReviewCommentState, ReviewNoteKind, ReviewSide, ReviewStatus, ReviewVerdict,
};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{now_ms, Db};
use crate::error::Result;

pub struct ReviewRow {
    pub id: String,
    pub workstream_id: String,
    pub status: ReviewStatus,
    pub notes_offset: u64,
}

pub struct NewReviewNote {
    pub fingerprint: String,
    pub path: String,
    pub line: u32,
    pub anchor_text: String,
    pub kind: ReviewNoteKind,
    pub body: String,
    pub task: Option<String>,
}

pub struct ReviewCommentRow {
    pub id: String,
    pub parent_id: Option<String>,
    /// The agent note this comment answers.
    pub reply_to_note_id: Option<String>,
    pub path: String,
    pub side: ReviewSide,
    pub line: u32,
    pub anchor_text: String,
    pub body: String,
    pub state: ReviewCommentState,
    pub round: Option<u32>,
    /// The lines around the comment when it was sent, to tell whether the agent touched them.
    pub sent_context: Option<String>,
    pub resolved_at: Option<f64>,
    pub created_at: f64,
    pub updated_at: f64,
}

pub struct ReviewNoteRow {
    pub id: String,
    pub path: String,
    pub line: u32,
    pub anchor_text: String,
    pub kind: ReviewNoteKind,
    pub body: String,
    pub task: Option<String>,
    pub resolved_at: Option<f64>,
    pub created_at: f64,
}

pub struct ReviewRoundRow {
    pub number: u32,
    pub verdict: ReviewVerdict,
    pub summary: Option<String>,
    pub message: Option<String>,
    pub viewed_count: u32,
    pub file_count: u32,
    pub comment_count: u32,
    pub submitted_at: f64,
    pub delivered_at: Option<f64>,
}

pub struct NewReviewRound {
    pub verdict: ReviewVerdict,
    pub summary: Option<String>,
    pub message: Option<String>,
    pub viewed_count: u32,
    pub file_count: u32,
}

/// An unsent comment the round takes, and what it becomes.
pub struct RoundComment {
    pub id: String,
    pub sent_context: Option<String>,
}

pub enum RoundSubmit {
    Submitted(ReviewRoundRow),
    /// The latest round is a change request that never reached the agent.
    Undelivered(u32),
}

/// What an edit or delete found.
pub enum CommentChange {
    Applied,
    Missing,
    /// Sending or sent: no longer the reviewer's to change.
    Locked,
}

/// The states a reviewer may still edit or delete.
const UNSENT: [ReviewCommentState; 2] = [ReviewCommentState::Pending, ReviewCommentState::Saved];

fn row_to_review(row: &Row<'_>) -> rusqlite::Result<ReviewRow> {
    let status: String = row.get("status")?;
    let notes_offset: i64 = row.get("notes_offset")?;
    Ok(ReviewRow {
        id: row.get("id")?,
        workstream_id: row.get("workstream_id")?,
        status: ReviewStatus::parse(&status),
        notes_offset: notes_offset.max(0) as u64,
    })
}

fn row_to_comment(row: &Row<'_>) -> rusqlite::Result<ReviewCommentRow> {
    let side: String = row.get("side")?;
    let state: String = row.get("state")?;
    Ok(ReviewCommentRow {
        id: row.get("id")?,
        parent_id: row.get("parent_id")?,
        reply_to_note_id: row.get("reply_to_note_id")?,
        path: row.get("path")?,
        side: ReviewSide::parse(&side),
        line: row.get("line")?,
        anchor_text: row.get("anchor_text")?,
        body: row.get("body")?,
        state: ReviewCommentState::parse(&state),
        round: row.get("round")?,
        sent_context: row.get("sent_context")?,
        resolved_at: row.get("resolved_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_note(row: &Row<'_>) -> rusqlite::Result<ReviewNoteRow> {
    let kind: String = row.get("kind")?;
    Ok(ReviewNoteRow {
        id: row.get("id")?,
        path: row.get("path")?,
        line: row.get("line")?,
        anchor_text: row.get("anchor_text")?,
        kind: ReviewNoteKind::parse(&kind),
        body: row.get("body")?,
        task: row.get("task")?,
        resolved_at: row.get("resolved_at")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_round(row: &Row<'_>) -> rusqlite::Result<ReviewRoundRow> {
    let verdict: String = row.get("verdict")?;
    Ok(ReviewRoundRow {
        number: row.get("number")?,
        verdict: ReviewVerdict::parse(&verdict),
        summary: row.get("summary")?,
        message: row.get("message")?,
        viewed_count: row.get("viewed_count")?,
        file_count: row.get("file_count")?,
        comment_count: row.get("comment_count")?,
        submitted_at: row.get("submitted_at")?,
        delivered_at: row.get("delivered_at")?,
    })
}

const COMMENT_COLS: &str = "id, parent_id, reply_to_note_id, path, side, line, anchor_text, body, \
                            state, round, sent_context, resolved_at, created_at, updated_at";
const NOTE_COLS: &str = "id, path, line, anchor_text, kind, body, task, resolved_at, created_at";
const ROUND_COLS: &str = "number, verdict, summary, message, viewed_count, file_count, \
                          comment_count, submitted_at, delivered_at";

fn comment_change(
    conn: &Connection,
    review_id: &str,
    id: &str,
    changed: usize,
) -> rusqlite::Result<CommentChange> {
    if changed > 0 {
        return Ok(CommentChange::Applied);
    }
    let exists = conn
        .query_row(
            "SELECT 1 FROM review_comments WHERE id = ?1 AND review_id = ?2",
            params![id, review_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(if exists {
        CommentChange::Locked
    } else {
        CommentChange::Missing
    })
}

impl Db {
    /// Never `INSERT OR REPLACE`: its implicit delete would cascade through the review.
    pub fn ensure_review(&self, workstream_id: &str) -> Result<ReviewRow> {
        let now = now_ms();
        self.with(|conn| {
            conn.execute(
                "INSERT INTO reviews (id, workstream_id, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(workstream_id) DO NOTHING",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    workstream_id,
                    ReviewStatus::InProgress.as_str(),
                    now
                ],
            )?;
            conn.query_row(
                "SELECT id, workstream_id, status, notes_offset FROM reviews WHERE workstream_id = ?1",
                params![workstream_id],
                row_to_review,
            )
        })
    }

    pub fn set_viewed(&self, review_id: &str, path: &str, content_oid: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO review_viewed (review_id, path, content_oid, viewed_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(review_id, path) DO UPDATE SET
                     content_oid = excluded.content_oid, viewed_at = excluded.viewed_at",
                params![review_id, path, content_oid, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn clear_viewed(&self, review_id: &str, path: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM review_viewed WHERE review_id = ?1 AND path = ?2",
                params![review_id, path],
            )?;
            Ok(())
        })
    }

    /// Path → the content hash it was viewed at.
    pub fn viewed_map(&self, review_id: &str) -> Result<HashMap<String, String>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT path, content_oid FROM review_viewed WHERE review_id = ?1")?;
            let rows = stmt.query_map(params![review_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
    }

    /// A reply to a note resolves it in the same transaction.
    pub fn insert_comment(&self, review_id: &str, comment: &ReviewCommentRow) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                &format!(
                    "INSERT INTO review_comments (review_id, {COMMENT_COLS})
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"
                ),
                params![
                    review_id,
                    comment.id,
                    comment.parent_id,
                    comment.reply_to_note_id,
                    comment.path,
                    comment.side.as_str(),
                    comment.line,
                    comment.anchor_text,
                    comment.body,
                    comment.state.as_str(),
                    comment.round,
                    comment.sent_context,
                    comment.resolved_at,
                    comment.created_at,
                    comment.updated_at,
                ],
            )?;
            if let Some(note_id) = &comment.reply_to_note_id {
                tx.execute(
                    "UPDATE review_notes SET resolved_at = COALESCE(resolved_at, ?3)
                     WHERE id = ?1 AND review_id = ?2",
                    params![note_id, review_id, now_ms()],
                )?;
            }
            tx.commit()
        })
    }

    pub fn review_note(&self, review_id: &str, id: &str) -> Result<Option<ReviewNoteRow>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {NOTE_COLS} FROM review_notes WHERE id = ?1 AND review_id = ?2"),
                params![id, review_id],
                row_to_note,
            )
            .optional()
        })
    }

    pub fn review_comment(&self, review_id: &str, id: &str) -> Result<Option<ReviewCommentRow>> {
        self.with(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {COMMENT_COLS} FROM review_comments WHERE id = ?1 AND review_id = ?2"
                ),
                params![id, review_id],
                row_to_comment,
            )
            .optional()
        })
    }

    pub fn update_comment_body(
        &self,
        review_id: &str,
        id: &str,
        body: &str,
    ) -> Result<CommentChange> {
        self.with(|conn| {
            let changed = conn.execute(
                "UPDATE review_comments SET body = ?3, updated_at = ?4
                 WHERE id = ?1 AND review_id = ?2 AND state IN (?5, ?6)",
                params![
                    id,
                    review_id,
                    body,
                    now_ms(),
                    UNSENT[0].as_str(),
                    UNSENT[1].as_str()
                ],
            )?;
            comment_change(conn, review_id, id, changed)
        })
    }

    /// Its replies go with it (`ON DELETE CASCADE`).
    pub fn delete_comment(&self, review_id: &str, id: &str) -> Result<CommentChange> {
        self.with(|conn| {
            let changed = conn.execute(
                "DELETE FROM review_comments WHERE id = ?1 AND review_id = ?2 AND state IN (?3, ?4)",
                params![id, review_id, UNSENT[0].as_str(), UNSENT[1].as_str()],
            )?;
            comment_change(conn, review_id, id, changed)
        })
    }

    /// Returns whether this review has the comment.
    pub fn set_comment_resolved(&self, review_id: &str, id: &str, resolved: bool) -> Result<bool> {
        self.with(|conn| {
            let changed = conn.execute(
                "UPDATE review_comments SET resolved_at = CASE WHEN ?3 THEN COALESCE(resolved_at, ?4) END
                 WHERE id = ?1 AND review_id = ?2",
                params![id, review_id, resolved, now_ms()],
            )?;
            Ok(changed > 0)
        })
    }

    /// Returns whether this review has the note.
    pub fn set_note_resolved(&self, review_id: &str, id: &str, resolved: bool) -> Result<bool> {
        self.with(|conn| {
            let changed = conn.execute(
                "UPDATE review_notes SET resolved_at = CASE WHEN ?3 THEN COALESCE(resolved_at, ?4) END
                 WHERE id = ?1 AND review_id = ?2",
                params![id, review_id, resolved, now_ms()],
            )?;
            Ok(changed > 0)
        })
    }

    /// Notes already stored (same fingerprint) are kept; the offset moves with them.
    pub fn ingest_notes(
        &self,
        review_id: &str,
        notes: &[NewReviewNote],
        notes_offset: u64,
    ) -> Result<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let now = now_ms();
            for note in notes {
                tx.execute(
                    "INSERT OR IGNORE INTO review_notes
                         (id, review_id, fingerprint, path, line, anchor_text, kind, body, task, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        uuid::Uuid::new_v4().to_string(),
                        review_id,
                        note.fingerprint,
                        note.path,
                        note.line,
                        note.anchor_text,
                        note.kind.as_str(),
                        note.body,
                        note.task,
                        now
                    ],
                )?;
            }
            tx.execute(
                "UPDATE reviews SET notes_offset = ?2 WHERE id = ?1",
                params![review_id, notes_offset as i64],
            )?;
            tx.commit()
        })
    }

    pub fn list_comments(&self, review_id: &str) -> Result<Vec<ReviewCommentRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {COMMENT_COLS} FROM review_comments WHERE review_id = ?1
                 ORDER BY created_at, rowid"
            ))?;
            let rows = stmt.query_map(params![review_id], row_to_comment)?;
            rows.collect()
        })
    }

    pub fn list_notes(&self, review_id: &str) -> Result<Vec<ReviewNoteRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {NOTE_COLS} FROM review_notes WHERE review_id = ?1
                 ORDER BY created_at, rowid"
            ))?;
            let rows = stmt.query_map(params![review_id], row_to_note)?;
            rows.collect()
        })
    }

    /// One transaction, so a refused submit leaves no trace. Moves only comments
    /// still unsent; `comment_count` is how many it moved.
    pub fn submit_round(
        &self,
        review_id: &str,
        round: &NewReviewRound,
        files: &[(String, String)],
        comments: &[RoundComment],
        to: ReviewCommentState,
        status: Option<ReviewStatus>,
    ) -> Result<RoundSubmit> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let latest: Option<(u32, String, Option<f64>)> = tx
                .query_row(
                    "SELECT number, verdict, delivered_at FROM review_rounds
                     WHERE review_id = ?1 ORDER BY number DESC LIMIT 1",
                    params![review_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((number, verdict, None)) = &latest {
                if ReviewVerdict::parse(verdict) == ReviewVerdict::RequestChanges {
                    return Ok(RoundSubmit::Undelivered(*number));
                }
            }
            let number = latest.map_or(1, |(n, _, _)| n + 1);
            let round_id = uuid::Uuid::new_v4().to_string();
            let now = now_ms();
            let mut moved = 0u32;
            for c in comments {
                moved += tx.execute(
                    "UPDATE review_comments SET state = ?3, round = ?4, sent_context = ?5, updated_at = ?6
                     WHERE id = ?1 AND review_id = ?2 AND state IN (?7, ?8)",
                    params![
                        c.id,
                        review_id,
                        to.as_str(),
                        number,
                        c.sent_context,
                        now,
                        UNSENT[0].as_str(),
                        UNSENT[1].as_str()
                    ],
                )? as u32;
            }
            tx.execute(
                "INSERT INTO review_rounds (id, review_id, number, verdict, summary, message,
                     viewed_count, file_count, comment_count, submitted_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    round_id,
                    review_id,
                    number,
                    round.verdict.as_str(),
                    round.summary,
                    round.message,
                    round.viewed_count,
                    round.file_count,
                    moved,
                    now
                ],
            )?;
            for (path, oid) in files {
                tx.execute(
                    "INSERT INTO review_round_files (round_id, path, content_oid) VALUES (?1, ?2, ?3)",
                    params![round_id, path, oid],
                )?;
            }
            if let Some(status) = status {
                tx.execute(
                    "UPDATE reviews SET status = ?2, updated_at = ?3 WHERE id = ?1",
                    params![review_id, status.as_str(), now],
                )?;
            }
            tx.commit()?;
            Ok(RoundSubmit::Submitted(ReviewRoundRow {
                number,
                verdict: round.verdict,
                summary: round.summary.clone(),
                message: round.message.clone(),
                viewed_count: round.viewed_count,
                file_count: round.file_count,
                comment_count: moved,
                submitted_at: now,
                delivered_at: None,
            }))
        })
    }

    /// Returns whether the review has that change request round. Marking it again
    /// changes nothing, and only the latest round sets the review's status.
    pub fn mark_round_delivered(&self, review_id: &str, number: u32) -> Result<bool> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;
            let found: Option<(String, u32)> = tx
                .query_row(
                    "SELECT id, (SELECT MAX(number) FROM review_rounds WHERE review_id = ?1)
                     FROM review_rounds WHERE review_id = ?1 AND number = ?2 AND verdict = ?3",
                    params![review_id, number, ReviewVerdict::RequestChanges.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((round_id, latest)) = found else {
                return Ok(false);
            };
            let now = now_ms();
            tx.execute(
                "UPDATE review_rounds SET delivered_at = COALESCE(delivered_at, ?2) WHERE id = ?1",
                params![round_id, now],
            )?;
            tx.execute(
                "UPDATE review_comments SET state = ?3, updated_at = ?4
                 WHERE review_id = ?1 AND round = ?2 AND state = ?5",
                params![
                    review_id,
                    number,
                    ReviewCommentState::Sent.as_str(),
                    now,
                    ReviewCommentState::Sending.as_str()
                ],
            )?;
            if number == latest {
                tx.execute(
                    "UPDATE reviews SET status = ?2, updated_at = ?3 WHERE id = ?1",
                    params![review_id, ReviewStatus::ChangesRequested.as_str(), now],
                )?;
            }
            tx.commit()?;
            Ok(true)
        })
    }

    /// Path → content hash, as the round saw the branch.
    pub fn round_files(&self, review_id: &str, number: u32) -> Result<HashMap<String, String>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT f.path, f.content_oid FROM review_round_files f
                 JOIN review_rounds r ON r.id = f.round_id
                 WHERE r.review_id = ?1 AND r.number = ?2",
            )?;
            let rows = stmt.query_map(params![review_id, number], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            rows.collect()
        })
    }

    pub fn list_rounds(&self, review_id: &str) -> Result<Vec<ReviewRoundRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ROUND_COLS} FROM review_rounds WHERE review_id = ?1 ORDER BY number"
            ))?;
            let rows = stmt.query_map(params![review_id], row_to_round)?;
            rows.collect()
        })
    }
}
