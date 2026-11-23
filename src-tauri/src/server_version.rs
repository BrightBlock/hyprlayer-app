//! Which `hyprlayer-server` releases this app can drive. The server is versioned
//! apart from the app (ADR-0009), so an app update restarts nothing unless it
//! raises `MIN_SERVER_VERSION`.

use semver::Version;

/// The server this app bundles and installs on remote hosts.
pub const SERVER_VERSION: &str = hyprlayer_server::VERSION;

/// The oldest server this app works against. Raise it, and bump the server,
/// when the app starts depending on a server change such as a new command.
pub const MIN_SERVER_VERSION: &str = "0.8.23";

/// Servers up to 0.8.2 send no version; none of them can be told apart.
const UNVERSIONED_SERVER: &str = "0.8.2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Below `MIN_SERVER_VERSION`: this app cannot work against it.
    TooOld,
    /// Usable, but older than the server this app ships.
    Older,
    /// The shipped release, but a different dev build of it that commit counts
    /// cannot order; only `exact_fit` says so.
    OtherBuild,
    /// The shipped version or newer.
    Current,
}

/// Build metadata is dropped: a dev build stamped `0.8.3+dev.<commits>.<sha>` is 0.8.3.
pub fn parse(version: &str) -> Option<Version> {
    let mut parsed = Version::parse(version.trim()).ok()?;
    parsed.build = semver::BuildMetadata::EMPTY;
    Some(parsed)
}

fn known(version: &str) -> Version {
    parse(version).expect("a version constant is valid semver")
}

/// `reported` is the server's hello `version`; unparseable reads as unversioned.
pub fn fit(reported: Option<&str>) -> Fit {
    fit_against(reported, SERVER_VERSION)
}

/// `fit` for a server this app may restart. Dev installs stamp `+dev.<commits>.<sha>`
/// on an unchanged release version, so only the build metadata tells their builds
/// apart, and only the commit count orders them.
pub fn exact_fit(reported: Option<&str>) -> Fit {
    exact_fit_against(reported, SERVER_VERSION)
}

/// The commit count a dev install stamps; `None` for releases and for the older
/// `+dev.<sha>` stamp, whose sha may itself be all digits.
fn dev_commits(version: &str) -> Option<u64> {
    let version = Version::parse(version.trim()).ok()?;
    match version.build.as_str().split('.').collect::<Vec<_>>()[..] {
        ["dev", commits, _sha] => commits.parse().ok(),
        _ => None,
    }
}

fn fit_against(reported: Option<&str>, shipped: &str) -> Fit {
    let version = reported
        .and_then(parse)
        .unwrap_or_else(|| known(UNVERSIONED_SERVER));
    if version < known(MIN_SERVER_VERSION) {
        Fit::TooOld
    } else if version < known(shipped) {
        Fit::Older
    } else {
        Fit::Current
    }
}

fn exact_fit_against(reported: Option<&str>, shipped: &str) -> Fit {
    let fit = fit_against(reported, shipped);
    let other_build = reported.map(str::trim).is_some_and(|reported| {
        reported != shipped && (reported.contains('+') || shipped.contains('+'))
    });
    if fit != Fit::Current || !other_build {
        return fit;
    }
    match (reported.and_then(dev_commits), dev_commits(shipped)) {
        (Some(reported), Some(shipped)) if reported > shipped => Fit::Current,
        (Some(reported), Some(shipped)) if reported < shipped => Fit::Older,
        _ => Fit::OtherBuild,
    }
}

pub fn describe(reported: Option<&str>) -> String {
    reported.map_or_else(|| format!("{UNVERSIONED_SERVER} or older"), str::to_string)
}

