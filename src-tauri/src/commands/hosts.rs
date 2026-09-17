//! The commands that work with no client (plan Phase 1 §7, Phase 7 §3-§4):
//! the app has to be able to name, test, provision, and switch servers before
//! it has one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, CmdResult};
use crate::hosts::{self, Host, HostsFile};
use crate::provision::{self, HostProvisioning, ProvisionStep};
use crate::state::AppState;
use crate::supervisor::{self, HostProbe};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Connected,
    /// Never established this launch.
    Failed,
    /// Was connected and the stream ended — the only state that can still
    /// promise sessions and terminals are running on the far side.
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub state: ConnectionState,
    /// `None` for the local server.
    pub host: Option<Host>,
    pub server_version: Option<u32>,
    /// `Some` for `Failed` and `Lost`.
    pub error: Option<AppError>,
}

fn app_config_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::internal(format!("could not resolve the app config directory: {e}")))
}

fn app_data_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| AppError::internal(format!("could not resolve the app data directory: {e}")))
}

/// Reads `hosts.json` directly — deciding which server to reach is
/// necessarily local, since there is no server yet to ask (plan Phase 1 §7).
#[tauri::command]
#[specta::specta]
pub fn list_hosts(app: AppHandle) -> CmdResult<HostsFile> {
    Ok(hosts::load(&app_config_dir(&app)?))
}

/// Writes the whole file. There is one window and one writer, so there is
/// nothing to merge (`BranchPrefixEditor`'s shape, plan Phase 1 §7).
#[tauri::command]
#[specta::specta]
pub fn save_hosts(app: AppHandle, file: HostsFile) -> CmdResult<()> {
    let dir = app_config_dir(&app)?;
    hosts::save(&dir, &file)
        .map_err(|e| AppError::internal(format!("could not write hosts.json: {e}")))
}

/// JetBrains Gateway's explicit "Test Connection" (plan Phase 1 §3): probes
/// the host without touching `AppState` or leaving a relay behind.
#[tauri::command]
#[specta::specta]
pub async fn test_host_connection(app: AppHandle, host: Host) -> CmdResult<HostProbe> {
    let data_dir = app_data_dir(&app)?;
    Ok(supervisor::probe_remote(&host, &data_dir).await)
}

/// The level read `useConnectionStatus` polls at mount and on focus.
#[tauri::command]
#[specta::specta]
pub fn connection_status(state: State<'_, AppState>) -> CmdResult<ConnectionStatus> {
    Ok(state.status())
}

/// Refuses a reconnect whose destination moved. The localStorage namespace is
/// baked into the window at creation (plan Phase 4 §2), so a changed host needs
/// a new window — attaching here would pair this window's tabs, pins and last
/// route with another server's data (plan Phase 8 §4).
fn host_change(connected: Option<&Host>, configured: Option<&Host>) -> Option<AppError> {
    if connected.map(|h| &h.destination) == configured.map(|h| &h.destination) {
        return None;
    }
    let name = match configured {
        Some(host) if !host.label.is_empty() => host.label.as_str(),
        Some(host) => host.destination.as_str(),
        None => "this machine",
    };
    Some(AppError::host_changed(name))
}

/// Reconnects in place after a drop, so a rebuilt server or a slept laptop
/// costs no relaunch (plan Phase 8 §4). Same host only: the connect sequence is
/// re-runnable, the profile is not.
#[tauri::command]
#[specta::specta]
pub async fn reconnect_server(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<ConnectionStatus> {
    let config_dir = app_config_dir(&app)?;
    let data_dir = app_data_dir(&app)?;
    let configured = hosts::active_host(&config_dir);
    if let Some(refusal) = host_change(state.host().as_ref(), configured.as_ref()) {
        return Err(refusal);
    }
    // Already live: a second connection would leave the first one's bridge
    // running against a client nothing reads.
    if state.status().state == ConnectionState::Connected {
        return Ok(state.status());
    }

    let outcome = match &configured {
        Some(host) => supervisor::connect_remote(host, &data_dir).await,
        None => supervisor::connect_or_spawn(&config_dir, &data_dir).await,
    };
    match outcome {
        supervisor::Outcome::Connected(client) => {
            // Swapped before the bridge is spawned: a connection that dies in
            // between then retires against the client the state holds, so it
            // still reports itself lost rather than sitting there as `Connected`.
            state.reconnected(client.clone());
            crate::attach_client(&app, client).await;
            Ok(state.status())
        }
        supervisor::Outcome::Blocked { error, .. } => Err(error),
    }
}

/// What the connect surface needs before it can offer an install: the host's
/// platform, whether this version is already there, and what else this client
/// family has put there (plan Phase 7 §4).
#[tauri::command]
#[specta::specta]
pub async fn inspect_host_provisioning(app: AppHandle, host: Host) -> CmdResult<HostProvisioning> {
    let data_dir = app_data_dir(&app)?;
    provision::readiness(&host, &data_dir).await
}

/// Installs this client's server on the host. A fresh `Channel` per invoke —
/// one is single-use, and reusing it silently drops the second call's steps.
#[tauri::command]
#[specta::specta]
pub async fn provision_host(
    app: AppHandle,
    host: Host,
    on_step: Channel<ProvisionStep>,
) -> CmdResult<()> {
    let data_dir = app_data_dir(&app)?;
    provision::provision(&host, &data_dir, |step| {
        let _ = on_step.send(step);
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(destination: &str, label: &str) -> Host {
        Host {
            destination: destination.into(),
            label: label.into(),
        }
    }

    #[test]
    fn a_reconnect_to_the_same_destination_is_allowed() {
        let connected = host("nexus", "Nexus");
        // The label is editable and free to change; the destination is the
        // profile namespace, so only it decides.
        let configured = host("nexus", "Nexus (linux)");
        assert!(host_change(Some(&connected), Some(&configured)).is_none());
        assert!(host_change(None, None).is_none());
    }

    #[test]
    fn a_changed_host_is_refused_and_names_relaunching() {
        let err = host_change(
            Some(&host("nexus", "Nexus")),
            Some(&host("office", "Office")),
        )
        .expect("a different destination is refused");
        assert_eq!(err.code, "host_changed");
        assert!(err.message.contains("Office"));
        assert!(err.remedy.unwrap().contains("Relaunch"));
    }

    #[test]
    fn switching_to_or_from_the_local_server_is_refused_too() {
        let err = host_change(Some(&host("nexus", "Nexus")), None)
            .expect("the local server is a different profile");
        assert_eq!(err.code, "host_changed");
        assert!(err.message.contains("this machine"));
        assert_eq!(
            host_change(None, Some(&host("nexus", "Nexus")))
                .expect("a remote host is a different profile")
                .code,
            "host_changed"
        );
    }
}
