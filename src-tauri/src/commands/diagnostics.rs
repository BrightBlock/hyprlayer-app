//! What a bug report needs: the versions, the connection, and the tail of
//! every log on both machines, with the home directory masked.

use std::path::{Path, PathBuf};

use hyprlayer_server::logging as server_logging;
use tauri::{AppHandle, State};

use crate::commands::hosts::{app_data_dir, app_log_dir};
use crate::error::{AppError, CmdResult};
use crate::hosts::Target;
use crate::provision;
use crate::ssh::Ssh;
use crate::state::AppState;
use crate::supervisor;

const TAIL_BYTES: u64 = 64 * 1024;

/// tauri-plugin-log names its file after the product.
fn client_log(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(app_log_dir(app)?.join(format!("{}.log", app.package_info().name)))
}

pub(crate) fn reveal(app: &AppHandle) -> CmdResult<()> {
    let file = client_log(app)?;
    let target = if file.exists() {
        file
    } else {
        app_log_dir(app)?
    };
    tauri_plugin_opener::reveal_item_in_dir(&target)
        .map_err(|e| AppError::internal(format!("could not reveal {}: {e}", target.display())))
}

#[tauri::command]
#[specta::specta]
pub fn reveal_logs(app: AppHandle) -> CmdResult<()> {
    reveal(&app)
}

#[tauri::command]
#[specta::specta]
pub async fn collect_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
    ssh: State<'_, Ssh>,
) -> CmdResult<String> {
    let package = app.package_info();
    let status = state.status();
    let target = state.target();
    let mut out = format!(
        "{} {} ({}, {} build) on {}/{}, protocol {}, ships server {} (needs {})\n",
        package.name,
        package.version,
        app.config().identifier,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH,
        hyprlayer_proto::PROTOCOL_VERSION,
        crate::server_version::SERVER_VERSION,
        crate::server_version::MIN_SERVER_VERSION,
    );
    out.push_str(&format!(
        "server: {}, {:?}, version {}, protocol {}{}\n",
        target.name(),
        status.state,
        status.server_release.as_deref().unwrap_or("unknown"),
        status
            .server_version
            .map_or("unknown".to_string(), |v| v.to_string()),
        if status.server_outdated {
            ", not the server this app ships"
        } else {
            ""
        }
    ));
    if let Some(error) = &status.error {
        out.push_str(&format!(
            "connection error: {}: {}\n",
            error.code, error.message
        ));
    }

    let client_log = client_log(&app)?;
    out.push_str(&format!("\n== {} ==\n", client_log.display()));
    out.push_str(&server_logging::tail_file(&client_log, TAIL_BYTES));
    out.push_str("\n== server ==\n");
    // A report never stops to ask for a password; it says sign-in is needed.
    let ssh = ssh.batch();
    out.push_str(&server_logs(&target, &app_log_dir(&app)?, &app_data_dir(&app)?, &ssh).await);
    Ok(mask(&out))
}

async fn server_logs(
    target: &Target,
    own_log_dir: &Path,
    app_data_dir: &Path,
    ssh: &Ssh,
) -> String {
    match target {
        Target::Own => server_logging::tail_logs(own_log_dir, TAIL_BYTES),
        Target::LocalProfile(data_dir) => {
            server_logging::tail_logs(&server_logging::log_dir(data_dir), TAIL_BYTES)
        }
        Target::Remote(host) => {
            // A stale install cannot be relayed through but can still print logs.
            let version =
                match provision::readiness(host, app_data_dir, ssh).await {
                    Ok(found) => found
                        .usable_version
                        .or((found.state == provision::HostServerState::Stale)
                            .then_some(found.version)),
                    Err(e) => {
                        return format!(
                            "could not inspect the server on {}: {}: {}\n",
                            host.destination, e.code, e.message
                        )
                    }
                };
            let Some(version) = version else {
                return format!("no usable server is installed on {}\n", host.destination);
            };
            match supervisor::ssh_capture(host, &provision::logs_command(&version), ssh).await {
                Ok(text) => format!("on {}:\n{text}", host.destination),
                Err(e) => format!(
                    "could not read the server logs on {}: {}: {}\n",
                    host.destination, e.code, e.message
                ),
            }
        }
    }
}

/// A report gets pasted into issues and chats: no home path, no URL tokens.
fn mask(text: &str) -> String {
    let text = hyprlayer_core::redact::url_credentials(text);
    match dirs::home_dir().map(|home| home.display().to_string()) {
        Some(home) if !home.is_empty() => text.replace(&home, "~"),
        _ => text,
    }
}
