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

/// `<project>/archived-contexts` — Conductor-style: archived worktrees live
/// alongside their source repo so the relationship is visible in Finder.
/// `.git/info/exclude` keeps them out of the user's `git status`.
pub const ARCHIVE_SUBDIR: &str = "archived-contexts";

/// Where a workstream's worktree lands after archive: one directory per slug
/// under [`ARCHIVE_SUBDIR`].
pub fn archive_dir_for(project_repo: &Path, slug: &str) -> PathBuf {
    project_repo.join(ARCHIVE_SUBDIR).join(slug)
}

/// Append `archived-contexts/` to the project's per-clone exclude so its
/// archived worktrees don't show as untracked in the user's `git status`.
/// Idempotent: re-running is a no-op once the entry is present.
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
    /// Override base (e.g. "origin/release"); detected when None.
    /// Only consulted in [`CreateMode::Isolated`].
    pub base_override: Option<String>,
    /// Branch-name prefix (e.g. "hyprlayer/"). Used verbatim before the slug.
    /// Only consulted in [`CreateMode::Isolated`].
    pub branch_prefix: String,
    /// How to back the workstream.
    pub mode: CreateMode,
}

/// The three ways to back a new workstream. The default — [`CreateMode::Isolated`]
/// — is the original behavior; the other two adopt a branch the user already has.
#[derive(Debug, Clone)]
pub enum CreateMode {
    /// Create branch `<prefix><slug>` and a dedicated worktree from the base.
    Isolated,
    /// Check an existing *local* branch out into a dedicated worktree (no new
    /// branch). A remote-tracking ref would detach HEAD, so the caller must pass
    /// a local branch name.
    Existing { branch: String },
    /// No worktree: switch the project's own checkout onto an existing local
    /// `branch`. `worktree_path` becomes the project repo itself.
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

