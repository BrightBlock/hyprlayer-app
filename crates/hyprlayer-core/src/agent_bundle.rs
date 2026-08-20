//! Freshness and recovery for the agent bundle the hyprlayer CLI installs.
//!
//! The bundle is the `claude/` directory of BrightBlock/hyprlayer-cli@master,
//! synced into `~/.claude` by `hyprlayer ai reinstall`. Freshness is therefore
//! a git SHA comparison, not a version check, and the CLI has no check-only
//! command — so the app resolves `master` itself and compares against the
//! `agentsInstalledSha` the CLI cached. The HTTP half lives in the Tauri shell
//! (ADR-0001 keeps this crate dependency-light); everything here is pure or
//! filesystem-only.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::agents_dir;
use crate::error::{CoreError, Result};

/// The ref advertisement URL to probe. Matches the CLI's own source of truth
/// (`hyprlayer-cli/src/agents.rs`, `REPO`/`fetch_master_sha`) so the app and
/// the CLI can never disagree about which bundle is current.
pub const BUNDLE_REFS_URL: &str =
    "https://github.com/BrightBlock/hyprlayer-cli.git/info/refs?service=git-upload-pack";

/// The one ref this app ever needs out of the advertisement.
const MASTER_REFNAME: &str = "refs/heads/master";

/// Everything `hyprlayer ai reinstall` can overwrite inside `~/.claude`, and
/// therefore everything the backup has to cover.
///
/// `claude/` in the CLI repo is `{agents, skills, settings.json}`, and
/// `install()` runs `sync_tree` over the whole staged tree: every file whose
/// bytes differ is replaced, `settings.json` included. Backing up only
/// `agents/` was a data-loss bug — a user's `permissions`, `env`, `hooks` and
/// `statusLine` live in that one file, and the bundle's copy is 12 lines.
/// `sync_tree` never deletes, so a dest-only file (`settings.local.json`, a
/// hand-written skill) survives an update and needs no backup.
const BUNDLE_ENTRIES: [&str; 3] = ["agents", "skills", "settings.json"];

/// Extract `refs/heads/master`'s SHA from a smart-HTTP ref advertisement.
///
/// Ports `parse_ref_sha` from the CLI (`hyprlayer-cli/src/agents.rs`) so the
/// app and the CLI can never disagree about which bundle is current. Each
/// pkt-line carries a 4-hex length header we never parse: scanning by
/// `.lines()` walks the raw text header and all, and only a line's TAIL is
/// inspected, so the header rides along as harmless leading noise.
///
/// The match requires an exact refname at the line's end (or immediately
/// before the NUL-separated capabilities list on the first advertised ref),
/// so `refs/heads/master` can't be matched by an advertised `master-old`.
pub fn parse_master_sha(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        // Capabilities (if any) trail the refname after a NUL on the first
        // advertised ref; strip them before comparing tails.
        let line = line.split('\0').next().unwrap_or(line).trim_end();
        if !line.ends_with(MASTER_REFNAME) {
            return None;
        }
        // The SHA ends immediately before the space preceding the refname.
        let idx = line.len() - MASTER_REFNAME.len();
        let sha = line.get(idx.checked_sub(41)?..idx.checked_sub(1)?)?;
        (sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit())).then(|| sha.to_string())
    })
}

/// Whether `installed` is behind `latest`. A missing installed SHA is stale:
/// the CLI writes `None` when ref resolution failed during an install, which
/// means we genuinely do not know what is on disk.
pub fn is_stale(installed: Option<&str>, latest: &str) -> bool {
    installed != Some(latest)
}

/// One agent the bundle changed, as observed by diffing the library either
/// side of an install.
///
/// There is deliberately no "the user's own edit was reverted" variant: a
/// before/after snapshot cannot tell that apart from an upstream change (both
/// are the same backup≠current diff) and the CLI exposes no per-file signal
/// that would. Anything not [`Added`](Self::Added) may have been the user's,
/// which is what the report's restore affordance keys off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum AgentChange {
    /// Present after the update but not in the backup: new from the bundle.
    Added,
    /// Present either side, with different bytes.
    Updated,
}

