use std::collections::HashMap;
use std::sync::{Arc, Mutex as SyncMutex, PoisonError};

use tokio::sync::Mutex;

use crate::terminal::PtySession;

#[derive(Default)]
pub struct TerminalRegistry {
    live: Mutex<HashMap<String, Arc<PtySession>>>,
    agent_fit: SyncMutex<Option<AgentFit>>,
}

/// The last agent tab's size and the desktop that reported it.
#[derive(Clone, Copy)]
struct AgentFit {
    viewer: Option<u64>,
    size: (u16, u16),
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

    pub fn note_agent_fit(&self, viewer: Option<u64>, cols: u16, rows: u16) {
        if cols > 0 && rows > 0 {
            *self.lock_fit() = Some(AgentFit {
                viewer,
                size: (cols, rows),
            });
        }
    }

    /// A spawn takes the size of the agent tab the desktop that reported it would open in.
    pub fn agent_fit(&self) -> Option<(u16, u16)> {
        self.lock_fit().map(|fit| fit.size)
    }

    /// A desktop that left can no longer open the agent, so its tab's size is stale.
    pub fn forget_agent_fit(&self, viewer: u64) {
        let mut fit = self.lock_fit();
        if fit.is_some_and(|fit| fit.viewer == Some(viewer)) {
            *fit = None;
        }
    }

    fn lock_fit(&self) -> std::sync::MutexGuard<'_, Option<AgentFit>> {
        self.agent_fit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// Kills them and returns the sessions, for a caller that must wait for them to let go.
    pub async fn remove_for_workstream(&self, ws: &str) -> Vec<Arc<PtySession>> {
        let mut g = self.live.lock().await;
        let ids: Vec<String> = g
            .iter()
            .filter(|(_, s)| s.workstream_id == ws)
            .map(|(k, _)| k.clone())
            .collect();
        let removed: Vec<_> = ids.iter().filter_map(|id| g.remove(id)).collect();
        for s in &removed {
            s.kill();
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_departed_desktop_takes_its_agent_fit_with_it() {
        let registry = TerminalRegistry::new();
        registry.note_agent_fit(Some(1), 150, 45);
        registry.forget_agent_fit(2);
        assert_eq!(registry.agent_fit(), Some((150, 45)));
        registry.forget_agent_fit(1);
        assert_eq!(registry.agent_fit(), None);
    }
}
