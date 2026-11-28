//! Agents starting, waiting on and messaging composer sessions and PTY agents.

use std::sync::Arc;
use std::time::Duration;

use hyprlayer_models::{
    AgentCaller, ClientRef, Goal, Harness, MessageDelivery, PermissionMode, PtyAgent,
    PtyAgentState, PtyFirstPrompt, PtyLaunchSession, RuntimeEvent, SessionEvent, SessionOrigin,
    SpawnPtyAgentRequest, SpawnSessionRequest, SpawnedAgent, UiRequest, WaitOutcome, WaitUntil,
    WorkstreamStatus,
};
use tokio::sync::OwnedMutexGuard;
use tokio::time::Instant;

use super::pty_agents::{asking, PtyAgents, PtySnapshot};
use super::pty_controls::{self, composing};
use super::resume::{ResumeCause, Resumed};
use super::{blocking, denied, in_project, AgentControlService, AgentIdentity};
use crate::capability::launch_policy;
use crate::claude::driver::permission_mode_flag;
use crate::error::{CoreError, Result};
use crate::pipeline::{self, Stage};
use crate::runtime::goals::{GoalTarget, StartGoalRequest};
use crate::runtime::sessions::goal_loop::{is_idle, Watch};
use crate::runtime::sessions::{
    MessageSender, SendContext, SessionLineage, StartStageSessionRequest,
};
use crate::runtime::terminals::{CloseTerminalRequest, LaunchAgentRequest, PtyLineage};
use crate::store::{PtyAgentRow, Session, Workstream};
use crate::terminal::{engines, text, PtySession, RingReader};

/// A wait's caller may be gone; the request task still runs to its end, so the end is bounded.
const WAIT_CAP: Duration = Duration::from_secs(600);
const STALL: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(250);
/// A spawned PTY has no window yet: it takes the size a desktop last fitted an agent tab to, so
/// opening its tab resizes nothing. claude reprints its whole transcript on a resize and leaves the
/// old width's copy in scrollback. This is the size before any desktop has fitted one.
pub(super) const SPAWN_COLS: u16 = 120;
pub(super) const SPAWN_ROWS: u16 = 32;
/// A first prompt still waiting on its TUI counts as owed this long past each check.
pub(super) const PROMPT_WAIT: Duration = Duration::from_secs(60);
/// An unhooked TUI reports no ready state; this much quiet output stands in for one.
const PROMPT_QUIET: Duration = Duration::from_secs(3);
/// A paste the TUI swallowed never submits; past this a wait stops counting it as pending.
pub(super) const PROMPT_ACK: Duration = Duration::from_secs(10);
/// The TUI must see the paste end before the Enter, or the Enter joins the paste.
const PASTE_SETTLE: Duration = Duration::from_millis(100);
const INTERRUPT_SETTLE: Duration = Duration::from_secs(10);
/// Apart enough that the TUI reads two Esc keys, not one escape sequence.
const ESC_GAP: Duration = Duration::from_millis(500);
const READ_LINES_CAP: u32 = 500;

/// What a spawn may start, once every refusal has been ruled out.
pub struct SpawnCheck {
    pub workstream: Workstream,
    pub harness: Option<Harness>,
    pub permission_mode: PermissionMode,
    /// The child's depth.
    pub depth: u32,
    pub lineage: SessionLineage,
    /// Held until the child is live, so the caller's next spawn counts it.
    _slot: OwnedMutexGuard<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Observed {
    Working,
    NeedsYou,
    Idle,
    Exited,
}

impl Observed {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Observed::Working => "working",
            Observed::NeedsYou => "needs_you",
            Observed::Idle => "idle",
            Observed::Exited => "exited",
        }
    }

    /// A wait stops at its state or any later one: an asking child never reaches idle on its own.
    pub(super) fn reaches(self, until: WaitUntil) -> bool {
        match until {
            WaitUntil::Idle => self != Observed::Working,
            WaitUntil::NeedsYou => matches!(self, Observed::NeedsYou | Observed::Exited),
            WaitUntil::Exited => self == Observed::Exited,
        }
    }
}

struct PtyChild {
    workstream_id: String,
    engine_id: String,
    /// Set when the child runs a stage: its prompt is composed and sent with the launch.
    stage: Option<Stage>,
    attached_docs: Vec<String>,
    prompt: Option<String>,
    /// `None` takes the caller's own.
    permission_mode: Option<PermissionMode>,
    model: Option<String>,
    effort: Option<String>,
    title: Option<String>,
    /// An unhooked TUI the caller may not type into yields no agent rather than a refusal.
    session_fallback: bool,
}

pub(super) struct LastTurn {
    pub(super) asking: bool,
    pub(super) reply: Option<String>,
}

