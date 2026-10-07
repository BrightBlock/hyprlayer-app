//! The commands that work with no client (plan Phase 1 §7, Phase 7 §3-§4):
//! the app has to be able to name, test, provision, and switch servers before
//! it has one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, CmdResult};
use crate::hosts::{self, Host, HostsFile, Target};
use crate::provision::{self, HostProvisioning, ProvisionStep};
use crate::ssh::{Ssh, SshPrompt};
use crate::state::AppState;
use crate::supervisor::{self, HostActivity, HostProbe, LocalServer};

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
    /// Set when attached to another profile's server on this machine.
    #[specta(optional)]
    pub local_server_name: Option<String>,
    pub server_version: Option<u32>,
    /// The server's release version; `None` when unconnected or from a server
    /// up to 0.8.2, which does not report one.
    pub server_release: Option<String>,
    /// Connected to an older server than this app ships; `restart_server`
    /// (local) or `update_host_server` (remote) replaces it.
    pub server_outdated: bool,
    /// The server targets transcript-sync events at this client.
    pub transcript_sync: bool,
    /// `Some` for `Failed` and `Lost`.
    pub error: Option<AppError>,
}

fn app_config_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::internal(format!("could not resolve the app config directory: {e}")))
}

pub(crate) fn app_data_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| AppError::internal(format!("could not resolve the app data directory: {e}")))
}

pub(crate) fn app_log_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path()
        .app_log_dir()
        .map_err(|e| AppError::internal(format!("could not resolve the app log directory: {e}")))
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
        .map_err(|e| AppError::internal(format!("could not write hosts.json: {e}")))?;
    log::info!(
        "saved {} host(s); active: {}",
        file.hosts.len(),
        file.active.as_deref().unwrap_or("this machine")
    );
    Ok(())
}

/// JetBrains Gateway's explicit "Test Connection" (plan Phase 1 §3): probes
/// the host without touching `AppState` or leaving a relay behind.
#[tauri::command]
#[specta::specta]
pub async fn test_host_connection(
    app: AppHandle,
    ssh: State<'_, Ssh>,
    host: Host,
) -> CmdResult<HostProbe> {
    let data_dir = app_data_dir(&app)?;
    let probe = supervisor::probe_remote(&host, &data_dir, &ssh).await;
    match &probe.error {
        None => log::info!("connection test to {} passed", host.destination),
        Some(e) => log::warn!(
            "connection test to {} failed: {}: {}",
            host.destination,
            e.code,
            e.message
        ),
    }
    Ok(probe)
}

/// The level read `useConnectionStatus` polls at mount and on focus.
#[tauri::command]
#[specta::specta]
pub fn connection_status(state: State<'_, AppState>) -> CmdResult<ConnectionStatus> {
    Ok(state.status())
}

