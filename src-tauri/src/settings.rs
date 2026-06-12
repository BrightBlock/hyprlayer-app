//! App-local settings, persisted as JSON in the app config dir.
//!
//! This holds only what belongs to the app itself (binary overrides, UI
//! flags). Anything about projects/backends lives in the hyprlayer CLI
//! config, which the CLI owns.

use std::collections::HashMap;
use std::path::PathBuf;

use hyprlayer_core::process::binaries::BinaryKind;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::{AppError, CmdResult};

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub binary_overrides: HashMap<BinaryKind, String>,
    pub onboarding_complete: bool,
    pub last_project_path: Option<String>,
}

pub struct SettingsStore {
    path: PathBuf,
    cache: RwLock<AppSettings>,
}

impl SettingsStore {
    /// Load (or default) settings from `<dir>/settings.json`.
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("settings.json");
        let settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            cache: RwLock::new(settings),
        }
    }

    pub async fn get(&self) -> AppSettings {
        self.cache.read().await.clone()
    }

    /// Atomic read-modify-write: the lock is held across the mutation AND the
    /// file write, so concurrent updates can't lose each other's edits, and
    /// the in-memory cache only advances when the write stuck.
    pub async fn update<F>(&self, mutate: F) -> CmdResult<AppSettings>
    where
        F: FnOnce(&mut AppSettings),
    {
        let mut guard = self.cache.write().await;
        let mut next = guard.clone();
        mutate(&mut next);
        persist(&self.path, &next).await?;
        *guard = next.clone();
        Ok(next)
    }

    pub async fn set(&self, settings: AppSettings) -> CmdResult<()> {
        self.update(|s| *s = settings).await.map(|_| ())
    }
}

async fn persist(path: &std::path::Path, settings: &AppSettings) -> CmdResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::internal(format!("create settings dir: {e}")))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::internal(format!("serialize settings: {e}")))?;

    // tmp + rename for atomicity, mirroring the CLI's discipline.
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, json)
        .await
        .map_err(|e| AppError::internal(format!("write settings: {e}")))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| AppError::internal(format!("persist settings: {e}")))?;
    Ok(())
}
