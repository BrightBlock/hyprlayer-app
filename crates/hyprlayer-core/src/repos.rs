//! Project repositories.
//!
//! A *project* is a real git repo the app drives worktrees from. It comes from
//! one of two paths: cloning a hosted repo (a GitHub URL) into
//! `~/hyprlayer/repos`, or importing an existing repo from disk — which is
//! registered *in place*, never copied, so the app shares the user's working
//! copy instead of duplicating it. Worktrees are always cut from the project
//! repo, never from another worktree, so a project never breaks when a worktree
//! is merged or removed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::Exec;

/// `~/hyprlayer/repos` — sibling of the worktree root; the app-owned canonical
/// clones live here, one directory per project.
pub fn default_repos_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hyprlayer")
        .join("repos")
}

/// Derive a filesystem-safe project directory name from a clone source — the
/// final path segment with any `.git` suffix and surrounding slashes stripped.
/// `https://github.com/acme/Cloud-Director.git` → `Cloud-Director`,
/// `git@github.com:acme/cloud-director.git` → `cloud-director`,
/// `/Users/jt/code/myrepo/` → `myrepo`. Returns None for inputs with no usable
/// segment (so the caller can reject them rather than clone into a junk path).
pub fn repo_name_from_url(source: &str) -> Option<String> {
    let trimmed = source.trim().trim_end_matches('/');
    // Both URL and scp-style (`host:owner/repo`) sources end with the repo
    // after the final `/` or `:`.
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    // Keep it to a single safe path component — never one that could escape
    // the repos root.
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return None;
    }
    Some(name.to_string())
}

/// `git -C <repo> remote get-url origin`, or None when there's no origin (a
/// local-only repo) or the call fails. Lets an imported on-disk repo be
/// re-cloned from the same hosted source it already tracks.
pub async fn origin_url(git: &Path, repo: &Path) -> Option<String> {
    let out = Exec::new(git)
        .arg("-C")
        .arg(repo)
        .args(["remote", "get-url", "origin"])
        .timeout(Duration::from_secs(15))
        .run()
        .await
        .ok()?;
    let url = out.stdout.trim();
    (!url.is_empty()).then(|| url.to_string())
}

/// `git clone <source> <dest>`. `source` is a URL or a local path; `dest` must
/// not already exist. Network-bound, so a generous timeout.
pub async fn clone_repo(git: &Path, source: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Err(CoreError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("a repository already exists at {}", dest.display()),
        )));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dest_str = dest.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "git clone".into(),
        detail: "destination path is not valid UTF-8".into(),
    })?;
    Exec::new(git)
        .args(["clone", source, dest_str])
        .timeout(Duration::from_secs(600))
        .run()
        .await?;
    Ok(())
}

/// `git init -b main <path>` — creates the directory if needed, then initialises
/// a new empty git repository. Used by onboarding's "create new thoughts repo"
/// path so users don't have to leave the app.
pub async fn init_repo(git: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(dest)?;
    let dest_str = dest.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "git init".into(),
        detail: "destination path is not valid UTF-8".into(),
    })?;
    Exec::new(git)
        .args(["init", "-b", "main", dest_str])
        .timeout(Duration::from_secs(30))
        .run()
        .await?;
    Ok(())
}

/// `git init -b <branch>` in an existing on-disk folder, then an empty initial
/// commit so `<branch>` is born and usable by `git worktree add`. Existing files
/// are left untracked. Unlike [`init_repo`], the folder already exists, so no
/// dirs are created.
pub async fn init_repo_with_commit(git: &Path, dest: &Path, branch: &str) -> Result<()> {
    let dest_str = dest.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "git init".into(),
        detail: "destination path is not valid UTF-8".into(),
    })?;
    Exec::new(git)
        .args(["init", "-b", branch, dest_str])
        .timeout(Duration::from_secs(30))
        .run()
        .await?;
    commit_empty_initial(git, dest).await
}

