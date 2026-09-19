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

use hyprlayer_server::socket::profile_hash;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    /// The `ssh` destination, e.g. `nexus` or `jt@nexus.local`.
    pub destination: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(default, rename_all = "camelCase")]
pub struct HostsFile {
    pub hosts: Vec<Host>,
    /// The active host's `destination`, or `None` for the local server.
    pub active: Option<String>,
    /// With no active host, another profile's data dir on this machine whose
    /// running server to attach to in place of this app's own.
    pub local_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Own,
    /// Never spawned or evicted: it belongs to whoever started it.
    LocalProfile(PathBuf),
    Remote(Host),
}

impl Target {
    pub fn profile_id(&self) -> String {
        match self {
            Target::Own => LOCAL_PROFILE.to_string(),
            Target::LocalProfile(dir) => dir.display().to_string(),
            Target::Remote(host) => host.destination.clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Target::Own => "this machine".to_string(),
            Target::LocalProfile(dir) => server_name(dir),
            Target::Remote(host) if !host.label.is_empty() => host.label.clone(),
            Target::Remote(host) => host.destination.clone(),
        }
    }
}

const ADJECTIVES: &[&str] = &[
    "amber", "bold", "brave", "bright", "calm", "clever", "cosmic", "crisp", "dapper", "eager",
    "frosty", "gentle", "golden", "happy", "jolly", "keen", "lively", "lucky", "mellow", "merry",
    "nimble", "noble", "plucky", "quiet", "rapid", "rustic", "silver", "snappy", "sunny", "swift",
    "tidy", "witty",
];

const ANIMALS: &[&str] = &[
    "badger", "beaver", "bison", "cougar", "crane", "dingo", "falcon", "ferret", "gecko", "heron",
    "ibis", "jackal", "koala", "lemur", "lynx", "marten", "moose", "narwhal", "ocelot", "otter",
    "owl", "panda", "puffin", "quokka", "raven", "seal", "sloth", "stoat", "tapir", "toucan",
    "walrus", "wombat",
];

pub fn server_name(data_dir: &Path) -> String {
    let hash = usize::from_str_radix(&profile_hash(data_dir), 16).unwrap_or_default();
    let adjective = ADJECTIVES[hash % ADJECTIVES.len()];
    let animal = ANIMALS[hash / ADJECTIVES.len() % ANIMALS.len()];
    format!("{adjective}-{animal}")
}

fn hosts_path(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join("hosts.json")
}

/// Tolerant: a missing or unreadable file is an empty list, not an error —
/// there is nothing a fresh profile or a rolled-back binary should fail on.
/// Logged, though: an empty list means the local server, whatever was active.
pub fn load(app_config_dir: &Path) -> HostsFile {
    let path = hosts_path(app_config_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HostsFile::default(),
        Err(e) => {
            log::warn!("cannot read {}; using this machine: {e}", path.display());
            return HostsFile::default();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!("{} is not valid; using this machine: {e}", path.display());
        HostsFile::default()
    })
}

/// Tmp-then-rename, like every other config write in this tree
/// (`settings.rs::persist`, `config/mod.rs::write_config_atomic`). Called by
/// the `save_hosts` command (plan Phase 1 §7) — there is one window and one
/// writer, so `save_hosts` always writes the whole file rather than merging.
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

/// An active destination that names no listed host counts as no host.
pub fn target(app_config_dir: &Path) -> Target {
    let file = load(app_config_dir);
    let active = file.active;
    match file
        .hosts
        .into_iter()
        .find(|h| Some(&h.destination) == active.as_ref())
    {
        Some(host) => Target::Remote(host),
        None => {
            if let Some(active) = &active {
                log::warn!("the active host {active} is not in hosts.json; ignoring it");
            }
            file.local_profile
                .map_or(Target::Own, |dir| Target::LocalProfile(PathBuf::from(dir)))
        }
    }
}

/// The local server's profile name, reserved so no host can claim it
/// (`src/lib/domain/hosts.ts` rejects it on the way in).
pub const LOCAL_PROFILE: &str = "local";

/// The webview's localStorage namespace. The *configured* active host, not the
/// connected one: a host that is asleep still owns its own tabs.
pub fn profile_id(app_config_dir: &Path) -> String {
    target(app_config_dir).profile_id()
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
        assert_eq!(target(dir.path()), Target::Own);
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
            ..Default::default()
        };
        save(dir.path(), &file).unwrap();

        assert_eq!(load(dir.path()), file);
        assert_eq!(target(dir.path()), Target::Remote(file.hosts[0].clone()));
    }

    #[test]
    fn the_profile_id_is_the_active_destination_or_local() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(profile_id(dir.path()), "local");

        let host = Host {
            destination: "nexus".into(),
            label: "Nexus".into(),
        };
        save(
            dir.path(),
            &HostsFile {
                hosts: vec![host],
                active: Some("nexus".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(profile_id(dir.path()), "nexus");
    }

    #[test]
    fn an_active_destination_naming_no_host_is_treated_as_local() {
        let dir = tempfile::tempdir().unwrap();
        save(
            dir.path(),
            &HostsFile {
                hosts: vec![],
                active: Some("gone".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(target(dir.path()), Target::Own);
    }
}
