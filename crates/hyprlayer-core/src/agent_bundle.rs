use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::agents_dir;
use crate::error::{CoreError, Result};

pub const BUNDLE_REFS_URL: &str =
    "https://github.com/BrightBlock/hyprlayer-cli.git/info/refs?service=git-upload-pack";

const MASTER_REFNAME: &str = "refs/heads/master";

const BUNDLE_ENTRIES: [&str; 3] = ["agents", "skills", "settings.json"];

pub fn parse_master_sha(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let line = line.split('\0').next().unwrap_or(line).trim_end();
        if !line.ends_with(MASTER_REFNAME) {
            return None;
        }
        let idx = line.len() - MASTER_REFNAME.len();
        let sha = line.get(idx.checked_sub(41)?..idx.checked_sub(1)?)?;
        (sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit())).then(|| sha.to_string())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum BundleSource {
    ReleaseAsset,
    Fallback,
    Unknown,
}

pub fn reinstall_source(stdout: &str) -> BundleSource {
    if stdout.contains("trying the next source") {
        return BundleSource::Fallback;
    }
    stdout
        .lines()
        .find(|line| line.trim_start().starts_with("Downloaded "))
        .map(|line| {
            if line.contains("release asset") {
                BundleSource::ReleaseAsset
            } else {
                BundleSource::Fallback
            }
        })
        .unwrap_or(BundleSource::Unknown)
}

pub fn is_stale(installed: Option<&str>, wanted: &str) -> bool {
    installed != Some(wanted)
}

