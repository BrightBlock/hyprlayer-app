//! Reading and writing `.hlprompts` files — the on-disk form of an exported
//! prompt-library bundle. The webview drives the native file dialog (open for
//! import, save for export) and hands the chosen path here; these commands only
//! touch that one path.
//!
//! Deliberately thin: prompts are a frontend-owned settings concept, so the
//! serialize/merge logic lives in `features/prompts/promptsIo.ts`. All this
//! layer does is move UTF-8 text between disk and the webview, with a size cap
//! so a mis-picked (huge or binary) file can't be streamed into the frontend.

use crate::commands::blocking;
use crate::error::{AppError, CmdResult};

/// Largest `.hlprompts` file we'll read. A prompt library is a few KB of text;
/// anything past this is a wrong or hostile file, not a prompt export.
const MAX_PROMPTS_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Read a `.hlprompts` file's UTF-8 text for the webview to parse. `path` comes
/// from the native open dialog, so it is whatever file the user picked.
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
        // `read_to_string` fails on non-UTF-8 input, which is exactly the guard
        // we want: a `.hlprompts` file is always JSON text.
        std::fs::read_to_string(&path).map_err(|e| AppError {
            code: "prompts_file_unreadable".into(),
            message: format!("Couldn't read {path}: {e}"),
            remedy: Some("The file may be corrupted or not a Hyprlayer prompt export.".into()),
        })
    })
    .await?
}

/// Write `contents` to `path` (chosen via the native save dialog). The frontend
/// has already ensured the path ends in `.hlprompts` and built the JSON body.
#[tauri::command]
#[specta::specta]
pub async fn write_prompts_file(path: String, contents: String) -> CmdResult<()> {
    blocking("write_prompts_file", move || {
        std::fs::write(&path, contents.as_bytes())
            .map_err(|e| AppError::internal(format!("write {path}: {e}")))
    })
    .await?
}
