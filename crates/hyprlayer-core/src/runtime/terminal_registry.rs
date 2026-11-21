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

    /// Registered until the session exits, whether or not anyone reads it.
    pub async fn insert(self: &Arc<Self>, id: String, s: Arc<PtySession>) {
        let mut exit = s.subscribe();
        let session = Arc::downgrade(&s);
        self.live.lock().await.insert(id.clone(), s);
        let registry = self.clone();
        tokio::spawn(async move {
            while exit.skip().await {}
            let mut live = registry.live.lock().await;
            // A relaunch may have reused the id.
            if live
                .get(&id)
                .is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), session.as_ptr()))
            {
                live.remove(&id);
            }
        });
    }

    pub async fn get(&self, id: &str) -> Option<Arc<PtySession>> {
        self.live.lock().await.get(id).cloned()
    }

    pub async fn all(&self) -> Vec<Arc<PtySession>> {
        self.live.lock().await.values().cloned().collect()
    }

    pub async fn remove(&self, id: &str) -> Option<Arc<PtySession>> {
        self.live.lock().await.remove(id)
    }

    pub async fn is_empty(&self) -> bool {
        self.live.lock().await.is_empty()
    }

    /// How many were killed, or `None` when the registry was busy and none were.
    pub fn kill_all_blocking(&self) -> Option<usize> {
        let Ok(mut live) = self.live.try_lock() else {
            tracing::warn!("terminal registry busy at shutdown; its shells were not killed");
            return None;
        };
        let count = live.len();
        for (_, session) in live.drain() {
            session.kill();
        }
        Some(count)
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
