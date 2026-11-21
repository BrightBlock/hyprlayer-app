use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use hyprlayer_models::RuntimeEvent;

use crate::error::{CoreError, Result};
use crate::process::Exec;
use crate::repo_settings::{self, ScriptSettings};
use crate::scripts::{self, ScriptKind};
use crate::store::Workstream;
use crate::terminal::{PtySession, RingReader, SpawnTerminal, TerminalChunk};

use super::script_registry::{RunPhase, ScriptRegistry, ScriptRuntime};
use super::terminals::{default_shell, login_shell_args};
use super::HyprlayerRuntime;

const SETUP_TIMEOUT_SECS: u64 = 600;
const ARCHIVE_TIMEOUT_SECS: u64 = 300;

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

pub struct ScriptSettingsRequest {
    pub project_path: String,
    pub set: Option<ScriptSettings>,
}

pub struct RunScriptRequest {
    pub workstream_id: String,
    pub kind: ScriptKind,
    pub cols: u16,
    pub rows: u16,
}

pub struct ScriptRun {
    pub terminal_id: String,
    pub live: RingReader,
}

pub struct StopScriptRequest {
    pub workstream_id: String,
}

pub struct ScriptOutputRequest {
    pub workstream_id: String,
    pub kind: ScriptKind,
    pub clear: bool,
    pub terminal_id: Option<String>,
}

pub type ScriptOutput = Option<String>;

pub struct ScriptRuntimeRequest {
    pub workstream_id: String,
}

pub struct RunLifecycleRequest {
    pub workstream: Workstream,
    pub kind: ScriptKind,
}

/// `None` means no script is configured for the requested kind (the same
/// "skip" contract `runtime::ports::LifecycleRunner` documents).
pub type LifecycleResult = Option<RunPhase>;

pub struct ScriptService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn scripts(&self) -> ScriptService<'_> {
        ScriptService { runtime: self }
    }
}

