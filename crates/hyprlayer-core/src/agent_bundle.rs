use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprcli::AiPlatformStatus;

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
}
