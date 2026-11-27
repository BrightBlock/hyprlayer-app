//! The per-identifier dirs the shell resolves, as Tauri's `app.path()` does, so every
//! client of one identifier derives the same socket, `hosts.json` and `client-id`.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub log: PathBuf,
}

impl AppPaths {
    pub fn resolve(identifier: &str) -> Option<Self> {
        #[cfg(target_os = "macos")]
        let log = dirs::home_dir()?.join("Library/Logs").join(identifier);
        #[cfg(not(target_os = "macos"))]
        let log = dirs::data_local_dir()?.join(identifier).join("logs");
        Some(Self {
            config: dirs::config_dir()?.join(identifier),
            data: dirs::data_dir()?.join(identifier),
            log,
        })
    }
}
