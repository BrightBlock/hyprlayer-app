//! Thoughts-repository root scaffolding.
//!
//! `thoughts init` accepts a git root only when it's already shaped
//! (`<reposDir>/` + `<globalDir>/`) or effectively empty, anything else is
//! refused for good without `--force`. So the app shapes a root fully at
//! creation rather than leaving a bare `git init` that the first stray file
//! bricks.

use std::path::Path;
use std::time::Duration;

use crate::error::{CoreError, Result};
use crate::process::Exec;
use crate::repos::{classify_repo_path, commit_empty_initial, RepoPathStatus};

/// Byte-for-byte the CLI's, so a later CLI run finds nothing to change.
const GITIGNORE: &str = "# OS files\n.DS_Store\nThumbs.db\n\n# Editor files\n\
                         .vscode/\n.idea/\n*.swp\n*.swo\n*~\n\n# Temporary files\n*.tmp\n*.bak\n";

/// Without a tracked file the dirs are invisible to a clone.
const KEEP: &str = ".gitkeep";

/// Whether `root` presents as a thoughts repo, what the CLI adopts.
pub fn is_thoughts_repo(root: &Path, repos_dir: &str, global_dir: &str) -> bool {
    root.join(dir_or_default(repos_dir, "repos")).is_dir()
        && root.join(dir_or_default(global_dir, "global")).is_dir()
}

/// How a candidate thoughts root looks to [`scaffold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootState {
    /// Already shaped, scaffolding just fills gaps.
    Adoptable,
    /// Absent or holding nothing worth preserving.
    Scaffoldable,
    /// Someone else's populated directory, leave it for the CLI to refuse
    /// rather than restructure a path picked by mistake.
    Occupied,
    /// Bare repo, no working tree to scaffold into.
    Bare,
}

/// Mirrors the CLI's allowlist so both agree on what "empty" means.
const IGNORABLE: &[&str] = &[
    ".git",
    ".gitignore",
    ".gitattributes",
    ".DS_Store",
    "Thumbs.db",
    "README",
    "README.md",
    "README.txt",
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
];

/// `also_ignore` carries the tree names, so a half-shaped root still counts.
fn is_effectively_empty(root: &Path, also_ignore: &[&str]) -> bool {
    match std::fs::read_dir(root) {
        Ok(entries) => !entries.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !IGNORABLE
                .iter()
                .chain(also_ignore)
                .any(|ig| name.eq_ignore_ascii_case(ig))
        }),
        // Can't enumerate isn't "empty", don't scaffold blind.
        Err(_) => false,
    }
}

/// More permissive than the CLI, because the app can repair what the CLI only
/// refuses: non-git roots get a `git init`, and half-shaped ones (init dies
/// between creating `<reposDir>/` and `<globalDir>/`) get completed instead of
/// rejected forever.
pub fn classify_root(root: &Path, repos_dir: &str, global_dir: &str) -> RootState {
    if !root.exists() {
        return RootState::Scaffoldable;
    }
    if classify_repo_path(root) == RepoPathStatus::Bare {
        return RootState::Bare;
    }
    if is_thoughts_repo(root, repos_dir, global_dir) {
        return RootState::Adoptable;
    }
    let own = [
        dir_or_default(repos_dir, "repos"),
        dir_or_default(global_dir, "global"),
    ];
    if is_effectively_empty(root, &own) {
        return RootState::Scaffoldable;
    }
    RootState::Occupied
}

/// Config may carry an empty dir name; the CLI falls back to the standard one,
/// so we must agree (mirrors `worktree::thoughts_links::expected_targets`).
fn dir_or_default<'a>(configured: &'a str, fallback: &'a str) -> &'a str {
    if configured.is_empty() {
        fallback
    } else {
        configured
    }
}

