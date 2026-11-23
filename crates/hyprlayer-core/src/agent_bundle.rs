use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::hyprcli::AiStatus;

pub const BUNDLE_REFS_URL: &str =
    "https://github.com/BrightBlock/hyprlayer-cli.git/info/refs?service=git-upload-pack";

const MASTER_REFNAME: &str = "refs/heads/master";
const MANAGEABLE_AGENT_TOOL: &str = "Claude + Codex";
const REQUIRED_PLATFORMS: [&str; 2] = ["claude", "codex"];

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

/// Whether answering "is my bundle behind" needs the network. Only an
/// unversioned CLI does: from 1.6.0 on, `assetsVersion` against the desired
/// version answers it locally and the SHA would answer a different question
/// ("has master moved") — see ADR-0002.
pub fn needs_sha_probe(status: &AiStatus) -> bool {
    is_manageable_pair(status)
        && desired_version(
            status.pinned_version.as_deref(),
            status.binary_version.as_deref(),
        )
        .is_none()
}

pub fn update_available(status: &AiStatus, latest_sha: Option<&str>) -> bool {
    if !is_manageable_pair(status) {
        return false;
    }
    match desired_version(
        status.pinned_version.as_deref(),
        status.binary_version.as_deref(),
    ) {
        Some(desired) => is_stale(status.assets_version.as_deref(), desired),
        None => latest_sha
            .is_some_and(|latest| is_stale(status.agents_installed_sha.as_deref(), latest)),
    }
}

pub fn is_manageable_pair(status: &AiStatus) -> bool {
    status.installed
        && status.agent_tool.as_deref() == Some(MANAGEABLE_AGENT_TOOL)
        && REQUIRED_PLATFORMS.iter().all(|required| {
            status
                .platforms
                .iter()
                .any(|platform| platform.id == *required && platform.installed)
        })
}

pub(crate) fn expand_tilde(path: &str, home: Option<&Path>) -> std::path::PathBuf {
    let expanded = shellexpand::tilde_with_context(path, || home.and_then(Path::to_str));
    std::path::PathBuf::from(expanded.as_ref())
}