impl ScriptService<'_> {
    async fn find_workstream(&self, id: &str) -> Result<Workstream> {
        let owned = id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_workstream(&owned))
            .await?
            .ok_or_else(|| CoreError::WorkstreamNotFound { id: id.to_string() })
    }

    async fn read_settings(&self, project_path: &str) -> Result<ScriptSettings> {
        let repo = PathBuf::from(project_path);
        let settings = blocking(move || repo_settings::read(&repo)).await?;
        Ok(settings.scripts)
    }

    async fn resolve_script(&self, project_path: &str, kind: ScriptKind) -> Result<String> {
        let settings = self.read_settings(project_path).await?;
        Ok(match kind {
            ScriptKind::Setup => settings.setup,
            ScriptKind::Run => settings.run,
            ScriptKind::Archive => settings.archive,
        })
    }

    /// Reads a project's script settings, or writes them when `req.set` is
    /// `Some` — folded into one method because the supported facade names
    /// exactly one `settings` verb for `ScriptService`, unlike the separate
    /// `get`/`set`/`update` verbs `SettingsService` exposes.
    pub async fn settings(&self, req: ScriptSettingsRequest) -> Result<ScriptSettings> {
        ensure_running(self.runtime)?;
        let ScriptSettingsRequest { project_path, set } = req;
        if let Some(update) = set {
            let repo = PathBuf::from(&project_path);
            let to_write = update.clone();
            blocking(move || repo_settings::set_scripts(&repo, &to_write)).await?;
            return Ok(update);
        }
        self.read_settings(&project_path).await
    }

    pub async fn run(&self, req: RunScriptRequest) -> Result<ScriptRun> {
        ensure_running(self.runtime)?;
        let RunScriptRequest {
            workstream_id,
            kind,
            cols,
            rows,
        } = req;
        let item = self.find_workstream(&workstream_id).await?;
        let commands = self.resolve_script(&item.project_path, kind).await?;
        if commands.trim().is_empty() {
            return Err(CoreError::ScriptEmpty { kind });
        }

        if kind == ScriptKind::Run {
            self.runtime.scripts.stop_run(&workstream_id).await;
        }

        let port = self.runtime.scripts.take_port(&item.id).await;
        let augmented = self.runtime.binaries.augmented_path().await;
        let env = script_env(
            &item,
            port,
            &augmented,
            self.runtime.environment.overlay_env(),
        );
        let args = login_shell_args(&scripts::build_block(&commands));

        let session = PtySession::spawn(SpawnTerminal {
            shell: default_shell(),
            args,
            cwd: PathBuf::from(&item.worktree_path),
            cols,
            rows,
            env,
            workstream_id: item.id.clone(),
        })
        .map_err(|e| io_context("spawn script pty", e))?;

        let terminal_id = uuid::Uuid::new_v4().to_string();
        self.runtime
            .terminals
            .insert(terminal_id.clone(), session.clone())
            .await;

        self.runtime
            .scripts
            .set_status(&workstream_id, kind, RunPhase::Running, None)
            .await;
        if kind == ScriptKind::Run {
            self.runtime
                .scripts
                .register_run(&workstream_id, terminal_id.clone(), session.clone())
                .await;
        }
        emit_changed(
            &self.runtime.events,
            &workstream_id,
            kind,
            RunPhase::Running,
            None,
            None,
        );

        spawn_monitor(
            self.runtime.events.clone(),
            self.runtime.scripts.clone(),
            session.clone(),
            workstream_id.clone(),
            kind,
            terminal_id.clone(),
        );

        let live = session.subscribe();
        Ok(ScriptRun { terminal_id, live })
    }

    pub async fn stop(&self, req: StopScriptRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        self.runtime.scripts.stop_run(&req.workstream_id).await;
        Ok(())
    }

    /// Reads a workstream's captured output for one script kind, or clears it
    /// (and the associated terminal's scrollback) when `req.clear` is `true`
    /// — folded for the same reason `settings` is: the facade names one
    /// `output` verb.
    pub async fn output(&self, req: ScriptOutputRequest) -> Result<ScriptOutput> {
        ensure_running(self.runtime)?;
        let ScriptOutputRequest {
            workstream_id,
            kind,
            clear,
            terminal_id,
        } = req;
        if clear {
            self.runtime
                .scripts
                .clear_output(&workstream_id, kind)
                .await;
            if let Some(id) = terminal_id {
                if let Some(session) = self.runtime.terminals.get(&id).await {
                    session.clear_scrollback();
                }
            }
            return Ok(None);
        }
        Ok(self.runtime.scripts.get_output(&workstream_id, kind).await)
    }

    pub async fn runtime(&self, req: ScriptRuntimeRequest) -> Result<ScriptRuntime> {
        ensure_running(self.runtime)?;
        Ok(self.runtime.scripts.get_runtime(&req.workstream_id).await)
    }

    /// Workstream ids with a Run script alive right now.
    pub async fn running_ids(&self) -> Result<Vec<String>> {
        ensure_running(self.runtime)?;
        Ok(self.runtime.scripts.running_ids().await)
    }

    /// Runs a workstream's configured Setup/Archive command line to
    /// completion and reports pass/fail, capturing combined stdout/stderr as
    /// the script's output. Never returns `Err` in practice — an unreadable
    /// or empty script settings file degrades to `Ok(None)` with a warning
    /// logged, the same "skip" contract the desktop's script runner always
    /// had — so `runtime::ports::LifecycleRunner` implementations can treat
    /// this as infallible.
    pub async fn run_lifecycle(&self, req: RunLifecycleRequest) -> Result<LifecycleResult> {
        ensure_running(self.runtime)?;
        let RunLifecycleRequest {
            workstream: item,
            kind,
        } = req;

        let commands = match self.resolve_script(&item.project_path, kind).await {
            Ok(c) if !c.trim().is_empty() => c,
            Ok(_) => return Ok(None),
            Err(e) => {
                tracing::warn!("read {} script: {}", kind.as_str(), e);
                return Ok(None);
            }
        };

        let port = self.runtime.scripts.take_port(&item.id).await;
        let augmented = self.runtime.binaries.augmented_path().await;
        let args = login_shell_args(&scripts::build_block(&commands));
        let timeout = match kind {
            ScriptKind::Archive => ARCHIVE_TIMEOUT_SECS,
            _ => SETUP_TIMEOUT_SECS,
        };

        self.runtime
            .scripts
            .set_status(&item.id, kind, RunPhase::Running, None)
            .await;
        emit_changed(
            &self.runtime.events,
            &item.id,
            kind,
            RunPhase::Running,
            None,
            None,
        );

        let mut exec = Exec::new(default_shell())
            .args(args)
            .cwd(item.worktree_path.clone())
            .own_process_group()
            .timeout(Duration::from_secs(timeout));
        let overlay = self.runtime.environment.overlay_env();
        for (key, value) in script_env(&item, port, &augmented, overlay) {
            exec = exec.env(key, value);
        }

        let started = std::time::Instant::now();
        let (phase, code, output) = match exec.run_unchecked().await {
            Ok(out) => {
                let phase = if out.exit_code == Some(0) {
                    tracing::info!(
                        "{} script for workstream {} passed in {:?}",
                        kind.as_str(),
                        item.id,
                        started.elapsed()
                    );
                    RunPhase::Passed
                } else {
                    // The full output is kept for the UI; the log gets its end.
                    tracing::warn!(
                        "{} script for workstream {} exited {:?} after {:?}: {}",
                        kind.as_str(),
                        item.id,
                        out.exit_code,
                        started.elapsed(),
                        crate::redact::tail(out.stderr.trim(), 1000)
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
                tracing::warn!(
                    "{} script for workstream {} failed to run after {:?}: {e}",
                    kind.as_str(),
                    item.id,
                    started.elapsed()
                );
                (RunPhase::Failed, None, format!("{e}"))
            }
        };

        self.runtime
            .scripts
            .set_status(&item.id, kind, phase, code)
            .await;
        self.runtime
            .scripts
            .set_output(&item.id, kind, output)
            .await;
        emit_changed(&self.runtime.events, &item.id, kind, phase, code, None);
        Ok(Some(phase))
    }
}

pub(crate) fn script_env(
    item: &Workstream,
    port: u16,
    augmented: &OsStr,
    overlay: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("PATH".to_string(), augmented.to_string_lossy().into_owned()),
        (scripts::ENV_PORT.to_string(), port.to_string()),
        (
            scripts::ENV_WORKTREE.to_string(),
            item.worktree_path.clone(),
        ),
        (scripts::ENV_BRANCH.to_string(), item.branch.clone()),
        (scripts::ENV_PROJECT.to_string(), item.project_path.clone()),
        (scripts::ENV_SLUG.to_string(), item.slug.clone()),
    ];
    // A repo script pushing or pulling needs the client's agent socket, same as
    // an agent does — the server process has none of its own (ADR-0009 §7).
    env.extend(overlay);
    env
}

