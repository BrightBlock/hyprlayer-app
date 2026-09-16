//! macOS Keychain adapter for Claude's OAuth credential. It lives with the
//! runtime (ADR-0009), so a server must run as a launchd user agent: `security`
//! needs the unlocked login keychain of a user session.

use super::ports::{
    CredentialLookup, CredentialLookupError, CredentialLookupRequest, CredentialLookupResult,
    CredentialValue,
};
use super::providers::extract_claude_oauth_token;

const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

pub struct KeychainCredentialLookup;

impl CredentialLookup for KeychainCredentialLookup {
    fn lookup(
        &self,
        request: &CredentialLookupRequest,
    ) -> std::result::Result<CredentialLookupResult, CredentialLookupError> {
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

        match extract_claude_oauth_token(raw.trim()) {
            Some(token) => Ok(CredentialLookupResult::Found(CredentialValue::new(token))),
            None => Ok(CredentialLookupResult::NotFound),
        }
    }
}
