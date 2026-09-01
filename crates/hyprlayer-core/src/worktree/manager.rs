use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::process::Exec;

use super::thoughts_links::LinkStatus;

pub const ARCHIVE_SUBDIR: &str = "archived-contexts";

pub fn archive_dir_for(project_repo: &Path, slug: &str) -> PathBuf {
    project_repo.join(ARCHIVE_SUBDIR).join(slug)
}

pub fn ensure_archive_excluded(project_repo: &Path) -> std::io::Result<()> {
    let exclude = project_repo.join(".git").join("info").join("exclude");
    let Some(parent) = exclude.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let needle = format!("/{}/", ARCHIVE_SUBDIR);
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == needle) {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&format!(
        "# hyprlayer: hide archived worktrees from git status\n{needle}\n"
    ));
    std::fs::write(&exclude, next)
}

pub struct CreateSpec {
    pub project_repo: PathBuf,
    pub mapped_name: String,
    pub title: String,
    pub base_override: Option<String>,
    pub branch_prefix: String,
    pub mode: CreateMode,
}

#[derive(Debug, Clone)]
pub enum CreateMode {
    Isolated,
    Existing { branch: String },
    InPlace { branch: String },
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CreatedWorktree {
    pub path: String,
    pub branch: String,
    pub slug: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    pub exists: bool,
    pub branch: Option<String>,
    pub dirty: u32,
    pub additions: u32,
    pub deletions: u32,
    pub untracked: u32,
    pub ahead: u32,
    pub behind: u32,
    pub ahead_of_base: u32,
    pub thoughts: LinkStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovePolicy {
    Safe,
    ForceDiscard,
}

pub struct WorktreeManager {
    root: PathBuf,
}

impl WorktreeManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn worktree_dir(&self, mapped_name: &str, slug: &str) -> PathBuf {
        self.root.join(mapped_name).join(slug)
    }

    pub async fn create(&self, git: &Path, spec: &CreateSpec) -> Result<CreatedWorktree> {
        let slug = unique_slug(&spec.title);

        let _ = git_in(git, &spec.project_repo, &["fetch", "--prune", "origin"])
            .timeout(Duration::from_secs(30))
            .run()
            .await;

        match &spec.mode {
            CreateMode::Isolated => {
                let branch = format!("{}{slug}", spec.branch_prefix);
                let path = self.prepare_worktree_dir(&spec.mapped_name, &slug)?;
                let base = match &spec.base_override {
                    Some(b) => b.clone(),
                    None => detect_base_branch(git, &spec.project_repo).await?,
                };
                self.worktree_add(git, &spec.project_repo, &path, &["-b", &branch], &base)
                    .await?;
                self.windows_longpaths(git, &path).await;
                Ok(CreatedWorktree {
                    path: path.display().to_string(),
                    branch,
                    slug,
                    base_branch: base,
                })
            }
            CreateMode::Existing { branch } => {
                let path = self.prepare_worktree_dir(&spec.mapped_name, &slug)?;
                let base = detect_base_branch(git, &spec.project_repo)
                    .await
                    .unwrap_or_else(|_| branch.clone());
                self.worktree_add(git, &spec.project_repo, &path, &[], branch)
                    .await?;
                self.windows_longpaths(git, &path).await;
                Ok(CreatedWorktree {
                    path: path.display().to_string(),
                    branch: branch.clone(),
                    slug,
                    base_branch: base,
                })
            }
            CreateMode::InPlace { branch } => {
                let already_here = crate::gitops::current_branch(&spec.project_repo).as_deref()
                    == Some(branch.as_str());
                if !already_here {
                    let dirty = status(&spec.project_repo, branch, None).dirty;
                    if dirty > 0 {
                        return Err(CoreError::CheckoutDirty { count: dirty });
                    }
                    git_in(git, &spec.project_repo, &["checkout", branch])
                        .timeout(Duration::from_secs(60))
                        .run()
                        .await?;
                }
                let base = detect_base_branch(git, &spec.project_repo)
                    .await
                    .unwrap_or_else(|_| branch.clone());
                Ok(CreatedWorktree {
                    path: spec.project_repo.display().to_string(),
                    branch: branch.clone(),
                    slug,
                    base_branch: base,
                })
            }
        }
    }

    fn prepare_worktree_dir(&self, mapped_name: &str, slug: &str) -> Result<PathBuf> {
        let path = self.worktree_dir(mapped_name, slug);
        if path.exists() {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("worktree dir already exists: {}", path.display()),
            )));
        }
        std::fs::create_dir_all(path.parent().expect("worktree dir has parent"))?;
        Ok(path)
    }

    async fn worktree_add(
        &self,
        git: &Path,
        project_repo: &Path,
        path: &Path,
        extra: &[&str],
        committish: &str,
    ) -> Result<()> {
        let path_str = path.to_str().ok_or_else(|| CoreError::OutputParse {
            program: "worktree add".into(),
            detail: "worktree path is not valid UTF-8".into(),
        })?;
        let mut args = vec!["worktree", "add"];
        args.extend_from_slice(extra);
        args.push(path_str);
        args.push(committish);
        git_in(git, project_repo, &args)
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(())
    }

    async fn windows_longpaths(&self, _git: &Path, _worktree: &Path) {
        #[cfg(windows)]
        {
            let _ = git_in(_git, _worktree, &["config", "core.longpaths", "true"])
                .run()
                .await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn remove(
        &self,
        git: &Path,
        project_repo: &Path,
        worktree: &Path,
        branch: &str,
        base_branch: &str,
        policy: RemovePolicy,
        delete_branch: bool,
    ) -> Result<()> {
        if is_live_worktree(worktree) {
            if policy == RemovePolicy::Safe {
                ensure_removable(worktree, base_branch, branch)?;
            }
            let wt = worktree.display().to_string();
            git_in(git, project_repo, &["worktree", "remove", "--force", &wt])
                .timeout(Duration::from_secs(60))
                .run()
                .await?;
        } else {
            let _ = git_in(git, project_repo, &["worktree", "prune"])
                .run()
                .await;
            if worktree.exists() {
                let _ = std::fs::remove_dir_all(worktree);
            }
        }

        if delete_branch {
            let delete_flag = if policy == RemovePolicy::ForceDiscard {
                "-D"
            } else {
                "-d"
            };
            let _ = git_in(git, project_repo, &["branch", delete_flag, branch])
                .run()
                .await;
        }

        Ok(())
    }

    pub async fn prune(&self, git: &Path, project_repo: &Path) -> Result<()> {
        git_in(git, project_repo, &["worktree", "prune"])
            .run()
            .await?;
        Ok(())
    }

    pub async fn archive(
        &self,
        git: &Path,
        project_repo: &Path,
        worktree: &Path,
        dest: &Path,
    ) -> Result<bool> {
        if !is_live_worktree(worktree) {
            let _ = git_in(git, project_repo, &["worktree", "prune"])
                .run()
                .await;
            if worktree.exists() {
                let _ = std::fs::remove_dir_all(worktree);
            }
            return Ok(false);
        }

        if dest.exists() {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("archive destination already exists: {}", dest.display()),
            )));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let from = worktree.display().to_string();
        let to = dest.display().to_string();
        git_in(git, project_repo, &["worktree", "move", &from, &to])
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(true)
    }

    pub async fn restore(
        &self,
        git: &Path,
        project_repo: &Path,
        archive: &Path,
        target: &Path,
    ) -> Result<()> {
        if target.exists() {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("restore target already exists: {}", target.display()),
            )));
        }
        if !is_live_worktree(archive) {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("archived worktree missing: {}", archive.display()),
            )));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let from = archive.display().to_string();
        let to = target.display().to_string();
        git_in(git, project_repo, &["worktree", "move", &from, &to])
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(())
    }

    pub async fn clean_ignored(&self, git: &Path, worktree: &Path, keep: &[String]) -> Result<()> {
        let mut args: Vec<String> = vec!["clean".into(), "-d".into(), "-f".into(), "-X".into()];
        for pat in keep {
            args.push("-e".into());
            args.push(format!("!{pat}"));
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        git_in(git, worktree, &arg_refs)
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(())
    }

    pub async fn purge_archive(
        &self,
        git: &Path,
        project_repo: &Path,
        archive: &Path,
        branch: &str,
        delete_branch: bool,
    ) -> Result<()> {
        if archive.exists() {
            let _ = std::fs::remove_dir_all(archive);
        }
        let _ = git_in(git, project_repo, &["worktree", "prune"])
            .run()
            .await;
        if delete_branch {
            let _ = git_in(git, project_repo, &["branch", "-D", branch])
                .run()
                .await;
            let _ = delete_remote_branch(git, project_repo, branch).await;
        }
        Ok(())
    }
}

