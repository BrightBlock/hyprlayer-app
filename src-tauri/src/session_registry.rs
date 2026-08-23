//! Live-session registry: the in-memory map of running drivers. Each entry is an
//! `Arc<dyn AgentDriver>`, so Claude and OpenCode sessions share one map without
//! the registry knowing the harness. Session rows and event logs live in SQLite;
//! this holds only the handles needed to talk to a live session.
//!
//! There is no concurrency cap, the number of simultaneous agent sessions is
//! bounded only by the host's resources. Sessions across all projects share this
//! one map, keyed by session id.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::workflow::SessionWorkflows;
use hyprlayer_core::claude::{PermissionMode, SessionEvent};
use tauri::async_runtime::JoinHandle;
use tokio::sync::{mpsc, oneshot, Mutex};

/// A running forwarder task plus the trigger to stop it COOPERATIVELY. Aborting a
/// forwarder mid-write only detaches the task, an in-flight `spawn_blocking` DB
/// append keeps running and could land after a rewind's cut, re-adding a dropped
/// row. So a live rewind signals `shutdown` and awaits `join` (see
/// [`SessionRegistry::stop_forwarder`]), draining any in-flight write first.
pub struct ForwarderHandle {
    pub join: JoinHandle<()>,
    pub shutdown: oneshot::Sender<()>,
}

/// The workflow plumbing shared by everything that touches one session's
/// `Workflow` runs: the injection channel into its forwarder, and the runs
/// themselves plus the single `notify` watch feeding them.
///
/// Per SESSION, not per forwarder, and that is the whole point. A live rewind
/// stops one forwarder and spawns a replacement across three awaited DB
/// round-trips (`sessions.rs:944` → `:971`) while the driver, the watch and
/// its reader task all keep running. A receiver owned by the forwarder would
/// be dropped by the first one — closing the channel and discarding its queue
/// — and the reader would write into nothing with its byte cursors already
/// advanced, so those events would be gone for good. Held here, the pair
/// outlives both forwarders.
#[derive(Clone)]
pub struct WorkflowChannel {
    /// Cloned into the reader task, and kept by the forwarder as well so
    /// `recv()` can never return `None` while the session is live.
    pub tx: mpsc::UnboundedSender<SessionEvent>,
    /// Shared rather than owned by the forwarder, per the note above.
    /// `recv()` and `lock()` are both cancel-safe, and `stop_forwarder`'s
    /// join guarantees no two forwarders ever poll it at once.
    pub rx: Arc<Mutex<mpsc::UnboundedReceiver<SessionEvent>>>,
    /// The runs being tailed plus this session's one recursive watch.
    /// Locked with `blocking_lock()` from inside `spawn_blocking`: every
    /// operation on it (`read_tick`, arming) is synchronous file I/O.
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

/// One live session's handle plus the `--permission-mode` its process was
/// actually launched with.
struct LiveSession {
    driver: Arc<dyn AgentDriver>,
    /// The mode the running claude process was launched with, `None` for
    /// OpenCode (which has no permission modes). claude binds its mode at launch
    /// and only changes it by relaunching, so this stays accurate for the life of
    /// the process. `resume_session` consults it to tell a stale re-attach (the
    /// UI wants a mode the live process isn't in) from a genuine no-change resume.
    permission_mode: Option<PermissionMode>,
    /// The forwarder task draining this session's event stream into the DB + UI
    /// channel. Held so a live rewind can stop it and hand a fresh forwarder to a
    /// new channel on the SAME process, the process stays alive, so the model's
    /// rewound memory survives (a kill + `--resume` would reload it from disk).
    forwarder: Option<ForwarderHandle>,
    /// This session's workflow watch, reader task and injection channel.
    /// Held HERE, not in `AppState`, so leaving the registry drops them:
    /// `SessionWorkflows`'s watch owns both the debouncer and the reader
    /// task handle, so [`remove`](SessionRegistry::remove) stops both by
    /// RAII with no teardown code — which is why nothing outside the registry
    /// may hold `workflow.workflows` strongly past the session's life; the
    /// reader task takes a `Weak` for exactly that reason
    /// (`commands::sessions::spawn_workflow_reader`). (`AppState`'s two watcher slots are
    /// single-slot — one index watch, one keyed worktree watch — which is
    /// the wrong shape for a resource several concurrent sessions each need
    /// one of.)
    workflow: WorkflowChannel,
}

#[derive(Default)]
pub struct SessionRegistry {
    live: Mutex<HashMap<String, LiveSession>>,
    /// Per-session operation lock, held for the span of a mutating op (a rewind
    /// peel, a user send) so those can't interleave on the one stdin: a send
    /// landing between two peel steps would reach the model at a point the log
    /// then deletes. Keyed by session id, kept out of `live` so it survives the
    /// forwarder swap a live rewind performs.
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

