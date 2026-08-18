//! Host-OS capability probes.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Whether this process can read files macOS gates behind Full Disk Access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum FullDiskAccess {
    Granted,
    /// The probe returned EPERM. Either the grant was never added, or it was
    /// recorded against a code signature this build no longer matches, in
    /// which case System Settings still shows the toggle on while every gated
    /// read fails.
    Denied,
    /// Not a macOS gate, or the probe file is missing. Never nag on this.
    Unknown,
}

/// `open()` (never a read) of a path only Full Disk Access can reach.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn classify(path: &Path) -> FullDiskAccess {
    match std::fs::File::open(path) {
        Ok(_) => FullDiskAccess::Granted,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => FullDiskAccess::Denied,
        Err(_) => FullDiskAccess::Unknown,
    }
}

/// Probe for Full Disk Access.
///
/// macOS never prompts for this grant. It can only be added by hand in System
/// Settings, and a process without it just gets EPERM. So the app has to detect
/// the absence itself. Without it, CLIs the app hosts in PTYs (`op`, and
/// anything else reaching another app's container) trip the promptable
/// `kTCCServiceSystemPolicyAppData` dialog instead, which names *this* app
/// because macOS attributes a child's access to its responsible process.
///
/// `TCC.db` is the canonical probe: it always exists and is readable only with
/// the grant.
pub fn full_disk_access() -> FullDiskAccess {
    #[cfg(target_os = "macos")]
    {
        let Some(home) = dirs::home_dir() else {
            return FullDiskAccess::Unknown;
        };
        classify(&home.join("Library/Application Support/com.apple.TCC/TCC.db"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        FullDiskAccess::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_file_is_granted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.db");
        std::fs::write(&path, b"x").unwrap();
        assert_eq!(classify(&path), FullDiskAccess::Granted);
    }

    #[test]
    fn missing_probe_is_unknown_not_denied() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            classify(&dir.path().join("absent.db")),
            FullDiskAccess::Unknown
        );
    }

    /// EPERM, the state a stale or absent grant produces.
    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_denied() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.db");
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        // root ignores the mode bits; nothing to assert there.
        if classify(&path) == FullDiskAccess::Granted {
            return;
        }
        assert_eq!(classify(&path), FullDiskAccess::Denied);
    }
}
