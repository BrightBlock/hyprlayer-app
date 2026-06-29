//! PTY terminal IPC commands: open, attach, write, resize, close.
//!
//! Mirrors the session-command pattern from `sessions.rs`:
//! - `terminal_open` spawns a PTY and starts a forwarder task.
//! - `terminal_attach` replays the in-memory scrollback then tails live output.
//! - `terminal_write` forwards keyboard input (base64-encoded for binary safety).
//! - `terminal_resize` notifies the PTY of a size change.
//! - `terminal_close` kills the shell and drops the registry entry.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use hyprlayer_core::terminal::{PtySession, SpawnTerminal, TerminalEvent};
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::broadcast;

use super::db_task;
use crate::error::{AppError, CmdResult};
use crate::state::AppState;
use crate::terminal_registry::TerminalRegistry;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// Open a new PTY for the given workstream, spawn the user's shell in the
/// worktree directory, and start forwarding output to `on_event`.
///
/// Returns the `terminal_id` used for subsequent write/resize/close calls.
#[tauri::command]
#[specta::specta]
pub async fn terminal_open(
    state: State<'_, AppState>,
    workstream_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let ws = {
        let id = workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .ok_or_else(|| AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {workstream_id}"),
        remedy: None,
    })?;

    let path = state.binaries.augmented_path().await;
    let env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];

    let session = PtySession::spawn(SpawnTerminal {
        shell: default_shell(),
        // Spawn as a login shell, the way Terminal.app / iTerm do — it sources the
        // user's full profile chain (`~/.zprofile`/`~/.zlogin` as well as
        // `~/.zshrc`), so the terminal matches the shell the user gets elsewhere.
        args: vec!["-l".to_string()],
        cwd: PathBuf::from(&ws.worktree_path),
        cols,
        rows,
        env,
        workstream_id: ws.id.clone(),
    })
    .map_err(|e| AppError::internal(format!("open pty: {e}")))?;

    let terminal_id = uuid::Uuid::new_v4().to_string();
    state
        .terminals
        .insert(terminal_id.clone(), session.clone())
        .await;
    attach_stream(
        &session,
        on_event,
        state.terminals.clone(),
        terminal_id.clone(),
    );
    Ok(terminal_id)
}

