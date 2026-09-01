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

#[cfg(test)]
mod tests {
    use super::*;

    const V0_7_APP_ERRORS_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/compat/v0_7/app-errors.json"
    ));

    fn pinned(slice: &str) -> (String, String, Option<String>) {
        let fixture: serde_json::Value = serde_json::from_str(V0_7_APP_ERRORS_FIXTURE).unwrap();
        let entry = &fixture[slice];
        (
            entry["code"].as_str().unwrap().to_string(),
            entry["message"].as_str().unwrap().to_string(),
            entry["remedy"].as_str().map(str::to_string),
        )
    }

    #[test]
    fn binary_missing_maps_to_the_pinned_v0_7_fixture() {
        let err = AppError::from(CoreError::BinaryMissing {
            name: "claude".into(),
        });
        assert_eq!((err.code, err.message, err.remedy), pinned("binaries"));
    }

    #[test]
    fn config_missing_carries_no_extra_punctuation() {
        let err = AppError::from(CoreError::ConfigMissing {
            path: "/nonexistent-hyprlayer-config-fixture/hyprlayer/config.json".into(),
        });
        let (code, message, remedy) = pinned("settingsConfig");
        assert_eq!((err.code, err.message, err.remedy), (code, message, remedy));
    }

    #[test]
    fn project_not_found_is_a_public_constructor() {
        let err = AppError::project_not_found("/tmp/example-project");
        let (code, message, remedy) = pinned("projects");
        assert_eq!((err.code, err.message, err.remedy), (code, message, remedy));
    }

    #[test]
    fn internal_carries_no_remedy() {
        assert_eq!(AppError::internal("boom").remedy, None);
    }
}
