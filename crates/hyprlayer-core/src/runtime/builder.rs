#![cfg(any(test, feature = "test-support"))]

use std::sync::Arc;

use crate::error::Result;

use super::ports::{
    ConfigCli, CredentialLookup, LifecycleRunner, NoCredentialLookup, ProviderHttp,
};
use super::{HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths};

/// Not part of the supported facade — a `#[doc(hidden)]` seam for injecting
/// failure-oriented port implementations in tests. `CredentialLookup` is the
/// one exception carried on `RuntimeOptions` itself; this builder only makes
/// replacing it (and the other ports) convenient in tests.
#[doc(hidden)]
pub struct RuntimeBuilder {
    environment: RuntimeEnvironment,
    paths: RuntimePaths,
    credential_lookup: Arc<dyn CredentialLookup + Send + Sync>,
    lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
    config_cli: Option<Arc<dyn ConfigCli>>,
    provider_http: Option<Arc<dyn ProviderHttp + Send + Sync>>,
}

impl RuntimeBuilder {
    pub fn new(environment: RuntimeEnvironment, paths: RuntimePaths) -> Self {
        Self {
            environment,
            paths,
            credential_lookup: Arc::new(NoCredentialLookup),
            lifecycle_runner: None,
            config_cli: None,
            provider_http: None,
        }
    }

    pub fn with_credential_lookup(
        mut self,
        credential_lookup: Arc<dyn CredentialLookup + Send + Sync>,
    ) -> Self {
        self.credential_lookup = credential_lookup;
        self
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

    pub fn with_provider_http(
        mut self,
        provider_http: Arc<dyn ProviderHttp + Send + Sync>,
    ) -> Self {
        self.provider_http = Some(provider_http);
        self
    }

    pub async fn open(self) -> Result<Arc<HyprlayerRuntime>> {
        let options = RuntimeOptions {
            environment: self.environment,
            paths: self.paths,
            credential_lookup: self.credential_lookup,
        };
        HyprlayerRuntime::open_with(
            options,
            self.config_cli,
            self.provider_http,
            self.lifecycle_runner,
        )
        .await
    }
}
