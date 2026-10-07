//! Cognito and Cloud API requests and answers, with no I/O: `runtime::account` sends them,
//! because a module outside `runtime/` may not reach `ProviderHttp` (ADR-0008).

use serde::Deserialize;

use super::endpoints::CloudEndpoints;
use crate::error::{CoreError, Result};

const SCOPES: &str = "openid email profile";
const GITHUB_IDP: &str = "GitHubOidc";
/// Cognito's access-token lifetime, for an answer that omits `expires_in`.
const DEFAULT_EXPIRES_IN: u64 = 900;

pub(crate) fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

/// The first leg: clearing Cognito's session makes the authorize leg federate to GitHub
/// afresh instead of reusing it.
pub(crate) fn logout_url(endpoints: &CloudEndpoints, port: u16) -> Result<String> {
    let logout_uri = format!("http://localhost:{port}/auth/loggedout");
    url_with(
        &format!("{}/logout", endpoints.login_url),
        &[
            ("client_id", endpoints.client_id.as_str()),
            ("logout_uri", &logout_uri),
        ],
    )
}

pub(crate) fn authorize_url(
    endpoints: &CloudEndpoints,
    challenge: &str,
    state: &str,
    port: u16,
) -> Result<String> {
    url_with(
        &format!("{}/oauth2/authorize", endpoints.login_url),
        &[
            ("client_id", endpoints.client_id.as_str()),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("redirect_uri", &redirect_uri(port)),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("identity_provider", GITHUB_IDP),
        ],
    )
}

fn url_with(base: &str, params: &[(&str, &str)]) -> Result<String> {
    reqwest::Url::parse_with_params(base, params)
        .map(String::from)
        .map_err(|e| CoreError::AccountSignInFailed {
            detail: format!("{base}: {e}"),
        })
}

pub(crate) fn token_url(endpoints: &CloudEndpoints) -> String {
    format!("{}/oauth2/token", endpoints.login_url)
}

/// `redirect_uri` must be byte-identical to the one the authorize request carried.
pub(crate) fn code_exchange_form(
    endpoints: &CloudEndpoints,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Vec<(String, String)> {
    [
        ("grant_type", "authorization_code"),
        ("client_id", endpoints.client_id.as_str()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

/// A refused or unreadable answer: its status and OAuth `error` code, never its body.
pub(crate) struct Refusal {
    pub status: u16,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: String,
    /// Absent from a refresh answer.
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: Option<String>,
}

pub(crate) fn classify_token_response(
    status: u16,
    body: &str,
) -> std::result::Result<Tokens, Refusal> {
    if !(200..300).contains(&status) {
        return Err(Refusal {
            status,
            error: oauth_error(body),
        });
    }
    let parsed: TokenBody = serde_json::from_str(body).map_err(|_| unreadable(status))?;
    Ok(Tokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token.ok_or_else(|| unreadable(status))?,
        expires_in: parsed.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
    })
}

pub(crate) fn refresh_form(
    endpoints: &CloudEndpoints,
    refresh_token: &str,
) -> Vec<(String, String)> {
    [
        ("grant_type", "refresh_token"),
        ("client_id", endpoints.client_id.as_str()),
        ("refresh_token", refresh_token),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

/// Cognito returns no new refresh token, so the caller keeps the one it sent.
pub(crate) struct RefreshedTokens {
    pub access_token: String,
    pub expires_in: u64,
}

pub(crate) enum RefreshOutcome {
    Refreshed(RefreshedTokens),
    /// The refresh token is dead: the only answer that signs the user out.
    InvalidGrant,
    Transient(String),
}

pub(crate) fn classify_refresh_response(status: u16, body: &str) -> RefreshOutcome {
    if status == 400 && oauth_error(body).as_deref() == Some("invalid_grant") {
        return RefreshOutcome::InvalidGrant;
    }
    if !(200..300).contains(&status) {
        return RefreshOutcome::Transient(format!("HTTP {status}"));
    }
    match serde_json::from_str::<TokenBody>(body) {
        Ok(parsed) => RefreshOutcome::Refreshed(RefreshedTokens {
            access_token: parsed.access_token,
            expires_in: parsed.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
        }),
        Err(_) => RefreshOutcome::Transient(format!("HTTP {status}, unreadable answer")),
    }
}

pub(crate) fn revoke_url(endpoints: &CloudEndpoints) -> String {
    format!("{}/oauth2/revoke", endpoints.login_url)
}

pub(crate) fn revoke_form(
    endpoints: &CloudEndpoints,
    refresh_token: &str,
) -> Vec<(String, String)> {
    [
        ("token", refresh_token),
        ("client_id", endpoints.client_id.as_str()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

pub(crate) fn session_url(endpoints: &CloudEndpoints) -> String {
    format!("{}/v1/session", endpoints.api_url)
}

pub(crate) struct Identity {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Deserialize)]
struct SessionBody {
    author: AuthorBody,
}

#[derive(Deserialize)]
struct AuthorBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// `author.name` is the backend's display name for the GitHub account, never blank.
pub(crate) fn parse_session(status: u16, body: &str) -> std::result::Result<Identity, Refusal> {
    if !(200..300).contains(&status) {
        return Err(Refusal {
            status,
            error: oauth_error(body),
        });
    }
    let author = serde_json::from_str::<SessionBody>(body)
        .map_err(|_| unreadable(status))?
        .author;
    let name = author
        .name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| unreadable(status))?;
    Ok(Identity {
        name,
        email: author.email.filter(|email| !email.trim().is_empty()),
    })
}

fn unreadable(status: u16) -> Refusal {
    Refusal {
        status,
        error: Some("unreadable_answer".into()),
    }
}

/// Only a code-shaped `error` survives, so no body text can reach a log.
fn oauth_error(body: &str) -> Option<String> {
    serde_json::from_str::<ErrorBody>(body)
        .ok()?
        .error
        .filter(|error| {
            !error.is_empty()
                && error.len() <= 64
                && error
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
