//! PR description records in the thoughts tree.
//!
//! The `describe_pr` skill files each PR's description at
//! `<root>/<reposDir>/<mapped>/shared/prs/<number>_description.md` with the
//! 10-field thought schema in frontmatter. That skill is the only thing that
//! ever wrote the record's `status`, so a PR merged anywhere else — the GitHub
//! UI, `gh pr merge`, a queued auto-merge — left its record advertising an open
//! PR forever. The PR watcher closes that loop: when it sees a PR land it
//! promotes the record here.
//!
//! Filesystem backends only (`git`, `obsidian`). `notion`/`anytype` keep their
//! metadata as typed properties with no local tree to patch, so
//! [`record_path`] has nothing to resolve for them and the watcher skips them.

use std::path::{Path, PathBuf};

use crate::docs::frontmatter::{self, FmPatch, FmValue};
use crate::error::Result;

/// Terminal `status` for a record whose PR has merged.
///
/// Deliberately *not* `merged`: the backend's `status` vocabulary is
/// `draft, active, implemented, superseded, archived` (see the CLI's
/// `schema.rs`, mirrored by `hyprlayer storage info --json`), and `merged` is
/// not in it. `implemented` is the in-schema terminal value.
pub const MERGED_STATUS: &str = "implemented";

/// Statuses a merge is allowed to overwrite.
///
/// `merged` is listed because records written by older skill runs used that
/// out-of-schema value; promoting them folds the strays back into the schema.
/// Anything else — `implemented` already, or a deliberate `superseded` /
/// `archived` — is left alone, as is a record with no `status` at all (the
/// watcher repairs stale values, it does not fill in missing ones).
const PROMOTABLE: [&str; 3] = ["draft", "active", "merged"];

/// Where `describe_pr` files PR `number`'s record on a filesystem backend.
///
/// Mirrors the layout `create_doc` builds for a `shared`-scope `pr` document,
/// plus the skill's `<number>_description.md` filename convention.
pub fn record_path(root: &Path, repos_dir: &str, mapped_name: &str, number: u32) -> PathBuf {
    root.join(repos_dir)
        .join(mapped_name)
        .join("shared")
        .join("prs")
        .join(format!("{number}_description.md"))
}

/// Promote `content`'s `status` to [`MERGED_STATUS`], returning the new file
/// contents — or `None` when the record needs no change.
///
/// Pure; the caller does the IO. Body bytes and every untouched frontmatter
/// span survive verbatim (see [`frontmatter::patch_frontmatter`]).
pub fn promote_merged(content: &str) -> Result<Option<String>> {
    let Some(current) = frontmatter::parse(content).fm.status else {
        return Ok(None);
    };
    if !PROMOTABLE.contains(&current.trim()) {
        return Ok(None);
    }
    let patch = FmPatch {
        set: vec![(
            "status".to_string(),
            FmValue::Str(MERGED_STATUS.to_string()),
        )],
        remove: Vec::new(),
    };
    Ok(Some(frontmatter::patch_frontmatter(content, &patch)?))
}

/// Read the record at `path`, promote it if its PR just merged, and write it
/// back. `Ok(false)` means nothing was written: no record on disk (the PR was
/// never described), or its `status` was already terminal.
///
/// A missing record is the common case — most PRs never get a description —
/// so it is deliberately not an error.
pub fn promote_merged_record(path: &Path) -> Result<bool> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let Some(patched) = promote_merged(&content)? else {
        return Ok(false);
    };
    std::fs::write(path, patched)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(status: &str) -> String {
        format!(
            "---\ntitle: \"PR #216: Ship Linux packages\"\ntype: pr\ndate: 2026-08-15\n\
             status: {status}\nticket:\nproject: hyprlayer-app\nscope: shared\n\
             author: Jon Tsiros\ntags: [linux, aur]\nrelated: []\n---\n\n\
             ## What problem(s) was I solving?\n\nBody stays put.\n"
        )
    }

    #[test]
    fn resolves_the_skill_s_record_path() {
        let p = record_path(Path::new("/t/thoughts"), "repos", "hyprlayer-app", 216);
        assert_eq!(
            p,
            Path::new("/t/thoughts/repos/hyprlayer-app/shared/prs/216_description.md")
        );
    }

    #[test]
    fn promotes_active_and_draft() {
        for stale in ["active", "draft"] {
            let out = promote_merged(&record(stale)).unwrap().expect("promoted");
            assert!(out.contains("status: implemented"));
            assert!(!out.contains(&format!("status: {stale}")));
        }
    }

    #[test]
    fn folds_out_of_schema_merged_back_into_the_schema() {
        let out = promote_merged(&record("merged"))
            .unwrap()
            .expect("promoted");
        assert!(out.contains("status: implemented"));
    }

    #[test]
    fn leaves_terminal_and_deliberate_statuses_alone() {
        for terminal in ["implemented", "superseded", "archived"] {
            assert!(promote_merged(&record(terminal)).unwrap().is_none());
        }
    }

    #[test]
    fn leaves_a_record_with_no_status_alone() {
        let no_status = "---\ntitle: t\ntype: pr\n---\n\nbody\n";
        assert!(promote_merged(no_status).unwrap().is_none());
        assert!(promote_merged("no frontmatter at all\n").unwrap().is_none());
    }

    #[test]
    fn preserves_body_and_every_other_field() {
        let out = promote_merged(&record("active"))
            .unwrap()
            .expect("promoted");
        assert!(out.contains("## What problem(s) was I solving?"));
        assert!(out.contains("Body stays put."));
        assert!(out.contains("title: \"PR #216: Ship Linux packages\""));
        assert!(out.contains("tags: [linux, aur]"));
        assert!(out.contains("author: Jon Tsiros"));
    }

    #[test]
    fn missing_record_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("999_description.md");
        assert!(!promote_merged_record(&missing).unwrap());
    }

    #[test]
    fn round_trips_through_disk_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("216_description.md");
        std::fs::write(&path, record("active")).unwrap();

        assert!(promote_merged_record(&path).unwrap());
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("status: implemented"));

        // Second pass is a no-op — the watcher may see the same merged PR again.
        assert!(!promote_merged_record(&path).unwrap());
        assert_eq!(after, std::fs::read_to_string(&path).unwrap());
    }
}
