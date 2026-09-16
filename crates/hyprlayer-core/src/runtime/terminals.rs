use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use tokio::sync::mpsc;

use crate::error::{CoreError, Result};
use crate::process::binaries::BinaryKind;
use crate::store::Workstream;
use crate::terminal::{PtySession, SpawnTerminal, TerminalEvent};

use super::terminal_registry::TerminalRegistry;
use super::HyprlayerRuntime;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;
const CAPTURE_SKEW_MS: f64 = 2000.0;
const ATTACH_CHANNEL_CAPACITY: usize = 256;
const MINT_TIMEOUT: Duration = Duration::from_secs(90);

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

fn io_context(label: &str, err: std::io::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(format!("{label}: {err}")))
}

pub struct TerminalAttachment {
    pub terminal_id: String,
    pub live: mpsc::Receiver<TerminalEvent>,
}

pub struct OpenTerminalRequest {
    pub workstream_id: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct AttachTerminalRequest {
    pub terminal_id: String,
}

pub struct WriteTerminalRequest {
    pub terminal_id: String,
    pub data: String,
}

pub struct ResizeTerminalRequest {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct CloseTerminalRequest {
    pub terminal_id: String,
}

pub struct OpenCliTerminalRequest {
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

pub struct OpenShellTerminalRequest {
    pub command: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct OpenAgentTerminalRequest {
    pub workstream_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

pub struct ResizeAgentTerminalRequest {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct CaptureSessionRequest {
    pub workstream_id: String,
    pub engine_id: String,
    pub after_ms: f64,
    pub exclude: Vec<String>,
}

pub type CapturedSession = Option<String>;

pub struct MintCodexSessionRequest {
    pub workstream_id: String,
    pub command: String,
}

pub struct SessionExistsRequest {
    pub workstream_id: String,
    pub engine_id: String,
    pub session_id: String,
}

pub struct TerminalService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn terminals(&self) -> TerminalService<'_> {
        TerminalService { runtime: self }
    }
}

impl TerminalService<'_> {
    fn home(&self) -> PathBuf {
        self.runtime.environment.home_dir.clone()
    }

    async fn find_workstream(&self, id: &str) -> Result<Workstream> {
        let owned = id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_workstream(&owned))
            .await?
            .ok_or_else(|| CoreError::WorkstreamNotFound { id: id.to_string() })
    }

    async fn find_workstream_optional(&self, id: &str) -> Result<Option<Workstream>> {
        let owned = id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_workstream(&owned)).await
    }

    async fn register(&self, session: Arc<PtySession>) -> TerminalAttachment {
        let terminal_id = uuid::Uuid::new_v4().to_string();
        self.runtime
            .terminals
            .insert(terminal_id.clone(), session.clone())
            .await;
        let live = spawn_forward(session, terminal_id.clone(), self.runtime.terminals.clone());
        TerminalAttachment { terminal_id, live }
    }

    pub async fn open(&self, req: OpenTerminalRequest) -> Result<TerminalAttachment> {
        ensure_running(self.runtime)?;
        let ws = self.find_workstream(&req.workstream_id).await?;
        let path = self.runtime.binaries.augmented_path().await;
        let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
        env.extend(self.runtime.environment.overlay_env());
        let session = PtySession::spawn(SpawnTerminal {
            shell: default_shell(),
            args: vec!["-l".to_string()],
            cwd: PathBuf::from(&ws.worktree_path),
            cols: req.cols,
            rows: req.rows,
            env,
            workstream_id: ws.id.clone(),
        })
        .map_err(|e| io_context("open pty", e))?;
        Ok(self.register(session).await)
    }

    pub async fn attach(&self, req: AttachTerminalRequest) -> Result<Option<TerminalAttachment>> {
        ensure_running(self.runtime)?;
        let Some(session) = self.runtime.terminals.get(&req.terminal_id).await else {
            return Ok(None);
        };
        let live = spawn_forward(
            session,
            req.terminal_id.clone(),
            self.runtime.terminals.clone(),
        );
        Ok(Some(TerminalAttachment {
            terminal_id: req.terminal_id,
            live,
        }))
    }

    pub async fn write(&self, req: WriteTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        let bytes = B64
            .decode(req.data)
            .map_err(|e| io_context("bad input", std::io::Error::other(e)))?;
        if let Some(s) = self.runtime.terminals.get(&req.terminal_id).await {
            s.write_input(&bytes)?;
        }
        Ok(())
    }

