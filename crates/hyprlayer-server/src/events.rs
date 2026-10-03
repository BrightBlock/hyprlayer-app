//! One subscription to the runtime's event broadcast, fanned out to every
//! connected client as an unsolicited control frame.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_core::{HyprlayerRuntime, RuntimeEvent};
use hyprlayer_models::ClientRef;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::transport::{encode_control, Control};

/// Kinds a released desktop cannot decode; a connection gets one only if it announced it.
pub const TARGETED_EVENTS: [&str; 3] = ["SessionRevised", "SessionReset", "SessionViewers"];
/// Reported by `announce_client` when this server targets them.
pub const TRANSCRIPT_SYNC: &str = "transcript_sync";

/// A display-only identity (ADR-0009): it attributes and targets, and grants nothing.
struct Announced {
    client: ClientRef,
    capabilities: HashSet<String>,
}

struct Outbound {
    sender: mpsc::Sender<Vec<u8>>,
    /// Events lost in the current episode of not draining.
    dropped: u64,
    announced: Option<Announced>,
    watching: HashSet<String>,
}

impl Outbound {
    fn receives(&self, kind: &str, session_id: &str) -> bool {
        self.watching.contains(session_id)
            && self
                .announced
                .as_ref()
                .is_some_and(|a| a.capabilities.contains(kind))
    }
}

/// The outbound queues of every connected client, keyed by connection id.
#[derive(Clone, Default)]
pub struct Clients {
    inner: Arc<Mutex<HashMap<u64, Outbound>>>,
}

impl Clients {
    pub fn register(&self, conn: u64, sender: mpsc::Sender<Vec<u8>>) {
        self.lock().insert(
            conn,
            Outbound {
                sender,
                dropped: 0,
                announced: None,
                watching: HashSet::new(),
            },
        );
    }

    /// Returns the sessions the connection watched, whose viewers may have changed.
    pub fn unregister(&self, conn: u64) -> Vec<String> {
        let Some(outbound) = self.lock().remove(&conn) else {
            return Vec::new();
        };
        if outbound.dropped > 0 {
            tracing::warn!(
                "conn {conn}: closed having dropped {} runtime event(s)",
                outbound.dropped
            );
        }
        outbound.watching.into_iter().collect()
    }

    /// Returns the sessions whose viewers changed: every one this connection watches.
    pub fn announce(&self, conn: u64, client: ClientRef, capabilities: Vec<String>) -> Vec<String> {
        let mut clients = self.lock();
        let Some(outbound) = clients.get_mut(&conn) else {
            return Vec::new();
        };
        outbound.announced = Some(Announced {
            client,
            capabilities: capabilities.into_iter().collect(),
        });
        outbound.watching.iter().cloned().collect()
    }

    pub fn client_ref(&self, conn: u64) -> Option<ClientRef> {
        let clients = self.lock();
        Some(clients.get(&conn)?.announced.as_ref()?.client.clone())
    }

    /// Replaces the watched set; returns the sessions whose viewers changed.
    pub fn watch(&self, conn: u64, sessions: HashSet<String>) -> Vec<String> {
        let mut clients = self.lock();
        let Some(outbound) = clients.get_mut(&conn) else {
            return Vec::new();
        };
        let previous = std::mem::replace(&mut outbound.watching, sessions);
        // Only announced watchers are viewers.
        if outbound.announced.is_none() {
            return Vec::new();
        }
        previous
            .symmetric_difference(&outbound.watching)
            .cloned()
            .collect()
    }

    /// Announced watchers, one per client id, sorted by device.
    pub fn viewers(&self, session_id: &str) -> Vec<ClientRef> {
        let mut viewers: Vec<ClientRef> = self
            .lock()
            .values()
            .filter(|o| o.watching.contains(session_id))
            .filter_map(|o| o.announced.as_ref().map(|a| a.client.clone()))
            .collect();
        viewers.sort_by(|a, b| (&a.device, &a.client_id).cmp(&(&b.device, &b.client_id)));
        let mut seen = HashSet::new();
        viewers.retain(|v| seen.insert(v.client_id.clone()));
        viewers
    }

    pub fn send_viewers(&self, sessions: &[String]) {
        for session_id in sessions {
            let viewers = self.viewers(session_id);
            self.send_event(RuntimeEvent::SessionViewers {
                session_id: session_id.clone(),
                viewers,
            });
        }
    }

    /// Non-blocking on purpose: a client that has stopped draining loses the
    /// event, the same outcome a lagged broadcast subscriber had in-process.
    /// A dropped targeted event is recovered by the client's next `watch_sessions`.
    pub fn send_event(&self, event: RuntimeEvent) {
        let kind = event_kind(&event);
        let target = audience(&event).map(|(kind, session_id)| (kind, session_id.to_string()));
        let bytes = match encode_control(&Control::Event { event }) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("{kind} runtime event is not serializable: {e}");
                return;
            }
        };
        let mut clients = self.lock();
        clients.retain(|conn, outbound| {
            if let Some((kind, session_id)) = &target {
                if !outbound.receives(kind, session_id) {
                    return true;
                }
            }
            match outbound.sender.try_send(bytes.clone()) {
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
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Outbound>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The kind and session of a targeted event; `None` goes to everyone.
fn audience(event: &RuntimeEvent) -> Option<(&'static str, &str)> {
    match event {
        RuntimeEvent::SessionRevised { session_id, .. } => Some(("SessionRevised", session_id)),
        RuntimeEvent::SessionReset { session_id, .. } => Some(("SessionReset", session_id)),
        RuntimeEvent::SessionViewers { session_id, .. } => Some(("SessionViewers", session_id)),
        _ => None,
    }
}

/// Subscribes once and forwards each event to its audience. `Lagged` is
/// handled here, where the broadcast receiver lives.
pub fn spawn_forwarder(runtime: &HyprlayerRuntime, clients: Clients) -> JoinHandle<()> {
    let mut rx = runtime.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => clients.send_event(event),
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
        E::ReviewChanged { .. } => "ReviewChanged",
        E::WorkstreamBranchChanged { .. } => "WorkstreamBranchChanged",
        E::SessionLifecycle { .. } => "SessionLifecycle",
        E::SessionRevised { .. } => "SessionRevised",
        E::SessionReset { .. } => "SessionReset",
        E::SessionViewers { .. } => "SessionViewers",
        E::ScriptRunChanged { .. } => "ScriptRunChanged",
        E::ProjectCreateProgress { .. } => "ProjectCreateProgress",
    }
}
