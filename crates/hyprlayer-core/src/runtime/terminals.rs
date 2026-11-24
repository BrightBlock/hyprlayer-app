use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use hyprlayer_models::{
    AgentCaller, PermissionMode, PtyAgent, PtyAgentState, PtyLaunchSession, PtyLaunched,
    TerminalTheme,
};

use crate::claude::driver::permission_mode_flag;
use crate::error::{CoreError, Result};
use crate::pipeline::{self, Stage};
use crate::process::binaries::{parse_semver, BinaryKind};
use crate::process::Exec;
use crate::settings::Engine;
use crate::store::{PtyAgentRow, Workstream};
use crate::terminal::engines;
use crate::terminal::{PtySession, RingReader, SpawnTerminal};

#[cfg(unix)]
use super::agent_control::codex_remote;
use super::agent_control::codex_remote::CodexRemote;
use super::agent_control::pty_agents::{is_terminal_reply, CodexGate, PtyLaunchEntry};
use super::agent_control::{inject, resume, spawn, AgentIdentity};
use super::HyprlayerRuntime;

type CodexGateCell = tokio::sync::OnceCell<CodexGate>;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;
const CAPTURE_SKEW_MS: f64 = 2000.0;
const MINT_TIMEOUT: Duration = Duration::from_secs(90);
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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

/// `live` reads the ring only when its holder asks, so a slow one leaves output there to coalesce.
pub struct TerminalAttachment {
    pub terminal_id: String,
    pub live: RingReader,
}

pub struct AttachedTerminal {
    pub live: RingReader,
    /// The ring held the requested offset at open; a trim can still beat the first read, so output offsets decide.
    pub resumed: bool,
}

pub struct OpenTerminalRequest {
    pub workstream_id: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct AttachTerminalRequest {
    pub terminal_id: String,
    pub offset: Option<u64>,
}

/// `viewer` is the client the call came from; `None` in process.
pub struct WriteTerminalRequest {
    pub terminal_id: String,
    pub data: String,
    pub viewer: Option<u64>,
}

pub struct ResizeTerminalRequest {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
    pub viewer: Option<u64>,
}

pub struct FocusTerminalRequest {
    pub terminal_id: String,
    pub viewer: u64,
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

/// A PTY agent the server builds argv for, from the engine table and settings.
pub struct LaunchAgentRequest {
    pub workstream_id: String,
    pub engine_id: String,
    pub session: PtyLaunchSession,
    pub cols: u16,
    pub rows: u16,
    pub lineage: Option<PtyLineage>,
    /// A stored agent to resume in place, under its own terminal id.
    pub resume_of: Option<PtyAgentRow>,
}

/// The agent that asked for a PTY launch; the desktop's own launches have none.
pub struct PtyLineage {
    pub parent: AgentCaller,
    pub depth: u32,
    pub mode: PermissionMode,
}

pub struct ResizeAgentTerminalRequest {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
    pub viewer: Option<u64>,
    pub frames: bool,
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
        self.register_with_id(uuid::Uuid::new_v4().to_string(), session)
            .await
    }

    async fn register_with_id(
        &self,
        terminal_id: String,
        session: Arc<PtySession>,
    ) -> TerminalAttachment {
        self.runtime
            .terminals
            .insert(terminal_id.clone(), session.clone())
            .await;
        let live = session.subscribe();
        TerminalAttachment { terminal_id, live }
    }

