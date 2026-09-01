use std::sync::Arc;

use hyprlayer_core::runtime::HyprlayerRuntime;

pub struct AppState {
    pub runtime: Arc<HyprlayerRuntime>,
}

impl AppState {
    pub fn new(runtime: Arc<HyprlayerRuntime>) -> Self {
        Self { runtime }
    }
}
