use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsRepoStatus {
    pub branch: Option<String>,
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
    pub has_remote: bool,
}

pub(crate) fn open_repo(path: &Path) -> Result<git2::Repository> {
    git2::Repository::open(path).map_err(|e| CoreError::OutputParse {
        program: "git2 open".into(),
        detail: format!("{}: {}", path.display(), e.message()),
    })
}

pub fn git_dir(worktree: &Path) -> Option<PathBuf> {
    Some(open_repo(worktree).ok()?.path().to_path_buf())
}

/// A linked worktree's git dir names the shared one in its `commondir` file; git2 0.19 has no accessor.
pub(crate) fn common_dir(repo: &git2::Repository) -> PathBuf {
    let git_dir = repo.path();
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(shared) => git_dir.join(shared.trim()),
        Err(_) => git_dir.to_path_buf(),
    }
}

mod renames;

pub(crate) fn status_options() -> git2::StatusOptions {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .exclude_submodules(false);
    opts
}

const WT_ANY: git2::Status = git2::Status::WT_NEW
    .union(git2::Status::WT_MODIFIED)
    .union(git2::Status::WT_DELETED)
    .union(git2::Status::WT_TYPECHANGE)
    .union(git2::Status::WT_RENAMED);

pub(crate) struct Change {
    pub(crate) path: String,
    pub(crate) status: git2::Status,
    pub(crate) renamed_from: Option<String>,
    pub(crate) copied_from: Option<String>,
    pub(crate) gitlink: bool,
    head: Option<Side>,
    staged: Option<Side>,
}

type Side = (git2::Oid, u32);

impl Change {
    fn unstaged(&self) -> bool {
        self.status.intersects(WT_ANY)
    }

    fn old_path(&self) -> Option<&str> {
        self.renamed_from.as_deref().or(self.copied_from.as_deref())
    }

    pub(crate) fn tracked(&self) -> bool {
        !(self.status - git2::Status::WT_NEW).is_empty()
    }
}

fn is_app_internal(path: &str) -> bool {
    use crate::repo_settings::{HYPRLAYER_DIR, SETTINGS_FILE};
    is_hyprlayer_internal(path)
        && path
            .strip_prefix(HYPRLAYER_DIR)
            .and_then(|p| p.strip_prefix('/'))
            != Some(SETTINGS_FILE)
}

fn checked_out_head(worktree: &Path, rel: &str) -> Option<git2::Oid> {
    git2::Repository::open(worktree.join(rel))
        .ok()?
        .head()
        .ok()?
        .target()
}

fn submodule_head_moved(worktree: Option<&Path>, rel: &str, recorded: Option<git2::Oid>) -> bool {
    worktree
        .and_then(|w| checked_out_head(w, rel))
        .is_some_and(|head| Some(head) != recorded)
}

fn only_dirty_content(delta: &git2::DiffDelta<'_>) -> bool {
    let gitlink = delta.new_file().mode() == git2::FileMode::Commit
        || delta.old_file().mode() == git2::FileMode::Commit;
    gitlink
        && delta.status() == git2::Delta::Modified
        && delta.old_file().id() == delta.new_file().id()
}

fn is_skip_worktree(index: Option<&git2::Index>, rel: &str) -> bool {
    index
        .and_then(|i| i.get_path(Path::new(rel), 0))
        .is_some_and(|e| {
            git2::IndexEntryExtendedFlag::from_bits_truncate(e.flags_extended).is_skip_worktree()
        })
}

pub(crate) fn read_changes(repo: &git2::Repository, show: git2::StatusShow) -> Result<Vec<Change>> {
    let mut opts = status_options();
    opts.show(show);
    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| git_err("git2 status", e))?;
    let index = repo.index().ok();
    let worktree = repo.workdir().map(Path::to_path_buf);
    let side = |f: git2::DiffFile<'_>| (f.id(), u32::from(f.mode()));

    let mut changes = Vec::new();
    for entry in statuses.iter() {
        let Some(path) = entry.path() else {
            continue;
        };
        if crate::worktree::is_thoughts_path(path) {
            continue;
        }
        let mut status = entry.status();
        let to_index = entry.head_to_index();
        let to_workdir = entry.index_to_workdir();
        let head_side = to_index
            .as_ref()
            .filter(|_| !status.contains(git2::Status::INDEX_NEW))
            .map(|d| side(d.old_file()));
        let index_side = match &to_index {
            Some(d) if !status.contains(git2::Status::INDEX_DELETED) => Some(side(d.new_file())),
            _ => to_workdir
                .as_ref()
                .filter(|_| !status.contains(git2::Status::WT_NEW))
                .map(|d| side(d.old_file())),
        };
        let gitlink = [head_side, index_side]
            .iter()
            .flatten()
            .any(|&(_, mode)| mode == u32::from(git2::FileMode::Commit));
        if is_app_internal(path) {
            status.remove(git2::Status::WT_NEW);
        }
        if status.contains(git2::Status::WT_DELETED) && is_skip_worktree(index.as_ref(), path) {
            status.remove(git2::Status::WT_DELETED);
        }
        if gitlink
            && status.contains(git2::Status::WT_MODIFIED)
            && !submodule_head_moved(worktree.as_deref(), path, index_side.map(|(oid, _)| oid))
        {
            status.remove(git2::Status::WT_MODIFIED);
        }
        if !status.is_empty() {
            let intent_to_add = status.contains(git2::Status::INDEX_NEW)
                && index
                    .as_ref()
                    .and_then(|i| i.get_path(Path::new(path), 0))
                    .is_some_and(|e| {
                        git2::IndexEntryExtendedFlag::from_bits_truncate(e.flags_extended)
                            .is_intent_to_add()
                    });
            changes.push(Change {
                path: path.to_string(),
                status,
                renamed_from: None,
                copied_from: None,
                gitlink,
                head: head_side.filter(|(oid, _)| !oid.is_zero()),
                staged: index_side.filter(|_| !intent_to_add),
            });
        }
    }
    pair_renames(repo, &mut changes);
    Ok(changes)
}

fn pair_renames(repo: &git2::Repository, changes: &mut Vec<Change>) {
    let Some(detect) = renames::rename_detection(repo) else {
        return;
    };
    let mut source_flags = git2::Status::INDEX_DELETED;
    if detect.copies {
        source_flags |= git2::Status::INDEX_MODIFIED | git2::Status::INDEX_TYPECHANGE;
    }
    let collect =
        |flags: git2::Status, side: fn(&Change) -> Option<Side>| -> Vec<(usize, git2::Oid, u32)> {
            changes
                .iter()
                .enumerate()
                .filter(|(_, c)| c.status.intersects(flags))
                .filter_map(|(i, c)| side(c).map(|(oid, mode)| (i, oid, mode)))
                .collect()
        };
    let sources = collect(source_flags, |c| c.head);
    let targets = collect(git2::Status::INDEX_NEW, |c| c.staged);
    if sources.is_empty() || targets.is_empty() {
        return;
    }
    let sides = |list: &[(usize, git2::Oid, u32)]| -> Vec<renames::Side<'_>> {
        list.iter()
            .map(|&(i, oid, mode)| renames::Side {
                path: &changes[i].path,
                oid,
                mode,
                stays: !changes[i].status.contains(git2::Status::INDEX_DELETED),
            })
            .collect()
    };
    let pairs = renames::pair(repo, &sides(&sources), &sides(&targets), &detect);
    for pair in pairs {
        let (source, target) = (sources[pair.src].0, targets[pair.dst].0);
        let from = changes[source].path.clone();
        changes[target].head = changes[source].head;
        if pair.copy {
            changes[target].copied_from = Some(from);
            continue;
        }
        changes[source].status.remove(git2::Status::INDEX_DELETED);
        let target = &mut changes[target];
        target.status.remove(git2::Status::INDEX_NEW);
        target.status.insert(git2::Status::INDEX_RENAMED);
        target.renamed_from = Some(from);
    }
    changes.retain(|c| !c.status.is_empty());
}

fn delta_path(delta: &git2::DiffDelta<'_>) -> String {
    delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn hides_delta(index: Option<&git2::Index>, delta: &git2::DiffDelta<'_>) -> bool {
    let path = delta_path(delta);
    crate::worktree::is_thoughts_path(&path)
        || (is_app_internal(&path) && delta.status() == git2::Delta::Untracked)
        || (delta.status() == git2::Delta::Deleted && is_skip_worktree(index, &path))
        || only_dirty_content(delta)
}

pub(crate) fn upstream_oid(repo: &git2::Repository, head: &git2::Reference) -> Option<git2::Oid> {
    head.shorthand()
        .and_then(|name| repo.find_branch(name, git2::BranchType::Local).ok())
        .and_then(|b| b.upstream().ok())
        .and_then(|u| u.get().target())
}

pub(crate) fn remote_ref_parts(repo: &git2::Repository, refname: &str) -> Option<(String, String)> {
    let remote = repo.branch_remote_name(refname).ok()?;
    let remote = remote.as_str()?;
    let branch = refname.strip_prefix(&format!("refs/remotes/{remote}/"))?;
    Some((remote.to_string(), branch.to_string()))
}

fn is_base_ref(repo: &git2::Repository, upstream: &git2::Reference, base: &str) -> bool {
    let base_branch = remote_ref_parts(repo, &format!("refs/remotes/{base}"))
        .map_or_else(|| base.to_string(), |(_, branch)| branch);
    upstream
        .name()
        .and_then(|name| remote_ref_parts(repo, name))
        .is_some_and(|(_, branch)| branch == base_branch)
}

pub(crate) fn pushed_ref<'r>(
    repo: &'r git2::Repository,
    branch: &str,
    base: &str,
) -> Option<git2::Reference<'r>> {
    repo.find_branch(branch, git2::BranchType::Local)
        .ok()
        .and_then(|b| b.upstream().ok())
        .map(git2::Branch::into_reference)
        .filter(|r| r.is_remote() && !is_base_ref(repo, r, base))
        .or_else(|| {
            repo.find_reference(&format!("refs/remotes/origin/{branch}"))
                .ok()
        })
}

pub(crate) fn untrack_base(dir: &Path, branch: &str, base: &str) -> Result<bool> {
    let repo = open_repo(dir)?;
    let Ok(mut local) = repo.find_branch(branch, git2::BranchType::Local) else {
        return Ok(false);
    };
    let tracks_base = local
        .upstream()
        .is_ok_and(|u| is_base_ref(&repo, u.get(), base));
    if tracks_base {
        local
            .set_upstream(None)
            .map_err(|e| git_err("git2 branch --unset-upstream", e))?;
    }
    Ok(tracks_base)
}

pub(crate) fn fetch_remote_for(dir: &Path, base: Option<&str>) -> Option<String> {
    let repo = open_repo(dir).ok()?;
    let configured = |base: &str| {
        remote_ref_parts(&repo, &format!("refs/remotes/{base}"))
            .map(|(remote, _)| remote)
            .or_else(|| {
                let remote = repo
                    .branch_upstream_remote(&format!("refs/heads/{base}"))
                    .ok()?;
                remote.as_str().filter(|r| *r != ".").map(String::from)
            })
    };
    base.and_then(configured).or_else(|| {
        repo.find_remote("origin")
            .ok()
            .map(|_| "origin".to_string())
    })
}

