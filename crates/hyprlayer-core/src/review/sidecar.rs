//! The agent's review notes: a JSON Lines file per workstream, git-excluded and read by byte offset.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use hyprlayer_models::review::ReviewNoteKind;
use serde::Deserialize;

use crate::gitops::{common_dir, is_excluded, safe_join};
use crate::pipeline::review_notes_path;

const REVIEW_RULE: &str = "/.hyprlayer/review/";

pub struct SidecarNote {
    pub fingerprint: String,
    pub path: String,
    pub line: u32,
    pub kind: ReviewNoteKind,
    pub body: String,
    pub task: Option<String>,
}

#[derive(Deserialize)]
struct RawNote {
    path: String,
    line: u32,
    kind: String,
    body: String,
    #[serde(default)]
    task: Option<String>,
}

/// Never `/.hyprlayer/`: `settings.toml` is tracked.
pub fn ensure_review_dir_excluded(worktree: &Path) -> std::io::Result<()> {
    let repo = git2::Repository::open(worktree).map_err(std::io::Error::other)?;
    let exclude = common_dir(&repo).join("info").join("exclude");
    let Some(parent) = exclude.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == REVIEW_RULE) {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&format!("# hyprlayer: agent review notes\n{REVIEW_RULE}\n"));
    std::fs::write(&exclude, next)
}

/// The notes in the complete lines past `offset`, and the offset after them. A
/// trailing partial line waits for the next read; a file shorter than `offset`
/// was recreated and is read from the start.
pub fn read_new_notes(
    worktree: &Path,
    workstream_id: &str,
    offset: u64,
) -> (Vec<SidecarNote>, u64) {
    let path = worktree.join(review_notes_path(workstream_id));
    let Ok(mut file) = std::fs::File::open(path) else {
        return (Vec::new(), 0);
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let start = if len < offset { 0 } else { offset };
    let mut tail = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut tail).is_err() {
        return (Vec::new(), start);
    }
    let Some(end) = tail.iter().rposition(|&b| b == b'\n') else {
        return (Vec::new(), start);
    };
    let notes = tail[..end]
        .split(|&b| b == b'\n')
        .filter_map(|line| parse_line(worktree, line))
        .collect();
    (notes, start + end as u64 + 1)
}

fn parse_line(worktree: &Path, line: &[u8]) -> Option<SidecarNote> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let skip = |why: &str| {
        tracing::debug!("review notes: skipped a line: {why}");
        None
    };
    let raw: RawNote = match serde_json::from_slice(line) {
        Ok(raw) => raw,
        Err(e) => return skip(&e.to_string()),
    };
    if raw.path.is_empty() || raw.line < 1 || raw.body.trim().is_empty() {
        return skip("empty path or body, or a line below 1");
    }
    if safe_join(worktree, &raw.path).is_err() || is_excluded(&raw.path) {
        return skip("a path outside the review");
    }
    let Some(kind) = ReviewNoteKind::try_parse(&raw.kind) else {
        return skip("an unknown kind");
    };
    let joined = format!("{}\0{}\0{}\0{}", raw.path, raw.line, raw.kind, raw.body);
    let Ok(fingerprint) = git2::Oid::hash_object(git2::ObjectType::Blob, joined.as_bytes()) else {
        return skip("an unhashable line");
    };
    Some(SidecarNote {
        fingerprint: fingerprint.to_string(),
        path: raw.path,
        line: raw.line,
        kind,
        body: raw.body,
        task: raw.task.filter(|t| !t.trim().is_empty()),
    })
}