/// An empty initial commit that births the current (unborn) branch. Used after
/// a fresh `git init` and to fix up an already-initialized-but-commitless repo;
/// never re-inits or renames the branch. `commit.gpgsign=false` avoids hanging
/// on a non-interactive signer; falls back to a placeholder identity only if
/// none is configured.
pub async fn commit_empty_initial(git: &Path, repo: &Path) -> Result<()> {
    let repo_str = repo.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "git commit".into(),
        detail: "repository path is not valid UTF-8".into(),
    })?;
    let commit = ["-C", repo_str, "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "Initial commit"];
    if Exec::new(git)
        .args(commit)
        .timeout(Duration::from_secs(30))
        .run()
        .await
        .is_err()
    {
        Exec::new(git)
            .args([
                "-C", repo_str,
                "-c", "commit.gpgsign=false",
                "-c", "user.name=hyprlayer",
                "-c", "user.email=hyprlayer@localhost",
                "commit", "--allow-empty", "-m", "Initial commit",
            ])
            .timeout(Duration::from_secs(30))
            .run()
            .await?;
    }
    Ok(())
}

/// Returns `true` when `path` is a git repository root (i.e. `<path>/.git`
/// exists as a directory or a file). Does not verify that it's a valid repo —
/// just the cheapest possible "this looks like git" check.
pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// How a browsed disk path looks as a candidate project, for the Add-project
/// disk flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum RepoPathStatus {
    /// The path doesn't exist or isn't a directory.
    Missing,
    /// Exists, but isn't a git repo — offer to `git init` it.
    NonGit,
    /// A bare repository — no working tree to cut a worktree from.
    Bare,
    /// A git repo whose HEAD is unborn (no commits yet).
    Unborn,
    /// A ready git repo (working tree with at least one commit).
    Ready,
}

/// Classify a disk path for the Add-project flow. Uses a real git open (not a
/// bare `.git`-existence check) so bare and unborn repos aren't misread as
/// plain folders or ready repos.
pub fn classify_repo_path(path: &Path) -> RepoPathStatus {
    if !path.is_dir() {
        return RepoPathStatus::Missing;
    }
    let Ok(repo) = crate::gitops::open_repo(path) else {
        return RepoPathStatus::NonGit;
    };
    if repo.is_bare() {
        return RepoPathStatus::Bare;
    }
    // HEAD errors with `UnbornBranch` when there are no commits yet.
    let unborn = matches!(repo.head(), Err(e) if e.code() == git2::ErrorCode::UnbornBranch);
    if unborn {
        RepoPathStatus::Unborn
    } else {
        RepoPathStatus::Ready
    }
}

/// One of the user's GitHub repos, for the clone dialog's "Recent repos" list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GithubRepo {
    pub name_with_owner: String,
    #[serde(default)]
    pub description: Option<String>,
    pub url: String,
}

