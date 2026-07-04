//! Live-session registry: the in-memory map of running drivers. Each entry is an
//! `Arc<dyn AgentDriver>`, so Claude and OpenCode sessions share one map without
//! the registry knowing the harness. Session rows and event logs live in SQLite;
//! this holds only the handles needed to talk to a live session.
//!
//! There is no concurrency cap — the number of simultaneous agent sessions is
//! bounded only by the host's resources. Sessions across all projects share this
//! one map, keyed by session id.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::PermissionMode;
use tauri::async_runtime::JoinHandle;
use tokio::sync::{oneshot, Mutex};

/// A running forwarder task plus the trigger to stop it COOPERATIVELY. Aborting a
/// forwarder mid-write only detaches the task — an in-flight `spawn_blocking` DB
/// append keeps running and could land after a rewind's cut, re-adding a dropped
/// row. So a live rewind signals `shutdown` and awaits `join` (see
/// [`SessionRegistry::stop_forwarder`]), draining any in-flight write first.
pub struct ForwarderHandle {
    pub join: JoinHandle<()>,
    pub shutdown: oneshot::Sender<()>,
}

/// One live session's handle plus the `--permission-mode` its process was
/// actually launched with.
struct LiveSession {
    driver: Arc<dyn AgentDriver>,
    /// The mode the running claude process was launched with — `None` for
    /// OpenCode (which has no permission modes). claude binds its mode at launch
    /// and only changes it by relaunching, so this stays accurate for the life of
    /// the process. `resume_session` consults it to tell a stale re-attach (the
    /// UI wants a mode the live process isn't in) from a genuine no-change resume.
    permission_mode: Option<PermissionMode>,
    /// The forwarder task draining this session's event stream into the DB + UI
    /// channel. Held so a live rewind can stop it and hand a fresh forwarder to a
    /// new channel on the SAME process — the process stays alive, so the model's
    /// rewound memory survives (a kill + `--resume` would reload it from disk).
    forwarder: Option<ForwarderHandle>,
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
            },
        );
    }

    /// Record (or replace) the forwarder task driving this session, so a later
    /// live rewind can [`stop_forwarder`](Self::stop_forwarder) it. Replacing an
    /// existing handle aborts the old one — the only callers that replace an
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

    /// Stop this session's forwarder COOPERATIVELY, without touching the driver —
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
        self.live.lock().await.get(id).and_then(|s| s.permission_mode)
    }

    pub async fn remove(&self, id: &str) {
        self.live.lock().await.remove(id);
        // Drop the op lock too — the session is gone, so nothing can still be
        // serializing against it (any op holding a clone keeps its own Arc alive).
        self.op_locks.lock().await.remove(id);
    }

    /// Snapshot the ids of all currently-live sessions.
    pub async fn live_ids(&self) -> Vec<String> {
        self.live.lock().await.keys().cloned().collect()
    }
}
