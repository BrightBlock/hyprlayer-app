use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn import_attachment(
    state: State<'_, AppState>,
    workstream_id: String,
    src_path: String,
) -> CmdResult<String> {
    if let Some(err) = state.remote_unsupported("Importing an attachment from a local path") {
        return Err(err);
    }
    state
        .client
        .import_attachment(workstream_id, src_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn save_attachment(
    state: State<'_, AppState>,
    workstream_id: String,
    file_name: String,
    data: Vec<u8>,
) -> CmdResult<String> {
    state
        .client
        .save_attachment(workstream_id, file_name, data)
        .await
        .map_err(AppError::from)
}