    /// Record (or replace) the forwarder task driving this session, so a later
    /// live rewind can [`stop_forwarder`](Self::stop_forwarder) it. Replacing an
    /// existing handle aborts the old one, the only callers that replace an
    /// existing forwarder (resume/restart) kill the process first, so its old
    /// forwarder is already ending; a live rewind takes the handle out via
    /// `stop_forwarder` before wiring the new one, so it never hits this path.
    /// No-op if the session isn't live.
    pub async fn set_forwarder(&self, id: &str, handle: ForwarderHandle) {
        if let Some(session) = self.live.lock().await.get_mut(id) {
            if let Some(old) = session.forwarder.replace(handle) {
                old.join.abort();
            }
        }
    }

    /// Stop this session's forwarder COOPERATIVELY, without touching the driver,
    /// the process stays live. Signals shutdown and awaits the task so any
    /// in-flight DB write finishes before the caller mutates the log (e.g. a
    /// rewind's cut). A fresh forwarder is then wired via `set_forwarder`.
    pub async fn stop_forwarder(&self, id: &str) {
        // Take the handle out under the lock, then release it before awaiting the
        // join (never hold the registry mutex across an await).
        let handle = {
            let mut live = self.live.lock().await;
            live.get_mut(id).and_then(|s| s.forwarder.take())
        };
        if let Some(handle) = handle {
            let _ = handle.shutdown.send(()); // ask the loop to break at its next poll
            let _ = handle.join.await; // …and wait until it has (writes drained)
        }
    }

    /// The operation lock for a session, created on first use. Callers hold its
    /// guard (`op_lock(id).await.lock().await`) for the span of a rewind/send so
    /// the two can't interleave on one stdin.
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

    /// The `--permission-mode` the live session's process is actually running, or
    /// `None` if the session isn't live or has no mode (OpenCode).
    pub async fn permission_mode(&self, id: &str) -> Option<PermissionMode> {
        self.live
            .lock()
            .await
            .get(id)
            .and_then(|s| s.permission_mode)
    }

    /// This session's workflow plumbing, or `None` if it isn't live. Every
    /// caller gets clones of the SAME three handles, which is what lets a
    /// forwarder, its replacement after a rewind, and the reader task all
    /// talk to one channel.
    pub async fn workflow(&self, id: &str) -> Option<WorkflowChannel> {
        self.live.lock().await.get(id).map(|s| s.workflow.clone())
    }

    pub async fn remove(&self, id: &str) {
        // Take the entry out under the lock and drop it AFTER the guard is
        // released. A `LiveSession` now owns a notify `Debouncer` (the
        // workflow watch), and while its `Drop` is one relaxed atomic store
        // on Linux, macOS's FSEvents backend joins its CFRunLoop thread
        // there. Dropping it inline as `remove(id)`'s discarded value would
        // block a runtime worker with the registry mutex held — the same
        // rule `stop_forwarder` follows for its await.
        let gone = self.live.lock().await.remove(id);
        drop(gone);
        // Drop the op lock too, the session is gone, so nothing can still be
        // serializing against it (any op holding a clone keeps its own Arc alive).
        self.op_locks.lock().await.remove(id);
    }

    /// Snapshot the ids of all currently-live sessions.
    pub async fn live_ids(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }

