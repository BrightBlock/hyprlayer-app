use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_models::{AgentCaller, Harness, PermissionMode};

#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub caller: AgentCaller,
    pub workstream_id: String,
    pub project_path: String,
    pub depth: u32,
    pub parent: Option<AgentCaller>,
    /// Terminals keep it; sessions resolve theirs live and fall back to it until the registry knows them.
    pub launch_mode: Option<PermissionMode>,
    pub report_only: bool,
    /// What the caller runs on: its composer harness, or the harness its PTY engine is.
    pub harness: Option<Harness>,
    pub lead: bool,
}

/// Std mutex: `TokenGuard::drop` cannot await.
#[derive(Clone, Default)]
pub struct AgentTokens {
    live: Arc<Mutex<HashMap<String, AgentIdentity>>>,
}

impl AgentTokens {
    pub fn mint(&self, identity: AgentIdentity) -> TokenGuard {
        let token = uuid::Uuid::new_v4().to_string();
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(token.clone(), identity);
        TokenGuard {
            token,
            tokens: self.clone(),
        }
    }

    /// `None` once the spawn that held the token has exited.
    pub fn lookup(&self, token: &str) -> Option<AgentIdentity> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(token)
            .cloned()
    }

    fn revoke(&self, token: &str) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(token);
    }
}

/// Revokes its token on drop.
pub struct TokenGuard {
    token: String,
    tokens: AgentTokens,
}

impl TokenGuard {
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for TokenGuard {
    fn drop(&mut self) {
        self.tokens.revoke(&self.token);
    }
}
