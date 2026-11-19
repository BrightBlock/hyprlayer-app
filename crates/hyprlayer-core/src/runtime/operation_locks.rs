use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex, Weak};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Per-key async mutexes for workstream lifecycle transactions, keyed by
/// project path and by workstream id. Entries are held only by `Weak`
/// references between checkouts, so a project or workstream that stops being
/// touched does not pin a mutex in memory forever; the map itself keeps
/// growing (dead `Weak`s are pruned lazily on the next checkout for the same
/// key), which is the cheap side of the trade.
///
/// Lock ordering is always project key, then workstream id — every caller
/// that needs both goes through [`WorkstreamLocks::lock_pair`] so the order
/// is enforced in one place rather than re-derived at each call site.
#[derive(Default)]
pub(crate) struct WorkstreamLocks {
    projects: StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>,
    workstreams: StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

fn checkout(
    registry: &StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>,
    key: &str,
) -> Arc<AsyncMutex<()>> {
    let mut map = registry.lock().expect("lock registry mutex poisoned");
    if let Some(existing) = map.get(key).and_then(Weak::upgrade) {
        return existing;
    }
    let created = Arc::new(AsyncMutex::new(()));
    map.insert(key.to_string(), Arc::downgrade(&created));
    map.retain(|_, weak| weak.strong_count() > 0);
    created
}

/// Per-store async mutexes for skill installs, removes and restores, keyed by the
/// canonical path of the store's owning directory (ADR-0010).
#[derive(Default)]
pub(crate) struct SkillLocks {
    stores: StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

impl SkillLocks {
    pub(crate) async fn lock(&self, key: &Path) -> OwnedMutexGuard<()> {
        checkout(&self.stores, &key.to_string_lossy())
            .lock_owned()
            .await
    }
}

/// Holds both guards for the lifetime of a locked workstream operation. The
/// workstream guard is optional because a handful of protected operations
/// (an in-place create before its row exists, for instance) have only a
/// project key to lock against.
pub(crate) struct WorkstreamLockGuard {
    _project: OwnedMutexGuard<()>,
    _workstream: Option<OwnedMutexGuard<()>>,
}

impl WorkstreamLocks {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Locks only the project key. Used for the one-active-in-place guard,
    /// which runs before the workstream it might create has an id.
    pub(crate) async fn lock_project(&self, project_key: &str) -> WorkstreamLockGuard {
        let project = checkout(&self.projects, project_key).lock_owned().await;
        WorkstreamLockGuard {
            _project: project,
            _workstream: None,
        }
    }

    /// Locks the project key, then the workstream id, in that order — the
    /// one ordering every protected workstream operation uses, so two
    /// operations that both need both locks can never deadlock against each
    /// other.
    pub(crate) async fn lock_pair(
        &self,
        project_key: &str,
        workstream_id: &str,
    ) -> WorkstreamLockGuard {
        let project = checkout(&self.projects, project_key).lock_owned().await;
        let workstream = checkout(&self.workstreams, workstream_id)
            .lock_owned()
            .await;
        WorkstreamLockGuard {
            _project: project,
            _workstream: Some(workstream),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn same_project_key_serializes() {
        let locks = Arc::new(WorkstreamLocks::new());
        let order = Arc::new(StdMutex::new(Vec::new()));

        let locks2 = locks.clone();
        let order_a = order.clone();
        let order_b = order.clone();
        let first = tokio::spawn(async move {
            let _g = locks2.lock_project("proj-a").await;
            order_a.lock().unwrap().push("first-start");
            tokio::time::sleep(Duration::from_millis(30)).await;
            order_a.lock().unwrap().push("first-end");
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let locks3 = locks.clone();
        let second = tokio::spawn(async move {
            let _g = locks3.lock_project("proj-a").await;
            order_b.lock().unwrap().push("second-start");
        });

        first.await.unwrap();
        second.await.unwrap();

        let seen = order.lock().unwrap().clone();
        assert_eq!(seen, vec!["first-start", "first-end", "second-start"]);
    }

    #[tokio::test]
    async fn different_project_keys_run_concurrently() {
        let locks = Arc::new(WorkstreamLocks::new());
        let started = Arc::new(tokio::sync::Notify::new());

        let locks_a = locks.clone();
        let started_a = started.clone();
        let a = tokio::spawn(async move {
            let _g = locks_a.lock_project("proj-a").await;
            started_a.notify_one();
            tokio::time::sleep(Duration::from_millis(50)).await;
        });

        started.notified().await;
        // proj-b must not block behind proj-a's still-held guard.
        let locks_b = locks.clone();
        tokio::time::timeout(Duration::from_millis(20), async move {
            let _g = locks_b.lock_project("proj-b").await;
        })
        .await
        .expect("independent project keys do not contend");

        a.await.unwrap();
    }

    #[tokio::test]
    async fn dead_weak_entries_are_pruned_on_next_checkout() {
        let locks = WorkstreamLocks::new();
        {
            let _g = locks.lock_project("proj-a").await;
        }
        assert_eq!(locks.projects.lock().unwrap().len(), 1);
        assert!(
            locks
                .projects
                .lock()
                .unwrap()
                .get("proj-a")
                .and_then(Weak::upgrade)
                .is_none(),
            "the guard's Arc was the only strong reference"
        );

        // A second, unrelated checkout should sweep the dead entry above.
        {
            let _g = locks.lock_project("proj-b").await;
        }
        assert!(!locks.projects.lock().unwrap().contains_key("proj-a"));
    }

    #[tokio::test]
    async fn lock_pair_orders_project_then_workstream() {
        let locks = Arc::new(WorkstreamLocks::new());
        let held = checkout(&locks.workstreams, "ws-1").lock_owned().await;
        let pair = tokio::spawn({
            let locks = locks.clone();
            async move {
                let _g = locks.lock_pair("proj-a", "ws-1").await;
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            checkout(&locks.projects, "proj-a").try_lock().is_err(),
            "the project lock is taken before waiting on the workstream"
        );
        drop(held);
        pair.await.unwrap();
    }
}