/// What one `hyprlayer ai reinstall` changed, as observed by diffing the
/// bundle either side of the install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BundleDiff {
    /// Per-agent changes, sorted by name.
    pub agents: Vec<(String, AgentChange)>,
    /// Whether `settings.json` was replaced. Surfaced separately rather than
    /// as another row: it is not an agent, and it is the one entry in the
    /// bundle whose previous contents were entirely the user's own.
    pub settings_changed: bool,
    /// How many files under `skills/` were added or replaced.
    pub skills_changed: usize,
}

/// `<claude_root>/hyprlayer-backup`, the rolling pre-update copy.
///
/// Not `agents.bak`: the backup covers [`BUNDLE_ENTRIES`], not just agents,
/// and a name that says "agents" would misdescribe a directory holding the
/// user's `settings.json`. Not dot-prefixed either — a user told to go
/// recover a file by hand should be able to see the directory.
fn backup_dir(claude_root: &Path) -> PathBuf {
    claude_root.join("hyprlayer-backup")
}

/// Where [`stage_backup`] writes, before [`promote_backup`] makes it the
/// backup of record. A sibling of [`backup_dir`] so the promotion is a rename
/// within one filesystem.
fn staging_dir(claude_root: &Path) -> PathBuf {
    claude_root.join("hyprlayer-backup.staging")
}

/// Copy every [`BUNDLE_ENTRIES`] entry under `claude_root` into a staging
/// directory, leaving any existing backup untouched. Returns the staging path.
///
/// Staged rather than written straight to [`backup_dir`] because the update
/// this protects against can fail: overwriting the backup up front means a
/// reinstall that dies on a DNS error still destroys the copy holding the
/// edit the *previous* update reverted, in exchange for nothing. The caller
/// promotes on success and discards on failure.
pub fn stage_backup(claude_root: &Path) -> Result<PathBuf> {
    let staging = staging_dir(claude_root);
    let staged = stage_into(claude_root, &staging);
    if staged.is_err() {
        // Don't leave a half-written staging directory in the user's config
        // tree for a backup that never completed.
        let _ = std::fs::remove_dir_all(&staging);
    }
    staged.map(|()| staging)
}

fn stage_into(claude_root: &Path, staging: &Path) -> Result<()> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;
    for entry in BUNDLE_ENTRIES {
        let src = claude_root.join(entry);
        match std::fs::metadata(&src) {
            Ok(meta) if meta.is_dir() => copy_tree(&src, &staging.join(entry))?,
            Ok(_) => {
                std::fs::copy(&src, staging.join(entry))?;
            }
            // Nothing installed yet, or this entry is a dangling symlink:
            // either way there is nothing to save and nothing to lose. Only
            // for NotFound — a permission or I/O error must NOT read as
            // "nothing to save", which would hand back an empty backup and
            // let the caller reinstall over files it never copied.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Make the staged copy the backup of record, replacing any previous one.
/// Call only once the update it was taken for has actually landed. Returns
/// the backup's path, so a caller can name it to the user without keeping a
/// second copy of the layout.
pub fn promote_backup(claude_root: &Path) -> Result<PathBuf> {
    let backup = backup_dir(claude_root);
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    std::fs::rename(staging_dir(claude_root), &backup)?;
    Ok(backup)
}

/// Drop the staged copy, leaving the previous backup as the one of record.
/// Best-effort: a failure here leaves a stale staging directory that the next
/// [`stage_backup`] clears, which is not worth failing an already-failed
/// update over.
pub fn discard_staged_backup(claude_root: &Path) {
    let _ = std::fs::remove_dir_all(staging_dir(claude_root));
}

/// Recursive copy that FOLLOWS symlinks.
///
/// `entry.file_type()` does not: it reports a symlink as a symlink, so a
/// symlinked directory took the file branch and `fs::copy` failed the whole
/// backup with "the source path is neither a regular file nor a symlink to a
/// regular file". Real case: `~/.claude/skills/omarchy` links to
/// `/usr/share/omarchy/default/agents/skills/omarchy`.
///
/// Following is also the correct semantics, not just the working one. The
/// CLI's `sync_tree` writes THROUGH a symlink, so a link pointing into a tree
/// the user can write is exactly as clobberable as a real file and belongs in
/// the backup.
fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            // A dangling symlink has no contents to save.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.is_dir() {
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, target)?;
        }
    }
    Ok(())
}

