//! This desktop's display-only identity on the server (ADR-0009): it attributes
//! turns and targets sync events, and grants nothing.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientIdentity {
    pub client_id: String,
    pub device: String,
}

/// One id per Tauri identifier, so a dev build and the release are different clients.
pub fn load_or_create(app_config_dir: &Path) -> ClientIdentity {
    ClientIdentity {
        client_id: client_id(app_config_dir),
        device: device_name(),
    }
}

fn client_id(app_config_dir: &Path) -> String {
    let path = app_config_dir.join("client-id");
    match std::fs::read_to_string(&path) {
        Ok(saved) => match uuid::Uuid::parse_str(saved.trim()) {
            Ok(id) => return id.to_string(),
            Err(e) => log::warn!("{} is not a client id; replacing it: {e}", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            log::warn!(
                "cannot read {}; using a client id for this launch only: {e}",
                path.display()
            );
            return uuid::Uuid::new_v4().to_string();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = save(&path, &id) {
        log::warn!(
            "cannot write {}; using a client id for this launch only: {e}",
            path.display()
        );
    }
    id
}

/// Tmp-then-rename, as `hosts::save`.
fn save(path: &Path, id: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, id)?;
    std::fs::rename(&tmp, path)
}

/// `Jons-MacBook-Pro.local` → `jons-macbook-pro`.
fn device_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().to_lowercase();
    let host = host.trim();
    let name = host.strip_suffix(".local").unwrap_or(host);
    if name.is_empty() {
        "this-device".to_string()
    } else {
        name.to_string()
    }
}
