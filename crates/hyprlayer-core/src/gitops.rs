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

/// Open a repository, mapping the failure to a typed error.
pub(crate) fn open_repo(path: &Path) -> Result<git2::Repository> {
    git2::Repository::open(path).map_err(|e| CoreError::OutputParse {
        program: "git2 open".into(),
        detail: format!("{}: {}", path.display(), e.message()),
    })
}

/// Status options every status surface uses: untracked files as single
/// entries, submodules excluded.
pub(crate) fn status_options() -> git2::StatusOptions {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);
    opts
}

/// OID of HEAD's configured upstream branch, when one exists.
pub(crate) fn upstream_oid(repo: &git2::Repository, head: &git2::Reference) -> Option<git2::Oid> {
    head.shorthand()
        .and_then(|name| repo.find_branch(name, git2::BranchType::Local).ok())
        .and_then(|b| b.upstream().ok())
        .and_then(|u| u.get().target())
}

pub fn thoughts_repo_status(repo_path: &Path) -> Result<ThoughtsRepoStatus> {
    let repo = open_repo(repo_path)?;

    let dirty = repo
        .statuses(Some(&mut status_options()))
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
        let upstream = upstream_oid(&repo, head_ref)?;
        let (a, b) = repo.graph_ahead_behind(local_oid, upstream).ok()?;
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

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// added | modified | deleted | renamed | other
    pub change: String,
    pub patch: String,
}

/// The working-tree-vs-HEAD diff that every "what changed" surface is built
/// from (the diff panel and the sidebar line counters), so they share one
/// App-managed scratch inside a worktree (`.hyprlayer/`, e.g. prompt
/// attachments). Self-ignored from git, but filtered from the diff too so a
/// dropped screenshot never reads as a working-tree change.
fn is_hyprlayer_internal(path: &str) -> bool {
    path == ".hyprlayer" || path.starts_with(".hyprlayer/")
}

/// definition: untracked content included; `thoughts/` filtered by callers.
fn worktree_diff_raw(repo: &git2::Repository) -> Result<git2::Diff<'_>> {
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(|e| CoreError::OutputParse {
            program: "git2 diff".into(),
            detail: e.message().to_string(),
        })
}

/// Total added/deleted line counts for the working-tree diff vs HEAD,
/// excluding `thoughts/`. Summed from the same per-file patches the diff panel
/// renders, so the sidebar counter and the panel can never disagree.
pub fn worktree_diff_line_counts(repo: &git2::Repository) -> (u32, u32) {
    let Ok(diff) = worktree_diff_raw(repo) else {
        return (0, 0);
    };
    let (mut additions, mut deletions) = (0u32, 0u32);
    for (idx, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if crate::worktree::is_thoughts_path(&path) || is_hyprlayer_internal(&path) {
            continue;
        }
        if let Ok(Some(patch)) = git2::Patch::from_diff(&diff, idx) {
            if let Ok((_context, adds, dels)) = patch.line_stats() {
                additions += adds as u32;
                deletions += dels as u32;
            }
        }
    }
    (additions, deletions)
}

/// Working-tree diff vs HEAD (staged + unstaged + untracked content), with
/// the app-managed `thoughts/` tree excluded.
pub fn worktree_diff(worktree: &Path) -> Result<Vec<FileDiff>> {
    let repo = open_repo(worktree)?;
    let diff = worktree_diff_raw(&repo)?;

    let mut files = Vec::new();
    for (idx, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if crate::worktree::is_thoughts_path(&path) || is_hyprlayer_internal(&path) {
            continue;
        }
        let change = match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Renamed => "renamed",
            git2::Delta::Modified => "modified",
            _ => "other",
        };
        let patch = git2::Patch::from_diff(&diff, idx)
            .ok()
            .flatten()
            .and_then(|mut p| p.to_buf().ok())
            .map(|buf| String::from_utf8_lossy(&buf).to_string())
            .unwrap_or_default();
        files.push(FileDiff {
            path,
            change: change.to_string(),
            patch,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_diff_reports_changes_and_skips_thoughts() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "one\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("a.md")).unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("t", "t@t").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "new\n").unwrap();
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/x.md"), "skip\n").unwrap();

        let files = worktree_diff(dir.path()).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"a.md"));
        assert!(paths.contains(&"b.md"));
        assert!(!paths.iter().any(|p| p.starts_with("thoughts")));
        let a = files.iter().find(|f| f.path == "a.md").unwrap();
        assert_eq!(a.change, "modified");
        assert!(a.patch.contains("-one") && a.patch.contains("+two"));
        assert_eq!(
            files.iter().find(|f| f.path == "b.md").unwrap().change,
            "added"
        );
    }

    #[test]
    fn line_counts_sum_diff_and_skip_thoughts() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "one\ntwo\nthree\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("a.md")).unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("t", "t@t").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        // a.md: drop "two", append "four"+"five" -> 2 additions, 1 deletion.
        std::fs::write(dir.path().join("a.md"), "one\nthree\nfour\nfive\n").unwrap();
        // b.md untracked: 2 added lines.
        std::fs::write(dir.path().join("b.md"), "x\ny\n").unwrap();
        // thoughts/ must not contribute.
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/x.md"), "skip\nskip\n").unwrap();

        let (additions, deletions) = worktree_diff_line_counts(&repo);
        assert_eq!((additions, deletions), (4, 1));
    }

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
