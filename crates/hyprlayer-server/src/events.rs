//! One subscription to the runtime's event broadcast, fanned out to every
//! connected client as an unsolicited control frame.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_core::{HyprlayerRuntime, RuntimeEvent};
use hyprlayer_models::ClientRef;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::outbound::{EventPush, Outbound};

/// Kinds a released desktop cannot decode; a connection gets one only if it announced it.
pub const TARGETED_EVENTS: [&str; 9] = [
    "SessionRevised",
    "SessionReset",
    "SessionViewers",
    "AccountChanged",
    "GoalChanged",
    "PtyAgentChanged",
    "UiRequested",
    // Session rows, not runtime events.
    "permissionResolved",
    "questionAnswered",
];
/// Reported by `announce_client` when this server targets them.
pub const TRANSCRIPT_SYNC: &str = "transcript_sync";

/// What one connection decodes, read as each row goes out: an announce can follow a stream's open.
#[derive(Clone, Default)]
pub struct Decoder {
    clients: Clients,
    conn: Option<u64>,
}

impl Decoder {
    pub(crate) fn new(clients: Clients, conn: Option<u64>) -> Self {
        Self { clients, conn }
    }

    /// A targeted kind this connection never announced.
    pub fn withholds(&self, kind: &str) -> bool {
        TARGETED_EVENTS.contains(&kind) && !self.conn.is_some_and(|c| self.clients.decodes(c, kind))
    }
}

/// A display-only identity (ADR-0009): it attributes and targets, and grants nothing.
struct Announced {
    client: ClientRef,
    capabilities: HashSet<String>,
}

struct Conn {
    outbound: Outbound,
    /// Events lost in the current episode of not draining.
    dropped: u64,
    announced: Option<Announced>,
    watching: HashSet<String>,
    /// The workstream whose worktree changes this connection hears.
    worktree: Option<String>,
    /// The token an agent connection announced; such a connection hears no events.
    agent: Option<String>,
}

impl Conn {
    fn receives(&self, audience: &Audience) -> bool {
        if self.agent.is_some() {
            return false;
        }
        let announced = |kind: &str| {
            self.announced
                .as_ref()
                .is_some_and(|a| a.capabilities.contains(kind))
        };
        match audience {
            Audience::Everyone => true,
            Audience::Watchers(kind, session_id) => {
                self.watching.contains(session_id) && announced(kind)
            }
            Audience::Announced(kind) => announced(kind),
            Audience::Worktree(id) => self.worktree.as_deref() == Some(id),
        }
    }
}

