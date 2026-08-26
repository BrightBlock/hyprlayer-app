use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use hyprlayer_core::process::Exec;
use hyprlayer_core::repo_settings::{self, ScriptSettings};
use hyprlayer_core::scripts::{self, ScriptKind};
use hyprlayer_core::store::Workstream;
use hyprlayer_core::terminal::{PtySession, SpawnTerminal, TerminalEvent};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_specta::Event;
use tokio::sync::broadcast;

use super::terminal::{attach_stream, default_shell, login_shell_args};
use super::{blocking, db_task, ensure_registered_project};
use crate::error::{AppError, CmdResult};
use crate::events::ScriptRunChanged;
use crate::script_registry::{RunPhase, ScriptRuntime};
use crate::state::AppState;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

const SETUP_TIMEOUT_SECS: u64 = 600;
const ARCHIVE_TIMEOUT_SECS: u64 = 300;

async fn read_script_settings(project_path: &str) -> CmdResult<ScriptSettings> {
    let repo = PathBuf::from(project_path);
    let settings = blocking("read_scripts", move || repo_settings::read(&repo))
        .await?
        .map_err(AppError::from)?;
    Ok(settings.scripts)
}

#[tauri::command]
#[specta::specta]
pub async fn get_scripts(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ScriptSettings> {
    ensure_registered_project(&state, &project_path).await?;
    read_script_settings(&project_path).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_scripts(
    state: State<'_, AppState>,
    project_path: String,
    scripts: ScriptSettings,
) -> CmdResult<()> {
    ensure_registered_project(&state, &project_path).await?;
    let repo = PathBuf::from(&project_path);
    blocking("write_scripts", move || {
        repo_settings::set_scripts(&repo, &scripts)
    })
    .await?
    .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_script_runtime(
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<ScriptRuntime> {
    Ok(state.scripts.get_runtime(&workstream_id).await)
}

#[tauri::command]
#[specta::specta]
pub async fn list_running_scripts(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(state.scripts.running_ids().await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_script_output(
    state: State<'_, AppState>,
    workstream_id: String,
    kind: ScriptKind,
) -> CmdResult<Option<String>> {
    Ok(state.scripts.get_output(&workstream_id, kind).await)
}

#[tauri::command]
#[specta::specta]
pub async fn script_clear_output(
    state: State<'_, AppState>,
    workstream_id: String,
    kind: ScriptKind,
    terminal_id: Option<String>,
) -> CmdResult<()> {
    state.scripts.clear_output(&workstream_id, kind).await;
    if let Some(id) = terminal_id {
        if let Some(session) = state.terminals.get(&id).await {
            session.clear_scrollback();
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn script_run(
    app: AppHandle,
    state: State<'_, AppState>,
    workstream_id: String,
    kind: ScriptKind,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let item = load_workstream(&state, &workstream_id).await?;
    let commands = resolve_script(&item.project_path, kind).await?;
    if commands.trim().is_empty() {
        return Err(AppError {
            code: "script_empty".into(),
            message: format!(
                "No {} script is configured for this project.",
                kind.as_str()
            ),
            remedy: Some("Add commands in Project Settings → Scripts.".into()),
        });
    }

    if kind == ScriptKind::Run {
        state.scripts.stop_run(&workstream_id).await;
    }

    let (terminal_id, session) = spawn_script(&state, &item, &commands, cols, rows).await?;

    state
        .scripts
        .set_status(&workstream_id, kind, RunPhase::Running, None)
        .await;
    if kind == ScriptKind::Run {
        state
            .scripts
            .register_run(&workstream_id, terminal_id.clone(), session.clone())
            .await;
    }
    emit_changed(&app, &workstream_id, kind, RunPhase::Running, None, None);

    spawn_monitor(
        app,
        state.scripts.clone(),
        session.clone(),
        workstream_id,
        kind,
        terminal_id.clone(),
    );
    attach_stream(
        &session,
        on_event,
        state.terminals.clone(),
        terminal_id.clone(),
    );
    Ok(terminal_id)
}

#[tauri::command]
#[specta::specta]
pub async fn script_stop(state: State<'_, AppState>, workstream_id: String) -> CmdResult<()> {
    state.scripts.stop_run(&workstream_id).await;
    Ok(())
}

pub(crate) async fn run_lifecycle_script(
    app: &AppHandle,
    state: &AppState,
    item: &Workstream,
    kind: ScriptKind,
) -> Option<RunPhase> {
    let commands = match resolve_script(&item.project_path, kind).await {
        Ok(c) if !c.trim().is_empty() => c,
        Ok(_) => return None,
        Err(e) => {
            log::warn!("read {} script: {}", kind.as_str(), e.message);
            return None;
        }
    };

    let port = state.scripts.take_port(&item.id).await;
    let augmented = state.binaries.augmented_path().await;
    let args = login_shell_args(&scripts::build_block(&commands));
    let timeout = match kind {
        ScriptKind::Archive => ARCHIVE_TIMEOUT_SECS,
        _ => SETUP_TIMEOUT_SECS,
    };

    state
        .scripts
        .set_status(&item.id, kind, RunPhase::Running, None)
        .await;
    emit_changed(app, &item.id, kind, RunPhase::Running, None, None);

    let mut exec = Exec::new(default_shell())
        .args(args)
        .cwd(item.worktree_path.clone())
        .own_process_group()
        .timeout(Duration::from_secs(timeout));
    for (key, value) in script_env(item, port, &augmented) {
        exec = exec.env(key, value);
    }

    let (phase, code, output) = match exec.run_unchecked().await {
        Ok(out) => {
            let phase = if out.exit_code == Some(0) {
                RunPhase::Passed
            } else {
                log::warn!(
                    "{} script exited {:?}: {}",
                    kind.as_str(),
                    out.exit_code,
                    out.stderr.trim()
                );
                RunPhase::Failed
            };
            let mut combined = out.stdout;
            if !out.stderr.is_empty() {
                if !combined.is_empty() && !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push_str(&out.stderr);
            }
            (phase, out.exit_code, combined)
        }
        Err(e) => {
            log::warn!("{} script failed to run: {e}", kind.as_str());
            (RunPhase::Failed, None, format!("{e}"))
        }
    };

    state.scripts.set_status(&item.id, kind, phase, code).await;
    state.scripts.set_output(&item.id, kind, output).await;
    emit_changed(app, &item.id, kind, phase, code, None);
    Some(phase)
}

fn script_env(item: &Workstream, port: u16, augmented: &OsStr) -> Vec<(String, String)> {
    vec![
        ("PATH".to_string(), augmented.to_string_lossy().into_owned()),
        (scripts::ENV_PORT.to_string(), port.to_string()),
        (
            scripts::ENV_WORKTREE.to_string(),
            item.worktree_path.clone(),
        ),
        (scripts::ENV_BRANCH.to_string(), item.branch.clone()),
        (scripts::ENV_PROJECT.to_string(), item.project_path.clone()),
        (scripts::ENV_SLUG.to_string(), item.slug.clone()),
    ]
}

async fn spawn_script(
    state: &AppState,
    item: &Workstream,
    commands: &str,
    cols: u16,
    rows: u16,
) -> CmdResult<(String, Arc<PtySession>)> {
    let port = state.scripts.take_port(&item.id).await;
    let augmented = state.binaries.augmented_path().await;
    let env = script_env(item, port, &augmented);
    let args = login_shell_args(&scripts::build_block(commands));

    let session = PtySession::spawn(SpawnTerminal {
        shell: default_shell(),
        args,
        cwd: PathBuf::from(&item.worktree_path),
        cols,
        rows,
        env,
        workstream_id: item.id.clone(),
    })
    .map_err(|e| AppError::internal(format!("spawn script pty: {e}")))?;

    let terminal_id = uuid::Uuid::new_v4().to_string();
    state
        .terminals
        .insert(terminal_id.clone(), session.clone())
        .await;
    Ok((terminal_id, session))
}

fn spawn_monitor(
    app: AppHandle,
    scripts: Arc<crate::script_registry::ScriptRegistry>,
    session: Arc<PtySession>,
    workstream_id: String,
    kind: ScriptKind,
    terminal_id: String,
) {
    let mut rx = session.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut port_found = false;
        loop {
            match rx.recv().await {
                Ok(TerminalEvent::Output { data }) => {
                    if kind == ScriptKind::Run && !port_found {
                        if let Some(port) = scan_for_port(&data) {
                            port_found = true;
                            scripts
                                .set_run_port(&workstream_id, &terminal_id, port)
                                .await;
                            emit_changed(
                                &app,
                                &workstream_id,
                                kind,
                                RunPhase::Running,
                                None,
                                Some(port),
                            );
                        }
                    }
                }
                Ok(TerminalEvent::Exited { code }) => {
                    let phase = if code == Some(0) {
                        RunPhase::Passed
                    } else {
                        RunPhase::Failed
                    };
                    scripts.set_status(&workstream_id, kind, phase, code).await;
                    scripts
                        .set_output(
                            &workstream_id,
                            kind,
                            String::from_utf8_lossy(&session.scrollback_bytes()).into_owned(),
                        )
                        .await;
                    if kind == ScriptKind::Run {
                        scripts.clear_run(&workstream_id, &terminal_id).await;
                    }
                    emit_changed(&app, &workstream_id, kind, phase, code, None);
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn scan_for_port(b64: &str) -> Option<u16> {
    let bytes = B64.decode(b64).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    text.lines().find_map(scripts::detect_port)
}

fn emit_changed(
    app: &AppHandle,
    workstream_id: &str,
    kind: ScriptKind,
    phase: RunPhase,
    exit_code: Option<i32>,
    port: Option<u16>,
) {
    let _ = ScriptRunChanged {
        workstream_id: workstream_id.to_string(),
        kind,
        phase,
        exit_code,
        port,
    }
    .emit(app);
}

async fn resolve_script(project_path: &str, kind: ScriptKind) -> CmdResult<String> {
    let scripts = read_script_settings(project_path).await?;
    Ok(match kind {
        ScriptKind::Setup => scripts.setup,
        ScriptKind::Run => scripts.run,
        ScriptKind::Archive => scripts.archive,
    })
}

async fn load_workstream(state: &AppState, id: &str) -> CmdResult<Workstream> {
    let id_owned = id.to_string();
    db_task(state.db.clone(), move |db| db.get_workstream(&id_owned))
        .await?
        .ok_or_else(|| AppError {
            code: "workstream_not_found".into(),
            message: format!("No workstream with id {id}"),
            remedy: None,
        })
}
