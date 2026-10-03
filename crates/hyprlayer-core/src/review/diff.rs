use std::collections::HashMap;
use std::path::Path;

use hyprlayer_models::{FileChangeKind, ReviewSide};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::gitops::{
    delta_old_path, git_err, is_excluded, open_repo, renamed_from, safe_join, MAX_FILE_BYTES,
};

const DELETED_OID: &str = "deleted";
const BINARY_SNIFF_BYTES: usize = 8000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    pub path: String,
    pub old_path: Option<String>,
    pub change: FileChangeKind,
    pub additions: u32,
    pub deletions: u32,
    pub content_oid: String,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileList {
    pub base: String,
    pub merge_base: Option<String>,
    pub base_resolved: bool,
    pub files: Vec<ReviewFile>,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileSides {
    pub original: String,
    pub modified: String,
    pub original_exists: bool,
    pub modified_exists: bool,
    pub original_path: Option<String>,
    pub binary: bool,
    pub too_large: bool,
    pub content_oid: String,
}

fn commit_oid(repo: &git2::Repository, name: &str) -> Option<git2::Oid> {
    Some(repo.revparse_single(name).ok()?.peel_to_commit().ok()?.id())
}

pub(crate) fn resolve_base(repo: &git2::Repository, base_branch: &str) -> Option<git2::Oid> {
    let head = repo.head().ok()?.target()?;
    let base = commit_oid(repo, base_branch)
        .or_else(|| commit_oid(repo, base_branch.strip_prefix("origin/")?))?;
    repo.merge_base(head, base).ok()
}

fn base_tree(repo: &git2::Repository, merge_base: Option<git2::Oid>) -> Option<git2::Tree<'_>> {
    let commit = match merge_base {
        Some(oid) => repo.find_commit(oid).ok()?,
        None => repo.head().ok()?.peel_to_commit().ok()?,
    };
    commit.tree().ok()
}

fn branch_diff<'r>(
    repo: &'r git2::Repository,
    base: Option<&git2::Tree<'_>>,
) -> Result<git2::Diff<'r>> {
    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let mut diff = repo
        .diff_tree_to_workdir_with_index(base, Some(&mut opts))
        .map_err(|e| git_err("git2 review diff", e))?;
    // Explicit, not the repo's `diff.renames`; untracked targets make an unstaged `mv` a rename.
    let mut find = git2::DiffFindOptions::new();
    find.renames(true).for_untracked(true);
    diff.find_similar(Some(&mut find))
        .map_err(|e| git_err("git2 review rename detection", e))?;
    Ok(diff)
}

fn change_kind(status: git2::Delta) -> FileChangeKind {
    match status {
        git2::Delta::Added | git2::Delta::Untracked => FileChangeKind::Added,
        git2::Delta::Modified => FileChangeKind::Modified,
        git2::Delta::Deleted => FileChangeKind::Deleted,
        git2::Delta::Renamed => FileChangeKind::Renamed,
        git2::Delta::Typechange => FileChangeKind::Typechange,
        _ => FileChangeKind::Other,
    }
}

fn content_oid(abs: &Path) -> String {
    if std::fs::symlink_metadata(abs).is_err() {
        return DELETED_OID.to_string();
    }
    git2::Oid::hash_file(git2::ObjectType::Blob, abs)
        .map(|oid| oid.to_string())
        .unwrap_or_default()
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0)
}

