pub mod attachments;
pub mod harnesses;
pub mod library;
pub mod onboarding;
pub mod projects;
pub mod sessions;
pub mod system;
pub mod terminal;
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