/// `YYYYMMDD-HHMMSS` in UTC, for naming a backup the user can read at a glance.
pub fn backup_stamp(epoch_secs: u64) -> String {
    let (days, rem) = (epoch_secs / 86_400, epoch_secs % 86_400);
    let (year, month, day) = crate::store::civil_from_days(days as i64);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Clear every target path blocking a link, across each platform the status reports.
pub fn clear_link_collisions(
    status: &AiStatus,
    stamp: &str,
    home: Option<&Path>,
) -> CollisionReport {
    let (Some(root), Some(version)) = (
        status.location.as_deref(),
        desired_version(
            status.pinned_version.as_deref(),
            status.binary_version.as_deref(),
        ),
    ) else {
        return CollisionReport::default();
    };
    let mut report = CollisionReport::default();
    for platform in &status.platforms {
        let Some(target) = platform.location.as_deref() else {
            continue;
        };
        let target = expand_tilde(target, home);
        // codex is linked its skills out of the claude bundle, and its own manifest never
        // mentions them, so every bundle has to be checked against every target
        for source in &status.platforms {
            let bundle = expand_tilde(root, home).join(version).join(&source.id);
            let one = resolve_collisions(&bundle, &target, stamp);
            report.replaced.extend(one.replaced);
            report.backed_up.extend(one.backed_up);
        }
    }
    report.replaced.sort();
    report.replaced.dedup();
    report.backed_up.sort();
    report.backed_up.dedup();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprcli::AiPlatformStatus;

    fn sha_of(body: &str) -> String {
        format!("{:x}", Sha256::digest(body.as_bytes()))
    }

    /// bundle: skills/keep/SKILL.md + skills/mine/SKILL.md + agents/one.md, settings.json copied
    fn fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let bundle = root.join("bundle");
        let target = root.join("target");
        for (rel, body) in [
            ("skills/keep/SKILL.md", "keep"),
            ("skills/mine/SKILL.md", "mine"),
            ("agents/one.md", "one"),
            ("settings.json", "settings"),
        ] {
            let p = bundle.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
        }
        let files: Vec<String> = [
            ("skills/keep/SKILL.md", "keep"),
            ("skills/mine/SKILL.md", "mine"),
            ("agents/one.md", "one"),
            ("settings.json", "settings"),
        ]
        .iter()
        .map(|(rel, body)| format!(r#"{{"path":"{rel}","sha256":"{}"}}"#, sha_of(body)))
        .collect();
        std::fs::write(
            bundle.join("manifest.json"),
            format!(r#"{{"files":[{}]}}"#, files.join(",")),
        )
        .unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            target.join(TARGET_MANIFEST),
            format!(
                r#"{{"files":[{{"path":"settings.json","sha256":"{}"}}]}}"#,
                sha_of("settings")
            ),
        )
        .unwrap();
        (bundle, target)
    }

    fn plant(target: &Path, rel: &str, body: &str) {
        let p = target.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn backup_stamp_reads_as_a_utc_date() {
        assert_eq!(backup_stamp(0), "19700101-000000");
        assert_eq!(backup_stamp(1_787_851_216), "20260827-172016");
        // a leap day, the case an off-by-one in civil_from_days would break
        assert_eq!(backup_stamp(1_709_164_800), "20240229-000000");
    }

    #[test]
    fn deletes_a_stale_copy_that_already_matches_the_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let (bundle, target) = fixture(tmp.path());
        plant(&target, "skills/keep/SKILL.md", "keep");

        let report = resolve_collisions(&bundle, &target, "20260827");

        assert_eq!(report.replaced, vec!["skills/keep".to_string()]);
        assert!(report.backed_up.is_empty());
        assert!(!target.join("skills/keep").exists());
    }

    #[test]
    fn a_users_extra_file_makes_the_whole_unit_a_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let (bundle, target) = fixture(tmp.path());
        plant(&target, "skills/keep/SKILL.md", "keep");
        plant(&target, "skills/keep/notes.md", "mine alone");

        let report = resolve_collisions(&bundle, &target, "20260827");

        assert!(report.replaced.is_empty());
        assert_eq!(report.backed_up, vec!["skills/keep".to_string()]);
        assert_eq!(
            std::fs::read_to_string(target.join("skills/keep.bak-20260827/notes.md")).unwrap(),
            "mine alone"
        );
    }

    #[test]
    fn never_touches_a_path_the_cli_copies_and_keeps() {
        let tmp = tempfile::tempdir().unwrap();
        let (bundle, target) = fixture(tmp.path());
        plant(&target, "settings.json", "my settings");

        let report = resolve_collisions(&bundle, &target, "20260827");

        assert!(report.is_empty());
        assert_eq!(
            std::fs::read_to_string(target.join("settings.json")).unwrap(),
            "my settings"
        );
    }

    #[cfg(unix)]
    #[test]
    fn leaves_an_already_linked_target_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let (bundle, target) = fixture(tmp.path());
        std::fs::create_dir_all(target.join("skills")).unwrap();
        std::os::unix::fs::symlink(bundle.join("skills/keep"), target.join("skills/keep")).unwrap();

        let report = resolve_collisions(&bundle, &target, "20260827");

        assert!(report.is_empty());
        assert!(std::fs::symlink_metadata(target.join("skills/keep"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn does_nothing_without_a_bundle_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, target) = fixture(tmp.path());
        assert!(resolve_collisions(&tmp.path().join("absent"), &target, "20260827").is_empty());
    }

    const REAL_ADVERTISEMENT: &str = "001e# service=git-upload-pack\n0000015b4c5cf370aed5fffee3b98f741bd6d911965c9033 HEAD\0multi_ack thin-pack side-band side-band-64k ofs-delta shallow deepen-since deepen-not deepen-relative no-progress include-tag multi_ack_detailed allow-tip-sha1-in-want allow-reachable-sha1-in-want no-done symref=HEAD:refs/heads/master filter object-format=sha1 agent=git/github-fc08860132f4-Linux\n00470afb1cdf19d81169bcc77ee1c47756a702e2802e refs/heads/cla-signatures\n003f4c5cf370aed5fffee3b98f741bd6d911965c9033 refs/heads/master\n003e97ce1e1dd3f7b3d44a68951e5748c707220480b2 refs/pull/1/head\n0000";

    const RELEASE_ASSET_STDOUT: &str = "Downloading Claude + Codex agent files...\n  Downloaded 73 files from the v1.6.0 release asset\n";

    const FALLBACK_STDOUT: &str = "Downloading Claude + Codex agent files...\n  Fetching the v1.5.9 release asset failed (GitHub release `v1.5.9` exposes no SHA256 digest for the bundle set.); trying the next source.\n  Downloaded 79 files from the master repo archive\n";

    fn platform(id: &str, installed: bool) -> AiPlatformStatus {
        AiPlatformStatus {
            id: id.to_string(),
            name: id.to_string(),
            installed,
            location: Some(format!("~/.{id}")),
        }
    }

    fn paired_status() -> AiStatus {
        AiStatus {
            agent_tool: Some(MANAGEABLE_AGENT_TOOL.to_string()),
            installed: true,
            platforms: vec![platform("claude", true), platform("codex", true)],
            ..Default::default()
        }
    }

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
        let refusal = "  Fetching the v1.5.9 release asset failed (GitHub release `v1.5.9` exposes no SHA256 digest.); trying the next source.\n  Downloaded 79 files from the master repo archive\n";
        assert_ne!(reinstall_source(refusal), BundleSource::ReleaseAsset);
    }

    #[test]
    fn flags_a_third_source_install_from_the_abandon_marker_alone() {
        let walked = "Downloading Claude + Codex agent files...\n  Fetching the v1.5.9 release asset failed (…); trying the next source.\n  Fetching the master repo archive failed (…); trying the next source.\n  agents/cartographer.md\n";
        assert_eq!(reinstall_source(walked), BundleSource::Fallback);
    }

    #[test]
    fn tolerates_detail_appended_to_the_release_asset_line() {
        let suffixed = "Downloading Claude + Codex agent files...\n  Downloaded 73 files from the v1.6.1 release asset (12 MB)\n";
        assert_eq!(reinstall_source(suffixed), BundleSource::ReleaseAsset);
    }

    #[test]
    fn reports_unrecognized_output_as_unknown_rather_than_guessing() {
        assert_eq!(reinstall_source(""), BundleSource::Unknown);
        assert_eq!(
            reinstall_source("Downloading Claude + Codex agent files...\n"),
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
    fn desired_version_prefers_the_pin_over_the_binary() {
        assert_eq!(desired_version(Some("1.5.9"), Some("1.6.0")), Some("1.5.9"));
        assert_eq!(desired_version(None, Some("1.6.0")), Some("1.6.0"));
        assert_eq!(desired_version(Some("1.5.9"), None), Some("1.5.9"));
        assert_eq!(desired_version(None, None), None);
    }

    #[test]
    fn only_a_complete_reported_pair_is_manageable() {
        assert!(is_manageable_pair(&paired_status()));

        let mut missing_codex = paired_status();
        missing_codex.platforms.pop();
        assert!(!is_manageable_pair(&missing_codex));

        let mut broken_codex = paired_status();
        broken_codex.platforms[1].installed = false;
        assert!(!is_manageable_pair(&broken_codex));
    }

    #[test]
    fn legacy_claude_only_status_is_never_manageable() {
        let status = AiStatus {
            agent_tool: Some("Claude Code".to_string()),
            installed: true,
            platforms: vec![platform("claude", true)],
            ..Default::default()
        };
        assert!(!is_manageable_pair(&status));
    }

    #[test]
    fn a_versioned_cli_answers_freshness_without_the_network() {
        let mut status = paired_status();
        status.binary_version = Some("1.6.0".to_string());
        status.assets_version = Some("1.5.9".to_string());

        assert!(!needs_sha_probe(&status));
        assert!(update_available(&status, None));

        status.assets_version = Some("1.6.0".to_string());
        assert!(!update_available(&status, None));
    }

    #[test]
    fn a_pin_decides_freshness_even_when_the_binary_is_newer() {
        let mut status = paired_status();
        status.binary_version = Some("1.6.0".to_string());
        status.pinned_version = Some("1.5.9".to_string());
        status.assets_version = Some("1.5.9".to_string());

        assert!(!needs_sha_probe(&status));
        assert!(!update_available(&status, None));
    }

    #[test]
    fn only_an_unversioned_cli_reaches_for_the_sha() {
        let status = paired_status();
        assert!(needs_sha_probe(&status));

        let latest = "4c5cf370aed5fffee3b98f741bd6d911965c9033";
        assert!(update_available(&status, Some(latest)));

        let mut current = paired_status();
        current.agents_installed_sha = Some(latest.to_string());
        assert!(!update_available(&current, Some(latest)));
    }

    #[test]
    fn an_unreachable_probe_reports_no_update_rather_than_guessing() {
        assert!(!update_available(&paired_status(), None));
    }

    #[test]
    fn an_unmanageable_pair_is_never_stale_and_never_probes() {
        let mut status = paired_status();
        status.platforms.pop();
        status.binary_version = Some("1.6.0".to_string());
        status.assets_version = Some("1.5.9".to_string());

        assert!(!needs_sha_probe(&status));
        assert!(!update_available(&status, None));
        assert!(!update_available(
            &status,
            Some("4c5cf370aed5fffee3b98f741bd6d911965c9033")
        ));
    }

    fn bundle_at(dir: &Path, files: &[(&str, &str)]) {
        let manifest: Vec<String> = files
            .iter()
            .map(|(rel, body)| {
                let p = dir.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, body).unwrap();
                format!(r#"{{"path":"{rel}","sha256":"{}"}}"#, sha_of(body))
            })
            .collect();
        std::fs::write(
            dir.join("manifest.json"),
            format!(r#"{{"files":[{}]}}"#, manifest.join(",")),
        )
        .unwrap();
    }

    #[test]
    fn clear_link_collisions_checks_every_bundle_against_every_target() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let store = home.join("store");
        bundle_at(
            &store.join("1.6.0/claude"),
            &[
                ("skills/keep/SKILL.md", "keep"),
                ("skills/mine/SKILL.md", "mine"),
            ],
        );
        bundle_at(&store.join("1.6.0/codex"), &[("agents/one.md", "one")]);

        let claude_target = home.join(".claude");
        let codex_target = home.join(".codex");
        plant(&claude_target, "skills/keep/SKILL.md", "keep");
        plant(&codex_target, "skills/mine/SKILL.md", "my own edits");

        let mut status = paired_status();
        status.location = Some(store.to_string_lossy().into_owned());
        status.binary_version = Some("1.6.0".to_string());
        status.platforms[0].location = Some(claude_target.to_string_lossy().into_owned());
        status.platforms[1].location = Some(codex_target.to_string_lossy().into_owned());

        let report = clear_link_collisions(&status, "20260830", Some(home));

        assert_eq!(report.replaced, vec!["skills/keep".to_string()]);
        assert_eq!(report.backed_up, vec!["skills/mine".to_string()]);
        assert!(codex_target
            .join("skills/mine.bak-20260830/SKILL.md")
            .exists());
    }

    #[test]
    fn clear_link_collisions_does_nothing_without_a_location_or_version() {
        let status = paired_status();
        assert!(clear_link_collisions(&status, "20260830", None).is_empty());
    }
}

/// The CLI links every bundle file into its target except the ones it records in the
/// target-side manifest, which it copies once and then keeps.
const TARGET_MANIFEST: &str = ".hyprlayer-manifest.json";

#[derive(Debug, Clone, Deserialize)]
struct ManifestFile {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Manifest {
    #[serde(default)]
    files: Vec<ManifestFile>,
}

fn read_manifest(path: &Path) -> Vec<ManifestFile> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Manifest>(&raw).ok())
        .map(|m| m.files)
        .unwrap_or_default()
}

fn sha256_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// The CLI links whole skill directories but individual agent files, so the unit that has
/// to be cleared is the first two path components — `skills/commit`, `agents/herald.md`.
fn link_unit(rel: &str) -> String {
    let mut parts = rel.split('/');
    match (parts.next(), parts.next()) {
        (Some(a), Some(b)) => format!("{a}/{b}"),
        (Some(a), None) => a.to_string(),
        _ => rel.to_string(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CollisionReport {
    /// Units removed because every file in them matched the bundle byte for byte.
    pub replaced: Vec<String>,
    /// Units renamed aside because their contents differed from the bundle.
    pub backed_up: Vec<String>,
}

impl CollisionReport {
    pub fn is_empty(&self) -> bool {
        self.replaced.is_empty() && self.backed_up.is_empty()
    }
}

fn unit_matches_bundle(unit_dir: &Path, bundle_unit: &Path, expected: &[&ManifestFile]) -> bool {
    let mut seen = 0usize;
    for entry in walkdir::WalkDir::new(unit_dir).follow_links(false) {
        let Ok(entry) = entry else { return false };
        if entry.file_type().is_symlink() || entry.file_type().is_dir() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(unit_dir) else {
            return false;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        // an extra file the bundle does not know about is the user's, never ours to delete
        let Some(want) = expected.iter().find(|f| {
            Path::new(&f.path)
                .strip_prefix(bundle_unit)
                .map(|p| p.to_string_lossy().replace('\\', "/") == rel)
                .unwrap_or(false)
        }) else {
            return false;
        };
        if sha256_file(entry.path()).as_deref() != Some(want.sha256.as_str()) {
            return false;
        }
        seen += 1;
    }
    seen == expected.len()
}

/// Clear the target paths that block the CLI from linking its bundle: delete the ones that
/// already match it byte for byte, rename the rest aside as `<name>.bak-<stamp>`.
pub fn resolve_collisions(bundle_dir: &Path, target: &Path, stamp: &str) -> CollisionReport {
    let bundle_files = read_manifest(&bundle_dir.join("manifest.json"));
    if bundle_files.is_empty() {
        return CollisionReport::default();
    }
    let copied: Vec<String> = read_manifest(&target.join(TARGET_MANIFEST))
        .into_iter()
        .map(|f| f.path)
        .collect();

    let mut units: Vec<String> = Vec::new();
    for file in &bundle_files {
        if copied.iter().any(|c| c == &file.path) {
            continue;
        }
        let unit = link_unit(&file.path);
        if !units.contains(&unit) {
            units.push(unit);
        }
    }

    let mut report = CollisionReport::default();
    for unit in units {
        let path = target.join(&unit);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let expected: Vec<&ManifestFile> = bundle_files
            .iter()
            .filter(|f| link_unit(&f.path) == unit)
            .collect();
        let identical = if meta.is_dir() {
            unit_matches_bundle(&path, Path::new(&unit), &expected)
        } else {
            expected.len() == 1
                && sha256_file(&path).as_deref() == Some(expected[0].sha256.as_str())
        };

        if identical {
            let removed = if meta.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if removed.is_ok() {
                report.replaced.push(unit);
            }
            continue;
        }
        let backup = path.with_file_name(format!(
            "{}.bak-{stamp}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        if std::fs::rename(&path, &backup).is_ok() {
            report.backed_up.push(unit);
        }
    }
    report
}