    pub async fn resize(&self, req: ResizeTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        if let Some(s) = self.runtime.terminals.get(&req.terminal_id).await {
            let _ = s.resize(req.cols, req.rows);
        }
        Ok(())
    }

    pub async fn close(&self, req: CloseTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        if let Some(s) = self.runtime.terminals.remove(&req.terminal_id).await {
            s.kill();
        }
        Ok(())
    }

    pub async fn open_cli(&self, req: OpenCliTerminalRequest) -> Result<TerminalAttachment> {
        ensure_running(self.runtime)?;
        let bin = self.runtime.binaries.require(BinaryKind::Hyprlayer).await?;
        let path = self.runtime.binaries.augmented_path().await;
        let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
        env.extend(self.runtime.environment.overlay_env());
        let session = PtySession::spawn(SpawnTerminal {
            shell: bin,
            args: req.args,
            cwd: self.home(),
            cols: req.cols,
            rows: req.rows,
            env,
            workstream_id: "onboarding".into(),
        })
        .map_err(|e| io_context("open pty", e))?;
        Ok(self.register(session).await)
    }

    pub async fn open_shell(&self, req: OpenShellTerminalRequest) -> Result<TerminalAttachment> {
        ensure_running(self.runtime)?;
        let path = self.runtime.binaries.augmented_path().await;
        let args = login_shell_args(&req.command);
        let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
        env.extend(self.runtime.environment.overlay_env());
        let session = PtySession::spawn(SpawnTerminal {
            shell: default_shell(),
            args,
            cwd: self.home(),
            cols: req.cols,
            rows: req.rows,
            env,
            workstream_id: "harness".into(),
        })
        .map_err(|e| io_context("open pty", e))?;
        Ok(self.register(session).await)
    }

    pub async fn open_agent(&self, req: OpenAgentTerminalRequest) -> Result<TerminalAttachment> {
        ensure_running(self.runtime)?;
        if req.command.trim().is_empty() {
            return Err(CoreError::TerminalCommandEmpty);
        }
        let ws = self.find_workstream(&req.workstream_id).await?;
        let path = self.runtime.binaries.augmented_path().await;
        let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
        env.extend(self.runtime.environment.overlay_env());
        let session = PtySession::spawn(SpawnTerminal {
            shell: default_shell(),
            args: login_shell_args(&agent_command_line(&req.command, &req.args)),
            cwd: PathBuf::from(&ws.worktree_path),
            cols: req.cols,
            rows: req.rows,
            env,
            workstream_id: ws.id.clone(),
        })
        .map_err(|e| io_context("open agent pty", e))?;
        Ok(self.register(session).await)
    }

    pub async fn resize_agent(&self, req: ResizeAgentTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        let Some(session) = self.runtime.terminals.get(&req.terminal_id).await else {
            return Ok(());
        };
        let nudge = req.rows.saturating_sub(1).max(2);
        if nudge != req.rows {
            let _ = session.resize(req.cols, nudge);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        let _ = session.resize(req.cols, req.rows);
        Ok(())
    }

    pub async fn capture_session(&self, req: CaptureSessionRequest) -> Result<CapturedSession> {
        ensure_running(self.runtime)?;
        let worktree = self
            .find_workstream_optional(&req.workstream_id)
            .await?
            .map(|ws| ws.worktree_path);
        let home = self.home();
        let CaptureSessionRequest {
            engine_id,
            after_ms,
            exclude,
            ..
        } = req;
        blocking(move || {
            Ok(match engine_id.as_str() {
                "codex" => capture_codex(&home, worktree.as_deref(), after_ms, &exclude),
                "antigravity" => capture_antigravity(&home, after_ms, &exclude),
                _ => None,
            })
        })
        .await
    }

    pub async fn mint_codex_session(
        &self,
        req: MintCodexSessionRequest,
    ) -> Result<CapturedSession> {
        ensure_running(self.runtime)?;
        let ws = self.find_workstream(&req.workstream_id).await?;
        let path = self.runtime.binaries.augmented_path().await;
        let worktree = PathBuf::from(&ws.worktree_path);
        let path = path.to_string_lossy().into_owned();
        let command = req.command;
        let id = tokio::time::timeout(
            MINT_TIMEOUT,
            tokio::task::spawn_blocking(move || mint_codex_blocking(&command, &worktree, &path)),
        )
        .await
        .ok()
        .and_then(std::result::Result::ok)
        .flatten();
        Ok(id)
    }

    pub async fn session_exists(&self, req: SessionExistsRequest) -> Result<bool> {
        ensure_running(self.runtime)?;
        let worktree = self
            .find_workstream_optional(&req.workstream_id)
            .await?
            .map(|ws| ws.worktree_path);
        let home = self.home();
        let engine_id = req.engine_id;
        let session_id = req.session_id;
        blocking(move || {
            Ok(match engine_id.as_str() {
                "claude-code" => claude_session_exists(&home, worktree.as_deref(), &session_id),
                "codex" => codex_rollout_exists(&home, &session_id),
                "antigravity" => antigravity_brain_exists(&home, &session_id),
                _ => false,
            })
        })
        .await
    }
}

fn spawn_forward(
    session: Arc<PtySession>,
    terminal_id: String,
    registry: Arc<TerminalRegistry>,
) -> mpsc::Receiver<TerminalEvent> {
    let (tx, rx) = mpsc::channel(ATTACH_CHANNEL_CAPACITY);
    let mut live = session.subscribe();
    tokio::spawn(async move {
        while let Some(ev) = live.recv().await {
            let exited = matches!(ev, TerminalEvent::Exited { .. });
            let closed = tx.send(ev).await.is_err();
            if exited {
                registry.remove(&terminal_id).await;
                break;
            }
            if closed {
                break;
            }
        }
    });
    rx
}

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

#[cfg(not(windows))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn mtime_ms(path: &Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as f64,
    )
}

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

