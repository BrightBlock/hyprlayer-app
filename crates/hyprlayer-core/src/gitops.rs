use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

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

pub(crate) fn status_options() -> git2::StatusOptions {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);
    opts
}

pub(crate) fn upstream_oid(repo: &git2::Repository, head: &git2::Reference) -> Option<git2::Oid> {
    head.shorthand()
        .and_then(|name| repo.find_branch(name, git2::BranchType::Local).ok())
        .and_then(|b| b.upstream().ok())
        .and_then(|u| u.get().target())
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

pub fn worktree_diff_line_counts(repo: &git2::Repository) -> (u32, u32) {
    let Ok(diff) = worktree_diff_raw(repo) else {
        return (0, 0);
    };
    let (mut additions, mut deletions) = (0u32, 0u32);
    let _ = diff.foreach(
        &mut |_delta, _progress| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if crate::worktree::is_thoughts_path(&path) || is_hyprlayer_internal(&path) {
                return true;
            }
            match line.origin() {
                '+' => additions += 1,
                '-' => deletions += 1,
                _ => {}
            }
            true
        }),
    );
    (additions, deletions)
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
    if want_changed || want_diff {
        let files = worktree_diff(worktree).unwrap_or_default();
        if want_changed {
            let (adds, dels) = worktree_diff_line_counts(&repo);
            let n = files.len();
            let plural = if n == 1 { "" } else { "s" };
            vars.changed_files = format!("{n} file{plural} (+{adds} −{dels})");
        }
        if want_diff {
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

fn is_excluded(path: &str) -> bool {
    crate::worktree::is_thoughts_path(path) || is_hyprlayer_internal(path)
}

fn git_err(program: &str, e: git2::Error) -> CoreError {
    CoreError::OutputParse {
        program: program.to_string(),
        detail: e.message().to_string(),
    }
}

fn safe_join(worktree: &Path, rel: &str) -> Result<PathBuf> {
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
            (git2::Status::INDEX_NEW, "added"),
            (git2::Status::INDEX_MODIFIED, "modified"),
            (git2::Status::INDEX_DELETED, "deleted"),
            (git2::Status::INDEX_RENAMED, "renamed"),
            (git2::Status::INDEX_TYPECHANGE, "typechange"),
        ],
    )
}

