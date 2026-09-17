use tauri::State;

use hyprlayer_core::runtime::providers::{self, TitleModel};

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn suggest_tab_title(state: State<'_, AppState>, text: String) -> CmdResult<String> {
    let settings = state
        .client()?
        .get_app_settings()
        .await
        .map_err(AppError::from)?;
    if !settings.tab_titles.enabled {
        return Err(AppError {
            code: "tab_titles_disabled".into(),
            message: "auto tab titles are turned off".into(),
            remedy: None,
        });
    }
    let model_tag = settings
        .tab_titles
        .effective_model(&settings.providers)
        .to_string();
    let prompt = providers::build_title_prompt(&text);

    let raw = match providers::parse_model_tag(&model_tag) {
        TitleModel::Claude(flag) => {
            state
                .client()?
                .generate_title_claude(flag.to_string(), prompt)
                .await
        }
        TitleModel::OpenCode(model) => {
            state
                .client()?
                .generate_title_opencode(model.to_string(), prompt)
                .await
        }
        TitleModel::Codex(model) => {
            state
                .client()?
                .generate_title_codex(model.to_string(), prompt)
                .await
        }
    }
    .map_err(AppError::from)?;

    let title = providers::sanitize_title(&raw);
    if title.is_empty() {
        return Err(AppError {
            code: "empty_title".into(),
            message: "model produced no usable title".into(),
            remedy: None,
        });
    }
    Ok(title)
}
