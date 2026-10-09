use hyprlayer_core::runtime::providers::ValidationSummary;
use hyprlayer_core::runtime::reviews::{
    AddReviewCommentRequest, ReviewComment, ReviewView, SetReviewFileViewedRequest,
    SubmitReviewRequest, SubmitReviewResult,
};
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_review(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<ReviewView> {
    state
        .client()?
        .get_review(workstream_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_review_file_viewed(
    state: State<'_, AppState>,
    req: SetReviewFileViewedRequest,
) -> CmdResult<()> {
    state
        .client()?
        .set_review_file_viewed(req)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn add_review_comment(
    state: State<'_, AppState>,
    req: AddReviewCommentRequest,
) -> CmdResult<ReviewComment> {
    state
        .client()?
        .add_review_comment(req)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn edit_review_comment(
    state: State<'_, AppState>,
    workstream_id: String,
    comment_id: String,
    body: String,
) -> CmdResult<()> {
    state
        .client()?
        .edit_review_comment(workstream_id, comment_id, body)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_review_comment(
    state: State<'_, AppState>,
    workstream_id: String,
    comment_id: String,
) -> CmdResult<()> {
    state
        .client()?
        .delete_review_comment(workstream_id, comment_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_review_comment(
    state: State<'_, AppState>,
    workstream_id: String,
    comment_id: String,
    resolved: bool,
) -> CmdResult<()> {
    state
        .client()?
        .resolve_review_comment(workstream_id, comment_id, resolved)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_review_note(
    state: State<'_, AppState>,
    workstream_id: String,
    note_id: String,
    resolved: bool,
) -> CmdResult<()> {
    state
        .client()?
        .resolve_review_note(workstream_id, note_id, resolved)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn submit_review(
    state: State<'_, AppState>,
    req: SubmitReviewRequest,
) -> CmdResult<SubmitReviewResult> {
    state
        .client()?
        .submit_review(req)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn mark_review_round_delivered(
    state: State<'_, AppState>,
    workstream_id: String,
    round: u32,
) -> CmdResult<()> {
    state
        .client()?
        .mark_review_round_delivered(workstream_id, round)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn summarize_validation_report(
    state: State<'_, AppState>,
    report: String,
) -> CmdResult<Option<ValidationSummary>> {
    state
        .client()?
        .summarize_validation_report(report)
        .await
        .map_err(AppError::from)
}
