use std::sync::Arc;

use crate::config::ConfigService;
use crate::error::Result;
use crate::process::binaries::BinaryResolver;
use crate::settings::AppSettings;

use super::HyprlayerRuntime;

pub struct SettingsService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn settings(&self) -> SettingsService<'_> {
        SettingsService { runtime: self }
    }
}

impl SettingsService<'_> {
    pub async fn get(&self) -> AppSettings {
        self.runtime.settings.get().await
    }

    pub async fn set(&self, settings: AppSettings) -> Result<()> {
        self.runtime.settings.set(settings).await
    }

    pub async fn update<F>(&self, mutate: F) -> Result<AppSettings>
    where
        F: FnOnce(&mut AppSettings),
    {
        self.runtime.settings.update(mutate).await
    }

    /// Desktop carve-out: `system.rs`/`harnesses.rs`/`workstreams.rs` need the
    /// resolver itself (override updates, redetection, `gh`/`git` gating,
    /// `augmented_path`), not a DTO-shaped operation on it.
    pub fn binaries(&self) -> Arc<BinaryResolver> {
        self.runtime.binaries.clone()
    }

    /// Desktop carve-out: `mod.rs::effective_for` needs the config service
    /// directly to read thoughts config outside the operation set this
    /// facade otherwise exposes.
    pub fn config(&self) -> Arc<ConfigService> {
        self.runtime.config.clone()
    }
}