/// Refuses a reconnect whose server moved. The localStorage namespace is
/// baked into the window at creation (plan Phase 4 §2), so a changed server
/// needs a new window — attaching here would pair this window's tabs, pins and
/// last route with another server's data (plan Phase 8 §4).
fn server_change(connected: &Target, configured: &Target) -> Option<AppError> {
    (connected.profile_id() != configured.profile_id())
        .then(|| AppError::host_changed(&configured.name()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_local_servers(app: AppHandle) -> CmdResult<Vec<LocalServer>> {
    let data_dir = app_data_dir(&app)?;
    Ok(supervisor::list_local_servers(&data_dir).await)
}

/// Reconnects in place after a drop, so a rebuilt server or a slept laptop
/// costs no relaunch (plan Phase 8 §4). Same host only: the connect sequence is
/// re-runnable, the profile is not.
#[tauri::command]
#[specta::specta]
pub async fn reconnect_server(
    app: AppHandle,
    state: State<'_, AppState>,
    ssh: State<'_, Ssh>,
) -> CmdResult<ConnectionStatus> {
    let config_dir = app_config_dir(&app)?;
    let data_dir = app_data_dir(&app)?;
    let log_dir = app_log_dir(&app)?;
    let configured = hosts::target(&config_dir);
    if let Some(refusal) = server_change(&state.target(), &configured) {
        log::warn!(
            "not reconnecting: the configured server changed from {} to {}",
            state.target().name(),
            configured.name()
        );
        return Err(refusal);
    }
    // Already live: a second connection would leave the first one's bridge
    // running against a client nothing reads.
    if state.status().state == ConnectionState::Connected {
        return Ok(state.status());
    }

    match supervisor::connect(&configured, &config_dir, &data_dir, &log_dir, &ssh).await {
        supervisor::Outcome::Connected { client, outdated } => {
            // Swapped before the bridge is spawned: a connection that dies in
            // between then retires against the client the state holds, so it
            // still reports itself lost rather than sitting there as `Connected`.
            state.reconnected(client.clone(), outdated);
            log::info!("reconnected to {}", configured.name());
            crate::attach_client(&app, client, &configured).await;
            Ok(state.status())
        }
        supervisor::Outcome::Blocked { error, .. } => {
            state.refailed(error.clone());
            Err(error)
        }
    }
}

/// Stops the connected server so the reconnect that follows starts this app's
/// build in its place: the dropped stream raises `server-disconnected`, the
/// frontend's reconnect loop dials again, and with nothing listening the local
/// supervisor or the remote relay spawns the binary it would have run. Ends
/// every session and terminal on that server.
#[tauri::command]
#[specta::specta]
pub async fn restart_server(state: State<'_, AppState>) -> CmdResult<()> {
    log::warn!(
        "restarting the server on {} at the user's request; its sessions and terminals end",
        state.target().name()
    );
    state
        .client()?
        .request_shutdown()
        .await
        .map_err(AppError::from)
}

/// Reads what runs on a host refused as too old; the auto-updater waits for it
/// to idle. Nobody pressed anything, so it never asks.
#[tauri::command]
#[specta::specta]
pub async fn inspect_outdated_host(
    app: AppHandle,
    ssh: State<'_, Ssh>,
    host: Host,
) -> CmdResult<HostActivity> {
    let data_dir = app_data_dir(&app)?;
    supervisor::outdated_host_activity(&host, &data_dir, &ssh.batch()).await
}

/// What the connect surface needs before it can offer an install: the host's
/// platform, whether this version is already there, and what else this client
/// family has put there (plan Phase 7 §4). A query, so it never asks.
#[tauri::command]
#[specta::specta]
pub async fn inspect_host_provisioning(
    app: AppHandle,
    ssh: State<'_, Ssh>,
    host: Host,
) -> CmdResult<HostProvisioning> {
    let data_dir = app_data_dir(&app)?;
    provision::readiness(&host, &data_dir, &ssh.batch()).await
}

/// Installs this client's server on the host. A fresh `Channel` per invoke —
/// one is single-use, and reusing it silently drops the second call's steps.
#[tauri::command]
#[specta::specta]
pub async fn provision_host(
    app: AppHandle,
    ssh: State<'_, Ssh>,
    host: Host,
    on_step: Channel<ProvisionStep>,
) -> CmdResult<()> {
    let data_dir = app_data_dir(&app)?;
    provision::provision(
        &host,
        &data_dir,
        |step| {
            let _ = on_step.send(step);
        },
        &ssh,
    )
    .await
}

/// Moves a host onto this client's server: installs it if absent, then stops
/// the running one so the reconnect starts it. Ends every session there.
#[tauri::command]
#[specta::specta]
pub async fn update_host_server(
    app: AppHandle,
    ssh: State<'_, Ssh>,
    host: Host,
    on_step: Channel<ProvisionStep>,
) -> CmdResult<()> {
    let data_dir = app_data_dir(&app)?;
    provision::update(
        &host,
        &data_dir,
        |step| {
            let _ = on_step.send(step);
        },
        &ssh,
    )
    .await
}

/// What ssh is waiting on the user for, oldest first; `SshPromptsChanged`
/// carries every later change.
#[tauri::command]
#[specta::specta]
pub fn list_ssh_prompts(ssh: State<'_, Ssh>) -> CmdResult<Vec<SshPrompt>> {
    Ok(ssh.prompts().borrow().clone())
}

/// `None` cancels, which stops that ssh run.
#[tauri::command]
#[specta::specta]
pub fn answer_ssh_prompt(ssh: State<'_, Ssh>, id: String, answer: Option<String>) -> CmdResult<()> {
    if !ssh.answer(&id, answer) {
        log::info!("ssh prompt {id} was already gone when it was answered");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(destination: &str, label: &str) -> Target {
        Target::Remote(Host {
            destination: destination.into(),
            label: label.into(),
        })
    }

    #[test]
    fn a_reconnect_to_the_same_destination_is_allowed() {
        let connected = host("nexus", "Nexus");
        // The label is editable and free to change; the destination is the
        // profile namespace, so only it decides.
        let configured = host("nexus", "Nexus (linux)");
        assert!(server_change(&connected, &configured).is_none());
        assert!(server_change(&Target::Own, &Target::Own).is_none());
    }

    #[test]
    fn a_changed_host_is_refused_and_names_relaunching() {
        let err = server_change(&host("nexus", "Nexus"), &host("office", "Office"))
            .expect("a different destination is refused");
        assert_eq!(err.code, "host_changed");
        assert!(err.message.contains("Office"));
        assert!(err.remedy.unwrap().contains("Relaunch"));
    }

    #[test]
    fn switching_to_or_from_the_local_server_is_refused_too() {
        let err = server_change(&host("nexus", "Nexus"), &Target::Own)
            .expect("the local server is a different profile");
        assert_eq!(err.code, "host_changed");
        assert!(err.message.contains("this machine"));
        assert_eq!(
            server_change(&Target::Own, &host("nexus", "Nexus"))
                .expect("a remote host is a different profile")
                .code,
            "host_changed"
        );
    }
}
