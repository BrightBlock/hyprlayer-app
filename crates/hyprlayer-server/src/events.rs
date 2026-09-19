//! One subscription to the runtime's event broadcast, fanned out to every
//! connected client as an unsolicited control frame.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_core::HyprlayerRuntime;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::transport::{encode_control, Control};

struct Outbound {
    sender: mpsc::Sender<Vec<u8>>,
    /// Events lost in the current episode of not draining.
    dropped: u64,
}

/// The outbound queues of every connected client, keyed by connection id.
#[derive(Clone, Default)]
pub struct Clients {
    inner: Arc<Mutex<HashMap<u64, Outbound>>>,
}

impl Clients {
    pub fn register(&self, conn: u64, sender: mpsc::Sender<Vec<u8>>) {
        self.lock().insert(conn, Outbound { sender, dropped: 0 });
    }

    pub fn unregister(&self, conn: u64) {
        if let Some(outbound) = self.lock().remove(&conn) {
            if outbound.dropped > 0 {
                tracing::warn!(
                    "conn {conn}: closed having dropped {} runtime event(s)",
                    outbound.dropped
                );
            }
        }
    }

    /// Non-blocking on purpose: a client that has stopped draining loses the
    /// event, the same outcome a lagged broadcast subscriber had in-process.
    fn broadcast(&self, bytes: &[u8], kind: &str) {
        let mut clients = self.lock();
        clients.retain(|conn, outbound| match outbound.sender.try_send(bytes.to_vec()) {
            Ok(()) => {
                if outbound.dropped > 0 {
                    tracing::info!(
                        "conn {conn}: draining again after dropping {} runtime event(s)",
                        outbound.dropped
                    );
                    outbound.dropped = 0;
                }
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Err(mpsc::error::TrySendError::Full(_)) => {
                // The queue is shared with responses and stream frames, so a busy
                // terminal can fill it for a client that is still reading.
                if outbound.dropped == 0 {
                    tracing::warn!("conn {conn}: outbound queue full; dropping runtime events, starting with {kind}");
                }
                outbound.dropped += 1;
                true
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Outbound>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Subscribes once and forwards each event to every client. `Lagged` is
/// handled here, where the broadcast receiver lives.
pub fn spawn_forwarder(runtime: &HyprlayerRuntime, clients: Clients) -> JoinHandle<()> {
    let mut rx = runtime.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let kind = event_kind(&event);
                    match encode_control(&Control::Event { event }) {
                        Ok(bytes) => clients.broadcast(&bytes, kind),
                        Err(e) => tracing::warn!("{kind} runtime event is not serializable: {e}"),
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("event forwarder fell {n} runtime events behind");
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
        }
        tracing::info!("runtime event stream closed; no more events reach clients");
    })
}

fn event_kind(event: &hyprlayer_core::models::RuntimeEvent) -> &'static str {
    use hyprlayer_core::models::RuntimeEvent as E;
    match event {
        E::ConfigChanged => "ConfigChanged",
        E::BinariesChanged => "BinariesChanged",
        E::ThoughtsIndexUpdated { .. } => "ThoughtsIndexUpdated",
        E::ProjectsChanged => "ProjectsChanged",
        E::PrStatusChanged { .. } => "PrStatusChanged",
        E::WorktreeChanged { .. } => "WorktreeChanged",
        E::WorkstreamBranchChanged { .. } => "WorkstreamBranchChanged",
        E::SessionLifecycle { .. } => "SessionLifecycle",
        E::ScriptRunChanged { .. } => "ScriptRunChanged",
        E::ProjectCreateProgress { .. } => "ProjectCreateProgress",
    }
}
