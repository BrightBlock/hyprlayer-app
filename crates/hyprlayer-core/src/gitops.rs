//! Local-only git operations via git2 (no network — push/pull/fetch go
//! through the system git binary so credential helpers keep working).
//!
//! Phase 1 needs just enough to replace the missing
//! `hyprlayer thoughts status --json`: status of the thoughts repo.

use std::path::Path;

use serde::Serialize;

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsRepoStatus {
    pub branch: Option<String>,
    /// Working-tree entries that differ from HEAD (staged + unstaged + untracked).
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
    pub has_remote: bool,
}

pub fn thoughts_repo_status(repo_path: &Path) -> Result<ThoughtsRepoStatus> {
    let repo = git2::Repository::open(repo_path).map_err(|e| CoreError::OutputParse {
        program: "git2 open".into(),
        detail: format!("{}: {}", repo_path.display(), e.message()),
    })?;

    let mut status_opts = git2::StatusOptions::new();
    status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);
    let dirty = repo
        .statuses(Some(&mut status_opts))
        .map(|s| s.len() as u32)
        .unwrap_or(0);

    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .and_then(|h| h.shorthand())
        .map(|s| s.to_string());

    let (ahead, behind, has_remote) = (|| -> Option<(u32, u32, bool)> {
        let head_ref = head.as_ref()?;
        let local_oid = head_ref.target()?;
        let branch_name = head_ref.shorthand()?;
        let local_branch = repo
            .find_branch(branch_name, git2::BranchType::Local)
            .ok()?;
        let upstream = local_branch.upstream().ok()?;
        let upstream_oid = upstream.get().target()?;
        let (a, b) = repo.graph_ahead_behind(local_oid, upstream_oid).ok()?;
        Some((a as u32, b as u32, true))
    })()
    .unwrap_or((0, 0, false));

    Ok(ThoughtsRepoStatus {
        branch,
        dirty,
        ahead,
        behind,
        has_remote,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_of_fresh_repo() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("note.md"), "hello").unwrap();

        let status = thoughts_repo_status(dir.path()).unwrap();
        assert_eq!(status.dirty, 1);
        assert!(!status.has_remote);
        assert_eq!(status.ahead, 0);
    }

    #[test]
    fn non_repo_errors_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        assert!(thoughts_repo_status(dir.path()).is_err());
    }
}
