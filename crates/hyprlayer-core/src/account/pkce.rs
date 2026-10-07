use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result};

/// RFC 7636 S256. Only the challenge leaves the server; the verifier is redeemed by it alone.
pub(crate) struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub(crate) fn generate() -> Result<Self> {
        let verifier = random_token()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

/// 32 CSPRNG bytes as base64url: 43 characters, all RFC 7636 unreserved.
pub(crate) fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| CoreError::Io(std::io::Error::other(format!("no system randomness: {e}"))))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
