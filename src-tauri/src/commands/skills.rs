use tauri::State;

use hyprlayer_core::models::{SkillScope, SkillTarget};
use hyprlayer_core::skill_sources::skills_sh::{
    SkillLeaderboardPage, SkillSearchResult, SkillsShView,
};
use hyprlayer_core::skill_sources::{
    SkillInstallPreview, SkillInstallReport, SkillLockStatus, SkillRestoreReport, SkillSourceInput,
    SkillUpdate, SkillfishCandidate,
};
use hyprlayer_core::skills::{SkillAssetText, SkillFile};

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_skill_files(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<SkillFile>> {
    state
        .client()?
        .list_skill_files(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn read_skill_file(
    state: State<'_, AppState>,
    project_path: Option<String>,
    path: String,
) -> CmdResult<String> {
    state
        .client()?
        .read_skill_file(project_path, path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn read_skill_asset(
    state: State<'_, AppState>,
    project_path: Option<String>,
    skill_path: String,
    rel_path: String,
) -> CmdResult<SkillAssetText> {
    state
        .client()?
        .read_skill_asset(project_path, skill_path, rel_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn preview_skill_install(
    state: State<'_, AppState>,
    project_path: Option<String>,
    source: SkillSourceInput,
) -> CmdResult<SkillInstallPreview> {
    state
        .client()?
        .preview_skill_install(project_path, source)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn install_skill(
    state: State<'_, AppState>,
    project_path: Option<String>,
    token: String,
    scope: SkillScope,
    targets: Vec<SkillTarget>,
) -> CmdResult<SkillInstallReport> {
    state
        .client()?
        .install_skill(project_path, token, scope, targets)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_skill(
    state: State<'_, AppState>,
    project_path: Option<String>,
    scope: SkillScope,
    name: String,
) -> CmdResult<()> {
    state
        .client()?
        .remove_skill(project_path, scope, name)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn search_skill_source(
    state: State<'_, AppState>,
    query: String,
) -> CmdResult<Vec<SkillSearchResult>> {
    state
        .client()?
        .search_skill_source(query)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn skill_leaderboard(
    state: State<'_, AppState>,
    view: SkillsShView,
    page: u32,
) -> CmdResult<SkillLeaderboardPage> {
    state
        .client()?
        .skill_leaderboard(view, page)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_skill_locks(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<SkillLockStatus>> {
    state
        .client()?
        .list_skill_locks(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn check_skill_updates(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<SkillUpdate>> {
    state
        .client()?
        .check_skill_updates(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn preview_skill_update(
    state: State<'_, AppState>,
    project_path: Option<String>,
    scope: SkillScope,
    name: String,
) -> CmdResult<SkillInstallPreview> {
    state
        .client()?
        .preview_skill_update(project_path, scope, name)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn restore_skills(
    state: State<'_, AppState>,
    project_path: Option<String>,
    scope: SkillScope,
) -> CmdResult<SkillRestoreReport> {
    state
        .client()?
        .restore_skills(project_path, scope)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_skillfish_candidates(
    state: State<'_, AppState>,
    project_path: Option<String>,
) -> CmdResult<Vec<SkillfishCandidate>> {
    state
        .client()?
        .list_skillfish_candidates(project_path)
        .await
        .map_err(AppError::from)
}
