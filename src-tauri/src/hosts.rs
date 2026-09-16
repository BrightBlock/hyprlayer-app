//! The remote host list and which one is active, at
//! `<app_config_dir>/hosts.json` — not `settings.json` (ADR-0009, plan Phase 5
//! §2). An older binary tolerates an unknown key on read and erases it on its
//! next write, since `AppSettings` has no flatten-catchall; and a local server
//! already owns wholesale rewrites of `settings.json`, so a second shell-side
//! writer would violate ADR-0002's single-writer rule. This file has exactly
//! one writer: the desktop, and only the desktop ever reads it — deciding
//! which server to reach is necessarily local, since there is no server to
//! ask before you've picked one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// The `ssh` destination, e.g. `nexus` or `jt@nexus.local`.
    pub destination: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostsFile {
    pub hosts: Vec<Host>,
    /// The active host's `destination`, or `None` for the local server.
    pub active: Option<String>,
}

fn hosts_path(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join("hosts.json")
}

/// Tolerant: a missing or unreadable file is an empty list, not an error —
/// there is nothing a fresh profile or a rolled-back binary should fail on.
pub fn load(app_config_dir: &Path) -> HostsFile {
    std::fs::read(hosts_path(app_config_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Tmp-then-rename, like every other config write in this tree
/// (`settings.rs::persist`, `config/mod.rs::write_config_atomic`). No
/// production caller yet — the picker UI that would write a host is deferred
/// (plan Phase 5 §3) — so this exists for the round-trip test below and for
/// that later work to call.
#[allow(dead_code)]
pub fn save(app_config_dir: &Path, file: &HostsFile) -> std::io::Result<()> {
    let path = hosts_path(app_config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// The active host, or `None` when the file names no host (the local server)
/// or names one that isn't in the list.
pub fn active_host(app_config_dir: &Path) -> Option<Host> {
    let file = load(app_config_dir);
    let active = file.active?;
    file.hosts.into_iter().find(|h| h.destination == active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_yields_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let file = load(dir.path());
        assert!(file.hosts.is_empty());
        assert!(file.active.is_none());
        assert!(active_host(dir.path()).is_none());
    }

    #[test]
    fn hosts_json_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let file = HostsFile {
            hosts: vec![
                Host {
                    destination: "nexus".into(),
                    label: "Nexus".into(),
                },
                Host {
                    destination: "jt@office.local".into(),
                    label: "Office".into(),
                },
            ],
            active: Some("nexus".into()),
        };
        save(dir.path(), &file).unwrap();

        assert_eq!(load(dir.path()), file);
        assert_eq!(active_host(dir.path()), Some(file.hosts[0].clone()));
    }

    #[test]
    fn an_active_destination_naming_no_host_is_treated_as_local() {
        let dir = tempfile::tempdir().unwrap();
        save(
            dir.path(),
            &HostsFile {
                hosts: vec![],
                active: Some("gone".into()),
            },
        )
        .unwrap();
        assert!(active_host(dir.path()).is_none());
    }
}
