use hyprlayer_core::repo_settings::ScriptSettings;
use hyprlayer_core::runtime::script_registry::ScriptRuntime;
use hyprlayer_core::scripts::ScriptKind;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_core::CoreError;
use tauri::ipc::Channel;
use tauri::State;

use super::ensure_registered_project;
use super::terminal::pump;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_scripts(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ScriptSettings> {
    ensure_registered_project(&state, &project_path).await?;
    state
        .client()?
        .get_scripts(project_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_scripts(
    state: State<'_, AppState>,
    project_path: String,
    scripts: ScriptSettings,
) -> CmdResult<()> {
    ensure_registered_project(&state, &project_path).await?;
    state
        .client()?
        .set_scripts(project_path, scripts)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_script_runtime(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<ScriptRuntime> {
    state
        .client()?
        .get_script_runtime(workstream_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_running_scripts(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    state
        .client()?
        .list_running_scripts()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_script_output(
    state: State<'_, AppState>,
    workstream_id: String,
    kind: ScriptKind,
) -> CmdResult<Option<String>> {
    state
        .client()?
        .get_script_output(workstream_id, kind)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn script_clear_output(
    state: State<'_, AppState>,
    workstream_id: String,
    kind: ScriptKind,
    terminal_id: Option<String>,
) -> CmdResult<()> {
    state
        .client()?
        .script_clear_output(workstream_id, kind, terminal_id)
        .await
        .map_err(AppError::from)
}

pub fn script_empty(kind: ScriptKind) -> AppError {
    AppError::from(CoreError::ScriptEmpty { kind })
}

#[tauri::command]
#[specta::specta]
pub async fn script_run(
    state: State<'_, AppState>,
    workstream_id: String,
    kind: ScriptKind,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let (terminal_id, stream_id, events) = state
        .client()?
        .script_run_stream(workstream_id, kind, cols, rows)
        .await
        .map_err(AppError::from)?;
    pump(state.client()?.clone(), stream_id, events, on_event);
    Ok(terminal_id)
}

#[tauri::command]
#[specta::specta]
pub async fn script_stop(state: State<'_, AppState>, workstream_id: String) -> CmdResult<()> {
    state
        .client()?
        .script_stop(workstream_id)
        .await
        .map_err(AppError::from)
}
