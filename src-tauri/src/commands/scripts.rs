use hyprlayer_core::repo_settings::ScriptSettings;
use hyprlayer_core::runtime::script_registry::ScriptRuntime;
use hyprlayer_core::runtime::scripts as core_scripts;
use hyprlayer_core::scripts::ScriptKind;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_core::CoreError;
use tauri::ipc::Channel;
use tauri::State;

use super::ensure_registered_project;
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
        .runtime
        .scripts()
        .settings(core_scripts::ScriptSettingsRequest {
            project_path,
            set: None,
        })
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
        .runtime
        .scripts()
        .settings(core_scripts::ScriptSettingsRequest {
            project_path,
            set: Some(scripts),
        })
        .await
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_script_runtime(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<ScriptRuntime> {
    state
        .runtime
        .scripts()
        .runtime(core_scripts::ScriptRuntimeRequest { workstream_id })
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_running_scripts(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    state
        .runtime
        .scripts()
        .running_ids()
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
        .runtime
        .scripts()
        .output(core_scripts::ScriptOutputRequest {
            workstream_id,
            kind,
            clear: false,
            terminal_id: None,
        })
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
        .runtime
        .scripts()
        .output(core_scripts::ScriptOutputRequest {
            workstream_id,
            kind,
            clear: true,
            terminal_id,
        })
        .await
        .map_err(AppError::from)?;
    Ok(())
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
    let run = state
        .runtime
        .scripts()
        .run(core_scripts::RunScriptRequest {
            workstream_id,
            kind,
            cols,
            rows,
        })
        .await
        .map_err(AppError::from)?;
    Ok(pump(run, on_event))
}

#[tauri::command]
#[specta::specta]
pub async fn script_stop(state: State<'_, AppState>, workstream_id: String) -> CmdResult<()> {
    state
        .runtime
        .scripts()
        .stop(core_scripts::StopScriptRequest { workstream_id })
        .await
        .map_err(AppError::from)
}

fn pump(run: core_scripts::ScriptRun, on_event: Channel<TerminalEvent>) -> String {
    let core_scripts::ScriptRun {
        terminal_id,
        mut live,
    } = run;
    let id = terminal_id.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = live.recv().await {
            if on_event.send(ev).is_err() {
                break;
            }
        }
    });
    id
}
