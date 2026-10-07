use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::endpoints::CloudEndpoints;
use crate::error::Result;

/// `<app_config_dir>/account.json`: one server host's session, and the pool that issued it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAccount {
    pub client_id: String,
    pub login_url: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub name: String,
    pub email: Option<String>,
}

impl std::fmt::Debug for StoredAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredAccount")
            .field("client_id", &self.client_id)
            .field("login_url", &self.login_url)
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("name", &self.name)
            .field("email", &self.email)
            .finish()
    }
}

impl StoredAccount {
    /// A debug and a release server can share an `app_config_dir`; the other pool's file
    /// reads as signed out and is never touched.
    pub(crate) fn belongs_to(&self, endpoints: &CloudEndpoints) -> bool {
        self.client_id == endpoints.client_id && self.login_url == endpoints.login_url
    }
}

pub(crate) fn path(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join("account.json")
}

pub(crate) fn read(path: &Path) -> Option<StoredAccount> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("could not read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str(&raw) {
        Ok(account) => Some(account),
        Err(_) => {
            tracing::warn!(
                "{} is not a readable account; reading it as signed out",
                path.display()
            );
            None
        }
    }
}

/// Atomic, and owner-only from its first byte on unix: the temp file is created 0600.
pub(crate) fn write(path: &Path, account: &StoredAccount) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(account).map_err(std::io::Error::other)?;
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{seq}", std::process::id()));
    let written = create_private(&tmp, &json).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written?;
    Ok(())
}

fn create_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// `true` when there was a file to remove.
pub(crate) fn delete(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