fn branch_tip(repo: &git2::Repository, branch: &str) -> Option<git2::Oid> {
    repo.find_branch(branch, git2::BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
        .or_else(|| repo.head().ok()?.target())
}

fn has_object(repo: &git2::Repository, oid: git2::Oid) -> bool {
    repo.odb().is_ok_and(|odb| odb.exists(oid))
}

pub(crate) fn pr_head_is_ours(
    dir: &Path,
    branch: &str,
    base: &str,
    head_oid: &str,
) -> Option<bool> {
    let repo = open_repo(dir).ok()?;
    let oid = git2::Oid::from_str(head_oid).ok()?;
    let contains =
        |tip: git2::Oid| tip == oid || repo.graph_descendant_of(tip, oid).unwrap_or(false);
    let pushed = pushed_ref(&repo, branch, base).and_then(|r| r.target());
    Some(branch_tip(&repo, branch).is_some_and(contains) || pushed.is_some_and(contains))
}

pub(crate) fn local_tip(dir: &Path, branch: &str) -> Option<String> {
    branch_tip(&open_repo(dir).ok()?, branch).map(|o| o.to_string())
}

pub(crate) fn remote_tip(dir: &Path, remote: &str, branch: &str) -> Option<String> {
    let repo = open_repo(dir).ok()?;
    let oid = repo
        .refname_to_id(&format!("refs/remotes/{remote}/{branch}"))
        .ok()?;
    Some(oid.to_string())
}

pub(crate) fn has_work_past(
    dir: &Path,
    branch: &str,
    head_oid: &str,
    tip_then: Option<&str>,
) -> bool {
    let Ok(repo) = open_repo(dir) else {
        return false;
    };
    let Some(tip) = branch_tip(&repo, branch) else {
        return false;
    };
    match git2::Oid::from_str(head_oid)
        .ok()
        .filter(|h| has_object(&repo, *h))
    {
        Some(head) => tip != head && !repo.graph_descendant_of(head, tip).unwrap_or(false),
        None => tip_then.is_some_and(|then| then != tip.to_string()),
    }
}

pub fn list_branches(repo_path: &Path) -> Result<Vec<String>> {
    let repo = open_repo(repo_path)?;
    let branches = repo.branches(None).map_err(|e| CoreError::OutputParse {
        program: "git2 branches".into(),
        detail: e.message().to_string(),
    })?;
    let mut names = Vec::new();
    for branch in branches.flatten() {
        let (branch, _kind) = branch;
        if let Ok(Some(name)) = branch.name() {
            if name.ends_with("/HEAD") {
                continue;
            }
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn list_local_branches(repo_path: &Path) -> Result<Vec<String>> {
    let repo = open_repo(repo_path)?;
    let branches =
        repo.branches(Some(git2::BranchType::Local))
            .map_err(|e| CoreError::OutputParse {
                program: "git2 local branches".into(),
                detail: e.message().to_string(),
            })?;
    let mut names = Vec::new();
    for (branch, _kind) in branches.flatten() {
        if let Ok(Some(name)) = branch.name() {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn list_remote_branches(repo_path: &Path) -> Result<Vec<String>> {
    let repo = open_repo(repo_path)?;
    let branches =
        repo.branches(Some(git2::BranchType::Remote))
            .map_err(|e| CoreError::OutputParse {
                program: "git2 remote branches".into(),
                detail: e.message().to_string(),
            })?;
    let mut names = Vec::new();
    for (branch, _kind) in branches.flatten() {
        if let Ok(Some(name)) = branch.name() {
            if name.ends_with("/HEAD") {
                continue;
            }
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn local_name_for_remote(remote_ref: &str) -> Option<&str> {
    remote_ref
        .split_once('/')
        .map(|(_remote, rest)| rest)
        .filter(|rest| !rest.is_empty())
}

pub fn current_branch(repo_path: &Path) -> Option<String> {
    let repo = open_repo(repo_path).ok()?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().map(|s| s.to_string())
}

/// Linked worktrees on a branch, other than `repo_path` itself.
pub fn branch_worktrees(repo_path: &Path) -> Result<Vec<(String, PathBuf)>> {
    let repo = open_repo(repo_path)?;
    let names = repo.worktrees().map_err(|e| CoreError::OutputParse {
        program: "git2 worktrees".into(),
        detail: e.message().to_string(),
    })?;
    let own = dunce::canonicalize(repo_path).ok();
    let mut found = Vec::new();
    for name in names.iter().flatten() {
        let Ok(wt) = repo.find_worktree(name) else {
            continue;
        };
        if wt.validate().is_err() {
            continue;
        }
        let path = dunce::canonicalize(wt.path()).unwrap_or_else(|_| wt.path().to_path_buf());
        if own.as_ref() == Some(&path) {
            continue;
        }
        // Read HEAD through the `.git` pointer rather than opening a repository per worktree.
        let Some(branch) = linked_head_branch(&path) else {
            continue;
        };
        found.push((branch, path));
    }
    Ok(found)
}

pub fn linked_head_branch(worktree: &Path) -> Option<String> {
    let pointer = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let admin = worktree.join(pointer.trim().strip_prefix("gitdir: ")?);
    let head = std::fs::read_to_string(admin.join("HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
}

/// False when HEAD can't be read, which is not the same as being off a branch.
pub fn linked_head_detached(worktree: &Path) -> bool {
    let Some(pointer) = std::fs::read_to_string(worktree.join(".git")).ok() else {
        return false;
    };
    let Some(admin) = pointer.trim().strip_prefix("gitdir: ") else {
        return false;
    };
    std::fs::read_to_string(worktree.join(admin).join("HEAD"))
        .is_ok_and(|head| !head.starts_with("ref: "))
}

pub fn remote_to_web_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (host, path) = if let Some(rest) = raw
        .strip_prefix("ssh://")
        .or_else(|| raw.strip_prefix("git://"))
        .or_else(|| raw.strip_prefix("https://"))
        .or_else(|| raw.strip_prefix("http://"))
    {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host);
        (host.to_string(), path.to_string())
    } else if let Some((authority, path)) = raw.split_once(':') {
        if path.contains('\\') {
            return None;
        }
        let host = authority.rsplit('@').next().unwrap_or(authority);
        (host.to_string(), path.to_string())
    } else {
        return None;
    };

    if !host.contains('.') {
        return None;
    }
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

fn remotes_of(dir: &Path) -> Option<Vec<(String, String, bool)>> {
    let repo = git2::Repository::discover(dir).ok()?;
    let config = repo.config().ok()?;
    let names = repo.remotes().ok()?;
    let remotes = names
        .iter()
        .flatten()
        .filter_map(|name| {
            let url = repo.find_remote(name).ok()?.url()?.to_string();
            let resolved = config
                .get_string(&format!("remote.{name}.gh-resolved"))
                .is_ok();
            Some((name.to_string(), url, resolved))
        })
        .collect();
    Some(remotes)
}

const NON_GITHUB_FORGES: [&str; 6] = [
    "gitlab.com",
    "bitbucket.org",
    "dev.azure.com",
    "visualstudio.com",
    "codeberg.org",
    "sr.ht",
];

pub(crate) struct GitHubRemote {
    name: String,
    host: String,
    pub(crate) path: String,
}

impl GitHubRemote {
    pub(crate) fn api_host(&self) -> Option<&str> {
        (self.host != "github.com" && !self.host.ends_with(".github.com"))
            .then_some(self.host.as_str())
    }
}

fn web_host_and_path(url: &str) -> Option<(String, String)> {
    let web = remote_to_web_url(url)?;
    let (host, path) = web.strip_prefix("https://")?.split_once('/')?;
    Some((host.to_ascii_lowercase(), path.to_string()))
}

fn on_other_forge(host: &str) -> bool {
    NON_GITHUB_FORGES.iter().any(|forge| {
        host.strip_suffix(forge)
            .is_some_and(|sub| sub.is_empty() || sub.ends_with('.'))
    })
}

fn github_remotes(remotes: &[(String, String, bool)]) -> Vec<GitHubRemote> {
    let rank = |name: &str| match name.to_ascii_lowercase().as_str() {
        "upstream" => 0,
        "github" => 1,
        "origin" => 2,
        _ => 3,
    };
    let mut ranked: Vec<_> = remotes.iter().collect();
    ranked.sort_by_key(|(name, _, resolved)| (!resolved, rank(name)));
    ranked
        .into_iter()
        .filter_map(|(name, url, _)| {
            let (host, path) = web_host_and_path(url)?;
            (!on_other_forge(&host)).then(|| GitHubRemote {
                name: name.clone(),
                host,
                path,
            })
        })
        .collect()
}

pub(crate) fn lookup_remotes(dir: &Path) -> Vec<GitHubRemote> {
    one_per_host(&remotes_of(dir).unwrap_or_default())
}

fn one_per_host(remotes: &[(String, String, bool)]) -> Vec<GitHubRemote> {
    let mut picked: Vec<GitHubRemote> = Vec::new();
    for remote in github_remotes(remotes) {
        if !picked.iter().any(|p| p.api_host() == remote.api_host()) {
            picked.push(remote);
        }
    }
    picked
}

pub(crate) fn has_github_remote(dir: &Path) -> bool {
    remotes_of(dir).is_some_and(|remotes| !github_remotes(&remotes).is_empty())
}

pub(crate) fn base_remote_web_url(dir: &Path) -> Option<String> {
    let remotes = remotes_of(dir)?;
    let base = github_remotes(&remotes).into_iter().next()?;
    Some(format!("https://{}/{}", base.host, base.path))
}

pub(crate) fn fork_owner(dir: &Path, branch: &str, base: &GitHubRemote) -> Option<String> {
    let repo = git2::Repository::discover(dir).ok()?;
    let config = repo.config().ok()?;
    let get = |key: &str| config.get_string(key).ok();
    // gh's `@{push}` resolves with no push config only under push.default current or matching,
    // and only once `refs/remotes/origin/<branch>` exists.
    let pushes_by_name = matches!(get("push.default").as_deref(), Some("current" | "matching"));
    let push = get(&format!("branch.{branch}.pushRemote"))
        .or_else(|| get("remote.pushDefault"))
        .or_else(|| get(&format!("branch.{branch}.remote")))
        .or_else(|| {
            let pushed = repo
                .find_reference(&format!("refs/remotes/origin/{branch}"))
                .is_ok();
            (pushes_by_name && pushed).then(|| "origin".to_string())
        })?;
    let (host, path) = web_host_and_path(repo.find_remote(&push).ok()?.url()?)?;
    (push != base.name && host == base.host && !path.eq_ignore_ascii_case(&base.path))
        .then(|| path.split('/').next().unwrap_or_default().to_string())
}

pub fn file_web_url(file_abs: &Path) -> Option<String> {
    let file_abs = file_abs.canonicalize().ok()?;
    let start = file_abs.parent().unwrap_or(&file_abs);
    let repo = git2::Repository::discover(start).ok()?;
    let workdir = repo.workdir()?.canonicalize().ok()?;
    let rel = file_abs
        .strip_prefix(&workdir)
        .ok()?
        .to_str()?
        .replace('\\', "/");
    if rel.is_empty() {
        return None;
    }
    let web = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().map(str::to_string))
        .as_deref()
        .and_then(remote_to_web_url)?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let branch = head.shorthand()?;
    Some(format!("{web}/blob/{branch}/{rel}"))
}

pub fn thoughts_repo_status(repo_path: &Path) -> Result<ThoughtsRepoStatus> {
    let repo = open_repo(repo_path)?;

    let dirty = repo
        .statuses(Some(&mut status_options()))
        .map(|s| s.len() as u32)
        .unwrap_or(0);

    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .and_then(|h| h.shorthand())
        .map(|s| s.to_string());

    let (ahead, behind, has_remote) = (|| -> Option<(u32, u32, bool)> {
        let head_ref = head.as_ref()?;
        let local_oid = head_ref.target()?;
        let upstream = upstream_oid(&repo, head_ref)?;
        let (a, b) = repo.graph_ahead_behind(local_oid, upstream).ok()?;
        Some((a as u32, b as u32, true))
    })()
    .unwrap_or((0, 0, false));

    Ok(ThoughtsRepoStatus {
        branch,
        dirty,
        ahead,
        behind,
        has_remote,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub change: String,
    pub patch: String,
}

fn is_hyprlayer_internal(path: &str) -> bool {
    path == ".hyprlayer" || path.starts_with(".hyprlayer/")
}

fn worktree_diff_raw(repo: &git2::Repository) -> Result<git2::Diff<'_>> {
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))
        .map_err(|e| CoreError::OutputParse {
            program: "git2 diff".into(),
            detail: e.message().to_string(),
        })
}

pub(crate) fn worktree_diff_line_counts(repo: &git2::Repository, changes: &[Change]) -> (u32, u32) {
    let Ok(diff) = worktree_diff_raw(repo) else {
        return (0, 0);
    };
    let index = repo.index().ok();
    let mut against_head: Vec<(Option<Side>, &str)> = Vec::new();
    let mut skipped: HashSet<&str> = HashSet::new();
    for c in changes {
        let edited = c.status.intersects(WT_ANY);
        match (c.old_path(), c.head) {
            (Some(from), Some(head)) => {
                if edited && !renames::still_similar(repo, from, head, &c.path) {
                    continue;
                }
                skipped.extend(c.renamed_from.as_deref());
                if edited || c.staged.map(|(oid, _)| oid) != Some(head.0) {
                    against_head.push((Some(head), &c.path));
                }
            }
            (None, head) if c.status.contains(git2::Status::CONFLICTED) => {
                against_head.push((head, &c.path));
            }
            _ => continue,
        }
        skipped.insert(&c.path);
    }

    let tally = |diff: &git2::Diff<'_>, skipped: &HashSet<&str>| -> (usize, usize) {
        diff.deltas()
            .enumerate()
            .filter(|(_, d)| {
                !hides_delta(index.as_ref(), d) && !skipped.contains(delta_path(d).as_str())
            })
            .filter_map(|(i, _)| git2::Patch::from_diff(diff, i).ok().flatten())
            .filter_map(|patch| patch.line_stats().ok())
            .fold((0, 0), |(a, d), (_, x, y)| (a + x, d + y))
    };
    let (mut additions, mut deletions) = tally(&diff, &skipped);
    if let Some(diff) = diff_from_head_at(repo, &against_head) {
        let (a, d) = tally(&diff, &HashSet::new());
        (additions, deletions) = (additions + a, deletions + d);
    }
    (additions as u32, deletions as u32)
}

fn diff_from_head_at<'r>(
    repo: &'r git2::Repository,
    entries: &[(Option<Side>, &str)],
) -> Option<git2::Diff<'r>> {
    if entries.is_empty() {
        return None;
    }
    let mut index = git2::Index::new().ok()?;
    let mut opts = git2::DiffOptions::new();
    opts.disable_pathspec_match(true)
        .include_untracked(true)
        .show_untracked_content(true);
    let zero = git2::IndexTime::new(0, 0);
    for &(head, path) in entries {
        opts.pathspec(path);
        let Some((id, mode)) = head else {
            continue;
        };
        index
            .add(&git2::IndexEntry {
                ctime: zero,
                mtime: zero,
                dev: 0,
                ino: 0,
                mode,
                uid: 0,
                gid: 0,
                file_size: 0,
                id,
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            })
            .ok()?;
    }
    repo.diff_index_to_workdir(Some(&index), Some(&mut opts))
        .ok()
}

pub fn worktree_diff(worktree: &Path) -> Result<Vec<FileDiff>> {
    let repo = open_repo(worktree)?;
    let diff = worktree_diff_raw(&repo)?;

    let mut files = Vec::new();
    for (idx, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if crate::worktree::is_thoughts_path(&path) || is_hyprlayer_internal(&path) {
            continue;
        }
        let patch = git2::Patch::from_diff(&diff, idx)
            .ok()
            .flatten()
            .and_then(|mut p| p.to_buf().ok())
            .map(|buf| String::from_utf8_lossy(&buf).to_string())
            .unwrap_or_default();
        files.push(FileDiff {
            path,
            change: delta_change(delta.status()).to_string(),
            patch,
        });
    }
    Ok(files)
}

const MAX_PROMPT_DIFF_CHARS: usize = 12_000;

#[derive(Debug, Clone, Default)]
pub struct PromptGitVars {
    pub changed_files: String,
    pub diff: String,
    pub last_commit: String,
}

pub fn prompt_git_vars(
    worktree: &Path,
    want_changed: bool,
    want_diff: bool,
    want_last: bool,
) -> PromptGitVars {
    let mut vars = PromptGitVars::default();
    let Ok(repo) = open_repo(worktree) else {
        return vars;
    };
    if want_last {
        vars.last_commit = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .and_then(|c| c.summary().map(str::to_string))
            .unwrap_or_default();
    }
    if want_changed {
        vars.changed_files = match read_changes(&repo, git2::StatusShow::IndexAndWorkdir) {
            Ok(changes) => {
                let (adds, dels) = worktree_diff_line_counts(&repo, &changes);
                let n = changes.len();
                let plural = if n == 1 { "" } else { "s" };
                format!("{n} file{plural} (+{adds} −{dels})")
            }
            Err(e) => format!("unknown: {e}"),
        };
    }
    if want_diff {
        let files = worktree_diff(worktree).unwrap_or_default();
        let mut out = String::new();
        for f in &files {
            if out.len() + f.patch.len() > MAX_PROMPT_DIFF_CHARS {
                if out.is_empty() {
                    let mut end = MAX_PROMPT_DIFF_CHARS;
                    while !f.patch.is_char_boundary(end) {
                        end -= 1;
                    }
                    out.push_str(&f.patch[..end]);
                    out.push('\n');
                }
                out.push_str("… (diff truncated)");
                break;
            }
            out.push_str(&f.patch);
        }
        vars.diff = out;
    }
    vars
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GitFileEntry {
    pub path: String,
    pub change: String,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeGitStatus {
    pub staged: Vec<GitFileEntry>,
    pub unstaged: Vec<GitFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FileSides {
    pub original: String,
    pub modified: String,
    pub original_exists: bool,
    pub modified_exists: bool,
    pub original_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

const MAX_COMMITS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CommitEntry {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub time_ms: f64,
}

pub(crate) fn is_excluded(path: &str) -> bool {
    crate::worktree::is_thoughts_path(path) || is_hyprlayer_internal(path)
}

pub(crate) fn git_err(program: &str, e: git2::Error) -> CoreError {
    if e.class() == git2::ErrorClass::Index && e.message().contains("extension: 'sdir'") {
        return CoreError::SparseIndexUnsupported;
    }
    CoreError::OutputParse {
        program: program.to_string(),
        detail: e.message().to_string(),
    }
}

pub(crate) fn safe_join(worktree: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    let unsafe_component = rel_path.is_absolute()
        || rel_path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    if unsafe_component {
        return Err(CoreError::InvalidPath {
            path: rel.to_string(),
        });
    }
    Ok(worktree.join(rel_path))
}

fn first_change(s: git2::Status, flags: &[(git2::Status, &'static str)]) -> Option<&'static str> {
    flags
        .iter()
        .find(|(flag, _)| s.contains(*flag))
        .map(|(_, label)| *label)
}

fn staged_change(s: git2::Status) -> Option<&'static str> {
    first_change(
        s,
        &[
            (git2::Status::INDEX_RENAMED, "renamed"),
            (git2::Status::INDEX_NEW, "added"),
            (git2::Status::INDEX_MODIFIED, "modified"),
            (git2::Status::INDEX_DELETED, "deleted"),
            (git2::Status::INDEX_TYPECHANGE, "typechange"),
        ],
    )
}

fn unstaged_change(s: git2::Status) -> Option<&'static str> {
    first_change(
        s,
        &[
            (git2::Status::CONFLICTED, "conflict"),
            (git2::Status::WT_RENAMED, "renamed"),
            (git2::Status::WT_NEW, "untracked"),
            (git2::Status::WT_MODIFIED, "modified"),
            (git2::Status::WT_DELETED, "deleted"),
            (git2::Status::WT_TYPECHANGE, "typechange"),
        ],
    )
}

fn delta_change(status: git2::Delta) -> &'static str {
    match status {
        git2::Delta::Added | git2::Delta::Untracked => "added",
        git2::Delta::Deleted => "deleted",
        git2::Delta::Renamed => "renamed",
        git2::Delta::Modified => "modified",
        git2::Delta::Typechange => "typechange",
        _ => "other",
    }
}

pub(crate) fn delta_old_path(delta: &git2::DiffDelta<'_>, new_path: &str) -> Option<String> {
    if delta.status() != git2::Delta::Renamed {
        return None;
    }
    let old = delta.old_file().path()?.to_string_lossy().to_string();
    (old != new_path).then_some(old)
}

fn is_tracked(repo: &git2::Repository, rel: &str) -> bool {
    let p = Path::new(rel);
    repo.index().is_ok_and(|i| i.get_path(p, 0).is_some())
        || repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .is_ok_and(|t| t.get_path(p).is_ok())
}

fn excluded_from_actions(repo: &git2::Repository, rel: &str) -> bool {
    crate::worktree::is_thoughts_path(rel) || (is_app_internal(rel) && !is_tracked(repo, rel))
}

fn included_paths(repo: &git2::Repository, rels: &[String]) -> Vec<String> {
    rels.iter()
        .filter(|r| !excluded_from_actions(repo, r))
        .cloned()
        .collect()
}

pub fn worktree_git_status(worktree: &Path) -> Result<WorktreeGitStatus> {
    let repo = open_repo(worktree)?;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    for c in read_changes(&repo, git2::StatusShow::IndexAndWorkdir)? {
        if let Some(change) = staged_change(c.status) {
            let change = if c.copied_from.is_some() {
                "copied"
            } else {
                change
            };
            staged.push(GitFileEntry {
                path: c.path.clone(),
                change: change.to_string(),
                old_path: c.old_path().map(str::to_string),
            });
        }
        if let Some(change) = unstaged_change(c.status) {
            unstaged.push(GitFileEntry {
                path: c.path,
                change: change.to_string(),
                old_path: None,
            });
        }
    }
    Ok(WorktreeGitStatus { staged, unstaged })
}

pub fn stage_files(worktree: &Path, rels: &[String]) -> Result<()> {
    let repo = open_repo(worktree)?;
    let mut index = repo.index().map_err(|e| git_err("git2 index", e))?;
    for rel in rels {
        let rel = rel.trim_end_matches('/');
        if excluded_from_actions(&repo, rel) {
            continue;
        }
        let abs = safe_join(worktree, rel)?;
        let p = Path::new(rel);
        if abs.exists() {
            index.add_path(p).map_err(|e| git_err("git2 add", e))?;
        } else {
            index
                .remove_path(p)
                .map_err(|e| git_err("git2 remove", e))?;
        }
    }
    index.write().map_err(|e| git_err("git2 index write", e))?;
    Ok(())
}

pub fn unstage_files(worktree: &Path, rels: &[String]) -> Result<()> {
    let repo = open_repo(worktree)?;
    let mut owned = included_paths(&repo, rels);
    let sources: Vec<String> = read_changes(&repo, git2::StatusShow::Index)?
        .into_iter()
        .filter(|c| owned.contains(&c.path))
        .filter_map(|c| c.renamed_from)
        .collect();
    owned.extend(sources);
    if owned.is_empty() {
        return Ok(());
    }
    reset_index_to_head(&repo, &owned)
}

fn reset_index_to_head(repo: &git2::Repository, rels: &[String]) -> Result<()> {
    match repo.head().and_then(|h| h.peel(git2::ObjectType::Commit)) {
        Ok(commit_obj) => repo
            .reset_default(Some(&commit_obj), rels.iter().map(|s| s.as_str()))
            .map_err(|e| git_err("git2 reset", e)),
        Err(_) => {
            let mut index = repo.index().map_err(|e| git_err("git2 index", e))?;
            for rel in rels {
                let _ = index.remove_path(Path::new(rel));
            }
            index.write().map_err(|e| git_err("git2 index write", e))
        }
    }
}

fn refuse(path: &str, reason: &str) -> CoreError {
    CoreError::DiscardRefused {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

fn is_nested_repo(abs: &Path) -> bool {
    abs.is_dir() && abs.join(".git").exists()
}

pub fn discard_files(worktree: &Path, rels: &[String]) -> Result<()> {
    let repo = open_repo(worktree)?;
    let changes = read_changes(&repo, git2::StatusShow::IndexAndWorkdir)?;
    let mut picked = Vec::new();
    for rel in included_paths(&repo, rels) {
        safe_join(worktree, &rel)?;
        if let Some(c) = changes.iter().find(|c| c.path == rel) {
            picked.push(c);
        }
    }
    let (unstaged, staged): (Vec<&Change>, Vec<&Change>) =
        picked.into_iter().partition(|c| c.unstaged());
    for c in &unstaged {
        if c.status.contains(git2::Status::WT_NEW) && is_nested_repo(&worktree.join(&c.path)) {
            return Err(refuse(
                &c.path,
                "it is a git repository of its own; delete it by hand if you mean to",
            ));
        }
    }
    check_revert(worktree, &staged)?;

    let index = repo.index().map_err(|e| git_err("git2 index", e))?;
    for c in unstaged.iter().filter(|c| c.gitlink) {
        if let Some(e) = index.get_path(Path::new(&c.path), 0) {
            check_out_gitlink(&repo, worktree, &c.path, e.id)?;
        }
    }
    let restore: Vec<&str> = unstaged
        .iter()
        .filter(|c| !c.gitlink && !c.status.contains(git2::Status::WT_NEW))
        .map(|c| c.path.as_str())
        .collect();
    if !restore.is_empty() {
        let mut index = repo.index().map_err(|e| git_err("git2 index", e))?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        for path in &restore {
            unlink_symlinks(worktree, path)?;
            checkout.path(path);
        }
        repo.checkout_index(Some(&mut index), Some(&mut checkout))
            .map_err(|e| git_err("git2 checkout", e))?;
    }
    for c in unstaged
        .iter()
        .filter(|c| c.status.contains(git2::Status::WT_NEW))
    {
        remove_untracked(worktree, &c.path)?;
    }
    revert_to_head(&repo, worktree, &staged)
}

fn remove_untracked(worktree: &Path, rel: &str) -> Result<()> {
    if symlinked_ancestor(worktree, rel).is_some() {
        return Ok(());
    }
    let abs = &worktree.join(rel);
    if abs.is_dir() {
        std::fs::remove_dir_all(abs)?;
    } else if abs.exists() || abs.is_symlink() {
        std::fs::remove_file(abs)?;
    }
    Ok(())
}

fn symlinked_ancestor(worktree: &Path, rel: &str) -> Option<PathBuf> {
    let mut abs = worktree.to_path_buf();
    let mut parts = Path::new(rel).components().peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        abs.push(part);
        if abs.is_symlink() {
            return Some(abs);
        }
    }
    None
}

// libgit2's forced checkout writes through a symlink at the path or at one of its parents.
fn unlink_symlinks(worktree: &Path, rel: &str) -> Result<()> {
    if let Some(link) = symlinked_ancestor(worktree, rel) {
        std::fs::remove_file(link)?;
    }
    let abs = worktree.join(rel);
    if abs.is_symlink() {
        std::fs::remove_file(abs)?;
    }
    Ok(())
}

fn check_revert(worktree: &Path, changes: &[&Change]) -> Result<()> {
    for c in changes {
        let Some(from) = &c.renamed_from else {
            continue;
        };
        if c.gitlink {
            return Err(refuse(
                &c.path,
                &format!("it is a moved submodule; move it back to {from} with git mv"),
            ));
        }
        let source = worktree.join(from);
        if source.exists() || source.is_symlink() {
            return Err(refuse(
                from,
                "restoring the renamed file would overwrite it; move it aside first",
            ));
        }
    }
    Ok(())
}

fn revert_to_head(repo: &git2::Repository, worktree: &Path, changes: &[&Change]) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let in_head = |p: &str| head.as_ref().and_then(|t| t.get_path(Path::new(p)).ok());

    for c in changes.iter().filter(|c| c.gitlink) {
        if let Some(entry) = in_head(&c.path) {
            check_out_gitlink(repo, worktree, &c.path, entry.id())?;
        }
    }
    let mut paths: Vec<String> = Vec::new();
    let mut dropped: Vec<&str> = Vec::new();
    for c in changes {
        if c.gitlink && in_head(&c.path).is_none() {
            dropped.push(&c.path);
            continue;
        }
        paths.push(c.path.clone());
        paths.extend(c.renamed_from.clone());
    }
    if !dropped.is_empty() {
        let mut index = repo.index().map_err(|e| git_err("git2 index", e))?;
        for path in &dropped {
            index
                .remove_path(Path::new(path))
                .map_err(|e| git_err("git2 remove", e))?;
        }
        index.write().map_err(|e| git_err("git2 index write", e))?;
    }
    if paths.is_empty() {
        return Ok(());
    }
    reset_index_to_head(repo, &paths)?;

    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    let mut any = false;
    for path in &paths {
        match in_head(path) {
            Some(entry) if is_gitlink(&entry) => {}
            Some(_) => {
                unlink_symlinks(worktree, path)?;
                checkout.path(path);
                any = true;
            }
            None => remove_untracked(worktree, path)?,
        }
    }
    if any {
        repo.checkout_head(Some(&mut checkout))
            .map_err(|e| git_err("git2 checkout", e))?;
    }
    Ok(())
}

fn is_gitlink(entry: &git2::TreeEntry<'_>) -> bool {
    entry.filemode() == i32::from(git2::FileMode::Commit)
}

fn check_out_gitlink(
    repo: &git2::Repository,
    worktree: &Path,
    rel: &str,
    oid: git2::Oid,
) -> Result<()> {
    let err = |e| git_err("git2 submodule checkout", e);
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    let sub = match git2::Repository::open(worktree.join(rel)) {
        Ok(sub) => sub,
        Err(_) => {
            checkout.recreate_missing(true);
            match reattach_submodule_checkout(repo, worktree, rel)? {
                Some(sub) => sub,
                None => return Ok(()),
            }
        }
    };
    let commit = sub.find_commit(oid).map_err(err)?;
    sub.checkout_tree(commit.as_object(), Some(&mut checkout))
        .map_err(err)?;
    sub.set_head_detached(oid).map_err(err)
}

fn reattach_submodule_checkout(
    repo: &git2::Repository,
    worktree: &Path,
    rel: &str,
) -> Result<Option<git2::Repository>> {
    let err = |e| git_err("git2 submodule checkout", e);
    let abs = worktree.join(rel);
    let name = repo
        .find_submodule(rel)
        .ok()
        .and_then(|s| s.name().map(str::to_string))
        .unwrap_or_else(|| rel.to_string());
    let gitdir = common_dir(repo).join("modules").join(name);
    std::fs::create_dir_all(&abs)?;
    if !gitdir.is_dir() {
        return Ok(None);
    }
    let up = "../".repeat(Path::new(rel).components().count());
    let pointer = gitdir
        .strip_prefix(worktree)
        .map(|p| format!("{up}{}", p.display()))
        .unwrap_or_else(|_| gitdir.display().to_string());
    std::fs::write(abs.join(".git"), format!("gitdir: {pointer}\n"))?;
    let sub = git2::Repository::open(&gitdir).map_err(err)?;
    sub.set_workdir(&abs, false).map_err(err)?;
    Ok(Some(sub))
}

fn reverts_on_discard(c: &Change) -> bool {
    !c.gitlink && c.tracked() && !c.status.contains(git2::Status::WT_NEW)
}

pub(crate) fn checkout_blockers(worktree: &Path) -> Result<u32> {
    let changes = read_changes(&open_repo(worktree)?, git2::StatusShow::IndexAndWorkdir)?;
    Ok(changes.iter().filter(|c| reverts_on_discard(c)).count() as u32)
}

pub fn discard_tracked_changes(worktree: &Path) -> Result<u32> {
    let repo = open_repo(worktree)?;
    let changes = read_changes(&repo, git2::StatusShow::IndexAndWorkdir)?;
    let picked: Vec<&Change> = changes.iter().filter(|c| reverts_on_discard(c)).collect();
    if picked.is_empty() {
        return Ok(0);
    }
    check_revert(worktree, &picked)?;
    revert_to_head(&repo, worktree, &picked)?;
    Ok(picked.len() as u32)
}

fn subproject_text(oid: git2::Oid) -> String {
    format!("Subproject commit {oid}\n")
}

fn tree_blob_text(repo: &git2::Repository, tree: &git2::Tree, rel: &Path) -> Option<String> {
    let entry = tree.get_path(rel).ok()?;
    if is_gitlink(&entry) {
        return Some(subproject_text(entry.id()));
    }
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

fn head_blob_text(repo: &git2::Repository, rel: &Path) -> Option<String> {
    let tree = repo.head().ok()?.peel_to_tree().ok()?;
    tree_blob_text(repo, &tree, rel)
}

fn index_blob_text(repo: &git2::Repository, rel: &Path) -> Option<String> {
    let index = repo.index().ok()?;
    let entry = index.get_path(rel, 0)?;
    if entry.mode == u32::from(git2::FileMode::Commit) {
        return Some(subproject_text(entry.id));
    }
    let blob = repo.find_blob(entry.id).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

fn workdir_text(repo: &git2::Repository, abs: &Path, rel: &str) -> Option<String> {
    if abs.is_dir() {
        return checked_out_head(repo.workdir()?, rel).map(subproject_text);
    }
    let bytes = std::fs::read(abs).ok()?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

pub fn worktree_file_sides(worktree: &Path, rel: &str, staged: bool) -> Result<FileSides> {
    let abs = safe_join(worktree, rel)?;
    let repo = open_repo(worktree)?;
    let p = Path::new(rel);

    let original_path = if staged {
        read_changes(&repo, git2::StatusShow::Index)?
            .into_iter()
            .find(|c| c.path == rel)
            .and_then(|c| c.old_path().map(str::to_string))
    } else {
        None
    };
    let (original, modified) = if staged {
        let original_p = original_path.as_deref().map_or(p, Path::new);
        (head_blob_text(&repo, original_p), index_blob_text(&repo, p))
    } else {
        let original = index_blob_text(&repo, p).or_else(|| head_blob_text(&repo, p));
        (original, workdir_text(&repo, &abs, rel))
    };

    Ok(FileSides {
        original_exists: original.is_some(),
        modified_exists: modified.is_some(),
        original: original.unwrap_or_default(),
        modified: modified.unwrap_or_default(),
        original_path,
    })
}

pub fn list_dir(worktree: &Path, rel: &str) -> Result<Vec<DirEntry>> {
    let dir = if rel.is_empty() {
        worktree.to_path_buf()
    } else {
        safe_join(worktree, rel)?
    };
    let repo = open_repo(worktree).ok();

    let mut entries = Vec::new();
    for ent in std::fs::read_dir(&dir)? {
        let ent = ent?;
        let name = ent.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if is_excluded(&child_rel) {
            continue;
        }
        if let Some(repo) = repo.as_ref() {
            if repo.is_path_ignored(Path::new(&child_rel)).unwrap_or(false) {
                continue;
            }
        }
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(DirEntry {
            name,
            path: child_rel,
            is_dir,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

pub fn search_files(worktree: &Path, query: &str, limit: usize) -> Result<Vec<String>> {
    const MAX_VISITED: usize = 20_000;

    let repo = open_repo(worktree).ok();
    let needle = query.to_lowercase();

    let mut scored: Vec<(i32, String)> = Vec::new();

    let walker = walkdir::WalkDir::new(worktree)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .flatten()
        .take(MAX_VISITED);

    for entry in walker {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel_path) = entry.path().strip_prefix(worktree) else {
            continue;
        };
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        if is_excluded(&rel) {
            continue;
        }
        if let Some(repo) = repo.as_ref() {
            if repo.is_path_ignored(rel_path).unwrap_or(false) {
                continue;
            }
        }
        if let Some(score) = subsequence_score(&rel, &needle) {
            scored.push((score, rel));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
    scored.truncate(limit);
    Ok(scored.into_iter().map(|(_, p)| p).collect())
}

fn subsequence_score(path: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let lower = path.to_lowercase();
    if let Some(pos) = lower.find(needle) {
        let filename_start = lower.rfind('/').map(|s| s + 1).unwrap_or(0);
        let in_filename = pos >= filename_start;
        return Some(1000 + if in_filename { 500 } else { 0 } - pos as i32);
    }
    let mut chars = needle.chars();
    let mut current = chars.next();
    for c in lower.chars() {
        if Some(c) == current {
            current = chars.next();
            if current.is_none() {
                return Some(100 - path.len() as i32);
            }
        }
    }
    None
}

pub fn read_text_file(worktree: &Path, rel: &str) -> Result<String> {
    let abs = safe_join(worktree, rel)?;
    let meta = std::fs::metadata(&abs)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(CoreError::OutputParse {
            program: "read_file".to_string(),
            detail: format!("file is {} bytes (max {MAX_FILE_BYTES})", meta.len()),
        });
    }
    let bytes = std::fs::read(&abs)?;
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

pub fn file_mtime_ms(worktree: &Path, rel: &str) -> Result<f64> {
    let abs = safe_join(worktree, rel)?;
    let modified = std::fs::metadata(&abs)?.modified()?;
    Ok(modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0))
}

pub fn write_text_file(worktree: &Path, rel: &str, content: &str) -> Result<()> {
    let abs = safe_join(worktree, rel)?;
    let name = abs
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CoreError::InvalidPath {
            path: rel.to_string(),
        })?;
    let tmp = abs.with_file_name(format!(".{name}.hyprtmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &abs)?;
    Ok(())
}

pub fn list_commits(worktree: &Path, base_branch: &str) -> Result<Vec<CommitEntry>> {
    let repo = open_repo(worktree)?;
    let mut walk = repo.revwalk().map_err(|e| git_err("git2 revwalk", e))?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .map_err(|e| git_err("git2 revwalk sort", e))?;
    if walk.push_head().is_err() {
        return Ok(Vec::new());
    }

    let Some(base) = repo
        .revparse_single(base_branch)
        .and_then(|o| o.peel_to_commit())
        .ok()
        .map(|c| c.id())
    else {
        return Ok(Vec::new());
    };
    if walk.hide(base).is_err() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for oid in walk.flatten().take(MAX_COMMITS) {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let author = commit.author();
        out.push(CommitEntry {
            sha: oid.to_string(),
            subject: commit.summary().unwrap_or_default().to_string(),
            author: author.name().unwrap_or_default().to_string(),
            time_ms: author.when().seconds() as f64 * 1000.0,
        });
    }
    Ok(out)
}

fn commit_at<'r>(repo: &'r git2::Repository, sha: &str) -> Result<git2::Commit<'r>> {
    repo.revparse_single(sha)
        .and_then(|o| o.peel_to_commit())
        .map_err(|e| git_err("git2 commit", e))
}

pub fn commit_files(worktree: &Path, sha: &str) -> Result<Vec<GitFileEntry>> {
    let repo = open_repo(worktree)?;
    let commit = commit_at(&repo, sha)?;
    let diff = commit_diff(&repo, &commit)?;

    let mut files = Vec::new();
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if path.is_empty() || is_excluded(&path) {
            continue;
        }
        files.push(GitFileEntry {
            old_path: delta_old_path(&delta, &path),
            path,
            change: delta_change(delta.status()).to_string(),
        });
    }
    Ok(files)
}

fn commit_diff<'r>(
    repo: &'r git2::Repository,
    commit: &git2::Commit<'_>,
) -> Result<git2::Diff<'r>> {
    let tree = commit.tree().map_err(|e| git_err("git2 commit tree", e))?;
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let mut diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .map_err(|e| git_err("git2 commit diff", e))?;
    diff.find_similar(None)
        .map_err(|e| git_err("git2 commit rename detection", e))?;
    Ok(diff)
}

pub(crate) fn renamed_from(diff: &git2::Diff<'_>, rel: &str) -> Option<String> {
    diff.deltas().find_map(|delta| {
        (delta.new_file().path() == Some(Path::new(rel)))
            .then(|| delta_old_path(&delta, rel))
            .flatten()
    })
}

pub fn commit_file_sides(worktree: &Path, sha: &str, rel: &str) -> Result<FileSides> {
    let repo = open_repo(worktree)?;
    let commit = commit_at(&repo, sha)?;
    let p = Path::new(rel);

    let original_path = commit_diff(&repo, &commit)
        .ok()
        .and_then(|diff| renamed_from(&diff, rel));
    let original_p = original_path.as_deref().map_or(p, Path::new);

    let modified = commit
        .tree()
        .ok()
        .and_then(|t| tree_blob_text(&repo, &t, p));
    let original = commit
        .parent(0)
        .ok()
        .and_then(|parent| parent.tree().ok())
        .and_then(|t| tree_blob_text(&repo, &t, original_p));

    Ok(FileSides {
        original_exists: original.is_some(),
        modified_exists: modified.is_some(),
        original: original.unwrap_or_default(),
        modified: modified.unwrap_or_default(),
        original_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_branches_are_listed_apart_from_locals_and_map_to_local_names() {
        assert_eq!(local_name_for_remote("origin/feat/x"), Some("feat/x"));
        assert_eq!(local_name_for_remote("upstream/main"), Some("main"));
        assert_eq!(local_name_for_remote("main"), None);
        assert_eq!(local_name_for_remote("origin/"), None);

        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        let repo_dir = tmp.path().join("clone");
        let git = |cwd: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git runs");
            assert!(out.status.success(), "git {args:?}");
        };
        std::fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-b", "main"]);
        std::fs::write(origin.join("README.md"), "hi\n").unwrap();
        git(&origin, &["add", "."]);
        git(
            &origin,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );
        git(&origin, &["branch", "linux-support"]);
        git(
            tmp.path(),
            &[
                "clone",
                origin.to_str().unwrap(),
                repo_dir.to_str().unwrap(),
            ],
        );

        assert_eq!(list_local_branches(&repo_dir).unwrap(), vec!["main"]);
        let remotes = list_remote_branches(&repo_dir).unwrap();
        assert!(remotes.contains(&"origin/linux-support".to_string()));
        assert!(!remotes.iter().any(|r| r.ends_with("/HEAD")), "{remotes:?}");
        let all = list_branches(&repo_dir).unwrap();
        assert!(all.contains(&"main".to_string()));
        assert!(all.contains(&"origin/linux-support".to_string()));
    }

    fn git_out(cwd: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["-c", "commit.gpgsign=false"])
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(cwd: &Path, message: &str) -> String {
        git_out(cwd, &["commit", "-q", "--allow-empty", "-m", message]);
        git_out(cwd, &["rev-parse", "HEAD"])
    }

    fn clone_with_origin(tmp: &Path) -> (PathBuf, PathBuf) {
        let origin = tmp.join("origin.git");
        let clone = tmp.join("clone");
        git_out(tmp, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        git_out(tmp, &["init", "-q", "-b", "main", "clone"]);
        commit(&clone, "init");
        git_out(
            &clone,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git_out(&clone, &["push", "-q", "origin", "main"]);
        git_out(
            &clone,
            &["switch", "-q", "-c", "feat", "--track", "origin/main"],
        );
        (origin, clone)
    }

    const NEVER_FETCHED: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn a_pr_head_is_ours_when_the_branch_or_its_pushed_ref_holds_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (origin, clone) = clone_with_origin(tmp.path());
        let ours = |oid: &str| pr_head_is_ours(&clone, "feat", "origin/main", oid) == Some(true);
        let older = commit(&clone, "a");
        let tip = commit(&clone, "b");
        assert!(ours(&tip) && ours(&older));

        let tree = git_out(&clone, &["rev-parse", "HEAD^{tree}"]);
        let unrelated = git_out(&clone, &["commit-tree", &tree, "-m", "elsewhere"]);
        assert!(!ours(&unrelated));
        assert!(!ours(NEVER_FETCHED), "a never-pushed branch reusing a name");
        assert_eq!(pr_head_is_ours(&clone, "feat", "origin/main", ""), None);

        git_out(&clone, &["push", "-q", "-u", "origin", "feat"]);
        assert!(
            !ours(NEVER_FETCHED),
            "pushing a reused branch name does not claim a head it never held"
        );

        let mate = tmp.path().join("mate");
        git_out(
            tmp.path(),
            &[
                "clone",
                "-q",
                "-b",
                "feat",
                origin.to_str().unwrap(),
                "mate",
            ],
        );
        let pushed = commit(&mate, "c");
        git_out(&mate, &["push", "-q", "origin", "feat"]);
        git_out(&clone, &["fetch", "-q", "origin"]);
        assert!(ours(&pushed), "a teammate's push, fetched but not pulled");

        git_out(&mate, &["push", "-q", "origin", "--delete", "feat"]);
        git_out(&clone, &["fetch", "-q", "--prune", "origin"]);
        assert!(!ours(&pushed), "the pushed ref is gone");
    }

    #[test]
    fn only_a_remote_with_a_web_url_can_be_github() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, clone) = clone_with_origin(tmp.path());
        assert!(!has_github_remote(&clone), "a local-path origin");
        assert_eq!(base_remote_web_url(&clone), None);
        git_out(
            &clone,
            &["remote", "add", "gitlab", "git@gitlab.com:o/r.git"],
        );
        assert!(!has_github_remote(&clone), "a remote on gitlab.com");
        assert_eq!(base_remote_web_url(&clone), None);
        git_out(
            &clone,
            &["remote", "add", "github", "git@github.com:o/r.git"],
        );
        assert!(has_github_remote(&clone));
        assert_eq!(
            base_remote_web_url(&clone).as_deref(),
            Some("https://github.com/o/r")
        );
    }

    #[test]
    fn a_branch_pushed_to_a_fork_names_the_fork_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        git_out(tmp.path(), &["init", "-q", "-b", "main", "repo"]);
        commit(&repo, "init");
        let owner = || fork_owner(&repo, "feat", &lookup_remotes(&repo)[0]);
        let config = |args: &[&str]| git_out(&repo, &[["config"].as_slice(), args].concat());

        git_out(
            &repo,
            &["remote", "add", "origin", "git@github.com:me/app.git"],
        );
        config(&["branch.feat.remote", "origin"]);
        assert_eq!(owner(), None, "a single remote is the base repo");
        config(&["--unset", "branch.feat.remote"]);

        git_out(
            &repo,
            &[
                "remote",
                "add",
                "upstream",
                "https://github.com/acme/app.git",
            ],
        );
        assert_eq!(owner(), None, "no push config: the head repo is the base");
        config(&["branch.feat.remote", "upstream"]);
        config(&["branch.feat.merge", "refs/heads/main"]);
        assert_eq!(owner(), None, "tracking the base it was cut from");
        config(&["branch.feat.pushRemote", "origin"]);
        assert_eq!(owner().as_deref(), Some("me"), "pushRemote overrides");
        config(&["--unset", "branch.feat.pushRemote"]);
        config(&["remote.pushDefault", "origin"]);
        assert_eq!(
            owner().as_deref(),
            Some("me"),
            "remote.pushDefault beats the upstream"
        );
        config(&["--unset", "remote.pushDefault"]);
        config(&["branch.feat.remote", "origin"]);
        config(&["branch.feat.merge", "refs/heads/feat"]);
        assert_eq!(owner().as_deref(), Some("me"), "pushed with -u to the fork");
        config(&["--unset", "branch.feat.remote"]);
        config(&["--unset", "branch.feat.merge"]);

        config(&["push.default", "current"]);
        assert_eq!(owner(), None, "push.default current, never pushed");
        git_out(&repo, &["update-ref", "refs/remotes/origin/feat", "HEAD"]);
        assert_eq!(
            owner().as_deref(),
            Some("me"),
            "push.default current, pushed to origin without -u"
        );
        config(&["--unset", "push.default"]);
        assert_eq!(owner(), None, "push.default simple ignores that ref");
        config(&["branch.feat.remote", "origin"]);

        git_out(
            &repo,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/ACME/App.git",
            ],
        );
        assert_eq!(owner(), None, "the base repo under another spelling");
        git_out(
            &repo,
            &[
                "remote",
                "set-url",
                "origin",
                "git@ghe.corp.example:me/app.git",
            ],
        );
        assert_eq!(owner(), None, "a push remote on another host than the base");
    }

    #[test]
    fn work_past_a_pr_head_is_a_local_commit_it_does_not_hold() {
        let tmp = tempfile::tempdir().unwrap();
        let (origin, clone) = clone_with_origin(tmp.path());
        let past = |head: &str, then: Option<&str>| has_work_past(&clone, "feat", head, then);
        let head = commit(&clone, "a");
        assert!(!past(&head, None));

        git_out(&clone, &["push", "-q", "-u", "origin", "feat"]);
        let mate = tmp.path().join("mate");
        git_out(
            tmp.path(),
            &[
                "clone",
                "-q",
                "-b",
                "feat",
                origin.to_str().unwrap(),
                "mate",
            ],
        );
        let ahead = commit(&mate, "b");
        git_out(&mate, &["push", "-q", "origin", "feat"]);
        git_out(&clone, &["fetch", "-q", "origin"]);
        assert!(!past(&ahead, None), "the PR's head is ahead of the tip");

        let tip = commit(&clone, "c");
        assert!(past(&head, None));
        assert!(!past(NEVER_FETCHED, Some(&tip)));
        assert!(past(NEVER_FETCHED, Some(&head)));
    }

    #[test]
    fn search_files_ranks_filename_hits_and_skips_infra() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join("src/session")).unwrap();
        std::fs::write(dir.path().join("src/session/chat.rs"), "x").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/chat.md"), "x").unwrap();

        let hits = search_files(dir.path(), "chat", 10).unwrap();
        assert!(hits.contains(&"src/session/chat.rs".to_string()));
        assert!(!hits.iter().any(|p| p.starts_with("thoughts")));

        let sub = search_files(dir.path(), "ssnchat", 10).unwrap();
        assert!(sub.contains(&"src/session/chat.rs".to_string()));

        assert!(!search_files(dir.path(), "", 10).unwrap().is_empty());
    }

    #[test]
    fn git_dir_resolves_linked_worktree_metadata_outside_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        let repo = git2::Repository::init(&main).unwrap();
        let sig = git2::Signature::now("t", "t@t.t").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let wt = tmp.path().join("wt");
        repo.worktree("feat", &wt, None).unwrap();

        let gd = git_dir(&wt).unwrap();
        assert!(
            !gd.starts_with(&wt),
            "git dir {gd:?} must be outside the worktree"
        );
        assert!(gd.ends_with("worktrees/feat"), "unexpected git dir: {gd:?}");
        assert!(wt.join(".git").is_file());
        assert!(main.join(".git").is_dir());
        assert!(git_dir(&main).unwrap().ends_with(".git"));
    }

    #[test]
    fn common_dir_of_a_linked_worktree_is_the_main_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        let repo = git2::Repository::init(&main).unwrap();
        let sig = git2::Signature::now("t", "t@t.t").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let wt = tmp.path().join("wt");
        repo.worktree("feat", &wt, None).unwrap();

        let main_git = main.join(".git").canonicalize().unwrap();
        let linked = open_repo(&wt).unwrap();
        assert_eq!(common_dir(&linked).canonicalize().unwrap(), main_git);
        assert_eq!(common_dir(&repo).canonicalize().unwrap(), main_git);
    }

    #[test]
    fn worktree_diff_reports_changes_and_skips_thoughts() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "one\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("a.md")).unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("t", "t@t").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "new\n").unwrap();
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/x.md"), "skip\n").unwrap();

        let files = worktree_diff(dir.path()).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"a.md"));
        assert!(paths.contains(&"b.md"));
        assert!(!paths.iter().any(|p| p.starts_with("thoughts")));
        let a = files.iter().find(|f| f.path == "a.md").unwrap();
        assert_eq!(a.change, "modified");
        assert!(a.patch.contains("-one") && a.patch.contains("+two"));
        assert_eq!(
            files.iter().find(|f| f.path == "b.md").unwrap().change,
            "added"
        );
    }

    #[test]
    fn line_counts_sum_diff_and_skip_thoughts() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "one\ntwo\nthree\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("a.md")).unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("t", "t@t").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }

        std::fs::write(dir.path().join("a.md"), "one\nthree\nfour\nfive\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "x\ny\n").unwrap();
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/x.md"), "skip\nskip\n").unwrap();

        assert_eq!(line_counts(dir.path()), (4, 1));
    }

    #[test]
    fn status_of_fresh_repo() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("note.md"), "hello").unwrap();

        let status = thoughts_repo_status(dir.path()).unwrap();
        assert_eq!(status.dirty, 1);
        assert!(!status.has_remote);
        assert_eq!(status.ahead, 0);
    }

    #[test]
    fn non_repo_errors_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        assert!(thoughts_repo_status(dir.path()).is_err());
    }

    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "one\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        dir
    }

    fn entry_paths(entries: &[GitFileEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn status_splits_staged_and_unstaged() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "new\n").unwrap();

        let status = worktree_git_status(dir.path()).unwrap();
        assert!(status.staged.is_empty());
        assert!(entry_paths(&status.unstaged).contains(&"a.md"));
        let b = status.unstaged.iter().find(|e| e.path == "b.md").unwrap();
        assert_eq!(b.change, "untracked");

        stage_files(dir.path(), &["a.md".into(), "b.md".into()]).unwrap();
        let status = worktree_git_status(dir.path()).unwrap();
        assert!(entry_paths(&status.staged).contains(&"a.md"));
        let b = status.staged.iter().find(|e| e.path == "b.md").unwrap();
        assert_eq!(b.change, "added");

        unstage_files(dir.path(), &["a.md".into(), "b.md".into()]).unwrap();
        let status = worktree_git_status(dir.path()).unwrap();
        assert!(status.staged.is_empty());
        assert!(entry_paths(&status.unstaged).contains(&"a.md"));
    }

    #[test]
    fn status_skips_thoughts_and_internal() {
        let dir = repo_with_commit();
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/x.md"), "skip\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".hyprlayer")).unwrap();
        std::fs::write(dir.path().join(".hyprlayer/a.png"), "skip\n").unwrap();
        std::fs::write(dir.path().join("real.md"), "keep\n").unwrap();

        let status = worktree_git_status(dir.path()).unwrap();
        let paths = entry_paths(&status.unstaged);
        assert!(paths.contains(&"real.md"));
        assert!(!paths.iter().any(|p| p.starts_with("thoughts")));
        assert!(!paths.iter().any(|p| p.starts_with(".hyprlayer")));
    }

    #[test]
    fn status_recurses_new_untracked_dir() {
        let dir = repo_with_commit();
        std::fs::create_dir_all(dir.path().join("pkg/sub")).unwrap();
        std::fs::write(dir.path().join("pkg/a.rs"), "a\n").unwrap();
        std::fs::write(dir.path().join("pkg/sub/b.rs"), "b\n").unwrap();

        let status = worktree_git_status(dir.path()).unwrap();
        let paths = entry_paths(&status.unstaged);
        assert!(
            paths.contains(&"pkg/a.rs"),
            "missing nested file: {paths:?}"
        );
        assert!(
            paths.contains(&"pkg/sub/b.rs"),
            "missing deeply nested file: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with('/')),
            "collapsed dir entry leaked: {paths:?}"
        );
    }

    #[test]
    fn discard_reverts_tracked_and_deletes_untracked() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "new\n").unwrap();

        discard_files(dir.path(), &["a.md".into(), "b.md".into()]).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
            "one\n"
        );
        assert!(!dir.path().join("b.md").exists());
        assert!(worktree_git_status(dir.path()).unwrap().unstaged.is_empty());
    }

    #[test]
    fn discard_tracked_changes_spares_untracked_files() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        std::fs::write(dir.path().join("scratch.md"), "wip\n").unwrap();

        assert_eq!(discard_tracked_changes(dir.path()).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
            "one\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("scratch.md")).unwrap(),
            "wip\n"
        );
        assert_eq!(discard_tracked_changes(dir.path()).unwrap(), 0);
    }

    #[test]
    fn file_sides_for_modified_file() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();

        let sides = worktree_file_sides(dir.path(), "a.md", false).unwrap();
        assert_eq!(sides.original, "one\n");
        assert_eq!(sides.modified, "two\n");
        assert!(sides.original_exists && sides.modified_exists);

        stage_files(dir.path(), &["a.md".into()]).unwrap();
        let staged = worktree_file_sides(dir.path(), "a.md", true).unwrap();
        assert_eq!(staged.original, "one\n");
        assert_eq!(staged.modified, "two\n");
    }

    #[test]
    fn file_sides_for_added_file_marks_missing_original() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("b.md"), "new\n").unwrap();
        let sides = worktree_file_sides(dir.path(), "b.md", false).unwrap();
        assert!(!sides.original_exists);
        assert!(sides.modified_exists);
        assert_eq!(sides.modified, "new\n");
    }

    #[test]
    fn list_dir_skips_git_thoughts_and_ignored() {
        let dir = repo_with_commit();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(dir.path().join("thoughts")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "x\n").unwrap();

        let entries = list_dir(dir.path(), "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"src"));
        assert!(names.contains(&"a.md"));
        assert!(!names.contains(&".git"));
        assert!(!names.contains(&"thoughts"));
        assert!(!names.contains(&"ignored.txt"));
        assert!(entries[0].is_dir);
    }

    #[test]
    fn read_text_file_and_path_safety() {
        let dir = repo_with_commit();
        assert_eq!(read_text_file(dir.path(), "a.md").unwrap(), "one\n");
        assert!(matches!(
            read_text_file(dir.path(), "../escape"),
            Err(CoreError::InvalidPath { .. })
        ));
    }

    #[test]
    fn write_text_file_round_trips_bumps_mtime_and_is_path_safe() {
        let dir = repo_with_commit();
        let before = file_mtime_ms(dir.path(), "a.md").unwrap();
        assert!(before > 0.0);

        write_text_file(dir.path(), "a.md", "two\n").unwrap();
        assert_eq!(read_text_file(dir.path(), "a.md").unwrap(), "two\n");
        assert!(!dir.path().join(".a.md.hyprtmp").exists());
        assert!(file_mtime_ms(dir.path(), "a.md").unwrap() >= before);

        write_text_file(dir.path(), "fresh.txt", "hi").unwrap();
        assert_eq!(read_text_file(dir.path(), "fresh.txt").unwrap(), "hi");

        assert!(matches!(
            write_text_file(dir.path(), "../escape", "x"),
            Err(CoreError::InvalidPath { .. })
        ));
        assert!(matches!(
            file_mtime_ms(dir.path(), "../escape"),
            Err(CoreError::InvalidPath { .. })
        ));
    }

    #[test]
    fn remote_to_web_url_maps_common_shapes() {
        let web = |u: &str| remote_to_web_url(u);
        assert_eq!(
            web("git@github.com:acme/repo.git").as_deref(),
            Some("https://github.com/acme/repo")
        );
        assert_eq!(
            web("https://github.com/acme/repo.git").as_deref(),
            Some("https://github.com/acme/repo")
        );
        assert_eq!(
            web("https://github.com/acme/repo").as_deref(),
            Some("https://github.com/acme/repo")
        );
        assert_eq!(
            web("ssh://git@github.com/acme/repo.git").as_deref(),
            Some("https://github.com/acme/repo")
        );
        assert_eq!(
            web("http://git.example.com:443/team/docs.git").as_deref(),
            Some("https://git.example.com/team/docs")
        );
        assert_eq!(web("/srv/git/repo.git"), None);
        assert_eq!(web("C:\\repos\\thing"), None);
        assert_eq!(web("file:///srv/git/repo.git"), None);
        assert_eq!(web("git@localhost:repo.git"), None);
    }

    #[test]
    fn gh_api_host_follows_the_remote_gh_would_pick() {
        let hosts = |remotes: &[(&str, &str, bool)]| {
            let remotes: Vec<_> = remotes
                .iter()
                .map(|(n, u, r)| (n.to_string(), u.to_string(), *r))
                .collect();
            one_per_host(&remotes)
                .iter()
                .map(|r| r.api_host().map(String::from))
                .collect::<Vec<_>>()
        };
        let host = |remotes: &[(&str, &str, bool)]| hosts(remotes).into_iter().next().flatten();
        let ghe = "git@ghe.corp.example:team/app.git";
        let dotcom = "https://github.com/acme/app.git";
        assert_eq!(host(&[("origin", dotcom, false)]), None);
        assert_eq!(
            host(&[("origin", "git@ssh.github.com:acme/app.git", false)]),
            None
        );
        assert_eq!(
            host(&[("origin", ghe, false)]).as_deref(),
            Some("ghe.corp.example")
        );
        assert_eq!(
            host(&[("origin", dotcom, false), ("upstream", ghe, false)]).as_deref(),
            Some("ghe.corp.example")
        );
        assert_eq!(
            host(&[("fork", ghe, false), ("origin", dotcom, false)]),
            None
        );
        assert_eq!(
            host(&[
                ("origin", dotcom, false),
                ("mine", "https://GHE.Corp.Example/me/app", true)
            ])
            .as_deref(),
            Some("ghe.corp.example")
        );
        assert_eq!(hosts(&[("origin", "/srv/git/app.git", false)]), vec![]);
        assert_eq!(hosts(&[]), vec![]);

        let origin = ("origin", "git@github.com:me/app.git", false);
        for upstream in [
            "https://gitlab.com/acme/app.git",
            "ssh://git@altssh.gitlab.com:443/acme/app.git",
            "git@bitbucket.org:acme/app.git",
            "https://dev.azure.com/acme/app/_git/app",
            "git@ssh.dev.azure.com:v3/acme/app/app",
            "https://acme.visualstudio.com/app/_git/app",
            "https://codeberg.org/acme/app.git",
            "https://git.sr.ht/~acme/app",
        ] {
            assert_eq!(
                hosts(&[("upstream", upstream, false), origin]),
                vec![None],
                "{upstream}"
            );
        }
        assert_eq!(
            hosts(&[("mine", "https://GitLab.com/me/app", true), origin]),
            vec![None],
            "a gh-resolved remote on gitlab.com"
        );
        assert_eq!(
            hosts(&[
                (
                    "upstream",
                    "https://gitlab.corp.example/acme/app.git",
                    false
                ),
                origin
            ]),
            vec![Some("gitlab.corp.example".to_string()), None],
            "a self-hosted forge gh cannot tell from GitHub Enterprise"
        );
        assert_eq!(
            hosts(&[
                ("upstream", "/srv/git/app.git", false),
                ("origin", "git@ghe.corp.example:me/app.git", false)
            ]),
            vec![Some("ghe.corp.example".to_string())],
            "a local-path upstream"
        );
        assert_eq!(
            hosts(&[("upstream", "https://notgitlab.com/acme/app", false)]),
            vec![Some("notgitlab.com".to_string())]
        );
    }

    #[test]
    fn file_web_url_builds_blob_link() {
        let dir = repo_with_commit();
        let repo = open_repo(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:acme/thoughts.git")
            .unwrap();
        let branch = current_branch(dir.path()).unwrap();

        let url = file_web_url(&dir.path().join("a.md")).unwrap();
        assert_eq!(
            url,
            format!("https://github.com/acme/thoughts/blob/{branch}/a.md")
        );

        let bare = repo_with_commit();
        assert_eq!(file_web_url(&bare.path().join("a.md")), None);
    }

    const T0: i64 = 1_700_000_000;
    fn sig_at(n: i64) -> git2::Signature<'static> {
        git2::Signature::new("t", "t@t", &git2::Time::new(T0 + n, 0)).unwrap()
    }

    #[test]
    fn commits_stop_at_the_merge_base_with_the_base_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        std::fs::write(dir.path().join("a.md"), "one\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree0 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let base_oid = repo
            .commit(Some("HEAD"), &sig_at(0), &sig_at(0), "base", &tree0, &[])
            .unwrap();
        let base_commit = repo.find_commit(base_oid).unwrap();
        repo.branch("base", &base_commit, false).unwrap();

        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree1 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let c1_oid = repo
            .commit(
                Some("HEAD"),
                &sig_at(1),
                &sig_at(1),
                "first",
                &tree1,
                &[&base_commit],
            )
            .unwrap();
        let c1 = repo.find_commit(c1_oid).unwrap();

        std::fs::write(dir.path().join("a.md"), "three\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree2 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        repo.commit(
            Some("HEAD"),
            &sig_at(2),
            &sig_at(2),
            "second",
            &tree2,
            &[&c1],
        )
        .unwrap();

        let commits = list_commits(dir.path(), "base").unwrap();
        assert_eq!(commits.len(), 2, "{commits:?}");
        assert_eq!(commits[0].subject, "second");
        assert_eq!(commits[1].subject, "first");
        assert_eq!(commits[0].sha.len(), 40);
        assert_eq!(commits[0].author, "t");
        assert_eq!(commits[0].time_ms, (T0 + 2) as f64 * 1000.0);
    }

    #[test]
    fn commits_match_git_log_after_the_base_is_merged_in_and_moves_on() {
        let tmp = tempfile::tempdir().unwrap();
        let (origin, clone) = clone_with_origin(tmp.path());
        commit(&clone, "feature 1");
        git_out(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), "mate"],
        );
        let mate = tmp.path().join("mate");
        commit(&mate, "teammate 1");
        commit(&mate, "teammate 2");
        git_out(&mate, &["push", "-q", "origin", "main"]);
        git_out(&clone, &["fetch", "-q", "origin"]);
        git_out(&clone, &["merge", "-q", "--no-edit", "origin/main"]);
        commit(&clone, "feature 2");

        let mut ours: Vec<String> = list_commits(&clone, "origin/main")
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        let log = git_out(&clone, &["log", "--format=%s", "origin/main..HEAD"]);
        let mut git: Vec<String> = log.lines().map(str::to_string).collect();
        ours.sort();
        git.sort();
        assert_eq!(ours, git);
        assert_eq!(git.len(), 3, "two features and the merge: {git:?}");
    }

    #[test]
    fn commits_after_a_criss_cross_merge_match_git_log() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git_out(d, &["init", "-q", "-b", "main"]);
        commit(d, "root");
        git_out(d, &["switch", "-q", "-c", "feat"]);
        commit(d, "f1");
        git_out(d, &["switch", "-q", "main"]);
        commit(d, "m1");
        git_out(d, &["branch", "-q", "m1tip"]);
        let merge = |into: &str, from: &str, message: &str| {
            git_out(d, &["switch", "-q", into]);
            git_out(d, &["merge", "-q", "--no-ff", "-m", message, from]);
        };
        merge("main", "feat", "main merges f1");
        merge("feat", "m1tip", "feat merges m1");
        commit(d, "f3");
        let bases = git_out(d, &["merge-base", "--all", "main", "HEAD"]);
        assert_eq!(bases.lines().count(), 2, "control: a criss-cross");

        let mut ours: Vec<String> = list_commits(d, "main")
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        let log = git_out(d, &["log", "--format=%s", "main..HEAD"]);
        let mut git: Vec<String> = log.lines().map(str::to_string).collect();
        ours.sort();
        git.sort();
        assert_eq!(ours, git);
        assert_eq!(git, vec!["f3", "feat merges m1"]);
    }

    #[test]
    fn the_commit_list_stops_at_200_while_the_base_count_keeps_them_all() {
        let tmp = tempfile::tempdir().unwrap();
        let (_origin, clone) = clone_with_origin(tmp.path());
        let repo = open_repo(&clone).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let mut parent = repo.head().unwrap().peel_to_commit().unwrap();
        for i in 0..205 {
            let oid = repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    &format!("c{i}"),
                    &tree,
                    &[&parent],
                )
                .unwrap();
            parent = repo.find_commit(oid).unwrap();
        }

        let listed: Vec<String> = list_commits(&clone, "origin/main")
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        let newest: Vec<String> = (5..205).rev().map(|i| format!("c{i}")).collect();
        assert_eq!(
            listed, newest,
            "the newest 200, newest first, within one second"
        );
        assert_eq!(
            crate::worktree::status(&clone, "origin/main", None, None)
                .base
                .ahead,
            205
        );
    }

    #[test]
    fn commits_are_empty_when_the_base_is_unresolvable() {
        let dir = repo_with_commit();
        assert!(list_commits(dir.path(), "no-such-branch")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn commits_are_empty_when_head_is_the_base() {
        let dir = repo_with_commit();
        let repo = open_repo(dir.path()).unwrap();
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("base", &head_commit, false).unwrap();

        assert!(list_commits(dir.path(), "base").unwrap().is_empty());
    }

    #[test]
    fn commits_are_empty_on_an_unborn_head() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        assert!(list_commits(dir.path(), "base").unwrap().is_empty());
    }

    #[test]
    fn commit_files_lists_the_first_parent_diff_and_skips_infra() {
        let dir = repo_with_commit();
        let repo = open_repo(dir.path()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();

        std::fs::write(dir.path().join("real.md"), "keep\n").unwrap();
        std::fs::create_dir_all(dir.path().join("thoughts/jt")).unwrap();
        std::fs::write(dir.path().join("thoughts/jt/x.md"), "skip\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".hyprlayer")).unwrap();
        std::fs::write(dir.path().join(".hyprlayer/a.png"), "skip\n").unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new("real.md")).unwrap();
        index.add_path(Path::new("thoughts/jt/x.md")).unwrap();
        index.add_path(Path::new(".hyprlayer/a.png")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let oid = repo
            .commit(
                Some("HEAD"),
                &sig_at(1),
                &sig_at(1),
                "add real",
                &tree,
                &[&parent],
            )
            .unwrap();

        let files = commit_files(dir.path(), &oid.to_string()).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(files[0].path, "real.md");
        assert_eq!(files[0].change, "added");
    }

    #[test]
    fn commit_file_sides_diffs_a_rename_against_its_old_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        let before = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
        let after = "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\neight\n";

        std::fs::write(dir.path().join("a.md"), before).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree0 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let c0_oid = repo
            .commit(Some("HEAD"), &sig_at(0), &sig_at(0), "init", &tree0, &[])
            .unwrap();
        let c0 = repo.find_commit(c0_oid).unwrap();

        std::fs::remove_file(dir.path().join("a.md")).unwrap();
        std::fs::write(dir.path().join("b.md"), after).unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("a.md")).unwrap();
        index.add_path(Path::new("b.md")).unwrap();
        index.write().unwrap();
        let tree1 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let c1_oid = repo
            .commit(
                Some("HEAD"),
                &sig_at(1),
                &sig_at(1),
                "rename and edit",
                &tree1,
                &[&c0],
            )
            .unwrap();
        let c1_sha = c1_oid.to_string();

        let files = commit_files(dir.path(), &c1_sha).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(files[0].path, "b.md");
        assert_eq!(files[0].change, "renamed");
        assert_eq!(files[0].old_path.as_deref(), Some("a.md"));

        let sides = commit_file_sides(dir.path(), &c1_sha, "b.md").unwrap();
        assert!(sides.original_exists && sides.modified_exists, "{sides:?}");
        assert_eq!(sides.original, before);
        assert_eq!(sides.modified, after);
        assert_eq!(sides.original_path.as_deref(), Some("a.md"));
    }

    #[test]
    fn commit_files_uses_the_first_parent_for_a_merge() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        let base_blob = repo.blob(b"base\n").unwrap();
        let b_blob = repo.blob(b"b\n").unwrap();
        let c_blob = repo.blob(b"c\n").unwrap();

        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert("base.md", base_blob, 0o100_644).unwrap();
        let tree_a = repo.find_tree(tb.write().unwrap()).unwrap();
        let a_oid = repo
            .commit(None, &sig_at(0), &sig_at(0), "A", &tree_a, &[])
            .unwrap();
        let a = repo.find_commit(a_oid).unwrap();

        let mut tb = repo.treebuilder(Some(&tree_a)).unwrap();
        tb.insert("b.md", b_blob, 0o100_644).unwrap();
        let tree_b = repo.find_tree(tb.write().unwrap()).unwrap();
        let b_oid = repo
            .commit(None, &sig_at(1), &sig_at(1), "B", &tree_b, &[&a])
            .unwrap();
        let b = repo.find_commit(b_oid).unwrap();

        let mut tb = repo.treebuilder(Some(&tree_a)).unwrap();
        tb.insert("c.md", c_blob, 0o100_644).unwrap();
        let tree_c = repo.find_tree(tb.write().unwrap()).unwrap();
        let c_oid = repo
            .commit(None, &sig_at(2), &sig_at(2), "C", &tree_c, &[&a])
            .unwrap();
        let c = repo.find_commit(c_oid).unwrap();

        let m_oid = repo
            .commit(
                Some("HEAD"),
                &sig_at(3),
                &sig_at(3),
                "M",
                &tree_b,
                &[&b, &c],
            )
            .unwrap();

        let files = commit_files(dir.path(), &m_oid.to_string()).unwrap();
        assert!(files.is_empty(), "{files:?}");
    }

    #[test]
    fn commit_file_sides_reads_both_trees() {
        let dir = repo_with_commit();
        let repo = open_repo(dir.path()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();

        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let oid = repo
            .commit(
                Some("HEAD"),
                &sig_at(1),
                &sig_at(1),
                "modify",
                &tree,
                &[&parent],
            )
            .unwrap();

        let sides = commit_file_sides(dir.path(), &oid.to_string(), "a.md").unwrap();
        assert_eq!(sides.original, "one\n");
        assert_eq!(sides.modified, "two\n");
        assert!(sides.original_exists && sides.modified_exists);
        assert_eq!(sides.original_path, None);

        let root_sha = parent.id().to_string();
        let root_sides = commit_file_sides(dir.path(), &root_sha, "a.md").unwrap();
        assert!(!root_sides.original_exists);
        assert!(root_sides.modified_exists);
        assert_eq!(root_sides.modified, "one\n");
    }

    fn rows(entries: &[GitFileEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| format!("{}:{}", e.change, e.path))
            .collect()
    }

    fn porcelain(cwd: &Path) -> Vec<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["status", "--porcelain", "-uall"])
            .output()
            .expect("git runs");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn line_counts(dir: &Path) -> (u32, u32) {
        let chip = crate::worktree::status(dir, "main", None, None);
        (chip.additions, chip.deletions)
    }

    fn repo_with_submodule(tmp: &Path) -> PathBuf {
        git_out(tmp, &["init", "-q", "-b", "main", "lib"]);
        let lib = tmp.join("lib");
        std::fs::write(lib.join("lib.txt"), "v1\n").unwrap();
        git_out(&lib, &["add", "."]);
        commit(&lib, "lib v1");
        git_out(tmp, &["init", "-q", "-b", "main", "repo"]);
        let repo = tmp.join("repo");
        commit(&repo, "init");
        git_out(
            &repo,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                lib.to_str().unwrap(),
                "vendor/lib",
            ],
        );
        commit(&repo, "add submodule");
        repo
    }

    #[test]
    fn a_submodule_pointer_bump_is_one_change_with_subproject_sides_and_discard_resets_it() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        let sub = repo.join("vendor/lib");
        let recorded = git_out(&sub, &["rev-parse", "HEAD"]);
        let bumped = commit(&sub, "bump");

        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(rows(&st.unstaged), vec!["modified:vendor/lib"]);
        assert!(st.staged.is_empty(), "{st:?}");
        let sides = worktree_file_sides(&repo, "vendor/lib", false).unwrap();
        assert_eq!(sides.original, format!("Subproject commit {recorded}\n"));
        assert_eq!(sides.modified, format!("Subproject commit {bumped}\n"));
        assert!(sides.original_exists && sides.modified_exists);
        assert_eq!(line_counts(&repo), (1, 1));

        git_out(&repo, &["add", "vendor/lib"]);
        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(rows(&st.staged), vec!["modified:vendor/lib"]);
        assert!(st.unstaged.is_empty(), "{st:?}");

        discard_files(&repo, &["vendor/lib".into()]).unwrap();
        assert_eq!(git_out(&sub, &["rev-parse", "HEAD"]), recorded);
        assert_eq!(porcelain(&repo), Vec::<String>::new());
        let st = worktree_git_status(&repo).unwrap();
        assert!(st.staged.is_empty() && st.unstaged.is_empty(), "{st:?}");
    }

    #[test]
    fn a_commit_that_bumps_a_submodule_diffs_as_its_subproject_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        let sub = repo.join("vendor/lib");
        let recorded = git_out(&sub, &["rev-parse", "HEAD"]);
        let bumped = commit(&sub, "bump");
        git_out(&repo, &["add", "vendor/lib"]);
        let sha = commit(&repo, "bump lib");

        let sides = commit_file_sides(&repo, &sha, "vendor/lib").unwrap();
        assert_eq!(sides.original, format!("Subproject commit {recorded}\n"));
        assert_eq!(sides.modified, format!("Subproject commit {bumped}\n"));
    }

    #[test]
    fn discarding_a_submodule_bump_never_overwrites_edits_inside_it() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        let sub = repo.join("vendor/lib");
        std::fs::write(sub.join("lib.txt"), "v2\n").unwrap();
        git_out(&sub, &["add", "."]);
        let bumped = commit(&sub, "lib v2");
        std::fs::write(sub.join("lib.txt"), "wip\n").unwrap();

        assert!(discard_files(&repo, &["vendor/lib".into()]).is_err());
        assert_eq!(git_out(&sub, &["rev-parse", "HEAD"]), bumped);
        assert_eq!(
            std::fs::read_to_string(sub.join("lib.txt")).unwrap(),
            "wip\n"
        );
    }

    #[test]
    fn dirty_content_inside_a_submodule_is_not_a_change() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        let sub = repo.join("vendor/lib");
        std::fs::write(sub.join("lib.txt"), "edited\n").unwrap();
        std::fs::write(sub.join("scratch.txt"), "new\n").unwrap();

        let st = worktree_git_status(&repo).unwrap();
        assert!(st.staged.is_empty() && st.unstaged.is_empty(), "{st:?}");
        assert_eq!(line_counts(&repo), (0, 0));
    }

    #[test]
    fn sparse_and_skip_worktree_files_are_clean_but_a_plain_delete_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        git_out(tmp.path(), &["init", "-q", "-b", "main", "repo"]);
        let repo = tmp.path().join("repo");
        for f in [
            "README.md",
            "apps/web/index.ts",
            "apps/api/main.rs",
            "docs/guide.md",
        ] {
            std::fs::create_dir_all(repo.join(f).parent().unwrap()).unwrap();
            std::fs::write(repo.join(f), "one\ntwo\nthree\n").unwrap();
        }
        git_out(&repo, &["add", "."]);
        commit(&repo, "init");

        let wt = tmp.path().join("wt");
        git_out(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        );
        git_out(&wt, &["sparse-checkout", "set", "apps/web"]);
        assert!(!wt.join("docs/guide.md").exists(), "control: sparse");
        assert!(porcelain(&wt).is_empty(), "control: git calls it clean");
        let st = worktree_git_status(&wt).unwrap();
        assert!(st.staged.is_empty() && st.unstaged.is_empty(), "{st:?}");
        assert_eq!(line_counts(&wt), (0, 0));

        git_out(&repo, &["update-index", "--skip-worktree", "docs/guide.md"]);
        std::fs::remove_file(repo.join("docs/guide.md")).unwrap();
        assert!(porcelain(&repo).is_empty(), "control: git calls it clean");
        let st = worktree_git_status(&repo).unwrap();
        assert!(st.staged.is_empty() && st.unstaged.is_empty(), "{st:?}");
        assert_eq!(line_counts(&repo), (0, 0));

        std::fs::remove_file(repo.join("README.md")).unwrap();
        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(rows(&st.unstaged), vec!["deleted:README.md"]);
        assert_eq!(line_counts(&repo), (0, 3));
    }

    #[test]
    fn a_sparse_index_is_an_error_and_never_a_clean_chip() {
        let tmp = tempfile::tempdir().unwrap();
        git_out(tmp.path(), &["init", "-q", "-b", "main", "repo"]);
        let repo = tmp.path().join("repo");
        for f in ["README.md", "apps/web/index.ts", "docs/guide.md"] {
            std::fs::create_dir_all(repo.join(f).parent().unwrap()).unwrap();
            std::fs::write(repo.join(f), "one\n").unwrap();
        }
        git_out(&repo, &["add", "."]);
        commit(&repo, "init");
        let wt = tmp.path().join("wt");
        git_out(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        );
        git_out(
            &wt,
            &["sparse-checkout", "set", "--sparse-index", "apps/web"],
        );
        std::fs::write(wt.join("apps/web/index.ts"), "edited\n").unwrap();
        assert_eq!(porcelain(&wt), vec![" M apps/web/index.ts"], "control");

        assert_eq!(
            worktree_git_status(&wt).unwrap_err().code(),
            "sparse_index_unsupported"
        );
        let chip = crate::worktree::status(&wt, "main", None, None);
        assert!(chip.unreadable.is_some(), "{chip:?}");
        assert_eq!(
            crate::worktree::ensure_removable(&wt, "main", "feat")
                .unwrap_err()
                .code(),
            "worktree_unreadable"
        );
        assert_eq!(
            checkout_blockers(&wt).unwrap_err().code(),
            "sparse_index_unsupported"
        );
        let prompt = prompt_git_vars(&wt, true, false, false).changed_files;
        assert!(prompt.starts_with("unknown: "), "{prompt}");
    }

    const RENAMED_BODY: &str = "fn main() {\n    println!(\"a fairly long body\");\n}\n";

    fn repo_with_staged_rename() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_out(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("old_name.rs"), RENAMED_BODY).unwrap();
        git_out(dir.path(), &["add", "."]);
        commit(dir.path(), "init");
        git_out(dir.path(), &["mv", "old_name.rs", "new_name.rs"]);
        dir
    }

    #[test]
    fn a_staged_rename_is_one_row_that_diffs_against_its_old_path() {
        let dir = repo_with_staged_rename();

        let st = worktree_git_status(dir.path()).unwrap();
        assert_eq!(rows(&st.staged), vec!["renamed:new_name.rs"]);
        assert_eq!(st.staged[0].old_path.as_deref(), Some("old_name.rs"));
        assert!(st.unstaged.is_empty(), "{st:?}");

        let sides = worktree_file_sides(dir.path(), "new_name.rs", true).unwrap();
        assert_eq!(sides.original, RENAMED_BODY);
        assert_eq!(sides.modified, RENAMED_BODY);
        assert!(sides.original_exists && sides.modified_exists);
        assert_eq!(sides.original_path.as_deref(), Some("old_name.rs"));
    }

    #[test]
    fn the_chip_counts_a_rename_against_its_old_path_as_git_does() {
        let dir = repo_with_staged_rename();
        let numstat = || git_out(dir.path(), &["diff", "--numstat", "HEAD"]);
        assert_eq!(numstat(), "0\t0\told_name.rs => new_name.rs");
        assert_eq!(line_counts(dir.path()), (0, 0));
        assert_eq!(
            prompt_git_vars(dir.path(), true, false, false).changed_files,
            "1 file (+0 −0)"
        );

        let edited = format!("{RENAMED_BODY}// one more line\n");
        std::fs::write(dir.path().join("new_name.rs"), edited).unwrap();
        assert_eq!(numstat(), "1\t0\told_name.rs => new_name.rs");
        assert_eq!(line_counts(dir.path()), (1, 0));

        let rewritten = "nothing\nlike\nthe\nold one\n";
        std::fs::write(dir.path().join("new_name.rs"), rewritten).unwrap();
        assert_eq!(numstat(), "4\t0\tnew_name.rs\n0\t3\told_name.rs");
        assert_eq!(line_counts(dir.path()), (4, 3));
    }

    #[test]
    fn a_merge_conflict_counts_its_lines_against_head_as_git_does() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git_out(d, &["init", "-q", "-b", "main"]);
        std::fs::write(d.join("f.txt"), "a\nb\n").unwrap();
        git_out(d, &["add", "."]);
        commit(d, "init");
        git_out(d, &["switch", "-q", "-c", "other"]);
        std::fs::write(d.join("f.txt"), "a\nOTHER\n").unwrap();
        git_out(d, &["commit", "-qam", "other"]);
        git_out(d, &["switch", "-q", "main"]);
        std::fs::write(d.join("f.txt"), "a\nMAIN\n").unwrap();
        git_out(d, &["commit", "-qam", "main"]);
        let merge = || {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(d)
                .args(["merge", "-q", "other"])
                .output()
                .unwrap();
            assert!(!out.status.success(), "control: the merge conflicts");
        };
        merge();

        let st = worktree_git_status(d).unwrap();
        assert_eq!(rows(&st.unstaged), vec!["conflict:f.txt"]);
        assert_eq!(git_out(d, &["diff", "--numstat", "HEAD"]), "4\t0\tf.txt");
        assert_eq!(line_counts(d), (4, 0));

        git_out(d, &["merge", "--abort"]);
        git_out(d, &["rm", "-q", "f.txt"]);
        git_out(d, &["commit", "-qm", "delete"]);
        merge();
        assert_eq!(porcelain(d), vec!["DU f.txt"], "control: deleted by us");
        assert_eq!(git_out(d, &["diff", "--numstat", "HEAD"]), "2\t0\tf.txt");
        assert_eq!(line_counts(d), (2, 0));
    }

    #[test]
    fn unstaging_or_discarding_a_rename_takes_both_of_its_paths() {
        let dir = repo_with_staged_rename();
        unstage_files(dir.path(), &["new_name.rs".into()]).unwrap();
        assert_eq!(
            porcelain(dir.path()),
            vec![" D old_name.rs", "?? new_name.rs"]
        );

        let dir = repo_with_staged_rename();
        discard_files(dir.path(), &["new_name.rs".into()]).unwrap();
        assert_eq!(porcelain(dir.path()), Vec::<String>::new());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("old_name.rs")).unwrap(),
            RENAMED_BODY
        );

        let dir = repo_with_staged_rename();
        discard_tracked_changes(dir.path()).unwrap();
        assert_eq!(porcelain(dir.path()), Vec::<String>::new());
        assert!(dir.path().join("old_name.rs").exists());
        assert!(!dir.path().join("new_name.rs").exists());
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn discarding_a_staged_rename_never_overwrites_a_file_recreated_at_its_source() {
        let dir = repo_with_staged_rename();
        std::fs::write(dir.path().join("old_name.rs"), "brand new\n").unwrap();

        assert_eq!(
            discard_files(dir.path(), &["new_name.rs".into()])
                .unwrap_err()
                .code(),
            "discard_refused"
        );
        assert_eq!(read(&dir.path().join("old_name.rs")), "brand new\n");
        assert_eq!(
            porcelain(dir.path()),
            vec!["R  old_name.rs -> new_name.rs", "?? old_name.rs"]
        );
    }

    #[test]
    fn discarding_the_unstaged_edit_of_a_renamed_file_keeps_the_rename() {
        let dir = repo_with_staged_rename();
        std::fs::write(dir.path().join("new_name.rs"), "edited\n").unwrap();

        discard_files(dir.path(), &["new_name.rs".into()]).unwrap();
        assert_eq!(porcelain(dir.path()), vec!["R  old_name.rs -> new_name.rs"]);
        assert_eq!(read(&dir.path().join("new_name.rs")), RENAMED_BODY);
    }

    #[cfg(unix)]
    #[test]
    fn discarding_a_file_replaced_by_a_symlink_restores_the_file_and_spares_the_target() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("target.txt");
        let dir = repo_with_commit();
        let file = dir.path().join("a.md");
        for staged in [false, true] {
            std::fs::write(&target, "outside the repo\n").unwrap();
            std::fs::remove_file(&file).unwrap();
            std::os::unix::fs::symlink(&target, &file).unwrap();
            if staged {
                git_out(dir.path(), &["add", "a.md"]);
            }

            discard_files(dir.path(), &["a.md".into()]).unwrap();
            assert!(!file.is_symlink(), "staged={staged}");
            assert_eq!(read(&file), "one\n");
            assert_eq!(read(&target), "outside the repo\n", "staged={staged}");
            assert_eq!(porcelain(dir.path()), Vec::<String>::new());
        }
    }

    #[cfg(unix)]
    #[test]
    fn discard_never_writes_or_deletes_through_a_symlinked_directory() {
        let outside = tempfile::tempdir().unwrap();
        let out = outside.path();
        std::fs::write(out.join("x"), "precious\n").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git_out(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir(d.join("d")).unwrap();
        std::fs::write(d.join("d/x"), "orig\n").unwrap();
        git_out(d, &["add", "."]);
        commit(d, "init");
        let link = |rel: &str| {
            std::fs::remove_dir_all(d.join(rel)).unwrap();
            std::os::unix::fs::symlink(out, d.join(rel)).unwrap();
        };

        for stage_all in [false, true] {
            link("d");
            if stage_all {
                git_out(d, &["add", "-A"]);
            }
            discard_files(d, &["d/x".into()]).unwrap();
            assert_eq!(read(&out.join("x")), "precious\n", "stage_all={stage_all}");
            assert!(!d.join("d").is_symlink());
            assert_eq!(read(&d.join("d/x")), "orig\n");
            assert_eq!(porcelain(d), Vec::<String>::new());
        }

        std::fs::create_dir(d.join("n")).unwrap();
        std::fs::write(d.join("n/y"), "new\n").unwrap();
        git_out(d, &["add", "n/y"]);
        link("n");
        discard_files(d, &["n/y".into()]).unwrap();
        assert!(!out.join("y").exists());
        assert_eq!(porcelain(d), vec!["A  n/y"]);

        std::fs::write(out.join("y"), "precious\n").unwrap();
        link("n");
        discard_tracked_changes(d).unwrap();
        assert_eq!(read(&out.join("y")), "precious\n");
        assert_eq!(read(&out.join("x")), "precious\n");
    }

    #[test]
    fn discarding_an_added_gitlink_keeps_its_checkout_and_refuses_to_delete_a_nested_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        git_out(&repo, &["reset", "-q", "--soft", "HEAD~1"]);
        std::fs::write(repo.join("vendor/lib/lib.txt"), "wip\n").unwrap();
        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(
            rows(&st.staged),
            vec!["added:.gitmodules", "added:vendor/lib"]
        );
        assert!(st.unstaged.is_empty(), "edits inside are not a row: {st:?}");

        discard_files(&repo, &["vendor/lib".into()]).unwrap();
        assert_eq!(read(&repo.join("vendor/lib/lib.txt")), "wip\n");
        assert_eq!(porcelain(&repo), vec!["A  .gitmodules", "?? vendor/lib/"]);
        assert_eq!(
            discard_files(&repo, &["vendor/lib/".into()])
                .unwrap_err()
                .code(),
            "discard_refused"
        );
        assert!(repo.join("vendor/lib/lib.txt").exists());

        let tool = repo.join("tool");
        git_out(&repo, &["init", "-q", "-b", "main", "tool"]);
        std::fs::write(tool.join("t.txt"), "t\n").unwrap();
        git_out(&tool, &["add", "."]);
        commit(&tool, "tool history");
        std::fs::write(tool.join("wip.txt"), "uncommitted\n").unwrap();
        git_out(&repo, &["add", "tool"]);
        discard_files(&repo, &["tool".into()]).unwrap();
        assert_eq!(read(&tool.join("wip.txt")), "uncommitted\n");
        assert_eq!(git_out(&tool, &["rev-list", "--count", "HEAD"]), "1");
    }

    #[test]
    fn staging_an_untracked_nested_repo_adds_its_head_as_git_add_does() {
        let dir = repo_with_commit();
        let nested = dir.path().join("nested");
        git_out(dir.path(), &["init", "-q", "-b", "main", "nested"]);
        std::fs::write(nested.join("n.txt"), "n\n").unwrap();
        git_out(&nested, &["add", "."]);
        let head = commit(&nested, "n");
        let st = worktree_git_status(dir.path()).unwrap();
        assert_eq!(rows(&st.unstaged), vec!["untracked:nested/"]);

        stage_files(dir.path(), &["nested/".into()]).unwrap();
        assert_eq!(porcelain(dir.path()), vec!["A  nested"]);
        assert_eq!(
            git_out(dir.path(), &["ls-files", "--stage", "nested"]),
            format!("160000 {head} 0\tnested")
        );
    }

    #[test]
    fn discarding_a_deleted_submodule_restores_its_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        std::fs::remove_dir_all(repo.join("vendor/lib")).unwrap();
        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(rows(&st.unstaged), vec!["deleted:vendor/lib"]);

        discard_files(&repo, &["vendor/lib".into()]).unwrap();
        assert_eq!(read(&repo.join("vendor/lib/lib.txt")), "v1\n");
        assert_eq!(porcelain(&repo), Vec::<String>::new());

        git_out(&repo, &["rm", "-q", "vendor/lib"]);
        discard_files(&repo, &["vendor/lib".into()]).unwrap();
        assert_eq!(read(&repo.join("vendor/lib/lib.txt")), "v1\n");
        assert_eq!(porcelain(&repo), vec!["M  .gitmodules"]);
    }

    #[test]
    fn a_refused_submodule_discard_leaves_the_index_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        let sub = repo.join("vendor/lib");
        std::fs::write(sub.join("lib.txt"), "v2\n").unwrap();
        git_out(&sub, &["add", "."]);
        commit(&sub, "lib v2");
        git_out(&repo, &["add", "vendor/lib"]);
        std::fs::write(sub.join("lib.txt"), "wip\n").unwrap();
        let before = porcelain(&repo);

        assert!(discard_files(&repo, &["vendor/lib".into()]).is_err());
        assert_eq!(porcelain(&repo), before);
        assert_eq!(read(&sub.join("lib.txt")), "wip\n");
    }

    #[test]
    fn dirty_content_beside_a_staged_bump_is_not_an_unstaged_row() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        let sub = repo.join("vendor/lib");
        commit(&sub, "bump");
        git_out(&repo, &["add", "vendor/lib"]);
        std::fs::write(sub.join("build.log"), "artifact\n").unwrap();
        std::fs::write(sub.join("lib.txt"), "patched\n").unwrap();

        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(rows(&st.staged), vec!["modified:vendor/lib"]);
        assert!(st.unstaged.is_empty(), "{st:?}");
        assert_eq!(crate::worktree::status(&repo, "main", None, None).dirty, 1);
    }

    #[test]
    fn a_moved_submodule_is_one_renamed_row_that_discard_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo_with_submodule(tmp.path());
        git_out(&repo, &["mv", "vendor/lib", "third_party_lib"]);
        std::fs::write(repo.join("third_party_lib/lib.txt"), "patched\n").unwrap();

        let st = worktree_git_status(&repo).unwrap();
        assert_eq!(
            rows(&st.staged),
            vec!["modified:.gitmodules", "renamed:third_party_lib"]
        );
        assert_eq!(st.staged[1].old_path.as_deref(), Some("vendor/lib"));
        assert!(st.unstaged.is_empty(), "{st:?}");

        assert_eq!(
            discard_files(&repo, &["third_party_lib".into()])
                .unwrap_err()
                .code(),
            "discard_refused"
        );
        assert_eq!(read(&repo.join("third_party_lib/lib.txt")), "patched\n");
    }

    #[test]
    fn a_rename_into_thoughts_still_shows_the_deletion() {
        let dir = repo_with_commit();
        std::fs::create_dir_all(dir.path().join("thoughts")).unwrap();
        git_out(dir.path(), &["mv", "a.md", "thoughts/a.md"]);

        let st = worktree_git_status(dir.path()).unwrap();
        assert_eq!(rows(&st.staged), vec!["deleted:a.md"]);
        assert_eq!(
            crate::worktree::status(dir.path(), "main", None, None).dirty,
            1
        );
    }

    #[test]
    fn team_settings_under_hyprlayer_stay_visible_through_index_changes() {
        let settings = ".hyprlayer/settings.toml";
        let dir = repo_with_commit();
        std::fs::create_dir_all(dir.path().join(".hyprlayer")).unwrap();
        std::fs::write(dir.path().join(settings), "[worktree]\n").unwrap();
        std::fs::write(dir.path().join(".hyprlayer/a.png"), "internal\n").unwrap();
        stage_files(dir.path(), &[settings.into()]).unwrap();
        unstage_files(dir.path(), &[settings.into()]).unwrap();
        let st = worktree_git_status(dir.path()).unwrap();
        assert_eq!(rows(&st.unstaged), vec![format!("untracked:{settings}")]);

        stage_files(dir.path(), &[settings.into()]).unwrap();
        commit(dir.path(), "settings");
        git_out(dir.path(), &["rm", "-q", "--cached", settings]);
        let st = worktree_git_status(dir.path()).unwrap();
        assert_eq!(rows(&st.staged), vec![format!("deleted:{settings}")]);
        assert_eq!(rows(&st.unstaged), vec![format!("untracked:{settings}")]);
        let chip = crate::worktree::status(dir.path(), "main", None, None);
        assert_eq!((chip.dirty, chip.untracked, chip.deletions), (1, 1, 1));
    }

    fn renamed_targets(dir: &Path) -> Vec<String> {
        let mut ours: Vec<String> = worktree_git_status(dir)
            .unwrap()
            .staged
            .into_iter()
            .filter(|e| e.change == "renamed")
            .map(|e| e.path)
            .collect();
        ours.sort();
        ours
    }

    fn git_renamed_targets(dir: &Path) -> Vec<String> {
        let mut theirs: Vec<String> = porcelain(dir)
            .into_iter()
            .filter_map(|l| Some(l.strip_prefix("R  ")?.split(" -> ").nth(1)?.to_string()))
            .collect();
        theirs.sort();
        theirs
    }

    #[test]
    fn rename_detection_follows_git_config() {
        let dir = repo_with_staged_rename();
        let both = vec!["deleted:old_name.rs", "added:new_name.rs"];
        for key in ["status.renames", "diff.renames"] {
            git_out(dir.path(), &["config", key, "false"]);
            let st = worktree_git_status(dir.path()).unwrap();
            let mut staged = rows(&st.staged);
            staged.sort_by_key(|r| r.starts_with("added"));
            assert_eq!(staged, both, "{key}=false");
            git_out(dir.path(), &["config", "--unset", key]);
        }
        assert_eq!(renamed_targets(dir.path()), vec!["new_name.rs"]);
    }

    #[test]
    fn rename_detection_pairs_what_git_pairs_by_similarity() {
        let dir = tempfile::tempdir().unwrap();
        git_out(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("pad.txt"), "pad\n").unwrap();
        let lines = |tag: &str, n: usize| -> String {
            (1..=n).map(|i| format!("line {i} of {tag}\n")).collect()
        };
        let ks = [6, 8, 9, 10, 11, 12, 14];
        for k in ks {
            std::fs::write(
                dir.path().join(format!("src{k}.txt")),
                lines(&format!("f{k}"), 20),
            )
            .unwrap();
        }
        git_out(dir.path(), &["add", "."]);
        commit(dir.path(), "files");
        for k in ks {
            git_out(
                dir.path(),
                &["mv", &format!("src{k}.txt"), &format!("dst{k}.txt")],
            );
            let kept: String = lines(&format!("f{k}"), 20 - k);
            let new: String = (1..=k)
                .map(|i| format!("totally different content number {i} {k}\n"))
                .collect();
            std::fs::write(dir.path().join(format!("dst{k}.txt")), kept + &new).unwrap();
        }
        git_out(dir.path(), &["add", "-A"]);

        assert_eq!(renamed_targets(dir.path()), git_renamed_targets(dir.path()));
    }

    #[test]
    fn an_intent_to_add_file_is_never_a_rename_target() {
        let dir = tempfile::tempdir().unwrap();
        git_out(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("empty.log"), "").unwrap();
        std::fs::write(dir.path().join("x"), "base\n").unwrap();
        git_out(dir.path(), &["add", "-A"]);
        commit(dir.path(), "init");
        git_out(dir.path(), &["rm", "-q", "empty.log"]);
        std::fs::write(dir.path().join("notes.md"), "draft\n").unwrap();
        git_out(dir.path(), &["add", "-N", "notes.md"]);

        let st = worktree_git_status(dir.path()).unwrap();
        assert!(
            rows(&st.staged).contains(&"deleted:empty.log".to_string()),
            "{st:?}"
        );
        assert!(renamed_targets(dir.path()).is_empty(), "{st:?}");
        assert_eq!(
            crate::worktree::status(dir.path(), "main", None, None).dirty,
            2
        );

        unstage_files(dir.path(), &["notes.md".into()]).unwrap();
        assert_eq!(porcelain(dir.path()), vec!["D  empty.log", "?? notes.md"]);
    }

    #[test]
    fn a_binary_attribute_keeps_crs_when_scoring_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        git_out(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join(".gitattributes"), "*.dat binary\n").unwrap();
        let lf: String = (0..40).map(|i| format!("l{i} aaaaaaaaaaa\n")).collect();
        std::fs::write(dir.path().join("a.dat"), &lf).unwrap();
        git_out(dir.path(), &["add", "-A"]);
        commit(dir.path(), "init");
        git_out(dir.path(), &["rm", "-q", "a.dat"]);
        std::fs::write(dir.path().join("b.dat"), lf.replace('\n', "\r\n")).unwrap();
        git_out(dir.path(), &["add", "b.dat"]);

        assert_eq!(git_renamed_targets(dir.path()), Vec::<String>::new());
        assert_eq!(renamed_targets(dir.path()), git_renamed_targets(dir.path()));
    }

    #[test]
    fn a_copies_config_shows_copied_rows_whose_actions_leave_the_source_alone() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git_out(d, &["init", "-q", "-b", "main"]);
        git_out(d, &["config", "status.renames", "copies"]);
        let lines =
            |from: u32| -> String { (from..from + 100).map(|i| format!("{i}\n")).collect() };
        let (body, edited) = (lines(1), lines(200) + "edited\n");
        std::fs::write(d.join("a"), &body).unwrap();
        std::fs::write(d.join("m"), lines(200)).unwrap();
        git_out(d, &["add", "."]);
        commit(d, "init");
        std::fs::copy(d.join("a"), d.join("b")).unwrap();
        git_out(d, &["mv", "a", "c"]);
        std::fs::write(d.join("m"), &edited).unwrap();
        std::fs::write(d.join("n"), &edited).unwrap();
        git_out(d, &["add", "-A"]);
        assert_eq!(
            porcelain(d),
            vec!["C  a -> b", "R  a -> c", "M  m", "C  m -> n"],
            "control"
        );

        let st = worktree_git_status(d).unwrap();
        let staged: Vec<String> = st
            .staged
            .iter()
            .map(|e| {
                format!(
                    "{}:{}<-{}",
                    e.change,
                    e.path,
                    e.old_path.as_deref().unwrap_or("")
                )
            })
            .collect();
        assert_eq!(
            staged,
            vec!["copied:b<-a", "renamed:c<-a", "modified:m<-", "copied:n<-m"]
        );
        assert_eq!(line_counts(d), (2, 0));
        let sides = worktree_file_sides(d, "b", true).unwrap();
        assert_eq!(sides.original_path.as_deref(), Some("a"));
        assert_eq!(sides.original, body);

        unstage_files(d, &["b".into()]).unwrap();
        assert_eq!(porcelain(d), vec!["R  a -> c", "M  m", "C  m -> n", "?? b"]);
        stage_files(d, &["b".into()]).unwrap();
        discard_files(d, &["b".into(), "n".into()]).unwrap();
        assert_eq!(porcelain(d), vec!["R  a -> c", "M  m"]);
        assert!(!d.join("b").exists() && !d.join("n").exists());
    }
}