    pub async fn open(&self, req: OpenTerminalRequest) -> Result<TerminalAttachment> {
        ensure_running(self.runtime)?;
        let (ws, _held) = self
            .runtime
            .workstreams()
            .hold_unarchived(&req.workstream_id)
            .await?;
        let env = self.runtime.child_env().await;
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

    pub async fn attach(&self, req: AttachTerminalRequest) -> Result<Option<AttachedTerminal>> {
        ensure_running(self.runtime)?;
        let Some(session) = self.runtime.terminals.get(&req.terminal_id).await else {
            return Ok(None);
        };
        let (live, resumed) = match req.offset {
            Some(offset) => session.resume(offset),
            None => (session.subscribe(), false),
        };
        Ok(Some(AttachedTerminal { live, resumed }))
    }

    pub async fn write(&self, req: WriteTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        let bytes = B64
            .decode(req.data)
            .map_err(|e| io_context("bad input", std::io::Error::other(e)))?;
        if let Some(s) = self.runtime.terminals.get(&req.terminal_id).await {
            let typed = !is_terminal_reply(&bytes);
            if let Some(viewer) = req.viewer.filter(|_| typed) {
                s.claim(viewer);
            }
            s.write_input(&bytes)?;
            if typed {
                self.runtime.pty_agents.note_input(&req.terminal_id);
                self.runtime
                    .agent_control()
                    .note_human_turn(&req.terminal_id);
            }
        }
        Ok(())
    }

    pub async fn resize(&self, req: ResizeTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        if self.runtime.pty_agents.snapshot(&req.terminal_id).is_some() {
            self.runtime.terminals.note_agent_fit(req.cols, req.rows);
        }
        if let Some(s) = self.runtime.terminals.get(&req.terminal_id).await {
            s.fit(req.viewer, req.cols, req.rows);
        }
        Ok(())
    }

    /// The viewer's person is at this terminal, so it takes their fit.
    pub async fn focus(&self, req: FocusTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        if let Some(s) = self.runtime.terminals.get(&req.terminal_id).await {
            s.claim(req.viewer);
        }
        Ok(())
    }

    pub async fn take_size(&self, req: FocusTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        if let Some(s) = self.runtime.terminals.get(&req.terminal_id).await {
            s.take_size(req.viewer);
        }
        Ok(())
    }

    /// A terminal this viewer joins later takes the theme from its seat.
    pub async fn set_theme(&self, viewer: u64, theme: TerminalTheme) {
        for session in self.runtime.terminals.all().await {
            session.set_theme(viewer, theme);
        }
    }

    pub async fn close(&self, req: CloseTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        let _resume = self
            .runtime
            .pty_resume_locks
            .acquire(&req.terminal_id)
            .await;
        self.end_agent(&req.terminal_id).await
    }

    /// Kills it and drops its row, so no server resumes it; the caller holds its resume lock.
    async fn end_agent(&self, terminal_id: &str) -> Result<()> {
        if let Some(s) = self.runtime.terminals.remove(terminal_id).await {
            s.kill();
        }
        // Not left to the exit watcher: a PTY that never started or never reports its exit would stay listed.
        let agents = self.runtime.pty_agents.clone();
        agents.release(terminal_id);
        agents.forget(terminal_id);
        blocking(move || {
            agents.flush_store();
            Ok(())
        })
        .await
    }

    /// A launch resuming a conversation a stored agent holds replaces that agent, so two never
    /// share it; `true` when the replaced one still owed its restart nudge, which the launch takes.
    async fn supersede(&self, ws: &Workstream, engine_id: &str, session_id: &str) -> Result<bool> {
        let db = self.runtime.db.clone();
        let (ws_id, engine, sid) = (ws.id.clone(), engine_id.to_string(), session_id.to_string());
        let Some(row) = blocking(move || db.pty_agent_holding(&ws_id, &engine, &sid)).await? else {
            return Ok(false);
        };
        let _resume = self
            .runtime
            .pty_resume_locks
            .acquire(&row.terminal_id)
            .await;
        let live = self.runtime.pty_agents.snapshot(&row.terminal_id).is_some();
        tracing::info!(
            "terminal {}: replaced by a new launch resuming its conversation",
            row.terminal_id
        );
        self.end_agent(&row.terminal_id).await?;
        Ok(row.working && (!live || row.resumes > 0))
    }

    pub async fn open_cli(&self, req: OpenCliTerminalRequest) -> Result<TerminalAttachment> {
        ensure_running(self.runtime)?;
        let bin = self.runtime.binaries.require(BinaryKind::Hyprlayer).await?;
        let env = self.runtime.child_env().await;
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
        let args = login_shell_args(&req.command);
        let env = self.runtime.child_env().await;
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
        let (ws, _held) = self
            .runtime
            .workstreams()
            .hold_unarchived(&req.workstream_id)
            .await?;
        let env = self.runtime.child_env().await;
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

    /// Spawns `[...injected hooks and MCP, ...engine.args, ...session args, ...bypass]` with the
    /// agent's identity env, and tracks it in the PTY agent registry until it exits. A hooked codex
    /// that can runs its TUI over an app-server of its own, which carries the hooks and MCP instead.
    pub async fn launch_agent(
        &self,
        req: LaunchAgentRequest,
    ) -> Result<(TerminalAttachment, PtyLaunched)> {
        ensure_running(self.runtime)?;
        let (ws, _held) = self
            .runtime
            .workstreams()
            .hold_unarchived(&req.workstream_id)
            .await?;
        let configured = self.runtime.settings.get().await.multiplexer.engines;
        let engine = engines::resolve(&configured, &req.engine_id);
        if engine.command.trim().is_empty() {
            return Err(CoreError::TerminalCommandEmpty);
        }
        let resume_of = req.resume_of;
        let takes_nudge = match req.session.session_id.as_deref() {
            Some(sid) if req.session.resume && resume_of.is_none() => {
                self.supersede(&ws, &engine.id, sid).await?
            }
            _ => false,
        };
        let mode = match &req.lineage {
            Some(lineage) => lineage.mode,
            None if req.session.bypass => PermissionMode::BypassPermissions,
            None => PermissionMode::Default,
        };
        let terminal_id = resume_of.as_ref().map_or_else(
            || uuid::Uuid::new_v4().to_string(),
            |row| row.terminal_id.clone(),
        );
        let identity = self
            .runtime
            .agent_control()
            .identity_env(AgentIdentity {
                caller: AgentCaller::Terminal {
                    terminal_id: terminal_id.clone(),
                },
                workstream_id: ws.id.clone(),
                project_path: ws.project_path.clone(),
                depth: req.lineage.as_ref().map_or(0, |l| l.depth),
                parent: req.lineage.as_ref().map(|l| l.parent.clone()),
                launch_mode: Some(mode),
                report_only: false,
                harness: engines::harness_of(&engine.id),
                lead: false,
            })
            .await;
        let gate = CodexGateCell::new();
        let injected = match &identity {
            Some((identity_env, _)) => {
                self.injected_args(&engine, &terminal_id, &ws, identity_env, mode, &gate)
                    .await
            }
            None => None,
        };
        let hooked = injected.is_some();
        let hooked_codex = hooked && engine.id == "codex";

        let mut session = req.session;
        let first_prompt = session.first_prompt.take();
        let claude_stage = first_prompt
            .as_ref()
            .filter(|_| engine.id == "claude-code")
            .map(|fp| fp.stage);
        // A claude stage turn opens in its stage's mode, as a native start does (Implement accepts edits).
        if let Some(stage) = claude_stage.filter(|_| session.permission_mode.is_none()) {
            let mode = pipeline::policy_for(stage).permission_mode;
            if mode != PermissionMode::Default {
                session.permission_mode = Some(permission_mode_flag(mode).into());
            }
        }
        if !hooked
            && engines::mints_via_exec(&engine.id)
            && !session.resume
            && session.session_id.is_none()
        {
            session.session_id = self
                .mint_codex_session(MintCodexSessionRequest {
                    workstream_id: ws.id.clone(),
                    command: engine.command.clone(),
                })
                .await?;
        }

        let launch_settings = session.clone();
        let (identity_env, token) = identity.unzip();
        let mut env = self.runtime.child_env().await;
        env.extend(identity_env.unwrap_or_default());
        #[cfg(unix)]
        let remote = match injected.as_ref().filter(|_| hooked_codex) {
            Some(config) => {
                self.start_codex_remote(&engine, &terminal_id, &ws, &env, config, &gate)
                    .await
            }
            None => None,
        };
        #[cfg(not(unix))]
        let remote: Option<Arc<CodexRemote>> = None;
        // A remote resume rejects permission flags; bypass goes through the API once the thread loads.
        let owe_bypass = remote.is_some() && session.resume && session.bypass;
        if owe_bypass {
            session.bypass = false;
        }
        let (engine_argv, provider_session_id) =
            engines::session_argv(&engine, &session, hooked_codex);
        let mut argv = match &remote {
            Some(remote) => [
                "--remote".to_string(),
                remote.url(),
                "-C".to_string(),
                ws.worktree_path.clone(),
                inject::CODEX_HOOK_TRUST.to_string(),
            ]
            .into_iter()
            .chain(inject::CODEX_REMOTE_FOOTER.map(String::from))
            .collect(),
            None => {
                let mut args = injected.unwrap_or_default();
                if hooked_codex {
                    args.push(inject::CODEX_HOOK_TRUST.to_string());
                }
                args
            }
        };
        argv.extend(engine_argv);
        if let Some(deny) = claude_stage.and_then(Self::pty_disallowed_tools) {
            argv.extend(["--disallowedTools".to_string(), deny]);
        }

        let pty = match PtySession::spawn(SpawnTerminal {
            shell: default_shell(),
            args: login_shell_args(&agent_command_line(&engine.command, &argv)),
            cwd: PathBuf::from(&ws.worktree_path),
            cols: req.cols,
            rows: req.rows,
            env,
            workstream_id: ws.id.clone(),
        }) {
            Ok(pty) => pty,
            Err(e) => {
                if let Some(remote) = &remote {
                    remote.stop();
                }
                self.runtime.pty_agents.remove_launch_dir(&terminal_id);
                return Err(io_context("open agent pty", e));
            }
        };
        let attachment = self
            .register_with_id(terminal_id.clone(), pty.clone())
            .await;
        // Before the exit watcher starts, so a fast exit cannot leave the link behind.
        let spawned_by = req.lineage.as_ref().map(|l| l.parent.clone());
        let agent_spawned = req.lineage.is_some();
        let nudge_owed = takes_nudge || resume_of.as_ref().is_some_and(|row| row.working);
        let record = hooked.then(|| match resume_of {
            Some(row) => PtyAgentRow {
                launch: launch_settings,
                ..row
            },
            None => PtyAgentRow {
                terminal_id: terminal_id.clone(),
                workstream_id: ws.id.clone(),
                engine_id: engine.id.clone(),
                launch: launch_settings,
                spawned_by: spawned_by.clone(),
                depth: req.lineage.as_ref().map_or(0, |l| l.depth),
                mode: req.lineage.as_ref().map(|l| l.mode),
                provider_session_id: provider_session_id.clone(),
                title: None,
                working: takes_nudge,
                resumes: 0,
            },
        });
        if let Some(lineage) = req.lineage {
            self.runtime
                .agent_links
                .link(&terminal_id, lineage.parent, lineage.depth);
        }
        self.runtime.pty_agents.register(PtyLaunchEntry {
            agent: PtyAgent {
                terminal_id: terminal_id.clone(),
                workstream_id: ws.id.clone(),
                engine_id: engine.id.clone(),
                state: PtyAgentState::Starting,
                provider_session_id: provider_session_id.clone(),
                activity: None,
                last_reply: None,
                spawned_by,
                hooked,
                title: None,
                status: Default::default(),
            },
            session: pty.clone(),
            token,
            remote: remote.clone(),
            record,
            nudge_owed,
        });
        if takes_nudge {
            spawn::send_first_prompt(
                self.runtime.pty_agents.clone(),
                &terminal_id,
                resume::nudge_text(),
            );
        }
        // After the spawn, so a launch that fails activates no plan.
        // An agent's spawn sends its own, once it has ruled out an unhooked TUI.
        if let Some(fp) = first_prompt.filter(|_| !agent_spawned) {
            let text = self
                .runtime
                .sessions()
                .pty_first_prompt(&ws, engines::harness_of(&engine.id), fp)
                .await;
            spawn::send_first_prompt(self.runtime.pty_agents.clone(), &terminal_id, text);
        }
        #[cfg(unix)]
        if let Some(remote) = remote {
            // It may have died before the entry existed to hear of it.
            if !remote.alive() {
                pty.kill();
            } else if session.resume {
                remote.resume(provider_session_id.clone(), owe_bypass);
                match self.runtime.pty_agents.snapshot(&terminal_id) {
                    Some(snapshot) => {
                        // Waits for the lock: skipping it would leave the dialog for typed input to answer.
                        tokio::spawn(async move {
                            let input = snapshot.input.lock_owned().await;
                            codex_remote::skip_hook_review(pty, input).await;
                        });
                    }
                    None => tracing::warn!(
                        "resumed codex {terminal_id}: gone before its hook review could be dismissed"
                    ),
                }
            }
        }
        Ok((
            attachment,
            PtyLaunched {
                terminal_id,
                provider_session_id,
                hooked,
            },
        ))
    }

    /// A stage's claude deny list, less `Skill`: a PTY ship runs `/describe_pr` itself.
    fn pty_disallowed_tools(stage: Stage) -> Option<String> {
        let deny: Vec<String> = pipeline::policy_for(stage)
            .disallowed_tools
            .into_iter()
            .filter(|t| t != "Skill")
            .collect();
        (!deny.is_empty()).then(|| deny.join(","))
    }

    /// Hooks plus the MCP server, ahead of the engine's own args; `None` leaves it on the regex detector.
    async fn injected_args(
        &self,
        engine: &Engine,
        terminal_id: &str,
        ws: &Workstream,
        identity_env: &[(String, String)],
        mode: PermissionMode,
        gate: &CodexGateCell,
    ) -> Option<Vec<String>> {
        if !matches!(engine.id.as_str(), "claude-code" | "codex") {
            return None;
        }
        let endpoint = self.runtime.agent_control_endpoint.as_ref()?;
        let ctl = &endpoint.ctl;
        if engine
            .args
            .iter()
            .any(|a| a == "--settings" || a.starts_with("--settings="))
        {
            tracing::info!(
                "{} launches unhooked: its args carry their own --settings",
                engine.id
            );
            return None;
        }
        if engine.id == "codex" {
            if !self
                .codex_gate(gate, &engine.command, Path::new(&ws.worktree_path))
                .await
                .hooks
            {
                return None;
            }
            let Some(hooks) = inject::codex_hook_args(ctl) else {
                tracing::info!(
                    "codex launches unhooked: {} cannot run unquoted in a hook command",
                    ctl.display()
                );
                return None;
            };
            let read_only = mode != PermissionMode::BypassPermissions;
            let mut args =
                inject::codex_mcp_args(ctl, identity_env, read_only).unwrap_or_else(|| {
                    tracing::info!("codex launches without MCP: a value cannot be a TOML literal");
                    Vec::new()
                });
            args.extend(hooks);
            return Some(args);
        }
        let settings = inject::claude_settings(ctl).to_string();
        let settings = match self.runtime.pty_agents.write_launch_file(
            terminal_id,
            "settings.json",
            settings.as_bytes(),
        ) {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!("claude launches unhooked: cannot write its hook settings: {e}");
                return None;
            }
        };
        let mcp = inject::claude_mcp_config(ctl, identity_env).to_string();
        // `--mcp-config` is variadic: our single-value `--settings` must follow it.
        let mut args =
            match self
                .runtime
                .pty_agents
                .write_launch_file(terminal_id, "mcp.json", mcp.as_bytes())
            {
                Ok(path) => vec![
                    "--mcp-config".to_string(),
                    path.to_string_lossy().into_owned(),
                ],
                Err(e) => {
                    tracing::warn!("claude launches without MCP: cannot write its config: {e}");
                    Vec::new()
                }
            };
        args.extend([
            "--settings".to_string(),
            settings.to_string_lossy().into_owned(),
        ]);
        if let Some(plugin) = &endpoint.plugin_dir {
            args.extend([
                "--plugin-dir".to_string(),
                plugin.to_string_lossy().into_owned(),
            ]);
        }
        Some(args)
    }

    /// Probes the binary the launch will run, through the same login shell and cwd, once per launch.
    async fn codex_gate(&self, gate: &CodexGateCell, command: &str, cwd: &Path) -> CodexGate {
        *gate
            .get_or_init(|| self.probe_codex_gate(command, cwd))
            .await
    }

    /// `--version` every launch: a project can pin its own codex (mise, asdf), and an upgrade must
    /// show. Only the slower `--help` probes are cached, per command and version.
    async fn probe_codex_gate(&self, command: &str, cwd: &Path) -> CodexGate {
        // The first semver anywhere: a version manager's shim may print a banner first.
        let version = self
            .probe_codex(command, &["--version"], cwd)
            .await
            .and_then(|out| out.split_whitespace().find_map(parse_semver));
        // Uncached when a probe can't run, so a codex installed later is seen on the next launch.
        let unhooked = CodexGate {
            hooks: false,
            remote: false,
        };
        let Some(version) = version else {
            tracing::info!("codex launches unhooked: `{command} --version` gave no version");
            return unhooked;
        };
        let hooks = version >= inject::CODEX_HOOKS_MIN;
        if !hooks {
            tracing::info!(
                "codex launches unhooked: {}.{}.{} predates hook support",
                version[0],
                version[1],
                version[2]
            );
        }
        let pty_agents = &self.runtime.pty_agents;
        let remote = if !cfg!(unix) || !hooks {
            false
        } else if let Some(known) = pty_agents.codex_remote(command, version) {
            known
        } else {
            let remote = self.codex_runs_remote(command, cwd).await;
            if let Some(remote) = remote {
                pty_agents.set_codex_remote(command, version, remote);
            }
            remote.unwrap_or(false)
        };
        CodexGate { hooks, remote }
    }

    /// `--remote` on the TUI and `--listen unix://PATH` on its app-server; `--help` only, so nothing
    /// starts. `None` when a probe couldn't run.
    async fn codex_runs_remote(&self, command: &str, cwd: &Path) -> Option<bool> {
        let lists = |help: &str, flag: &str| help.contains(flag) && help.contains("unix://PATH");
        let tui = self.probe_codex(command, &["--help"], cwd).await?;
        let runs = lists(&tui, "--remote")
            && lists(
                &self
                    .probe_codex(command, &["app-server", "--help"], cwd)
                    .await?,
                "--listen",
            );
        if !runs {
            tracing::info!("codex runs embedded: `{command}` has no unix --remote/--listen pair");
        }
        Some(runs)
    }

    async fn probe_codex(&self, command: &str, args: &[&str], cwd: &Path) -> Option<String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        Exec::new(default_shell())
            .args(login_shell_args(&agent_command_line(command, &args)))
            .cwd(cwd)
            .envs(self.runtime.child_env().await)
            .timeout(VERSION_PROBE_TIMEOUT)
            .run_unchecked()
            .await
            .ok()
            .map(|out| out.stdout)
    }