/// The user's GitHub repos — personal AND org-member — most-recently-updated
/// first, for the clone dialog's "Recent repos" list. Uses `gh api /user/repos`
/// rather than `gh repo list`: the latter only returns the user's *personal*
/// repos, so org repos (e.g. an employer's) never appear. Errors if gh is
/// missing/unauthenticated — the caller degrades that to an empty list, since
/// recents are a convenience atop the always-present URL field.
pub async fn github_repos(gh: &Path, limit: u32) -> Result<Vec<GithubRepo>> {
    // `/user/repos` defaults to affiliation=owner,collaborator,organization_member,
    // so it spans every repo the user can reach, across personal and orgs.
    let endpoint = format!("user/repos?per_page={}&sort=updated", limit.min(100));
    let out = Exec::new(gh)
        .args(["api", &endpoint])
        .timeout(Duration::from_secs(30))
        .run()
        .await?;

    // The REST shape (full_name / html_url) differs from `gh repo list --json`.
    #[derive(Deserialize)]
    struct ApiRepo {
        full_name: String,
        #[serde(default)]
        description: Option<String>,
        html_url: String,
    }
    let raw: Vec<ApiRepo> =
        serde_json::from_str(&out.stdout).map_err(|e| CoreError::OutputParse {
            program: "gh api user/repos".into(),
            detail: e.to_string(),
        })?;
    Ok(raw
        .into_iter()
        .map(|r| GithubRepo {
            name_with_owner: r.full_name,
            description: r.description,
            url: r.html_url,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_various_source_shapes() {
        let name = |s: &str| repo_name_from_url(s);
        assert_eq!(
            name("https://github.com/acme/repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            name("https://github.com/acme/repo").as_deref(),
            Some("repo")
        );
        assert_eq!(
            name("git@github.com:acme/Cloud-Director.git").as_deref(),
            Some("Cloud-Director")
        );
        assert_eq!(
            name("ssh://git@github.com/acme/repo.git/").as_deref(),
            Some("repo")
        );
        assert_eq!(name("/Users/jt/code/myrepo/").as_deref(), Some("myrepo"));
        // Nothing usable / nothing that could escape the root.
        assert_eq!(name(""), None);
        assert_eq!(name("/"), None);
        assert_eq!(name("   "), None);
    }

    fn sh(repo: &Path, args: &[&str]) {
        // `-c commit.gpgsign=false`: keep test commits hermetic from the user's
        // global git config (e.g. a 1Password SSH signing key that can't sign
        // in a non-interactive test run).
        let out = std::process::Command::new("git")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn init_with_commit_births_a_usable_branch_and_leaves_files_untracked() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("proj");
        std::fs::create_dir(&repo).unwrap();
        // A pre-existing file, as when importing a real (non-git) project folder.
        std::fs::write(repo.join("notes.md"), "hi\n").unwrap();

        let git = which::which("git").unwrap();
        init_repo_with_commit(&git, &repo, "trunk").await.unwrap();

        let r = git2::Repository::open(&repo).unwrap();
        let head = r.head().unwrap();
        // The chosen branch name is what HEAD points at — and it's *born* (has a
        // commit), so a worktree can be cut from it immediately.
        assert!(head.is_branch());
        assert_eq!(head.shorthand(), Some("trunk"));
        let commit = head.peel_to_commit().unwrap();
        assert!(commit.parents().next().is_none(), "it's the initial commit");
        // The initial commit is empty — we didn't stage the user's files.
        assert_eq!(commit.tree().unwrap().len(), 0, "initial commit is empty");

        // The pre-existing file is present but untracked, left for the user to
        // commit deliberately.
        assert!(repo.join("notes.md").exists());
        let statuses = r.statuses(None).unwrap();
        assert!(
            statuses
                .iter()
                .any(|s| s.path() == Some("notes.md") && s.status().contains(git2::Status::WT_NEW)),
            "existing file is left untracked"
        );
    }

    #[tokio::test]
    async fn clones_a_local_repo_and_reads_its_origin() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        sh(&source, &["init", "-b", "main"]);
        std::fs::write(source.join("README.md"), "hello\n").unwrap();
        sh(&source, &["add", "."]);
        sh(&source, &["commit", "-m", "init"]);

        let git = which::which("git").unwrap();
        let dest = dir.path().join("clones").join("myproj");
        clone_repo(&git, source.to_str().unwrap(), &dest)
            .await
            .unwrap();
        assert!(
            dest.join("README.md").exists(),
            "clone checked out the tree"
        );

        // The clone tracks the source as its origin.
        assert_eq!(
            origin_url(&git, &dest).await.as_deref(),
            Some(source.to_str().unwrap())
        );

        // Cloning onto an existing directory is refused, not silently merged.
        let err = clone_repo(&git, source.to_str().unwrap(), &dest)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "io");
    }

    #[test]
    fn classify_missing_and_non_git_paths() {
        let dir = tempfile::tempdir().unwrap();
        // A path that doesn't exist (and a file, not a dir) → Missing.
        assert_eq!(
            classify_repo_path(&dir.path().join("nope")),
            RepoPathStatus::Missing
        );
        // A plain folder with no git → NonGit (offer to init).
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(classify_repo_path(&plain), RepoPathStatus::NonGit);
    }

    #[tokio::test]
    async fn classify_unborn_becomes_ready_after_initial_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("proj");
        std::fs::create_dir(&repo).unwrap();
        // A fresh `git init` with no commits → Unborn (has `.git`, but nothing to
        // cut a worktree from — the exact case `is_git_repo` misses).
        sh(&repo, &["init", "-b", "main"]);
        assert_eq!(classify_repo_path(&repo), RepoPathStatus::Unborn);

        // Birthing an empty commit makes it usable → Ready.
        let git = which::which("git").unwrap();
        commit_empty_initial(&git, &repo).await.unwrap();
        assert_eq!(classify_repo_path(&repo), RepoPathStatus::Ready);
    }

    #[test]
    fn classify_bare_repo_is_not_mistaken_for_non_git() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.git");
        std::fs::create_dir(&bare).unwrap();
        // A bare repo has no child `.git`, so the old `.git`-existence check read
        // it as a plain folder and would `git init` inside it — corrupting it.
        sh(&bare, &["init", "--bare", "-b", "main"]);
        assert!(!is_git_repo(&bare), "bare repo has no child .git");
        assert_eq!(classify_repo_path(&bare), RepoPathStatus::Bare);
    }
}