fn emit_changed(
    events: &broadcast::Sender<RuntimeEvent>,
    workstream_id: &str,
    kind: ScriptKind,
    phase: RunPhase,
    exit_code: Option<i32>,
    port: Option<u16>,
) {
    let _ = events.send(RuntimeEvent::ScriptRunChanged {
        workstream_id: workstream_id.to_string(),
        kind,
        phase,
        exit_code,
        port,
    });
}

fn spawn_monitor(
    events: broadcast::Sender<RuntimeEvent>,
    scripts: Arc<ScriptRegistry>,
    session: Arc<PtySession>,
    workstream_id: String,
    kind: ScriptKind,
    terminal_id: String,
) {
    let mut rx = session.subscribe();
    tokio::spawn(async move {
        let mut port_found = false;
        while let Some(chunk) = rx.recv().await {
            match chunk {
                TerminalChunk::Output { bytes, .. } => {
                    if kind == ScriptKind::Run && !port_found {
                        if let Some(port) = scan_for_port(&bytes) {
                            port_found = true;
                            scripts
                                .set_run_port(&workstream_id, &terminal_id, port)
                                .await;
                            emit_changed(
                                &events,
                                &workstream_id,
                                kind,
                                RunPhase::Running,
                                None,
                                Some(port),
                            );
                        }
                    }
                }
                TerminalChunk::Resized { .. } => {}
                TerminalChunk::Exited { code } => {
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
                    emit_changed(&events, &workstream_id, kind, phase, code, None);
                    break;
                }
            }
        }
    });
}