    /// The app-server a PTY codex's TUI connects to, with its socket in the server's short runtime
    /// dir; `None` keeps the embedded launch.
    #[cfg(unix)]
    async fn start_codex_remote(
        &self,
        engine: &Engine,
        terminal_id: &str,
        ws: &Workstream,
        env: &[(String, String)],
        config: &[String],
        gate: &CodexGateCell,
    ) -> Option<Arc<CodexRemote>> {
        use codex_remote::{AppServerSpec, MAX_SOCKET_PATH};

        let cwd = PathBuf::from(&ws.worktree_path);
        if !self.codex_gate(gate, &engine.command, &cwd).await.remote {
            return None;
        }
        let dir = self
            .runtime
            .agent_control_endpoint
            .as_ref()?
            .runtime_dir
            .as_ref()?;
        let name: String = terminal_id.chars().filter(|c| *c != '-').take(12).collect();
        let socket = dir.join(format!("{name}.sock"));
        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            tracing::info!(
                "codex runs embedded: {} is too long for a socket",
                socket.display()
            );
            return None;
        }
        let spec = AppServerSpec {
            command: engine.command.clone(),
            config: config.to_vec(),
            cwd,
            env: env.to_vec(),
            socket,
            pid_file: dir.join(format!("{name}.pid")),
            terminal_id: terminal_id.to_string(),
        };
        match CodexRemote::start(spec, &self.runtime.pty_agents).await {
            Ok(remote) => Some(remote),
            Err(e) => {
                tracing::warn!("codex runs embedded: {e}");
                None
            }
        }
    }

