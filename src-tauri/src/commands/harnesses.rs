use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_specta::Event;

use hyprlayer_core::claude::{self, ClaudeProbe};
use hyprlayer_core::codex::{self, CodexProbe};
use hyprlayer_core::opencode::config as oc;
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::process::Exec;

use crate::error::{AppError, CmdResult};
use crate::events::BinariesChanged;
use crate::state::AppState;

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const CATALOG_CACHE_FILE: &str = "opencode-catalog.json";
const CATALOG_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const CATALOG_SNAPSHOT: &str = include_str!("opencode_catalog_snapshot.json");

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProvider {
    pub id: String,
    pub name: String,
    pub env: Vec<String>,
    pub doc: Option<String>,
    pub models: Vec<CatalogModel>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpencodeCatalog {
    pub providers: Vec<CatalogProvider>,
    pub source: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedCatalog {
    fetched_at_ms: u64,
    providers: Vec<CatalogProvider>,
}

#[derive(Deserialize)]
struct SnapshotCatalog {
    providers: Vec<CatalogProvider>,
}

#[derive(Deserialize)]
struct RawProvider {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    doc: Option<String>,
    #[serde(default)]
    models: std::collections::HashMap<String, RawModel>,
}
#[derive(Deserialize)]
struct RawModel {
    id: Option<String>,
    name: Option<String>,
}

fn parse_api_json(body: &str) -> Result<Vec<CatalogProvider>, serde_json::Error> {
    let raw: std::collections::HashMap<String, RawProvider> = serde_json::from_str(body)?;
    let mut providers: Vec<CatalogProvider> = raw
        .into_iter()
        .map(|(pid, p)| {
            let mut models: Vec<CatalogModel> = p
                .models
                .into_iter()
                .map(|(mid, m)| {
                    let id = m.id.unwrap_or(mid);
                    let name = m.name.unwrap_or_else(|| id.clone());
                    CatalogModel { id, name }
                })
                .collect();
            models.sort_by_key(|m| m.name.to_lowercase());
            let id = p.id.unwrap_or(pid);
            let name = p.name.unwrap_or_else(|| id.clone());
            CatalogProvider {
                id,
                name,
                env: p.env,
                doc: p.doc,
                models,
            }
        })
        .collect();
    providers.sort_by_key(|p| p.name.to_lowercase());
    Ok(providers)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_cache(path: &Path) -> Option<CachedCatalog> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(path: &Path, providers: &[CatalogProvider]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = CachedCatalog {
        fetched_at_ms: now_ms(),
        providers: providers.to_vec(),
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

async fn fetch_catalog() -> Result<Vec<CatalogProvider>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("hyprlayer-app")
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(MODELS_DEV_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    parse_api_json(&body).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_catalog(app: tauri::AppHandle, force: bool) -> CmdResult<OpencodeCatalog> {
    let cache = app
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(CATALOG_CACHE_FILE));

    if !force {
        if let Some(ref p) = cache {
            if let Some(c) = read_cache(p) {
                let fresh =
                    now_ms().saturating_sub(c.fetched_at_ms) < CATALOG_TTL.as_millis() as u64;
                if fresh {
                    return Ok(OpencodeCatalog {
                        providers: c.providers,
                        source: "cache".into(),
                    });
                }
            }
        }
    }

    match fetch_catalog().await {
        Ok(providers) => {
            if let Some(ref p) = cache {
                write_cache(p, &providers);
            }
            Ok(OpencodeCatalog {
                providers,
                source: "network".into(),
            })
        }
        Err(net_err) => {
            if let Some(ref p) = cache {
                if let Some(c) = read_cache(p) {
                    return Ok(OpencodeCatalog {
                        providers: c.providers,
                        source: "cache".into(),
                    });
                }
            }
            match serde_json::from_str::<SnapshotCatalog>(CATALOG_SNAPSHOT) {
                Ok(snap) => Ok(OpencodeCatalog {
                    providers: snap.providers,
                    source: "bundled".into(),
                }),
                Err(_) => Err(AppError::internal(format!(
                    "could not load the models.dev catalog: {net_err}"
                ))),
            }
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_auth_list() -> CmdResult<Vec<oc::OpencodeAuthEntry>> {
    oc::list_auth().map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_set_provider_key(
    state: State<'_, AppState>,
    provider: String,
    key: String,
) -> CmdResult<Vec<oc::OpencodeAuthEntry>> {
    oc::set_provider_key(&provider, &key).map_err(AppError::from)?;
    state.opencode.invalidate().await;
    oc::list_auth().map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_remove_provider(
    state: State<'_, AppState>,
    provider: String,
) -> CmdResult<Vec<oc::OpencodeAuthEntry>> {
    oc::remove_provider(&provider).map_err(AppError::from)?;
    state.opencode.invalidate().await;
    oc::list_auth().map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn opencode_set_default_model(
    state: State<'_, AppState>,
    model: Option<String>,
) -> CmdResult<()> {
    let normalized = model.filter(|m| !m.is_empty());
    state
        .settings
        .update(|s| s.providers.opencode.default_model = normalized.clone())
        .await?;
    oc::set_default_model(normalized.as_deref()).map_err(AppError::from)
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAuthStatus {
    pub connected: bool,
    pub provider: Option<String>,
    pub plan: Option<String>,
    pub org: Option<String>,
    pub account: Option<String>,
}

impl ClaudeAuthStatus {
    fn disconnected() -> Self {
        Self {
            connected: false,
            provider: None,
            plan: None,
            org: None,
            account: None,
        }
    }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn claude_auth_status(state: State<'_, AppState>) -> CmdResult<ClaudeAuthStatus> {
    let Some(bin) = state.binaries.resolve(BinaryKind::Claude).await else {
        return Ok(ClaudeAuthStatus::disconnected());
    };
    let path = state.binaries.augmented_path().await;
    let Ok(out) = Exec::new(&bin)
        .args(["auth", "status", "--json"])
        .env("PATH", path)
        .timeout(Duration::from_secs(10))
        .run_unchecked()
        .await
    else {
        return Ok(ClaudeAuthStatus::disconnected());
    };

    let Ok(v) = serde_json::from_str::<serde_json::Value>(out.stdout.trim()) else {
        return Ok(ClaudeAuthStatus::disconnected());
    };

    let connected = v.get("loggedIn").and_then(|x| x.as_bool()).unwrap_or(false);
    let provider = match v.get("apiProvider").and_then(|x| x.as_str()) {
        Some("firstParty") => Some("Anthropic API".to_string()),
        Some(other) => Some(other.to_string()),
        None => v
            .get("authMethod")
            .and_then(|x| x.as_str())
            .map(str::to_string),
    };
    let plan = v
        .get("subscriptionType")
        .and_then(|x| x.as_str())
        .map(title_case);
    let org = v
        .get("orgName")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let account = v.get("email").and_then(|x| x.as_str()).map(str::to_string);

    Ok(ClaudeAuthStatus {
        connected,
        provider,
        plan,
        org,
        account,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn claude_probe(state: State<'_, AppState>) -> CmdResult<ClaudeProbe> {
    let Some(bin) = state.binaries.resolve(BinaryKind::Claude).await else {
        return Ok(ClaudeProbe::default());
    };
    let path = state.binaries.augmented_path().await;
    let env = crate::commands::claude_auth_env(&state.settings.get().await.providers.claude);
    claude::probe::probe(&bin, &path, &claude::probe::neutral_cwd(), &env)
        .await
        .map_err(Into::into)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_probe(state: State<'_, AppState>) -> CmdResult<CodexProbe> {
    let Some(bin) = state.binaries.resolve(BinaryKind::Codex).await else {
        return Ok(CodexProbe::default());
    };
    let path = state.binaries.augmented_path().await;
    codex::probe::probe(&bin, &path).await.map_err(Into::into)
}

#[tauri::command]
#[specta::specta]
pub async fn codex_login_with_api_key(
    state: State<'_, AppState>,
    api_key: String,
) -> CmdResult<CodexProbe> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err(AppError {
            code: "codex_api_key_empty".into(),
            message: "Enter an API key first.".into(),
            remedy: None,
        });
    }
    let bin = state
        .binaries
        .require(BinaryKind::Codex)
        .await
        .map_err(AppError::from)?;
    let path = state.binaries.augmented_path().await;

    Exec::new(&bin)
        .args(["login", "--with-api-key"])
        .stdin(format!("{key}\n").into_bytes())
        .env("PATH", path.clone())
        .timeout(Duration::from_secs(30))
        .run()
        .await
        .map_err(AppError::from)?;

    Ok(codex::probe::probe(&bin, &path).await.unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub async fn codex_logout(state: State<'_, AppState>) -> CmdResult<CodexProbe> {
    let bin = state
        .binaries
        .require(BinaryKind::Codex)
        .await
        .map_err(AppError::from)?;
    let path = state.binaries.augmented_path().await;
    let _ = Exec::new(&bin)
        .arg("logout")
        .env("PATH", path.clone())
        .timeout(Duration::from_secs(15))
        .run_unchecked()
        .await;
    Ok(codex::probe::probe(&bin, &path).await.unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub async fn install_opencode(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let path = state.binaries.augmented_path().await;
    let shell = super::terminal::default_shell();

    #[cfg(windows)]
    {
        let has_npm = Exec::new(&shell)
            .args([
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "npm --version".to_string(),
            ])
            .env("PATH", path.as_os_str())
            .timeout(Duration::from_secs(20))
            .run()
            .await
            .is_ok();
        if !has_npm {
            return Err(AppError {
                code: "npm_missing".into(),
                message: "Node.js (npm) isn't installed, so OpenCode can't be installed automatically."
                    .into(),
                remedy: Some(
                    "Install Node.js (it includes npm) and click Refresh, or install OpenCode with \
                     Scoop (scoop install opencode), Chocolatey (choco install opencode), or WSL. \
                     You can also use Locate… to point at an existing opencode.exe."
                        .into(),
                ),
            });
        }
    }

    #[cfg(not(windows))]
    let args = vec![
        "-lc".to_string(),
        "curl -fsSL https://opencode.ai/install | bash".to_string(),
    ];
    #[cfg(windows)]
    let args = vec![
        "-NoProfile".to_string(),
        "-Command".to_string(),
        "npm install -g opencode-ai".to_string(),
    ];

    let result = Exec::new(&shell)
        .args(args)
        .env("PATH", path)
        .timeout(Duration::from_secs(300))
        .run()
        .await;

    #[cfg(windows)]
    result.map_err(|e| AppError {
        code: "opencode_install_failed".into(),
        message: format!("Couldn't install OpenCode via npm. {e}"),
        remedy: Some(
            "Install Node.js/npm, or install OpenCode with Scoop \
             (scoop install opencode), Chocolatey (choco install opencode), or WSL. \
             then click Refresh. You can also use Locate… to point at an existing \
             opencode.exe."
                .into(),
        ),
    })?;
    #[cfg(not(windows))]
    result.map_err(AppError::from)?;

    state.binaries.redetect().await;
    let _ = BinariesChanged.emit(&app);
    Ok(())
}
