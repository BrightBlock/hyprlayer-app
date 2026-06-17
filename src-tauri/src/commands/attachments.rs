//! Persisting prompt attachments (dropped files, pasted screenshots) into the
//! workstream's worktree, so volatile sources survive long enough for the agent
//! to read them. The motivating case is a macOS screenshot dragged straight off
//! the capture thumbnail — its source lives under
//! `/var/folders/.../TemporaryItems/NSIRD_screencaptureui_*/` and the OS reaps
//! it moments later — but the same applies to clipboard image bytes (which have
//! no path at all).
//!
//! Files land in `<worktree>/.hyprlayer/attachments/`. The worktree is the
//! agent's cwd, so it can `Read` them with no extra `--add-dir`. A self-ignoring
//! `.gitignore` (`*`) in `.hyprlayer/` keeps them out of git, and the diff
//! surface filters `.hyprlayer/` regardless (see `gitops::worktree_diff`).

use std::path::{Path, PathBuf};

use tauri::State;

use super::db_task;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// The worktree path for a workstream, or `workstream_not_found`.
async fn worktree_path(state: &AppState, workstream_id: &str) -> CmdResult<String> {
    let id = workstream_id.to_string();
    db_task(state.db.clone(), move |db| db.get_workstream(&id))
        .await?
        .map(|ws| ws.worktree_path)
        .ok_or_else(|| AppError {
            code: "workstream_not_found".into(),
            message: format!("No workstream with id {workstream_id}"),
            remedy: None,
        })
}

/// `<worktree>/.hyprlayer/attachments`, created on demand. Drops a self-ignoring
/// `.gitignore` into `.hyprlayer/` the first time so nothing here is ever staged.
fn ensure_attachments_dir(worktree: &str) -> CmdResult<PathBuf> {
    let internal = Path::new(worktree).join(".hyprlayer");
    let dir = internal.join("attachments");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::internal(format!("create attachments dir: {e}")))?;
    let ignore = internal.join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(&ignore, "*\n");
    }
    Ok(dir)
}

/// Reduce an arbitrary file name to a safe basename for the attachments dir.
fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || c == '/' || c == '\\' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

/// A collision-proof destination inside `dir` for `file_name` — a short random
/// prefix means re-pasting the same name never clobbers an earlier attachment.
fn unique_dest(dir: &Path, file_name: &str) -> PathBuf {
    let id = uuid::Uuid::new_v4().simple().to_string();
    dir.join(format!("{}-{}", &id[..8], sanitize_file_name(file_name)))
}

/// Copy a dropped file (often a volatile screenshot temp file) into the
/// workstream's attachments dir. Returns the new stable absolute path.
#[tauri::command]
#[specta::specta]
pub async fn import_attachment(
    state: State<'_, AppState>,
    workstream_id: String,
    src_path: String,
) -> CmdResult<String> {
    let worktree = worktree_path(&state, &workstream_id).await?;
    let name = Path::new(&src_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_string());
    blocking_fs(move || {
        let dir = ensure_attachments_dir(&worktree)?;
        let dest = unique_dest(&dir, &name);
        std::fs::copy(&src_path, &dest)
            .map_err(|e| AppError::internal(format!("copy attachment {src_path}: {e}")))?;
        Ok(dest.to_string_lossy().into_owned())
    })
    .await
}

/// Write raw bytes (a pasted clipboard image) into the workstream's attachments
/// dir under `file_name`. Returns the new stable absolute path.
#[tauri::command]
#[specta::specta]
pub async fn save_attachment(
    state: State<'_, AppState>,
    workstream_id: String,
    file_name: String,
    data: Vec<u8>,
) -> CmdResult<String> {
    let worktree = worktree_path(&state, &workstream_id).await?;
    blocking_fs(move || {
        let dir = ensure_attachments_dir(&worktree)?;
        let dest = unique_dest(&dir, &file_name);
        std::fs::write(&dest, &data)
            .map_err(|e| AppError::internal(format!("write attachment: {e}")))?;
        Ok(dest.to_string_lossy().into_owned())
    })
    .await
}

/// Run attachment filesystem work off the async runtime (copies/writes can be
/// multi-MB — they must not block a runtime worker), flattening the join error.
async fn blocking_fs<F>(f: F) -> CmdResult<String>
where
    F: FnOnce() -> CmdResult<String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("attachment task: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths_and_control_chars() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("a/b/c.png"), "c.png");
        assert_eq!(sanitize_file_name(""), "attachment");
        assert_eq!(sanitize_file_name("Screenshot 1.png"), "Screenshot 1.png");
    }

    #[test]
    fn unique_dest_keeps_name_and_dedupes() {
        let dir = Path::new("/tmp/x");
        let a = unique_dest(dir, "shot.png");
        let b = unique_dest(dir, "shot.png");
        assert!(a.to_string_lossy().ends_with("-shot.png"));
        assert_ne!(a, b);
    }
}