pub fn review_changed_files(worktree: &Path, base_branch: &str) -> Result<ReviewFileList> {
    let repo = open_repo(worktree)?;
    let merge_base = resolve_base(&repo, base_branch);
    let tree = base_tree(&repo, merge_base);
    let diff = branch_diff(&repo, tree.as_ref())?;

    let mut files = Vec::new();
    for (idx, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if path.is_empty() || is_excluded(&path) {
            continue;
        }
        let patch = git2::Patch::from_diff(&diff, idx).ok().flatten();
        // Loading a patch can show a stat-dirty file to be unchanged.
        if delta.status() == git2::Delta::Unmodified {
            continue;
        }
        let (additions, deletions) = patch
            .and_then(|p| p.line_stats().ok())
            .map_or((0, 0), |(_, added, removed)| (added as u32, removed as u32));
        files.push(ReviewFile {
            old_path: delta_old_path(&delta, &path),
            change: change_kind(delta.status()),
            additions,
            deletions,
            content_oid: content_oid(&worktree.join(&path)),
            binary: delta.flags().is_binary(),
            path,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(ReviewFileList {
        base: base_branch.to_string(),
        merge_base: merge_base.map(|oid| oid.to_string()),
        base_resolved: merge_base.is_some(),
        additions: files.iter().map(|f| f.additions).sum(),
        deletions: files.iter().map(|f| f.deletions).sum(),
        files,
    })
}

/// One read per `(side, path)`. The base side comes from the tree `list` was diffed
/// against, at a rename's old path; `None` where a side has no file or is too large.
pub(crate) fn side_texts<'p>(
    worktree: &Path,
    list: &ReviewFileList,
    wanted: impl IntoIterator<Item = (ReviewSide, &'p str)>,
) -> HashMap<(ReviewSide, &'p str), Option<String>> {
    let repo = open_repo(worktree).ok();
    let merge_base = list
        .merge_base
        .as_deref()
        .and_then(|oid| git2::Oid::from_str(oid).ok());
    let tree = repo.as_ref().and_then(|repo| base_tree(repo, merge_base));
    let mut texts = HashMap::new();
    for (side, path) in wanted {
        texts.entry((side, path)).or_insert_with(|| match side {
            ReviewSide::Modified => workdir_text(worktree, path),
            ReviewSide::Original => {
                let base_path = list
                    .files
                    .iter()
                    .find(|f| f.path == path)
                    .and_then(|f| f.old_path.as_deref())
                    .unwrap_or(path);
                repo.as_ref()
                    .zip(tree.as_ref())
                    .and_then(|(repo, tree)| base_text(repo, tree, base_path))
            }
        });
    }
    texts
}

fn workdir_text(worktree: &Path, rel: &str) -> Option<String> {
    let abs = safe_join(worktree, rel).ok()?;
    std::fs::metadata(&abs)
        .ok()
        .filter(|m| m.is_file() && m.len() <= MAX_FILE_BYTES)?;
    let bytes = std::fs::read(&abs).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn base_text(repo: &git2::Repository, tree: &git2::Tree<'_>, rel: &str) -> Option<String> {
    let blob = repo
        .find_blob(tree.get_path(Path::new(rel)).ok()?.id())
        .ok()?;
    (blob.size() as u64 <= MAX_FILE_BYTES)
        .then(|| String::from_utf8_lossy(blob.content()).into_owned())
}

pub fn review_file_sides(worktree: &Path, base_branch: &str, rel: &str) -> Result<ReviewFileSides> {
    let abs = safe_join(worktree, rel)?;
    let repo = open_repo(worktree)?;
    let tree = base_tree(&repo, resolve_base(&repo, base_branch));
    let rel_path = Path::new(rel);

    // Only a path the base lacks can be a rename's new name.
    let original_path = match &tree {
        Some(t) if t.get_path(rel_path).is_ok() => None,
        _ => branch_diff(&repo, tree.as_ref())
            .ok()
            .and_then(|diff| renamed_from(&diff, rel)),
    };
    let original_blob = tree
        .as_ref()
        .and_then(|t| {
            t.get_path(original_path.as_deref().map_or(rel_path, Path::new))
                .ok()
        })
        .and_then(|entry| repo.find_blob(entry.id()).ok());

    // Hashed before the read, so a late edit leaves the id stale, never ahead of the text.
    let oid = content_oid(&abs);
    let modified_size = std::fs::metadata(&abs)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len());
    let too_large = original_blob
        .as_ref()
        .is_some_and(|b| b.size() as u64 > MAX_FILE_BYTES)
        || modified_size.is_some_and(|len| len > MAX_FILE_BYTES);

    let original = original_blob
        .as_ref()
        .filter(|_| !too_large)
        .map(|b| b.content());
    let modified = modified_size
        .filter(|_| !too_large)
        .and_then(|_| std::fs::read(&abs).ok());
    let binary = [original, modified.as_deref()]
        .into_iter()
        .flatten()
        .any(looks_binary);
    let text = |bytes: Option<&[u8]>| match bytes {
        Some(bytes) if !binary => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    };

    Ok(ReviewFileSides {
        original: text(original),
        modified: text(modified.as_deref()),
        original_exists: original_blob.is_some(),
        modified_exists: modified_size.is_some(),
        original_path,
        binary,
        too_large,
        content_oid: oid,
    })
}