/// Whether `stem` is a bare file stem, as [`restore_agent_from_backup`]
/// requires.
///
/// The stem crosses the IPC boundary from the webview, and `Path::join`
/// neither resolves `..` nor keeps its base when handed an absolute path — so
/// an unchecked stem escapes the backup on the read side and `~/.claude` on
/// the write side. Worse, both sides join the same string onto sibling bases,
/// so a traversing stem resolves them to one path, and `std::fs::copy(p, p)`
/// opens the destination `O_TRUNC`: the "restore" would zero the file.
fn is_bare_stem(stem: &str) -> bool {
    !stem.is_empty() && stem != "." && stem != ".." && !stem.contains(['/', '\\', '\0'])
}

/// Restore one agent from the rolling backup, by file stem (`"cartographer"`
/// for `cartographer.md`). Errors when that stem isn't in the backup, either
/// because it was never installed or a later update already replaced the
/// one rolling copy.
pub fn restore_agent_from_backup(claude_root: &Path, stem: &str) -> Result<()> {
    if !is_bare_stem(stem) {
        return Err(CoreError::InvalidPath {
            path: stem.to_string(),
        });
    }
    let file_name = format!("{stem}.md");
    let dest_dir = agents_dir(claude_root);
    std::fs::create_dir_all(&dest_dir)?;
    std::fs::copy(
        agents_dir(&backup_dir(claude_root)).join(&file_name),
        dest_dir.join(&file_name),
    )?;
    Ok(())
}

/// Diff the live bundle under `claude_root` against the copy at `backup_root`
/// (a [`stage_backup`] result, before promotion).
///
/// Each `.md` in `agents/` is classified: present live but not in the backup
/// is [`AgentChange::Added`]; present either side with differing bytes is
/// [`AgentChange::Updated`]; identical bytes are omitted. `settings.json` and
/// `skills/` are counted, not itemised — the report this feeds is the Agents
/// view's.
///
/// Nothing is reported as deleted because nothing can be: the CLI's
/// `sync_tree` only ever writes files present in the staged bundle and leaves
/// dest-only files alone (`sync_tree_preserves_a_dest_only_file`).
pub fn diff_against_backup(claude_root: &Path, backup_root: &Path) -> Result<BundleDiff> {
    let mut agents = Vec::new();
    let live_agents = agents_dir(claude_root);
    let backed_up_agents = agents_dir(backup_root);
    if let Ok(entries) = std::fs::read_dir(&live_agents) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let (Some(stem), Some(name)) =
                (path.file_stem().and_then(|s| s.to_str()), path.file_name())
            else {
                continue;
            };
            let Ok(current) = std::fs::read(&path) else {
                continue; // unreadable: skip rather than fail the whole report
            };
            match std::fs::read(backed_up_agents.join(name)) {
                // Compared byte-for-byte rather than by hash: both buffers are
                // already resident, `!=` short-circuits on length and on the
                // first differing byte, and it cannot collide two genuinely
                // different agents into "unchanged".
                Ok(previous) => {
                    if current != previous {
                        agents.push((stem.to_string(), AgentChange::Updated));
                    }
                }
                Err(_) => agents.push((stem.to_string(), AgentChange::Added)),
            }
        }
    }
    agents.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(BundleDiff {
        agents,
        settings_changed: !same_file(
            &claude_root.join("settings.json"),
            &backup_root.join("settings.json"),
        ),
        skills_changed: count_changed(&claude_root.join("skills"), &backup_root.join("skills"))?,
    })
}

