pub mod agents;
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

use hyprlayer_core::branch_prefix::BranchPrefix;
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

/// Env vars carrying Claude API-key-mode auth/routing into a spawned `claude`.
/// Empty for CLI-login auth (the child inherits the user's `~/.claude`). Shared
/// by the session driver and the tab-title generator so the two never drift.
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

/// Trim a branch-type list to the bytes that get stored: labels and prefixes
/// trimmed, keywords trimmed, lowercased and stripped of empties.
///
/// Shared by the two write paths — `set_branch_prefixes` (per-repo) and
/// `set_app_settings` (global) — so the same list saved from the same editor
/// cannot end up normalized differently depending on which surface it came
/// from. It also makes each write path validate exactly what it stores, since
/// `validate_prefixes` compares after trimming.
pub(crate) fn normalize_branch_prefixes(prefixes: Vec<BranchPrefix>) -> Vec<BranchPrefix> {
    prefixes
        .into_iter()
        .map(|p| BranchPrefix {
            label: p.label.trim().to_string(),
            prefix: p.prefix.trim().to_string(),
            keywords: p
                .keywords
                .into_iter()
                .map(|k| k.trim().to_lowercase())
                .filter(|k| !k.is_empty())
                .collect(),
        })
        .collect()
}

/// Reject a `project_path` that isn't a registered project. These commands are
/// webview-callable with an arbitrary path and read/write a file under it, so
/// they must not act on paths the user never added (the run paths instead take
/// the project from a db-loaded workstream, already trusted).
pub(crate) async fn ensure_registered_project(
    state: &AppState,
    project_path: &str,
) -> CmdResult<()> {
    let path = project_path.to_string();
    let known = db_task(state.db.clone(), move |db| db.get_project_record(&path)).await?;
    if known.is_none() {
        return Err(AppError {
            code: "unknown_project".into(),
            message: format!("{project_path} is not a registered project."),
            remedy: None,
        });
    }
    Ok(())
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
