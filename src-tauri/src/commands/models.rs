use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::settings::ClaudeSettings;
use crate::state::AppState;

const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelWindow {
    pub id: String,
    pub context_window: u32,
}

#[derive(Deserialize)]
struct RawModels {
    #[serde(default)]
    data: Vec<RawModel>,
}
#[derive(Deserialize)]
struct RawModel {
    id: String,
    max_input_tokens: Option<u32>,
}

#[tauri::command]
#[specta::specta]
pub async fn claude_model_windows(state: State<'_, AppState>) -> CmdResult<Vec<ModelWindow>> {
    let cfg = state.settings.get().await.providers.claude;
    let auth = resolve_auth(&cfg).await.ok_or_else(|| AppError {
        code: "no_claude_auth".into(),
        message: "No Claude credentials found (log in with the claude CLI or set an API key)."
            .into(),
        remedy: None,
    })?;

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("hyprlayer-app")
        .build()
        .map_err(|e| AppError::internal(format!("build http client: {e}")))?;

    let base = match &auth {
        Auth::ApiKey {
            base_url: Some(b), ..
        } if !b.is_empty() => b.trim_end_matches('/'),
        _ => "https://api.anthropic.com",
    };
    let url = format!("{base}/v1/models?limit=1000");

    let req = client.get(&url).header("anthropic-version", "2023-06-01");
    let req = match &auth {
        Auth::ApiKey { key, .. } => req.header("x-api-key", key),
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

enum Auth {
    ApiKey {
        key: String,
        base_url: Option<String>,
    },
    OAuth(String),
}

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
