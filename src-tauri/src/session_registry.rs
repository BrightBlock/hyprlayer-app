//! Live-session registry: the in-memory map of running drivers. Each entry is an
//! `Arc<dyn AgentDriver>`, so Claude and OpenCode sessions share one map without
//! the registry knowing the harness. Session rows and event logs live in SQLite;
//! this holds only the handles needed to talk to a live session.
//!
//! There is no concurrency cap — the number of simultaneous agent sessions is
//! bounded only by the host's resources. Sessions across all projects share this
//! one map, keyed by session id.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::PermissionMode;
use tokio::sync::Mutex;

/// One live session's handle plus the `--permission-mode` its process was
/// actually launched with.
struct LiveSession {
    driver: Arc<dyn AgentDriver>,
    /// The mode the running claude process was launched with — `None` for
    /// OpenCode (which has no permission modes). claude binds its mode at launch
    /// and only changes it by relaunching, so this stays accurate for the life of
    /// the process. `resume_session` consults it to tell a stale re-attach (the
    /// UI wants a mode the live process isn't in) from a genuine no-change resume.
    permission_mode: Option<PermissionMode>,
}

#[derive(Default)]
pub struct SessionRegistry {
    live: Mutex<HashMap<String, LiveSession>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(
        &self,
        id: String,
        driver: Arc<dyn AgentDriver>,
        permission_mode: Option<PermissionMode>,
    ) {
        self.live.lock().await.insert(
            id,
            LiveSession {
                driver,
                permission_mode,
            },
        );
    }

    pub async fn get(&self, id: &str) -> Option<Arc<dyn AgentDriver>> {
        self.live.lock().await.get(id).map(|s| s.driver.clone())
    }

    /// The `--permission-mode` the live session's process is actually running, or
    /// `None` if the session isn't live or has no mode (OpenCode).
    pub async fn permission_mode(&self, id: &str) -> Option<PermissionMode> {
        self.live.lock().await.get(id).and_then(|s| s.permission_mode)
    }

    pub async fn remove(&self, id: &str) {
        self.live.lock().await.remove(id);
    }

    /// Snapshot the ids of all currently-live sessions.
    pub async fn live_ids(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }
}
