use tauri::State;

use crate::error::CmdResult;
use crate::state::AppState;

pub use hyprlayer_core::runtime::providers::ModelWindow;

#[tauri::command]
#[specta::specta]
pub async fn claude_model_windows(state: State<'_, AppState>) -> CmdResult<Vec<ModelWindow>> {
    Ok(state.runtime.providers().claude_model_windows().await?)
}

#[cfg(target_os = "macos")]
pub struct KeychainCredentialLookup;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

#[cfg(target_os = "macos")]
impl hyprlayer_core::runtime::ports::CredentialLookup for KeychainCredentialLookup {
    fn lookup(
        &self,
        request: &hyprlayer_core::runtime::ports::CredentialLookupRequest,
    ) -> std::result::Result<
        hyprlayer_core::runtime::ports::CredentialLookupResult,
        hyprlayer_core::runtime::ports::CredentialLookupError,
    > {
        use hyprlayer_core::runtime::ports::{CredentialLookupError, CredentialLookupResult};

        if request.provider != "claude" {
            return Ok(CredentialLookupResult::NotFound);
        }

        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output()
            .map_err(|e| CredentialLookupError(format!("security lookup failed: {e}")))?;
        if !out.status.success() {
            return Ok(CredentialLookupResult::NotFound);
        }
        let raw = String::from_utf8(out.stdout)
            .map_err(|e| CredentialLookupError(format!("security output was not utf-8: {e}")))?;

        match hyprlayer_core::runtime::providers::extract_claude_oauth_token(raw.trim()) {
            Some(token) => Ok(CredentialLookupResult::Found(
                hyprlayer_core::runtime::ports::CredentialValue::new(token),
            )),
            None => Ok(CredentialLookupResult::NotFound),
        }
    }
}
