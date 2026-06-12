use hyprlayer_core::CoreError;
use serde::Serialize;

/// Error shape every IPC command returns. The frontend renders `message`,
/// shows `remedy` as the call-to-action, and may branch on `code`.
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
}

pub type CmdResult<T> = Result<T, AppError>;
