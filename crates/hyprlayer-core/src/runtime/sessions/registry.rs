use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent::AgentDriver;
use crate::claude::PermissionMode;

struct LiveSession {
    driver: Arc<dyn AgentDriver>,
    permission_mode: Option<PermissionMode>,
}

#[derive(Default)]
pub struct SessionRegistry {
    live: Mutex<HashMap<String, LiveSession>>,
    op_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
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

    pub async fn op_lock(&self, id: &str) -> Arc<Mutex<()>> {
        self.op_locks
            .lock()
            .await
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn get(&self, id: &str) -> Option<Arc<dyn AgentDriver>> {
        self.live.lock().await.get(id).map(|s| s.driver.clone())
    }

    pub async fn permission_mode(&self, id: &str) -> Option<PermissionMode> {
        self.live
            .lock()
            .await
            .get(id)
            .and_then(|s| s.permission_mode)
    }

    pub async fn remove(&self, id: &str) {
        let gone = self.live.lock().await.remove(id);
        drop(gone);
        self.op_locks.lock().await.remove(id);
    }

    pub async fn live_ids(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }

    pub fn kill_all_blocking(&self) {
        if let Ok(live) = self.live.try_lock() {
            for session in live.values() {
                session.driver.kill_detached();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::SessionEvent;
    use crate::error::Result;
    use hyprlayer_models::session::QuestionAnswers;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::broadcast;

    #[derive(Default)]
    struct Kills {
        hard: AtomicUsize,
        detached: AtomicUsize,
    }

    struct FakeDriver(Arc<Kills>);

    impl AgentDriver for FakeDriver {
        fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
            broadcast::channel(1).1
        }
        fn send_user_turn(&self, _: &str, _: Option<&str>, _: Option<&str>) -> Result<()> {
            Ok(())
        }
        fn respond_permission(&self, _: &str, _: bool, _: Option<&str>) -> Result<()> {
            Ok(())
        }
        fn answer_question(&self, _: &str, _: QuestionAnswers) -> Result<()> {
            Ok(())
        }
        fn interrupt(&self) -> Result<()> {
            Ok(())
        }
        fn kill(&self) {
            self.0.hard.fetch_add(1, Ordering::SeqCst);
        }
        fn kill_detached(&self) {
            self.0.detached.fetch_add(1, Ordering::SeqCst);
        }
        fn pid(&self) -> Option<u32> {
            None
        }
    }

    async fn registry_with(n: usize, kills: &Arc<Kills>) -> SessionRegistry {
        let reg = SessionRegistry::new();
        for i in 0..n {
            reg.insert(format!("s{i}"), Arc::new(FakeDriver(kills.clone())), None)
                .await;
        }
        reg
    }

    #[tokio::test]
    async fn kill_all_blocking_detaches_every_live_session() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(3, &kills).await;

        reg.kill_all_blocking();

        assert_eq!(kills.detached.load(Ordering::SeqCst), 3);
        assert_eq!(kills.hard.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn kill_all_blocking_returns_promptly() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(1, &kills).await;

        let start = Instant::now();
        reg.kill_all_blocking();

        assert_eq!(kills.detached.load(Ordering::SeqCst), 1);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "blocked on exit"
        );
        assert_eq!(reg.live_ids().await.len(), 1);
    }

    #[tokio::test]
    async fn kill_all_blocking_on_an_empty_registry_is_a_no_op() {
        let reg = SessionRegistry::new();
        reg.kill_all_blocking();
        assert!(reg.live_ids().await.is_empty());
    }

    #[tokio::test]
    async fn kill_all_blocking_skips_a_held_lock_instead_of_hanging() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(1, &kills).await;

        let guard = reg.live.lock().await;
        reg.kill_all_blocking();
        drop(guard);

        assert_eq!(kills.detached.load(Ordering::SeqCst), 0);
    }
}