/// Attach to an existing PTY: replay its scrollback then tail live output.
///
/// Returns `false` if the PTY is gone (e.g. after an app restart), in which
/// case the frontend should call `terminal_open` to start a fresh one.
#[tauri::command]
#[specta::specta]
pub async fn terminal_attach(
    state: State<'_, AppState>,
    terminal_id: String,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<bool> {
    let Some(session) = state.terminals.get(&terminal_id).await else {
        return Ok(false); // dead (e.g. after restart) — caller opens a fresh one
    };
    attach_stream(&session, on_event, state.terminals.clone(), terminal_id);
    Ok(true)
}

/// Forward base64-encoded keyboard input to the PTY.
#[tauri::command]
#[specta::specta]
pub async fn terminal_write(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> CmdResult<()> {
    let bytes = B64
        .decode(data)
        .map_err(|e| AppError::internal(format!("bad input: {e}")))?;
    if let Some(s) = state.terminals.get(&terminal_id).await {
        s.write_input(&bytes)
            .map_err(|e| AppError::internal(e.to_string()))?;
    }
    Ok(())
}

/// Notify the PTY of a terminal-size change so it can reflow output.
#[tauri::command]
#[specta::specta]
pub async fn terminal_resize(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    if let Some(s) = state.terminals.get(&terminal_id).await {
        let _ = s.resize(cols, rows);
    }
    Ok(())
}

/// Kill the shell process and remove the terminal from the registry.
#[tauri::command]
#[specta::specta]
pub async fn terminal_close(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    if let Some(s) = state.terminals.remove(&terminal_id).await {
        s.kill();
    }
    Ok(())
}

/// Open a new PTY running `hyprlayer <args>` (no workstream context needed).
///
/// Used by the onboarding wizard to embed `hyprlayer ai configure` or similar
/// interactive CLI commands inside the app without leaving to a terminal.
/// Reuses the same id-based write/resize/close/attach commands as a workstream
/// PTY — the only difference is how it is opened (binary + args vs. shell in cwd).
#[tauri::command]
#[specta::specta]
pub async fn spawn_cli_terminal(
    state: State<'_, AppState>,
    args: Vec<String>,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let bin = state
        .binaries
        .require(hyprlayer_core::process::binaries::BinaryKind::Hyprlayer)
        .await
        .map_err(AppError::from)?;
    let path = state.binaries.augmented_path().await;
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

    let session = PtySession::spawn(SpawnTerminal {
        shell: bin,
        args,
        cwd,
        cols,
        rows,
        env: vec![("PATH".into(), path.to_string_lossy().into_owned())],
        workstream_id: "onboarding".into(),
    })
    .map_err(|e| AppError::internal(format!("open pty: {e}")))?;

    let terminal_id = uuid::Uuid::new_v4().to_string();
    state
        .terminals
        .insert(terminal_id.clone(), session.clone())
        .await;
    attach_stream(
        &session,
        on_event,
        state.terminals.clone(),
        terminal_id.clone(),
    );
    Ok(terminal_id)
}

/// Open a PTY running a single command line through the user's login shell, in
/// the home directory. Used by the Harnesses settings page to run interactive
/// flows that aren't the hyprlayer CLI — e.g. `claude auth login` or the
/// OpenCode installer (`curl -fsSL https://opencode.ai/install | bash`). The
/// command is run exactly as if the user typed it into their own shell.
#[tauri::command]
#[specta::specta]
pub async fn spawn_shell_terminal(
    state: State<'_, AppState>,
    command: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let path = state.binaries.augmented_path().await;
    let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

    // Login shell so the command sees the user's full PATH/profile.
    let args = login_shell_args(&command);

    let session = PtySession::spawn(SpawnTerminal {
        shell: default_shell(),
        args,
        cwd,
        cols,
        rows,
        env: vec![("PATH".into(), path.to_string_lossy().into_owned())],
        workstream_id: "harness".into(),
    })
    .map_err(|e| AppError::internal(format!("open pty: {e}")))?;

    let terminal_id = uuid::Uuid::new_v4().to_string();
    state
        .terminals
        .insert(terminal_id.clone(), session.clone())
        .await;
    attach_stream(
        &session,
        on_event,
        state.terminals.clone(),
        terminal_id.clone(),
    );
    Ok(terminal_id)
}

/// Open a PTY running a Multiplexer engine CLI (Claude Code / Codex / Gemini /
/// a custom command) in the workstream's worktree.
///
/// The command runs through the user's login shell so it resolves on the same
/// PATH and profile the user gets in their own terminal; on unix it's `exec`'d
/// so the CLI takes over the shell's PID and SIGINT/exit stay clean. Shares the
/// id-based write/resize/close/attach lifecycle with [`terminal_open`] — only
/// the spawned program differs, so the frontend reuses `terminal_write` (e.g. a
/// `\x03` to Stop), `terminal_resize`, `terminal_attach`, and `terminal_close`.
#[tauri::command]
#[specta::specta]
pub async fn agent_open(
    state: State<'_, AppState>,
    workstream_id: String,
    command: String,
    args: Vec<String>,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<String> {
    let ws = {
        let id = workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .ok_or_else(|| AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {workstream_id}"),
        remedy: None,
    })?;

    if command.trim().is_empty() {
        return Err(AppError::internal("multiplexer engine command is empty"));
    }

    let path = state.binaries.augmented_path().await;
    let env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];

    let session = PtySession::spawn(SpawnTerminal {
        shell: default_shell(),
        args: login_shell_args(&agent_command_line(&command, &args)),
        cwd: PathBuf::from(&ws.worktree_path),
        cols,
        rows,
        env,
        workstream_id: ws.id.clone(),
    })
    .map_err(|e| AppError::internal(format!("open agent pty: {e}")))?;

    let terminal_id = uuid::Uuid::new_v4().to_string();
    state
        .terminals
        .insert(terminal_id.clone(), session.clone())
        .await;
    attach_stream(
        &session,
        on_event,
        state.terminals.clone(),
        terminal_id.clone(),
    );
    Ok(terminal_id)
}

/// Resize an agent PTY and then "nudge" it so a TUI child (Claude Code / Codex /
/// Gemini) actually repaints at the new size. A single SIGWINCH while the CLI is
/// idle sometimes doesn't trigger a redraw — the composer stays pinned where the
/// old bottom was, stranded mid-pane after the pane grows (e.g. hiding the
/// scripts dock). Resizing off-by-one, pausing, then back to the real size forces
/// the redraw (mirrors herdr's `nudge_child_redraw`). Used for discrete resizes
/// (dock toggle, (re)attach); continuous drags use the plain [`terminal_resize`].
#[tauri::command]
#[specta::specta]
pub async fn agent_resize(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    let Some(session) = state.terminals.get(&terminal_id).await else {
        return Ok(());
    };
    let nudge = rows.saturating_sub(1).max(2);
    if nudge != rows {
        let _ = session.resize(cols, nudge);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let _ = session.resize(cols, rows);
    Ok(())
}

/// Build the single command line handed to [`login_shell_args`] for an engine.
/// On unix we `exec` the program so the CLI inherits the login shell's PID —
/// then Ctrl-C and the process-group kill on close hit the CLI directly. Windows
/// PowerShell has no `exec`; the program runs as a child via the call operator
/// (`&`), and the registry still tears it down by process group.
fn agent_command_line(command: &str, args: &[String]) -> String {
    #[cfg(not(windows))]
    {
        let mut parts = Vec::with_capacity(args.len() + 2);
        parts.push("exec".to_string());
        parts.push(sh_quote(command));
        parts.extend(args.iter().map(|a| sh_quote(a)));
        parts.join(" ")
    }
    #[cfg(windows)]
    {
        let mut parts = Vec::with_capacity(args.len() + 1);
        parts.push(ps_quote(command));
        parts.extend(args.iter().map(|a| ps_quote(a)));
        format!("& {}", parts.join(" "))
    }
}

/// POSIX single-quote: wrap in `'…'`, escaping embedded quotes as `'\''`.
#[cfg(not(windows))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// PowerShell single-quote: wrap in `'…'`, doubling embedded single quotes.
#[cfg(windows)]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Replay scrollback then forward live output — the shared open/attach path.
/// Snapshots scrollback and subscribes atomically (see
/// [`PtySession::subscribe_with_scrollback`]) so no output is lost or duplicated
/// across the handoff, then streams the rest onto the UI channel.
pub(crate) fn attach_stream(
    session: &PtySession,
    on_event: Channel<TerminalEvent>,
    registry: Arc<TerminalRegistry>,
    terminal_id: String,
) {
    let (rx, scrollback) = session.subscribe_with_scrollback();
    if !scrollback.is_empty() {
        let _ = on_event.send(TerminalEvent::Output { data: scrollback });
    }
    forward(rx, on_event, registry, terminal_id);
}

/// Forward a PTY's broadcast onto a UI Channel; remove from the registry on exit.
fn forward(
    mut rx: broadcast::Receiver<TerminalEvent>,
    on_event: Channel<TerminalEvent>,
    registry: Arc<TerminalRegistry>,
    terminal_id: String,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let exited = matches!(ev, TerminalEvent::Exited { .. });
                    let closed = on_event.send(ev).is_err();
                    if exited {
                        registry.remove(&terminal_id).await;
                        break;
                    }
                    if closed {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(not(windows))]
pub(crate) fn default_shell() -> PathBuf {
    // `$SHELL` is the user's login shell (folded in from the login environment
    // for GUI launches); `/bin/sh` is the POSIX-guaranteed fallback when it's
    // unset, which `/bin/zsh` is not on every Unix.
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

#[cfg(windows)]
pub(crate) fn default_shell() -> PathBuf {
    // Windows has no `$SHELL`, and the Unix `/bin/sh` fallback doesn't exist —
    // it would fail to spawn and leave the terminal blank. Prefer PowerShell 7
    // (`pwsh`), fall back to the built-in Windows PowerShell, then `%ComSpec%`
    // (cmd.exe). Resolved to a full path so the PTY's CreateProcess doesn't
    // depend on its own PATH search.
    which::which("pwsh")
        .or_else(|_| which::which("powershell"))
        .ok()
        .or_else(|| std::env::var_os("ComSpec").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

/// The default shell's display name — its file stem (e.g. "zsh", "bash", "pwsh",
/// "powershell", "cmd"). Labels the dock's ad-hoc terminal tabs by the shell they
/// actually spawn, instead of a hardcoded guess.
#[tauri::command]
#[specta::specta]
pub fn default_shell_name() -> CmdResult<String> {
    Ok(default_shell()
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("shell")
        .to_string())
}

/// Args to run `command` through [`default_shell`]: `-lc` on unix, PowerShell's
/// `-NoProfile -Command` on Windows.
pub(crate) fn login_shell_args(command: &str) -> Vec<String> {
    #[cfg(not(windows))]
    {
        vec!["-lc".to_string(), command.to_string()]
    }
    #[cfg(windows)]
    {
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ]
    }
}
