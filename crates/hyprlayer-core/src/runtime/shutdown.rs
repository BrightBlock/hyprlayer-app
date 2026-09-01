use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

pub const PER_TASK_TIMEOUT: Duration = Duration::from_secs(2);
pub const OVERALL_TIMEOUT: Duration = Duration::from_secs(10);

const OPEN: u8 = 0;
const RUNNING: u8 = 1;
const SHUTTING_DOWN: u8 = 2;
const STOPPED: u8 = 3;

#[derive(Debug, Clone, Default)]
pub struct ShutdownReport {
    pub elapsed: Duration,
    pub forced_process_count: usize,
    pub task_failures: Vec<String>,
}

/// A background loop the runtime owns end to end: `stop` wakes it (alongside
/// whatever timer/event it already selects on) and asks it to exit; if it
/// hasn't within its budget, `abort` forces it rather than leaking it past
/// shutdown.
pub struct TrackedTask {
    stop: Arc<Notify>,
    handle: tokio::task::JoinHandle<()>,
}

impl TrackedTask {
    pub fn spawn<F, Fut>(f: F) -> Self
    where
        F: FnOnce(Arc<Notify>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let stop = Arc::new(Notify::new());
        let handle = tokio::spawn(f(stop.clone()));
        Self { stop, handle }
    }

    /// Returns `true` if the task had to be aborted rather than exiting on its own.
    pub async fn stop(mut self, budget: Duration) -> bool {
        self.stop.notify_waiters();
        tokio::select! {
            _ = &mut self.handle => false,
            _ = tokio::time::sleep(budget) => {
                self.handle.abort();
                true
            }
        }
    }
}

pub struct ShutdownCoordinator {
    state: AtomicU8,
    notify: Notify,
    report: tokio::sync::Mutex<Option<ShutdownReport>>,
    sync_fired: std::sync::atomic::AtomicBool,
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(OPEN),
            notify: Notify::new(),
            report: tokio::sync::Mutex::new(None),
            sync_fired: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ShutdownCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_start(&self) -> crate::error::Result<()> {
        match self
            .state
            .compare_exchange(OPEN, RUNNING, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => Ok(()),
            Err(RUNNING) => Err(crate::CoreError::RuntimeAlreadyStarted),
            Err(_) => Err(crate::CoreError::RuntimeStopped),
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.state.load(Ordering::SeqCst) == STOPPED
    }

    /// Runs `teardown` exactly once regardless of how many callers race here;
    /// everyone else waits for the winner and clones its report.
    pub async fn shutdown(&self, teardown: impl Future<Output = ShutdownReport>) -> ShutdownReport {
        loop {
            let current = self.state.load(Ordering::SeqCst);
            if current == STOPPED {
                return self.report.lock().await.clone().unwrap_or_default();
            }
            if current == SHUTTING_DOWN {
                self.notify.notified().await;
                continue;
            }
            if self
                .state
                .compare_exchange(current, SHUTTING_DOWN, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let report = teardown.await;
                *self.report.lock().await = Some(report.clone());
                self.state.store(STOPPED, Ordering::SeqCst);
                self.notify.notify_waiters();
                return report;
            }
        }
    }

    /// The synchronous, idempotent fallback used from a non-async exit hook.
    /// `kill` runs at most once; a later async `shutdown` still runs in full.
    pub fn shutdown_now(&self, kill: impl FnOnce()) {
        if self
            .sync_fired
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            kill();
        }
    }
}

/// Stops every tracked task within the overall deadline, aborting whatever is
/// left once it's spent; the caller adds any forced-process count from
/// registered child processes (sessions, terminals, …) on top of this.
pub async fn stop_tasks(tasks: Vec<TrackedTask>) -> (usize, Duration) {
    let start = Instant::now();
    let mut forced = 0usize;
    for task in tasks {
        let remaining = OVERALL_TIMEOUT.saturating_sub(start.elapsed());
        let budget = PER_TASK_TIMEOUT.min(remaining);
        if task.stop(budget).await {
            forced += 1;
        }
    }
    (forced, start.elapsed())
}
