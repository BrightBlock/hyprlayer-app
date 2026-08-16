//! PTY terminal IPC commands: open, attach, write, resize, close.
//! Mirrors the session-command pattern from `sessions.rs`.

use std::io::BufRead;
use std::path::{Path, PathBuf};
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

/// Sessions whose mtime is within this window *before* a tab's launch are still
/// considered (clock granularity / a file written a beat before the watermark).
const CAPTURE_SKEW_MS: f64 = 2000.0;

/// Open a new PTY for the workstream (user's shell in the worktree), forwarding
/// output to `on_event`. Returns the `terminal_id` for write/resize/close.
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
        // Login shell (like Terminal.app/iTerm): sources the full profile chain
        // (`~/.zprofile`/`~/.zlogin`/`~/.zshrc`) so it matches the user's shell.
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

/// Attach to an existing PTY: replay scrollback then tail live output. Returns
/// `false` if the PTY is gone (e.g. after restart) — caller opens a fresh one.
#[tauri::command]
#[specta::specta]
pub async fn terminal_attach(
    state: State<'_, AppState>,
    terminal_id: String,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<bool> {
    let Some(session) = state.terminals.get(&terminal_id).await else {
        return Ok(false);
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

/// Open a new PTY running `hyprlayer <args>` (no workstream context). Used by
/// onboarding to embed interactive CLI flows (`hyprlayer ai configure`). Reuses
/// the id-based write/resize/close/attach commands.
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

/// Open a PTY running a single command line through the user's login shell in
/// the home dir. Used by Harnesses settings for interactive non-CLI flows (e.g.
/// `claude auth login`, the OpenCode installer) — run as if the user typed it.
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
/// custom) in the workstream's worktree. Runs through the login shell; on unix
/// it's `exec`'d so the CLI takes over the shell's PID and SIGINT/exit stay
/// clean. Shares the id-based write/resize/close/attach lifecycle with
/// [`terminal_open`] (e.g. frontend sends `\x03` via `terminal_write` to Stop).
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

/// Resize an agent PTY, then "nudge" it so an idle TUI child (Claude/Codex/
/// Gemini) actually repaints: a lone SIGWINCH while idle often doesn't trigger a
/// redraw, stranding the composer at the old bottom. Resizing off-by-one, pausing,
/// then back to the real size forces it (mirrors herdr's `nudge_child_redraw`).
/// For discrete resizes (dock toggle, (re)attach); drags use [`terminal_resize`].
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

/// Read back the conversation id a "capture-mode" CLI (Codex/Antigravity, which
/// auto-generate ids unlike Claude/Gemini) assigned itself, by scanning its
/// on-disk store — launched fresh, then resumed by this id.
///
/// Returns the newest session created after `after_ms` (epoch ms, the tab's
/// launch) not already claimed (`exclude`); for Codex it must also match this
/// worktree's cwd, so another worktree's session can't be mis-attributed. `None`
/// until the session hits disk (first message), so the frontend polls. Best-
/// effort: two tabs opened in the same worktree at once may swap ids (no per-
/// process key); `exclude` still keeps them distinct.
#[tauri::command]
#[specta::specta]
pub async fn capture_agent_session(
    state: State<'_, AppState>,
    workstream_id: String,
    engine_id: String,
    after_ms: f64,
    exclude: Vec<String>,
) -> CmdResult<Option<String>> {
    let worktree = {
        let id = workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .map(|ws| ws.worktree_path);
    let id = tokio::task::spawn_blocking(move || match engine_id.as_str() {
        "codex" => capture_codex(worktree.as_deref(), after_ms, &exclude),
        "antigravity" => capture_antigravity(after_ms, &exclude),
        _ => None,
    })
    .await
    .map_err(|e| AppError::internal(format!("capture join: {e}")))?;
    Ok(id)
}

/// Mint a Codex conversation id *deterministically*, sidestepping the disk-scan
/// attribution race: run `codex exec --json` (first event is
/// `{"type":"thread.started","thread_id":…}`), take that id, wait for its rollout
/// to hit disk (so `resume <id>` finds it), then kill the probe. Reading the id
/// from our own process can't confuse it with a sibling tab's. `None` if Codex
/// can't run or never announces a thread (caller falls back to disk capture).
#[tauri::command]
#[specta::specta]
pub async fn mint_codex_session(
    state: State<'_, AppState>,
    workstream_id: String,
    command: String,
) -> CmdResult<Option<String>> {
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
    let worktree = PathBuf::from(&ws.worktree_path);
    let path = path.to_string_lossy().into_owned();
    // Bound the probe: a hung CLI (auth/network) must not block the tab forever —
    // time out to None and let the caller fall back. The orphaned probe exits on
    // its own once its turn ends.
    let id = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        tokio::task::spawn_blocking(move || mint_codex_blocking(&command, &worktree, &path)),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    Ok(id)
}

/// Whether `session_id` is actually persisted in `engine_id`'s on-disk store —
/// the authoritative resume gate. A pinned-but-never-used id has no file yet, so
/// resuming would exit "No conversation found"; this lets the caller launch fresh.
/// Unknown engines (Gemini, custom) return false — caller uses its own heuristic.
#[tauri::command]
#[specta::specta]
pub async fn agent_session_exists(
    state: State<'_, AppState>,
    workstream_id: String,
    engine_id: String,
    session_id: String,
) -> CmdResult<bool> {
    let worktree = {
        let id = workstream_id.clone();
        db_task(state.db.clone(), move |db| db.get_workstream(&id)).await?
    }
    .map(|ws| ws.worktree_path);
    let exists = tokio::task::spawn_blocking(move || match engine_id.as_str() {
        "claude-code" => claude_session_exists(worktree.as_deref(), &session_id),
        "codex" => codex_rollout_exists(&session_id),
        "antigravity" => antigravity_brain_exists(&session_id),
        _ => false,
    })
    .await
    .map_err(|e| AppError::internal(format!("session-exists join: {e}")))?;
    Ok(exists)
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

/// File/dir mtime as epoch milliseconds.
fn mtime_ms(path: &Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as f64,
    )
}

/// `(path, mtime_ms)` for every `*.jsonl` under `dir` (bounded depth) whose mtime
/// is at or after `cutoff_ms`. Stats are cheap and the cutoff is recent, so only a
/// handful survive — the caller reads contents for just those.
fn recent_jsonl(dir: &Path, max_depth: usize, cutoff_ms: f64) -> Vec<(PathBuf, f64)> {
    fn walk(dir: &Path, depth: usize, cutoff: f64, out: &mut Vec<(PathBuf, f64)>) {
        if depth == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                walk(&path, depth - 1, cutoff, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(m) = mtime_ms(&path) {
                    if m >= cutoff {
                        out.push((path, m));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, max_depth, cutoff_ms, &mut out);
    out
}

/// First-line `session_meta` of a Codex rollout → `(session_id, cwd)`.
fn codex_session_meta(file: &Path) -> Option<(String, String)> {
    let f = std::fs::File::open(file).ok()?;
    let mut first = String::new();
    std::io::BufReader::new(f).read_line(&mut first).ok()?;
    let v: serde_json::Value = serde_json::from_str(first.trim()).ok()?;
    let payload = v.get("payload")?;
    let id = payload.get("session_id")?.as_str()?.to_string();
    let cwd = payload.get("cwd")?.as_str()?.to_string();
    Some((id, cwd))
}

/// Newest Codex session created after the watermark whose recorded cwd matches the
/// worktree and that isn't already claimed. See [`capture_agent_session`]. Codex
/// lays sessions out as `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (depth 4).
fn capture_codex(worktree: Option<&str>, after_ms: f64, exclude: &[String]) -> Option<String> {
    let root = dirs::home_dir()?.join(".codex").join("sessions");
    let cutoff = after_ms - CAPTURE_SKEW_MS;
    let mut best: Option<(f64, String)> = None;
    for (file, mtime) in recent_jsonl(&root, 4, cutoff) {
        let Some((id, cwd)) = codex_session_meta(&file) else {
            continue;
        };
        if worktree.is_some_and(|w| w != cwd) || exclude.iter().any(|e| e == &id) {
            continue;
        }
        if best.as_ref().is_none_or(|(m, _)| mtime > *m) {
            best = Some((mtime, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Spawn `codex exec --json` in `worktree`, read its first `thread.started` event
/// for the id, wait for the session's rollout to be on disk (so `resume` works),
/// then kill the probe. Read-only sandbox so the throwaway prompt can't mutate the
/// tree. See [`mint_codex_session`].
fn mint_codex_blocking(command: &str, worktree: &Path, path: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(command);
    hyprlayer_core::process::appimage::scrub_std(&mut cmd);
    let mut child = cmd
        .args([
            "exec",
            "--json",
            "--skip-git-repo-check",
            "-s",
            "read-only",
            "hello.",
        ])
        .current_dir(worktree)
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut id = None;
    if let Some(stdout) = child.stdout.take() {
        // Drain to EOF — let the seed turn finish and the process exit — so the
        // session is persisted *complete*. Killing mid-turn leaves it "interrupted",
        // which codex then surfaces on resume. The id is the first event.
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if id.is_none() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    if v.get("type").and_then(|t| t.as_str()) == Some("thread.started") {
                        id = v
                            .get("thread_id")
                            .and_then(|t| t.as_str())
                            .map(String::from);
                    }
                }
            }
        }
    }
    let _ = child.wait();
    id
}

/// Whether Claude has a transcript for `session_id` under this worktree. Claude
/// stores it at `~/.claude/projects/<cwd-with-non-alnum→->/<session_id>.jsonl`.
fn claude_session_exists(worktree: Option<&str>, session_id: &str) -> bool {
    let (Some(worktree), Some(home)) = (worktree, dirs::home_dir()) else {
        return false;
    };
    let slug: String = worktree
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude")
        .join("projects")
        .join(slug)
        .join(format!("{session_id}.jsonl"))
        .exists()
}

/// Whether a Codex rollout file for `session_id` exists yet (its id is in the
/// `rollout-<ts>-<id>.jsonl` filename).
fn codex_rollout_exists(session_id: &str) -> bool {
    let Some(root) = dirs::home_dir().map(|h| h.join(".codex").join("sessions")) else {
        return false;
    };
    recent_jsonl(&root, 4, 0.0).iter().any(|(p, _)| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(session_id))
    })
}

/// Whether an Antigravity conversation dir for `session_id` exists.
fn antigravity_brain_exists(session_id: &str) -> bool {
    dirs::home_dir()
        .map(|h| {
            h.join(".gemini")
                .join("antigravity-cli")
                .join("brain")
                .join(session_id)
                .is_dir()
        })
        .unwrap_or(false)
}

/// Newest Antigravity conversation dir created after the watermark and not already
/// claimed. The store (`~/.gemini/antigravity-cli/brain/<id>/`) isn't cwd-scoped,
/// so this is timestamp-only (no worktree match available).
fn capture_antigravity(after_ms: f64, exclude: &[String]) -> Option<String> {
    let root = dirs::home_dir()?
        .join(".gemini")
        .join("antigravity-cli")
        .join("brain");
    let cutoff = after_ms - CAPTURE_SKEW_MS;
    let mut best: Option<(f64, String)> = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || exclude.iter().any(|e| e == &name) {
            continue;
        }
        let Some(m) = mtime_ms(&entry.path()) else {
            continue;
        };
        if m >= cutoff && best.as_ref().is_none_or(|(bm, _)| m > *bm) {
            best = Some((m, name));
        }
    }
    best.map(|(_, id)| id)
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
