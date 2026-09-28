use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use hyprlayer_models::RuntimeEvent;
use tokio::sync::Notify;

use crate::runtime::shutdown::TrackedTask;
use crate::runtime::HyprlayerRuntime;

/// Sessions awaiting a generated title, worked one at a time since each is a
/// model call. An id is tried once per server lifetime, plus one retry after a
/// failed call.
#[derive(Default)]
pub(crate) struct SessionTitler {
    state: Mutex<TitleQueue>,
    wake: Notify,
}

#[derive(Default)]
struct TitleQueue {
    pending: VecDeque<String>,
    tried: HashSet<String>,
    retried: HashSet<String>,
}

impl SessionTitler {
    /// A just-started session jumps any backfill queued ahead of it.
    pub(crate) fn enqueue(&self, session_id: &str, urgent: bool) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.tried.insert(session_id.to_string()) {
            return;
        }
        if urgent {
            state.pending.push_front(session_id.to_string());
        } else {
            state.pending.push_back(session_id.to_string());
        }
        drop(state);
        self.wake.notify_one();
    }

    /// Behind the rest of the queue, so a slow provider isn't hit again at once.
    fn retry(&self, session_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.retried.insert(session_id.to_string()) {
            state.pending.push_back(session_id.to_string());
        }
    }

    fn pop(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .pop_front()
    }
}

/// Holds a `Weak` between items so an idle worker never keeps the runtime alive.
pub(crate) fn spawn_worker(
    titler: Arc<SessionTitler>,
    runtime: Weak<HyprlayerRuntime>,
) -> TrackedTask {
    TrackedTask::spawn(move |stop| async move {
        loop {
            while let Some(session_id) = titler.pop() {
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                tokio::select! {
                    failed = title_session(&runtime, &session_id) => {
                        if failed {
                            titler.retry(&session_id);
                        }
                    }
                    _ = stop.notified() => return,
                }
            }
            tokio::select! {
                _ = titler.wake.notified() => {}
                _ = stop.notified() => return,
            }
        }
    })
}

/// `true` when the model call failed and is worth another try.
async fn title_session(runtime: &HyprlayerRuntime, session_id: &str) -> bool {
    let db = runtime.db.clone();
    let id = session_id.to_string();
    let source = match tokio::task::spawn_blocking(move || db.session_title_source(&id)).await {
        Ok(Ok(Some(source))) => source,
        Ok(Ok(None)) => return false,
        Ok(Err(e)) => {
            tracing::warn!("session {session_id}: reading title source: {e}");
            return false;
        }
        Err(e) => {
            tracing::warn!("session {session_id}: reading title source: {e}");
            return false;
        }
    };
    let title = match runtime.providers().suggest_title(&source.input).await {
        Ok(Some(title)) => title,
        Ok(None) => return false,
        Err(e) => {
            tracing::info!("session {session_id}: no title generated: {e}");
            return true;
        }
    };

    let db = runtime.db.clone();
    let id = session_id.to_string();
    let written = tokio::task::spawn_blocking(move || {
        if !db.set_session_title_if_unset(&id, &title)? {
            return Ok(None);
        }
        db.get_session(&id)
    })
    .await;
    match written {
        // Clients refetch session lists on any lifecycle event; a new event
        // kind would break older clients' frame decoding.
        Ok(Ok(Some(session))) => {
            let _ = runtime.events.send(RuntimeEvent::SessionLifecycle {
                session_id: session.id,
                workstream_id: source.workstream_id,
                state: session.state,
            });
        }
        Ok(Ok(None)) => {}
        Ok(Err(e)) => tracing::warn!("session {session_id}: storing title: {e}"),
        Err(e) => tracing::warn!("session {session_id}: storing title: {e}"),
    }
    false
}
