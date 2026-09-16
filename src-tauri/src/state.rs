use std::sync::Arc;

use hyprlayer_core::CoreError;

use crate::client::RpcClient;
use crate::error::AppError;
use crate::hosts::Host;

pub struct AppState {
    pub client: Arc<RpcClient>,
    /// `Some` when the active server (per `hosts.json`) is reached over SSH;
    /// `None` for the local profile (ADR-0009, plan Phase 5).
    pub remote_host: Option<Host>,
}

impl AppState {
    pub fn new(client: Arc<RpcClient>, remote_host: Option<Host>) -> Self {
        Self {
            client,
            remote_host,
        }
    }

    /// `Some` when connected to a remote host, for a command whose action
    /// only makes sense against the machine the desktop itself is running on
    /// (editor launching, revealing a file, a client-supplied import path).
    pub fn remote_unsupported(&self, action: &str) -> Option<AppError> {
        self.remote_host.as_ref().map(|host| {
            AppError::from(CoreError::RemoteUnsupported {
                action: action.to_string(),
                host: host.label.clone(),
            })
        })
    }
}
