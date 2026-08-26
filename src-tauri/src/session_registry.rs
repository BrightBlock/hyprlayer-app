use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::workflow::SessionWorkflows;
use hyprlayer_core::claude::{PermissionMode, SessionEvent};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{mpsc, oneshot, Mutex};

pub struct ForwarderHandle {
    pub join: JoinHandle<()>,
    pub shutdown: oneshot::Sender<()>,
}

#[derive(Clone)]
pub struct WorkflowChannel {
    pub tx: mpsc::UnboundedSender<SessionEvent>,
    pub rx: Arc<Mutex<mpsc::UnboundedReceiver<SessionEvent>>>,
    pub workflows: Arc<Mutex<SessionWorkflows>>,
}

impl WorkflowChannel {
    fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            workflows: Arc::new(Mutex::new(SessionWorkflows::new())),
        }
    }
}

struct LiveSession {
    driver: Arc<dyn AgentDriver>,
    permission_mode: Option<PermissionMode>,
    forwarder: Option<ForwarderHandle>,
    workflow: WorkflowChannel,
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
                forwarder: None,
                workflow: WorkflowChannel::new(),
            },
        );
    }

    pub async fn set_forwarder(&self, id: &str, handle: ForwarderHandle) {
        if let Some(session) = self.live.lock().await.get_mut(id) {
            if let Some(old) = session.forwarder.replace(handle) {
                old.join.abort();
            }
        }
    }

    pub async fn stop_forwarder(&self, id: &str) {
        let handle = {
            let mut live = self.live.lock().await;
            live.get_mut(id).and_then(|s| s.forwarder.take())
        };
        if let Some(handle) = handle {
            let _ = handle.shutdown.send(());
            let _ = handle.join.await;
        }
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

    pub async fn workflow(&self, id: &str) -> Option<WorkflowChannel> {
        self.live.lock().await.get(id).map(|s| s.workflow.clone())
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
    use hyprlayer_core::error::Result;
    use serde_json::Value;
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
        fn answer_question(&self, _: &str, _: Value) -> Result<()> {
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
    async fn the_workflow_channel_is_per_session_not_per_forwarder() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(1, &kills).await;

        let first = reg.workflow("s0").await.expect("a live session has one");
        first.tx.send(SessionEvent::Rewound {}).unwrap();
        drop(first);

        let second = reg.workflow("s0").await.unwrap();
        assert!(
            second.rx.lock().await.try_recv().is_ok(),
            "the queued event must survive the forwarder swap"
        );

        reg.remove("s0").await;
        assert!(reg.workflow("s0").await.is_none());
    }

    #[tokio::test]
    async fn remove_drops_the_sessions_workflow_state() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(1, &kills).await;

        let weak = {
            let w = reg.workflow("s0").await.expect("a live session has one");
            Arc::downgrade(&w.workflows)
        };
        assert!(weak.upgrade().is_some(), "the live session holds it");

        reg.remove("s0").await;

        assert!(
            weak.upgrade().is_none(),
            "something outside the registry is keeping this session's workflow watch alive"
        );
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
