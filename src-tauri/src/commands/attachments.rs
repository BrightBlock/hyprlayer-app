use std::path::{Path, PathBuf};

use tauri::State;

use super::db_task;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

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

fn unique_dest(dir: &Path, file_name: &str) -> PathBuf {
    let id = uuid::Uuid::new_v4().simple().to_string();
    dir.join(format!("{}-{}", &id[..8], sanitize_file_name(file_name)))
}

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
