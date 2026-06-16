//! Worktree lifecycle. Hybrid git strategy (mirrors the hyprlayer CLI):
//! `git worktree add/remove/prune` via subprocess — the porcelain handles
//! branch-from-base creation, sparse checkout, and hooks correctly — while
//! status polling uses git2 so it never forks.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::process::Exec;

use super::thoughts_links::LinkStatus;

/// `~/hyprlayer/worktrees` — user-visible (like Conductor's workspaces),
/// short (Windows path budgets), and outside any repo (no gitignore or
/// watcher interplay).
pub fn default_worktree_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hyprlayer")
        .join("worktrees")
}

pub struct CreateSpec {
    pub project_repo: PathBuf,
    pub mapped_name: String,
    pub title: String,
    /// Override base (e.g. "origin/release"); detected when None.
    pub base_override: Option<String>,
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
    /// Tracked changes (staged + unstaged), excluding thoughts/.
    pub dirty: u32,
    /// Added lines in the working-tree diff vs HEAD, excluding thoughts/.
    pub additions: u32,
    /// Deleted lines in the working-tree diff vs HEAD, excluding thoughts/.
    pub deletions: u32,
    /// Untracked files, excluding thoughts/ (informational only).
    pub untracked: u32,
    pub ahead: u32,
    pub behind: u32,
    pub thoughts: LinkStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovePolicy {
    /// Refuse when dirty or carrying unmerged commits.
    Safe,
    /// Drop everything: changes, commits, branch.
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

    /// Create branch `hyprlayer/<slug>` + worktree from the detected base.
    /// Thoughts provisioning is the caller's next step (it needs config
    /// context this module doesn't own).
    pub async fn create(&self, git: &Path, spec: &CreateSpec) -> Result<CreatedWorktree> {
        let slug = unique_slug(&spec.title);
        let branch = format!("hyprlayer/{slug}");
        let path = self.worktree_dir(&spec.mapped_name, &slug);
        if path.exists() {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("worktree dir already exists: {}", path.display()),
            )));
        }
        std::fs::create_dir_all(path.parent().expect("worktree dir has parent"))?;

        // Freshen remote refs so the base is current; offline is fine.
        let _ = git_in(git, &spec.project_repo, &["fetch", "--prune", "origin"])
            .timeout(Duration::from_secs(30))
            .run()
            .await;

        let base = match &spec.base_override {
            Some(b) => b.clone(),
            None => detect_base_branch(git, &spec.project_repo).await?,
        };

        git_in(
            git,
            &spec.project_repo,
            &[
                "worktree",
                "add",
                "-b",
                &branch,
                path.to_str().ok_or_else(|| CoreError::OutputParse {
                    program: "worktree add".into(),
                    detail: "worktree path is not valid UTF-8".into(),
                })?,
                &base,
            ],
        )
        .timeout(Duration::from_secs(120))
        .run()
        .await?;

        #[cfg(windows)]
        {
            let _ = git_in(git, &path, &["config", "core.longpaths", "true"])
                .run()
                .await;
        }

        Ok(CreatedWorktree {
            path: path.display().to_string(),
            branch,
            slug,
            base_branch: base,
        })
    }

    /// Remove worktree + branch. `Safe` refuses to destroy work; the caller
    /// maps the typed errors to confirm-dialog affordances.
    pub async fn remove(
        &self,
        git: &Path,
        project_repo: &Path,
        worktree: &Path,
        branch: &str,
        base_branch: &str,
        policy: RemovePolicy,
    ) -> Result<()> {
        if worktree.exists() {
            if policy == RemovePolicy::Safe {
                ensure_removable(worktree, base_branch, branch)?;
            }
            // Always --force the porcelain removal: the Safe gate above is
            // the real protection, and leftover app infrastructure (the
            // untracked thoughts/ links) would otherwise make git refuse.
            let wt = worktree.display().to_string();
            git_in(git, project_repo, &["worktree", "remove", "--force", &wt])
                .timeout(Duration::from_secs(60))
                .run()
                .await?;
        } else {
            // Directory gone out-of-band: clear the stale registration.
            let _ = git_in(git, project_repo, &["worktree", "prune"])
                .run()
                .await;
        }

        // Branch cleanup: only-if-merged on Safe (tolerate failure and leave
        // the branch), forced on discard.
        let delete_flag = if policy == RemovePolicy::ForceDiscard {
            "-D"
        } else {
            "-d"
        };
        let _ = git_in(git, project_repo, &["branch", delete_flag, branch])
            .run()
            .await;

        Ok(())
    }

    /// Reconcile git's worktree registry after out-of-band deletions.
    pub async fn prune(&self, git: &Path, project_repo: &Path) -> Result<()> {
        git_in(git, project_repo, &["worktree", "prune"])
            .run()
            .await?;
        Ok(())
    }
}

