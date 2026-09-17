use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::store::Workstream;

use super::HyprlayerRuntime;

pub type AttachmentPath = PathBuf;

/// An attachment is an image, not a JSON document — cap what crosses the wire
/// rather than streaming an arbitrarily large file into a blob.
pub const MAX_ATTACHMENT_READ_BYTES: u64 = 20 * 1024 * 1024;

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

/// `read_attachment`'s RPC signature carries no `workstream_id`, so the caller
/// supplies every registered worktree path and this accepts the file only if it
/// sits exactly where `ensure_attachments_dir` puts one for one of them:
/// `<worktree>/.hyprlayer/attachments/<file>`.
///
/// Deliberately not "under `worktree_root()`": an `inplace` workstream's
/// worktree is the repo itself, which lives outside that root entirely, so that
/// test refused every attachment belonging to one.
fn validate_attachment_path(worktrees: &[String], path: &Path) -> Result<PathBuf> {
    let invalid = || CoreError::InvalidPath {
        path: path.display().to_string(),
    };
    let canonical = std::fs::canonicalize(path).map_err(|_| invalid())?;
    let parent = canonical.parent();
    let grandparent = parent.and_then(Path::parent);
    let in_attachments_dir = parent.and_then(Path::file_name) == Some(OsStr::new("attachments"))
        && grandparent.and_then(Path::file_name) == Some(OsStr::new(".hyprlayer"));
    // Compare canonically on both sides: a worktree reached through a symlink
    // would otherwise never match the canonicalized file above it.
    let owned_by_a_workstream = || {
        let worktree = match grandparent.and_then(Path::parent) {
            Some(dir) => dir,
            None => return false,
        };
        worktrees
            .iter()
            .any(|candidate| std::fs::canonicalize(candidate).is_ok_and(|known| known == worktree))
    };
    if !in_attachments_dir || !owned_by_a_workstream() {
        return Err(invalid());
    }
    Ok(canonical)
}

fn read_attachment_bytes(worktrees: &[String], path: &Path) -> Result<Vec<u8>> {
    let validated = validate_attachment_path(worktrees, path)?;
    let meta = std::fs::metadata(&validated).map_err(|e| io_context("stat attachment", e))?;
    if meta.len() > MAX_ATTACHMENT_READ_BYTES {
        return Err(CoreError::AttachmentTooLarge);
    }
    std::fs::read(&validated).map_err(|e| io_context("read attachment", e))
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

    /// Carries an attachment's bytes back across the wire — the
    /// remote-rendering counterpart to `save`. No `workstream_id`: the caller
    /// is a webview resolving a stored path, so the path itself is the only
    /// identifier, and `validate_attachment_path` is what keeps it honest —
    /// against every registered worktree, since an `inplace` workstream's lives
    /// outside `worktree_root()`.
    pub async fn read(&self, path: String) -> Result<Vec<u8>> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        let worktrees = blocking(move || db.list_worktree_paths()).await?;
        blocking(move || read_attachment_bytes(&worktrees, &PathBuf::from(&path))).await
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

    #[test]
    fn read_attachment_bytes_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        let attachments_dir = ensure_attachments_dir(&worktree).unwrap();
        let dest = attachments_dir.join("photo.png");
        std::fs::write(&dest, b"some bytes").unwrap();

        let known = vec![worktree.display().to_string()];
        let bytes = read_attachment_bytes(&known, &dest).unwrap();
        assert_eq!(bytes, b"some bytes");
    }

    /// An `inplace` workstream's worktree is the repo itself, which never lives
    /// under `worktree_root()`. The first cut of this guard tested against that
    /// root and so refused every attachment belonging to one — write-only
    /// against a remote host, which is the exact bug the read half exists to fix.
    #[test]
    fn read_attachment_bytes_accepts_an_inplace_worktree_outside_the_worktree_root() {
        let dir = tempfile::tempdir().unwrap();
        let worktree_root = dir.path().join("worktrees");
        std::fs::create_dir_all(&worktree_root).unwrap();
        let inplace = dir.path().join("repos").join("hyprlayer-app");
        let attachments_dir = ensure_attachments_dir(&inplace).unwrap();
        let dest = attachments_dir.join("shot.png");
        std::fs::write(&dest, b"in-place bytes").unwrap();
        assert!(!dest.starts_with(&worktree_root));

        let known = vec![inplace.display().to_string()];
        assert_eq!(
            read_attachment_bytes(&known, &dest).unwrap(),
            b"in-place bytes"
        );
    }

    #[test]
    fn read_attachment_bytes_refuses_a_worktree_no_workstream_claims() {
        let dir = tempfile::tempdir().unwrap();
        let known_worktree = dir.path().join("known");
        ensure_attachments_dir(&known_worktree).unwrap();
        let stranger = dir.path().join("stranger");
        let attachments_dir = ensure_attachments_dir(&stranger).unwrap();
        let dest = attachments_dir.join("secret.png");
        std::fs::write(&dest, b"not yours").unwrap();

        let known = vec![known_worktree.display().to_string()];
        assert!(matches!(
            read_attachment_bytes(&known, &dest),
            Err(CoreError::InvalidPath { .. })
        ));
    }

    #[test]
    fn read_attachment_bytes_refuses_a_path_outside_the_attachments_dir() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        ensure_attachments_dir(&worktree).unwrap();
        let outside = worktree.join("not-an-attachment.png");
        std::fs::write(&outside, b"nope").unwrap();

        let known = vec![worktree.display().to_string()];
        assert!(matches!(
            read_attachment_bytes(&known, &outside),
            Err(CoreError::InvalidPath { .. })
        ));
    }

    #[test]
    fn read_attachment_bytes_refuses_a_file_over_the_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        let attachments_dir = ensure_attachments_dir(&worktree).unwrap();
        let dest = attachments_dir.join("huge.png");
        std::fs::write(&dest, vec![0u8; (MAX_ATTACHMENT_READ_BYTES + 1) as usize]).unwrap();

        let known = vec![worktree.display().to_string()];
        assert!(matches!(
            read_attachment_bytes(&known, &dest),
            Err(CoreError::AttachmentTooLarge)
        ));
    }
}
