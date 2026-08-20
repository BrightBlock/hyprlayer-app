//! Local-only git operations via git2 (no network, push/pull/fetch go
//! through the system git binary so credential helpers keep working).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CoreError, Result};

/// Cap on files served to the viewer/diff sides, refuse rather than stream
/// megabytes of (likely binary) content into the UI.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsRepoStatus {
    pub branch: Option<String>,
    /// Working-tree entries that differ from HEAD (staged + unstaged + untracked).
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
    pub has_remote: bool,
}

/// Open a repository, mapping the failure to a typed error.
pub(crate) fn open_repo(path: &Path) -> Result<git2::Repository> {
    git2::Repository::open(path).map_err(|e| CoreError::OutputParse {
        program: "git2 open".into(),
        detail: format!("{}: {}", path.display(), e.message()),
    })
}

/// The git directory backing `worktree` (`.git/worktrees/<name>/` for a linked
/// worktree, outside the tree). `None` when not a git worktree.
pub fn git_dir(worktree: &Path) -> Option<PathBuf> {
    Some(open_repo(worktree).ok()?.path().to_path_buf())
}

/// Status options every status surface shares: untracked as single entries, submodules excluded.
pub(crate) fn status_options() -> git2::StatusOptions {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);
    opts
}

/// OID of HEAD's configured upstream branch, when one exists.
pub(crate) fn upstream_oid(repo: &git2::Repository, head: &git2::Reference) -> Option<git2::Oid> {
    head.shorthand()
        .and_then(|name| repo.find_branch(name, git2::BranchType::Local).ok())
        .and_then(|b| b.upstream().ok())
        .and_then(|u| u.get().target())
}

/// Local + remote branch shorthands for the base-branch picker, deduped + sorted.
/// Skips the symbolic `origin/HEAD` alias (not a pickable base). Cheap git2; run on the blocking pool.
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

/// Local branch names only, sorted + deduped. The "existing branch"/"in-place" modes
/// check a branch out, so a remote ref like `origin/main` would detach HEAD. They must
/// offer locals only. Cheap git2; run on the blocking pool.
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

/// Remote-tracking branch shorthands (`origin/main`), sorted + deduped, skipping the
/// symbolic `*/HEAD` aliases. The branch picker offers these so a branch that only
/// exists on the remote can be adopted, the caller materializes a local tracking
/// branch before any checkout. Cheap git2; run on the blocking pool.
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

/// The local branch name a remote-tracking shorthand maps to: `origin/feat/x` →
/// `feat/x`. `None` when there's no remote prefix to strip (not a remote ref).
pub fn local_name_for_remote(remote_ref: &str) -> Option<&str> {
    remote_ref
        .split_once('/')
        .map(|(_remote, rest)| rest)
        .filter(|rest| !rest.is_empty())
}

/// Shorthand of the repo's current branch (HEAD); `None` when detached or unborn.
/// The "in-place" mode pre-selects it so attaching to the current branch needs no switch.
pub fn current_branch(repo_path: &Path) -> Option<String> {
    let repo = open_repo(repo_path).ok()?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().map(|s| s.to_string())
}

/// Map a git remote URL to its https web base (e.g. `git@github.com:acme/repo.git`
/// → `https://github.com/acme/repo`). Handles `scheme://[user@]host[:port]/path` and
/// scp-like `[user@]host:path`; `None` for non-web remotes (local paths, `file://`,
/// dotless hosts). Best-effort, only builds the "View on GitHub" link.
fn remote_to_web_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (host, path) = if let Some(rest) = raw
        .strip_prefix("ssh://")
        .or_else(|| raw.strip_prefix("git://"))
        .or_else(|| raw.strip_prefix("https://"))
        .or_else(|| raw.strip_prefix("http://"))
    {
        // scheme://[user@]host[:port]/owner/repo
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host);
        (host.to_string(), path.to_string())
    } else if let Some((authority, path)) = raw.split_once(':') {
        // scp-like: [user@]host:owner/repo. Reject a Windows path (`C:\…`): its
        // backslash in the "path" half must not masquerade as scp-like.
        if path.contains('\\') {
            return None;
        }
        let host = authority.rsplit('@').next().unwrap_or(authority);
        (host.to_string(), path.to_string())
    } else {
        return None;
    };

    // A real web host has a dot; drops `file`, `localhost`, other non-web remotes.
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

