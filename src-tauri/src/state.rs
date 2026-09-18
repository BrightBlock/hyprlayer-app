use std::sync::{Arc, RwLock};

use hyprlayer_core::CoreError;
use hyprlayer_proto::WireError;

use crate::client::RpcClient;
use crate::commands::hosts::{ConnectionState, ConnectionStatus};
use crate::error::{AppError, CmdResult};
use crate::hosts::Target;

struct Connection {
    client: Option<Arc<RpcClient>>,
    /// The server `hosts.json` named when this connection was opened
    /// (ADR-0009, plan Phase 5).
    target: Target,
    failure: Option<AppError>,
    /// Set only by `mark_lost()`. Distinguishes "never connected this
    /// launch" (`Failed`) from "was connected and the stream ended"
    /// (`Lost`) — both leave `client: None`, but only `Lost` can promise
    /// sessions and terminals are still running on the far side.
    lost: bool,
    outdated: bool,
}

pub struct AppState {
    connection: RwLock<Connection>,
}

impl AppState {
    pub fn new(client: Arc<RpcClient>, target: Target, outdated: bool) -> Self {
        Self {
            connection: RwLock::new(Connection {
                client: Some(client),
                target,
                failure: None,
                lost: false,
                outdated,
            }),
        }
    }

    /// A connection that never established this launch (`setup()`'s
    /// `Outcome::Blocked` path, plan Phase 1 §5): no client, a named reason.
    pub fn failed(target: Target, error: AppError) -> Self {
        Self {
            connection: RwLock::new(Connection {
                client: None,
                target,
                failure: Some(error),
                lost: false,
                outdated: false,
            }),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Connection> {
        self.connection
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The live client, or a clone of the stored failure — so every command
    /// answers with the *actual* named reason rather than a generic one.
    pub fn client(&self) -> CmdResult<Arc<RpcClient>> {
        let guard = self.read();
        match &guard.client {
            Some(client) => Ok(client.clone()),
            None => Err(guard
                .failure
                .clone()
                .unwrap_or_else(|| AppError::from(WireError::disconnected()))),
        }
    }

    /// Read by `reconnect_server` to refuse a server that changed under it
    /// (plan Phase 8 §4).
    pub fn target(&self) -> Target {
        self.read().target.clone()
    }

    /// `Some` when connected to a remote host, for a command whose action
    /// only makes sense against the machine the desktop itself is running on
    /// (editor launching, revealing a file, a client-supplied import path).
    pub fn remote_unsupported(&self, action: &str) -> Option<AppError> {
        match &self.read().target {
            Target::Remote(host) => Some(AppError::from(CoreError::RemoteUnsupported {
                action: action.to_string(),
                host: host.label.clone(),
            })),
            Target::Own | Target::LocalProfile(_) => None,
        }
    }

    /// The level read: what the frontend polls to know whether it can trust
    /// the client it has.
    pub fn status(&self) -> ConnectionStatus {
        let guard = self.read();
        let state = if guard.client.is_some() {
            ConnectionState::Connected
        } else if guard.lost {
            ConnectionState::Lost
        } else {
            ConnectionState::Failed
        };
        let (host, local_server_name) = match &guard.target {
            Target::Own => (None, None),
            Target::LocalProfile(_) => (None, Some(guard.target.name())),
            Target::Remote(host) => (Some(host.clone()), None),
        };
        ConnectionStatus {
            state,
            host,
            local_server_name,
            server_version: guard.client.as_ref().map(|c| c.server_version()),
            server_outdated: guard.client.is_some() && guard.outdated,
            error: guard.failure.clone(),
        }
    }

    /// The stream ended: drop the client and record `server_disconnected`, so
    /// `status()` reports `Lost` — the state that can still promise sessions
    /// are running on the far side.
    ///
    /// Only the bridge for the *live* client may say so, which is why it hands
    /// its own back: a reconnect's predecessor retires after the swap (plan
    /// Phase 8 §2), and letting it mark the new connection lost would strand a
    /// working client behind a `Lost` status. `false` means it was stale.
    pub fn mark_lost(&self, client: &Arc<RpcClient>) -> bool {
        let mut guard = self
            .connection
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !guard
            .client
            .as_ref()
            .is_some_and(|live| Arc::ptr_eq(live, client))
        {
            return false;
        }
        guard.client = None;
        guard.failure = Some(AppError::from(WireError::disconnected()));
        guard.lost = true;
        true
    }

    /// A fresh client for the same host (plan Phase 8 §1). Clearing `failure`
    /// and `lost` is what puts `status()` back on `Connected`.
    pub fn reconnected(&self, client: Arc<RpcClient>, outdated: bool) {
        let mut guard = self
            .connection
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.client = Some(client);
        guard.outdated = outdated;
        guard.failure = None;
        guard.lost = false;
    }

    /// The handshake passed but the connection turned out unusable before it
    /// was ever handed to a command — `setup()`'s second exit, downgraded
    /// (plan Phase 1 §5). `Failed`, not `Lost`: this build never had a
    /// working connection to promise anything from.
    pub fn mark_failed(&self, error: AppError) {
        let mut guard = self
            .connection
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.client = None;
        guard.failure = Some(error);
        guard.lost = false;
    }
}

#[cfg(test)]
mod tests {
    use hyprlayer_proto::PROTOCOL_VERSION;
    use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};

    use super::*;

    /// A client over an in-memory duplex whose peer answers the hello and then
    /// holds the connection open — enough to reach `Connected`, which needs a
    /// real `Arc<RpcClient>` and nothing else.
    async fn connected_client() -> Arc<RpcClient> {
        let (ours, theirs) = tokio::io::duplex(8 * 1024);
        tokio::spawn(async move {
            let (rd, wr) = tokio::io::split(theirs);
            let mut reader = FrameReader::new(rd);
            let mut writer = FrameWriter::new(wr);
            let _ = reader.next_control().await;
            let _ = writer
                .write_control(&Control::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    build: None,
                })
                .await;
            // The connection must outlive the handshake; reading to EOF parks
            // here until the client half is dropped.
            while matches!(reader.next_control().await, Ok(Some(_))) {}
        });
        RpcClient::connect(ours).await.expect("the peer says hello")
    }

    #[test]
    fn client_on_a_failed_connection_returns_the_stored_errors_code() {
        let state = AppState::failed(Target::Own, AppError::server_unavailable("could not start"));
        let Err(err) = state.client() else {
            panic!("no client is connected");
        };
        assert_eq!(err.code, "server_unavailable");
    }

    #[test]
    fn status_on_a_failed_connection_is_failed_not_lost() {
        let state = AppState::failed(Target::Own, AppError::server_unavailable("could not start"));
        assert_eq!(state.status().state, ConnectionState::Failed);
    }

    #[tokio::test]
    async fn status_after_mark_lost_is_lost() {
        let client = connected_client().await;
        let state = AppState::new(client.clone(), Target::Own, false);
        assert!(state.mark_lost(&client));
        assert_eq!(state.status().state, ConnectionState::Lost);
        let Err(err) = state.client() else {
            panic!("no client is connected");
        };
        assert_eq!(err.code, "server_disconnected");
    }

    #[tokio::test]
    async fn reconnected_clears_the_failure_and_the_lost_flag() {
        let old = connected_client().await;
        let state = AppState::new(old.clone(), Target::Own, false);
        assert!(state.mark_lost(&old));

        state.reconnected(connected_client().await, false);

        let status = state.status();
        assert_eq!(status.state, ConnectionState::Connected);
        assert!(status.error.is_none());
        let guard = state.read();
        assert!(guard.failure.is_none());
        assert!(!guard.lost);
    }

    #[tokio::test]
    async fn a_retired_clients_bridge_cannot_mark_the_new_connection_lost() {
        let old = connected_client().await;
        let state = AppState::new(old.clone(), Target::Own, false);
        state.reconnected(connected_client().await, false);

        assert!(!state.mark_lost(&old));
        assert_eq!(state.status().state, ConnectionState::Connected);
    }
}
