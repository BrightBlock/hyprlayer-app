use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum FullDiskAccess {
    Granted,
    Denied,
    Unknown,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn classify(path: &Path) -> FullDiskAccess {
    match std::fs::File::open(path) {
        Ok(_) => FullDiskAccess::Granted,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => FullDiskAccess::Denied,
        Err(_) => FullDiskAccess::Unknown,
    }
}

pub fn full_disk_access(home: Option<&Path>) -> FullDiskAccess {
    #[cfg(target_os = "macos")]
    {
        let Some(home) = home else {
            return FullDiskAccess::Unknown;
        };
        classify(&home.join("Library/Application Support/com.apple.TCC/TCC.db"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = home;
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

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_denied() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.db");
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        if classify(&path) == FullDiskAccess::Granted {
            return;
        }
        assert_eq!(classify(&path), FullDiskAccess::Denied);
    }
}