/// Best-effort `origin` web URL for a tracked file at the current branch (e.g.
/// `https://github.com/acme/thoughts/blob/main/repos/app/research/x.md`). `None`
/// when not in a repo, no `origin`, detached HEAD, or unmappable remote, which
/// just hides the "View on GitHub" link.
pub fn file_web_url(file_abs: &Path) -> Option<String> {
    // Canonicalize both sides before strip_prefix: the caller's path may run through a
    // symlink (thoughts trees are symlinked in; macOS /var/folders → /private/var) while
    // git2 reports workdir canonically, a raw strip would miss and hide the link.
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

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// added | modified | deleted | renamed | other
    pub change: String,
    pub patch: String,
}

/// App-managed scratch inside a worktree (`.hyprlayer/`, e.g. prompt attachments).
/// Self-ignored from git, but filtered from the diff too so a dropped screenshot
/// never reads as a working-tree change.
fn is_hyprlayer_internal(path: &str) -> bool {
    path == ".hyprlayer" || path.starts_with(".hyprlayer/")
}

/// Raw working-tree-vs-HEAD diff: untracked content included; `thoughts/` filtered by callers.
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

/// Added/deleted line counts for the working-tree diff vs HEAD, excluding `thoughts/`.
/// Summed from the same patches the diff panel renders, so counter and panel can't disagree.
pub fn worktree_diff_line_counts(repo: &git2::Repository) -> (u32, u32) {
    let Ok(diff) = worktree_diff_raw(repo) else {
        return (0, 0);
    };
    let (mut additions, mut deletions) = (0u32, 0u32);
    // Stream line-by-line instead of materializing a `Patch` per file. We only need
    // +/- totals. The thoughts/ + .hyprlayer/ filter uses the same predicates as
    // `worktree_diff`, so the counter can't drift from the panel.
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

/// Working-tree diff vs HEAD (staged + unstaged + untracked content), with
/// the app-managed `thoughts/` tree excluded.
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

/// Char cap for the `{{diff}}` prompt variable, cut (at a patch boundary where
/// possible) with a truncation note so a huge working tree can't balloon a prompt.
const MAX_PROMPT_DIFF_CHARS: usize = 12_000;

/// Git-derived values for prompt `{{variables}}`. Best-effort: any git failure
/// degrades the affected field to an empty string rather than failing the send.
#[derive(Debug, Clone, Default)]
pub struct PromptGitVars {
    /// Working-tree change summary vs HEAD, e.g. `"4 files (+182 −40)"`.
    pub changed_files: String,
    /// Working-tree diff vs HEAD (staged + unstaged + untracked), truncated at
    /// [`MAX_PROMPT_DIFF_CHARS`].
    pub diff: String,
    /// Subject line of the most recent commit.
    pub last_commit: String,
}

/// Resolve git-derived prompt variables for a worktree. Each field is computed only
/// when requested (the diff materializes every patch). Blocking git2; call on the blocking pool.
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
        // Same per-file patches (and thoughts/.hyprlayer filter) the diff panel
        // renders, so the variable and the panel can never disagree.
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
                        // A single oversized patch: cut at a char boundary.
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

// ───────────────────────────────────────────────────────────────────────────
// Source-control panel: split staged/unstaged status, staging actions, the two
// sides of a file for the diff viewer, and a lazy worktree file listing. All of
// it skips the app-managed `thoughts/` and `.hyprlayer/` trees, the same set the
// diff above filters, so the panel never offers to stage infrastructure.
// ───────────────────────────────────────────────────────────────────────────

/// One changed file within a worktree, as it appears in the staged or unstaged
/// list. `change` is one of: added | modified | deleted | renamed | untracked |
/// typechange | conflict | other. `other` is the fallback for deltas none of
/// the surfaces above have a glyph for (copied, ignored, unreadable, conflicted
/// in a tree-to-tree diff).
///
/// `path` is always the file's name *after* the change. `old_path` carries the
/// name before it, and is `Some` only on a detected rename — the panels render
/// `old_path → path` so a rename reads as one file moving rather than as a new
/// file appearing at an unfamiliar name.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GitFileEntry {
    pub path: String,
    pub change: String,
    pub old_path: Option<String>,
}

