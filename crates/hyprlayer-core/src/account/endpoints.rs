use crate::env::RuntimeEnvironment;

/// The Cognito pool a build signs in to: the build profile decides, and nothing switches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudPool {
    Dev,
    Prod,
}

impl CloudPool {
    pub fn current() -> Self {
        if cfg!(debug_assertions) {
            CloudPool::Dev
        } else {
            CloudPool::Prod
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CloudPool::Dev => "dev",
            CloudPool::Prod => "prod",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudEndpoints {
    pub pool: CloudPool,
    /// The pool's custom hosted-UI domain: the GitHub federation admits no other.
    pub login_url: String,
    pub api_url: String,
    pub client_id: String,
}

impl CloudEndpoints {
    pub fn for_pool(pool: CloudPool) -> Self {
        let (login_url, api_url, client_id) = match pool {
            CloudPool::Dev => (
                "https://login.dev.cloud.hyprlayer.dev",
                "https://api.dev.cloud.hyprlayer.dev",
                "5qu3dne9cln4ugit8i2aeupv1h",
            ),
            CloudPool::Prod => (
                "https://login.cloud.hyprlayer.dev",
                "https://api.cloud.hyprlayer.dev",
                "6chvucvj17rrg8dosl2dvmjvkk",
            ),
        };
        Self {
            pool,
            login_url: login_url.into(),
            api_url: api_url.into(),
            client_id: client_id.into(),
        }
    }

    pub fn resolve(env: &RuntimeEnvironment) -> Self {
        with_test_knobs(env, Self::for_pool(CloudPool::current()))
    }
}

/// Debug builds only: these knobs redirect bearer tokens, so a release binary carries
/// neither the code nor the names.
#[cfg(debug_assertions)]
fn with_test_knobs(env: &RuntimeEnvironment, mut endpoints: CloudEndpoints) -> CloudEndpoints {
    use std::sync::Once;
    static LOGIN: Once = Once::new();
    static API: Once = Once::new();
    static CLIENT: Once = Once::new();
    if let Some(url) = url_knob(env, "HYPRLAYER_CLOUD_LOGIN_URL", &LOGIN) {
        endpoints.login_url = url;
    }
    if let Some(url) = url_knob(env, "HYPRLAYER_CLOUD_API_URL", &API) {
        endpoints.api_url = url;
    }
    let client = env
        .var("HYPRLAYER_CLOUD_CLIENT_ID")
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty());
    if let Some(client) = client {
        CLIENT.call_once(|| {
            tracing::warn!("HYPRLAYER_CLOUD_CLIENT_ID is set; Cloud sign-in uses client {client}")
        });
        endpoints.client_id = client.to_string();
    }
    endpoints
}

#[cfg(debug_assertions)]
fn url_knob(env: &RuntimeEnvironment, var: &str, warned: &std::sync::Once) -> Option<String> {
    let raw = env.var(var)?.to_str()?;
    warned.call_once(|| tracing::warn!("{var} is set; Cloud sign-in goes to {raw}"));
    if crate::env::https_or_loopback_http(raw) {
        return Some(raw.trim_end_matches('/').to_string());
    }
    tracing::warn!("{var} must be https:// or loopback http://; ignoring it");
    None
}

#[cfg(not(debug_assertions))]
fn with_test_knobs(_env: &RuntimeEnvironment, endpoints: CloudEndpoints) -> CloudEndpoints {
    endpoints
}
