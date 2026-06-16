//! Project repositories: the app's own canonical clones.
//!
//! A *project* is a real git clone the app owns under `~/hyprlayer/repos`,
//! created either by cloning a hosted repo (a GitHub URL) or by importing a
//! repo from disk — in which case we re-clone it from its `origin` so the
//! app's copy tracks the same source the user already pushes to. Worktrees are
//! always cut from these clones, never from another worktree, so a project
//! never breaks when a worktree is merged or removed.

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
}
