use std::collections::BTreeMap;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::binaries::BinaryResolver;
use crate::process::Exec;

use super::thoughts_links::LinkStatus;

/// Empty overlay, for tests that don't care about it.
#[cfg(test)]
const NO_OVERLAY: &[(String, String)] = &[];

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
    Attached { branch: String, path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CreatedWorktree {
    pub path: String,
    pub branch: String,
    pub slug: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStatus {
    pub exists: bool,
    pub branch: Option<String>,
    pub dirty: u32,
    pub additions: u32,
    pub deletions: u32,
    pub untracked: u32,
    pub base: Divergence,
    pub upstream: Option<Upstream>,
    pub unpushed: u32,
    pub thoughts: LinkStatus,
    pub unreadable: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Divergence {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Upstream {
    pub name: String,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovePolicy {
    Safe,
    ForceDiscard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchCleanup {
    Keep,
    /// `git branch -d`, which refuses a branch whose commits exist nowhere else.
    SafeLocal,
    /// `git branch -D` and the remote branch.
    Force,
}

/// What an archived worktree holds; `None` where it could not be read.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArchiveScan {
    pub size_bytes: Option<f64>,
    pub uncommitted: Option<u32>,
    pub unpushed: Option<u32>,
}

/// Symlinks are not followed, so the thoughts links never count.
pub fn scan_archive(dir: &Path, base_branch: &str, merged_head: Option<&str>) -> ArchiveScan {
    if !dir.is_dir() {
        return ArchiveScan::default();
    }
    let size: u64 = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum();
    let git = is_live_worktree(dir)
        .then(|| status(dir, base_branch, merged_head, None))
        .filter(|s| s.exists);
    ArchiveScan {
        size_bytes: Some(size as f64),
        uncommitted: git
            .as_ref()
            .filter(|s| s.unreadable.is_none())
            .map(|s| s.dirty + s.untracked),
        unpushed: git.as_ref().map(|s| s.unpushed),
    }
}

pub struct WorktreeManager {
    root: PathBuf,
    env_overlay: Arc<RwLock<BTreeMap<String, String>>>,
    binaries: Option<Arc<BinaryResolver>>,
}

impl WorktreeManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            env_overlay: Arc::default(),
            binaries: None,
        }
    }

    /// Shares the runtime's live overlay (ADR-0009 §7) rather than copying
    /// it once, so a later attach's `SSH_AUTH_SOCK` reaches every git shell
    /// this manager spawns from then on.
    pub fn with_env_overlay(mut self, env_overlay: Arc<RwLock<BTreeMap<String, String>>>) -> Self {
        self.env_overlay = env_overlay;
        self
    }

    /// The resolver's `PATH` for every git this manager spawns; without it git
    /// inherits the process's own, which launchd keeps minimal.
    pub fn with_binaries(mut self, binaries: Arc<BinaryResolver>) -> Self {
        self.binaries = Some(binaries);
        self
    }

    async fn child_env(&self) -> Vec<(String, String)> {
        let overlay: Vec<(String, String)> = self
            .env_overlay
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        match &self.binaries {
            Some(binaries) => binaries.child_env(overlay).await,
            None => overlay,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn worktree_dir(&self, mapped_name: &str, slug: &str) -> PathBuf {
        self.root.join(mapped_name).join(slug)
    }

    pub async fn create(&self, git: &Path, spec: &CreateSpec) -> Result<CreatedWorktree> {
        let slug = unique_slug(&spec.title);
        let overlay = self.child_env().await;

        // Best effort: offline still creates, from whatever refs are local.
        if !matches!(spec.mode, CreateMode::Attached { .. }) {
            if let Err(e) = git_in(
                git,
                &spec.project_repo,
                &["fetch", "--prune", "origin"],
                &overlay,
            )
            .timeout(Duration::from_secs(30))
            .run()
            .await
            {
                tracing::warn!(
                    "fetch in {} failed; creating from local refs: {e}",
                    spec.project_repo.display()
                );
            }
        }

        match &spec.mode {
            CreateMode::Isolated => {
                let branch = format!("{}{slug}", spec.branch_prefix);
                let path = self.prepare_worktree_dir(&spec.mapped_name, &slug)?;
                let base = match &spec.base_override {
                    Some(b) => b.clone(),
                    None => detect_base_branch(git, &spec.project_repo, &overlay).await?,
                };
                self.worktree_add(
                    git,
                    &spec.project_repo,
                    &path,
                    &["--no-track", "-b", &branch],
                    &base,
                )
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
                let base = detect_base_branch(git, &spec.project_repo, &overlay)
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
            CreateMode::Attached { branch, path } => {
                let base = detect_base_branch(git, &spec.project_repo, &overlay)
                    .await
                    .unwrap_or_else(|_| branch.clone());
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
                    let dirty = crate::gitops::checkout_blockers(&spec.project_repo)?;
                    if dirty > 0 {
                        return Err(CoreError::CheckoutDirty { count: dirty });
                    }
                    git_in(git, &spec.project_repo, &["checkout", branch], &overlay)
                        .timeout(Duration::from_secs(60))
                        .run()
                        .await?;
                }
                let base = detect_base_branch(git, &spec.project_repo, &overlay)
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
        git_in(git, project_repo, &args, &self.child_env().await)
            .timeout(Duration::from_secs(120))
            .run()
            .await?;
        Ok(())
    }

    async fn windows_longpaths(&self, _git: &Path, _worktree: &Path) {
        #[cfg(windows)]
        {
            let _ = git_in(
                _git,
                _worktree,
                &["config", "core.longpaths", "true"],
                &self.child_env().await,
            )
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
        let overlay = self.child_env().await;
        if is_live_worktree(worktree) {
            if policy == RemovePolicy::Safe {
                ensure_removable(worktree, base_branch, branch)?;
            }
            let wt = worktree.display().to_string();
            git_in(
                git,
                project_repo,
                &["worktree", "remove", "--force", &wt],
                &overlay,
            )
            .timeout(Duration::from_secs(60))
            .run()
            .await?;
        } else {
            if let Err(e) = git_in(git, project_repo, &["worktree", "prune"], &overlay)
                .run()
                .await
            {
                tracing::warn!("worktree prune in {} failed: {e}", project_repo.display());
            }
            if worktree.exists() {
                if let Err(e) = std::fs::remove_dir_all(worktree) {
                    tracing::warn!("could not remove {}: {e}", worktree.display());
                }
            }
        }

        if delete_branch {
            let delete_flag = if policy == RemovePolicy::ForceDiscard {
                "-D"
            } else {
                "-d"
            };
            // `-d` refuses an unmerged branch by design; the branch stays.
            if let Err(e) = git_in(
                git,
                project_repo,
                &["branch", delete_flag, branch],
                &overlay,
            )
            .run()
            .await
            {
                tracing::info!("kept branch {branch}: {e}");
            }
        }

        Ok(())
    }

    pub async fn prune(&self, git: &Path, project_repo: &Path) -> Result<()> {
        git_in(
            git,
            project_repo,
            &["worktree", "prune"],
            &self.child_env().await,
        )
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
        let overlay = self.child_env().await;
        if !is_live_worktree(worktree) {
            let _ = git_in(git, project_repo, &["worktree", "prune"], &overlay)
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
        git_in(
            git,
            project_repo,
            &["worktree", "move", &from, &to],
            &overlay,
        )
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
        git_in(
            git,
            project_repo,
            &["worktree", "move", &from, &to],
            &self.child_env().await,
        )
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
        git_in(git, worktree, &arg_refs, &self.child_env().await)
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
        cleanup: BranchCleanup,
    ) -> Result<()> {
        let overlay = self.child_env().await;
        // Each step is best effort; the log is the only record of what was left.
        if archive.exists() {
            if let Err(e) = std::fs::remove_dir_all(archive) {
                tracing::warn!("purge left {}: {e}", archive.display());
            }
        }
        if let Err(e) = git_in(git, project_repo, &["worktree", "prune"], &overlay)
            .run()
            .await
        {
            tracing::warn!("worktree prune in {} failed: {e}", project_repo.display());
        }
        let flag = match cleanup {
            BranchCleanup::Keep => return Ok(()),
            BranchCleanup::SafeLocal => "-d",
            BranchCleanup::Force => "-D",
        };
        if let Err(e) = git_in(git, project_repo, &["branch", flag, branch], &overlay)
            .run()
            .await
        {
            tracing::warn!("purge kept local branch {branch}: {e}");
        }
        if cleanup == BranchCleanup::Force {
            if let Err(e) = delete_remote_branch(git, project_repo, branch, &overlay).await {
                tracing::warn!("purge kept remote branch {branch}: {e}");
            }
        }
        Ok(())
    }
}

pub fn is_live_worktree(path: &Path) -> bool {
    path.join(".git").exists()
}

pub fn ensure_removable(worktree: &Path, base_branch: &str, branch: &str) -> Result<()> {
    let status = status(worktree, base_branch, None, None);
    if let Some(detail) = status.unreadable {
        return Err(CoreError::WorktreeUnreadable { detail });
    }
    if status.dirty + status.untracked > 0 {
        return Err(CoreError::WorktreeDirty(status.dirty + status.untracked));
    }
    if status.unpushed > 0 {
        return Err(CoreError::WorktreeUnpushed {
            branch: branch.to_string(),
            count: status.unpushed,
        });
    }
    Ok(())
}

fn git_in(git: &Path, repo: &Path, args: &[&str], overlay: &[(String, String)]) -> Exec {
    let repo_arg: OsString = repo.as_os_str().to_os_string();
    Exec::new(git)
        .arg("-C")
        .arg(repo_arg)
        .args(args.iter().copied())
        .envs(overlay.iter().cloned())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PullOutcome {
    UpToDate,
    Merged,
    Conflicts { files: Vec<String> },
}

pub async fn pull_base(
    git: &Path,
    worktree: &Path,
    base: &str,
    overlay: &[(String, String)],
) -> Result<PullOutcome> {
    let branch = base.strip_prefix("origin/").unwrap_or(base);

    if let Err(e) = git_in(git, worktree, &["fetch", "origin", branch], overlay)
        .timeout(Duration::from_secs(60))
        .run()
        .await
    {
        tracing::warn!("fetching {branch} failed; merging the local origin/{branch}: {e}");
    }

    let merge_ref = crate::gitops::open_repo(worktree)
        .map_or_else(|_| base.to_string(), |repo| update_ref(&repo, base));

    let out = git_in(git, worktree, &["merge", "--no-edit", &merge_ref], overlay)
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

    let files = unmerged_files(git, worktree, overlay).await;
    if !files.is_empty() {
        return Ok(PullOutcome::Conflicts { files });
    }
    Err(CoreError::Exec {
        program: "git merge".into(),
        exit: out.exit_code,
        stderr_tail: out.stderr.trim().to_string(),
    })
}

pub async fn pull_upstream(
    git: &Path,
    worktree: &Path,
    base: &str,
    overlay: &[(String, String)],
) -> Result<PullOutcome> {
    let (refname, remote, branch) = crate::gitops::current_branch(worktree)
        .and_then(|branch| {
            let repo = crate::gitops::open_repo(worktree).ok()?;
            let refname = crate::gitops::pushed_ref(&repo, &branch, base)?
                .name()?
                .to_string();
            let (remote, branch) = crate::gitops::remote_ref_parts(&repo, &refname)?;
            Some((refname, remote, branch))
        })
        .ok_or_else(|| CoreError::Exec {
            program: "git pull".into(),
            exit: None,
            stderr_tail: "this branch has not been pushed".into(),
        })?;

    if let Err(e) = git_in(git, worktree, &["fetch", &remote, &branch], overlay)
        .timeout(Duration::from_secs(60))
        .run()
        .await
    {
        tracing::warn!("fetching {remote}/{branch} failed; merging the local {refname}: {e}");
    }

    let out = git_in(git, worktree, &["merge", "--ff-only", &refname], overlay)
        .timeout(Duration::from_secs(120))
        .run_unchecked()
        .await?;
    if out.exit_code != Some(0) {
        return Err(CoreError::Exec {
            program: "git merge --ff-only".into(),
            exit: out.exit_code,
            stderr_tail: out.stderr.trim().to_string(),
        });
    }
    Ok(if out.stdout.contains("Already up to date") {
        PullOutcome::UpToDate
    } else {
        PullOutcome::Merged
    })
}

pub async fn fetch_remote(
    git: &Path,
    worktree: &Path,
    base: Option<&str>,
    overlay: &[(String, String)],
) -> Result<()> {
    let Some(remote) = crate::gitops::fetch_remote_for(worktree, base) else {
        return Ok(());
    };
    git_in(git, worktree, &["fetch", "--prune", &remote], overlay)
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

pub async fn fetch_unattended(
    git: &Path,
    repo: &Path,
    remotes: &[String],
    overlay: &[(String, String)],
) -> Result<()> {
    let mut args = vec![
        "-c",
        "core.askPass=",
        "-c",
        "credential.interactive=never",
        "fetch",
        "--prune",
        "--no-write-fetch-head",
        "--multiple",
    ];
    args.extend(remotes.iter().map(String::as_str));
    let no_prompts = [
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_ASKPASS", ""),
        ("SSH_ASKPASS", ""),
        ("SSH_ASKPASS_REQUIRE", "never"),
        ("GCM_INTERACTIVE", "never"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    let env: Vec<(String, String)> = overlay.iter().cloned().chain(no_prompts).collect();
    git_in(git, repo, &args, &env)
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
    overlay: &[(String, String)],
) -> Result<()> {
    git_in(
        git,
        repo,
        &["branch", "--track", local, remote_ref],
        overlay,
    )
    .timeout(Duration::from_secs(60))
    .run()
    .await?;
    Ok(())
}

pub async fn delete_remote_branch(
    git: &Path,
    repo: &Path,
    branch: &str,
    overlay: &[(String, String)],
) -> Result<()> {
    git_in(git, repo, &["push", "origin", "--delete", branch], overlay)
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

/// Run inside the worktree, so git moves that worktree's HEAD along with the branch.
pub async fn rename_branch(
    git: &Path,
    worktree: &Path,
    from: &str,
    to: &str,
    overlay: &[(String, String)],
) -> Result<()> {
    git_in(git, worktree, &["branch", "-m", from, to], overlay)
        .run()
        .await?;
    Ok(())
}

async fn unmerged_files(git: &Path, worktree: &Path, overlay: &[(String, String)]) -> Vec<String> {
    match git_in(
        git,
        worktree,
        &["diff", "--name-only", "--diff-filter=U"],
        overlay,
    )
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

pub async fn detect_base_branch(
    git: &Path,
    repo: &Path,
    overlay: &[(String, String)],
) -> Result<String> {
    if let Ok(out) = git_in(
        git,
        repo,
        &["symbolic-ref", "refs/remotes/origin/HEAD"],
        overlay,
    )
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
            overlay,
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
        base: Divergence::default(),
        upstream: None,
        unpushed: 0,
        thoughts: LinkStatus::Unsupported,
        unreadable: None,
    }
}

fn update_ref(repo: &git2::Repository, base: &str) -> String {
    let remote = format!("origin/{}", base.strip_prefix("origin/").unwrap_or(base));
    if repo
        .find_reference(&format!("refs/remotes/{remote}"))
        .is_ok()
    {
        remote
    } else {
        base.to_string()
    }
}

fn commit_of(repo: &git2::Repository, rev: &str) -> Option<git2::Oid> {
    let commit = repo.revparse_single(rev).ok()?.peel_to_commit().ok()?;
    Some(commit.id())
}

// Hiding 5,000 remote tips costs ~20 ms per walk, and status() runs on every watcher event; the
// count is a pure function of these oids, so repeat reads skip the walk.
static LOCAL_ONLY: Mutex<BTreeMap<u64, u32>> = Mutex::new(BTreeMap::new());

fn local_only(
    repo: &git2::Repository,
    head: git2::Oid,
    base_tip: Option<git2::Oid>,
    merged: Option<git2::Oid>,
) -> Option<u32> {
    let mut tips: Vec<git2::Oid> = repo
        .references_glob("refs/remotes/*")
        .ok()?
        .flatten()
        .filter_map(|r| r.target())
        .collect();
    tips.sort_unstable();
    tips.dedup();
    // With no remote-tracking ref, `--not --remotes` hides nothing: count past the base.
    let floor = if tips.is_empty() { base_tip } else { None };
    let hidden: Vec<git2::Oid> = tips.into_iter().chain(floor).chain(merged).collect();
    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (head, &hidden).hash(&mut hasher);
        hasher.finish()
    };
    if let Some(&count) = LOCAL_ONLY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return Some(count);
    }
    let mut walk = repo.revwalk().ok()?;
    walk.push(head).ok()?;
    for oid in &hidden {
        let _ = walk.hide(*oid);
    }
    let count = walk.count() as u32;
    let mut memo = LOCAL_ONLY.lock().unwrap_or_else(PoisonError::into_inner);
    if memo.len() >= 4096 {
        memo.clear();
    }
    memo.insert(key, count);
    Some(count)
}

/// `merged_head` is the head of the branch's merged PR: a squash merge delivered the commits it
/// reaches under other ids, so they are not unpushed.
pub fn status(
    worktree: &Path,
    base_branch: &str,
    merged_head: Option<&str>,
    thoughts: Option<LinkStatus>,
) -> WorktreeStatus {
    let missing = WorktreeStatus {
        exists: false,
        branch: None,
        dirty: 0,
        additions: 0,
        deletions: 0,
        untracked: 0,
        base: Divergence::default(),
        upstream: None,
        unpushed: 0,
        thoughts: thoughts.unwrap_or(LinkStatus::Broken),
        unreadable: None,
    };
    if !worktree.exists() {
        return missing;
    }
    let Ok(repo) = crate::gitops::open_repo(worktree) else {
        return missing;
    };

    let (changes, unreadable) =
        match crate::gitops::read_changes(&repo, git2::StatusShow::IndexAndWorkdir) {
            Ok(changes) => (changes, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
    let (mut dirty, mut untracked) = (0u32, 0u32);
    for change in &changes {
        if change.status.intersects(git2::Status::WT_NEW) {
            untracked += 1;
        }
        if change.tracked() {
            dirty += 1;
        }
    }

    let (additions, deletions) = crate::gitops::worktree_diff_line_counts(&repo, &changes);

    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .filter(|h| h.is_branch())
        .and_then(|h| h.shorthand())
        .map(String::from);

    let local = head.as_ref().and_then(|h| h.target());
    let divergence = |target: git2::Oid| -> Option<Divergence> {
        let (ahead, behind) = repo.graph_ahead_behind(local?, target).ok()?;
        Some(Divergence {
            ahead: ahead as u32,
            behind: behind as u32,
        })
    };
    let base_ref = update_ref(&repo, base_branch);
    let base_tip = commit_of(&repo, &base_ref);
    let base = base_tip.and_then(divergence).unwrap_or_default();
    let upstream = branch
        .as_deref()
        .and_then(|b| crate::gitops::pushed_ref(&repo, b, base_branch))
        .and_then(|r| {
            let d = divergence(r.target()?)?;
            Some(Upstream {
                name: r.shorthand()?.to_string(),
                ahead: d.ahead,
                behind: d.behind,
            })
        });
    let base_is_remote = repo
        .find_reference(&format!("refs/remotes/{base_ref}"))
        .is_ok();
    let all_pushed = upstream.as_ref().is_some_and(|u| u.ahead == 0)
        || (base_is_remote && base_tip.is_some() && base.ahead == 0);
    let unpushed = match local {
        Some(head) if !all_pushed => {
            let merged = merged_head
                .and_then(|h| git2::Oid::from_str(h).ok())
                .filter(|oid| repo.odb().is_ok_and(|odb| odb.exists(*oid)));
            local_only(&repo, head, base_tip, merged).unwrap_or(base.ahead)
        }
        _ => 0,
    };

    WorktreeStatus {
        exists: true,
        branch,
        dirty,
        additions,
        deletions,
        untracked,
        base,
        upstream,
        unpushed,
        thoughts: thoughts.unwrap_or(LinkStatus::Ok),
        unreadable,
    }
}

pub fn unique_slug(title: &str) -> String {
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..4];
    format!("{}-{suffix}", slug_stem(title))
}

pub fn slug_stem(title: &str) -> String {
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
    if kebab.is_empty() { "work-item" } else { kebab }.to_string()
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

        assert!(!status(&notes, "", None, None).exists);

        let st = folder_status(&notes);
        assert!(st.exists);
        assert_eq!(st.branch, None);
        assert_eq!(
            (st.dirty, st.untracked, st.base, st.upstream, st.unpushed),
            (0, 0, Divergence::default(), None, 0)
        );
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

        create_tracking_branch(
            &git,
            &repo,
            "linux-support",
            "origin/linux-support",
            NO_OVERLAY,
        )
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
            status(Path::new(&created.path), "main", None, None)
                .branch
                .as_deref(),
            Some("linux-support")
        );

        assert!(create_tracking_branch(
            &git,
            &repo,
            "linux-support",
            "origin/linux-support",
            NO_OVERLAY,
        )
        .await
        .is_err());
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

        let st = status(&wt, &created.base_branch, None, None);
        assert!(st.exists);
        assert_eq!((st.dirty, st.untracked, st.unpushed), (0, 0, 0));

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
        let st = status(&wt, &created.base_branch, None, None);
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
        let st = status(&wt, &created.base_branch, None, None);
        assert_eq!((st.dirty, st.unpushed, st.base.ahead), (0, 1, 1));
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
            status(Path::new(&created.path), "main", None, None)
                .branch
                .as_deref(),
            Some(created.branch.as_str())
        );
    }

    fn divergence(ahead: u32, behind: u32) -> Divergence {
        Divergence { ahead, behind }
    }

    fn upstream(name: &str, ahead: u32, behind: u32) -> Option<Upstream> {
        Some(Upstream {
            name: name.into(),
            ahead,
            behind,
        })
    }

    fn commit_file(repo: &Path, name: &str) {
        std::fs::write(repo.join(name), format!("{name}\n")).unwrap();
        sh(repo, &["add", name]);
        sh(repo, &["commit", "-m", name]);
    }

    #[test]
    fn base_and_upstream_diverge_independently() {
        let (guard, repo) = fixture_repo();
        sh(&repo, &["checkout", "-b", "feature/x"]);
        commit_file(&repo, "feature.txt");

        let before = status(&repo, "main", None, None);
        assert_eq!(before.upstream, None);
        assert_eq!((before.base, before.unpushed), (divergence(1, 0), 1));

        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "feature/x"]);

        let after = status(&repo, "main", None, None);
        assert_eq!(after.upstream, upstream("origin/feature/x", 0, 0));
        assert_eq!((after.base, after.unpushed), (divergence(1, 0), 0));

        sh(&repo, &["checkout", "main"]);
        commit_file(&repo, "main-1.txt");
        commit_file(&repo, "main-2.txt");
        sh(&repo, &["checkout", "feature/x"]);

        let moved = status(&repo, "main", None, None);
        assert_eq!(moved.base, divergence(1, 2));
        assert_eq!(moved.upstream, upstream("origin/feature/x", 0, 0));
    }

    #[tokio::test]
    async fn a_fresh_workstream_off_origin_main_has_no_upstream_until_pushed() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "main"]);
        sh(&repo, &["push", "origin", "main:release"]);
        let wt_root = tempfile::tempdir().unwrap();
        let git = which::which("git").expect("git on PATH");

        let created = WorktreeManager::new(wt_root.path().to_path_buf())
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "Fresh".into(),
                    base_override: Some("origin/main".into()),
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let wt = PathBuf::from(&created.path);
        let configured = std::process::Command::new(&git)
            .arg("-C")
            .arg(&wt)
            .args(["rev-parse", "--abbrev-ref"])
            .arg(format!("{}@{{upstream}}", created.branch))
            .output()
            .unwrap();
        assert!(
            !configured.status.success(),
            "the new branch tracks nothing"
        );
        assert_eq!(status(&wt, "origin/main", None, None).upstream, None);
        assert_eq!(status(&wt, "origin/release", None, None).upstream, None);
        assert!(pull_upstream(&git, &wt, "origin/release", NO_OVERLAY)
            .await
            .is_err());

        sh(&wt, &["push", "-u", "origin", &created.branch]);
        assert_eq!(
            status(&wt, "origin/main", None, None).upstream,
            upstream(&format!("origin/{}", created.branch), 0, 0)
        );
    }

    #[test]
    fn a_branch_renamed_after_push_keeps_its_upstream() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["checkout", "-b", "hyprlayer/feat"]);
        commit_file(&repo, "feature.txt");
        sh(&repo, &["push", "-u", "origin", "hyprlayer/feat"]);
        sh(&repo, &["branch", "-m", "feature/renamed"]);

        let st = status(&repo, "main", None, None);
        assert_eq!(st.upstream, upstream("origin/hyprlayer/feat", 0, 0));
        assert_eq!(st.unpushed, 0);
        ensure_removable(&repo, "main", "feature/renamed").unwrap();
    }

    #[test]
    fn a_push_without_u_stays_pushed_after_a_rename_or_to_another_remote() {
        let (guard, repo) = fixture_repo();
        for name in ["origin", "fork"] {
            let remote = guard.path().join(format!("{name}.git"));
            sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
            sh(&repo, &["remote", "add", name, remote.to_str().unwrap()]);
        }
        sh(&repo, &["push", "-u", "origin", "main"]);
        sh(&repo, &["checkout", "-b", "hyprlayer/feat"]);
        commit_file(&repo, "feature.txt");
        sh(&repo, &["push", "origin", "HEAD"]);
        sh(&repo, &["branch", "-m", "feature/renamed"]);

        assert_eq!(status(&repo, "origin/main", None, None).unpushed, 0);
        ensure_removable(&repo, "origin/main", "feature/renamed").unwrap();

        sh(&repo, &["checkout", "-b", "forked"]);
        commit_file(&repo, "forked.txt");
        sh(&repo, &["push", "fork", "HEAD"]);
        assert_eq!(status(&repo, "origin/main", None, None).unpushed, 0);

        commit_file(&repo, "local.txt");
        assert_eq!(status(&repo, "origin/main", None, None).unpushed, 1);
    }

    #[test]
    fn unpushed_matches_git_wherever_the_commits_live() {
        let git_count = |repo: &Path| -> u32 {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["rev-list", "--count", "HEAD", "--not", "--remotes"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
        };
        let with_origin = || {
            let (guard, repo) = fixture_repo();
            let remote = guard.path().join("origin.git");
            sh(
                &repo,
                &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
            );
            sh(
                &repo,
                &["remote", "add", "origin", remote.to_str().unwrap()],
            );
            sh(&repo, &["push", "-u", "origin", "main"]);
            let teammate = guard.path().join("teammate");
            sh(
                guard.path(),
                &[
                    "clone",
                    remote.to_str().unwrap(),
                    teammate.to_str().unwrap(),
                ],
            );
            (guard, repo, remote, teammate)
        };
        let unpushed = |repo: &Path, base: &str| {
            let counted = status(repo, base, None, None).unpushed;
            assert_eq!(counted, git_count(repo), "base {base}");
            counted
        };

        let (_guard, repo, _remote, teammate) = with_origin();
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "a.txt");
        commit_file(&repo, "b.txt");
        sh(&repo, &["push", "origin", "HEAD:review"]);
        sh(&teammate, &["fetch", "origin"]);
        sh(&teammate, &["checkout", "-b", "review", "origin/review"]);
        commit_file(&teammate, "theirs.txt");
        sh(&teammate, &["push", "origin", "review"]);
        sh(&repo, &["fetch", "origin"]);
        assert_eq!(unpushed(&repo, "origin/main"), 0, "a teammate built on it");

        let (guard, repo, _remote, _teammate) = with_origin();
        let fork = guard.path().join("fork.git");
        sh(&repo, &["init", "--bare", fork.to_str().unwrap()]);
        sh(&repo, &["remote", "add", "fork", fork.to_str().unwrap()]);
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "a.txt");
        sh(&repo, &["push", "-u", "origin", "feat"]);
        commit_file(&repo, "b.txt");
        commit_file(&repo, "c.txt");
        sh(&repo, &["push", "fork", "HEAD:feat"]);
        assert_eq!(unpushed(&repo, "origin/main"), 0, "the rest is on a fork");

        let (_guard, repo, remote, teammate) = with_origin();
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "mine.txt");
        sh(&teammate, &["push", "origin", "main:trunk"]);
        sh(&remote, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
        sh(&teammate, &["push", "origin", ":main"]);
        sh(&repo, &["fetch", "--prune", "origin"]);
        assert_eq!(unpushed(&repo, "origin/main"), 1, "the base was renamed");

        let (_guard, repo, _remote, _teammate) = with_origin();
        commit_file(&repo, "l1.txt");
        commit_file(&repo, "l2.txt");
        assert_eq!(unpushed(&repo, "main"), 2, "HEAD is on the base");
        assert_eq!(unpushed(&repo, "origin/main"), 2);
    }

    fn head_of(repo: &Path) -> String {
        let repo = git2::Repository::open(repo).unwrap();
        let head = repo.head().unwrap().target().unwrap();
        head.to_string()
    }

    #[test]
    fn a_squash_merged_branch_counts_only_work_past_its_merged_pr() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(
            &repo,
            &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "main"]);
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "one.txt");
        commit_file(&repo, "two.txt");
        sh(&repo, &["push", "-u", "origin", "feat"]);
        let merged = head_of(&repo);
        let teammate = guard.path().join("teammate");
        sh(
            guard.path(),
            &[
                "clone",
                remote.to_str().unwrap(),
                teammate.to_str().unwrap(),
            ],
        );
        sh(&teammate, &["merge", "--squash", "origin/feat"]);
        sh(&teammate, &["commit", "-m", "squash"]);
        sh(&teammate, &["push", "origin", "main", ":feat"]);
        sh(&repo, &["fetch", "--prune", "origin"]);

        assert_eq!(status(&repo, "origin/main", None, None).unpushed, 2);
        assert_eq!(
            status(&repo, "origin/main", Some(&merged), None).unpushed,
            0
        );
        commit_file(&repo, "after.txt");
        assert_eq!(
            status(&repo, "origin/main", Some(&merged), None).unpushed,
            1
        );
        assert_eq!(
            scan_archive(&repo, "origin/main", Some(&merged)).unpushed,
            Some(1)
        );
    }

    #[test]
    fn only_a_remote_ref_that_is_not_the_base_counts_as_pushed() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("upstream.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "upstream", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "upstream", "main"]);

        sh(&repo, &["checkout", "-b", "off-upstream", "upstream/main"]);
        commit_file(&repo, "off-upstream.txt");
        assert_eq!(status(&repo, "main", None, None).upstream, None);

        sh(&repo, &["checkout", "-b", "dev", "main"]);
        commit_file(&repo, "dev.txt");
        sh(&repo, &["checkout", "-b", "feat"]);
        sh(&repo, &["branch", "--set-upstream-to=dev"]);
        assert_eq!(status(&repo, "main", None, None).upstream, None);
        sh(&repo, &["branch", "--set-upstream-to=main"]);
        assert_eq!(status(&repo, "upstream/main", None, None).upstream, None);
    }

    #[tokio::test]
    async fn pull_merges_the_remote_ref_when_a_local_branch_shares_its_name() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "main"]);
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "pushed.txt");
        sh(&repo, &["push", "-u", "origin", "feat"]);
        sh(&repo, &["reset", "--hard", "main"]);
        sh(&repo, &["checkout", "-b", "origin/feat"]);
        commit_file(&repo, "decoy.txt");
        sh(&repo, &["checkout", "feat"]);

        let git = which::which("git").expect("git on PATH");
        pull_upstream(&git, &repo, "main", NO_OVERLAY)
            .await
            .unwrap();
        let rev = |r: &str| {
            std::process::Command::new(&git)
                .arg("-C")
                .arg(&repo)
                .args(["rev-parse", r])
                .output()
                .unwrap()
                .stdout
        };
        assert_eq!(rev("HEAD"), rev("refs/remotes/origin/feat"));
    }

    #[tokio::test]
    async fn fetch_remote_fetches_the_remote_the_base_lives_on() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("upstream.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "upstream", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "upstream", "main"]);
        let teammate = guard.path().join("teammate");
        sh(
            guard.path(),
            &[
                "clone",
                "-b",
                "main",
                remote.to_str().unwrap(),
                teammate.to_str().unwrap(),
            ],
        );
        commit_file(&teammate, "theirs.txt");
        sh(&teammate, &["push", "origin", "main"]);

        let git = which::which("git").expect("git on PATH");
        fetch_remote(&git, &repo, Some("upstream/main"), NO_OVERLAY)
            .await
            .unwrap();
        assert_eq!(status(&repo, "upstream/main", None, None).base.behind, 1);

        let (_other_guard, no_remote) = fixture_repo();
        fetch_remote(&git, &no_remote, None, NO_OVERLAY)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_local_base_counts_what_update_from_base_merges() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(
            &repo,
            &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "main"]);
        sh(&repo, &["checkout", "-b", "feat"]);
        let teammate = guard.path().join("teammate");
        sh(
            guard.path(),
            &[
                "clone",
                remote.to_str().unwrap(),
                teammate.to_str().unwrap(),
            ],
        );
        commit_file(&teammate, "theirs-1.txt");
        commit_file(&teammate, "theirs-2.txt");
        sh(&teammate, &["push", "origin", "main"]);
        sh(&repo, &["fetch", "origin"]);

        assert_eq!(status(&repo, "main", None, None).base, divergence(0, 2));
        let git = which::which("git").expect("git on PATH");
        let pulled = pull_base(&git, &repo, "main", NO_OVERLAY).await.unwrap();
        assert!(matches!(pulled, PullOutcome::Merged), "{pulled:?}");
        assert_eq!(status(&repo, "main", None, None).base, divergence(0, 0));
    }

    #[test]
    fn push_u_to_another_name_counts_that_ref() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "feature.txt");
        sh(&repo, &["push", "-u", "origin", "HEAD:other"]);

        let st = status(&repo, "main", None, None);
        assert_eq!(st.upstream, upstream("origin/other", 0, 0));
        assert_eq!(st.unpushed, 0);
    }

    #[tokio::test]
    async fn a_teammate_push_to_the_branch_is_behind_upstream_not_base() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "origin", "main"]);
        sh(&repo, &["checkout", "-b", "feat"]);
        commit_file(&repo, "feature.txt");
        sh(&repo, &["push", "-u", "origin", "feat"]);

        let teammate = guard.path().join("teammate");
        sh(
            guard.path(),
            &[
                "clone",
                "-b",
                "feat",
                remote.to_str().unwrap(),
                teammate.to_str().unwrap(),
            ],
        );
        commit_file(&teammate, "theirs.txt");
        sh(&teammate, &["push", "origin", "feat"]);
        sh(&repo, &["fetch", "origin"]);

        let st = status(&repo, "main", None, None);
        assert_eq!(st.upstream, upstream("origin/feat", 0, 1));
        assert_eq!(st.base, divergence(1, 0));

        let git = which::which("git").expect("git on PATH");
        let pulled = pull_upstream(&git, &repo, "main", NO_OVERLAY)
            .await
            .unwrap();
        assert!(matches!(pulled, PullOutcome::Merged), "{pulled:?}");
        assert_eq!(
            status(&repo, "main", None, None).upstream,
            upstream("origin/feat", 0, 0)
        );
    }

    #[tokio::test]
    async fn pull_follows_the_counted_ref_when_pushed_without_u() {
        let (guard, repo) = fixture_repo();
        let remote = guard.path().join("remote.git");
        sh(&repo, &["init", "--bare", remote.to_str().unwrap()]);
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-u", "origin", "main"]);
        let wt_root = tempfile::tempdir().unwrap();
        let git = which::which("git").expect("git on PATH");
        let created = WorktreeManager::new(wt_root.path().to_path_buf())
            .create(
                &git,
                &CreateSpec {
                    project_repo: repo.clone(),
                    mapped_name: "proj".into(),
                    title: "No u".into(),
                    base_override: Some("origin/main".into()),
                    branch_prefix: "hyprlayer/".into(),
                    mode: CreateMode::Isolated,
                },
            )
            .await
            .unwrap();
        let (wt, branch) = (PathBuf::from(&created.path), created.branch);
        commit_file(&wt, "feature.txt");
        sh(&wt, &["push", "origin", "HEAD"]);

        let teammate = guard.path().join("teammate");
        sh(
            guard.path(),
            &[
                "clone",
                "-b",
                &branch,
                remote.to_str().unwrap(),
                teammate.to_str().unwrap(),
            ],
        );
        commit_file(&teammate, "theirs.txt");
        sh(&teammate, &["push", "origin", &branch]);
        let theirs = std::process::Command::new(&git)
            .arg("-C")
            .arg(&teammate)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        sh(&teammate, &["checkout", "main"]);
        commit_file(&teammate, "main-moved.txt");
        sh(&teammate, &["push", "origin", "main"]);
        sh(&wt, &["fetch", "origin"]);

        let counted = format!("origin/{branch}");
        let before = status(&wt, "origin/main", None, None);
        assert_eq!(before.upstream, upstream(&counted, 0, 1));
        assert_eq!(before.base.behind, 1);

        pull_upstream(&git, &wt, "origin/main", NO_OVERLAY)
            .await
            .unwrap();
        let head = std::process::Command::new(&git)
            .arg("-C")
            .arg(&wt)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(head.stdout, theirs.stdout);
        let after = status(&wt, "origin/main", None, None);
        assert_eq!(after.upstream, upstream(&counted, 0, 0));
        assert_eq!(after.base.behind, 1, "the base was not merged");
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
        let st = status(&wt, &created.base_branch, None, None);
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
        assert_eq!(
            status(&repo, "main", None, None).branch.as_deref(),
            Some("main")
        );

        sh(&repo, &["checkout", "-b", "feature/x"]);
        assert_eq!(
            status(&repo, "main", None, None).branch.as_deref(),
            Some("feature/x")
        );

        sh(&repo, &["checkout", "--detach"]);
        assert_eq!(status(&repo, "main", None, None).branch, None);
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
        assert_eq!(
            detect_base_branch(&git, &repo, NO_OVERLAY).await.unwrap(),
            "main"
        );
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
        let st = status(&wt, &created.base_branch, None, None);
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

    /// A fake `git` that only ever echoes an env var, standing in for the
    /// real binary so the assertion is about the overlay reaching the child
    /// process, not about git itself.
    #[cfg(unix)]
    fn fake_git_echoing_ssh_auth_sock(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("fake-git");
        std::fs::write(&path, "#!/bin/sh\nprintf '%s' \"$SSH_AUTH_SOCK\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_overlay_reaches_a_real_git_child_through_git_in() {
        let dir = tempfile::tempdir().unwrap();
        let fake_git = fake_git_echoing_ssh_auth_sock(dir.path());
        // A value no ambient test-runner environment would already have,
        // so a pass can only mean the overlay itself made the trip.
        let overlay = vec![(
            "SSH_AUTH_SOCK".to_string(),
            "/tmp/git-in-overlay-probe.sock".to_string(),
        )];

        let out = git_in(&fake_git, dir.path(), &["status"], &overlay)
            .run()
            .await
            .unwrap();
        assert_eq!(out.stdout, "/tmp/git-in-overlay-probe.sock");
    }

    fn panel_rows(dir: &Path) -> usize {
        let st = crate::gitops::worktree_git_status(dir).unwrap();
        st.staged.len() + st.unstaged.len()
    }

    #[test]
    fn the_chip_counts_what_the_changes_panel_lists() {
        let (_guard, repo) = fixture_repo();
        for f in ["newdir/a.txt", "newdir/b.txt", "newdir/sub/c.txt"] {
            std::fs::create_dir_all(repo.join(f).parent().unwrap()).unwrap();
            std::fs::write(repo.join(f), "x\n").unwrap();
        }
        let st = status(&repo, "main", None, None);
        assert_eq!((st.dirty, st.untracked), (0, 3));
        assert_eq!(panel_rows(&repo), 3);

        std::fs::remove_dir_all(repo.join("newdir")).unwrap();
        sh(&repo, &["mv", "README.md", "README.txt"]);
        let st = status(&repo, "main", None, None);
        assert_eq!((st.dirty, st.untracked), (1, 0), "a rename is one change");
        assert_eq!(panel_rows(&repo), 1);
    }

    #[test]
    fn a_tracked_hyprlayer_file_shows_in_chip_and_panel_and_stages() {
        let (_guard, repo) = fixture_repo();
        std::fs::create_dir_all(repo.join(".hyprlayer")).unwrap();
        std::fs::write(repo.join(".hyprlayer/settings.toml"), "[worktree]\n").unwrap();
        sh(&repo, &["add", ".hyprlayer/settings.toml"]);
        sh(&repo, &["commit", "-m", "repo settings"]);

        std::fs::write(
            repo.join(".hyprlayer/settings.toml"),
            "[worktree]\ncopy = [\".env\"]\n",
        )
        .unwrap();
        std::fs::write(repo.join(".hyprlayer/a.png"), "app internal\n").unwrap();
        let st = status(&repo, "main", None, None);
        assert_eq!((st.dirty, st.untracked), (1, 0));
        assert_eq!((st.additions, st.deletions), (1, 0));
        assert_eq!(panel_rows(&repo), 1);

        crate::gitops::stage_files(
            &repo,
            &[".hyprlayer/settings.toml".into(), ".hyprlayer/a.png".into()],
        )
        .unwrap();
        let staged: Vec<String> = crate::gitops::worktree_git_status(&repo)
            .unwrap()
            .staged
            .into_iter()
            .map(|e| e.path)
            .collect();
        assert_eq!(staged, vec![".hyprlayer/settings.toml"]);
    }

    #[test]
    fn a_sparse_checkout_reads_clean_in_the_chip() {
        let (_guard, repo) = fixture_repo();
        for f in ["apps/web/index.ts", "apps/api/main.rs", "docs/guide.md"] {
            std::fs::create_dir_all(repo.join(f).parent().unwrap()).unwrap();
            std::fs::write(repo.join(f), "x\ny\n").unwrap();
        }
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-m", "apps"]);
        sh(&repo, &["sparse-checkout", "set", "apps/web"]);
        assert!(!repo.join("docs/guide.md").exists(), "control: sparse");

        let st = status(&repo, "main", None, None);
        assert_eq!(
            (st.dirty, st.untracked, st.additions, st.deletions),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn only_a_submodule_pointer_bump_makes_the_worktree_dirty() {
        let (guard, repo) = fixture_repo();
        let lib = guard.path().join("lib");
        std::fs::create_dir(&lib).unwrap();
        sh(&lib, &["init", "-b", "main"]);
        sh(&lib, &["commit", "--allow-empty", "-m", "lib"]);
        sh(
            &repo,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                lib.to_str().unwrap(),
                "vendor/lib",
            ],
        );
        sh(&repo, &["commit", "-m", "add submodule"]);
        let sub = repo.join("vendor/lib");

        std::fs::write(sub.join("scratch.txt"), "wip\n").unwrap();
        let st = status(&repo, "main", None, None);
        assert_eq!((st.dirty, st.untracked), (0, 0));
        ensure_removable(&repo, "main", "main").unwrap();

        sh(&sub, &["commit", "--allow-empty", "-m", "bump"]);
        let sub_head = || {
            git2::Repository::open(&sub)
                .unwrap()
                .head()
                .unwrap()
                .target()
        };
        let bumped = sub_head();
        assert_eq!(status(&repo, "main", None, None).dirty, 1);

        assert_eq!(crate::gitops::discard_tracked_changes(&repo).unwrap(), 0);
        assert_eq!(
            sub_head(),
            bumped,
            "archive leaves the submodule's checkout alone"
        );
    }

    #[tokio::test]
    async fn a_submodule_pointer_bump_does_not_block_an_in_place_switch() {
        let (guard, repo) = fixture_repo();
        let wt_root = tempfile::tempdir().unwrap();
        let manager = WorktreeManager::new(wt_root.path().to_path_buf());
        let git = which::which("git").unwrap();
        let lib = guard.path().join("lib");
        std::fs::create_dir(&lib).unwrap();
        sh(&lib, &["init", "-b", "main"]);
        sh(&lib, &["commit", "--allow-empty", "-m", "lib"]);
        sh(
            &repo,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                lib.to_str().unwrap(),
                "vendor/lib",
            ],
        );
        sh(&repo, &["commit", "-m", "add submodule"]);
        sh(&repo, &["branch", "other"]);
        sh(
            &repo.join("vendor/lib"),
            &["commit", "--allow-empty", "-m", "newer lib commit"],
        );

        let created = manager
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
            .unwrap();
        assert_eq!(created.branch, "other");
        assert_eq!(
            crate::gitops::current_branch(&repo).as_deref(),
            Some("other")
        );
    }
}
