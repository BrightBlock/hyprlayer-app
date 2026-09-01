use tauri::State;

use hyprlayer_core::runtime::providers::{self, TitleModel};

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn suggest_tab_title(state: State<'_, AppState>, text: String) -> CmdResult<String> {
    let settings = state.runtime.settings().get().await;
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
    let providers_service = state.runtime.providers();

    let raw = match providers::parse_model_tag(&model_tag) {
        TitleModel::Claude(flag) => {
            providers_service
                .generate_title_claude(flag, &prompt)
                .await?
        }
        TitleModel::OpenCode(model) => {
            providers_service
                .generate_title_opencode(model, &prompt)
                .await?
        }
        TitleModel::Codex(model) => {
            providers_service
                .generate_title_codex(model, &prompt)
                .await?
        }
    };

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