fn scan_for_port(bytes: &[u8]) -> Option<u16> {
    String::from_utf8_lossy(bytes)
        .lines()
        .find_map(scripts::detect_port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_for_port_detects_the_port() {
        let data = "  \x1b[32m➜\x1b[39m  Local: \x1b[36mhttp://localhost:5174/\x1b[39m";
        assert_eq!(scan_for_port(data.as_bytes()), Some(5174));
    }

    #[test]
    fn scan_for_port_returns_none_when_no_url_is_present() {
        assert_eq!(scan_for_port(b"installing dependencies...\n"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_monitor_detects_the_port_and_records_the_exit_phase() {
        let (events, mut rx) = broadcast::channel(16);
        let scripts = Arc::new(ScriptRegistry::new());
        let session = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "echo 'Local: http://localhost:5174/'".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "ws-monitor".into(),
        })
        .expect("spawn pty");
        scripts
            .register_run("ws-monitor", "tid".into(), session.clone())
            .await;

        spawn_monitor(
            events,
            scripts.clone(),
            session,
            "ws-monitor".into(),
            ScriptKind::Run,
            "tid".into(),
        );

        let mut port = None;
        let mut exit = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while exit.is_none() {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, rx.recv()).await {
                Ok(Ok(RuntimeEvent::ScriptRunChanged {
                    kind: ScriptKind::Run,
                    phase: RunPhase::Running,
                    port: Some(p),
                    ..
                })) => port = Some(p),
                Ok(Ok(RuntimeEvent::ScriptRunChanged {
                    kind: ScriptKind::Run,
                    phase,
                    exit_code,
                    ..
                })) => exit = Some((phase, exit_code)),
                other => panic!("monitor never reported the exit: {other:?}"),
            }
        }

        assert_eq!(port, Some(5174), "port badge was not detected");
        assert_eq!(exit, Some((RunPhase::Passed, Some(0))));
        assert!(scripts.live_run("ws-monitor").await.is_none());
        let output = scripts
            .get_output("ws-monitor", ScriptKind::Run)
            .await
            .expect("exit captured the output");
        assert!(output.contains("localhost:5174"), "got: {output:?}");
    }

    #[tokio::test]
    async fn captured_output_is_capped_and_keeps_the_most_recent_tail() {
        let registry = ScriptRegistry::new();
        let head = "H".repeat(64);
        let filler = "x".repeat(100_000);
        let tail = "END-OF-OUTPUT";
        let output = format!("{head}{filler}{tail}");

        registry
            .set_output("ws-cap", ScriptKind::Run, output.clone())
            .await;
        let capped = registry
            .get_output("ws-cap", ScriptKind::Run)
            .await
            .expect("output was recorded");

        assert!(
            capped.len() < output.len(),
            "very large output should be capped"
        );
        assert!(
            capped.ends_with(tail),
            "capped output should keep the most recent tail"
        );
        assert!(
            !capped.starts_with(&head),
            "capped output should drop the oldest content"
        );
    }
}
