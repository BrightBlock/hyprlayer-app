use hyprlayer_core::hyprcli::AiStatus;
use hyprlayer_core::runtime::projects as core_projects;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_specta::Event;

use super::projects::Bootstrap;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum ThoughtsSource {
    Clone {
        url: String,
        location: Option<String>,
    },
    Disk {
        path: String,
    },
    Create {
        path: String,
    },
}

impl From<ThoughtsSource> for core_projects::ThoughtsSource {
    fn from(s: ThoughtsSource) -> Self {
        match s {
            ThoughtsSource::Clone { url, location } => Self::Clone { url, location },
            ThoughtsSource::Disk { path } => Self::Disk { path },
            ThoughtsSource::Create { path } => Self::Create { path },
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_ai_status(state: State<'_, AppState>) -> CmdResult<AiStatus> {
    state
        .runtime
        .projects()
        .ai_status()
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsProbe {
    pub default_path: String,
    pub exists: bool,
    pub is_git_repo: bool,
}

impl From<core_projects::ThoughtsProbe> for ThoughtsProbe {
    fn from(p: core_projects::ThoughtsProbe) -> Self {
        Self {
            default_path: p.default_path,
            exists: p.exists,
            is_git_repo: p.is_git_repo,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn probe_default_thoughts(state: State<'_, AppState>) -> ThoughtsProbe {
    state.runtime.projects().probe_default_thoughts().into()
}

#[tauri::command]
#[specta::specta]
pub async fn init_thoughts_repo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    user: String,
    source: ThoughtsSource,
) -> CmdResult<Bootstrap> {
    let result = state
        .runtime
        .projects()
        .run_onboarding(core_projects::RunOnboardingRequest {
            user,
            source: source.into(),
        })
        .await
        .map_err(AppError::from)?;
    let _ = crate::events::ConfigChanged.emit(&app);
    Ok(Bootstrap::from(result))
}