    /// Provision the worktree for a workstream per its [`CreateMode`]. Thoughts
    /// provisioning is the caller's next step (it needs config context this
    /// module doesn't own) — except in [`CreateMode::InPlace`], where the target
    /// is the already-provisioned project checkout.
    pub async fn create(&self, git: &Path, spec: &CreateSpec) -> Result<CreatedWorktree> {
        let slug = unique_slug(&spec.title);

        // Freshen remote refs so the base/branch is current; offline is fine.
        // Shared by every mode.
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
                // `-b <branch>`: create the branch as part of the checkout.
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
                // Base is informational here (ahead/behind fallback); detection
                // failing must not block adopting a branch the user already has.
                let base = detect_base_branch(git, &spec.project_repo)
                    .await
                    .unwrap_or_else(|_| branch.clone());
                // No `-b`: check the existing local branch out into the worktree.
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
                // No worktree: switch the project's own checkout onto `branch`.
                // On checkout git refuses only *conflicting* dirty changes —
                // non-conflicting modified files silently follow the switch onto
                // the new branch. To avoid carrying a user's in-progress work
                // across branches, refuse a dirty tree up front. Exception: when
                // we're already on `branch` the switch is a no-op, and the dirty
                // tree is that branch's own work — leave it untouched.
                let already_here = crate::gitops::current_branch(&spec.project_repo).as_deref()
                    == Some(branch.as_str());
                if !already_here {
                    // Untracked files ride along harmlessly and aren't "carried
                    // onto" the branch in a surprising way; only modified tracked
                    // files are. Guard on those.
                    let dirty = status(&spec.project_repo, branch, None).dirty;
                    if dirty > 0 {
                        return Err(CoreError::WorktreeDirty(dirty));
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

    /// Resolve + reserve the worktree directory for `slug`, erroring if it's
    /// already taken and creating its parent. Shared by the worktree-backed
    /// modes (isolated, existing).
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

    /// `git -C <repo> worktree add <extra...> <path> <committish>`. `extra` is
    /// `["-b", branch]` to create a branch, or empty to check an existing one out.
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

    /// Windows path-length escape valve, applied per worktree. No-op elsewhere.
    async fn windows_longpaths(&self, _git: &Path, _worktree: &Path) {
        #[cfg(windows)]
        {
            let _ = git_in(_git, _worktree, &["config", "core.longpaths", "true"])
                .run()
                .await;
        }
    }

    /// Remove worktree + branch. `Safe` refuses to destroy work; the caller
    /// maps the typed errors to confirm-dialog affordances. When
    /// `delete_branch` is false the local branch is left in place (the
    /// worktree is still removed).
    // Each parameter is a distinct, independent input (paths, branch names,
    // policy, branch-deletion opt-out); bundling them into a struct would only
    // add indirection.
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
            // Always --force the porcelain removal: the Safe gate above is
            // the real protection, and leftover app infrastructure (the
            // untracked thoughts/ links) would otherwise make git refuse.
            let wt = worktree.display().to_string();
            git_in(git, project_repo, &["worktree", "remove", "--force", &wt])
                .timeout(Duration::from_secs(60))
                .run()
                .await?;
        } else {
            // Husk or fully gone: git's `worktree remove` would fail (the path
            // isn't in its registry anymore, or has no `.git` to read), and
            // ensure_removable can't even read status. Prune the registration
            // and best-effort delete the leftover directory — anything still
            // in it is post-teardown runtime crud (.vite/, logs/) recreated by
            // a process whose cwd outlived the archive.
            let _ = git_in(git, project_repo, &["worktree", "prune"])
                .run()
                .await;
            if worktree.exists() {
                let _ = std::fs::remove_dir_all(worktree);
            }
        }

        // Branch cleanup: only-if-merged on Safe (tolerate failure and leave
        // the branch), forced on discard. Skipped entirely when the caller
        // opts out of branch deletion.
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

    /// Reconcile git's worktree registry after out-of-band deletions.
    pub async fn prune(&self, git: &Path, project_repo: &Path) -> Result<()> {
        git_in(git, project_repo, &["worktree", "prune"])
            .run()
            .await?;
        Ok(())
    }

    /// Move a worktree to its archive location, preserving the branch + every
    /// uncommitted file. Restore is the inverse [`Self::restore`]. Husks
    /// (worktree dir without `.git`) can't be moved by git, so we fall back to
    /// pruning + best-effort `rm -rf`: there's nothing recoverable left.
    /// Returns `true` when a real archive was produced; `false` for a husk
    /// (caller should not promise the user a restore path).
    pub async fn archive(
        &self,
        git: &Path,
        project_repo: &Path,
        worktree: &Path,
        dest: &Path,
    ) -> Result<bool> {
        if !is_live_worktree(worktree) {
            // Same husk handling as Self::remove — nothing recoverable here.
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
        // `git worktree move` refuses if the worktree is locked or contains
        // submodules, which surfaces as a CoreError::Git for the caller to map.
        git_in(git, project_repo, &["worktree", "move", &from, &to])
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(true)
    }

    /// Move a previously-archived worktree back to its original path. The
    /// caller must guarantee `target` is free — collision is the one failure
    /// mode the UI prompts for.
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

    /// Best-effort removal of git-ignored files from a live worktree before
    /// archive, so build artifacts (target/, node_modules/) don't bloat the
    /// archive. `keep` patterns (the repo's `worktree.copy` globs) are passed as
    /// `git clean -e !<pat>` negations so seeded files like `.env` survive even
    /// when they appear in the repo's gitignore. Single `-f` (not `-ff`) leaves
    /// nested git repos untouched.
    pub async fn clean_ignored(&self, git: &Path, worktree: &Path, keep: &[String]) -> Result<()> {
        let mut args: Vec<String> = vec!["clean".into(), "-d".into(), "-f".into(), "-X".into()];
        for pat in keep {
            // Negation `!<pat>` un-ignores the pattern, keeping matching files.
            args.push("-e".into());
            args.push(format!("!{pat}"));
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        git_in(git, worktree, &arg_refs) // git -C <worktree> clean -d -f -X -e !<pat>...
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(())
    }

    /// Permanent purge: prune the archive directory + delete the branch.
    /// Used by the "Delete permanently" action on the archive history view.
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
        // The archive was a registered worktree until this moment; prune to
        // drop the registry entry now that its directory is gone.
        let _ = git_in(git, project_repo, &["worktree", "prune"])
            .run()
            .await;
        if delete_branch {
            let _ = git_in(git, project_repo, &["branch", "-D", branch])
                .run()
                .await;
        }
        Ok(())
    }
}

/// True iff `path` is a live git worktree (has a `.git` entry — directory for
/// the main checkout, regular file with `gitdir:` for a linked worktree). A
/// path that exists without `.git` is a husk left over from a partial teardown
/// (e.g. `git worktree remove` succeeded but a process whose cwd was inside
/// recreated runtime dirs after); callers treat it the same as "missing".
pub fn is_live_worktree(path: &Path) -> bool {
    path.join(".git").exists()
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
                true,
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
                true,
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
                true,
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

    /// Existing-branch mode checks a branch the user already has out into a
    /// dedicated worktree — no new branch is created, and the worktree's HEAD is
    /// that branch (not detached).
    #[tokio::test]
    async fn create_existing_checks_out_user_branch_without_creating_one() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        // A branch the user already created.
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
        // The worktree is on the branch, not detached.
        let st = status(&wt, &created.base_branch, None);
        assert_eq!(st.branch.as_deref(), Some("feature-x"));
    }

    /// In-place mode creates no worktree: it switches the project's own checkout
    /// onto the chosen branch and reports the repo itself as the path.
    #[tokio::test]
    async fn create_inplace_switches_main_checkout_and_makes_no_worktree() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        // Move the main checkout off main so the create has to switch it back.
        sh(&repo, &["checkout", "-b", "side"]);
        assert_eq!(crate::gitops::current_branch(&repo).as_deref(), Some("side"));

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
        // The path is the project repo itself — no dedicated worktree dir.
        assert_eq!(PathBuf::from(&created.path), repo);
        assert!(
            std::fs::read_dir(wt_root.path()).unwrap().next().is_none(),
            "in-place must not create a worktree under the root"
        );
        // The main checkout was switched onto the chosen branch.
        assert_eq!(crate::gitops::current_branch(&repo).as_deref(), Some("main"));
    }

    /// In-place onto a *different* branch refuses a dirty tree: a plain checkout
    /// would silently carry the user's modified tracked files across, so the
    /// branch + checkout are left exactly as they were.
    #[tokio::test]
    async fn create_inplace_refuses_dirty_tree_when_switching_branches() {
        let (_guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();

        // A branch to switch to, while a tracked file is left modified on main.
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

        assert!(matches!(err, CoreError::WorktreeDirty(_)), "got {err:?}");
        // Nothing moved: still on main, edit intact.
        assert_eq!(crate::gitops::current_branch(&repo).as_deref(), Some("main"));
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "dirty edit\n"
        );
    }

    /// In-place onto the branch already checked out is a no-op switch, so a dirty
    /// tree is fine — that work belongs to the branch being adopted and must be
    /// left untouched.
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
        // The in-progress edit is preserved exactly.
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

    /// Archive → restore round-trip preserves the branch and the uncommitted
    /// edits that block a Safe remove. This is the non-destructive
    /// alternative to remove(): work survives, restore returns the worktree
    /// to its original path with `git worktree move`.
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

        // Dirty edit + untracked file: archive must preserve both.
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

        // The branch survived archive and git still tracks the worktree at
        // its new location.
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

    /// Restore refuses (rather than clobbers) when the original path was
    /// re-used by another workstream while this one was archived.
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

        // Something else took the path before the user clicked Unarchive.
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

    /// `clean_ignored` removes git-ignored files but preserves `keep` patterns
    /// (worktree.copy), tracked edits, and untracked-not-ignored files.
    /// `.env` is included in the gitignore to verify that the negation exclude
    /// (`!.env`) overrides the gitignore rule and keeps the seeded file.
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

        // Gitignore: target/, *.log, and .env (seeded files are typically ignored).
        std::fs::write(wt.join(".gitignore"), "target/\n*.log\n.env\n").unwrap();
        sh(&wt, &["add", ".gitignore"]);
        sh(&wt, &["commit", "-m", "add gitignore"]);

        // Create ignored files/dirs.
        std::fs::create_dir_all(wt.join("target")).unwrap();
        std::fs::write(wt.join("target").join("junk"), "build output").unwrap();
        std::fs::write(wt.join("debug.log"), "log").unwrap();

        // Create untracked-but-NOT-ignored file (should survive).
        std::fs::write(wt.join("scratch.txt"), "wip").unwrap();

        // Create a "seeded" ignored file; its pattern is in keep so the `!.env`
        // negation un-ignores it and `git clean -X` leaves it alone.
        std::fs::write(wt.join(".env"), "SECRET=1").unwrap();

        // Tracked edit (should survive).
        std::fs::write(wt.join("README.md"), "changed").unwrap();

        manager
            .clean_ignored(&git, &wt, &[".env".to_string()])
            .await
            .unwrap();

        // Ignored files removed.
        assert!(!wt.join("target").exists(), "target/ must be cleaned");
        assert!(!wt.join("debug.log").exists(), "debug.log must be cleaned");

        // Kept: not-ignored, tracked, and seeded.
        assert!(
            wt.join("scratch.txt").exists(),
            "untracked non-ignored must survive"
        );
        assert!(wt.join(".env").exists(), ".env (keep pattern) must survive");
        assert!(wt.join("README.md").exists(), "tracked edit must survive");
    }

    /// Husk state: the worktree directory exists but its `.git` linkfile is
    /// gone and only post-teardown runtime crud remains. This reproduces
    /// project-removal-21c6 — a Safe archive there hung because ensure_removable
    /// couldn't open the repo and `git worktree remove --force` rejected a path
    /// no longer in git's registry. `remove` must succeed under Safe and clear
    /// the directory.
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

        // Simulate the partial teardown: drop git's record of the worktree
        // (which also removes the `.git` linkfile), then have a still-running
        // process recreate runtime dirs at the cwd.
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
