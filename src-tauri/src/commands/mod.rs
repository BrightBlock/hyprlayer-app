pub mod account;
pub mod agents;
pub mod attachments;
pub mod diagnostics;
pub mod harnesses;
pub mod hosts;
pub mod library;
pub mod ollama;
pub mod onboarding;
pub mod projects;
pub mod prompts;
pub mod reviews;
pub mod scripts;
pub mod sessions;
pub mod skills;
pub mod system;
pub mod terminal;
pub mod workstreams;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

pub(crate) async fn ensure_registered_project(
    state: &AppState,
    project_path: &str,
) -> CmdResult<()> {
    let known = state
        .client()?
        .find_project(project_path.to_string())
        .await
        .map_err(AppError::from)?;
    if known.is_none() {
        return Err(AppError {
            code: "unknown_project".into(),
            message: format!("{project_path} is not a registered project."),
            remedy: None,
            stale_transcript: None,
        });
    }
    Ok(())
}
