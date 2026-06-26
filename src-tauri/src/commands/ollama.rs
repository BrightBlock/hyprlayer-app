//! Tauri commands backing the Harnesses → Ollama settings page.
//!
//! Ollama is a model *server*, not an agent: local (or remote) models are run
//! *through* the OpenCode harness. These commands (a) probe the configured Ollama
//! endpoint for the models it has installed (`GET /api/tags`) and (b) keep the
//! `provider.ollama` block in OpenCode's `opencode.json` in sync with the user's
//! endpoint + model selection, recycling the shared `opencode serve` so it picks
//! up the change. The endpoint may be local or remote — a local install is not
//! required, so reachability of `/api/tags` (never a local binary) is the signal
//! for whether Ollama is usable.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use hyprlayer_core::opencode::config as oc;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// Per-request timeout for talking to Ollama. Short so an unreachable endpoint
/// (e.g. a remote host that's down) fails fast and the tab shows "not reachable".
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// One model installed on the Ollama server, from `GET /api/tags`.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelInfo {
    /// The canonical id, e.g. `"qwen2.5-coder:7b"` (the `models[].name` field).
    /// Used verbatim as the `models` key in the `provider.ollama` block and, with
    /// an `ollama/` prefix, as the composer model id.
    pub id: String,
    /// Architecture family (e.g. `"qwen2"`), for display.
    pub family: Option<String>,
    /// Human parameter size (e.g. `"7.6B"`), for display.
    pub parameter_size: Option<String>,
}

/// Reachability + installed-model snapshot for the configured Ollama endpoint.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    /// `true` when `GET <endpoint>/api/tags` answered. This is the authoritative
    /// "Ollama is usable" signal for both local and remote setups.
    pub reachable: bool,
    /// The effective base URL probed (default or user-configured).
    pub endpoint: String,
    /// Models installed on the server, sorted by id. Empty when unreachable.
    pub models: Vec<OllamaModelInfo>,
}

// --- /api/tags response (only the fields we surface) ---
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

/// Probe `<endpoint>/api/tags`. Never errors on an unreachable server — returns
/// an empty, `reachable: false` snapshot so the UI degrades gracefully.
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
    let resp = match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r,
        Err(e) => {
            // Don't swallow the reason — a connect error to a LAN/`.local` host on
            // macOS usually means the app lacks Local Network permission (System
            // Settings ▸ Privacy & Security ▸ Local Network), which otherwise looks
            // identical to "server down".
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
    models.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    OllamaStatus {
        reachable: true,
        endpoint: endpoint.to_string(),
        models,
    }
}

/// Mirror the current endpoint + selected models into `opencode.json`'s
/// `provider.ollama` block and recycle the shared server so it re-reads config.
async fn sync_opencode_provider(state: &AppState) -> CmdResult<()> {
    let s = state.settings.get().await;
    let endpoint = s.providers.ollama.effective_endpoint();
    let models = s.providers.ollama.selected_models.clone();
    oc::set_ollama_provider(&endpoint, &models).map_err(AppError::from)?;
    // `opencode serve` snapshots config at startup; recycle so the next session
    // sees the updated provider/model list. In-flight sessions are undisturbed.
    state.opencode.invalidate().await;
    Ok(())
}

/// Probe the configured Ollama endpoint for reachability + installed models.
#[tauri::command]
#[specta::specta]
pub async fn ollama_status(state: State<'_, AppState>) -> CmdResult<OllamaStatus> {
    let endpoint = state.settings.get().await.providers.ollama.effective_endpoint();
    Ok(probe(&endpoint).await)
}

/// Set (or clear, on `None`/empty) the Ollama base URL, re-sync the OpenCode
/// provider block, and return a fresh status probe of the new endpoint.
#[tauri::command]
#[specta::specta]
pub async fn ollama_set_endpoint(
    state: State<'_, AppState>,
    endpoint: Option<String>,
) -> CmdResult<OllamaStatus> {
    let normalized = endpoint.map(|e| e.trim().to_string()).filter(|e| !e.is_empty());
    state
        .settings
        .update(|s| s.providers.ollama.endpoint = normalized.clone())
        .await?;

    // Probe the new server first, then drop any selected/default model it doesn't
    // actually have — otherwise switching endpoints would carry the previous
    // server's selection into the composer and the `provider.ollama` block,
    // pointing at models that don't exist here. If the new endpoint is
    // unreachable we can't enumerate its models, so leave the selection as-is.
    let effective = state.settings.get().await.providers.ollama.effective_endpoint();
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

/// Set the bare Ollama model ids to expose in the composer (e.g.
/// `["qwen2.5-coder:7b"]`). Drops the stored default if it's no longer selected,
/// then re-syncs the OpenCode provider block.
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
            // Drop a stored default that's no longer in the selection.
            s.providers.ollama.reconcile_default();
        })
        .await?;
    sync_opencode_provider(&state).await
}

/// Set the default Ollama model for the composer, as an `"ollama/<id>"` id
/// (`None`/empty clears it). Stored in app settings only — it doesn't touch
/// `opencode.json`, since the provider block already lists every selected model.
///
/// The command is the state boundary, so it validates rather than trusting the
/// UI: only an `"ollama/<id>"` whose id is currently selected is persisted; any
/// stale or arbitrary value clears the default instead, so SessionChat can never
/// seed a model the `provider.ollama` block doesn't list.
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
                m.strip_prefix("ollama/")
                    .is_some_and(|id| s.providers.ollama.selected_models.iter().any(|sel| sel == id))
            });
            s.providers.ollama.default_model = if valid { requested.clone() } else { None };
        })
        .await?;
    Ok(())
}
