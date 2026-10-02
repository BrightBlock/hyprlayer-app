//! Which `hyprlayer-server` releases this app can drive. The server is versioned
//! apart from the app (ADR-0009), so an app update restarts nothing unless it
//! raises `MIN_SERVER_VERSION`.

use semver::Version;

/// The server this app bundles and installs on remote hosts.
pub const SERVER_VERSION: &str = hyprlayer_server::VERSION;

/// The oldest server this app works against. Raise it, and bump the server,
/// when the app starts depending on a server change such as a new command.
pub const MIN_SERVER_VERSION: &str = "0.8.7";

/// Servers up to 0.8.2 send no version; none of them can be told apart.
const UNVERSIONED_SERVER: &str = "0.8.2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Below `MIN_SERVER_VERSION`: this app cannot work against it.
    TooOld,
    /// Usable, but older than the server this app ships.
    Older,
    /// The shipped version or newer.
    Current,
}

/// Build metadata is dropped: a dev build stamped `0.8.3+dev.<sha>` is 0.8.3.
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
    let version = reported
        .and_then(parse)
        .unwrap_or_else(|| known(UNVERSIONED_SERVER));
    if version < known(MIN_SERVER_VERSION) {
        Fit::TooOld
    } else if version < known(SERVER_VERSION) {
        Fit::Older
    } else {
        Fit::Current
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

    #[test]
    fn the_newest_usable_install_ignores_old_and_junk_directories() {
        let installed = ["0.0.1", "not-a-version", MIN_SERVER_VERSION, "999.0.0"];
        assert_eq!(newest_usable(installed), Some("999.0.0"));
        assert_eq!(newest_usable(["0.0.1", "junk"]), None);
    }
}
