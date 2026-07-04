pub mod attachments;
pub mod harnesses;
pub mod library;
pub mod models;
pub mod ollama;
pub mod onboarding;
pub mod projects;
pub mod prompts;
pub mod scripts;
pub mod sessions;
pub mod system;
pub mod terminal;
pub mod titles;
pub mod workstreams;

use std::sync::Arc;

use hyprlayer_core::store::Db;
use hyprlayer_core::CoreError;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// Run a closure against the SQLite store on the blocking pool.
pub(crate) async fn db_task<T, F>(db: Arc<Db>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Db) -> Result<T, CoreError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(&db))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
}

/// Environment variables that carry Claude API-key-mode auth/routing into a
/// spawned `claude` process. Empty for CLI-login auth (the child inherits the
/// user's `~/.claude` session). Shared by the session driver spawn
/// ([`crate::commands::sessions`]) and the one-shot tab-title generator
/// ([`crate::commands::titles`]) so the two never drift.
pub(crate) fn claude_auth_env(cfg: &crate::settings::ClaudeSettings) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if cfg.auth_mode.as_deref() != Some("apiKey") {
        return env;
    }
    if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        env.push(("ANTHROPIC_API_KEY".to_string(), key.to_string()));
    }
    if let Some(url) = cfg.base_url.as_deref().filter(|u| !u.is_empty()) {
        env.push(("ANTHROPIC_BASE_URL".to_string(), url.to_string()));
    }
    if cfg.use_bedrock {
        env.push(("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string()));
    }
    if cfg.use_vertex {
        env.push(("CLAUDE_CODE_USE_VERTEX".to_string(), "1".to_string()));
    }
    env
}

/// Run filesystem/CPU-bound work on the blocking pool.
pub(crate) async fn blocking<T, F>(label: &str, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("{label} task: {e}")))
}

/// The effective hyprlayer config for one project path.
pub(crate) async fn effective_for(
    state: &AppState,
    project_path: &str,
) -> CmdResult<hyprlayer_core::config::EffectiveConfig> {
    let loaded = state.config.load().await.map_err(AppError::from)?;
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .ok_or_else(AppError::thoughts_not_configured)?;
    Ok(thoughts.effective_config_for(project_path))
}
