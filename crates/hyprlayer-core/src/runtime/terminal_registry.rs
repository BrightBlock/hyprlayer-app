use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::terminal::PtySession;

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

    pub fn kill_all_blocking(&self) {
        if let Ok(mut live) = self.live.try_lock() {
            for (_, session) in live.drain() {
                session.kill();
            }
        }
    }

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
