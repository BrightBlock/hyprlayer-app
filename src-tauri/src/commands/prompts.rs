use crate::commands::blocking;
use crate::error::{AppError, CmdResult};

const MAX_PROMPTS_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[tauri::command]
#[specta::specta]
pub async fn read_prompts_file(path: String) -> CmdResult<String> {
    blocking("read_prompts_file", move || {
        let meta = std::fs::metadata(&path)
            .map_err(|e| AppError::internal(format!("stat {path}: {e}")))?;
        if meta.len() > MAX_PROMPTS_FILE_BYTES {
            return Err(AppError {
                code: "prompts_file_too_large".into(),
                message: "That file is too large to be a prompt export.".into(),
                remedy: Some("Choose a .hlprompts file exported from Hyprlayer.".into()),
            });
        }
        std::fs::read_to_string(&path).map_err(|e| AppError {
            code: "prompts_file_unreadable".into(),
            message: format!("Couldn't read {path}: {e}"),
            remedy: Some("The file may be corrupted or not a Hyprlayer prompt export.".into()),
        })
    })
    .await?
}

#[tauri::command]
#[specta::specta]
pub async fn write_prompts_file(path: String, contents: String) -> CmdResult<()> {
    blocking("write_prompts_file", move || {
        std::fs::write(&path, contents.as_bytes())
            .map_err(|e| AppError::internal(format!("write {path}: {e}")))
    })
    .await?
}
