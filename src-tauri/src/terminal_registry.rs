//! Live-terminal registry: the in-memory map of running [`PtySession`]s.
//!
//! Mirrors [`SessionRegistry`] with the addition of a workstream-scoped
//! teardown used by archive_workstream.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::terminal::PtySession;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct TerminalRegistry {
    live: Mutex<HashMap<String, Arc<PtySession>>>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, id: String, s: Arc<PtySession>) {
        self.live.lock().await.insert(id, s);
    }

    pub async fn get(&self, id: &str) -> Option<Arc<PtySession>> {
        self.live.lock().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) -> Option<Arc<PtySession>> {
        self.live.lock().await.remove(id)
    }

    /// Best-effort synchronous teardown for app exit: kill every live shell.
    /// Uses `try_lock` so it can run from the (sync) `RunEvent` exit handler
    /// without an async context; on the rare miss, the PTY master's
    /// SIGHUP-on-drop when the process exits is the backstop.
    pub fn kill_all_blocking(&self) {
        if let Ok(mut live) = self.live.try_lock() {
            for (_, session) in live.drain() {
                session.kill();
            }
        }
    }

    /// Kill + drop every terminal belonging to a workstream (archive path).
    pub async fn remove_for_workstream(&self, ws: &str) {
        let mut g = self.live.lock().await;
        let ids: Vec<String> = g
            .iter()
            .filter(|(_, s)| s.workstream_id == ws)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some(s) = g.remove(&id) {
                s.kill();
            }
        }
    }
}
