//! The orchestration skill as a claude plugin, written per server version and passed per launch (ADR-0010).

use std::io;
use std::path::{Path, PathBuf};

use crate::VERSION;

const SKILL: &str = include_str!("../assets/agent-control/SKILL.md");

/// Writes `<app data>/agent-control/plugin/<VERSION>/`; `None` leaves launches without it.
pub fn install(app_data_dir: &Path) -> Option<PathBuf> {
    let root = plugin_root(app_data_dir);
    match write(&root) {
        Ok(dir) => Some(dir),
        Err(e) => {
            tracing::warn!(
                "agents get no orchestration skill: cannot write {}: {e}",
                root.display()
            );
            None
        }
    }
}

fn files() -> [(&'static str, String); 2] {
    let manifest = serde_json::json!({
        "name": "hyprlayer",
        "version": VERSION,
        "description": "Coordinate other hyprlayer agents from inside one.",
    });
    [
        (".claude-plugin/plugin.json", manifest.to_string()),
        ("skills/orchestrate/SKILL.md", SKILL.to_string()),
    ]
}

fn write(root: &Path) -> io::Result<PathBuf> {
    let dir = root.join(VERSION);
    let files = files();
    // Every restart lands here, and a dev build changes the skill without changing the version.
    let current = files
        .iter()
        .all(|(rel, body)| std::fs::read(dir.join(rel)).is_ok_and(|b| b == body.as_bytes()));
    if !current {
        let staging = root.join(format!(".{VERSION}.{}", std::process::id()));
        remove(&staging)?;
        for (rel, body) in &files {
            let path = staging.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, body)?;
        }
        remove(&dir)?;
        std::fs::rename(&staging, &dir)?;
    }
    Ok(dir)
}

fn plugin_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("agent-control").join("plugin")
}

/// Other versions, and staging a crashed write left behind. Call only while holding the
/// profile lock: a server that lost it must not delete the live server's plugin.
pub fn prune(app_data_dir: &Path) {
    let root = plugin_root(app_data_dir);
    let root = root.as_path();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("could not list {}: {e}", root.display());
            return;
        }
    };
    for entry in entries.flatten() {
        if entry.file_name() == VERSION {
            continue;
        }
        let path = entry.path();
        if let Err(e) = remove(&path) {
            tracing::warn!("could not prune {}: {e}", path.display());
        }
    }
}

fn remove(path: &Path) -> io::Result<()> {
    let removed = if path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match removed {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
