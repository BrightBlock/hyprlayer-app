use hyprlayer_core::models::goal::{Goal, GoalDetail, GoalSpec};
use hyprlayer_core::runtime::goals::{DeriveGoalRequest, GoalPatch, StartGoalRequest};
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn start_goal(state: State<'_, AppState>, request: StartGoalRequest) -> CmdResult<Goal> {
    state
        .client()?
        .start_goal(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn pause_goal(state: State<'_, AppState>, goal_id: String) -> CmdResult<Goal> {
    state
        .client()?
        .pause_goal(goal_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn resume_goal(state: State<'_, AppState>, goal_id: String) -> CmdResult<Goal> {
    state
        .client()?
        .resume_goal(goal_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn stop_goal(state: State<'_, AppState>, goal_id: String) -> CmdResult<Goal> {
    state
        .client()?
        .stop_goal(goal_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn update_goal(
    state: State<'_, AppState>,
    goal_id: String,
    patch: GoalPatch,
) -> CmdResult<Goal> {
    state
        .client()?
        .update_goal(goal_id, patch)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_goal(state: State<'_, AppState>, goal_id: String) -> CmdResult<GoalDetail> {
    state
        .client()?
        .get_goal(goal_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_goals(
    state: State<'_, AppState>,
    workstream_id: Option<String>,
    active_only: bool,
) -> CmdResult<Vec<Goal>> {
    state
        .client()?
        .list_goals(workstream_id, active_only)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn derive_goal(
    state: State<'_, AppState>,
    request: DeriveGoalRequest,
) -> CmdResult<GoalSpec> {
    state
        .client()?
        .derive_goal(request)
        .await
        .map_err(AppError::from)
}
