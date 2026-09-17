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
        .client()?
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
        .client()?
        .save_attachment(workstream_id, file_name, data)
        .await
        .map_err(AppError::from)
}

/// The read half of `save_attachment` — carries an attachment's bytes back
/// across the wire, so the client can render one that lives on a remote
/// server. Works identically whether the server is local or remote, since it
/// always goes through `state.client()`; the frontend decides when to call it
/// based on connection state. No remote guard.
#[tauri::command]
#[specta::specta]
pub async fn read_attachment(state: State<'_, AppState>, path: String) -> CmdResult<Vec<u8>> {
    state
        .client()?
        .read_attachment(path)
        .await
        .map_err(AppError::from)
}