    /// Hooked agents, plus unhooked ones an agent spawned; an unhooked entry's state means nothing.
    pub fn list_pty_agents(&self, workstream_id: Option<&str>) -> Vec<PtyAgent> {
        self.runtime.pty_agents.list(workstream_id)
    }

    pub async fn resize_agent(&self, req: ResizeAgentTerminalRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        self.runtime.terminals.note_agent_fit(req.cols, req.rows);
        let Some(session) = self.runtime.terminals.get(&req.terminal_id).await else {
            return Ok(());
        };
        let nudge = req.rows.saturating_sub(1).max(2);
        // A frame viewer needs no repaint to catch up, and every frame viewer would show the nudge's size.
        let unchanged = req.frames && session.size() == (req.rows, req.cols);
        if nudge != req.rows && !unchanged {
            session.fit(req.viewer, req.cols, nudge);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        session.fit(req.viewer, req.cols, req.rows);
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

pub(crate) fn agent_command_line(command: &str, args: &[String]) -> String {
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
pub(crate) fn sh_quote(s: &str) -> String {
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
    home.join(".claude")
        .join("projects")
        .join(crate::claude::memory::project_key(worktree))
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

    #[cfg(not(windows))]
    #[test]
    fn resolve_shell_prefers_the_supplied_env_value() {
        assert_eq!(
            resolve_shell(Some(PathBuf::from("/usr/bin/zsh"))),
            PathBuf::from("/usr/bin/zsh")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_shell_falls_back_to_bin_sh() {
        assert_eq!(resolve_shell(None), PathBuf::from("/bin/sh"));
    }

    #[cfg(unix)]
    #[test]
    fn login_shell_args_wraps_the_command_for_a_login_shell() {
        assert_eq!(
            login_shell_args("echo hi"),
            vec!["-lc".to_string(), "echo hi".to_string()]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_exited_terminal_leaves_the_registry_while_its_reader_stalls() {
        let registry = Arc::new(crate::runtime::terminal_registry::TerminalRegistry::new());
        let session = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "seq 1 20000; exit 0".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "ws".into(),
        })
        .expect("spawn pty");
        registry.insert("t".into(), session.clone()).await;
        let _unread = session.subscribe();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while registry.get("t").await.is_some() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            registry.get("t").await.is_none(),
            "an exited terminal must not wait on its reader to leave the registry"
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
