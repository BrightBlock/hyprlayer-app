use std::path::Path;
use std::time::Duration;

use crate::error::{CoreError, Result};
use crate::process::Exec;
use crate::repos::{classify_repo_path, commit_empty_initial, RepoPathStatus};

const GITIGNORE: &str = "# OS files\n.DS_Store\nThumbs.db\n\n# Editor files\n\
                         .vscode/\n.idea/\n*.swp\n*.swo\n*~\n\n# Temporary files\n*.tmp\n*.bak\n";

const KEEP: &str = ".gitkeep";

pub fn is_thoughts_repo(root: &Path, repos_dir: &str, global_dir: &str) -> bool {
    root.join(dir_or_default(repos_dir, "repos")).is_dir()
        && root.join(dir_or_default(global_dir, "global")).is_dir()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootState {
    Adoptable,
    Scaffoldable,
    Occupied,
    Bare,
}

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

fn is_effectively_empty(root: &Path, also_ignore: &[&str]) -> bool {
    match std::fs::read_dir(root) {
        Ok(entries) => !entries.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !IGNORABLE
                .iter()
                .chain(also_ignore)
                .any(|ig| name.eq_ignore_ascii_case(ig))
        }),
        Err(_) => false,
    }
}

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

fn dir_or_default<'a>(configured: &'a str, fallback: &'a str) -> &'a str {
    if configured.is_empty() {
        fallback
    } else {
        configured
    }
}

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
        // Local commit only, no remote touched — nothing in the overlay applies.
        commit_empty_initial(git, root, &[]).await?;
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

        let half = at("half");
        std::fs::create_dir_all(half.join("repos")).unwrap();
        assert_eq!(
            classify_root(&half, "repos", "global"),
            RootState::Scaffoldable,
            "a half-finished init must be completable, not written off"
        );

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