enum Audience {
    Everyone,
    /// Announced connections watching this session.
    Watchers(&'static str, String),
    /// Announced connections, whatever they watch.
    Announced(&'static str),
    /// Connections watching this workstream's worktree, announced or not: every desktop decodes it.
    Worktree(String),
}

/// The outbound queues of every connected client, keyed by connection id.
#[derive(Clone, Default)]
pub struct Clients {
    inner: Arc<Mutex<HashMap<u64, Conn>>>,
    unattended: Arc<Notify>,
}

impl Clients {
    pub fn register(&self, conn: u64, outbound: Outbound) {
        self.lock().insert(
            conn,
            Conn {
                outbound,
                dropped: 0,
                announced: None,
                watching: HashSet::new(),
                worktree: None,
                agent: None,
            },
        );
    }

    /// Returns the sessions the connection watched, whose viewers may have changed.
    pub fn unregister(&self, conn: u64) -> Vec<String> {
        let mut clients = self.lock();
        let Some(outbound) = clients.remove(&conn) else {
            return Vec::new();
        };
        self.note_departure(&clients, outbound.announced.is_some());
        drop(clients);
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

    pub fn tag_agent(&self, conn: u64, token: String) {
        if let Some(outbound) = self.lock().get_mut(&conn) {
            outbound.agent = Some(token);
        }
    }

    pub fn agent_token(&self, conn: u64) -> Option<String> {
        self.lock().get(&conn)?.agent.clone()
    }

    pub fn decodes(&self, conn: u64, kind: &str) -> bool {
        self.lock()
            .get(&conn)
            .and_then(|c| c.announced.as_ref())
            .is_some_and(|a| a.capabilities.contains(kind))
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

    pub fn watch_worktree(&self, conn: u64, workstream_id: Option<String>) {
        if let Some(outbound) = self.lock().get_mut(&conn) {
            outbound.worktree = workstream_id;
        }
    }

    pub fn watched_worktrees(&self) -> HashSet<String> {
        self.lock()
            .values()
            .filter_map(|o| o.worktree.clone())
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
    /// event, and a client that reads `EventsMissed` is told so in order.
    pub fn send_event(&self, event: RuntimeEvent) {
        let kind = event_kind(&event);
        let audience = audience(&event);
        // Serialized once, and only if some queue has room for it.
        let event = &event;
        let mut json: Option<Option<String>> = None;
        let mut clients = self.lock();
        let mut lost_announced = false;
        clients.retain(|conn, client| {
            if !client.receives(&audience) {
                return true;
            }
            let slot = &mut json;
            let pushed = client.outbound.push_event(move || {
                cached(slot, || {
                    serde_json::to_string(event)
                        .inspect_err(|e| {
                            tracing::warn!("{kind} runtime event is not serializable: {e}")
                        })
                        .ok()
                })
            });
            match pushed {
                EventPush::Queued => {
                    if client.dropped > 0 {
                        tracing::info!(
                            "conn {conn}: draining again after dropping {} runtime event(s)",
                            client.dropped
                        );
                        client.dropped = 0;
                    }
                    true
                }
                EventPush::Closed => {
                    lost_announced |= client.announced.is_some();
                    false
                }
                EventPush::Dropped => {
                    if client.dropped == 0 {
                        tracing::warn!("conn {conn}: event queue full; dropping runtime events, starting with {kind}");
                    }
                    client.dropped += 1;
                    true
                }
            }
        });
        self.note_departure(&clients, lost_announced);
    }

    /// The forwarder fell behind the runtime. A lag carries only a count, so
    /// every connection that hears events is told: one may have needed them.
    pub fn missed(&self, count: u64) {
        for client in self.lock().values().filter(|c| c.agent.is_none()) {
            client.outbound.missed(count);
        }
    }

    /// Resolves once the last announced client has gone; a client that never
    /// announced (`status`, a version probe) neither triggers nor holds it.
    pub async fn wait_unattended(&self) {
        self.unattended.notified().await;
    }

    pub fn is_unattended(&self) -> bool {
        no_announced(&self.lock())
    }

    fn note_departure(&self, clients: &HashMap<u64, Conn>, was_announced: bool) {
        if was_announced && no_announced(clients) {
            self.unattended.notify_one();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Conn>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn cached(
    slot: &mut Option<Option<String>>,
    encode: impl FnOnce() -> Option<String>,
) -> Option<&str> {
    slot.get_or_insert_with(encode).as_deref()
}

fn no_announced(clients: &HashMap<u64, Conn>) -> bool {
    clients.values().all(|o| o.announced.is_none())
}

fn audience(event: &RuntimeEvent) -> Audience {
    match event {
        RuntimeEvent::SessionRevised { session_id, .. } => {
            Audience::Watchers("SessionRevised", session_id.clone())
        }
        RuntimeEvent::SessionReset { session_id, .. } => {
            Audience::Watchers("SessionReset", session_id.clone())
        }
        RuntimeEvent::SessionViewers { session_id, .. } => {
            Audience::Watchers("SessionViewers", session_id.clone())
        }
        RuntimeEvent::AccountChanged => Audience::Announced("AccountChanged"),
        RuntimeEvent::GoalChanged { .. } => Audience::Announced("GoalChanged"),
        RuntimeEvent::PtyAgentChanged { .. } => Audience::Announced("PtyAgentChanged"),
        RuntimeEvent::UiRequested { .. } => Audience::Announced("UiRequested"),
        RuntimeEvent::WorktreeChanged { workstream_id } => {
            Audience::Worktree(workstream_id.clone())
        }
        _ => Audience::Everyone,
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
                    clients.missed(n);
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
        E::DocStateChanged { .. } => "DocStateChanged",
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
        E::AccountChanged => "AccountChanged",
        E::GoalChanged { .. } => "GoalChanged",
        E::PtyAgentChanged { .. } => "PtyAgentChanged",
        E::UiRequested { .. } => "UiRequested",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hyprlayer_proto::Features;

    use super::*;

    fn connect(clients: &Clients, conn: u64, announce: bool) -> Outbound {
        let outbound = Outbound::new(Features::ALL);
        clients.register(conn, outbound.clone());
        if announce {
            let client = ClientRef {
                client_id: format!("client-{conn}"),
                device: "test".into(),
            };
            clients.announce(conn, client, Vec::new());
        }
        outbound
    }

    async fn unattended(clients: &Clients) -> bool {
        tokio::time::timeout(Duration::from_millis(50), clients.wait_unattended())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn only_the_last_announced_client_leaving_is_unattended() {
        let clients = Clients::default();
        let _a = connect(&clients, 1, true);
        let _b = connect(&clients, 2, true);
        let _probe = connect(&clients, 3, false);
        assert!(!clients.is_unattended());

        clients.unregister(3);
        assert!(!unattended(&clients).await, "a probe never announced");
        clients.unregister(1);
        assert!(
            !unattended(&clients).await,
            "another client is still announced"
        );
        clients.unregister(2);
        assert!(unattended(&clients).await);
        assert!(clients.is_unattended());
    }

    #[tokio::test]
    async fn an_announced_client_pruned_by_a_send_is_unattended() {
        let clients = Clients::default();
        connect(&clients, 1, true).close();
        let _probe = connect(&clients, 2, false);

        clients.send_event(RuntimeEvent::BinariesChanged);
        assert!(unattended(&clients).await);
        assert!(
            clients.is_unattended(),
            "an unannounced probe does not count"
        );
    }
}