/// Create or complete a thoughts repo at `root`: `git init`, `<reposDir>/`,
/// `<globalDir>/{<user>,shared}`, `.gitignore`, and an initial commit.
///
/// Idempotent. Only a commitless repo gets the commit, so this never sweeps a
/// user's unrelated changes into one. Refuses a bare repo; callers decide
/// whether a populated non-thoughts directory is a legitimate target.
pub async fn scaffold(
    git: &Path,
    root: &Path,
    user: &str,
    repos_dir: &str,
    global_dir: &str,
) -> Result<()> {
    std::fs::create_dir_all(root)?;

    let status = classify_repo_path(root);
    if status == RepoPathStatus::Bare {
        return Err(CoreError::OutputParse {
            program: "thoughts scaffold".into(),
            detail: format!(
                "{} is a bare git repository and has no working tree to scaffold",
                root.display()
            ),
        });
    }
    if status == RepoPathStatus::NonGit {
        let root_str = root.to_str().ok_or_else(|| CoreError::OutputParse {
            program: "git init".into(),
            detail: "thoughts repository path is not valid UTF-8".into(),
        })?;
        Exec::new(git)
            .args(["init", "-b", "main", root_str])
            .timeout(Duration::from_secs(30))
            .run()
            .await?;
    }

    // `<globalDir>/<user>` is the tree the CLI symlinks as `thoughts/<user>`;
    // skipped without a user, since an empty segment re-creates the global dir.
    let repos = root.join(dir_or_default(repos_dir, "repos"));
    let global = root.join(dir_or_default(global_dir, "global"));
    let mut dirs = vec![repos, global.join("shared")];
    let user = user.trim();
    if !user.is_empty() {
        dirs.push(global.join(user));
    }
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
        let keep = dir.join(KEEP);
        if !keep.exists() {
            std::fs::write(&keep, "")?;
        }
    }

    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, GITIGNORE)?;
    }

    // A repo with history keeps its working tree untouched; it syncs the new
    // dirs on its next sync.
    if classify_repo_path(root) == RepoPathStatus::Unborn {
        let root_str = root.to_str().ok_or_else(|| CoreError::OutputParse {
            program: "git add".into(),
            detail: "thoughts repository path is not valid UTF-8".into(),
        })?;
        Exec::new(git)
            .args(["-C", root_str, "add", "-A"])
            .timeout(Duration::from_secs(30))
            .run()
            .await?;
        commit_empty_initial(git, root).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_bin() -> std::path::PathBuf {
        which::which("git").unwrap()
    }

    #[tokio::test]
    async fn scaffolds_a_fresh_directory_into_a_thoughts_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thoughts");
        let git = git_bin();

        scaffold(&git, &root, "jt", "repos", "global")
            .await
            .unwrap();

        assert!(root.join(".git").exists(), "should be a git repo");
        assert!(root.join("repos").is_dir());
        assert!(root.join("global/jt").is_dir());
        assert!(root.join("global/shared").is_dir());
        assert!(root.join(".gitignore").is_file());
        assert!(is_thoughts_repo(&root, "repos", "global"));
        // Committed, or it wouldn't survive a clone.
        assert_eq!(classify_repo_path(&root), RepoPathStatus::Ready);
    }

    #[tokio::test]
    async fn completes_an_empty_git_repo_without_reinitialising_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let git = git_bin();
        Exec::new(&git)
            .args(["init", "-b", "trunk", root.to_str().unwrap()])
            .run()
            .await
            .unwrap();

        scaffold(&git, &root, "jt", "repos", "global")
            .await
            .unwrap();

        assert!(is_thoughts_repo(&root, "repos", "global"));
        // The user's branch name is preserved.
        let head = std::fs::read_to_string(root.join(".git/HEAD")).unwrap();
        assert!(head.contains("trunk"), "unexpected HEAD: {head}");
    }

    #[tokio::test]
    async fn is_idempotent_and_keeps_existing_history() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let git = git_bin();
        scaffold(&git, &root, "jt", "repos", "global")
            .await
            .unwrap();
        std::fs::write(root.join(".gitignore"), "custom\n").unwrap();

        scaffold(&git, &root, "jt", "repos", "global")
            .await
            .unwrap();

        // A second pass doesn't overwrite the user's .gitignore.
        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "custom\n"
        );
        assert!(is_thoughts_repo(&root, "repos", "global"));
    }

    #[tokio::test]
    async fn honours_custom_dir_names_and_a_missing_user() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("t");
        let git = git_bin();

        scaffold(&git, &root, "  ", "projects", "shared-notes")
            .await
            .unwrap();

        assert!(root.join("projects").is_dir());
        assert!(root.join("shared-notes/shared").is_dir());
        assert!(is_thoughts_repo(&root, "projects", "shared-notes"));
        // No user, no personal tree.
        assert_eq!(
            std::fs::read_dir(root.join("shared-notes"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn classifies_candidate_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let at = |name: &str| tmp.path().join(name);

        assert_eq!(
            classify_root(&at("nope"), "repos", "global"),
            RootState::Scaffoldable,
            "an absent path is where a new repo goes"
        );

        let empty = at("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(
            classify_root(&empty, "repos", "global"),
            RootState::Scaffoldable
        );

        // A fresh clone carries these and is still free to shape.
        let fresh = at("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::write(fresh.join("README.md"), "hi").unwrap();
        std::fs::write(fresh.join(".gitignore"), "x").unwrap();
        assert_eq!(
            classify_root(&fresh, "repos", "global"),
            RootState::Scaffoldable
        );

        let shaped = at("shaped");
        std::fs::create_dir_all(shaped.join("repos")).unwrap();
        std::fs::create_dir_all(shaped.join("global")).unwrap();
        assert_eq!(
            classify_root(&shaped, "repos", "global"),
            RootState::Adoptable
        );

        // Init creates `repos/` before `global/`; dying between leaves this.
        let half = at("half");
        std::fs::create_dir_all(half.join("repos")).unwrap();
        assert_eq!(
            classify_root(&half, "repos", "global"),
            RootState::Scaffoldable,
            "a half-finished init must be completable, not written off"
        );

        // Someone's notes or a code repo, not ours to restructure.
        let occupied = at("occupied");
        std::fs::create_dir_all(occupied.join("src")).unwrap();
        assert_eq!(
            classify_root(&occupied, "repos", "global"),
            RootState::Occupied
        );
    }

    #[tokio::test]
    async fn completes_a_half_finished_init() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let git = git_bin();
        // The state a mid-run failure leaves; the CLI refuses it forever.
        Exec::new(&git)
            .args(["init", "-b", "main", root.to_str().unwrap()])
            .run()
            .await
            .unwrap();
        std::fs::create_dir_all(root.join("repos")).unwrap();

        scaffold(&git, &root, "jt", "repos", "global")
            .await
            .unwrap();

        assert!(is_thoughts_repo(&root, "repos", "global"));
    }

    #[tokio::test]
    async fn empty_dir_names_fall_back_to_the_standard_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("t");
        let git = git_bin();

        scaffold(&git, &root, "jt", "", "").await.unwrap();

        assert!(root.join("repos").is_dir());
        assert!(root.join("global").is_dir());
    }
}