/// The newest installed version this app can run, from a host's version
/// directory names.
pub fn newest_usable<'a>(installed: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let min = known(MIN_SERVER_VERSION);
    installed
        .into_iter()
        .filter_map(|name| parse(name).filter(|v| *v >= min).map(|v| (v, name)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_are_ordered() {
        assert!(known(MIN_SERVER_VERSION) <= known(SERVER_VERSION));
    }

    #[test]
    fn the_bundled_server_is_current_and_build_metadata_is_ignored() {
        assert_eq!(fit(Some(SERVER_VERSION)), Fit::Current);
        assert_eq!(
            fit(Some(&format!("{SERVER_VERSION}+dev.abc123"))),
            Fit::Current
        );
    }

    #[test]
    fn a_server_below_the_minimum_is_too_old() {
        assert_eq!(fit(Some("0.0.1")), Fit::TooOld);
    }

    #[test]
    fn a_newer_server_is_current() {
        assert_eq!(fit(Some("999.0.0")), Fit::Current);
    }

    #[test]
    fn an_unversioned_server_reads_as_the_last_unversioned_release() {
        assert_eq!(fit(None), fit(Some(UNVERSIONED_SERVER)));
        assert_eq!(fit(Some("garbage")), fit(None));
    }

    const RELEASE: &str = "999.1.0";
    const DEV_A: &str = "999.1.0+dev.aaa";
    const DEV_B: &str = "999.1.0+dev.bbb";

    #[test]
    fn a_server_matches_only_its_exact_build() {
        assert_eq!(exact_fit_against(Some(RELEASE), RELEASE), Fit::Current);
        assert_eq!(exact_fit_against(Some(DEV_A), DEV_A), Fit::Current);
        assert_eq!(
            exact_fit_against(Some(" 999.1.0+dev.aaa\n"), DEV_A),
            Fit::Current
        );
    }

    #[test]
    fn an_older_release_is_older_whatever_the_app_build() {
        assert_eq!(exact_fit_against(Some("999.0.0"), RELEASE), Fit::Older);
        assert_eq!(exact_fit_against(Some("999.0.0"), DEV_A), Fit::Older);
    }

    #[test]
    fn a_dev_build_on_either_side_of_the_same_release_is_another_build() {
        assert_eq!(exact_fit_against(Some(DEV_B), DEV_A), Fit::OtherBuild);
        assert_eq!(exact_fit_against(Some(RELEASE), DEV_A), Fit::OtherBuild);
        assert_eq!(exact_fit_against(Some(DEV_A), RELEASE), Fit::OtherBuild);
    }

    const DEV_100: &str = "999.1.0+dev.100.aaaaaaaa";
    const DEV_101: &str = "999.1.0+dev.101.bbbbbbbb";

    #[test]
    fn the_commit_count_orders_dev_builds_of_the_same_release() {
        assert_eq!(exact_fit_against(Some(DEV_101), DEV_100), Fit::Current);
        assert_eq!(exact_fit_against(Some(DEV_100), DEV_101), Fit::Older);
        assert_eq!(exact_fit_against(Some(DEV_100), DEV_100), Fit::Current);
    }

    #[test]
    fn dev_builds_a_commit_count_cannot_order_stay_other_builds() {
        let same_count_other_branch = "999.1.0+dev.100.cccccccc";
        assert_eq!(
            exact_fit_against(Some(same_count_other_branch), DEV_100),
            Fit::OtherBuild
        );
        assert_eq!(exact_fit_against(Some(DEV_A), DEV_100), Fit::OtherBuild);
        assert_eq!(exact_fit_against(Some(DEV_100), DEV_A), Fit::OtherBuild);
        assert_eq!(exact_fit_against(Some(RELEASE), DEV_100), Fit::OtherBuild);
        assert_eq!(
            exact_fit_against(Some("999.1.0+dev.12345678"), DEV_100),
            Fit::OtherBuild
        );
    }

    #[test]
    fn the_exact_rule_leaves_unversioned_and_too_old_servers_alone() {
        assert_eq!(exact_fit_against(None, DEV_A), fit_against(None, DEV_A));
        assert_eq!(
            exact_fit_against(Some("garbage"), DEV_A),
            fit_against(None, DEV_A)
        );
        assert_eq!(exact_fit_against(Some("0.0.1+dev.aaa"), DEV_A), Fit::TooOld);
        assert_eq!(exact_fit_against(Some("0.0.1"), RELEASE), Fit::TooOld);
    }

    #[test]
    fn the_newest_usable_install_ignores_old_and_junk_directories() {
        let installed = ["0.0.1", "not-a-version", MIN_SERVER_VERSION, "999.0.0"];
        assert_eq!(newest_usable(installed), Some("999.0.0"));
        assert_eq!(newest_usable(["0.0.1", "junk"]), None);
    }
}
