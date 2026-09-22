#![cfg(any(test, feature = "test-support"))]

use std::sync::Arc;

use crate::error::Result;

use super::ports::{ConfigCli, LifecycleRunner};
use super::{HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths};

/// Not part of the supported facade — a `#[doc(hidden)]` seam for injecting
/// failure-oriented port implementations in tests.
#[doc(hidden)]
pub struct RuntimeBuilder {
    environment: RuntimeEnvironment,
    paths: RuntimePaths,
    lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
    config_cli: Option<Arc<dyn ConfigCli>>,
}

impl RuntimeBuilder {
    pub fn new(environment: RuntimeEnvironment, paths: RuntimePaths) -> Self {
        Self {
            environment,
            paths,
            lifecycle_runner: None,
            config_cli: None,
        }
    }

    pub fn with_lifecycle_runner(
        mut self,
        lifecycle_runner: Arc<dyn LifecycleRunner + Send + Sync>,
    ) -> Self {
        self.lifecycle_runner = Some(lifecycle_runner);
        self
    }

    pub fn with_config_cli(mut self, config_cli: Arc<dyn ConfigCli>) -> Self {
        self.config_cli = Some(config_cli);
        self
    }

    pub async fn open(self) -> Result<Arc<HyprlayerRuntime>> {
        let options = RuntimeOptions::new(self.environment, self.paths);
        HyprlayerRuntime::open_with(options, self.config_cli, self.lifecycle_runner).await
    }
}
