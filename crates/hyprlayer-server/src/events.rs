//! One subscription to the runtime's event broadcast, fanned out to every
//! connected client as an unsolicited control frame.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_core::HyprlayerRuntime;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::transport::{encode_control, Control};

/// The outbound queues of every connected client, keyed by connection id.
#[derive(Clone, Default)]
pub struct Clients {
    inner: Arc<Mutex<HashMap<u64, mpsc::Sender<Vec<u8>>>>>,
    next_id: Arc<AtomicU64>,
}

impl Clients {
    pub fn register(&self, outbound: mpsc::Sender<Vec<u8>>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.lock().insert(id, outbound);
        id
    }

    pub fn unregister(&self, id: u64) {
        self.lock().remove(&id);
    }

    /// Non-blocking on purpose: a client that has stopped draining loses the
    /// event, the same outcome a lagged broadcast subscriber had in-process.
    fn broadcast(&self, bytes: &[u8]) {
        let mut clients = self.lock();
        clients.retain(|id, outbound| match outbound.try_send(bytes.to_vec()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("client {id} is not draining; dropping a runtime event");
                true
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, mpsc::Sender<Vec<u8>>>> {
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
                Ok(event) => match encode_control(&Control::Event { event }) {
                    Ok(bytes) => clients.broadcast(&bytes),
                    Err(e) => tracing::warn!("runtime event is not serializable: {e}"),
                },
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("event forwarder fell {n} runtime events behind");
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
        }
    })
}
