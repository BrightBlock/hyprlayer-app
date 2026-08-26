use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use hyprlayer_core::opencode::config as oc;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelInfo {
    pub id: String,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub reachable: bool,
    pub endpoint: String,
    pub models: Vec<OllamaModelInfo>,
}

#[derive(Deserialize)]
struct RawTags {
    #[serde(default)]
    models: Vec<RawTagModel>,
}
#[derive(Deserialize)]
struct RawTagModel {
    name: String,
    #[serde(default)]
    details: Option<RawTagDetails>,
}
#[derive(Deserialize)]
struct RawTagDetails {
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    parameter_size: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .user_agent("hyprlayer-app")
        .build()
        .map_err(|e| e.to_string())
}

async fn probe(endpoint: &str) -> OllamaStatus {
    let unreachable = || OllamaStatus {
        reachable: false,
        endpoint: endpoint.to_string(),
        models: Vec::new(),
    };
    let Ok(client) = client() else {
        return unreachable();
    };
    let url = format!("{endpoint}/api/tags");
    let resp = match client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("ollama probe {url} failed: {e}");
            return unreachable();
        }
    };
    let tags = match resp.json::<RawTags>().await {
        Ok(t) => t,
        Err(e) => {
            log::warn!("ollama probe {url} returned unparseable /api/tags: {e}");
            return unreachable();
        }
    };
    let mut models: Vec<OllamaModelInfo> = tags
        .models
        .into_iter()
        .map(|m| {
            let (family, parameter_size) = match m.details {
                Some(d) => (d.family, d.parameter_size),
                None => (None, None),
            };
            OllamaModelInfo {
                id: m.name,
                family,
                parameter_size,
            }
        })
        .collect();
    models.sort_by_key(|m| m.id.to_lowercase());
    OllamaStatus {
        reachable: true,
        endpoint: endpoint.to_string(),
        models,
    }
}

async fn sync_opencode_provider(state: &AppState) -> CmdResult<()> {
    let s = state.settings.get().await;
    let endpoint = s.providers.ollama.effective_endpoint();
    let models = s.providers.ollama.selected_models.clone();
    oc::set_ollama_provider(&endpoint, &models).map_err(AppError::from)?;
    state.opencode.invalidate().await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_status(state: State<'_, AppState>) -> CmdResult<OllamaStatus> {
    let endpoint = state
        .settings
        .get()
        .await
        .providers
        .ollama
        .effective_endpoint();
    Ok(probe(&endpoint).await)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_endpoint(
    state: State<'_, AppState>,
    endpoint: Option<String>,
) -> CmdResult<OllamaStatus> {
    let normalized = endpoint
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    state
        .settings
        .update(|s| s.providers.ollama.endpoint = normalized.clone())
        .await?;

    let effective = state
        .settings
        .get()
        .await
        .providers
        .ollama
        .effective_endpoint();
    let status = probe(&effective).await;
    if status.reachable {
        let available: std::collections::HashSet<&str> =
            status.models.iter().map(|m| m.id.as_str()).collect();
        state
            .settings
            .update(|s| {
                s.providers
                    .ollama
                    .selected_models
                    .retain(|id| available.contains(id.as_str()));
                s.providers.ollama.reconcile_default();
            })
            .await?;
    }
    sync_opencode_provider(&state).await?;
    Ok(status)
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_selected_models(
    state: State<'_, AppState>,
    models: Vec<String>,
) -> CmdResult<()> {
    state
        .settings
        .update(|s| {
            s.providers.ollama.selected_models = models.clone();
            s.providers.ollama.reconcile_default();
        })
        .await?;
    sync_opencode_provider(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn ollama_set_default_model(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<()> {
    let requested = model.filter(|m| !m.is_empty());
    state
        .settings
        .update(|s| {
            let valid = requested.as_deref().is_some_and(|m| {
                m.strip_prefix("ollama/").is_some_and(|id| {
                    s.providers
                        .ollama
                        .selected_models
                        .iter()
                        .any(|sel| sel == id)
                })
            });
            s.providers.ollama.default_model = if valid { requested.clone() } else { None };
        })
        .await?;
    Ok(())
}