pub fn desired_version<'a>(pinned: Option<&'a str>, binary: Option<&'a str>) -> Option<&'a str> {
    pinned.or(binary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum AgentChange {
    Added,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BundleDiff {
    pub agents: Vec<(String, AgentChange)>,
    pub settings_changed: bool,
    pub skills_changed: usize,
}

fn backup_dir(claude_root: &Path) -> PathBuf {
    claude_root.join("hyprlayer-backup")
}

fn staging_dir(claude_root: &Path) -> PathBuf {
    claude_root.join("hyprlayer-backup.staging")
}

pub fn stage_backup(claude_root: &Path) -> Result<PathBuf> {
    let staging = staging_dir(claude_root);
    let staged = stage_into(claude_root, &staging);
    if staged.is_err() {
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub fn promote_backup(claude_root: &Path) -> Result<PathBuf> {
    let backup = backup_dir(claude_root);
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    std::fs::rename(staging_dir(claude_root), &backup)?;
    Ok(backup)
}

pub fn discard_staged_backup(claude_root: &Path) {
    let _ = std::fs::remove_dir_all(staging_dir(claude_root));
}

fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
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

fn is_bare_stem(stem: &str) -> bool {
    !stem.is_empty() && stem != "." && stem != ".." && !stem.contains(['/', '\\', '\0'])
}

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
                continue;
            };
            match std::fs::read(backed_up_agents.join(name)) {
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

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        (Err(_), Err(_)) => true,
        _ => false,
    }
}

fn count_changed(live: &Path, previous: &Path) -> Result<usize> {
    let mut changed = 0;
    let Ok(entries) = std::fs::read_dir(live) else {
        return Ok(0);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let target = previous.join(entry.file_name());
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

    const REAL_ADVERTISEMENT: &str = "001e# service=git-upload-pack\n0000015b4c5cf370aed5fffee3b98f741bd6d911965c9033 HEAD\0multi_ack thin-pack side-band side-band-64k ofs-delta shallow deepen-since deepen-not deepen-relative no-progress include-tag multi_ack_detailed allow-tip-sha1-in-want allow-reachable-sha1-in-want no-done symref=HEAD:refs/heads/master filter object-format=sha1 agent=git/github-fc08860132f4-Linux\n00470afb1cdf19d81169bcc77ee1c47756a702e2802e refs/heads/cla-signatures\n003f4c5cf370aed5fffee3b98f741bd6d911965c9033 refs/heads/master\n003e97ce1e1dd3f7b3d44a68951e5748c707220480b2 refs/pull/1/head\n0000";

    fn claude_tree(root: &Path) {
        std::fs::create_dir_all(agents_dir(root)).unwrap();
        std::fs::create_dir_all(root.join("skills/code_review")).unwrap();
        std::fs::write(agents_dir(root).join("cartographer.md"), "original\n").unwrap();
        std::fs::write(agents_dir(root).join("foreman.md"), "foreman\n").unwrap();
        std::fs::write(root.join("skills/code_review/SKILL.md"), "review\n").unwrap();
        std::fs::write(root.join("settings.json"), "{\"mine\":true}\n").unwrap();
    }

    const RELEASE_ASSET_STDOUT: &str = "Downloading Claude Code agent files...\n  Downloaded 54 files from the v1.6.0 release asset\n  Kept your modified settings.json\n";

    const FALLBACK_STDOUT: &str = "Downloading Claude Code agent files...\n  Fetching the v1.5.9 release asset failed (GitHub release `v1.5.9` exposes no SHA256 digest for asset `hyprlayer-assets-claude-1.5.9.tar.gz`. Refusing to install an unverified bundle.); trying the next source.\n  Downloaded 60 files from the master repo archive\n";

    #[test]
    fn reads_a_versioned_release_asset_install_as_the_real_thing() {
        assert_eq!(
            reinstall_source(RELEASE_ASSET_STDOUT),
            BundleSource::ReleaseAsset
        );
    }

    #[test]
    fn catches_the_silent_fallback_to_masters_tree() {
        assert_eq!(reinstall_source(FALLBACK_STDOUT), BundleSource::Fallback);
    }

    #[test]
    fn does_not_read_the_failure_text_as_a_release_asset_install() {
        let refusal = "  Fetching the v1.5.9 release asset failed (GitHub release `v1.5.9` exposes no SHA256 digest for asset `hyprlayer-assets-claude-1.5.9.tar.gz`.); trying the next source.\n  Downloaded 60 files from the master repo archive\n";
        assert_ne!(reinstall_source(refusal), BundleSource::ReleaseAsset);
    }

    #[test]
    fn flags_a_third_source_install_from_the_abandon_marker_alone() {
        let walked = "Downloading Claude Code agent files...\n  Fetching the v1.5.9 release asset failed (…); trying the next source.\n  Fetching the master repo archive failed (…); trying the next source.\n  agents/cartographer.md\n";
        assert_eq!(reinstall_source(walked), BundleSource::Fallback);
    }

    #[test]
    fn tolerates_detail_appended_to_the_release_asset_line() {
        let suffixed = "Downloading Claude Code agent files...\n  Downloaded 54 files from the v1.6.1 release asset (12 MB)\n";
        assert_eq!(reinstall_source(suffixed), BundleSource::ReleaseAsset);
    }

    #[test]
    fn reports_unrecognized_output_as_unknown_rather_than_guessing() {
        assert_eq!(reinstall_source(""), BundleSource::Unknown);
        assert_eq!(
            reinstall_source("Downloading Claude Code agent files...\n"),
            BundleSource::Unknown
        );
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
    fn desired_version_prefers_the_pin_over_the_binary() {
        assert_eq!(desired_version(Some("1.5.9"), Some("1.6.0")), Some("1.5.9"));
        assert_eq!(desired_version(None, Some("1.6.0")), Some("1.6.0"));
        assert_eq!(desired_version(Some("1.5.9"), None), Some("1.5.9"));
        assert_eq!(desired_version(None, None), None);
    }

    #[test]
    fn an_unversioned_bundle_under_a_versioned_cli_is_stale() {
        let desired = desired_version(None, Some("1.6.0")).unwrap();
        assert!(is_stale(None, desired));
        assert!(is_stale(Some("1.5.9"), desired));
        assert!(!is_stale(Some("1.6.0"), desired));
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

    #[test]
    #[cfg(unix)]
    fn a_symlinked_skill_directory_is_followed_not_treated_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".claude");
        claude_tree(&root);

        let external = dir.path().join("system-skills/omarchy");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("SKILL.md"), "omarchy\n").unwrap();
        std::os::unix::fs::symlink(&external, root.join("skills/omarchy")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("gone"), root.join("skills/broken")).unwrap();

        let staging = stage_backup(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(staging.join("skills/omarchy/SKILL.md")).unwrap(),
            "omarchy\n"
        );
        assert!(!staging.join("skills/broken").exists());

        std::fs::write(external.join("SKILL.md"), "upstream\n").unwrap();
        let diff = diff_against_backup(&root, &staging).unwrap();
        assert_eq!(diff.skills_changed, 1);
    }

    #[test]
    fn a_failed_update_keeps_the_previous_backup_intact() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        claude_tree(root);

        stage_backup(root).unwrap();
        promote_backup(root).unwrap();
        std::fs::write(agents_dir(root).join("cartographer.md"), "clobbered\n").unwrap();

        stage_backup(root).unwrap();
        discard_staged_backup(root);

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

        std::fs::write(agents_dir(root).join("cartographer.md"), "new prompt\n").unwrap();
        std::fs::write(agents_dir(root).join("newcomer.md"), "brand new\n").unwrap();
        std::fs::write(agents_dir(root).join("notes.txt"), "ignore me\n").unwrap();

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