/// A worktree's working state split the way the panel renders it: what's in the
/// index (staged) vs. what differs between index and working tree (unstaged). A
/// partially-staged file legitimately appears in both lists.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorktreeGitStatus {
    pub staged: Vec<GitFileEntry>,
    pub unstaged: Vec<GitFileEntry>,
}

/// The two texts a CodeMirror merge view diffs. The frontend computes the diff,
/// so we hand it whole-file contents rather than a patch. The `*_exists` flags
/// distinguish an empty file from an absent one (added/deleted), which the
/// viewer uses to force unified mode.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FileSides {
    pub original: String,
    pub modified: String,
    pub original_exists: bool,
    pub modified_exists: bool,
    /// The path the original side was read from, when it differs from the path
    /// asked for — i.e. the pre-rename name. `None` on every other read, so
    /// only a rename's header has anything extra to say.
    pub original_path: Option<String>,
}

/// One immediate child of a directory in the "All Files" tree. `path` is
/// relative to the worktree root (forward-slashed), suitable for the read/diff
/// commands.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// Beyond this many commits we stop walking. A workstream branch is a unit of
/// work, not a history, so this is a backstop against a pathological branch,
/// not a page size — the merge-base bound in `list_commits` is what actually
/// scopes the list, and a workstream that reaches 200 commits is an outlier.
const MAX_COMMITS: usize = 200;

/// One commit on a workstream's branch, as the sidebar renders it. `sha` is the
/// full 40-char id (the frontend abbreviates for display and addresses diffs by
/// the full id); `time_ms` is the AUTHOR time in epoch milliseconds, matching
/// what `git log` shows by default.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CommitEntry {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub time_ms: f64,
}

/// Paths the panel never touches: the app-managed `thoughts/` links and the
/// `.hyprlayer/` scratch. Mirrors the filter the diff above applies.
fn is_excluded(path: &str) -> bool {
    crate::worktree::is_thoughts_path(path) || is_hyprlayer_internal(path)
}

/// Wrap a git2 error as a `CoreError`, tagging the operation that produced it.
fn git_err(program: &str, e: git2::Error) -> CoreError {
    CoreError::OutputParse {
        program: program.to_string(),
        detail: e.message().to_string(),
    }
}

/// Resolve a worktree-relative path to an absolute one, rejecting anything that
/// could escape the worktree (absolute paths, `..`, drive prefixes). Every
/// filesystem-touching command runs its input through this first.
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