fn capture_codex(
    home: &Path,
    worktree: Option<&str>,
    after_ms: f64,
    exclude: &[String],
) -> Option<String> {
    let root = home.join(".codex").join("sessions");
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

fn mint_codex_blocking(command: &str, worktree: &Path, path: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(command);
    crate::process::appimage::scrub_std(&mut cmd);
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
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
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

fn claude_session_exists(home: &Path, worktree: Option<&str>, session_id: &str) -> bool {
    let Some(worktree) = worktree else {
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

fn codex_rollout_exists(home: &Path, session_id: &str) -> bool {
    let root = home.join(".codex").join("sessions");
    recent_jsonl(&root, 4, 0.0).iter().any(|(p, _)| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(session_id))
    })
}

fn antigravity_brain_exists(home: &Path, session_id: &str) -> bool {
    home.join(".gemini")
        .join("antigravity-cli")
        .join("brain")
        .join(session_id)
        .is_dir()
}

fn capture_antigravity(home: &Path, after_ms: f64, exclude: &[String]) -> Option<String> {
    let root = home.join(".gemini").join("antigravity-cli").join("brain");
    let cutoff = after_ms - CAPTURE_SKEW_MS;
    let mut best: Option<(f64, String)> = None;
    let Ok(rd) = std::fs::read_dir(&root) else {
        return None;
    };
    for entry in rd.flatten() {
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

#[cfg(not(windows))]
fn resolve_shell(from_env: Option<PathBuf>) -> PathBuf {
    from_env.unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

#[cfg(not(windows))]
pub fn default_shell() -> PathBuf {
    resolve_shell(std::env::var_os("SHELL").map(PathBuf::from))
}

#[cfg(windows)]
pub fn default_shell() -> PathBuf {
    which::which("pwsh")
        .or_else(|_| which::which("powershell"))
        .ok()
        .or_else(|| std::env::var_os("ComSpec").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

pub fn login_shell_args(command: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_shell_prefers_the_supplied_env_value() {
        assert_eq!(
            resolve_shell(Some(PathBuf::from("/usr/bin/zsh"))),
            PathBuf::from("/usr/bin/zsh")
        );
    }

    #[test]
    fn resolve_shell_falls_back_to_bin_sh() {
        assert_eq!(resolve_shell(None), PathBuf::from("/bin/sh"));
    }

    #[test]
    fn login_shell_args_wraps_the_command_for_a_login_shell() {
        assert_eq!(
            login_shell_args("echo hi"),
            vec!["-lc".to_string(), "echo hi".to_string()]
        );
    }

    #[test]
    fn agent_command_line_quotes_command_and_args() {
        let line = agent_command_line("claude", &["--flag".to_string(), "it's ok".to_string()]);
        assert_eq!(line, "exec 'claude' '--flag' 'it'\\''s ok'");
    }

    fn write_codex_session(dir: &Path, name: &str, session_id: &str, cwd: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(r#"{{"payload":{{"session_id":"{session_id}","cwd":"{cwd}"}}}}"#),
        )
        .unwrap();
        path
    }

    #[test]
    fn codex_session_meta_reads_the_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = write_codex_session(dir.path(), "a.jsonl", "sess-1", "/repo/work");
        assert_eq!(
            codex_session_meta(&file),
            Some(("sess-1".to_string(), "/repo/work".to_string()))
        );
    }

    #[test]
    fn codex_session_meta_rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.jsonl");
        std::fs::write(&file, "not json\n").unwrap();
        assert_eq!(codex_session_meta(&file), None);
    }

    #[test]
    fn capture_codex_picks_the_most_recent_matching_session() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let older = write_codex_session(&sessions, "older.jsonl", "sess-old", "/repo/work");
        let anchor = mtime_ms(&older).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        write_codex_session(&sessions, "newer.jsonl", "sess-new", "/repo/work");
        assert_eq!(
            capture_codex(home.path(), Some("/repo/work"), anchor, &[]),
            Some("sess-new".to_string())
        );
    }

    #[test]
    fn capture_codex_filters_by_worktree_and_exclude_list() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let matching = write_codex_session(&sessions, "match.jsonl", "sess-match", "/repo/a");
        write_codex_session(&sessions, "other.jsonl", "sess-other", "/repo/b");
        let anchor = mtime_ms(&matching).unwrap();

        assert_eq!(
            capture_codex(home.path(), Some("/repo/a"), anchor, &[]),
            Some("sess-match".to_string())
        );
        assert_eq!(
            capture_codex(
                home.path(),
                Some("/repo/a"),
                anchor,
                &["sess-match".to_string()]
            ),
            None
        );
    }

    #[test]
    fn capture_codex_excludes_sessions_older_than_the_skew_window() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let file = write_codex_session(&sessions, "stale.jsonl", "sess-stale", "/repo/work");
        let too_late = mtime_ms(&file).unwrap() + CAPTURE_SKEW_MS + 1_000.0;
        assert_eq!(
            capture_codex(home.path(), Some("/repo/work"), too_late, &[]),
            None
        );
    }

    #[test]
    fn capture_antigravity_picks_the_most_recent_brain_dir() {
        let home = tempfile::tempdir().unwrap();
        let brain = home
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain");
        std::fs::create_dir_all(brain.join("older")).unwrap();
        let anchor = mtime_ms(&brain.join("older")).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::create_dir_all(brain.join("newer")).unwrap();
        std::fs::create_dir_all(brain.join(".hidden")).unwrap();
        assert_eq!(
            capture_antigravity(home.path(), anchor, &[]),
            Some("newer".to_string())
        );
    }

    #[test]
    fn capture_antigravity_skips_excluded_and_hidden_entries() {
        let home = tempfile::tempdir().unwrap();
        let brain = home
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain");
        std::fs::create_dir_all(brain.join("only")).unwrap();
        let anchor = mtime_ms(&brain.join("only")).unwrap();
        std::fs::create_dir_all(brain.join(".hidden")).unwrap();
        assert_eq!(
            capture_antigravity(home.path(), anchor, &["only".to_string()]),
            None
        );
    }

    #[test]
    fn claude_session_exists_matches_a_slugged_worktree_path() {
        let home = tempfile::tempdir().unwrap();
        let worktree = "/home/user/code/my-repo";
        let slug: String = worktree
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = home.path().join(".claude").join("projects").join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-1.jsonl"), "").unwrap();

        assert!(claude_session_exists(home.path(), Some(worktree), "sess-1"));
        assert!(!claude_session_exists(
            home.path(),
            Some(worktree),
            "sess-2"
        ));
        assert!(!claude_session_exists(home.path(), None, "sess-1"));
    }

    #[test]
    fn codex_rollout_exists_matches_by_filename_substring() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("rollout-sess-abc123.jsonl"), "{}\n").unwrap();

        assert!(codex_rollout_exists(home.path(), "sess-abc123"));
        assert!(!codex_rollout_exists(home.path(), "sess-missing"));
    }

    #[test]
    fn antigravity_brain_exists_checks_for_the_session_directory() {
        let home = tempfile::tempdir().unwrap();
        let brain = home
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain");
        std::fs::create_dir_all(brain.join("sess-1")).unwrap();

        assert!(antigravity_brain_exists(home.path(), "sess-1"));
        assert!(!antigravity_brain_exists(home.path(), "sess-2"));
    }
}
