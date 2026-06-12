use std::sync::Arc;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::process::binaries::BinaryResolver;

use crate::settings::SettingsStore;

pub struct AppState {
    pub config: Arc<ConfigService>,
    pub binaries: Arc<BinaryResolver>,
    pub settings: Arc<SettingsStore>,
}