/// Whether both paths hold the same bytes. Two missing files count as the
/// same (nothing changed); one missing does not.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        (Err(_), Err(_)) => true,
        _ => false,
    }
}

/// Files under `live` that are absent from `previous` or differ from it.
fn count_changed(live: &Path, previous: &Path) -> Result<usize> {
    let mut changed = 0;
    let Ok(entries) = std::fs::read_dir(live) else {
        return Ok(0);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let target = previous.join(entry.file_name());
        // Follows symlinks, for the reason `copy_tree` documents: a symlinked
        // skill directory read as a file here would compare two unreadable
        // paths and score "unchanged".
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            changed += count_changed(&path, &target)?;
        } else if !same_file(&path, &target) {
            changed += 1;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real ref advertisement from `git.../hyprlayer-cli.git/info/refs`,
    /// trimmed to a handful of lines: the service header, the first
    /// advertised ref (`HEAD`, carrying the NUL-separated capabilities list),
    /// one decoy branch, `refs/heads/master` itself, and one `refs/pull/*`
    /// ref, closed by the flush-pkt. Captured 2026-08-20.
    const REAL_ADVERTISEMENT: &str = "001e# service=git-upload-pack\n0000015b4c5cf370aed5fffee3b98f741bd6d911965c9033 HEAD\0multi_ack thin-pack side-band side-band-64k ofs-delta shallow deepen-since deepen-not deepen-relative no-progress include-tag multi_ack_detailed allow-tip-sha1-in-want allow-reachable-sha1-in-want no-done symref=HEAD:refs/heads/master filter object-format=sha1 agent=git/github-fc08860132f4-Linux\n00470afb1cdf19d81169bcc77ee1c47756a702e2802e refs/heads/cla-signatures\n003f4c5cf370aed5fffee3b98f741bd6d911965c9033 refs/heads/master\n003e97ce1e1dd3f7b3d44a68951e5748c707220480b2 refs/pull/1/head\n0000";

    /// Lay out a `~/.claude` the way `hyprlayer ai reinstall` leaves one.
    fn claude_tree(root: &Path) {
        std::fs::create_dir_all(agents_dir(root)).unwrap();
        std::fs::create_dir_all(root.join("skills/code_review")).unwrap();
        std::fs::write(agents_dir(root).join("cartographer.md"), "original\n").unwrap();
        std::fs::write(agents_dir(root).join("foreman.md"), "foreman\n").unwrap();
        std::fs::write(root.join("skills/code_review/SKILL.md"), "review\n").unwrap();
        std::fs::write(root.join("settings.json"), "{\"mine\":true}\n").unwrap();
    }

    #[test]
    fn parses_master_sha_from_a_real_advertisement() {
        assert_eq!(
            parse_master_sha(REAL_ADVERTISEMENT).as_deref(),
            Some("4c5cf370aed5fffee3b98f741bd6d911965c9033")
        );
    }

    #[test]
    fn strips_capabilities_when_master_is_the_first_advertised_ref() {
        let body = "001e# service=git-upload-pack\n0000015b4c5cf370aed5fffee3b98f741bd6d911965c9033 refs/heads/master\0multi_ack thin-pack side-band\n0000";
        assert_eq!(
            parse_master_sha(body).as_deref(),
            Some("4c5cf370aed5fffee3b98f741bd6d911965c9033")
        );
    }

    #[test]
    fn a_lookalike_branch_name_does_not_match() {
        let body = "001e# service=git-upload-pack\n0000015b4c5cf370aed5fffee3b98f741bd6d911965c9033 HEAD\0multi_ack\n0047abcd1234abcd1234abcd1234abcd1234abcd12 refs/heads/master-old\n0000";
        assert_eq!(parse_master_sha(body), None);
    }

    #[test]
    fn a_truncated_body_yields_none_rather_than_panicking() {
        assert_eq!(parse_master_sha(""), None);
        assert_eq!(
            parse_master_sha("003f4c5cf370aed5fffee3b98f741bd6d911965"),
            None
        );
        assert_eq!(parse_master_sha("not a git protocol response at all"), None);
    }

    #[test]
    fn is_stale_truth_table() {
        assert!(is_stale(None, "abc"));
        assert!(!is_stale(Some("abc"), "abc"));
        assert!(is_stale(Some("abc"), "def"));
        assert!(is_stale(Some(""), "abc"));
    }

    #[test]
    fn the_backup_covers_settings_and_skills_not_just_agents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);

        let staging = stage_backup(root).unwrap();
        assert_eq!(
            std::fs::read_to_string(staging.join("settings.json")).unwrap(),
            "{\"mine\":true}\n"
        );
        assert_eq!(
            std::fs::read_to_string(staging.join("skills/code_review/SKILL.md")).unwrap(),
            "review\n"
        );
        assert_eq!(
            std::fs::read_to_string(agents_dir(&staging).join("cartographer.md")).unwrap(),
            "original\n"
        );
    }

    /// Regression: `~/.claude/skills/omarchy` is a symlink to
    /// `/usr/share/omarchy/default/agents/skills/omarchy` on an Omarchy
    /// machine. `entry.file_type()` reported it as a symlink rather than a
    /// directory, so the copy took the file branch and `fs::copy` failed the
    /// whole backup with "the source path is neither a regular file nor a
    /// symlink to a regular file" — no backup, and the update refused to run.
    #[test]
    #[cfg(unix)]
    fn a_symlinked_skill_directory_is_followed_not_treated_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".claude");
        claude_tree(&root);

        // A skill living outside the tree, linked in — the Omarchy layout.
        let external = dir.path().join("system-skills/omarchy");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("SKILL.md"), "omarchy\n").unwrap();
        std::os::unix::fs::symlink(&external, root.join("skills/omarchy")).unwrap();
        // A dangling link must not fail the backup either.
        std::os::unix::fs::symlink(dir.path().join("gone"), root.join("skills/broken")).unwrap();

        let staging = stage_backup(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(staging.join("skills/omarchy/SKILL.md")).unwrap(),
            "omarchy\n"
        );
        assert!(!staging.join("skills/broken").exists());

        // And the diff walks it without scoring a directory as an unchanged file.
        std::fs::write(external.join("SKILL.md"), "upstream\n").unwrap();
        let diff = diff_against_backup(&root, &staging).unwrap();
        assert_eq!(diff.skills_changed, 1);
    }

    #[test]
    fn a_failed_update_keeps_the_previous_backup_intact() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);

        // Update #1 lands: its backup holds the user's edit.
        stage_backup(root).unwrap();
        promote_backup(root).unwrap();
        std::fs::write(agents_dir(root).join("cartographer.md"), "clobbered\n").unwrap();

        // Update #2 is triggered and its reinstall fails.
        stage_backup(root).unwrap();
        discard_staged_backup(root);

        // The pre-#1 copy is still the one of record.
        assert_eq!(
            std::fs::read_to_string(agents_dir(&backup_dir(root)).join("cartographer.md")).unwrap(),
            "original\n"
        );
        assert!(!staging_dir(root).exists());
    }

    #[test]
    fn promote_replaces_any_previous_backup_rather_than_accumulating() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);
        stage_backup(root).unwrap();
        promote_backup(root).unwrap();

        std::fs::write(agents_dir(root).join("cartographer.md"), "v2\n").unwrap();
        std::fs::write(agents_dir(root).join("newcomer.md"), "new\n").unwrap();
        stage_backup(root).unwrap();
        promote_backup(root).unwrap();

        let backed_up = agents_dir(&backup_dir(root));
        assert_eq!(
            std::fs::read_to_string(backed_up.join("cartographer.md")).unwrap(),
            "v2\n"
        );
        assert_eq!(
            std::fs::read_to_string(backed_up.join("newcomer.md")).unwrap(),
            "new\n"
        );
    }

    #[test]
    fn back_up_and_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);
        stage_backup(root).unwrap();
        promote_backup(root).unwrap();

        std::fs::write(agents_dir(root).join("cartographer.md"), "edited\n").unwrap();
        restore_agent_from_backup(root, "cartographer").unwrap();
        assert_eq!(
            std::fs::read_to_string(agents_dir(root).join("cartographer.md")).unwrap(),
            "original\n"
        );
        // Untouched files are unaffected by a single-file restore.
        assert_eq!(
            std::fs::read_to_string(agents_dir(root).join("foreman.md")).unwrap(),
            "foreman\n"
        );
    }

    #[test]
    fn restoring_a_stem_missing_from_the_backup_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(agents_dir(root)).unwrap();
        stage_backup(root).unwrap();
        promote_backup(root).unwrap();
        assert!(restore_agent_from_backup(root, "nope").is_err());
    }

    #[test]
    fn a_traversing_stem_is_rejected_rather_than_truncating_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".claude");
        claude_tree(&root);
        stage_backup(&root).unwrap();
        promote_backup(&root).unwrap();

        // Source and destination would resolve to this same path, and
        // `fs::copy(p, p)` truncates.
        let outside = dir.path().join("notes.md");
        std::fs::write(&outside, "precious\n").unwrap();

        for stem in ["../notes", "..", ".", "", "a/b", "a\\b"] {
            assert!(
                restore_agent_from_backup(&root, stem).is_err(),
                "{stem:?} should be rejected"
            );
        }
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "precious\n");
    }

    #[test]
    fn stage_backup_on_a_fresh_install_with_no_prior_bundle_is_an_empty_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let staging = stage_backup(dir.path()).unwrap();
        assert!(staging.is_dir());
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    }

    #[test]
    fn diff_classifies_added_and_updated_and_omits_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);
        let backup = stage_backup(root).unwrap();

        // Updated: present both sides, bytes differ.
        std::fs::write(agents_dir(root).join("cartographer.md"), "new prompt\n").unwrap();
        // Added: only in the post-update library.
        std::fs::write(agents_dir(root).join("newcomer.md"), "brand new\n").unwrap();
        // A non-.md file must never appear in the report.
        std::fs::write(agents_dir(root).join("notes.txt"), "ignore me\n").unwrap();
        // `foreman.md` is untouched, so it must be omitted.

        let diff = diff_against_backup(root, &backup).unwrap();
        assert_eq!(
            diff.agents,
            vec![
                ("cartographer".to_string(), AgentChange::Updated),
                ("newcomer".to_string(), AgentChange::Added),
            ]
        );
        assert!(!diff.settings_changed);
        assert_eq!(diff.skills_changed, 0);
    }

    #[test]
    fn diff_reports_a_replaced_settings_json_and_replaced_skills() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);
        let backup = stage_backup(root).unwrap();

        // What `sync_tree` does to a user's own config.
        std::fs::write(root.join("settings.json"), "{\"theirs\":true}\n").unwrap();
        std::fs::write(root.join("skills/code_review/SKILL.md"), "upstream\n").unwrap();
        std::fs::create_dir_all(root.join("skills/new_skill")).unwrap();
        std::fs::write(root.join("skills/new_skill/SKILL.md"), "added\n").unwrap();

        let diff = diff_against_backup(root, &backup).unwrap();
        assert!(diff.agents.is_empty());
        assert!(diff.settings_changed);
        assert_eq!(diff.skills_changed, 2);
    }

    #[test]
    fn diff_against_a_missing_bundle_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let diff = diff_against_backup(&dir.path().join("nope"), &dir.path().join("also-nope"));
        assert_eq!(
            diff.unwrap(),
            BundleDiff {
                agents: Vec::new(),
                settings_changed: false,
                skills_changed: 0,
            }
        );
    }
}
