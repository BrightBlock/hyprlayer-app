use hyprlayer_core::runtime::attachments as core_attachments;
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
    state
        .runtime
        .attachments()
        .import(core_attachments::ImportAttachmentRequest {
            workstream_id,
            src_path,
        })
        .await
        .map(|path| path.to_string_lossy().into_owned())
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
        .runtime
        .attachments()
        .save(core_attachments::SaveAttachmentRequest {
            workstream_id,
            file_name,
            data,
        })
        .await
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(AppError::from)
}
