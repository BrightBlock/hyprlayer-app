use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::Exec;

pub fn expand_user_path(raw: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(raw).into_owned())
}

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

pub async fn init_repo(git: &Path, dest: &Path, branch: &str) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let dest_str = dest.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "git init".into(),
        detail: "destination path is not valid UTF-8".into(),
    })?;
    Exec::new(git)
        .args(["init", "-b", branch, dest_str])
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

pub async fn stage_all(git: &Path, repo: &Path) -> Result<()> {
    Exec::new(git)
        .arg("-C")
        .arg(repo)
        .args(["add", "-A"])
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum GitignoreTemplate {
    Node,
    Rust,
    Python,
    Go,
}

impl GitignoreTemplate {
    pub fn body(self) -> &'static str {
        match self {
            GitignoreTemplate::Node => {
                "node_modules/\ndist/\nbuild/\ncoverage/\n.env\n.env.local\nnpm-debug.log*\nyarn-error.log*\n.DS_Store\n"
            }
            GitignoreTemplate::Rust => "target/\nCargo.lock.orig\n**/*.rs.bk\n.env\n.DS_Store\n",
            GitignoreTemplate::Python => {
                "__pycache__/\n*.py[cod]\n.venv/\nvenv/\n.pytest_cache/\n.mypy_cache/\ndist/\nbuild/\n*.egg-info/\n.env\n.DS_Store\n"
            }
            GitignoreTemplate::Go => "bin/\nvendor/\n*.exe\n*.test\n*.out\n.env\n.DS_Store\n",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Scaffold {
    pub readme: bool,
    pub gitignore: Option<GitignoreTemplate>,
}

pub fn write_scaffold(dir: &Path, project: &str, scaffold: &Scaffold) -> Result<Vec<String>> {
    let mut written = Vec::new();
    let mut put = |name: &str, body: String| -> Result<()> {
        let path = dir.join(name);
        if path.exists() {
            return Ok(());
        }
        std::fs::write(&path, body)?;
        written.push(name.to_string());
        Ok(())
    };
    if scaffold.readme {
        put("README.md", format!("# {project}\n"))?;
    }
    if let Some(t) = scaffold.gitignore {
        put(".gitignore", t.body().to_string())?;
    }
    Ok(written)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum GithubOwnerKind {
    User,
    Organization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GithubOwner {
    pub login: String,
    pub kind: GithubOwnerKind,
}

#[derive(Deserialize)]
struct ApiLogin {
    login: String,
}

async fn gh_json<T: serde::de::DeserializeOwned>(gh: &Path, endpoint: &str) -> Result<T> {
    let out = Exec::new(gh)
        .args(["api", endpoint])
        .timeout(Duration::from_secs(30))
        .run()
        .await?;
    serde_json::from_str(&out.stdout).map_err(|e| CoreError::OutputParse {
        program: format!("gh api {endpoint}"),
        detail: e.to_string(),
    })
}

// `gh` reports "you are not signed in" and "GitHub is unreachable" through the
// same non-zero exit, so the two have to be told apart by wording. Anything
// unmatched is treated as reachable-but-broken, because claiming the user is
// signed out is the answer that silently drops the GitHub half of a create.
pub fn is_github_auth_failure(stderr: &str) -> bool {
    let low = stderr.to_lowercase();
    low.contains("gh auth login")
        || low.contains("not logged in")
        || low.contains("no oauth token")
        || low.contains("authentication required")
        || low.contains("bad credentials")
        || low.contains("http 401")
}

pub async fn github_owners(gh: &Path) -> Result<Vec<GithubOwner>> {
    let me: ApiLogin = gh_json(gh, "user").await?;
    let mut owners = vec![GithubOwner {
        login: me.login,
        kind: GithubOwnerKind::User,
    }];
    let orgs: Vec<ApiLogin> = gh_json(gh, "user/orgs?per_page=100")
        .await
        .unwrap_or_default();
    owners.extend(orgs.into_iter().map(|o| GithubOwner {
        login: o.login,
        kind: GithubOwnerKind::Organization,
    }));
    Ok(owners)
}

// GitHub silently rewrites anything outside [A-Za-z0-9._-] to a dash rather
// than rejecting it, so a name accepted verbatim here would create a remote
// whose name no longer matches the folder on disk.
pub fn github_repo_name(name: &str) -> String {
    let mapped: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut out = String::with_capacity(mapped.len());
    let mut last_dash = false;
    for c in mapped.chars() {
        if c == '-' {
            if !last_dash && !out.is_empty() {
                out.push(c);
            }
            last_dash = true;
        } else {
            out.push(c);
            last_dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn is_reserved_repo_name(name: &str) -> bool {
    matches!(name, "" | "." | ".." | ".git")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubFailure {
    pub code: &'static str,
    pub message: String,
    pub remedy: Option<String>,
}

// Wording verified against gh 2.98.0 / the GitHub GraphQL API on 2026-08-29;
// an unrecognised failure keeps gh's own words rather than guessing.
pub fn classify_github_failure(stderr: &str, owner: &str, name: &str) -> GithubFailure {
    let raw = stderr.trim();
    let low = raw.to_lowercase();
    let fail = |code, message: String, remedy: Option<&str>| GithubFailure {
        code,
        message,
        remedy: remedy.map(str::to_string),
    };

    if low.contains("name already exists") {
        return fail(
            "github_name_taken",
            format!("`{owner}/{name}` already exists on GitHub."),
            Some("Choose a different project name, or add the existing repository as a project instead."),
        );
    }
    if low.contains("name is reserved") || low.contains("name is invalid") {
        return fail(
            "github_name_invalid",
            format!("GitHub won't accept `{name}` as a repository name."),
            Some("Use letters, digits, dots, dashes or underscores."),
        );
    }
    if low.contains("saml") || low.contains("oauth app access restrictions") {
        return fail(
            "github_sso",
            format!("Your GitHub token isn't authorized for the `{owner}` organization."),
            Some("Run `gh auth login` again and authorize the organization, or approve the token in the org's SSO settings."),
        );
    }
    if low.contains("does not have the correct permissions")
        || low.contains("resource not accessible")
        || low.contains("must have admin rights")
        || low.contains("permission")
    {
        return fail(
            "github_forbidden",
            format!("Your account can't create repositories in `{owner}`."),
            Some("Pick a different owner, or ask an organization admin for repository-creation rights."),
        );
    }
    if low.contains("error connecting")
        || low.contains("check your internet connection")
        || low.contains("dial tcp")
        || low.contains("no such host")
        || low.contains("timeout")
    {
        return fail(
            "github_unreachable",
            "Couldn't reach GitHub.".into(),
            Some("Check your internet connection, then try again. The project is already created locally."),
        );
    }
    if low.contains("gh auth login") || low.contains("authentication") || low.contains("not logged")
    {
        return fail(
            "github_auth",
            "GitHub CLI isn't signed in.".into(),
            Some("Run `gh auth login` in a terminal, then try again."),
        );
    }
    fail(
        "github_failed",
        if raw.is_empty() {
            format!("Couldn't create `{owner}/{name}` on GitHub.")
        } else {
            raw.to_string()
        },
        None,
    )
}

pub fn remote_points_at(remote_url: &str, owner: &str, name: &str) -> bool {
    crate::gitops::remote_to_web_url(remote_url)
        .is_some_and(|web| web.eq_ignore_ascii_case(&format!("https://github.com/{owner}/{name}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishPlan {
    Create,
    PushOnly { url: String },
    Conflict { origin: String },
}

// A retry after a failed push must push, not create: the repository already
// exists and `gh repo create` would only report the name as taken.
pub fn plan_publish(origin: Option<&str>, owner: &str, name: &str) -> PublishPlan {
    match origin {
        None => PublishPlan::Create,
        Some(url) if remote_points_at(url, owner, name) => PublishPlan::PushOnly {
            url: crate::gitops::remote_to_web_url(url)
                .unwrap_or_else(|| format!("https://github.com/{owner}/{name}")),
        },
        Some(url) => PublishPlan::Conflict {
            origin: url.to_string(),
        },
    }
}

pub fn classify_push_failure(stderr: &str) -> GithubFailure {
    let raw = stderr.trim();
    let low = raw.to_lowercase();
    if low.contains("authentication failed")
        || low.contains("could not read username")
        || low.contains("permission denied")
        || low.contains("403")
    {
        return GithubFailure {
            code: "push_auth",
            message: "Git couldn't authenticate with GitHub.".into(),
            remedy: Some(
                "Run `gh auth setup-git` (or check your SSH agent), then try again.".into(),
            ),
        };
    }
    if low.contains("could not resolve host") || low.contains("failed to connect") {
        return GithubFailure {
            code: "push_unreachable",
            message: "Couldn't reach GitHub to push.".into(),
            remedy: Some("Check your internet connection, then try again.".into()),
        };
    }
    // git 2.x no longer prints "non-fast-forward" in the common case; the
    // wording below is what it actually emits (verified against git 2.x).
    if low.contains("non-fast-forward")
        || low.contains("fetch first")
        || low.contains("updates were rejected")
        || low.contains("failed to push some refs")
    {
        return GithubFailure {
            code: "push_rejected",
            message: "The GitHub repository already has commits of its own.".into(),
            remedy: Some("Pull or rebase onto it in a terminal, or point this project at an empty repository.".into()),
        };
    }
    GithubFailure {
        code: "push_failed",
        message: if raw.is_empty() {
            "The push to GitHub failed.".into()
        } else {
            raw.to_string()
        },
        remedy: None,
    }
}

pub async fn create_github_repo(
    gh: &Path,
    repo: &Path,
    owner: &str,
    name: &str,
    private: bool,
) -> Result<String> {
    let repo_str = repo.to_str().ok_or_else(|| CoreError::OutputParse {
        program: "gh repo create".into(),
        detail: "repository path is not valid UTF-8".into(),
    })?;
    let slug = format!("{owner}/{name}");
    let out = Exec::new(gh)
        .args([
            "repo",
            "create",
            &slug,
            if private { "--private" } else { "--public" },
            "--source",
            repo_str,
            "--remote",
            "origin",
        ])
        .timeout(Duration::from_secs(300))
        .run()
        .await?;
    let url = out
        .stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("http"))
        .map(String::from)
        .unwrap_or_else(|| format!("https://github.com/{slug}"));
    Ok(url)
}

pub async fn push_upstream(git: &Path, repo: &Path, branch: &str) -> Result<()> {
    Exec::new(git)
        .arg("-C")
        .arg(repo)
        .args(["push", "-u", "origin", branch])
        .timeout(Duration::from_secs(300))
        .run()
        .await?;
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
    fn scaffold_writes_only_what_was_asked_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();

        let none = write_scaffold(repo, "aurora", &Scaffold::default()).unwrap();
        assert!(none.is_empty());
        assert!(!repo.join("README.md").exists());

        let both = write_scaffold(
            repo,
            "aurora",
            &Scaffold {
                readme: true,
                gitignore: Some(GitignoreTemplate::Rust),
            },
        )
        .unwrap();
        assert_eq!(both, ["README.md", ".gitignore"]);
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "# aurora\n"
        );
        assert!(std::fs::read_to_string(repo.join(".gitignore"))
            .unwrap()
            .contains("target/"));

        std::fs::write(repo.join("README.md"), "mine\n").unwrap();
        let again = write_scaffold(
            repo,
            "aurora",
            &Scaffold {
                readme: true,
                gitignore: Some(GitignoreTemplate::Node),
            },
        )
        .unwrap();
        assert!(again.is_empty(), "existing files are left alone");
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "mine\n"
        );
    }

    #[tokio::test]
    async fn init_stage_and_commit_puts_the_scaffold_in_the_first_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("nested").join("aurora");
        let git = which::which("git").unwrap();

        init_repo(&git, &repo, "trunk").await.unwrap();
        let written = write_scaffold(
            &repo,
            "aurora",
            &Scaffold {
                readme: true,
                gitignore: Some(GitignoreTemplate::Go),
            },
        )
        .unwrap();
        assert_eq!(written.len(), 2);
        stage_all(&git, &repo).await.unwrap();
        commit_empty_initial(&git, &repo).await.unwrap();

        assert_eq!(classify_repo_path(&repo), RepoPathStatus::Ready);
        let r = git2::Repository::open(&repo).unwrap();
        assert_eq!(r.head().unwrap().shorthand(), Some("trunk"));
        let tree = r.head().unwrap().peel_to_commit().unwrap().tree().unwrap();
        assert!(tree.get_name("README.md").is_some());
        assert!(tree.get_name(".gitignore").is_some());
        assert!(
            r.statuses(None).unwrap().is_empty(),
            "nothing left uncommitted"
        );
    }

    #[test]
    fn github_names_follow_githubs_own_rewriting() {
        assert_eq!(github_repo_name("aurora-api"), "aurora-api");
        assert_eq!(
            github_repo_name("hyprlayer smoke space test"),
            "hyprlayer-smoke-space-test"
        );
        assert_eq!(github_repo_name("My Project!!"), "My-Project");
        assert_eq!(github_repo_name("a  b"), "a-b");
        assert_eq!(github_repo_name("  padded  "), "padded");
        assert_eq!(github_repo_name("dots.and_dashes-1"), "dots.and_dashes-1");
        assert_eq!(github_repo_name("!!!"), "");
        assert!(is_reserved_repo_name(&github_repo_name("!!!")));
        assert!(is_reserved_repo_name(".."));
        assert!(!is_reserved_repo_name("aurora-api"));
    }

    #[test]
    fn real_gh_failures_become_actionable_messages() {
        let c = |s: &str| classify_github_failure(s, "jtsiros", "aurora-api");

        // Captured verbatim from gh 2.98.0.
        assert_eq!(
            c("GraphQL: Name already exists on this account (createRepository)").code,
            "github_name_taken"
        );
        assert_eq!(
            c("GraphQL: Name is reserved (createRepository)").code,
            "github_name_invalid"
        );
        assert_eq!(
            c("error connecting to github.invalid\ncheck your internet connection or https://githubstatus.com")
                .code,
            "github_unreachable"
        );

        assert_eq!(
            c("GraphQL: jtsiros does not have the correct permissions to execute `createRepository`")
                .code,
            "github_forbidden"
        );
        assert_eq!(
            c("the acme organization has enabled OAuth App access restrictions").code,
            "github_sso"
        );
        assert_eq!(
            c("To get started with GitHub CLI, please run: gh auth login").code,
            "github_auth"
        );

        let taken = c("GraphQL: Name already exists on this account (createRepository)");
        assert!(taken.message.contains("jtsiros/aurora-api"));
        assert!(taken.remedy.is_some());

        let unknown = c("something nobody has seen before");
        assert_eq!(unknown.code, "github_failed");
        assert_eq!(
            unknown.message, "something nobody has seen before",
            "an unrecognised failure keeps gh's own words"
        );
        assert!(unknown.remedy.is_none());
    }

    #[test]
    fn an_existing_origin_is_recognised_across_url_shapes() {
        assert!(remote_points_at(
            "https://github.com/jtsiros/aurora-api.git",
            "jtsiros",
            "aurora-api"
        ));
        assert!(remote_points_at(
            "git@github.com:jtsiros/aurora-api.git",
            "jtsiros",
            "aurora-api"
        ));
        assert!(remote_points_at(
            "https://github.com/JTsiros/Aurora-API",
            "jtsiros",
            "aurora-api"
        ));
        assert!(!remote_points_at(
            "https://github.com/someone-else/aurora-api.git",
            "jtsiros",
            "aurora-api"
        ));
        assert!(!remote_points_at("/local/path", "jtsiros", "aurora-api"));
    }

    #[test]
    fn a_retry_pushes_instead_of_creating_a_second_time() {
        let plan = |origin: Option<&str>| plan_publish(origin, "jtsiros", "aurora-api");

        assert_eq!(plan(None), PublishPlan::Create);
        assert_eq!(
            plan(Some("git@github.com:jtsiros/aurora-api.git")),
            PublishPlan::PushOnly {
                url: "https://github.com/jtsiros/aurora-api".into()
            },
            "the repo gh already created is pushed to, not re-created"
        );
        assert_eq!(
            plan(Some("https://github.com/someone/else.git")),
            PublishPlan::Conflict {
                origin: "https://github.com/someone/else.git".into()
            }
        );
    }

    #[test]
    fn push_failures_are_classified_by_git_stderr() {
        assert_eq!(
            classify_push_failure("fatal: Authentication failed for 'https://github.com/x/y.git/'")
                .code,
            "push_auth"
        );
        assert_eq!(
            classify_push_failure("fatal: could not resolve host: github.com").code,
            "push_unreachable"
        );
        assert_eq!(
            classify_push_failure("! [rejected] main -> main (non-fast-forward)").code,
            "push_rejected"
        );
        // Captured verbatim from git 2.x, which never says "non-fast-forward" here.
        assert_eq!(
            classify_push_failure(
                "error: failed to push some refs to '../remote.git'\n\
                 hint: Updates were rejected because the remote contains work that you do not\n\
                 hint: have locally."
            )
            .code,
            "push_rejected"
        );
        assert_eq!(
            classify_push_failure(
                "fatal: unable to access 'https://github.invalid/x/y.git/': Could not resolve host: github.invalid"
            )
            .code,
            "push_unreachable"
        );
        assert_eq!(
            classify_push_failure(
                "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
            )
            .code,
            "push_auth"
        );
        let unknown = classify_push_failure("something else entirely");
        assert_eq!(unknown.code, "push_failed");
        assert_eq!(unknown.message, "something else entirely");
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
