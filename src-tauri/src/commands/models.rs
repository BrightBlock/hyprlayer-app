//! Tauri command backing the context-window meter's denominator.
//!
//! The composer's per-model context windows (`src/features/session/models.ts`)
//! are a hand-maintained table that drifts when Anthropic ships a model or
//! resizes a window — Fable 5 shipped at 1M but was pinned to 200K, so the meter
//! over-read and never lined up with the CLI's real (1M) compaction point. This
//! command fetches the authoritative windows from the Anthropic Models API
//! (`GET /v1/models` → `max_input_tokens`), reusing the user's Claude Code login,
//! so the UI can prefer the live value and fall back to the static table whenever
//! the API is unreachable (offline, logged out, or Keychain access denied).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::settings::ClaudeSettings;
use crate::state::AppState;

/// Bound the fetch so a wedged network can't hang the meter refresh.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

/// macOS Keychain service Claude Code stores its OAuth login under (the account
/// is the OS username). Windows/Linux keep the same JSON in a file instead.
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// One model's authoritative context window, from `GET /v1/models`.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelWindow {
    /// Canonical model id, e.g. `"claude-fable-5"` (`data[].id`). Bare — the API
    /// exposes no `[1m]` variants, so the UI strips that suffix before lookup.
    pub id: String,
    /// Context window in tokens (`max_input_tokens`) — the meter's denominator.
    pub context_window: u32,
}

// --- GET /v1/models response (only the fields we surface) ---
#[derive(Deserialize)]
struct RawModels {
    #[serde(default)]
    data: Vec<RawModel>,
}
#[derive(Deserialize)]
struct RawModel {
    id: String,
    /// Added to the Models API in 2026 — absent on older models, which then fall
    /// back to the static table.
    max_input_tokens: Option<u32>,
}

/// Fetch authoritative per-model context windows from the Anthropic Models API,
/// reusing the user's Claude auth (a configured API key, else the CLI's OAuth
/// login). Best-effort: every failure path (no credentials, offline, denied
/// Keychain, non-2xx, unparseable) surfaces as an error the caller is expected to
/// swallow, leaving the composer's static per-model table as the denominator.
#[tauri::command]
#[specta::specta]
pub async fn claude_model_windows(state: State<'_, AppState>) -> CmdResult<Vec<ModelWindow>> {
    let cfg = state.settings.get().await.providers.claude;
    let auth = resolve_auth(&cfg).await.ok_or_else(|| AppError {
        code: "no_claude_auth".into(),
        message: "No Claude credentials found (log in with the claude CLI or set an API key).".into(),
        remedy: None,
    })?;

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("hyprlayer-app")
        .build()
        .map_err(|e| AppError::internal(format!("build http client: {e}")))?;

    // API-key/proxy users may point at a custom base; the OAuth login is scoped
    // to the first-party API, so it always hits api.anthropic.com.
    let base = match &auth {
        Auth::ApiKey { base_url: Some(b), .. } if !b.is_empty() => b.trim_end_matches('/'),
        _ => "https://api.anthropic.com",
    };
    let url = format!("{base}/v1/models?limit=1000");

    let req = client.get(&url).header("anthropic-version", "2023-06-01");
    let req = match &auth {
        Auth::ApiKey { key, .. } => req.header("x-api-key", key),
        // OAuth tokens go on Authorization: Bearer with the oauth beta header.
        Auth::OAuth(tok) => req
            .header("authorization", format!("Bearer {tok}"))
            .header("anthropic-beta", "oauth-2025-04-20"),
    };

    let resp = req
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| AppError {
            code: "models_fetch_failed".into(),
            message: format!("Models API request failed: {e}"),
            remedy: None,
        })?;
    let parsed = resp
        .json::<RawModels>()
        .await
        .map_err(|e| AppError::internal(format!("parse /v1/models: {e}")))?;

    Ok(parsed
        .data
        .into_iter()
        .filter_map(|m| {
            m.max_input_tokens.map(|context_window| ModelWindow {
                id: m.id,
                context_window,
            })
        })
        .collect())
}

/// A resolved Claude credential for a direct Anthropic API call.
enum Auth {
    /// Explicit api-key mode (Settings → Harnesses → Claude), with any custom
    /// base URL for a proxy/gateway.
    ApiKey { key: String, base_url: Option<String> },
    /// The Claude Code CLI login (an OAuth access token).
    OAuth(String),
}

/// Resolve Claude credentials, preferring an explicitly-configured API key over
/// the CLI login. `None` when neither is available (→ static-table fallback).
async fn resolve_auth(cfg: &ClaudeSettings) -> Option<Auth> {
    if cfg.auth_mode.as_deref() == Some("apiKey") {
        if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
            return Some(Auth::ApiKey {
                key: key.to_string(),
                base_url: cfg.base_url.clone(),
            });
        }
    }
    read_cli_oauth_token().await.map(Auth::OAuth)
}

/// Read Claude Code's stored OAuth access token, cross-platform. The token
/// expires after a few hours and the CLI refreshes it on its own use; a stale one
/// just yields a 401 upstream (→ static-table fallback), so we never refresh it
/// here. Storage differs by OS:
/// - Windows / Linux: `~/.claude/.credentials.json`.
/// - macOS: the login lives in the Keychain, which is authoritative — read it
///   first (via `security`, which may raise a one-time OS access prompt; a denial
///   or miss falls through to the file, only ever a rare flat-setup leftover).
async fn read_cli_oauth_token() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(tok) = token_from_keychain().await {
            return Some(tok);
        }
        token_from_file()
    }
    #[cfg(not(target_os = "macos"))]
    {
        token_from_file()
    }
}

fn token_from_file() -> Option<String> {
    let path = dirs::home_dir()?.join(".claude").join(".credentials.json");
    token_from_json(&std::fs::read_to_string(path).ok()?)
}

#[cfg(target_os = "macos")]
async fn token_from_keychain() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output()
            .ok()?;
        out.status.success().then_some(())?;
        token_from_json(String::from_utf8(out.stdout).ok()?.trim())
    })
    .await
    .ok()
    .flatten()
}

/// Pull `.claudeAiOauth.accessToken` out of the credentials JSON (identical shape
/// in the Keychain blob and the file).
fn token_from_json(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()?
        .get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::token_from_json;

    #[test]
    fn extracts_access_token_from_credentials_json() {
        // The shape stored in both the macOS Keychain blob and the Win/Linux file.
        let raw = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-abc","refreshToken":"x","expiresAt":1,"scopes":["user:inference"]}}"#;
        assert_eq!(token_from_json(raw).as_deref(), Some("sk-ant-oat01-abc"));
    }

    #[test]
    fn missing_or_malformed_credentials_yield_none() {
        assert_eq!(token_from_json("not json"), None);
        assert_eq!(token_from_json("{}"), None);
        assert_eq!(token_from_json(r#"{"claudeAiOauth":{}}"#), None);
    }
}
