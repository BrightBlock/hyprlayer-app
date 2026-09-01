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

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub(crate) async fn blocking<T, F>(label: &str, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("{label} task: {e}")))
}

pub(crate) async fn ensure_registered_project(
    state: &AppState,
    project_path: &str,
) -> CmdResult<()> {
    let known = state
        .runtime
        .projects()
        .find(project_path)
        .await
        .map_err(AppError::from)?;
    if known.is_none() {
        return Err(AppError {
            code: "unknown_project".into(),
            message: format!("{project_path} is not a registered project."),
            remedy: None,
        });
    }
    Ok(())
}

pub(crate) async fn effective_for(
    state: &AppState,
    project_path: &str,
) -> CmdResult<hyprlayer_core::config::EffectiveConfig> {
    let loaded = state
        .runtime
        .settings()
        .config()
        .load()
        .await
        .map_err(AppError::from)?;
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .ok_or_else(AppError::thoughts_not_configured)?;
    Ok(thoughts.effective_config_for(project_path))
}
