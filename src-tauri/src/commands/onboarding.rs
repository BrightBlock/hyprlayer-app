use hyprlayer_core::hyprcli::AiStatus;
use hyprlayer_core::runtime::projects as core_projects;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_specta::Event;

use super::projects::Bootstrap;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, specta::Type)]
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
        .client()?
        .get_ai_status()
        .await
        .map_err(AppError::from)
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
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

// Infallible in the bindings, so a dead transport degrades to an empty probe
// rather than changing the wire type or leaving the promise hanging.
#[tauri::command]
#[specta::specta]
pub async fn probe_default_thoughts(app: tauri::AppHandle) -> ThoughtsProbe {
    let empty = || ThoughtsProbe {
        default_path: String::new(),
        exists: false,
        is_git_repo: false,
    };
    let state = app.state::<AppState>();
    let Ok(client) = state.client() else {
        return empty();
    };
    match client.probe_default_thoughts().await {
        Ok(probe) => probe.into(),
        Err(e) => {
            log::error!("probe_default_thoughts: {}", e.message);
            empty()
        }
    }
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
        .client()?
        .init_thoughts_repo(user, source.into())
        .await
        .map_err(AppError::from)?;
    let _ = crate::events::ConfigChanged.emit(&app);
    Ok(Bootstrap::from(result))
}
