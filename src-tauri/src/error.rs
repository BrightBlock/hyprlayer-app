use hyprlayer_core::CoreError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
}

impl From<CoreError> for AppError {
    fn from(err: CoreError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.to_string(),
            remedy: err.remedy(),
        }
    }
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
            remedy: None,
        }
    }

    pub fn project_not_found(path: &str) -> Self {
        Self {
            code: "project_not_found".into(),
            message: format!("No hyprlayer project is mapped at {path}"),
            remedy: Some("Add the repository as a project first.".into()),
        }
    }

    pub fn thoughts_not_configured() -> Self {
        Self {
            code: "config_missing".into(),
            message: "Thoughts are not configured yet.".into(),
            remedy: Some("Add the repository as a project first.".into()),
        }
    }
}

pub type CmdResult<T> = Result<T, AppError>;
