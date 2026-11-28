//! What the UI shows for a failed call: the wire envelope's message and remedy (`errorMessage`/`errorRemedy` in the webview).

use hyprlayer_client::error::AppError;
use hyprlayer_proto::WireError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiError {
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
}

impl From<AppError> for UiError {
    fn from(e: AppError) -> Self {
        Self {
            code: e.code,
            message: e.message,
            remedy: e.remedy,
        }
    }
}

impl From<WireError> for UiError {
    fn from(e: WireError) -> Self {
        Self {
            code: e.code,
            message: e.message,
            remedy: e.remedy,
        }
    }
}