/// First matching change label for `s` among `flags`, in priority order. Shared
/// by the staged and unstaged classifiers so they can't drift apart.
fn first_change(s: git2::Status, flags: &[(git2::Status, &'static str)]) -> Option<&'static str> {
    flags
        .iter()
        .find(|(flag, _)| s.contains(*flag))
        .map(|(_, label)| *label)
}

/// The staged change for a status entry (HEAD→index), if any.
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

/// The unstaged change for a status entry (index→worktree), if any.
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

/// Change label for one diff delta. Shared by the working-tree diff and the
/// per-commit file list so the two surfaces can't label the same delta
/// differently. `Typechange` is spelled out because the status classifiers
/// already emit it and the panel has a glyph for it; everything left over
/// (Copied, Ignored, Unreadable, Conflicted) falls through to `other`.
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

/// The pre-rename path of one delta, for `GitFileEntry::old_path`. `None`
/// unless the delta is a detected rename that actually moved the file, so a
/// caller can hand this straight to the entry regardless of the change kind.
fn delta_old_path(delta: &git2::DiffDelta<'_>, new_path: &str) -> Option<String> {
    if delta.status() != git2::Delta::Renamed {
        return None;
    }
    let old = delta.old_file().path()?.to_string_lossy().to_string();
    (old != new_path).then_some(old)
}

/// The subset of `rels` the panel may act on (excluded infrastructure paths
/// dropped), owned so it can move into a blocking closure.
fn included_paths(rels: &[String]) -> Vec<String> {
    rels.iter().filter(|r| !is_excluded(r)).cloned().collect()
}

/// Split staged/unstaged status for the source-control panel.
pub fn worktree_git_status(worktree: &Path) -> Result<WorktreeGitStatus> {
    let repo = open_repo(worktree)?;
    // The panel recurses into brand-new (entirely untracked) directories so each
    // file inside shows up as its own entry, matching how a tracked dir's new
    // files already list, and the line counts we already report from the diff.
    // Collapsed (`recurse_untracked_dirs(false)`) would emit a single `dir/`
    // trailing-slash entry, hiding the nested contents. The count-only surfaces
    // (board chip, thoughts status) keep the collapsed `status_options()` so the
    // always-rendered board never walks a huge untracked tree per card.
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
                // The status walk's own delta names the rename's old side, so
                // the Changes panel renders `old → new` exactly as the Commits
                // panel does. It is `None` in practice today: `status_options`
                // leaves libgit2's rename detection off, so a staged `git mv`
                // still arrives as a deleted plus an added entry. Turning it on
                // is a separate change — the row's stage/unstage/discard
                // buttons act on one path, and a single `renamed` row would
                // strand the other half of the move.
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

/// Stage the given worktree-relative paths (HEAD→index). A path that no longer
/// exists on disk is staged as a deletion. Excluded paths are silently skipped.
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

/// Unstage the given paths (reset index entries back to HEAD). On a repo with no
/// commits yet there's nothing to reset to, so we drop the entries instead.
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

/// Discard working-tree changes for the given paths: tracked files are restored
/// to HEAD (index and working tree both), and added/untracked files are deleted
/// from disk. Destructive, callers confirm first.
pub fn discard_files(worktree: &Path, rels: &[String]) -> Result<()> {
    let repo = open_repo(worktree)?;
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    // Reset any staged version of these paths back to HEAD so the subsequent
    // checkout has a clean index to restore from.
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
            // Newly added / untracked: there's no HEAD version to restore to.
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

/// Paths of the modified *tracked* files, the changes a branch switch would
/// silently carry across. Untracked files are excluded because they ride along
/// harmlessly; the filter mirrors the `dirty` count in [`crate::worktree::status`]
/// so this can't disagree with the guard that reports it.
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

/// Discard every modified tracked file in `worktree` (restore to HEAD), leaving
/// untracked files in place, and report how many were reverted. The escape hatch
/// for an in-place create blocked by [`CoreError::CheckoutDirty`](crate::error::CoreError::CheckoutDirty).
/// Destructive, callers confirm first.
pub fn discard_tracked_changes(worktree: &Path) -> Result<u32> {
    let repo = open_repo(worktree)?;
    let paths = dirty_tracked_paths(&repo)?;
    if paths.is_empty() {
        return Ok(0);
    }
    discard_files(worktree, &paths)?;
    Ok(paths.len() as u32)
}

/// Content of a path inside a tree, when present.
fn tree_blob_text(repo: &git2::Repository, tree: &git2::Tree, rel: &Path) -> Option<String> {
    let entry = tree.get_path(rel).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

/// Content of a path in HEAD's tree, when present.
fn head_blob_text(repo: &git2::Repository, rel: &Path) -> Option<String> {
    let tree = repo.head().ok()?.peel_to_tree().ok()?;
    tree_blob_text(repo, &tree, rel)
}

/// Content of a path in the index (staged), when present.
fn index_blob_text(repo: &git2::Repository, rel: &Path) -> Option<String> {
    let index = repo.index().ok()?;
    let entry = index.get_path(rel, 0)?;
    let blob = repo.find_blob(entry.id).ok()?;
    Some(String::from_utf8_lossy(blob.content()).to_string())
}

/// The two file texts the diff viewer compares. For a staged file that's HEAD vs
/// index; for an unstaged file it's index (falling back to HEAD) vs the working
/// tree. The viewer renders the diff between them.
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
        // Working-tree reads diff a path against itself, so there is never a
        // second path to name. A staged rename's old side is an accepted gap:
        // the Changes panel labels the row `old → new` from the status walk,
        // but the diff still opens against the new name alone.
        original_path: None,
    })
}

/// Immediate children of `rel` within the worktree (one level, the tree loads
/// lazily). Skips `.git`, the excluded trees, and gitignored entries; lists
/// directories before files, alphabetically.
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

/// Worktree-relative file paths matching `query`, for the composer's `@file`
/// autocomplete. Matching is a case-insensitive subsequence test against the
/// path (so `srcsess` finds `src/features/session/...`), ranked so that hits in
/// the file name and shorter/shallower paths come first. Directories, `.git`,
/// git-ignored files, and the app's own `thoughts/`+`.hyprlayer/` infra are
/// skipped. The walk is capped so a giant repo can't stall the keystroke that
/// triggered it; `limit` bounds the returned set.
pub fn search_files(worktree: &Path, query: &str, limit: usize) -> Result<Vec<String>> {
    // Beyond this many filesystem entries we stop walking and rank what we have,
    // an autocomplete needs to be fast, not exhaustive.
    const MAX_VISITED: usize = 20_000;

    let repo = open_repo(worktree).ok();
    let needle = query.to_lowercase();

    let mut scored: Vec<(i32, String)> = Vec::new();

    // `.take` caps how many filesystem entries we visit so a giant repo can't
    // stall the keystroke; `.filter_entry` prunes `.git` whole rather than
    // descending into it.
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

    // Higher score first; ties broken by shorter path (shallower, closer match).
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
    scored.truncate(limit);
    Ok(scored.into_iter().map(|(_, p)| p).collect())
}

/// Rank a candidate path against a lowercase query. Returns `None` when the
/// query isn't a subsequence of the path. A contiguous substring scores higher
/// than a scattered subsequence, and a match within the file name (after the
/// last `/`) is boosted over one buried in the directory portion.
fn subsequence_score(path: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let lower = path.to_lowercase();
    // Strong signal: contiguous substring match, boosted when it lands in the
    // file name (at/after the last slash) rather than the directory portion.
    if let Some(pos) = lower.find(needle) {
        let filename_start = lower.rfind('/').map(|s| s + 1).unwrap_or(0);
        let in_filename = pos >= filename_start;
        return Some(1000 + if in_filename { 500 } else { 0 } - pos as i32);
    }
    // Weaker signal: characters appear in order but not adjacent.
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

/// Read a worktree file as text (utf8-lossy), refusing oversized files so the
/// viewer never tries to render megabytes of binary content.
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

/// Worktree file mtime in ms since the Unix epoch, the editor's save concurrency
/// token. Path-checked; `0.0` when unavailable.
pub fn file_mtime_ms(worktree: &Path, rel: &str) -> Result<f64> {
    let abs = safe_join(worktree, rel)?;
    let modified = std::fs::metadata(&abs)?.modified()?;
    Ok(modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0))
}

/// Write `content` to a worktree file via a temp-file + rename so a reader never
/// sees a half-written file. Path-checked; creates or overwrites.
pub fn write_text_file(worktree: &Path, rel: &str, content: &str) -> Result<()> {
    let abs = safe_join(worktree, rel)?;
    // Same-directory temp so the rename is atomic; hidden + .hyprtmp so it's ignored.
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

/// The commits a workstream added on top of its base: a revwalk from HEAD with
/// the merge-base of (`base_branch`, HEAD) hidden, newest first, capped at
/// [`MAX_COMMITS`].
///
/// The tab answers "what did this workstream commit", so a commit is only ever
/// listed when it is provably on this side of the divergence. If the base does
/// not resolve (it was deleted, or a fresh clone never fetched it) there is no
/// divergence point to measure from, and the list is empty rather than a walk
/// of whatever history HEAD happens to sit on — showing the project's commits
/// as if the workstream had made them is worse than showing none. Resolution
/// stays best-effort and never errors, the same posture as the ahead/behind
/// counts in `worktree::status`. An in-place workstream sitting on the base
/// branch itself has merge-base == HEAD and correctly yields nothing.
///
/// An unborn HEAD (a repo initialized but never committed to) yields an empty
/// list, not an error: it genuinely has no commits, and every command on the
/// `with_worktree` path carries the "the worktree may have been deleted;
/// archive this workstream" remedy, which would be actively wrong advice here.
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
    // Hiding the merge-base is the only bound on this walk, so a failure here is
    // not a detail to shrug off: the walk would run to the root and report the
    // project's whole history as the workstream's own commits. Same answer as an
    // unresolvable base above — no bound, no list.
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

/// Resolve a commit-ish to a commit. `revparse_single` rather than
/// `Oid::from_str` so a short sha resolves too; every call site is a read.
fn commit_at<'r>(repo: &'r git2::Repository, sha: &str) -> Result<git2::Commit<'r>> {
    repo.revparse_single(sha)
        .and_then(|o| o.peel_to_commit())
        .map_err(|e| git_err("git2 commit", e))
}

/// Files one commit changed, diffed against its FIRST parent. A merge therefore
/// reports what the merge itself introduced relative to the branch it landed on,
/// not the union of both sides. A root commit (no parents) diffs against an
/// empty tree, so everything reads as added.
///
/// Applies the same `thoughts/` + `.hyprlayer/` filter as the working-tree
/// surfaces, so a commit that touched the app's own infra lists the same files
/// the Changes panel would have listed for it.
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

/// One commit's first-parent tree diff, with rename detection already run.
/// A root commit (no parents) diffs against an empty tree.
///
/// Without `find_similar` a tree-to-tree diff never emits `Delta::Renamed` — a
/// rename arrives as an Added plus a Deleted, while the Changes tab (which
/// reads git2's status flags, INDEX_RENAMED/WT_RENAMED) calls the same file
/// renamed. `git show` has `diff.renames` on by default for the same reason.
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

/// The path `rel` was renamed from within this diff, if the commit renamed it.
fn renamed_from(diff: &git2::Diff<'_>, rel: &str) -> Option<String> {
    diff.deltas().find_map(|delta| {
        (delta.new_file().path() == Some(Path::new(rel)))
            .then(|| delta_old_path(&delta, rel))
            .flatten()
    })
}

/// The two file texts the diff viewer compares for a commit: the path's content
/// in the commit's first parent vs. its content in the commit. A path the commit
/// renamed reads its original side at the pre-rename name, reported back as
/// `original_path`, so the edits that came with the move are visible.
///
/// No `safe_join` here, unlike the working-tree readers: this touches only git
/// trees, never the filesystem, and a path that escapes simply misses both trees
/// and reports absent-on-both.
pub fn commit_file_sides(worktree: &Path, sha: &str, rel: &str) -> Result<FileSides> {
    let repo = open_repo(worktree)?;
    let commit = commit_at(&repo, sha)?;
    let p = Path::new(rel);

    // A renamed file is asked for under its new name, which the parent tree has
    // never heard of: reading the original side at `rel` reports absent and the
    // viewer draws a wholly-added file, hiding whatever edits rode along with
    // the move. The old name is re-derived from the same diff `commit_files`
    // already ran rather than threaded down from the row that was clicked —
    // that would put a second path through the IPC signature, the query key and
    // the persisted tab model, for one extra tree diff per file opened.
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
        // `origin/feat/x` → the local branch `git checkout feat/x` would create.
        assert_eq!(local_name_for_remote("origin/feat/x"), Some("feat/x"));
        assert_eq!(local_name_for_remote("upstream/main"), Some("main"));
        // Not a remote shorthand: nothing to strip.
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
        // A branch that exists only on the remote, a colleague's pushed work.
        git(&origin, &["branch", "linux-support"]);
        git(
            tmp.path(),
            &[
                "clone",
                origin.to_str().unwrap(),
                repo_dir.to_str().unwrap(),
            ],
        );

        // Only `main` is checked out locally; the other branch is remote-only.
        assert_eq!(list_local_branches(&repo_dir).unwrap(), vec!["main"]);
        let remotes = list_remote_branches(&repo_dir).unwrap();
        assert!(remotes.contains(&"origin/linux-support".to_string()));
        // `origin/HEAD` is a symbolic alias, never a branch you can adopt.
        assert!(!remotes.iter().any(|r| r.ends_with("/HEAD")), "{remotes:?}");
        // The combined base list still carries both kinds.
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
        // The app's thoughts/ infra is never offered.
        assert!(!hits.iter().any(|p| p.starts_with("thoughts")));

        // Subsequence matching: scattered chars in order still match.
        let sub = search_files(dir.path(), "ssnchat", 10).unwrap();
        assert!(sub.contains(&"src/session/chat.rs".to_string()));

        // An empty query lists files (capped), never errors.
        assert!(!search_files(dir.path(), "", 10).unwrap().is_empty());
    }

    #[test]
    fn git_dir_resolves_linked_worktree_metadata_outside_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        let repo = git2::Repository::init(&main).unwrap();
        // A worktree needs a valid HEAD to branch from.
        let sig = git2::Signature::now("t", "t@t.t").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let wt = tmp.path().join("wt");
        repo.worktree("feat", &wt, None).unwrap();

        // Git dir is the main repo's per-worktree metadata, outside the tree.
        let gd = git_dir(&wt).unwrap();
        assert!(
            !gd.starts_with(&wt),
            "git dir {gd:?} must be outside the worktree"
        );
        assert!(gd.ends_with("worktrees/feat"), "unexpected git dir: {gd:?}");
        // watch_worktree's discriminator: gitfile vs. real `.git` dir.
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

        // a.md: drop "two", append "four"+"five" -> 2 additions, 1 deletion.
        std::fs::write(dir.path().join("a.md"), "one\nthree\nfour\nfive\n").unwrap();
        // b.md untracked: 2 added lines.
        std::fs::write(dir.path().join("b.md"), "x\ny\n").unwrap();
        // thoughts/ must not contribute.
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

    /// Init a repo with a committed `a.md` ("one\n") and return its tempdir.
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
        // a.md modified in the working tree; b.md untracked.
        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "new\n").unwrap();

        let status = worktree_git_status(dir.path()).unwrap();
        assert!(status.staged.is_empty());
        assert!(entry_paths(&status.unstaged).contains(&"a.md"));
        let b = status.unstaged.iter().find(|e| e.path == "b.md").unwrap();
        assert_eq!(b.change, "untracked");

        // Stage both, then they move to the staged list.
        stage_files(dir.path(), &["a.md".into(), "b.md".into()]).unwrap();
        let status = worktree_git_status(dir.path()).unwrap();
        assert!(entry_paths(&status.staged).contains(&"a.md"));
        let b = status.staged.iter().find(|e| e.path == "b.md").unwrap();
        assert_eq!(b.change, "added");

        // Unstage reverses it.
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
        // A brand-new directory tree with no tracked ancestor. Git collapses this
        // to a single `pkg/` entry by default; the panel must list each file.
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
        // No collapsed directory entry (which would carry a trailing slash and
        // render as a nameless row in the tree view).
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

    /// The in-place create guard counts only modified tracked files, so its
    /// escape hatch must clear exactly those, an untracked file the user is
    /// mid-way through writing is not what blocked the branch switch.
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
        // Clean tree is a no-op, not an error.
        assert_eq!(discard_tracked_changes(dir.path()).unwrap(), 0);
    }

    #[test]
    fn file_sides_for_modified_file() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("a.md"), "two\n").unwrap();

        // Unstaged: index/HEAD ("one") vs working tree ("two").
        let sides = worktree_file_sides(dir.path(), "a.md", false).unwrap();
        assert_eq!(sides.original, "one\n");
        assert_eq!(sides.modified, "two\n");
        assert!(sides.original_exists && sides.modified_exists);

        // Stage it, then staged sides are HEAD ("one") vs index ("two").
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
        // Directories sort before files.
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
        // The write renames its temp away, leaving no stray file behind.
        assert!(!dir.path().join(".a.md.hyprtmp").exists());
        // mtime moves forward, so a stale save can be detected.
        assert!(file_mtime_ms(dir.path(), "a.md").unwrap() >= before);

        // A brand-new file is created.
        write_text_file(dir.path(), "fresh.txt", "hi").unwrap();
        assert_eq!(read_text_file(dir.path(), "fresh.txt").unwrap(), "hi");

        // Path traversal is refused on both the writer and the mtime probe.
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
        // http upgrades to https; a :port in the authority is dropped.
        assert_eq!(
            web("http://git.example.com:443/team/docs.git").as_deref(),
            Some("https://git.example.com/team/docs")
        );
        // Local paths and non-web hosts don't map.
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

        // No origin remote → no link.
        let bare = repo_with_commit();
        assert_eq!(file_web_url(&bare.path().join("a.md")), None);
    }

    // `Signature::now` has one-second resolution, so commits built back to back
    // share a timestamp and the Sort::TIME walk may emit them in either order.
    // An explicit clock makes both the order and the asserted `time_ms` exact.
    const T0: i64 = 1_700_000_000;
    fn sig_at(n: i64) -> git2::Signature<'static> {
        git2::Signature::new("t", "t@t", &git2::Time::new(T0 + n, 0)).unwrap()
    }

    #[test]
    fn commits_stop_at_the_merge_base_with_the_base_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Base commit at sig_at(0), pinned to an explicit `base` branch so the
        // test doesn't depend on the machine's init.defaultBranch.
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

        // Two more commits on HEAD, on top of the base.
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
        // No divergence point means nothing is provably the workstream's; the
        // project's own commits must not be listed as if it had made them.
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

        // An in-place workstream sitting on its own base branch: merge-base ==
        // HEAD, so nothing is provably on top of it. Accepted gap, pinned here.
        assert!(list_commits(dir.path(), "base").unwrap().is_empty());
    }

    #[test]
    fn commits_are_empty_on_an_unborn_head() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        // A repo initialized but never committed to genuinely has no commits;
        // this must never reach the shared "archive this workstream" remedy.
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

        // Without `find_similar` this sees two entries (added b.md + deleted
        // a.md) and the assertion below fails, which is the point.
        let files = commit_files(dir.path(), &c1_oid.to_string()).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(files[0].path, "b.md");
        assert_eq!(files[0].change, "renamed");
        // The name the panel renders to the left of the arrow.
        assert_eq!(files[0].old_path.as_deref(), Some("a.md"));
    }

    #[test]
    fn commit_file_sides_diffs_a_rename_against_its_old_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Similar enough for `find_similar` to call it a rename, but not
        // identical: the edited line is exactly what a rename read against the
        // new path in the parent tree would swallow.
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

        // Read against `b.md` alone the parent tree misses, the file reports as
        // wholly added and the `FOUR` edit is invisible.
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

        // Trees are built directly (no working-directory checkout needed): A is
        // the common base, B adds b.md on top of A, C adds c.md on top of A on a
        // divergent side branch.
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

        // Merge commit M, tree == B's tree: M didn't actually merge anything in,
        // we're only testing which parent `commit_files` diffs against.
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
        // M's tree equals B's tree: diffing against B (parent 0) yields nothing.
        // Diffing against C (parent 1) would show b.md added and c.md deleted.
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
        // Not a rename, so nothing extra for the header to say.
        assert_eq!(sides.original_path, None);

        // The root commit (repo_with_commit's own initial commit) has no
        // parent, so the original side is absent.
        let root_sha = parent.id().to_string();
        let root_sides = commit_file_sides(dir.path(), &root_sha, "a.md").unwrap();
        assert!(!root_sides.original_exists);
        assert!(root_sides.modified_exists);
        assert_eq!(root_sides.modified, "one\n");
    }
}
