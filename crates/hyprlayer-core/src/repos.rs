use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::Exec;

pub fn default_repos_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hyprlayer")
        .join("repos")
}

pub fn repo_name_from_url(source: &str) -> Option<String> {
    let trimmed = source.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return None;
    }
    Some(name.to_string())
}

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

pub async fn commit_empty_initial(git: &Path, repo: &Path) -> Result<()> {
    let repo_str = repo.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "git commit".into(),
        detail: "repository path is not valid UTF-8".into(),
    })?;
    let commit = [
        "-C",
        repo_str,
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--allow-empty",
        "-m",
        "Initial commit",
    ];
    if Exec::new(git)
        .args(commit)
        .timeout(Duration::from_secs(30))
        .run()
        .await
        .is_err()
    {
        Exec::new(git)
            .args([
                "-C",
                repo_str,
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=hyprlayer",
                "-c",
                "user.email=hyprlayer@localhost",
                "commit",
                "--allow-empty",
                "-m",
                "Initial commit",
            ])
            .timeout(Duration::from_secs(30))
            .run()
            .await?;
    }
    Ok(())
}

pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum RepoPathStatus {
    Missing,
    NonGit,
    Bare,
    Unborn,
    Ready,
}

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
    let unborn = matches!(repo.head(), Err(e) if e.code() == git2::ErrorCode::UnbornBranch);
    if unborn {
        RepoPathStatus::Unborn
    } else {
        RepoPathStatus::Ready
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GithubRepo {
    pub name_with_owner: String,
    #[serde(default)]
    pub description: Option<String>,
    pub url: String,
    pub pushed_at: Option<String>,
    pub private: bool,
}

pub async fn github_repos(gh: &Path, limit: u32) -> Result<Vec<GithubRepo>> {
    // `gh repo list` omits organization-member repositories.
    let endpoint = format!("user/repos?per_page={}&sort=pushed", limit.min(100));
    let out = Exec::new(gh)
        .args(["api", &endpoint])
        .timeout(Duration::from_secs(30))
        .run()
        .await?;

    #[derive(Deserialize)]
    struct ApiRepo {
        full_name: String,
        #[serde(default)]
        description: Option<String>,
        html_url: String,
        #[serde(default)]
        pushed_at: Option<String>,
        #[serde(default)]
        private: bool,
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
            pushed_at: r.pushed_at,
            private: r.private,
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
        assert_eq!(name(""), None);
        assert_eq!(name("/"), None);
        assert_eq!(name("   "), None);
    }

    fn sh(repo: &Path, args: &[&str]) {
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
        std::fs::write(repo.join("notes.md"), "hi\n").unwrap();

        let git = which::which("git").unwrap();
        init_repo_with_commit(&git, &repo, "trunk").await.unwrap();

        let r = git2::Repository::open(&repo).unwrap();
        let head = r.head().unwrap();
        assert!(head.is_branch());
        assert_eq!(head.shorthand(), Some("trunk"));
        let commit = head.peel_to_commit().unwrap();
        assert!(commit.parents().next().is_none(), "it's the initial commit");
        assert_eq!(commit.tree().unwrap().len(), 0, "initial commit is empty");

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

        assert_eq!(
            origin_url(&git, &dest).await.as_deref(),
            Some(source.to_str().unwrap())
        );

        let err = clone_repo(&git, source.to_str().unwrap(), &dest)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "io");
    }

    #[test]
    fn classify_missing_and_non_git_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            classify_repo_path(&dir.path().join("nope")),
            RepoPathStatus::Missing
        );
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(classify_repo_path(&plain), RepoPathStatus::NonGit);
    }

    #[tokio::test]
    async fn classify_unborn_becomes_ready_after_initial_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("proj");
        std::fs::create_dir(&repo).unwrap();
        sh(&repo, &["init", "-b", "main"]);
        assert_eq!(classify_repo_path(&repo), RepoPathStatus::Unborn);

        let git = which::which("git").unwrap();
        commit_empty_initial(&git, &repo).await.unwrap();
        assert_eq!(classify_repo_path(&repo), RepoPathStatus::Ready);
    }

    #[test]
    fn classify_bare_repo_is_not_mistaken_for_non_git() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.git");
        std::fs::create_dir(&bare).unwrap();
        sh(&bare, &["init", "--bare", "-b", "main"]);
        assert!(!is_git_repo(&bare), "bare repo has no child .git");
        assert_eq!(classify_repo_path(&bare), RepoPathStatus::Bare);
    }
}
