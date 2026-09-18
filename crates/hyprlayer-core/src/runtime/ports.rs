use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Result;
use crate::hyprcli::{HyprCli, StorageInfo};
use crate::process::binaries::BinaryResolver;
use crate::runtime::script_registry::RunPhase;
use crate::scripts::ScriptKind;
use crate::store::Workstream;

/// Wraps the hyprlayer CLI's `storage info` probe (which forces its
/// load→migrate→save path) behind a seam so config-migration tests can inject
/// CLI absence or failure without a real binary.
pub trait ConfigCli: Send + Sync {
    fn storage_info<'a>(
        &'a self,
        repo_path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<StorageInfo>> + Send + 'a>>;
}

pub struct CliConfigCli {
    binaries: Arc<BinaryResolver>,
}

impl CliConfigCli {
    pub fn new(binaries: Arc<BinaryResolver>) -> Self {
        Self { binaries }
    }
}

impl ConfigCli for CliConfigCli {
    fn storage_info<'a>(
        &'a self,
        repo_path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<StorageInfo>> + Send + 'a>> {
        Box::pin(async move { HyprCli::new(&self.binaries).storage_info(repo_path).await })
    }
}

pub struct ProviderHttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Option<std::time::Duration>,
    pub max_bytes: Option<usize>,
}

impl ProviderHttpRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            timeout: None,
            max_bytes: None,
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Refuse a body larger than `max`, so a caller reading an endpoint it does
    /// not control cannot be made to buffer an unbounded response.
    pub fn max_bytes(mut self, max: usize) -> Self {
        self.max_bytes = Some(max);
        self
    }
}

pub struct ProviderHttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug)]
pub struct ProviderHttpError(pub String);

impl std::fmt::Display for ProviderHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProviderHttpError {}

/// A GET-only network seam: every provider network call this plan moves into
/// core (Claude's `/v1/models`, Ollama's `/api/tags`) is a plain GET, so this
/// stays narrow rather than wrapping the whole of `reqwest`.
pub trait ProviderHttp: Send + Sync {
    fn get<'a>(
        &'a self,
        request: ProviderHttpRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<ProviderHttpResponse, ProviderHttpError>>
                + Send
                + 'a,
        >,
    >;
}

pub struct ReqwestProviderHttp {
    client: reqwest::Client,
}

impl ReqwestProviderHttp {
    pub fn new(timeout: std::time::Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("hyprlayer-app")
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

impl ProviderHttp for ReqwestProviderHttp {
    fn get<'a>(
        &'a self,
        request: ProviderHttpRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<ProviderHttpResponse, ProviderHttpError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let mut builder = self.client.get(&request.url);
            for (key, value) in &request.headers {
                builder = builder.header(key, value);
            }
            if let Some(timeout) = request.timeout {
                builder = builder.timeout(timeout);
            }
            let response = builder
                .send()
                .await
                .map_err(|e| ProviderHttpError(e.to_string()))?;
            let status = response.status().as_u16();
            let body = match request.max_bytes {
                Some(max) => read_capped(response, max).await?,
                None => response
                    .text()
                    .await
                    .map_err(|e| ProviderHttpError(e.to_string()))?,
            };
            Ok(ProviderHttpResponse { status, body })
        })
    }
}

/// Runs a workstream's configured Setup/Archive command line and reports
/// pass/fail. Production always uses `ScriptService::run_lifecycle`; this is
/// a test-only replacement, injected through `RuntimeBuilder`, so a test can
/// drive the Archive gate without a real script on disk. `None` means no
/// script is configured for the requested kind — the same "skip" contract
/// `ScriptService::run_lifecycle` returns.
pub trait LifecycleRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        item: &'a Workstream,
        kind: ScriptKind,
    ) -> Pin<Box<dyn Future<Output = Option<RunPhase>> + Send + 'a>>;
}

/// Enforced chunk by chunk rather than on the finished body, so an oversized
/// response is abandoned mid-stream instead of buffered and then rejected.
async fn read_capped(
    mut response: reqwest::Response,
    max: usize,
) -> std::result::Result<String, ProviderHttpError> {
    let too_big = || ProviderHttpError(format!("response body exceeds {max} bytes"));
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(too_big());
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ProviderHttpError(e.to_string()))?
    {
        if body.len() + chunk.len() > max {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}