pub fn is_live_worktree(path: &Path) -> bool {
    path.join(".git").exists()
}

pub fn ensure_removable(worktree: &Path, base_branch: &str, branch: &str) -> Result<()> {
    let status = status(worktree, base_branch, None);
    if status.dirty + status.untracked > 0 {
        return Err(CoreError::WorktreeDirty(status.dirty + status.untracked));
    }
    if status.ahead > 0 {
        return Err(CoreError::WorktreeUnpushed {
            branch: branch.to_string(),
            count: status.ahead,
        });
    }
    Ok(())
}

fn git_in(git: &Path, repo: &Path, args: &[&str]) -> Exec {
    let repo_arg: OsString = repo.as_os_str().to_os_string();
    Exec::new(git)
        .arg("-C")
        .arg(repo_arg)
        .args(args.iter().copied())
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PullOutcome {
    UpToDate,
    Merged,
    Conflicts { files: Vec<String> },
}

pub async fn pull_base(git: &Path, worktree: &Path, base: &str) -> Result<PullOutcome> {
    let branch = base.strip_prefix("origin/").unwrap_or(base);

    let _ = git_in(git, worktree, &["fetch", "origin", branch])
        .timeout(Duration::from_secs(60))
        .run()
        .await;

    let remote_ref = format!("origin/{branch}");
    let merge_ref = if git_in(
        git,
        worktree,
        &["rev-parse", "--verify", "--quiet", &remote_ref],
    )
    .run()
    .await
    .is_ok()
    {
        remote_ref
    } else {
        base.to_string()
    };

    let out = git_in(git, worktree, &["merge", "--no-edit", &merge_ref])
        .timeout(Duration::from_secs(120))
        .run_unchecked()
        .await?;

    if out.exit_code == Some(0) {
        return Ok(if out.stdout.contains("Already up to date") {
            PullOutcome::UpToDate
        } else {
            PullOutcome::Merged
        });
    }

    let files = unmerged_files(git, worktree).await;
    if !files.is_empty() {
        return Ok(PullOutcome::Conflicts { files });
    }
    Err(CoreError::Exec {
        program: "git merge".into(),
        exit: out.exit_code,
        stderr_tail: out.stderr.trim().to_string(),
    })
}

pub async fn fetch_remote(git: &Path, worktree: &Path) -> Result<()> {
    git_in(git, worktree, &["fetch", "--prune", "origin"])
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

pub async fn create_tracking_branch(
    git: &Path,
    repo: &Path,
    local: &str,
    remote_ref: &str,
) -> Result<()> {
    git_in(git, repo, &["branch", "--track", local, remote_ref])
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

pub async fn delete_remote_branch(git: &Path, repo: &Path, branch: &str) -> Result<()> {
    git_in(git, repo, &["push", "origin", "--delete", branch])
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

async fn unmerged_files(git: &Path, worktree: &Path) -> Vec<String> {
    match git_in(git, worktree, &["diff", "--name-only", "--diff-filter=U"])
        .run()
        .await
    {
        Ok(out) => out
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub async fn detect_base_branch(git: &Path, repo: &Path) -> Result<String> {
    if let Ok(out) = git_in(git, repo, &["symbolic-ref", "refs/remotes/origin/HEAD"])
        .run()
        .await
    {
        let full = out.stdout.trim();
        if let Some(short) = full.strip_prefix("refs/remotes/") {
            if !short.is_empty() {
                return Ok(short.to_string());
            }
        }
    }
    for candidate in ["origin/main", "origin/master", "main", "master"] {
        let probe = git_in(
            git,
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{candidate}^{{commit}}"),
            ],
        )
        .run_unchecked()
        .await?;
        if probe.exit_code == Some(0) {
            return Ok(candidate.to_string());
        }
    }
    Err(CoreError::OutputParse {
        program: "base detection".into(),
        detail: format!(
            "no main/master branch found in {}, pass a base branch explicitly",
            repo.display()
        ),
    })
}

pub fn folder_status(dir: &Path) -> WorktreeStatus {
    WorktreeStatus {
        exists: dir.is_dir(),
        branch: None,
        dirty: 0,
        additions: 0,
        deletions: 0,
        untracked: 0,
        ahead: 0,
        behind: 0,
        ahead_of_base: 0,
        thoughts: LinkStatus::Unsupported,
    }
}

pub fn status(worktree: &Path, base_branch: &str, thoughts: Option<LinkStatus>) -> WorktreeStatus {
    let missing = WorktreeStatus {
        exists: false,
        branch: None,
        dirty: 0,
        additions: 0,
        deletions: 0,
        untracked: 0,
        ahead: 0,
        behind: 0,
        ahead_of_base: 0,
        thoughts: thoughts.unwrap_or(LinkStatus::Broken),
    };
    if !worktree.exists() {
        return missing;
    }
    let Ok(repo) = crate::gitops::open_repo(worktree) else {
        return missing;
    };

    let (mut dirty, mut untracked) = (0u32, 0u32);
    if let Ok(statuses) = repo.statuses(Some(&mut crate::gitops::status_options())) {
        for entry in statuses.iter() {
            let path = entry.path().unwrap_or_default();
            if super::is_thoughts_path(path) {
                continue;
            }
            if entry.status().intersects(git2::Status::WT_NEW) {
                untracked += 1;
            } else {
                dirty += 1;
            }
        }
    }

    let (additions, deletions) = crate::gitops::worktree_diff_line_counts(&repo);

    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .filter(|h| h.is_branch())
        .and_then(|h| h.shorthand())
        .map(String::from);

    let (ahead, behind) = (|| -> Option<(u32, u32)> {
        let head_ref = head.as_ref()?;
        let local = head_ref.target()?;
        let target = match crate::gitops::upstream_oid(&repo, head_ref) {
            Some(oid) => oid,
            None => repo
                .revparse_single(base_branch)
                .ok()?
                .peel_to_commit()
                .ok()?
                .id(),
        };
        let (a, b) = repo.graph_ahead_behind(local, target).ok()?;
        Some((a as u32, b as u32))
    })()
    .unwrap_or((0, 0));

    let ahead_of_base = (|| -> Option<u32> {
        let head_ref = head.as_ref()?;
        let local = head_ref.target()?;
        let base = repo
            .revparse_single(base_branch)
            .ok()?
            .peel_to_commit()
            .ok()?
            .id();
        let (a, _) = repo.graph_ahead_behind(local, base).ok()?;
        Some(a as u32)
    })()
    .unwrap_or(0);

    WorktreeStatus {
        exists: true,
        branch,
        dirty,
        additions,
        deletions,
        untracked,
        ahead,
        behind,
        ahead_of_base,
        thoughts: thoughts.unwrap_or(LinkStatus::Ok),
    }
}

pub fn unique_slug(title: &str) -> String {
    let mut kebab = String::new();
    let mut last_dash = true;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            kebab.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            kebab.push('-');
            last_dash = true;
        }
        if kebab.len() >= 48 {
            break;
        }
    }
    let kebab = kebab.trim_matches('-');
    let stem = if kebab.is_empty() { "work-item" } else { kebab };
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..4];
    format!("{stem}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_status_reports_a_live_directory_git_status_would_call_missing() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir(&notes).unwrap();
        std::fs::write(notes.join("todo.md"), "hi\n").unwrap();

        assert!(!status(&notes, "", None).exists);

        let st = folder_status(&notes);
        assert!(st.exists);
        assert_eq!(st.branch, None);
        assert_eq!((st.dirty, st.untracked, st.ahead, st.behind), (0, 0, 0, 0));
        assert_eq!(st.thoughts, LinkStatus::Unsupported);

        assert!(!folder_status(&dir.path().join("gone")).exists);
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

    fn fixture_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("proj");
        std::fs::create_dir(&repo).unwrap();
        sh(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-m", "init"]);
        (dir, repo)
    }

    #[tokio::test]
    async fn remote_only_branch_is_adopted_via_a_local_tracking_branch() {
        let (_guard, origin) = fixture_repo();
        sh(&origin, &["branch", "linux-support"]);
        let clones = tempfile::tempdir().unwrap();
        let repo = clones.path().join("proj");
        sh(
            clones.path(),
            &["clone", origin.to_str().unwrap(), repo.to_str().unwrap()],
        );
        let git = which::which("git").expect("git on PATH");

        assert!(!crate::gitops::list_local_branches(&repo)
            .unwrap()
            .contains(&"linux-support".to_string()));

        create_tracking_branch(&git, &repo, "linux-support", "origin/linux-support")
            .await
            .unwrap();

        let upstream = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "--abbrev-ref", "linux-support@{upstream}"])
            .output()
            .expect("git runs");
        assert_eq!(
            String::from_utf8_lossy(&upstream.stdout).trim(),
            "origin/linux-support"
        );

        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Linux support".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Existing {
                        branch: "linux-support".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(created.branch, "linux-support");
        assert_eq!(
            status(Path::new(&created.path), "main", None)
                .branch
                .as_deref(),
            Some("linux-support")
        );

        assert!(
            create_tracking_branch(&git, &repo, "linux-support", "origin/linux-support")
                .await
                .is_err()
        );
    }

    #[test]
    fn slugs_are_kebab_and_unique() {
        let a = unique_slug("Add OAuth2 login!");
        assert!(a.starts_with("add-oauth2-login-"), "{a}");
        assert_ne!(unique_slug("x"), unique_slug("x"));
        assert!(unique_slug("!!!").starts_with("work-item-"));

        let long = unique_slug(&"very long words ".repeat(20));
        assert!(long.len() <= 48 + 5, "{long}");
    }

    #[tokio::test]
    async fn create_status_remove_lifecycle() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").expect("git on PATH");

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Auth work".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);
        assert!(wt.join("README.md").exists());
        assert_eq!(created.base_branch, "main");
        assert!(created.branch.starts_with("hyprlayer/auth-work-"));

        let st = status(&wt, &created.base_branch, None);
        assert!(st.exists);
        assert_eq!((st.dirty, st.untracked, st.ahead), (0, 0, 0));

        std::fs::write(wt.join("scratch.txt"), "wip\n").unwrap();
        let err = manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::Safe,
                true,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "worktree_dirty");
        std::fs::remove_file(wt.join("scratch.txt")).unwrap();

        std::fs::write(wt.join("README.md"), "changed\n").unwrap();
        let st = status(&wt, &created.base_branch, None);
        assert_eq!(st.dirty, 1);
        let err = manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::Safe,
                true,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "worktree_dirty");

        std::fs::create_dir_all(wt.join("thoughts/sub")).unwrap();
        std::fs::write(wt.join("thoughts/sub/x.md"), "ignored").unwrap();
        sh(&wt, &["add", "README.md"]);
        sh(&wt, &["commit", "-m", "wip"]);
        let st = status(&wt, &created.base_branch, None);
        assert_eq!((st.dirty, st.ahead, st.ahead_of_base), (0, 1, 1));
        assert_eq!(st.untracked, 0, "thoughts/ must not count");
        let err = manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::Safe,
                true,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "worktree_unpushed");

        manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::ForceDiscard,
                true,
            )
            .await
            .unwrap();
        assert!(!wt.exists());
        let branches = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["branch", "--list", &created.branch])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());
    }

    #[tokio::test]
    async fn isolated_branch_uses_the_spec_prefix_verbatim() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").expect("git on PATH");

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Add login".into(),
                    base_override: None,
                    branch_prefix: "hotfix-".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();

        assert!(
            created.branch.starts_with("hotfix-add-login-"),
            "{}",
            created.branch
        );
        assert_eq!(
            status(Path::new(&created.path), "main", None)
                .branch
                .as_deref(),
            Some(created.branch.as_str())
        );
    }

    #[test]
    fn ahead_of_base_survives_push_while_ahead_zeroes() {
        let (guard, repo) = fixture_repo();

        sh(&repo, &["checkout", "-b", "feature/x"]);
        std::fs::write(repo.join("feature.txt"), "work\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-m", "feature work"]);

        let before = status(&repo, "main", None);
        assert_eq!((before.ahead, before.ahead_of_base), (1, 1));

        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "feature/x"]);

        let after = status(&repo, "main", None);
        assert_eq!(after.ahead, 0, "pushed: nothing left to push vs upstream");
        assert_eq!(
            after.ahead_of_base, 1,
            "pushed: still a commit to ship vs base, Ship must stay visible",
        );
    }

    #[tokio::test]
    async fn create_existing_checks_out_user_branch_without_creating_one() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        sh(&repo, &["branch", "feature-x"]);

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Adopt feature".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Existing {
                        branch: "feature-x".into(),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(created.branch, "feature-x");
        let wt = PathBuf::from(&created.path);
        assert!(wt.join("README.md").exists());
        let st = status(&wt, &created.base_branch, None);
        assert_eq!(st.branch.as_deref(), Some("feature-x"));
    }

    #[tokio::test]
    async fn create_inplace_switches_main_checkout_and_makes_no_worktree() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        sh(&repo, &["checkout", "-b", "side"]);
        assert_eq!(
            crate::gitops::current_branch(&repo).as_deref(),
            Some("side")
        );

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Work on main".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::InPlace {
                        branch: "main".into(),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(created.branch, "main");
        assert_eq!(PathBuf::from(&created.path), repo);
        assert!(
            std::fs::read_dir(wt_root.path()).unwrap().next().is_none(),
            "in-place must not create a worktree under the root"
        );
        assert_eq!(
            crate::gitops::current_branch(&repo).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn status_branch_follows_head_and_reports_none_when_detached() {
        let (_guard, repo) = fixture_repo();
        assert_eq!(status(&repo, "main", None).branch.as_deref(), Some("main"));

        sh(&repo, &["checkout", "-b", "feature/x"]);
        assert_eq!(
            status(&repo, "main", None).branch.as_deref(),
            Some("feature/x")
        );

        sh(&repo, &["checkout", "--detach"]);
        assert_eq!(status(&repo, "main", None).branch, None);
    }

    #[tokio::test]
    async fn create_inplace_refuses_dirty_tree_when_switching_branches() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        sh(&repo, &["branch", "other"]);
        std::fs::write(repo.join("README.md"), "dirty edit\n").unwrap();

        let err = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Switch away".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::InPlace {
                        branch: "other".into(),
                    },
                },
            )
            .await
            .unwrap_err();

        assert!(
            matches!(err, CoreError::CheckoutDirty { count: 1 }),
            "got {err:?}"
        );
        assert_eq!(err.code(), "checkout_dirty");
        assert_eq!(
            err.to_string(),
            "the project checkout has 1 uncommitted change(s)"
        );
        assert_eq!(
            crate::gitops::current_branch(&repo).as_deref(),
            Some("main")
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "dirty edit\n"
        );
    }

    #[tokio::test]
    async fn create_inplace_allows_dirty_tree_on_current_branch() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        std::fs::write(repo.join("README.md"), "wip\n").unwrap();

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Adopt current".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::InPlace {
                        branch: "main".into(),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(created.branch, "main");
        assert_eq!(PathBuf::from(&created.path), repo);
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "wip\n"
        );
    }

    #[tokio::test]
    async fn base_detection_falls_back_to_local_main() {
        let (_guard, repo) = fixture_repo();
        let git = which::which("git").unwrap();
        assert_eq!(detect_base_branch(&git, &repo).await.unwrap(), "main");
    }

    #[tokio::test]
    async fn archive_restore_round_trip_preserves_uncommitted_work() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Restore me".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);

        std::fs::write(wt.join("README.md"), "edited\n").unwrap();
        std::fs::write(wt.join("scratch.txt"), "wip\n").unwrap();

        let archive_dir = repo.join("archived-contexts").join(&created.slug);
        let moved = manager
            .archive(&git, &repo, &wt, &archive_dir)
            .await
            .unwrap();
        assert!(moved);
        assert!(!wt.exists(), "source path freed after archive");
        assert!(archive_dir.join("README.md").exists());
        assert!(archive_dir.join("scratch.txt").exists());
        assert!(is_live_worktree(&archive_dir), "still a live worktree");

        let branches = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["branch", "--list", &created.branch])
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&branches.stdout).trim().is_empty());

        manager
            .restore(&git, &repo, &archive_dir, &wt)
            .await
            .unwrap();
        assert!(!archive_dir.exists());
        assert_eq!(
            std::fs::read_to_string(wt.join("README.md")).unwrap(),
            "edited\n"
        );
        assert_eq!(
            std::fs::read_to_string(wt.join("scratch.txt")).unwrap(),
            "wip\n"
        );
        let st = status(&wt, &created.base_branch, None);
        assert_eq!((st.dirty, st.untracked), (1, 1));
    }

    #[tokio::test]
    async fn restore_refuses_when_target_path_is_occupied() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Conflict".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);
        let archive_dir = repo.join("archived-contexts").join(&created.slug);
        manager
            .archive(&git, &repo, &wt, &archive_dir)
            .await
            .unwrap();

        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("squatter"), "occupied").unwrap();

        let err = manager
            .restore(&git, &repo, &archive_dir, &wt)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CoreError::Io(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists)
        );
    }

    #[tokio::test]
    async fn clean_ignored_removes_ignored_keeps_seeded_and_tracked() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Clean me".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);

        std::fs::write(wt.join(".gitignore"), "target/\n*.log\n.env\n").unwrap();
        sh(&wt, &["add", ".gitignore"]);
        sh(&wt, &["commit", "-m", "add gitignore"]);

        std::fs::create_dir_all(wt.join("target")).unwrap();
        std::fs::write(wt.join("target").join("junk"), "build output").unwrap();
        std::fs::write(wt.join("debug.log"), "log").unwrap();

        std::fs::write(wt.join("scratch.txt"), "wip").unwrap();

        std::fs::write(wt.join(".env"), "SECRET=1").unwrap();

        std::fs::write(wt.join("README.md"), "changed").unwrap();

        manager
            .clean_ignored(&git, &wt, &[".env".to_string()])
            .await
            .unwrap();

        assert!(!wt.join("target").exists(), "target/ must be cleaned");
        assert!(!wt.join("debug.log").exists(), "debug.log must be cleaned");

        assert!(
            wt.join("scratch.txt").exists(),
            "untracked non-ignored must survive"
        );
        assert!(wt.join(".env").exists(), ".env (keep pattern) must survive");
        assert!(wt.join("README.md").exists(), "tracked edit must survive");
    }

    #[tokio::test]
    async fn safe_remove_clears_husk_left_by_partial_teardown() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        let created = manager
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Husk".into(),
                    base_override: None,
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);

        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", wt.to_str().unwrap()])
            .output()
            .unwrap();
        std::fs::create_dir_all(wt.join(".vite")).unwrap();
        std::fs::create_dir_all(wt.join("logs")).unwrap();
        assert!(wt.exists() && !is_live_worktree(&wt), "set up husk");

        manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::Safe,
                true,
            )
            .await
            .unwrap();
        assert!(!wt.exists(), "husk directory must be cleaned up");
    }
}
