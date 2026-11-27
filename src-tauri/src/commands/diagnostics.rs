//! The bug-report builder lives in `hyprlayer_client::diagnostics`; this resolves the app's
//! paths and state for it.

use std::path::PathBuf;

use hyprlayer_client::diagnostics::{self, App, Connection, Report};
use tauri::{AppHandle, State};

use crate::commands::hosts::{app_data_dir, app_log_dir};
use crate::error::{AppError, CmdResult};
use crate::ssh::Ssh;
use crate::state::AppState;

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
    let report = Report {
        app: App {
            name: &package.name,
            version: &package.version.to_string(),
            identifier: &app.config().identifier,
        },
        connection: Connection {
            target: state.target(),
            state: format!("{:?}", status.state),
            server_release: status.server_release,
            server_protocol: status.server_version,
            server_outdated: status.server_outdated,
            error: status.error.map(|e| (e.code, e.message)),
        },
        client_log: client_log(&app)?,
        app_log_dir: app_log_dir(&app)?,
        app_data_dir: app_data_dir(&app)?,
    };
    // A report never stops to ask for a password; it says sign-in is needed.
    Ok(diagnostics::build(report, &ssh.batch()).await)
}
