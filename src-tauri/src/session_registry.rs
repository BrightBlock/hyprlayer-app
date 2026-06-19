//! Live-session registry: the in-memory map of running [`ClaudeDriver`]s. Session
//! rows and event logs live in SQLite; this holds only the handles needed to talk
//! to a process while it's alive.
//!
//! There is no concurrency cap — the number of simultaneous agent sessions is
//! bounded only by the host's resources. Sessions across all projects share this
//! one map, keyed by session id.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::claude::ClaudeDriver;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct SessionRegistry {
    live: Mutex<HashMap<String, Arc<ClaudeDriver>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, id: String, driver: Arc<ClaudeDriver>) {
        self.live.lock().await.insert(id, driver);
    }

    pub async fn get(&self, id: &str) -> Option<Arc<ClaudeDriver>> {
        self.live.lock().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) {
        self.live.lock().await.remove(id);
    }

    /// Snapshot the ids of all currently-live sessions.
    pub async fn live_ids(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }
}