fn unstaged_change(s: git2::Status) -> Option<&'static str> {
    first_change(
        s,
        &[
            (git2::Status::CONFLICTED, "conflict"),
            (git2::Status::WT_NEW, "untracked"),
            (git2::Status::WT_MODIFIED, "modified"),
            (git2::Status::WT_DELETED, "deleted"),
            (git2::Status::WT_RENAMED, "renamed"),
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

fn delta_old_path(delta: &git2::DiffDelta<'_>, new_path: &str) -> Option<String> {
    if delta.status() != git2::Delta::Renamed {
        return None;
    }
    let old = delta.old_file().path()?.to_string_lossy().to_string();
    (old != new_path).then_some(old)
}

fn included_paths(rels: &[String]) -> Vec<String> {
    rels.iter().filter(|r| !is_excluded(r)).cloned().collect()
}

pub fn worktree_git_status(worktree: &Path) -> Result<WorktreeGitStatus> {
    let repo = open_repo(worktree)?;
    let mut opts = status_options();
    opts.recurse_untracked_dirs(true);
    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| git_err("git2 status", e))?;

    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    for entry in statuses.iter() {
        let path = entry.path().unwrap_or_default().to_string();
        if path.is_empty() || is_excluded(&path) {
            continue;
        }
        let s = entry.status();
        if let Some(change) = staged_change(s) {
            staged.push(GitFileEntry {
                old_path: entry
                    .head_to_index()
                    .and_then(|d| delta_old_path(&d, &path)),
                path: path.clone(),
                change: change.to_string(),
            });
        }
        if let Some(change) = unstaged_change(s) {
            unstaged.push(GitFileEntry {
                old_path: entry
                    .index_to_workdir()
                    .and_then(|d| delta_old_path(&d, &path)),
                path,
                change: change.to_string(),
            });
        }
    }
    Ok(WorktreeGitStatus { staged, unstaged })
}

pub fn stage_files(worktree: &Path, rels: &[String]) -> Result<()> {
    let repo = open_repo(worktree)?;
    let mut index = repo.index().map_err(|e| git_err("git2 index", e))?;
    for rel in rels {
        if is_excluded(rel) {
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
    let owned = included_paths(rels);
    if owned.is_empty() {
        return Ok(());
    }
    match repo.head().and_then(|h| h.peel(git2::ObjectType::Commit)) {
        Ok(commit_obj) => {
            repo.reset_default(Some(&commit_obj), owned.iter().map(|s| s.as_str()))
                .map_err(|e| git_err("git2 reset", e))?;
        }
        Err(_) => {
            let mut index = repo.index().map_err(|e| git_err("git2 index", e))?;
            for rel in &owned {
                let _ = index.remove_path(Path::new(rel));
            }
            index.write().map_err(|e| git_err("git2 index write", e))?;
        }
    }
    Ok(())
}

pub fn discard_files(worktree: &Path, rels: &[String]) -> Result<()> {
    let repo = open_repo(worktree)?;
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    if let Ok(commit_obj) = repo.head().and_then(|h| h.peel(git2::ObjectType::Commit)) {
        let owned = included_paths(rels);
        let _ = repo.reset_default(Some(&commit_obj), owned.iter().map(|s| s.as_str()));
    }

    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    let mut any_tracked = false;
    for rel in rels {
        if is_excluded(rel) {
            continue;
        }
        let abs = safe_join(worktree, rel)?;
        let in_head = head_tree
            .as_ref()
            .map(|t| t.get_path(Path::new(rel)).is_ok())
            .unwrap_or(false);
        if in_head {
            checkout.path(rel);
            any_tracked = true;
        } else if abs.exists() {
            if abs.is_dir() {
                std::fs::remove_dir_all(&abs)?;
            } else {
                std::fs::remove_file(&abs)?;
            }
        }
    }
    if any_tracked {
        repo.checkout_head(Some(&mut checkout))
            .map_err(|e| git_err("git2 checkout", e))?;
    }
    Ok(())
}

fn dirty_tracked_paths(repo: &git2::Repository) -> Result<Vec<String>> {
    let statuses = repo
        .statuses(Some(&mut status_options()))
        .map_err(|e| git_err("git2 status", e))?;
    Ok(statuses
        .iter()
        .filter(|e| !e.status().intersects(git2::Status::WT_NEW))
        .filter_map(|e| e.path().map(String::from))
        .filter(|p| !crate::worktree::is_thoughts_path(p))
        .collect())
}

pub fn discard_tracked_changes(worktree: &Path) -> Result<u32> {
    let repo = open_repo(worktree)?;
    let paths = dirty_tracked_paths(&repo)?;
    if paths.is_empty() {
        return Ok(0);
    }
    discard_files(worktree, &paths)?;
    Ok(paths.len() as u32)
}

fn tree_blob_text(repo: &git2::Repository, tree: &git2::Tree, rel: &Path) -> Option<String> {
    let entry = tree.get_path(rel).ok()?;
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
    let blob = repo.find_blob(entry.id).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

pub fn worktree_file_sides(worktree: &Path, rel: &str, staged: bool) -> Result<FileSides> {
    let abs = safe_join(worktree, rel)?;
    let repo = open_repo(worktree)?;
    let p = Path::new(rel);

    let (original, modified) = if staged {
        (head_blob_text(&repo, p), index_blob_text(&repo, p))
    } else {
        let original = index_blob_text(&repo, p).or_else(|| head_blob_text(&repo, p));
        let modified = std::fs::read(&abs)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).to_string());
        (original, modified)
    };

    Ok(FileSides {
        original_exists: original.is_some(),
        modified_exists: modified.is_some(),
        original: original.unwrap_or_default(),
        modified: modified.unwrap_or_default(),
        original_path: None,
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
    Ok(String::from_utf8_lossy(&bytes).to_string())
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
    walk.set_sorting(git2::Sort::TIME)
        .map_err(|e| git_err("git2 revwalk sort", e))?;
    if walk.push_head().is_err() {
        return Ok(Vec::new());
    }

    let Some(base) = (|| -> Option<git2::Oid> {
        let head = repo.head().ok()?.target()?;
        let base = repo
            .revparse_single(base_branch)
            .ok()?
            .peel_to_commit()
            .ok()?
            .id();
        repo.merge_base(head, base).ok()
    })() else {
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

fn renamed_from(diff: &git2::Diff<'_>, rel: &str) -> Option<String> {
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

        let (additions, deletions) = worktree_diff_line_counts(&repo);
        assert_eq!((additions, deletions), (4, 1));
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
    fn commit_files_reports_a_rename_as_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        let content = "same content, long enough to be detected as similar\n";
        std::fs::write(dir.path().join("a.md"), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        let tree0 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let c0_oid = repo
            .commit(Some("HEAD"), &sig_at(0), &sig_at(0), "init", &tree0, &[])
            .unwrap();
        let c0 = repo.find_commit(c0_oid).unwrap();

        std::fs::remove_file(dir.path().join("a.md")).unwrap();
        std::fs::write(dir.path().join("b.md"), content).unwrap();
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
                "rename",
                &tree1,
                &[&c0],
            )
            .unwrap();

        let files = commit_files(dir.path(), &c1_oid.to_string()).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(files[0].path, "b.md");
        assert_eq!(files[0].change, "renamed");
        assert_eq!(files[0].old_path.as_deref(), Some("a.md"));
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
}
