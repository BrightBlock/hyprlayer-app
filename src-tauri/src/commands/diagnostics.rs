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

/// Perf harness hook: the scenario in, the report out, and the window size for the resize drag.
#[cfg(feature = "perf-probe")]
#[tauri::command]
pub async fn perf_probe(
    window: tauri::WebviewWindow,
    op: String,
    payload: Option<serde_json::Value>,
) -> CmdResult<serde_json::Value> {
    use serde_json::json;
    let env = |name: &str| std::env::var(name).ok();
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    match op.as_str() {
        "scenario" => {
            let path = env("HYPRLAYER_PERF_SCENARIO")
                .ok_or_else(|| AppError::internal("HYPRLAYER_PERF_SCENARIO is not set"))?;
            let text = std::fs::read_to_string(&path)
                .map_err(|e| AppError::internal(format!("could not read {path}: {e}")))?;
            let scenario: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| AppError::internal(format!("{path} is not JSON: {e}")))?;
            Ok(json!({
                "scenario": scenario,
                "t0Ns": env("HYPRLAYER_PERF_T0_NS").and_then(|v| v.parse::<u64>().ok()),
                "nowNs": now_ns,
                "bigFile": env("HYPRLAYER_PERF_BIGFILE"),
                "terminals": env("HYPRLAYER_PERF_TERMINALS")
                    .map(|v| v.split(',').map(str::to_owned).collect::<Vec<_>>())
                    .unwrap_or_default(),
            }))
        }
        "report" => {
            let path = env("HYPRLAYER_PERF_REPORT")
                .ok_or_else(|| AppError::internal("HYPRLAYER_PERF_REPORT is not set"))?;
            let body = serde_json::to_vec_pretty(&payload.unwrap_or_default())
                .map_err(|e| AppError::internal(e.to_string()))?;
            std::fs::write(&path, body)
                .map_err(|e| AppError::internal(format!("could not write {path}: {e}")))?;
            Ok(json!(true))
        }
        "resize" => {
            let size = payload.unwrap_or_default();
            let dim = |k: &str| size.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            window
                .set_size(tauri::LogicalSize::new(dim("width"), dim("height")))
                .map_err(|e| AppError::internal(e.to_string()))?;
            Ok(json!(true))
        }
        // The harness samples /proc while the scenario waits here, then writes the ack file.
        "phase" => {
            let name = payload
                .and_then(|p| p.as_str().map(str::to_owned))
                .unwrap_or_default();
            let dir = env("HYPRLAYER_PERF_PHASE_DIR")
                .ok_or_else(|| AppError::internal("HYPRLAYER_PERF_PHASE_DIR is not set"))?;
            let dir = std::path::PathBuf::from(dir);
            std::fs::write(dir.join(format!("phase-{name}")), now_ns.to_string())
                .map_err(|e| AppError::internal(e.to_string()))?;
            let ack = dir.join(format!("ack-{name}"));
            for _ in 0..1200 {
                if ack.exists() {
                    return Ok(json!(true));
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(AppError::internal(format!("no ack for phase {name}")))
        }
        // Where the scenario is, so a stalled run names its step.
        "breadcrumb" => {
            let dir = env("HYPRLAYER_PERF_PHASE_DIR")
                .ok_or_else(|| AppError::internal("HYPRLAYER_PERF_PHASE_DIR is not set"))?;
            let mut body = payload.unwrap_or_default();
            body["atNs"] = json!(now_ns);
            std::fs::write(
                std::path::Path::new(&dir).join("breadcrumb.json"),
                body.to_string(),
            )
            .map_err(|e| AppError::internal(e.to_string()))?;
            Ok(json!(true))
        }
        "now" => Ok(json!(now_ns)),
        other => Err(AppError::internal(format!("unknown perf op {other}"))),
    }
}