impl AgentControlService<'_> {
    /// Every refusal a spawn can meet, run before anything is written: level, project, a Lead's own
    /// workstream, docs, depth, children, escalation.
    pub async fn check_spawn(
        &self,
        identity: &AgentIdentity,
        workstream_id: &str,
        stage: Stage,
        harness: Option<Harness>,
        permission_mode: Option<PermissionMode>,
        attached_docs: &[String],
    ) -> Result<SpawnCheck> {
        self.require_full(identity).await?;
        let workstream = self.runtime.workstreams().find(workstream_id).await?;
        in_project(identity, &workstream.project_path)?;
        if identity.lead && workstream.id == identity.workstream_id {
            return Err(CoreError::LeadOwnWorkstream);
        }
        // An Implement or Validate start activates the plans it attaches, which writes them.
        let activates = matches!(stage, Stage::Implement | Stage::Validate);
        for doc in attached_docs {
            self.check_doc(identity, doc, activates).await?;
        }
        let limits = self.runtime.settings.get().await.agent_control;
        let depth = identity.depth.saturating_add(1);
        if depth > limits.max_depth {
            return Err(CoreError::AgentLimitReached {
                detail: format!(
                    "a child would be {depth} agents deep, past the Max depth of {}",
                    limits.max_depth
                ),
            });
        }
        let slot = self.admit_child(&identity.caller, "this agent").await?;
        let own = self
            .mode_of(identity)
            .await
            .unwrap_or(PermissionMode::Default);
        let permission_mode = permission_mode.unwrap_or(own);
        if rank(permission_mode) > rank(own) {
            return Err(denied(&format!(
                "a child cannot run in {} mode, above this agent's own {}",
                mode_name(permission_mode),
                mode_name(own)
            )));
        }
        // A stage's pre-approved tools run unasked whatever the mode, so they must not widen the caller's.
        if own != PermissionMode::BypassPermissions {
            let own_stage = self.caller_stage(identity).await;
            if !pipeline::pre_approves_within(stage, own_stage) {
                return Err(denied(&format!(
                    "a child cannot run the {} stage, which pre-approves tools this agent's {} stage does not",
                    stage.as_str(),
                    own_stage.as_str()
                )));
            }
        }
        let (parent_session_id, parent_terminal) = match &identity.caller {
            AgentCaller::Session { session_id } => (Some(session_id.clone()), None),
            AgentCaller::Terminal { terminal_id } => (None, Some((terminal_id.clone(), depth))),
        };
        Ok(SpawnCheck {
            workstream,
            harness: harness.or(identity.harness),
            permission_mode,
            depth,
            lineage: SessionLineage {
                origin: SessionOrigin::Agent,
                parent_session_id,
                parent_terminal,
                lead: false,
            },
            _slot: slot,
        })
    }

    pub(super) async fn admit_child(
        &self,
        parent: &AgentCaller,
        whose: &str,
    ) -> Result<OwnedMutexGuard<()>> {
        let slot = self.runtime.agent_spawn_locks.acquire(parent.id()).await;
        let control = self.runtime.settings.get().await.agent_control;
        let lead = control.lead();
        let (max, which) = if lead.enabled && self.is_lead(parent).await {
            (lead.max_live_children, "the Lead's Max live children")
        } else {
            (control.max_live_children, "the Max live children")
        };
        let live = self.live_children(parent).await?;
        if live >= max as usize {
            return Err(CoreError::AgentLimitReached {
                detail: format!("{whose} already has {live} live children, {which}"),
            });
        }
        Ok(slot)
    }

    /// Reviving an exited spawned session takes a live slot from whoever spawned it.
    async fn admit_revive(
        &self,
        identity: &AgentIdentity,
        target: &Session,
    ) -> Result<Option<OwnedMutexGuard<()>>> {
        let parent = self
            .runtime
            .agent_links
            .get(&target.id)
            .map(|link| link.parent)
            .or_else(|| {
                target.spawn_parent().map(|id| AgentCaller::Session {
                    session_id: id.to_string(),
                })
            });
        Ok(match parent {
            Some(parent) if parent == identity.caller => {
                Some(self.admit_child(&parent, "this agent").await?)
            }
            Some(parent) => Some(
                self.admit_child(&parent, "the agent that spawned it")
                    .await?,
            ),
            None => None,
        })
    }

    /// A child that names no model runs the user's default for its harness, as a new chat does;
    /// an engine with no composer harness keeps its own.
    async fn child_model(&self, asked: Option<String>, harness: Option<Harness>) -> Option<String> {
        if asked.is_some() {
            return asked;
        }
        let providers = self.runtime.settings.get().await.providers;
        harness.and_then(|h| providers.default_model(h))
    }

    async fn is_lead(&self, caller: &AgentCaller) -> bool {
        let AgentCaller::Session { session_id } = caller else {
            return false;
        };
        matches!(self.runtime.sessions().get(session_id).await, Ok(Some(session)) if session.lead)
    }

    /// A session caller's own stage; a terminal, or an unreadable row, counts as General.
    async fn caller_stage(&self, identity: &AgentIdentity) -> Stage {
        let AgentCaller::Session { session_id } = &identity.caller else {
            return Stage::General;
        };
        match self.runtime.sessions().get(session_id).await {
            Ok(Some(session)) => session.stage,
            _ => Stage::General,
        }
    }

    /// Starts a composer session for the caller with no stream; every attached desktop opens it in the background.
    pub async fn spawn_session(
        &self,
        identity: &AgentIdentity,
        request: SpawnSessionRequest,
    ) -> Result<Session> {
        let SpawnSessionRequest {
            workstream_id,
            stage,
            prompt,
            harness,
            model,
            effort,
            permission_mode,
            attached_docs,
            title,
        } = request;
        let stage = stage.unwrap_or(Stage::General);
        let check = self
            .check_spawn(
                identity,
                &workstream_id,
                stage,
                harness,
                permission_mode,
                &attached_docs,
            )
            .await?;
        let model = self
            .child_model(model, Some(check.harness.unwrap_or_default()))
            .await;
        let label = self.caller_label(identity).await;
        let sessions = self.runtime.sessions();
        let started = sessions
            .prepare_start_with(
                StartStageSessionRequest {
                    workstream: check.workstream,
                    stage,
                    input: prompt,
                    attached_docs,
                    harness: check.harness,
                    model: model.clone(),
                    effort: effort.clone(),
                    // Never `None`: a stage's own default can outrank the caller.
                    permission_mode: Some(check.permission_mode),
                    prompt_name: None,
                    client_message_id: None,
                    parent_session_id: None,
                },
                check.lineage,
            )
            .await?;
        let session = started.session;
        let mut launch = started.launch;
        launch.frame_prompt(&header(identity, &label), &spawner_note(identity, &label));
        sessions
            .start_detached(launch, Some(author(identity, label)))
            .await?;
        if title.is_some() {
            if let Err(e) = sessions.set_title(&session.id, title).await {
                tracing::warn!("session {}: its title was not set: {e}", session.id);
            }
        }
        // The launched mode, not the asked one: the launch policy may have resolved it differently.
        let launched = sessions.permission_mode(&session.id).await;
        let _ = self.runtime.events.send(RuntimeEvent::UiRequested {
            request: UiRequest::OpenSessionTab {
                workstream_id: session.workstream_id.clone(),
                session_id: session.id.clone(),
                focus: false,
                permission_mode: launched.or(Some(check.permission_mode)),
                model,
                effort,
            },
        });
        Ok(sessions.get(&session.id).await?.unwrap_or(session))
    }

    /// Returns once the session reaches `until`, stalls, or the timeout (at most 600 s) passes.
    pub async fn wait_session(
        &self,
        identity: &AgentIdentity,
        session_id: &str,
        until: WaitUntil,
        timeout_ms: u32,
    ) -> Result<WaitOutcome> {
        self.check_session(identity, session_id).await?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.into()).min(WAIT_CAP);
        let actors = self.runtime.session_actors.clone();
        // Lost or ended, the watch only stops waking the loop early; the poll still runs.
        let mut watch = Watch::attach(actors.clone(), session_id).await.ok();
        let mut asking = self.last_turn(session_id).await?.asking;
        let rev_of = || async { actors.rev_state(session_id, None).await.map(|s| s.rev) };
        let mut rev = rev_of().await;
        let mut moved = Instant::now();
        loop {
            let observed = self.observe(session_id, asking).await;
            if observed.reaches(until) {
                return self.outcome(session_id, observed, false, false).await;
            }
            let now = rev_of().await;
            if now != rev {
                rev = now;
                moved = Instant::now();
            } else if observed != Observed::Working && moved.elapsed() >= STALL {
                return self.outcome(session_id, observed, true, false).await;
            }
            if Instant::now() >= deadline {
                return self.outcome(session_id, observed, false, true).await;
            }
            let tick = tokio::time::sleep(POLL);
            let ended = match watch.as_mut() {
                Some(w) => tokio::select! {
                    event = w.next() => match event {
                        Some(event) => {
                            asking = asks_after(asking, &event);
                            false
                        }
                        None => true,
                    },
                    () = tick => false,
                },
                None => {
                    tick.await;
                    false
                }
            };
            if ended {
                watch = None;
            }
        }
    }

    /// An attributed, headed turn into a session in the caller's project; hop-counted once delivered,
    /// unless a Lead sends it to an agent it spawned: that relays the user's own work.
    pub async fn send_message(
        &self,
        identity: &AgentIdentity,
        session_id: &str,
        text: &str,
        interrupt: bool,
    ) -> Result<MessageDelivery> {
        self.require_full(identity).await?;
        let sessions = self.runtime.sessions();
        let target = sessions
            .get(session_id)
            .await?
            .ok_or_else(|| CoreError::SessionNotFound {
                session_id: session_id.to_string(),
            })?;
        self.check_workstream(identity, &target.workstream_id)
            .await?;
        self.check_message_rank(identity, &target).await?;
        let counted = !(identity.lead
            && self
                .check_descendant(identity, session_id, false)
                .await
                .is_ok());
        if counted {
            self.admit_hop(session_id, &target.workstream_id, &session_label(&target))
                .await?;
        }
        if !sessions.is_live(session_id).await {
            let workstream = self
                .runtime
                .workstreams()
                .find(&target.workstream_id)
                .await?;
            if workstream.status == WorkstreamStatus::Archived {
                return Err(denied("that session's workstream is archived"));
            }
            let _slot = self.admit_revive(identity, &target).await?;
            let op = sessions.op_lock(session_id).await;
            let guard = op.lock().await;
            let session =
                sessions
                    .get(session_id)
                    .await?
                    .ok_or_else(|| CoreError::SessionNotFound {
                        session_id: session_id.to_string(),
                    })?;
            if session.archived_at.is_some() {
                return Err(denied("that session is archived"));
            }
            if session.provider_session_id.is_none() {
                return Err(denied(
                    "that session never started a conversation to resume",
                ));
            }
            sessions
                .resume_if_exited_held(session, workstream.project_path, &guard)
                .await?;
        }
        let interrupted = interrupt && !is_idle(&self.runtime.session_actors, session_id).await;
        if interrupted {
            sessions.interrupt(session_id).await?;
            let deadline = Instant::now() + INTERRUPT_SETTLE;
            while !is_idle(&self.runtime.session_actors, session_id).await
                && Instant::now() < deadline
            {
                tokio::time::sleep(POLL).await;
            }
        }
        let label = self.caller_label(identity).await;
        let text = pipeline::headed(&header(identity, &label), text);
        sessions
            .send(
                session_id,
                &text,
                &[],
                None,
                None,
                SendContext {
                    base_rev: None,
                    author: Some(author(identity, label)),
                    sender: Some(MessageSender {
                        caller: identity.caller.clone(),
                        project_path: identity.project_path.clone(),
                    }),
                },
            )
            .await?;
        if counted {
            self.record_hop(session_id).await;
        }
        Ok(match interrupted {
            true => MessageDelivery::Interrupted,
            false => MessageDelivery::Sent,
        })
    }

    /// A message steers its target, so one running above the caller must be the caller's own child.
    async fn check_message_rank(&self, identity: &AgentIdentity, target: &Session) -> Result<()> {
        let own = self
            .mode_of(identity)
            .await
            .unwrap_or(PermissionMode::Default);
        let theirs = match self
            .runtime
            .session_registry
            .permission_mode(&target.id)
            .await
        {
            Some(mode) => mode,
            None => launch_policy(
                target.harness,
                target.stage,
                target.requested_permission_mode,
            )
            .permission_mode(),
        };
        if rank(theirs) <= rank(own)
            || self
                .check_descendant(identity, &target.id, false)
                .await
                .is_ok()
        {
            return Ok(());
        }
        Err(denied(&format!(
            "that session runs in {} mode, which outranks this agent's {}",
            mode_name(theirs),
            mode_name(own)
        )))
    }

    /// A composer session, or a PTY agent in the default engine when app actions open there.
    pub async fn spawn_agent(
        &self,
        identity: &AgentIdentity,
        request: SpawnSessionRequest,
    ) -> Result<SpawnedAgent> {
        let Some(engine_id) = self.action_engine(identity, &request).await else {
            let session = self.spawn_session(identity, request).await?;
            return Ok(SpawnedAgent::Session { session });
        };
        let SpawnSessionRequest {
            workstream_id,
            stage,
            prompt,
            model,
            effort,
            permission_mode,
            attached_docs,
            title,
            harness: _,
        } = request;
        let child = PtyChild {
            workstream_id: workstream_id.clone(),
            engine_id,
            stage: Some(stage.unwrap_or(Stage::General)),
            attached_docs: attached_docs.clone(),
            prompt: Some(prompt.clone()),
            permission_mode,
            model: model.clone(),
            effort: effort.clone(),
            title: title.clone(),
            session_fallback: true,
        };
        if let Some(agent) = self.launch_pty_child(identity, child).await? {
            return Ok(SpawnedAgent::Pty { agent });
        }
        let session = self
            .spawn_session(
                identity,
                SpawnSessionRequest {
                    workstream_id,
                    stage,
                    prompt,
                    harness: None,
                    model,
                    effort,
                    permission_mode,
                    attached_docs,
                    title,
                },
            )
            .await?;
        Ok(SpawnedAgent::Session { session })
    }

    /// The engine a spawn reroutes to; a pinned harness, or a model only the caller's harness takes, keeps it a session.
    async fn action_engine(
        &self,
        identity: &AgentIdentity,
        request: &SpawnSessionRequest,
    ) -> Option<String> {
        if request.harness.is_some() {
            return None;
        }
        let engine_id = self
            .runtime
            .settings
            .get()
            .await
            .multiplexer
            .action_engine()?;
        // Only claude's launch applies a stage's guardrails; codex's sandbox posture is the session's.
        if request.stage.is_some_and(|s| s != Stage::General) && engine_id != "claude-code" {
            return None;
        }
        let harness = engines::harness_of(&engine_id);
        if (request.model.is_some() || request.effort.is_some()) && harness != identity.harness {
            return None;
        }
        if harness.is_none() && self.check_unhooked_input(identity, false).await.is_err() {
            return None;
        }
        Some(engine_id)
    }

    /// Launches a PTY agent for the caller; every attached desktop adopts it in a background tab.
    pub async fn spawn_pty_agent(
        &self,
        identity: &AgentIdentity,
        request: SpawnPtyAgentRequest,
    ) -> Result<PtyAgent> {
        let SpawnPtyAgentRequest {
            workstream_id,
            engine_id,
            prompt,
            bypass,
            permission_mode,
            model,
            title,
        } = request;
        let permission_mode = if bypass {
            Some(PermissionMode::BypassPermissions)
        } else {
            permission_mode
        };
        self.launch_pty_child(
            identity,
            PtyChild {
                workstream_id,
                engine_id,
                stage: None,
                attached_docs: Vec::new(),
                prompt,
                permission_mode,
                model,
                effort: None,
                title,
                session_fallback: false,
            },
        )
        .await
        .map(|agent| agent.expect("only a session fallback yields no agent"))
    }

    /// `None` when the TUI came up unhooked for a caller that may not type into it, and the child falls back.
    async fn launch_pty_child(
        &self,
        identity: &AgentIdentity,
        child: PtyChild,
    ) -> Result<Option<PtyAgent>> {
        let PtyChild {
            workstream_id,
            engine_id,
            stage,
            attached_docs,
            prompt,
            permission_mode,
            model,
            effort,
            title,
            session_fallback,
        } = child;
        let configured = self.runtime.settings.get().await.multiplexer.engines;
        let model = self
            .child_model(model, engines::harness_of(&engine_id))
            .await;
        // An unknown id would launch as a bare command, a shell the agent could then type into.
        if !engines::is_known(&configured, &engine_id) {
            return Err(denied(&format!("{engine_id} is not a configured engine")));
        }
        let prompted = stage.is_some() || prompt.as_ref().is_some_and(|p| !p.trim().is_empty());
        if prompted && !matches!(engine_id.as_str(), "claude-code" | "codex") {
            self.check_unhooked_input(identity, false).await?;
        }
        // Asking for a mode makes the spawn check refuse one above the caller's.
        let check = self
            .check_spawn(
                identity,
                &workstream_id,
                stage.unwrap_or(Stage::General),
                None,
                permission_mode,
                &attached_docs,
            )
            .await?;
        let mode = check.permission_mode;
        let bypass = mode == PermissionMode::BypassPermissions;
        let label = self.caller_label(identity).await;
        let (stage_prompt, prompt) = match stage {
            Some(stage) => {
                let input = prompt.unwrap_or_default();
                (
                    Some(PtyFirstPrompt {
                        stage,
                        input,
                        attached_docs,
                    }),
                    None,
                )
            }
            None => (None, prompt),
        };
        let (cols, rows) = self
            .runtime
            .terminals
            .agent_fit()
            .unwrap_or((SPAWN_COLS, SPAWN_ROWS));
        let (_, launched) = self
            .runtime
            .terminals()
            .launch_agent(LaunchAgentRequest {
                workstream_id: check.workstream.id.clone(),
                engine_id: engine_id.clone(),
                session: PtyLaunchSession {
                    session_id: None,
                    resume: false,
                    bypass,
                    model,
                    // Always set for a stage, so its own default mode cannot outrank the caller.
                    permission_mode: (stage_prompt.is_some() || mode != PermissionMode::Default)
                        .then(|| permission_mode_flag(mode).to_string()),
                    effort,
                    first_prompt: stage_prompt.clone(),
                },
                cols,
                rows,
                lineage: Some(PtyLineage {
                    parent: identity.caller.clone(),
                    depth: check.depth,
                    mode,
                }),
                resume_of: None,
            })
            .await?;
        // A claude or codex that came up unhooked (old codex, a user `--settings`) gets the same rule.
        if prompted && !launched.hooked {
            if let Err(e) = self.check_unhooked_input(identity, false).await {
                let _ = self
                    .runtime
                    .terminals()
                    .close(CloseTerminalRequest {
                        terminal_id: launched.terminal_id.clone(),
                    })
                    .await;
                return if session_fallback { Ok(None) } else { Err(e) };
            }
        }
        let snapshot = self.runtime.pty_agents.snapshot(&launched.terminal_id);
        let mut agent = snapshot.as_ref().map_or_else(
            || PtyAgent {
                terminal_id: launched.terminal_id.clone(),
                workstream_id: check.workstream.id.clone(),
                engine_id,
                state: PtyAgentState::Exited,
                provider_session_id: launched.provider_session_id.clone(),
                activity: None,
                last_reply: None,
                spawned_by: Some(identity.caller.clone()),
                hooked: launched.hooked,
                title: None,
                status: Default::default(),
            },
            |s| s.agent.clone(),
        );
        let _ = self.runtime.events.send(RuntimeEvent::UiRequested {
            request: UiRequest::OpenTerminalTab {
                workstream_id: agent.workstream_id.clone(),
                terminal_id: agent.terminal_id.clone(),
                engine_id: agent.engine_id.clone(),
                provider_session_id: agent.provider_session_id.clone(),
                bypass,
            },
        });
        // After the tab opens, so desktops adopt it as they do a generated title, which it then rules out.
        if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
            self.runtime
                .pty_agents
                .set_title(&agent.terminal_id, title.clone());
            agent.title = Some(title);
        }
        // After the hooked check, so a refused spawn activates no plan; the header trails a leading `/skill`.
        let text = if let Some(fp) = stage_prompt {
            let text = self
                .runtime
                .sessions()
                .pty_first_prompt(&check.workstream, engines::harness_of(&agent.engine_id), fp)
                .await;
            Some(format!(
                "{text}\n\n{}\n\n{}",
                header(identity, &label),
                spawner_note(identity, &label)
            ))
        } else {
            prompt.filter(|p| !p.trim().is_empty()).map(|prompt| {
                format!(
                    "{}\n\n{}",
                    pipeline::headed(&header(identity, &label), &prompt),
                    spawner_note(identity, &label)
                )
            })
        };
        if let (Some(text), Some(snapshot)) = (text, snapshot) {
            // A wait right after the spawn must not see the pre-prompt idle as the agent's answer.
            let agents = self.runtime.pty_agents.clone();
            agents.owe_prompt(&agent.terminal_id, PROMPT_WAIT);
            let runtime = self.runtime.handle.clone();
            let (caller, project) = (identity.caller.clone(), identity.project_path.clone());
            let terminal_id = agent.terminal_id.clone();
            tokio::spawn(async move {
                if first_prompt(agents, snapshot, text).await {
                    return;
                }
                if let Some(rt) = runtime.upgrade() {
                    rt.agent_control().report_to(
                        caller,
                        project,
                        format!("terminal {terminal_id} exited before it was ready, so the prompt you spawned it with was never delivered"),
                    );
                }
            });
        }
        Ok(Some(agent))
    }

    /// A headed paste into a PTY agent the caller spawned; hop-counted unless the caller is a Lead.
    pub async fn send_pty_message(
        &self,
        identity: &AgentIdentity,
        terminal_id: &str,
        text: &str,
        interrupt: bool,
    ) -> Result<MessageDelivery> {
        self.require_full(identity).await?;
        if let Some(row) = self.stored(terminal_id).await? {
            if let Some(delivery) = self.resume_with(identity, &row, text).await? {
                return Ok(delivery);
            }
        }
        let target = self.pty_target(identity, terminal_id).await?;
        self.check_descendant(identity, terminal_id, false).await?;
        self.check_unhooked_input(identity, target.agent.hooked)
            .await?;
        let label = self.caller_label(identity).await;
        let text = pipeline::headed(&header(identity, &label), text);
        let agents = &self.runtime.pty_agents;
        let _input = target.input.lock().await;
        // Read under the lock: a bar change or message that held it may have moved the agent on.
        if let Some(reason) = paste_refusal(agents, terminal_id) {
            return Err(denied(reason));
        }
        if !identity.lead {
            self.admit_hop(
                terminal_id,
                &target.agent.workstream_id,
                &target.agent.engine_id,
            )
            .await?;
        }
        let busy = agents
            .snapshot(terminal_id)
            .is_some_and(|now| now.agent.state != PtyAgentState::Idle || now.prompt_owed);
        let interrupted = interrupt && busy;
        if interrupted {
            if !target.agent.hooked {
                return Err(denied(
                    "that agent's engine reports no state, so nothing would say when an interrupt ended its turn",
                ));
            }
            interrupt_turn(agents, terminal_id, &target.session).await?;
        }
        let delivery = if interrupted
            || agents.holds(terminal_id)
            || command_while_busy(agents, terminal_id, &text)
        {
            if agents.hold(terminal_id, text) == Some(true) {
                tokio::spawn(deliver_held(
                    self.runtime.pty_agents.clone(),
                    terminal_id.to_string(),
                ));
            }
            match interrupted {
                true => MessageDelivery::Interrupted,
                false => MessageDelivery::Queued,
            }
        } else {
            // Before the write, so a fast `UserPromptSubmit` settles it rather than racing it.
            agents.owe_prompt(terminal_id, PROMPT_ACK);
            paste(agents, terminal_id, &target.session, &text).await?;
            MessageDelivery::Sent
        };
        if !identity.lead {
            self.record_hop(terminal_id).await;
        }
        Ok(delivery)
    }

    /// An exited agent the caller spawned, resumed in place with the message as its first turn;
    /// `None` when something else resumed it first, so the message goes in as to any live one.
    async fn resume_with(
        &self,
        identity: &AgentIdentity,
        row: &PtyAgentRow,
        text: &str,
    ) -> Result<Option<MessageDelivery>> {
        let terminal_id = row.terminal_id.as_str();
        self.check_workstream(identity, &row.workstream_id).await?;
        self.check_descendant(identity, terminal_id, false).await?;
        if !identity.lead {
            self.admit_hop(terminal_id, &row.workstream_id, &row.engine_id)
                .await?;
        }
        let label = self.caller_label(identity).await;
        let text = pipeline::headed(&header(identity, &label), text);
        match self
            .resume_pty(terminal_id, ResumeCause::Message(text))
            .await?
        {
            Resumed::Launched => {
                if !identity.lead {
                    self.record_hop(terminal_id).await;
                }
                Ok(Some(MessageDelivery::Sent))
            }
            Resumed::Live => Ok(None),
            Resumed::Skipped(_) => Err(denied("that agent cannot be resumed")),
        }
    }

    /// Ends a PTY agent the caller spawned, freeing its live-children slot; an exited one's row
    /// goes, so nothing resumes it.
    pub async fn kill_pty_agent(&self, identity: &AgentIdentity, terminal_id: &str) -> Result<()> {
        self.require_full(identity).await?;
        match self.stored(terminal_id).await? {
            Some(row) => self.check_workstream(identity, &row.workstream_id).await?,
            None => {
                self.pty_target(identity, terminal_id).await?;
            }
        }
        self.check_descendant(identity, terminal_id, false).await?;
        self.runtime
            .terminals()
            .close(CloseTerminalRequest {
                terminal_id: terminal_id.to_string(),
            })
            .await
    }

    /// Renames a running PTY agent the caller spawned. An exited one is refused: its tab is gone.
    pub async fn set_pty_agent_title(
        &self,
        identity: &AgentIdentity,
        terminal_id: &str,
        title: &str,
    ) -> Result<()> {
        self.require_full(identity).await?;
        self.pty_target(identity, terminal_id).await?;
        self.check_descendant(identity, terminal_id, false).await?;
        let title = title.trim();
        if title.is_empty() {
            return Err(denied("a title cannot be blank"));
        }
        self.runtime
            .pty_agents
            .set_title(terminal_id, title.to_string());
        Ok(())
    }

    /// Esc, the TUI's interrupt, into a PTY agent the caller spawned; never a human turn.
    pub async fn interrupt_pty_agent(
        &self,
        identity: &AgentIdentity,
        terminal_id: &str,
    ) -> Result<()> {
        self.require_full(identity).await?;
        let target = self.pty_target(identity, terminal_id).await?;
        self.check_descendant(identity, terminal_id, false).await?;
        interrupt_turn(&self.runtime.pty_agents, terminal_id, &target.session).await
    }

    /// The PTY agent's screen as plain text: its last `lines` lines, at most 500.
    pub async fn read_terminal(
        &self,
        identity: &AgentIdentity,
        terminal_id: &str,
        lines: u32,
    ) -> Result<String> {
        let target = self.pty_target(identity, terminal_id).await?;
        let lines = lines.clamp(1, READ_LINES_CAP) as usize;
        let cols = target.session.size().1.into();
        Ok(text::tail(&target.session.scrollback_bytes(), cols, lines))
    }

    /// `wait_session` over a hooked PTY agent's folded state.
    pub async fn wait_pty_agent(
        &self,
        identity: &AgentIdentity,
        terminal_id: &str,
        until: WaitUntil,
        timeout_ms: u32,
    ) -> Result<WaitOutcome> {
        let first = self.pty_target(identity, terminal_id).await?;
        if !first.agent.hooked {
            return Err(denied(
                "that agent's engine reports no state to wait on; read its terminal instead",
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.into()).min(WAIT_CAP);
        let (mut signals, mut last_reply) = (first.signals, first.agent.last_reply);
        let mut moved = Instant::now();
        loop {
            let observed = match self.runtime.pty_agents.snapshot(terminal_id) {
                Some(now) => {
                    if now.signals != signals {
                        signals = now.signals;
                        moved = Instant::now();
                    }
                    last_reply = now.agent.last_reply;
                    if now.prompt_owed && !asking(now.agent.state) {
                        Observed::Working
                    } else {
                        observe_pty(now.agent.state)
                    }
                }
                None => Observed::Exited,
            };
            let outcome = |stalled, timed_out| WaitOutcome {
                state: observed.as_str().to_string(),
                last_reply: last_reply.clone(),
                stalled,
                timed_out,
            };
            if observed.reaches(until) {
                return Ok(outcome(false, false));
            }
            if observed != Observed::Working && moved.elapsed() >= STALL {
                return Ok(outcome(true, false));
            }
            if Instant::now() >= deadline {
                return Ok(outcome(false, true));
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// A goal on the caller or a session it spawned, or on a new session under `spawn_session`'s limits.
    pub async fn start_goal(
        &self,
        identity: &AgentIdentity,
        request: StartGoalRequest,
    ) -> Result<Goal> {
        self.require_full(identity).await?;
        let StartGoalRequest { target, spec } = request;
        let label = self.caller_label(identity).await;
        let author = Some(author(identity, label));
        let goals = self.runtime.goals();
        match target {
            GoalTarget::Session { session_id } => {
                self.check_session(identity, &session_id).await?;
                self.check_descendant(identity, &session_id, true).await?;
                let sessions = self.runtime.sessions();
                let _slot = match sessions.get(&session_id).await? {
                    Some(row) if !sessions.is_live(&session_id).await => {
                        self.admit_revive(identity, &row).await?
                    }
                    _ => None,
                };
                let target = GoalTarget::Session { session_id };
                goals.start(StartGoalRequest { target, spec }, author).await
            }
            GoalTarget::NewSession {
                workstream_id,
                stage,
                harness,
                model,
                effort,
                permission_mode,
                attached_docs,
            } => {
                let check = self
                    .check_spawn(
                        identity,
                        &workstream_id,
                        stage,
                        harness,
                        permission_mode,
                        &attached_docs,
                    )
                    .await?;
                let model = self
                    .child_model(model, Some(check.harness.unwrap_or_default()))
                    .await;
                let target = GoalTarget::NewSession {
                    workstream_id,
                    stage,
                    harness: check.harness,
                    model,
                    effort,
                    // Never `None`: a stage's own default can outrank the caller.
                    permission_mode: Some(check.permission_mode),
                    attached_docs,
                };
                goals
                    .start_with(StartGoalRequest { target, spec }, author, check.lineage)
                    .await
            }
        }
    }

    pub(super) async fn observe(&self, session_id: &str, asking: bool) -> Observed {
        if !self.runtime.sessions().is_live(session_id).await {
            Observed::Exited
        } else if asking {
            Observed::NeedsYou
        } else if is_idle(&self.runtime.session_actors, session_id).await {
            Observed::Idle
        } else {
            Observed::Working
        }
    }

    async fn outcome(
        &self,
        session_id: &str,
        observed: Observed,
        stalled: bool,
        timed_out: bool,
    ) -> Result<WaitOutcome> {
        Ok(WaitOutcome {
            state: observed.as_str().to_string(),
            last_reply: self.last_turn(session_id).await?.reply,
            stalled,
            timed_out,
        })
    }

    /// Only the latest turn's rows: its open ask and its last top-level reply.
    pub(super) async fn last_turn(&self, session_id: &str) -> Result<LastTurn> {
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        let tail = blocking(move || {
            let turns = db.count_user_messages(&id)?;
            let from = db.nth_user_message_seq(&id, turns)?;
            db.session_log_after(&id, from.map(|seq| seq - 1))
        })
        .await?;
        let mut last = LastTurn {
            asking: false,
            reply: None,
        };
        for entry in tail {
            last.asking = asks_after(last.asking, &entry.event);
            if let SessionEvent::AssistantText {
                text,
                parent_id: None,
                ..
            } = entry.event
            {
                last.reply = Some(text);
            }
        }
        Ok(last)
    }

    /// An unhooked PTY reports no prompt, so typed input could answer one; only a bypass caller may risk it.
    pub(super) async fn check_unhooked_input(
        &self,
        identity: &AgentIdentity,
        hooked: bool,
    ) -> Result<()> {
        if hooked || self.mode_of(identity).await == Some(PermissionMode::BypassPermissions) {
            return Ok(());
        }
        Err(denied(
            "that agent reports no state, so typed input could answer a prompt it shows",
        ))
    }

    /// How a message names its sender: its title, or a terminal's engine when it has none.
    async fn caller_label(&self, identity: &AgentIdentity) -> String {
        if identity.lead {
            return super::leads::LEAD_TITLE.to_string();
        }
        match &identity.caller {
            AgentCaller::Session { session_id } => {
                match self.runtime.sessions().get(session_id).await {
                    Ok(Some(session)) => session_label(&session),
                    _ => "an agent".to_string(),
                }
            }
            AgentCaller::Terminal { terminal_id } => self
                .runtime
                .pty_agents
                .list(Some(&identity.workstream_id))
                .into_iter()
                .find(|a| &a.terminal_id == terminal_id)
                .map_or_else(
                    || "a terminal agent".to_string(),
                    |a| a.title.unwrap_or(a.engine_id),
                ),
        }
    }
}

fn header(identity: &AgentIdentity, label: &str) -> String {
    format!("[from agent \"{label}\" · {}]", identity.caller.id())
}

/// Claude's own `SendMessage` reaches only the sessions its CLI tracks, never a hyprlayer agent.
fn spawner_note(identity: &AgentIdentity, label: &str) -> String {
    let id = identity.caller.id();
    match identity.caller {
        AgentCaller::Session { .. } => format!(
            "Your spawner is {label} (session {id}). Your final reply is your report. To message it \
             mid-task, use mcp__hyprlayer__message_agent with session_id {id}, not SendMessage. \
             hyprlayer refuses the message if that session runs in a higher permission mode than \
             yours."
        ),
        AgentCaller::Terminal { .. } => format!(
            "Your spawner is {label} (terminal {id}). Your final reply is your report. The agents a \
             terminal agent spawns can't message it, and SendMessage can't reach it."
        ),
    }
}

/// The `agent:` prefix is how desktops tell an agent's turn from a person's.
fn author(identity: &AgentIdentity, label: String) -> ClientRef {
    ClientRef {
        client_id: format!("{}{}", ClientRef::AGENT_PREFIX, identity.caller.id()),
        device: label,
    }
}

pub(super) fn session_label(session: &Session) -> String {
    session
        .title
        .clone()
        .unwrap_or_else(|| format!("{} session", session.stage.as_str()))
}

/// `exited` while still registered is a session ending inside a live process, as `/clear` does.
pub(super) fn observe_pty(state: PtyAgentState) -> Observed {
    match state {
        PtyAgentState::Idle => Observed::Idle,
        PtyAgentState::AwaitingPermission | PtyAgentState::AwaitingQuestion => Observed::NeedsYou,
        PtyAgentState::Starting | PtyAgentState::Working | PtyAgentState::Exited => {
            Observed::Working
        }
    }
}

/// Why a paste now would answer the agent's prompt or send a person's draft; `None` when it may go.
pub(super) fn paste_refusal(agents: &PtyAgents, terminal_id: &str) -> Option<&'static str> {
    let Some(now) = agents.snapshot(terminal_id) else {
        return Some("that agent is no longer running");
    };
    if asking(now.agent.state) {
        return Some("that agent is asking its user something, and a message now would answer it");
    }
    now.screen.as_deref().and_then(pty_controls::paste_refusal)
}

/// claude takes in a slash command sent mid-turn as a queued prompt, and can run it as plain text.
fn command_while_busy(agents: &PtyAgents, terminal_id: &str, text: &str) -> bool {
    pipeline::leading_command(text).is_some()
        && agents.snapshot(terminal_id).is_some_and(|now| {
            engines::harness_of(&now.agent.engine_id) == Some(Harness::Claude)
                && (now.agent.state != PtyAgentState::Idle || now.prompt_owed)
        })
}

/// Types the held messages one per idle, in order, as `send_pty_message` would have.
async fn deliver_held(agents: Arc<PtyAgents>, terminal_id: String) {
    let ready = || {
        agents
            .snapshot(&terminal_id)
            .is_some_and(|now| now.agent.state == PtyAgentState::Idle && !now.prompt_owed)
            && paste_refusal(&agents, &terminal_id).is_none()
    };
    loop {
        let Some(now) = agents.snapshot(&terminal_id) else {
            return;
        };
        if ready() {
            let _input = now.input.lock().await;
            if ready() {
                let Some(text) = agents.next_held(&terminal_id) else {
                    return;
                };
                agents.owe_prompt(&terminal_id, PROMPT_ACK);
                if let Err(e) = paste(&agents, &terminal_id, &now.session, &text).await {
                    tracing::warn!("terminal {terminal_id}: a held message was not delivered: {e}");
                }
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// In claude's vim mode the first Esc only leaves INSERT, so a second one stops the turn.
async fn interrupt_turn(
    agents: &Arc<PtyAgents>,
    terminal_id: &str,
    pty: &PtySession,
) -> Result<()> {
    let inserting = agents
        .snapshot(terminal_id)
        .and_then(|now| now.screen?.rows())
        .is_some_and(|rows| {
            rows.iter()
                .rev()
                .filter(|r| !r.trim().is_empty())
                .take(4)
                .any(|r| r.contains("-- INSERT --"))
        });
    pty.write_input(b"\x1b")?;
    if inserting {
        tokio::time::sleep(ESC_GAP).await;
        pty.write_input(b"\x1b")?;
    }
    // A cancelled ask fires no hook, so only the screen can say the agent is idle again.
    agents.note_input(terminal_id);
    Ok(())
}

/// Bracketed paste, then Enter on its own: a CR inside the paste is a newline to the TUI. Call it
/// under the input lock once `paste_refusal` passes.
pub(super) async fn paste(
    agents: &PtyAgents,
    terminal_id: &str,
    pty: &PtySession,
    text: &str,
) -> Result<()> {
    // An ESC in the text could end the paste early and type the rest as keys.
    let text = text.replace('\x1b', "");
    let claude = agents
        .snapshot(terminal_id)
        .is_some_and(|now| engines::harness_of(&now.agent.engine_id) == Some(Harness::Claude));
    let mut text = text.as_str();
    if let Some((command, rest)) = pipeline::leading_command(text).filter(|_| claude) {
        // claude reads a slash command inside a bracketed paste as plain text, so the command line
        // is typed; the rest pasted after it on the same input becomes the command's arguments.
        pty.write_input(command.as_bytes())?;
        tokio::time::sleep(PASTE_SETTLE).await;
        if rest.trim().is_empty() {
            return submit(agents, terminal_id, pty);
        }
        pty.write_input(b" ")?;
        text = rest;
    }
    pty.write_input(format!("\x1b[200~{text}\x1b[201~").as_bytes())?;
    tokio::time::sleep(PASTE_SETTLE).await;
    submit(agents, terminal_id, pty)
}

fn submit(agents: &PtyAgents, terminal_id: &str, pty: &PtySession) -> Result<()> {
    // A prompt that opened since would take the Enter as its answer.
    let clear = agents.snapshot(terminal_id).is_some_and(|now| {
        !asking(now.agent.state)
            && now
                .screen
                .is_none_or(|screen| screen.rows().is_some_and(|rows| composing(&rows)))
    });
    if !clear {
        agents.clear_prompt_owed(terminal_id);
        return Err(denied(
            "a prompt opened while the message was pasted, so it was left unsent in the agent's input",
        ));
    }
    pty.write_input(b"\r")?;
    Ok(())
}

/// Types an app action's prompt into a just-launched PTY agent once its TUI is ready.
pub(crate) fn send_first_prompt(agents: Arc<PtyAgents>, terminal_id: &str, text: String) {
    let Some(snapshot) = agents.snapshot(terminal_id) else {
        tracing::warn!("terminal {terminal_id}: gone before its first prompt");
        return;
    };
    agents.owe_prompt(terminal_id, PROMPT_WAIT);
    tokio::spawn(first_prompt(agents, snapshot, text));
}

/// Waits for the TUI to be ready, however long a startup prompt holds it, then pastes under the
/// input lock: hooked, idle with nothing over its input box; unhooked, a quiet spell. `false` when
/// the agent exited first.
async fn first_prompt(agents: Arc<PtyAgents>, launch: PtySnapshot, text: String) -> bool {
    let terminal_id = launch.agent.terminal_id.clone();
    let send = || async {
        agents.owe_prompt(&terminal_id, PROMPT_ACK);
        paste(&agents, &terminal_id, &launch.session, &text).await
    };
    let sent = if launch.agent.hooked {
        // `SessionStart` can idle it before claude has painted its input box.
        let idle = || {
            agents
                .snapshot(&terminal_id)
                .is_some_and(|now| now.agent.state == PtyAgentState::Idle)
                && paste_refusal(&agents, &terminal_id).is_none()
        };
        let starting = || {
            agents
                .snapshot(&terminal_id)
                .is_some_and(|now| now.agent.state == PtyAgentState::Starting)
        };
        let codex = launch.agent.engine_id == "codex";
        let mut reader = launch.session.subscribe();
        loop {
            let Some(now) = agents.snapshot(&terminal_id) else {
                break None;
            };
            agents.owe_prompt(&terminal_id, PROMPT_WAIT);
            if idle() {
                let _input = launch.input.lock().await;
                // Again under the lock: a bar change that held it may have opened a picker.
                if idle() {
                    break Some(send().await);
                }
            }
            // codex fires no hook until its first turn, so a quiet screen is its composer, up.
            if codex && starting() {
                if !quiet_spell(&mut reader).await {
                    break None;
                }
                let _input = launch.input.lock().await;
                if starting() {
                    break Some(send().await);
                }
                continue;
            }
            let prompting = now
                .screen
                .and_then(|screen| screen.rows())
                .is_some_and(|rows| pty_controls::covered(&rows));
            agents.startup_prompt(&terminal_id, prompting);
            tokio::time::sleep(POLL).await;
        }
    } else {
        let mut reader = launch.session.subscribe();
        if quiet_spell(&mut reader).await {
            let _input = launch.input.lock().await;
            Some(send().await)
        } else {
            None
        }
    };
    match sent {
        Some(Ok(())) => true,
        Some(Err(e)) => {
            tracing::warn!("terminal {terminal_id}: its first prompt was not written: {e}");
            true
        }
        None => {
            tracing::warn!("terminal {terminal_id}: exited before its first prompt was sent");
            false
        }
    }
}

/// `false` once the PTY has ended.
pub(super) async fn quiet_spell(reader: &mut RingReader) -> bool {
    loop {
        match tokio::time::timeout(PROMPT_QUIET, reader.skip()).await {
            Err(_) => return true,
            Ok(false) => return false,
            Ok(true) => {}
        }
    }
}

fn asks_after(asking: bool, event: &SessionEvent) -> bool {
    match event {
        SessionEvent::PermissionAsk { .. } | SessionEvent::QuestionAsk { .. } => true,
        SessionEvent::UserMessage { .. }
        | SessionEvent::AssistantText { .. }
        | SessionEvent::PermissionResolved { .. }
        | SessionEvent::QuestionAnswered { .. }
        | SessionEvent::ToolResult { .. }
        | SessionEvent::TurnDone { .. }
        | SessionEvent::Exited { .. } => false,
        _ => asking,
    }
}

pub(super) fn lower(a: PermissionMode, b: PermissionMode) -> PermissionMode {
    if rank(b) < rank(a) {
        b
    } else {
        a
    }
}

/// Plan < DontAsk < Default < AcceptEdits < Auto < Bypass: a child never runs above its parent.
fn rank(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::DontAsk => 1,
        PermissionMode::Default => 2,
        PermissionMode::AcceptEdits => 3,
        PermissionMode::Auto => 4,
        PermissionMode::BypassPermissions => 5,
    }
}

fn mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan => "plan",
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::Auto => "auto",
        PermissionMode::DontAsk => "dont-ask",
        PermissionMode::BypassPermissions => "bypass",
    }
}

#[cfg(test)]
mod tests {
    use hyprlayer_models::{AgentCaller, AgentControlLevel, Harness, PermissionMode};

    use super::super::{test_runtime, AgentIdentity};
    use crate::error::CoreError;
    use crate::pipeline::Stage;
    use crate::runtime::HyprlayerRuntime;
    use crate::store::{NewWorkstream, WorktreeMode};

    const PROJECT: &str = "/projects/app";

    async fn runtime(root: &std::path::Path) -> std::sync::Arc<HyprlayerRuntime> {
        let rt = test_runtime(root).await;
        rt.settings
            .update(|s| s.agent_control.level = AgentControlLevel::Full)
            .await
            .unwrap();
        rt
    }

    fn workstream(rt: &HyprlayerRuntime, slug: &str, mode: WorktreeMode) -> String {
        rt.db
            .insert_workstream(NewWorkstream {
                project_path: PROJECT.into(),
                title: slug.into(),
                slug: slug.into(),
                branch: slug.into(),
                worktree_path: format!("{PROJECT}/{slug}"),
                base_branch: "origin/main".into(),
                worktree_mode: mode,
            })
            .unwrap()
            .id
    }

    fn caller(workstream_id: &str, lead: bool) -> AgentIdentity {
        AgentIdentity {
            caller: AgentCaller::Session {
                session_id: "caller".into(),
            },
            workstream_id: workstream_id.into(),
            project_path: PROJECT.into(),
            depth: 0,
            parent: None,
            launch_mode: Some(PermissionMode::Default),
            report_only: false,
            harness: None,
            lead,
        }
    }

    #[tokio::test]
    async fn a_child_runs_the_model_it_names_else_the_default_for_its_harness() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path()).await;
        rt.settings
            .update(|s| {
                s.providers.claude.default_model = Some("haiku".into());
                s.providers.codex.default_model = Some("gpt-5.5".into());
            })
            .await
            .unwrap();
        let control = rt.agent_control();
        let model =
            |asked: Option<&str>, harness| control.child_model(asked.map(str::to_string), harness);
        assert_eq!(
            model(None, Some(Harness::Claude)).await.as_deref(),
            Some("haiku")
        );
        assert_eq!(
            model(None, Some(Harness::Codex)).await.as_deref(),
            Some("gpt-5.5")
        );
        assert_eq!(
            model(Some("sonnet"), Some(Harness::Claude))
                .await
                .as_deref(),
            Some("sonnet")
        );
        assert_eq!(model(None, None).await, None);
    }

    async fn spawn_into(
        rt: &HyprlayerRuntime,
        identity: &AgentIdentity,
        workstream_id: &str,
    ) -> crate::error::Result<()> {
        rt.agent_control()
            .check_spawn(identity, workstream_id, Stage::General, None, None, &[])
            .await
            .map(drop)
    }

    #[tokio::test]
    async fn a_lead_cannot_spawn_into_its_own_workstream() {
        let root = tempfile::tempdir().unwrap();
        let rt = runtime(root.path()).await;
        let main = workstream(&rt, "main", WorktreeMode::Inplace);
        let lead = caller(&main, true);

        let refused = spawn_into(&rt, &lead, &main).await.unwrap_err();

        assert!(matches!(refused, CoreError::LeadOwnWorkstream));
        assert_eq!(refused.code(), "agent_control_denied");
        assert_eq!(
            refused.remedy().as_deref(),
            Some("Call create_workstream first, then spawn_agent with its workstream_id.")
        );
    }

    #[tokio::test]
    async fn a_lead_spawns_into_a_workstream_it_created() {
        let root = tempfile::tempdir().unwrap();
        let rt = runtime(root.path()).await;
        let main = workstream(&rt, "main", WorktreeMode::Inplace);
        let created = workstream(&rt, "fix-1", WorktreeMode::Isolated);

        spawn_into(&rt, &caller(&main, true), &created)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_agent_that_is_not_a_lead_spawns_into_its_own_workstream() {
        let root = tempfile::tempdir().unwrap();
        let rt = runtime(root.path()).await;
        let own = workstream(&rt, "fix-1", WorktreeMode::Isolated);

        spawn_into(&rt, &caller(&own, false), &own).await.unwrap();
    }
}
