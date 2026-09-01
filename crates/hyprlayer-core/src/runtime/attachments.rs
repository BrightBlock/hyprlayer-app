use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::store::Workstream;

use super::HyprlayerRuntime;

pub type AttachmentPath = PathBuf;

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

fn io_context(label: &str, err: std::io::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(format!("{label}: {err}")))
}

fn ensure_attachments_dir(worktree: &Path) -> Result<PathBuf> {
    let internal = worktree.join(".hyprlayer");
    let dir = internal.join("attachments");
    std::fs::create_dir_all(&dir).map_err(|e| io_context("create attachments dir", e))?;
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

fn reject_escaping_name(name: &str) -> Result<()> {
    let rel = Path::new(name);
    let escapes = rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    if escapes {
        return Err(CoreError::InvalidPath {
            path: name.to_string(),
        });
    }
    Ok(())
}

fn unique_dest(dir: &Path, file_name: &str) -> Result<PathBuf> {
    reject_escaping_name(file_name)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    Ok(dir.join(format!("{}-{}", &id[..8], sanitize_file_name(file_name))))
}

pub struct ImportAttachmentRequest {
    pub workstream_id: String,
    pub src_path: String,
}

pub struct SaveAttachmentRequest {
    pub workstream_id: String,
    pub file_name: String,
    pub data: Vec<u8>,
}

pub struct AttachmentService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn attachments(&self) -> AttachmentService<'_> {
        AttachmentService { runtime: self }
    }
}

impl AttachmentService<'_> {
    async fn find_workstream(&self, id: &str) -> Result<Workstream> {
        let owned = id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_workstream(&owned))
            .await?
            .ok_or_else(|| CoreError::WorkstreamNotFound { id: id.to_string() })
    }

    pub async fn import(&self, req: ImportAttachmentRequest) -> Result<AttachmentPath> {
        ensure_running(self.runtime)?;
        let workstream = self.find_workstream(&req.workstream_id).await?;
        let name = Path::new(&req.src_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".to_string());
        let src_path = req.src_path;
        blocking(move || {
            let dir = ensure_attachments_dir(Path::new(&workstream.worktree_path))?;
            let dest = unique_dest(&dir, &name)?;
            std::fs::copy(&src_path, &dest)
                .map_err(|e| io_context(&format!("copy attachment {src_path}"), e))?;
            Ok(dest)
        })
        .await
    }

    pub async fn save(&self, req: SaveAttachmentRequest) -> Result<AttachmentPath> {
        ensure_running(self.runtime)?;
        let workstream = self.find_workstream(&req.workstream_id).await?;
        blocking(move || {
            let dir = ensure_attachments_dir(Path::new(&workstream.worktree_path))?;
            let dest = unique_dest(&dir, &req.file_name)?;
            std::fs::write(&dest, &req.data).map_err(|e| io_context("write attachment", e))?;
            Ok(dest)
        })
        .await
    }
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
        let a = unique_dest(dir, "shot.png").expect("safe name");
        let b = unique_dest(dir, "shot.png").expect("safe name");
        assert!(a.to_string_lossy().ends_with("-shot.png"));
        assert_ne!(a, b);
    }

    #[test]
    fn unique_dest_rejects_escaping_names() {
        let dir = Path::new("/tmp/x");
        assert!(matches!(
            unique_dest(dir, "../x"),
            Err(CoreError::InvalidPath { .. })
        ));
        assert!(matches!(
            unique_dest(dir, "../../etc/passwd"),
            Err(CoreError::InvalidPath { .. })
        ));
        assert!(matches!(
            unique_dest(dir, "/etc/passwd"),
            Err(CoreError::InvalidPath { .. })
        ));
    }
}
