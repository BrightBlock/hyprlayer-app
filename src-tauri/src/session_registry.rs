//! Live-session registry: the in-memory map of running [`ClaudeDriver`]s plus a
//! concurrency cap. Session rows and event logs live in SQLite; this holds only
//! the handles needed to talk to a process while it's alive.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::claude::ClaudeDriver;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

pub struct SessionRegistry {
    live: Mutex<HashMap<String, Arc<ClaudeDriver>>>,
    limit: Arc<Semaphore>,
}

impl SessionRegistry {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            live: Mutex::new(HashMap::new()),
            limit: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Reserve a session slot, or `None` if at the concurrency cap. The permit
    /// is held by the forwarder task for the session's whole life.
    pub fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.limit.clone().try_acquire_owned().ok()
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
}