    /// Best-effort synchronous teardown for app exit: kill every live agent so its
    /// process tree doesn't outlive us. Mirrors [`TerminalRegistry::kill_all_blocking`],
    /// `try_lock` because the `RunEvent` exit handler has no async context, and
    /// `AgentDriver::kill` is synchronous (SIGTERM to the process group / `taskkill
    /// /T`). On the rare lock miss, Windows still has the drivers' job objects
    /// (`process::spawn_supervised`); Unix has nothing, which is the case this
    /// exists for.
    ///
    /// Detaching, not killing: quitting is the app's own bookkeeping, so the
    /// exits report `Detached` and persist nothing. Whether a forwarder wins the
    /// race with the window teardown therefore doesn't matter, either way the
    /// session replays clean and resumable, exactly as it did back when quitting
    /// orphaned these processes instead of ending them.
    ///
    /// It deliberately does not drop the `LiveSession`s (its own test asserts
    /// they survive — draining is the forwarder's job), so a session's
    /// workflow watch and reader task are not stopped here either. They die
    /// with the process moments later, and reaping them on this path would
    /// mean running a `Drop` on the exit path this exists to keep fast.
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

    /// Counts each kind of kill separately, app-exit teardown must DETACH, not
    /// kill, or every session logs a "Session ended" on the next launch.
    #[derive(Default)]
    struct Kills {
        hard: AtomicUsize,
        detached: AtomicUsize,
    }

    /// A real driver's forwarder is what removes the session from the registry
    /// once the process exits, so the tests drive that half by hand to model
    /// "the exit landed" vs "the child is wedged".
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

    /// Every live session goes down, as a DETACH, so quitting leaves the
    /// transcripts clean and the sessions resumable.
    #[tokio::test]
    async fn kill_all_blocking_detaches_every_live_session() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(3, &kills).await;

        reg.kill_all_blocking();

        assert_eq!(kills.detached.load(Ordering::SeqCst), 3);
        assert_eq!(kills.hard.load(Ordering::SeqCst), 0);
    }

    /// Runs on the exit path, so it must never block, not even with a wedged
    /// child that will never drain from the registry.
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
        assert_eq!(reg.live_ids().await.len(), 1); // draining is the forwarder's job
    }

    /// The exit path fires twice (ExitRequested, then Exit); the second pass over
    /// an empty registry must be a no-op.
    #[tokio::test]
    async fn kill_all_blocking_on_an_empty_registry_is_a_no_op() {
        let reg = SessionRegistry::new();
        reg.kill_all_blocking();
        assert!(reg.live_ids().await.is_empty());
    }

    /// A live rewind stops one forwarder and spawns another; both take the
    /// SAME per-session channel, so an event the reader queued during the
    /// swap is still there for the replacement. A per-forwarder pair would
    /// have closed with the first forwarder, and the reader's byte cursors
    /// have already moved past those events.
    #[tokio::test]
    async fn the_workflow_channel_is_per_session_not_per_forwarder() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(1, &kills).await;

        let first = reg.workflow("s0").await.expect("a live session has one");
        first.tx.send(SessionEvent::Rewound {}).unwrap();
        drop(first); // the forwarder that queued it is gone

        let second = reg.workflow("s0").await.unwrap();
        // `try_recv`, not `recv().await`: a per-forwarder pair would leave
        // this receiver empty forever, and the point is to fail rather than park.
        assert!(
            second.rx.lock().await.try_recv().is_ok(),
            "the queued event must survive the forwarder swap"
        );

        reg.remove("s0").await;
        assert!(reg.workflow("s0").await.is_none());
    }

    /// Removing a session must drop its workflow state, not just its map
    /// entry: the watch's debouncer and the reader task it aborts are stopped
    /// by that drop and by nothing else, so a clone cached anywhere outside
    /// the registry leaves both running on a dead session's tree.
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

    /// The lock is held while an op is in flight; the exit handler must skip
    /// rather than deadlock (the Windows job object is the backstop there).
    #[tokio::test]
    async fn kill_all_blocking_skips_a_held_lock_instead_of_hanging() {
        let kills = Arc::new(Kills::default());
        let reg = registry_with(1, &kills).await;

        let guard = reg.live.lock().await;
        reg.kill_all_blocking(); // must not deadlock
        drop(guard);

        assert_eq!(kills.detached.load(Ordering::SeqCst), 0);
    }
}
