use std::collections::{HashMap, HashSet};
use std::path::Path;

use hyprlayer_models::review::{
    ReviewCommentState, ReviewNoteKind, ReviewSide, ReviewStatus, ReviewVerdict,
};
use hyprlayer_models::RuntimeEvent;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::gitops::safe_join;
use crate::review::anchor::reanchor;
use crate::review::diff::{self, ReviewFileList};
use crate::review::message::{self, MessageComment, MessageNote};
use crate::review::sidecar;
use crate::store::{
    now_ms, CommentChange, Db, NewReviewNote, NewReviewRound, ReviewCommentRow, ReviewNoteRow,
    ReviewRoundRow, ReviewRow, RoundComment, RoundSubmit, Workstream,
};

use super::HyprlayerRuntime;

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewView {
    pub id: String,
    pub workstream_id: String,
    pub status: ReviewStatus,
    pub files: ReviewFileList,
    pub viewed: Vec<String>,
    pub comments: Vec<ReviewComment>,
    pub notes: Vec<ReviewNote>,
    pub rounds: Vec<ReviewRound>,
    pub changed_since_round: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub parent_id: Option<String>,
    pub reply_to_note_id: Option<String>,
    pub path: String,
    pub side: ReviewSide,
    pub line: u32,
    /// Missing once the line is gone from its side's current text.
    pub current_line: Option<u32>,
    pub anchor_text: String,
    pub body: String,
    pub state: ReviewCommentState,
    pub round: Option<u32>,
    pub resolved: bool,
    pub addressed: bool,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewNote {
    pub id: String,
    pub path: String,
    pub line: u32,
    pub current_line: Option<u32>,
    pub kind: ReviewNoteKind,
    pub body: String,
    pub task: Option<String>,
    pub resolved: bool,
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewRound {
    pub number: u32,
    pub verdict: ReviewVerdict,
    pub summary: Option<String>,
    pub message: Option<String>,
    pub viewed_count: u32,
    pub file_count: u32,
    pub comment_count: u32,
    pub submitted_at: f64,
    pub delivered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SetReviewFileViewedRequest {
    pub workstream_id: String,
    pub path: String,
    pub content_oid: String,
    pub viewed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AddReviewCommentRequest {
    pub workstream_id: String,
    pub path: String,
    pub side: ReviewSide,
    pub line: u32,
    pub anchor_text: String,
    pub body: String,
    pub parent_id: Option<String>,
    /// Answers an agent note and resolves it; the note's location replaces the request's.
    #[serde(default)]
    pub reply_to_note_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SubmitReviewRequest {
    pub workstream_id: String,
    pub verdict: ReviewVerdict,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SubmitReviewResult {
    pub round: ReviewRound,
    /// Request changes only: what to deliver to the implement session.
    pub message: Option<String>,
}

/// Lines either side of a comment's line kept as its context.
const CONTEXT_RADIUS: usize = 2;

pub struct ReviewService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn reviews(&self) -> ReviewService<'_> {
        ReviewService { runtime: self }
    }
}

impl ReviewService<'_> {
    /// Creates the review on first read but never announces it: the event's
    /// listeners refetch this.
    pub async fn get(&self, workstream_id: &str) -> Result<ReviewView> {
        ensure_running(self.runtime)?;
        let item = self.reviewable(workstream_id).await?;
        let db = self.runtime.db.clone();
        blocking(move || {
            let worktree = Path::new(&item.worktree_path);
            if !worktree.is_dir() {
                return Err(CoreError::WorkstreamWorktreeMissing {
                    path: item.worktree_path.clone(),
                });
            }
            let review = db.ensure_review(&item.id)?;
            if let Err(e) = sidecar::ensure_review_dir_excluded(worktree) {
                tracing::warn!(
                    "workstream {}: could not exclude the review dir: {e}",
                    item.id
                );
            }
            let files = diff::review_changed_files(worktree, &item.base_branch)?;
            ingest_notes(&db, worktree, &files, &review)?;
            let stored = db.viewed_map(&review.id)?;
            let viewed = files
                .files
                .iter()
                .filter(|f| stored.get(&f.path) == Some(&f.content_oid))
                .map(|f| f.path.clone())
                .collect();
            let comments = db.list_comments(&review.id)?;
            let notes = db.list_notes(&review.id)?;
            let rounds = db.list_rounds(&review.id)?;
            let changed: Vec<String> =
                match rounds.iter().rev().find(|r| {
                    r.verdict == ReviewVerdict::RequestChanges && r.delivered_at.is_some()
                }) {
                    Some(round) => {
                        let snapshot = db.round_files(&review.id, round.number)?;
                        files
                            .files
                            .iter()
                            .filter(|f| snapshot.get(&f.path) != Some(&f.content_oid))
                            .map(|f| f.path.clone())
                            .collect()
                    }
                    None => Vec::new(),
                };
            let changed_set: HashSet<&str> = changed.iter().map(String::as_str).collect();

            let (comment_lines, addressed, note_lines) = {
                let wanted = comments.iter().map(|c| (c.side, c.path.as_str())).chain(
                    notes
                        .iter()
                        .map(|n| (ReviewSide::Modified, n.path.as_str())),
                );
                let texts = diff::side_texts(worktree, &files, wanted);
                let lines = split_texts(&texts);
                let current = |side, path: &str, line, anchor: &str| {
                    reanchor(lines.get(&(side, path))?, line, anchor)
                };
                let comment_lines: Vec<_> = comments
                    .iter()
                    .map(|c| current(c.side, &c.path, c.line, &c.anchor_text))
                    .collect();
                let addressed: Vec<bool> = comments
                    .iter()
                    .zip(&comment_lines)
                    .map(|(c, line)| {
                        c.state == ReviewCommentState::Sent
                            && changed_set.contains(c.path.as_str())
                            && match line {
                                None => true,
                                Some(line) => {
                                    context(lines.get(&(c.side, c.path.as_str())), *line)
                                        != c.sent_context
                                }
                            }
                    })
                    .collect();
                let note_lines: Vec<_> = notes
                    .iter()
                    .map(|n| current(ReviewSide::Modified, &n.path, n.line, &n.anchor_text))
                    .collect();
                (comment_lines, addressed, note_lines)
            };

            Ok(ReviewView {
                id: review.id,
                workstream_id: review.workstream_id,
                status: review.status,
                files,
                viewed,
                comments: comments
                    .into_iter()
                    .zip(comment_lines)
                    .zip(addressed)
                    .map(|((row, current_line), addressed)| {
                        comment_view(row, current_line, addressed)
                    })
                    .collect(),
                notes: notes
                    .into_iter()
                    .zip(note_lines)
                    .map(|(row, current_line)| note_view(row, current_line))
                    .collect(),
                rounds: rounds.into_iter().map(round_view).collect(),
                changed_since_round: changed,
            })
        })
        .await
    }

    pub async fn set_viewed(&self, req: SetReviewFileViewedRequest) -> Result<()> {
        let SetReviewFileViewedRequest {
            workstream_id,
            path,
            content_oid,
            viewed,
        } = req;
        self.on_review(&workstream_id, move |db, review_id| {
            if viewed {
                db.set_viewed(review_id, &path, &content_oid)
            } else {
                db.clear_viewed(review_id, &path)
            }
        })
        .await?;
        self.announce(&workstream_id);
        Ok(())
    }

    /// A reply is placed on its parent's line, a note reply on its note's; the request's
    /// location is ignored. A comment with a parent is a thread reply, so it never answers a note.
    pub async fn add_comment(&self, req: AddReviewCommentRequest) -> Result<ReviewComment> {
        ensure_running(self.runtime)?;
        let item = self.reviewable(&req.workstream_id).await?;
        if req.body.trim().is_empty() {
            return Err(CoreError::ReviewCommentEmpty);
        }
        if req.parent_id.is_none() && req.reply_to_note_id.is_none() {
            if req.line == 0 {
                return Err(CoreError::ReviewCommentLineInvalid);
            }
            if req.path.is_empty() {
                return Err(CoreError::InvalidPath { path: req.path });
            }
            safe_join(Path::new(&item.worktree_path), &req.path)?;
        }
        let db = self.runtime.db.clone();
        let workstream_id = item.id.clone();
        let row = blocking(move || {
            let review = db.ensure_review(&workstream_id)?;
            let (parent_id, note_id, path, side, line, anchor_text) =
                match (req.parent_id, req.reply_to_note_id) {
                    (Some(parent_id), _) => {
                        let parent =
                            db.review_comment(&review.id, &parent_id)?.ok_or_else(|| {
                                CoreError::ReviewCommentNotFound {
                                    id: parent_id.clone(),
                                }
                            })?;
                        (
                            Some(parent.id),
                            None,
                            parent.path,
                            parent.side,
                            parent.line,
                            parent.anchor_text,
                        )
                    }
                    (None, Some(note_id)) => {
                        let note = db.review_note(&review.id, &note_id)?.ok_or_else(|| {
                            CoreError::ReviewNoteNotFound {
                                id: note_id.clone(),
                            }
                        })?;
                        (
                            None,
                            Some(note.id),
                            note.path,
                            ReviewSide::Modified,
                            note.line,
                            note.anchor_text,
                        )
                    }
                    (None, None) => (None, None, req.path, req.side, req.line, req.anchor_text),
                };
            let now = now_ms();
            let row = ReviewCommentRow {
                id: uuid::Uuid::new_v4().to_string(),
                parent_id,
                reply_to_note_id: note_id,
                path,
                side,
                line,
                anchor_text,
                body: req.body,
                state: ReviewCommentState::Pending,
                round: None,
                sent_context: None,
                resolved_at: None,
                created_at: now,
                updated_at: now,
            };
            db.insert_comment(&review.id, &row)?;
            Ok(row)
        })
        .await?;
        self.announce(&item.id);
        let line = row.line;
        Ok(comment_view(row, Some(line), false))
    }

    pub async fn edit_comment(
        &self,
        workstream_id: &str,
        comment_id: &str,
        body: &str,
    ) -> Result<()> {
        if body.trim().is_empty() {
            return Err(CoreError::ReviewCommentEmpty);
        }
        let (id, body) = (comment_id.to_string(), body.to_string());
        let change = self
            .on_review(workstream_id, move |db, review_id| {
                db.update_comment_body(review_id, &id, &body)
            })
            .await?;
        self.settle(workstream_id, comment_id, change)
    }

    pub async fn delete_comment(&self, workstream_id: &str, comment_id: &str) -> Result<()> {
        let id = comment_id.to_string();
        let change = self
            .on_review(workstream_id, move |db, review_id| {
                db.delete_comment(review_id, &id)
            })
            .await?;
        self.settle(workstream_id, comment_id, change)
    }

    pub async fn resolve_comment(
        &self,
        workstream_id: &str,
        comment_id: &str,
        resolved: bool,
    ) -> Result<()> {
        let id = comment_id.to_string();
        let found = self
            .on_review(workstream_id, move |db, review_id| {
                db.set_comment_resolved(review_id, &id, resolved)
            })
            .await?;
        if !found {
            return Err(CoreError::ReviewCommentNotFound {
                id: comment_id.to_string(),
            });
        }
        self.announce(workstream_id);
        Ok(())
    }

    pub async fn resolve_note(
        &self,
        workstream_id: &str,
        note_id: &str,
        resolved: bool,
    ) -> Result<()> {
        let id = note_id.to_string();
        let found = self
            .on_review(workstream_id, move |db, review_id| {
                db.set_note_resolved(review_id, &id, resolved)
            })
            .await?;
        if !found {
            return Err(CoreError::ReviewNoteNotFound {
                id: note_id.to_string(),
            });
        }
        self.announce(workstream_id);
        Ok(())
    }

    /// Request changes leaves the round undelivered until `mark_delivered`; the
    /// caller delivers the returned message in between.
    pub async fn submit(&self, req: SubmitReviewRequest) -> Result<SubmitReviewResult> {
        ensure_running(self.runtime)?;
        let item = self.reviewable(&req.workstream_id).await?;
        let db = self.runtime.db.clone();
        let result = blocking(move || {
            let worktree = Path::new(&item.worktree_path);
            if !worktree.is_dir() {
                return Err(CoreError::WorkstreamWorktreeMissing {
                    path: item.worktree_path.clone(),
                });
            }
            let review = db.ensure_review(&item.id)?;
            let files = diff::review_changed_files(worktree, &item.base_branch)?;
            let stored = db.viewed_map(&review.id)?;
            let viewed_count = files
                .files
                .iter()
                .filter(|f| stored.get(&f.path) == Some(&f.content_oid))
                .count() as u32;
            let file_count = files.files.len() as u32;
            let snapshot: Vec<(String, String)> = files
                .files
                .iter()
                .map(|f| (f.path.clone(), f.content_oid.clone()))
                .collect();
            let all = db.list_comments(&review.id)?;
            let unsent = in_thread_order(&all);
            let summary = req
                .summary
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let (taken, message, to, status): (Vec<RoundComment>, _, _, _) = match req.verdict {
                // Saved comments stay on the round that saved them.
                ReviewVerdict::Comment | ReviewVerdict::Approve => {
                    let taken = unsent
                        .iter()
                        .filter(|c| c.state == ReviewCommentState::Pending)
                        .map(|c| RoundComment {
                            id: c.id.clone(),
                            sent_context: None,
                        })
                        .collect();
                    let status =
                        (req.verdict == ReviewVerdict::Approve).then_some(ReviewStatus::Approved);
                    (taken, None, ReviewCommentState::Saved, status)
                }
                ReviewVerdict::RequestChanges => {
                    let texts = diff::side_texts(
                        worktree,
                        &files,
                        unsent.iter().map(|c| (c.side, c.path.as_str())),
                    );
                    let lines = split_texts(&texts);
                    let placed: Vec<(u32, Option<String>)> = unsent
                        .iter()
                        .map(|c| {
                            let side_lines = lines.get(&(c.side, c.path.as_str()));
                            let current =
                                side_lines.and_then(|l| reanchor(l, c.line, &c.anchor_text));
                            let ctx = current.and_then(|line| context(side_lines, line));
                            (current.unwrap_or(c.line), ctx)
                        })
                        .collect();
                    let notes = db.list_notes(&review.id)?;
                    let items: Vec<MessageComment<'_>> = unsent
                        .iter()
                        .zip(&placed)
                        .map(|(c, (line, _))| MessageComment {
                            path: &c.path,
                            line: *line,
                            removed: c.side == ReviewSide::Original,
                            body: &c.body,
                            note: c
                                .reply_to_note_id
                                .as_deref()
                                .and_then(|id| notes.iter().find(|n| n.id == id))
                                .map(|n| MessageNote {
                                    kind: n.kind,
                                    body: &n.body,
                                }),
                        })
                        .collect();
                    let text =
                        message::compose(viewed_count, file_count, summary.as_deref(), &items);
                    let taken = unsent
                        .iter()
                        .zip(placed)
                        .map(|(c, (_, sent_context))| RoundComment {
                            id: c.id.clone(),
                            sent_context,
                        })
                        .collect();
                    (taken, Some(text), ReviewCommentState::Sending, None)
                }
            };

            let round = NewReviewRound {
                verdict: req.verdict,
                summary,
                message: message.clone(),
                viewed_count,
                file_count,
            };
            match db.submit_round(&review.id, &round, &snapshot, &taken, to, status)? {
                RoundSubmit::Submitted(row) => Ok(SubmitReviewResult {
                    round: round_view(row),
                    message,
                }),
                RoundSubmit::Undelivered(round) => Err(CoreError::ReviewRoundUndelivered { round }),
            }
        })
        .await?;
        self.announce(&req.workstream_id);
        Ok(result)
    }

    /// Idempotent: a round already delivered stays as it is.
    pub async fn mark_delivered(&self, workstream_id: &str, round: u32) -> Result<()> {
        let found = self
            .on_review(workstream_id, move |db, review_id| {
                db.mark_round_delivered(review_id, round)
            })
            .await?;
        if !found {
            return Err(CoreError::ReviewRoundNotFound { round });
        }
        self.announce(workstream_id);
        Ok(())
    }

    async fn reviewable(&self, workstream_id: &str) -> Result<Workstream> {
        let item = self.runtime.workstreams().find(workstream_id).await?;
        if item.is_folder() {
            return Err(CoreError::WorkstreamFolderNoGit);
        }
        Ok(item)
    }

    /// Scopes `write` to this workstream's review, so another's ids are never found.
    async fn on_review<T: Send + 'static>(
        &self,
        workstream_id: &str,
        write: impl FnOnce(&Db, &str) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        ensure_running(self.runtime)?;
        let item = self.reviewable(workstream_id).await?;
        let db = self.runtime.db.clone();
        blocking(move || {
            let review = db.ensure_review(&item.id)?;
            write(&db, &review.id)
        })
        .await
    }

    fn settle(&self, workstream_id: &str, comment_id: &str, change: CommentChange) -> Result<()> {
        match change {
            CommentChange::Applied => {
                self.announce(workstream_id);
                Ok(())
            }
            CommentChange::Missing => Err(CoreError::ReviewCommentNotFound {
                id: comment_id.to_string(),
            }),
            CommentChange::Locked => Err(CoreError::ReviewCommentLocked),
        }
    }

    fn announce(&self, workstream_id: &str) {
        let _ = self.runtime.events.send(RuntimeEvent::ReviewChanged {
            workstream_id: workstream_id.to_string(),
        });
    }
}

fn ingest_notes(
    db: &Db,
    worktree: &Path,
    files: &ReviewFileList,
    review: &ReviewRow,
) -> Result<()> {
    let (found, offset) =
        sidecar::read_new_notes(worktree, &review.workstream_id, review.notes_offset);
    if found.is_empty() && offset == review.notes_offset {
        return Ok(());
    }
    let texts = diff::side_texts(
        worktree,
        files,
        found
            .iter()
            .map(|n| (ReviewSide::Modified, n.path.as_str())),
    );
    let rows: Vec<_> = found
        .iter()
        .map(|n| {
            let anchor_text = texts
                .get(&(ReviewSide::Modified, n.path.as_str()))
                .and_then(|text| text.as_deref())
                .and_then(|text| text.lines().nth(n.line as usize - 1))
                .map(|line| line.trim_end().to_string())
                .unwrap_or_default();
            NewReviewNote {
                fingerprint: n.fingerprint.clone(),
                path: n.path.clone(),
                line: n.line,
                anchor_text,
                kind: n.kind,
                body: n.body.clone(),
                task: n.task.clone(),
            }
        })
        .collect();
    db.ingest_notes(&review.id, &rows, offset)
}

fn split_texts<'t, 'p>(
    texts: &'t HashMap<(ReviewSide, &'p str), Option<String>>,
) -> HashMap<(ReviewSide, &'p str), Vec<&'t str>> {
    texts
        .iter()
        .filter_map(|(key, text)| Some((*key, text.as_deref()?.lines().collect())))
        .collect()
}

/// The lines around `line` (1-based), trailing whitespace dropped like re-anchoring does.
fn context(lines: Option<&Vec<&str>>, line: u32) -> Option<String> {
    let lines = lines?;
    let at = (line as usize).checked_sub(1)?;
    let from = at.saturating_sub(CONTEXT_RADIUS);
    let to = (at + CONTEXT_RADIUS + 1).min(lines.len());
    let window = lines.get(from..to)?;
    Some(
        window
            .iter()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Unsent comments, each thread's root first and its replies after it.
fn in_thread_order(all: &[ReviewCommentRow]) -> Vec<&ReviewCommentRow> {
    let index: HashMap<&str, usize> = all
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    let root_of = |mut i: usize| {
        let mut seen = HashSet::new();
        while let Some(parent) = all[i].parent_id.as_deref().and_then(|p| index.get(p)) {
            if !seen.insert(i) {
                break;
            }
            i = *parent;
        }
        i
    };
    let mut unsent: Vec<(usize, usize)> = all
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            matches!(
                c.state,
                ReviewCommentState::Pending | ReviewCommentState::Saved
            )
        })
        .map(|(i, _)| (root_of(i), i))
        .collect();
    unsent.sort_unstable();
    unsent.into_iter().map(|(_, i)| &all[i]).collect()
}

fn comment_view(
    row: ReviewCommentRow,
    current_line: Option<u32>,
    addressed: bool,
) -> ReviewComment {
    ReviewComment {
        id: row.id,
        parent_id: row.parent_id,
        reply_to_note_id: row.reply_to_note_id,
        path: row.path,
        side: row.side,
        line: row.line,
        current_line,
        anchor_text: row.anchor_text,
        body: row.body,
        state: row.state,
        round: row.round,
        resolved: row.resolved_at.is_some(),
        addressed,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn note_view(row: ReviewNoteRow, current_line: Option<u32>) -> ReviewNote {
    ReviewNote {
        id: row.id,
        path: row.path,
        line: row.line,
        current_line,
        kind: row.kind,
        body: row.body,
        task: row.task,
        resolved: row.resolved_at.is_some(),
        created_at: row.created_at,
    }
}

fn round_view(row: ReviewRoundRow) -> ReviewRound {
    ReviewRound {
        number: row.number,
        verdict: row.verdict,
        summary: row.summary,
        message: row.message,
        viewed_count: row.viewed_count,
        file_count: row.file_count,
        comment_count: row.comment_count,
        submitted_at: row.submitted_at,
        delivered: row.delivered_at.is_some(),
    }
}