/// The Safe-removal gate: errors when removing the worktree would destroy
/// work. Public so callers that must act between the gate and the removal
/// (e.g. `thoughts uninit`) share the one definition of destroyable work.
pub fn ensure_removable(worktree: &Path, base_branch: &str, branch: &str) -> Result<()> {
    let status = status(worktree, base_branch, None);
    // Untracked files (outside thoughts/) count too: removal deletes them
    // just as surely as tracked edits.
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

/// `origin/HEAD` symref when set, else the first of origin/main,
/// origin/master, main, master that exists.
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
            "no main/master branch found in {} — pass a base branch explicitly",
            repo.display()
        ),
    })
}

/// Sync status snapshot via git2 (cheap; callers use spawn_blocking).
/// `base_branch` is the merge target used for ahead/behind when the branch
/// has no upstream yet (the usual state before first push).
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

    // Line-level counts come from the same diff the panel renders, so the
    // sidebar's "+adds −dels" always matches what's listed there.
    let (additions, deletions) = crate::gitops::worktree_diff_line_counts(&repo);

    let head = repo.head().ok();
    let branch = head.as_ref().and_then(|h| h.shorthand()).map(String::from);

    let (ahead, behind) = (|| -> Option<(u32, u32)> {
        let head_ref = head.as_ref()?;
        let local = head_ref.target()?;
        // Prefer the upstream once it exists; fall back to the base branch.
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

    WorktreeStatus {
        exists: true,
        branch,
        dirty,
        additions,
        deletions,
        untracked,
        ahead,
        behind,
        thoughts: thoughts.unwrap_or(LinkStatus::Ok),
    }
}

/// `<kebab-title>-<4 hex chars>`: readable, collision-resistant, filesystem-
/// and ref-safe.
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

    #[test]
    fn slugs_are_kebab_and_unique() {
        let a = unique_slug("Add OAuth2 login!");
        assert!(a.starts_with("add-oauth2-login-"), "{a}");
        assert_ne!(unique_slug("x"), unique_slug("x"));
        assert!(unique_slug("!!!").starts_with("work-item-"));
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
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);
        assert!(wt.join("README.md").exists());
        assert_eq!(created.base_branch, "main");
        assert!(created.branch.starts_with("hyprlayer/auth-work-"));

        // Clean tree: removable under Safe policy.
        let st = status(&wt, &created.base_branch, None);
        assert!(st.exists);
        assert_eq!((st.dirty, st.untracked, st.ahead), (0, 0, 0));

        // Untracked files (outside thoughts/) block Safe removal too.
        std::fs::write(wt.join("scratch.txt"), "wip\n").unwrap();
        let err = manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::Safe,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "worktree_dirty");
        std::fs::remove_file(wt.join("scratch.txt")).unwrap();

        // Tracked edit makes it dirty and blocks Safe removal.
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
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "worktree_dirty");

        // Commit → ahead of base → still blocked; thoughts/ is ignored.
        std::fs::create_dir_all(wt.join("thoughts/sub")).unwrap();
        std::fs::write(wt.join("thoughts/sub/x.md"), "ignored").unwrap();
        sh(&wt, &["add", "README.md"]);
        sh(&wt, &["commit", "-m", "wip"]);
        let st = status(&wt, &created.base_branch, None);
        assert_eq!((st.dirty, st.ahead), (0, 1));
        assert_eq!(st.untracked, 0, "thoughts/ must not count");
        let err = manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::Safe,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "worktree_unpushed");

        // ForceDiscard drops everything.
        manager
            .remove(
                &git,
                &repo,
                &wt,
                &created.branch,
                &created.base_branch,
                RemovePolicy::ForceDiscard,
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
    async fn base_detection_falls_back_to_local_main() {
        let (_guard, repo) = fixture_repo();
        let git = which::which("git").unwrap();
        assert_eq!(detect_base_branch(&git, &repo).await.unwrap(), "main");
    }
}
