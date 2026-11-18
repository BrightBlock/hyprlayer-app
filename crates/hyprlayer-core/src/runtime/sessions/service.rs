use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use hyprlayer_models::goal::GoalEndReason;
use hyprlayer_models::session::QuestionAnswers;
use hyprlayer_models::{AgentCaller, SessionOrigin};
use tokio::sync::{broadcast, mpsc};

use crate::agent::{AgentDriver, Harness};
use crate::agents;
use crate::capability::{launch_policy, LaunchPolicy};
use crate::claude::{
    memory, ClaudeDriver, PermissionMode, SessionEvent, SessionState, SpawnConfig,
};
use crate::codex::{
    config_sets_developer_instructions, CodexDriver, CollabMode, SpawnConfig as CodexSpawnConfig,
};
use crate::error::{CoreError, Result};
use crate::opencode::runtime::OpenCodeDriver;
use crate::opencode::support::opencode_model_is_supported;
use crate::pipeline::{self, Stage};
use crate::process::binaries::{parse_semver, BinaryKind};
use crate::review::sidecar;
use crate::runtime::agent_control::{inject, AgentIdentity, TokenGuard};
use crate::runtime::doc_status;
use crate::runtime::HyprlayerRuntime;
use crate::skills;
use crate::store::{LogCursor, NewSession, Session, SessionLogEntry, Workstream};
use hyprlayer_models::{ClientRef, RuntimeEvent, SessionRev, SessionTurns};
use serde::{Deserialize, Serialize};

use super::followup::{self, Followup, FollowupTurn};
use super::goal_loop::is_idle;
use super::recap;
use super::workflow_bridge::{self, WorkflowChannel};
use super::SessionAttachment;

/// How often a cap lift looks again at a session that was mid-turn.
const LIFT_RETRY: Duration = Duration::from_millis(500);

pub struct SessionService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn sessions(&self) -> SessionService<'_> {
        SessionService { runtime: self }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

pub struct StartStageSessionRequest {
    pub workstream: Workstream,
    pub stage: Stage,
    pub input: String,
    pub attached_docs: Vec<String>,
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub prompt_name: Option<String>,
    pub client_message_id: Option<String>,
    /// A person's link to the session this one follows from, such as a validation's implement session.
    pub parent_session_id: Option<String>,
}

/// Who started a session, as its row records it.
pub struct SessionLineage {
    pub origin: SessionOrigin,
    pub parent_session_id: Option<String>,
    /// A spawning terminal and the child's depth; no row can name a terminal, so it is kept in memory.
    pub parent_terminal: Option<(String, u32)>,
}

impl SessionLineage {
    pub fn user() -> Self {
        Self {
            origin: SessionOrigin::User,
            parent_session_id: None,
            parent_terminal: None,
        }
    }

    pub fn goal() -> Self {
        Self {
            origin: SessionOrigin::Goal,
            parent_session_id: None,
            parent_terminal: None,
        }
    }
}

pub struct StartedStageSession {
    pub session: Session,
    pub prompt: String,
    pub launch: LaunchAndConverseParams,
}

pub struct LaunchAndConverseParams {
    session: Session,
    project_path: String,
    wire_prompt: String,
    stage: Stage,
    prompt_name: Option<String>,
    client_message_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
    bodies: pipeline::ActionPromptBodies,
    extra_allowed_tools: Vec<String>,
    extra_disallowed_tools: Vec<String>,
    report_only: bool,
}

impl LaunchAndConverseParams {
    /// Claude allow rules on top of the stage's; other harnesses ignore them.
    pub(crate) fn allow_also(&mut self, rules: Vec<String>) {
        self.extra_allowed_tools.extend(rules);
    }

    /// Claude may not edit files or sync thoughts; other harnesses ignore it.
    pub(crate) fn report_only(&mut self) {
        self.extra_disallowed_tools.extend(
            pipeline::REPORT_ONLY_DENY
                .iter()
                .map(|rule| rule.to_string()),
        );
        self.report_only = true;
    }

    /// A line ahead of the first turn, so the harness knows who is asking.
    pub(crate) fn prefix_prompt(&mut self, header: &str) {
        self.wire_prompt = format!("{header}\n\n{}", self.wire_prompt);
    }

    fn into_parts(self) -> (LaunchSessionParams, FirstTurn) {
        let followup = followup::followup_for(self.session.harness, self.stage, &self.bodies);
        let first = FirstTurn {
            session_id: self.session.id.clone(),
            wire_prompt: self.wire_prompt,
            stage: self.stage,
            prompt_name: self.prompt_name,
            client_message_id: self.client_message_id,
            followup,
        };
        let launch = LaunchSessionParams {
            session: self.session,
            project_path: self.project_path,
            model: self.model,
            effort: self.effort,
            permission_mode: self.permission_mode,
            resume: false,
            boundary_on_relaunch: false,
            max_budget_usd: None,
            extra_allowed_tools: self.extra_allowed_tools,
            extra_disallowed_tools: self.extra_disallowed_tools,
            report_only: self.report_only,
        };
        (launch, first)
    }
}

/// A new session's first send and the follow-up its stage asks for after it.
struct FirstTurn {
    session_id: String,
    wire_prompt: String,
    stage: Stage,
    prompt_name: Option<String>,
    client_message_id: Option<String>,
    followup: Option<Followup>,
}

pub struct LaunchSessionParams {
    pub session: Session,
    pub project_path: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub resume: bool,
    pub boundary_on_relaunch: bool,
    /// Claude's `--max-budget-usd`; other harnesses ignore it.
    pub max_budget_usd: Option<f64>,
    /// Claude allow rules on top of the stage's; other harnesses ignore them.
    pub extra_allowed_tools: Vec<String>,
    /// Claude deny rules on top of the stage's; other harnesses ignore them.
    pub extra_disallowed_tools: Vec<String>,
    /// Withholds Validate's thoughts edit rules from Claude.
    pub report_only: bool,
}

pub struct RewindSessionRequest {
    pub session: Session,
    pub workstream: Workstream,
    pub user_turn: u32,
    pub permission_mode: Option<PermissionMode>,
}

pub struct RewindSessionOutcome {
    pub session: Session,
    pub live_rewound: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveSession {
    pub session: Session,
    pub workstream: Workstream,
}

/// What a send is checked against and attributed to; the check counts only other authors' turns.
#[derive(Debug, Clone, Default)]
pub struct SendContext {
    pub base_rev: Option<u64>,
    pub author: Option<ClientRef>,
}

/// A turn the app sends on its own: a stage follow-up or a goal iteration.
pub(crate) struct AppTurn<'a> {
    pub text: &'a str,
    pub prompt_name: Option<&'a str>,
    pub author: Option<ClientRef>,
    /// Minted by the caller, so a goal's iteration row can exist before the message echoes.
    pub nonce: String,
    /// Applied under the same op lock, just before the send.
    pub collab_mode: Option<CollabMode>,
}

/// Proof that the session's op lock is held.
pub(crate) type OpGuard<'a> = tokio::sync::MutexGuard<'a, ()>;

impl SessionService<'_> {
    pub(crate) async fn live_driver(&self, session_id: &str) -> Option<Arc<dyn AgentDriver>> {
        self.runtime.session_registry.get(session_id).await
    }

    async fn require_live(&self, session_id: &str) -> Result<Arc<dyn AgentDriver>> {
        self.live_driver(session_id)
            .await
            .ok_or_else(|| CoreError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub async fn is_live(&self, session_id: &str) -> bool {
        self.live_driver(session_id).await.is_some()
    }

    pub async fn send(
        &self,
        session_id: &str,
        text: &str,
        attached_docs: &[String],
        prompt_name: Option<&str>,
        client_message_id: Option<&str>,
        ctx: SendContext,
    ) -> Result<()> {
        let op = self.op_lock(session_id).await;
        let _op_guard = op.lock().await;

        let prompt = pipeline::append_docs(text.to_string(), attached_docs);
        let driver = self.require_live(session_id).await?;

        let nonce = self
            .reserve_turn(session_id, client_message_id, &ctx)
            .await?;
        // A session someone is still talking to belongs back in the list.
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        if let Err(e) = blocking(move || db.set_session_archived(&id, false)).await {
            // The driver never ran, so no UserMessage will release it.
            if let Some(nonce) = &nonce {
                self.runtime
                    .session_actors
                    .release_user_turn(session_id, nonce.clone())
                    .await;
            }
            return Err(e);
        }
        // A failure from here keeps the reservation: the driver broadcast its UserMessage first.
        driver.send_user_turn(
            &prompt,
            None,
            prompt_name,
            nonce.as_deref().or(client_message_id),
        )
    }

    /// The one send path for app-originated turns.
    pub(crate) async fn send_app_turn(&self, session_id: &str, turn: AppTurn<'_>) -> Result<()> {
        let op = self.op_lock(session_id).await;
        let guard = op.lock().await;
        self.send_app_turn_held(session_id, turn, &guard).await
    }

    /// For a caller holding the op lock. Always reserved, author or not, so another client's send waits until this one lands.
    pub(crate) async fn send_app_turn_held(
        &self,
        session_id: &str,
        turn: AppTurn<'_>,
        _held: &OpGuard<'_>,
    ) -> Result<()> {
        let driver = self.require_live(session_id).await?;
        self.runtime
            .session_actors
            .reserve_user_turn(session_id, turn.nonce.clone(), None, turn.author)
            .await?;
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        if let Err(e) = blocking(move || db.set_session_archived(&id, false)).await {
            // The driver never ran, so no UserMessage will release it.
            self.runtime
                .session_actors
                .release_user_turn(session_id, turn.nonce)
                .await;
            return Err(e);
        }
        if let Some(mode) = turn.collab_mode {
            driver.set_collab_mode(Some(mode));
        }
        driver.send_user_turn(turn.text, None, turn.prompt_name, Some(&turn.nonce))
    }

    /// Reserves a checked or attributed send under a nonce the actor can match.
    async fn reserve_turn(
        &self,
        session_id: &str,
        client_message_id: Option<&str>,
        ctx: &SendContext,
    ) -> Result<Option<String>> {
        if ctx.base_rev.is_none() && ctx.author.is_none() {
            return Ok(None);
        }
        let nonce =
            client_message_id.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_string);
        self.runtime
            .session_actors
            .reserve_user_turn(session_id, nonce.clone(), ctx.base_rev, ctx.author.clone())
            .await?;
        Ok(Some(nonce))
    }

    pub async fn respond_permission(
        &self,
        session_id: &str,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.require_live(session_id)
            .await?
            .respond_permission(request_id, allow, message)
    }

    pub async fn answer(
        &self,
        session_id: &str,
        request_id: &str,
        answers: QuestionAnswers,
    ) -> Result<()> {
        self.require_live(session_id)
            .await?
            .answer_question(request_id, answers)
    }

    pub async fn interrupt(&self, session_id: &str) -> Result<()> {
        tracing::info!("session {session_id}: interrupt requested");
        let driver = self.require_live(session_id).await?;
        self.runtime.goals().note_user_interrupt(session_id).await;
        driver.interrupt()
    }

    /// False once the harness took the message in, or when it cannot withdraw one.
    pub async fn cancel_queued_message(
        &self,
        session_id: &str,
        client_message_id: &str,
    ) -> Result<bool> {
        let driver = self.require_live(session_id).await?;
        driver.cancel_queued_message(client_message_id).await
    }

    pub async fn kill(&self, session_id: &str) -> Result<()> {
        tracing::info!("session {session_id}: kill requested");
        let driver = self.require_live(session_id).await?;
        self.runtime.goals().note_session_killed(session_id).await;
        driver.kill();
        Ok(())
    }

    /// The OpenCode check folded in from the shell (ADR-0009): a running
    /// server's own registry decides which models exist.
    pub async fn set_model(&self, session_id: &str, model: Option<String>) -> Result<()> {
        if let Some(model_id) = model.as_deref() {
            let session =
                self.get(session_id)
                    .await?
                    .ok_or_else(|| CoreError::SessionNotFound {
                        session_id: session_id.to_string(),
                    })?;
            if session.harness == Harness::Opencode {
                if !opencode_model_is_supported(model_id) {
                    return Err(CoreError::OpencodeProviderUnsupported);
                }
                if let Some(server) = self.runtime.providers().opencode_server_if_running().await {
                    if server.knows_model(model_id).await == Some(false) {
                        return Err(CoreError::OpencodeModelUnavailable {
                            model: model_id.to_string(),
                        });
                    }
                }
            }
        }
        if let Some(driver) = self.live_driver(session_id).await {
            driver.set_model(model);
        }
        Ok(())
    }

    pub async fn set_effort(&self, session_id: &str, effort: Option<String>) -> Result<()> {
        if let Some(driver) = self.live_driver(session_id).await {
            driver.set_effort(effort);
        }
        Ok(())
    }

    pub async fn set_permission_mode(
        &self,
        session_id: &str,
        mode: Option<PermissionMode>,
    ) -> Result<()> {
        let driver = self.require_live(session_id).await?;
        let mode = match mode {
            Some(mode) => mode,
            None => pipeline::policy_for(self.require_row(session_id).await?.stage).permission_mode,
        };
        driver.set_permission_mode(mode)?;
        self.runtime
            .session_registry
            .set_permission_mode(session_id, mode)
            .await;
        Ok(())
    }

    pub async fn permission_mode(&self, session_id: &str) -> Option<PermissionMode> {
        self.runtime
            .session_registry
            .permission_mode(session_id)
            .await
    }

    pub async fn op_lock(&self, session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.runtime.session_registry.op_lock(session_id).await
    }

    pub async fn list_active_ids(&self) -> Vec<String> {
        self.runtime.session_registry.live_ids().await
    }

    /// `Ok(None)` for a session id with no stored row — callers decide whether
    /// that is an error, because a session that has ended is expected in some
    /// commands and a failure in others.
    pub async fn get(&self, session_id: &str) -> Result<Option<Session>> {
        let id = session_id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_session(&id)).await
    }

    /// A blank title clears the row rather than storing whitespace; a missing
    /// session id updates nothing and succeeds.
    pub async fn set_title(&self, session_id: &str, title: Option<String>) -> Result<()> {
        let title = title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let id = session_id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.set_session_title(&id, title.as_deref())).await
    }

    /// Only an idle or exited session can be archived; unarchiving always works.
    pub async fn set_archived(&self, session_id: &str, archived: bool) -> Result<()> {
        let op = self.op_lock(session_id).await;
        let _op_guard = op.lock().await;
        let session = self.require_row(session_id).await?;
        if archived {
            self.refuse_live_goal(session_id).await?;
        }
        if archived && session.working {
            return Err(CoreError::SessionBusy {
                session_id: session_id.to_string(),
            });
        }
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        blocking(move || db.set_session_archived(&id, archived)).await?;
        self.announce(session);
        Ok(())
    }

    /// Removes an archived, exited session and its log for every client.
    pub async fn delete(&self, session_id: &str) -> Result<()> {
        let op = self.op_lock(session_id).await;
        let _op_guard = op.lock().await;
        let session = self.require_row(session_id).await?;
        self.refuse_live_goal(session_id).await?;
        if session.archived_at.is_none() {
            return Err(CoreError::SessionNotArchived {
                session_id: session_id.to_string(),
            });
        }
        if session.working || self.is_live(session_id).await {
            return Err(CoreError::SessionBusy {
                session_id: session_id.to_string(),
            });
        }
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        blocking(move || db.delete_session(&id)).await?;
        self.runtime.agent_links.unlink(session_id);
        self.runtime.session_actors.remove(session_id).await;
        self.announce(session);
        Ok(())
    }

    /// Removes a session whose harness never started and that no client was handed.
    pub(crate) async fn discard_unlaunched(&self, session: &Session) {
        self.runtime.agent_links.unlink(&session.id);
        let (db, id) = (self.runtime.db.clone(), session.id.clone());
        if let Err(e) = blocking(move || db.delete_session(&id)).await {
            tracing::warn!("session {}: its unlaunched row stays: {e}", session.id);
            return;
        }
        self.runtime.session_actors.remove(&session.id).await;
        self.announce(session.clone());
    }

    /// Archiving or deleting a session would strand its goal's runner.
    async fn refuse_live_goal(&self, session_id: &str) -> Result<()> {
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        match blocking(move || db.live_goal_for_session(&id)).await? {
            Some(_) => Err(CoreError::GoalActive {
                session_id: session_id.to_string(),
            }),
            None => Ok(()),
        }
    }

    async fn require_row(&self, session_id: &str) -> Result<Session> {
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        blocking(move || db.get_session(&id))
            .await?
            .ok_or_else(|| CoreError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    /// Clients refetch session lists on any lifecycle event.
    fn announce(&self, session: Session) {
        let _ = self.runtime.events.send(RuntimeEvent::SessionLifecycle {
            session_id: session.id,
            workstream_id: session.workstream_id,
            state: session.state,
        });
    }

    /// Also queues untitled sessions for a title, so only workstreams a client
    /// actually opens pay for backfill.
    pub async fn list(&self, workstream_id: &str) -> Result<Vec<Session>> {
        let id = workstream_id.to_string();
        let db = self.runtime.db.clone();
        let sessions = blocking(move || db.list_sessions(&id)).await?;
        // A preview means there is user text to title from.
        for session in sessions.iter().rev() {
            if session.title.is_none() && session.preview.is_some() && session.archived_at.is_none()
            {
                self.runtime.session_titler.enqueue(&session.id, false);
            }
        }
        Ok(sessions)
    }

    /// `None` when recaps are off or the last turn did nothing worth recapping.
    pub async fn recap(&self, session_id: &str) -> Result<Option<String>> {
        let settings = self.runtime.settings.get().await;
        if !settings.recaps.enabled {
            return Ok(None);
        }
        let id = session_id.to_string();
        let db = self.runtime.db.clone();
        // Only the last turn: from its user message on.
        let tail = blocking(move || {
            let turns = db.count_user_messages(&id)?;
            let from = db.nth_user_message_seq(&id, turns)?;
            db.session_log_after(&id, from.map(|seq| seq - 1))
        })
        .await?;
        let events: Vec<SessionEvent> = tail.into_iter().map(|e| e.event).collect();
        let Some(excerpt) = recap::recap_excerpt(&events) else {
            return Ok(None);
        };
        let model = settings.tab_titles.effective_model(&settings.providers);
        let reply = self
            .runtime
            .providers()
            .one_shot(
                model,
                recap::RECAP_SYSTEM,
                &recap::recap_prompt(&excerpt),
                recap::RECAP_TIMEOUT,
            )
            .await?;
        let text = reply.text.trim();
        Ok((!text.is_empty()).then(|| text.to_string()))
    }

    pub async fn log(&self, session_id: &str) -> Result<Vec<SessionLogEntry>> {
        let id = session_id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.session_log(&id)).await
    }

    /// Every live session with its workstream, newest first. Ids whose session
    /// or workstream row has since been deleted are skipped rather than failing
    /// the whole listing.
    pub async fn list_active(&self) -> Result<Vec<ActiveSession>> {
        let ids = self.list_active_ids().await;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.runtime.db.clone();
        blocking(move || {
            let mut out = Vec::new();
            for id in ids {
                let Some(session) = db.get_session(&id)? else {
                    continue;
                };
                let Some(workstream) = db.get_workstream(&session.workstream_id)? else {
                    continue;
                };
                out.push(ActiveSession {
                    session,
                    workstream,
                });
            }
            out.sort_by(|a, b| {
                b.session
                    .started_at
                    .partial_cmp(&a.session.started_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Ok(out)
        })
        .await
    }

    pub async fn attach(
        &self,
        session_id: &str,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        self.runtime
            .session_actors
            .attach(session_id, echo_user_messages)
            .await
    }

    pub async fn log_after(
        &self,
        session_id: &str,
        epoch: u64,
        after_seq: Option<i64>,
    ) -> Result<Vec<SessionLogEntry>> {
        self.runtime
            .session_actors
            .log_after(session_id, epoch, after_seq)
            .await
    }

    /// Resumes a lagged reader: everything persisted after `after_seq`, plus a
    /// live attachment that starts exactly where that replay ends.
    pub async fn attach_after(
        &self,
        session_id: &str,
        epoch: u64,
        after_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        self.runtime
            .session_actors
            .attach_after(session_id, epoch, after_seq, echo_user_messages)
            .await
    }

    /// The cursor-verified replay and a live attachment from that row, in one
    /// actor turn: closes the gap between a client's log read and its attach.
    pub async fn attach_from(
        &self,
        session_id: &str,
        cursor: LogCursor,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        self.runtime
            .session_actors
            .attach_from(session_id, cursor, echo_user_messages)
            .await
    }

    pub async fn list_turns(
        &self,
        session_id: &str,
        cursor: LogCursor,
        limit: u32,
        caller: Option<&str>,
    ) -> Result<SessionTurns> {
        self.runtime
            .session_actors
            .list_turns(session_id, cursor, limit, caller.map(str::to_string))
            .await
    }

    /// Each watched session's revision state for `caller`; ids with no row are omitted.
    pub async fn revs(&self, ids: &[String], caller: Option<&str>) -> Result<Vec<SessionRev>> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            // The row decides existence: an actor can outlive a deleted session.
            let (db, sid) = (self.runtime.db.clone(), id.clone());
            let Some((rev, reset_rev, working)) =
                blocking(move || db.session_rev_state(&sid)).await?
            else {
                continue;
            };
            let live = self
                .runtime
                .session_actors
                .rev_state(id, caller.map(str::to_string))
                .await;
            let (rev, reset_rev, working, in_flight) = match live {
                Some(s) => (s.rev, s.reset_rev, s.working, s.in_flight),
                None => (rev, reset_rev, working, 0),
            };
            out.push(SessionRev {
                session_id: id.clone(),
                rev,
                reset_rev,
                working,
                in_flight,
                viewers: Vec::new(),
            });
        }
        Ok(out)
    }

    pub async fn kill_and_await_exit(&self, session_id: &str) {
        if let Some(driver) = self.runtime.session_registry.get(session_id).await {
            tracing::info!("session {session_id}: stopping its harness to relaunch it");
            driver.kill_detached();
            let mut waited_ms = 0u64;
            while self
                .runtime
                .session_registry
                .get(session_id)
                .await
                .is_some()
                && waited_ms < 10_000
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                waited_ms += 50;
            }
            if waited_ms >= 10_000 {
                tracing::warn!(
                    "session {session_id}: the harness had not exited after 10s; relaunching anyway"
                );
            }
        }
    }

    /// For a relaunch no client asked for: open streams carry on to its `Relaunched` instead of ending on the exit.
    pub(crate) async fn kill_for_relaunch(&self, session_id: &str) {
        if self
            .runtime
            .session_registry
            .get(session_id)
            .await
            .is_some()
        {
            if let Err(e) = self
                .runtime
                .session_actors
                .keep_streams_across_exit(session_id)
                .await
            {
                tracing::warn!("session {session_id}: its streams end with the relaunch: {e}");
            }
        }
        self.kill_and_await_exit(session_id).await;
        // A harness still running relaunches nothing, so its exit must end the streams.
        if self.is_live(session_id).await {
            self.end_kept_streams(session_id).await;
        }
    }

    /// Ends the streams `kill_for_relaunch` kept open when no relaunch followed.
    pub(crate) async fn end_kept_streams(&self, session_id: &str) {
        if let Err(e) = self
            .runtime
            .session_actors
            .end_kept_streams(session_id)
            .await
        {
            tracing::warn!("session {session_id}: could not end its kept streams: {e}");
        }
    }

    /// Relaunches the harness a goal capped, without the cap, once no turn runs: the cap is sticky and would refuse the user's next message.
    pub(crate) async fn lift_budget_cap(
        &self,
        session_id: &str,
        goal_id: &str,
        owned: &Weak<dyn AgentDriver>,
    ) {
        loop {
            let op = self.op_lock(session_id).await;
            let guard = op.lock().await;
            if !self.cap_still_owned(session_id, goal_id, owned).await {
                return;
            }
            if is_idle(&self.runtime.session_actors, session_id).await {
                self.relaunch_uncapped(session_id, &guard).await;
                return;
            }
            drop(guard);
            tokio::time::sleep(LIFT_RETRY).await;
        }
    }

    /// False once another goal runs the session (its runner handles the cap), this one ended with the session or its workstream, or the capped harness is gone.
    async fn cap_still_owned(
        &self,
        session_id: &str,
        goal_id: &str,
        owned: &Weak<dyn AgentDriver>,
    ) -> bool {
        let (db, sid, gid) = (
            self.runtime.db.clone(),
            session_id.to_string(),
            goal_id.to_string(),
        );
        let read = blocking(move || Ok((db.live_goal_for_session(&sid)?, db.goal(&gid)?))).await;
        let (live, own) = match read {
            Ok(goals) => goals,
            Err(e) => {
                tracing::warn!("session {session_id}: its budget cap stays: {e}");
                return false;
            }
        };
        if live.is_some_and(|row| row.goal.id != goal_id) {
            return false;
        }
        let kept_down = own.is_some_and(|row| {
            matches!(
                row.goal.end_reason,
                Some(GoalEndReason::SessionEnded | GoalEndReason::WorkstreamArchived)
            )
        });
        if kept_down {
            return false;
        }
        match (owned.upgrade(), self.live_driver(session_id).await) {
            (Some(owned), Some(live)) => Arc::ptr_eq(&owned, &live),
            _ => false,
        }
    }

    async fn relaunch_uncapped(&self, session_id: &str, held: &OpGuard<'_>) {
        let session = match self.require_row(session_id).await {
            Ok(session) => session,
            Err(e) => {
                tracing::warn!("session {session_id}: its budget cap stays: {e}");
                return;
            }
        };
        let project_path = match self
            .runtime
            .workstreams()
            .find(&session.workstream_id)
            .await
        {
            Ok(workstream) => workstream.project_path,
            Err(e) => {
                tracing::warn!("session {session_id}: its budget cap stays: {e}");
                return;
            }
        };
        // The started model and a mode the user switched to live both survive the relaunch.
        let permission_mode = self.permission_mode(session_id).await;
        self.kill_for_relaunch(session_id).await;
        let model = session.model.clone();
        let params = LaunchSessionParams {
            session,
            project_path,
            model,
            effort: None,
            permission_mode,
            resume: true,
            boundary_on_relaunch: true,
            max_budget_usd: None,
            extra_allowed_tools: Vec::new(),
            extra_disallowed_tools: Vec::new(),
            report_only: false,
        };
        if let Err(e) = self.launch_if_exited_held(params, held).await {
            self.end_kept_streams(session_id).await;
            tracing::warn!("session {session_id}: the relaunch without its budget cap failed: {e}");
        }
    }

    async fn thoughts_add_dirs(&self, project_path: &str) -> Result<Vec<PathBuf>> {
        let loaded = self.runtime.config.load().await?;
        let thoughts = loaded
            .config
            .thoughts
            .as_ref()
            .ok_or(CoreError::ThoughtsNotConfigured)?;
        let effective = thoughts.effective_config_for(project_path);
        Ok(effective.backend.thoughts_root().into_iter().collect())
    }

    async fn memory_instructions(&self, workstream_id: &str) -> Option<String> {
        let listing = self
            .runtime
            .workstreams()
            .project_memory(workstream_id)
            .await
            .ok()?;
        if listing.key_check == memory::MemoryKeyCheck::Mismatch {
            return None;
        }
        let dir = PathBuf::from(listing.dir);
        let codex_config = self.runtime.paths.codex_home.join("config.toml");
        blocking(move || {
            // A thread's developerInstructions replaces a configured
            // developer_instructions rather than merging (observed on codex
            // 0.154.0), so a user who keeps their own gets no memory instead
            // of silently losing their rules.
            let user_has_own = std::fs::read_to_string(&codex_config)
                .is_ok_and(|text| config_sets_developer_instructions(&text));
            if user_has_own {
                return Ok(None);
            }
            Ok(memory::read_index(&dir).map(|index| memory::developer_instructions(&dir, &index)))
        })
        .await
        .ok()
        .flatten()
    }

    /// Review notes ride along only for Implement, and never over a user's own instructions.
    async fn codex_developer_instructions(&self, session: &Session) -> Option<String> {
        let memory = self.memory_instructions(&session.workstream_id).await;
        if session.stage != Stage::Implement {
            return memory;
        }
        let codex_config = self.runtime.paths.codex_home.join("config.toml");
        let user_has_own = blocking(move || {
            Ok(std::fs::read_to_string(&codex_config)
                .is_ok_and(|text| config_sets_developer_instructions(&text)))
        })
        .await
        .unwrap_or(true);
        if user_has_own {
            return memory;
        }
        let notes = pipeline::review_notes_instruction(&session.workstream_id);
        Some(match memory {
            Some(memory) => format!("{memory}\n\n{notes}"),
            None => notes,
        })
    }

    async fn prompt_vars_for(&self, ws: &Workstream, template: &str) -> pipeline::PromptVars {
        let want_changed = template.contains("{{changed_files}}");
        let want_diff = template.contains("{{diff}}");
        let want_last = template.contains("{{last_commit}}");
        let git_vars = if want_changed || want_diff || want_last {
            let worktree = ws.worktree_path.clone();
            tokio::task::spawn_blocking(move || {
                crate::gitops::prompt_git_vars(
                    std::path::Path::new(&worktree),
                    want_changed,
                    want_diff,
                    want_last,
                )
            })
            .await
            .unwrap_or_default()
        } else {
            Default::default()
        };
        pipeline::PromptVars {
            branch: ws.branch.clone(),
            base_branch: ws
                .base_branch
                .strip_prefix("origin/")
                .unwrap_or(&ws.base_branch)
                .to_string(),
            changed_files: git_vars.changed_files,
            diff: git_vars.diff,
            last_commit: git_vars.last_commit,
        }
    }

    async fn action_prompt_bodies(
        &self,
        stage: Stage,
        ws: &Workstream,
    ) -> pipeline::ActionPromptBodies {
        let key = match stage {
            Stage::Commit => "commit",
            Stage::Ship => "ship",
            _ => return Default::default(),
        };
        let prompts = self.runtime.settings.get().await.prompts;
        let Some(template) = prompts
            .builtin_bodies
            .get(key)
            .filter(|t| !t.trim().is_empty())
        else {
            return Default::default();
        };
        let vars = self.prompt_vars_for(ws, template).await;
        let body = pipeline::substitute_vars(template, &vars);
        match stage {
            Stage::Commit => pipeline::ActionPromptBodies {
                commit: Some(body),
                ship: None,
            },
            _ => pipeline::ActionPromptBodies {
                commit: None,
                ship: Some(body),
            },
        }
    }

    /// A PTY agent's first turn for an app action, with the side effects its native start has.
    pub(crate) async fn pty_first_prompt(
        &self,
        ws: &Workstream,
        harness: Option<Harness>,
        req: hyprlayer_models::PtyFirstPrompt,
    ) -> String {
        let bodies = self.action_prompt_bodies(req.stage, ws).await;
        let prompt = pipeline::pty_prompt(
            req.stage,
            &req.input,
            &ws.base_branch,
            &bodies,
            harness,
            &ws.id,
        );
        self.stage_start_effects(ws, req.stage, &req.attached_docs, &req.input)
            .await;
        pipeline::append_docs(prompt, &req.attached_docs)
    }

    /// What starting a stage does besides its prompt, for a native session and a PTY agent alike.
    async fn stage_start_effects(
        &self,
        ws: &Workstream,
        stage: Stage,
        attached_docs: &[String],
        input: &str,
    ) {
        if stage == Stage::Implement {
            let worktree = ws.worktree_path.clone();
            let excluded = blocking(move || {
                Ok(sidecar::ensure_review_dir_excluded(std::path::Path::new(
                    &worktree,
                ))?)
            })
            .await;
            if let Err(e) = excluded {
                tracing::warn!(
                    "workstream {}: could not exclude the review dir: {e}",
                    ws.id
                );
            }
        }
        // Any route into building a plan activates it, not only the Approve & build button.
        if matches!(stage, Stage::Implement | Stage::Validate) {
            doc_status::activate_plans(
                &self.runtime.config,
                &ws.worktree_path,
                attached_docs,
                input,
            )
            .await;
        }
    }

    pub async fn prepare_start(
        &self,
        mut request: StartStageSessionRequest,
    ) -> Result<StartedStageSession> {
        let mut lineage = SessionLineage::user();
        if let Some(parent) = request.parent_session_id.take() {
            let (db, workstream_id) = (self.runtime.db.clone(), request.workstream.id.clone());
            let lookup = parent.clone();
            let found = blocking(move || db.get_session(&lookup)).await?;
            // A stale or foreign link is dropped, never fatal: the session still starts.
            if found.is_some_and(|s| s.workstream_id == workstream_id) {
                lineage.parent_session_id = Some(parent);
            } else {
                tracing::warn!(
                    "workstream {}: ignored a link to session {parent}, which isn't in it",
                    request.workstream.id
                );
            }
        }
        self.prepare_start_with(request, lineage).await
    }

    pub async fn prepare_start_with(
        &self,
        request: StartStageSessionRequest,
        lineage: SessionLineage,
    ) -> Result<StartedStageSession> {
        let StartStageSessionRequest {
            workstream,
            stage,
            input,
            attached_docs,
            harness,
            model,
            effort,
            permission_mode,
            prompt_name,
            client_message_id,
            parent_session_id: _,
        } = request;

        let harness = harness.unwrap_or_default();
        let first_input = Some(input.trim().to_string()).filter(|t| !t.is_empty());
        let bodies = self.action_prompt_bodies(stage, &workstream).await;
        let mut prompt =
            pipeline::compose_prompt(stage, &input, &workstream.base_branch, &bodies, harness);
        if let Some(suffix) = pipeline::implement_notes_suffix(harness, stage, &workstream.id) {
            prompt.push_str(&suffix);
        }
        self.stage_start_effects(&workstream, stage, &attached_docs, &input)
            .await;
        let wire_prompt = pipeline::append_docs(prompt.clone(), &attached_docs);

        let parent_terminal = lineage.parent_terminal.clone();
        let session = {
            let new = NewSession {
                id: uuid::Uuid::new_v4().to_string(),
                workstream_id: workstream.id.clone(),
                stage,
                harness,
                cwd: workstream.worktree_path.clone(),
                first_input,
            };
            let db = self.runtime.db.clone();
            blocking(move || {
                let mut session = db.insert_session(new)?;
                if lineage.origin != SessionOrigin::User || lineage.parent_session_id.is_some() {
                    db.set_session_lineage(
                        &session.id,
                        lineage.origin,
                        lineage.parent_session_id.as_deref(),
                    )?;
                    session.origin = lineage.origin;
                    session.parent_session_id = lineage.parent_session_id;
                }
                Ok(session)
            })
            .await?
        };
        // Before the launch: its identity's depth is read from the link.
        if let Some((terminal_id, depth)) = parent_terminal {
            self.runtime.agent_links.link(
                &session.id,
                AgentCaller::Terminal { terminal_id },
                depth,
            );
        }
        self.runtime.session_titler.enqueue(&session.id, true);
        if let Err(e) = self
            .runtime
            .workstreams()
            .advance_stage(&workstream.id, stage)
            .await
        {
            tracing::warn!("workstream {}: stage not advanced: {e}", workstream.id);
        }

        Ok(StartedStageSession {
            session: session.clone(),
            prompt: prompt.clone(),
            launch: LaunchAndConverseParams {
                session,
                project_path: workstream.project_path,
                wire_prompt,
                stage,
                prompt_name,
                client_message_id,
                model,
                effort,
                permission_mode,
                bodies,
                extra_allowed_tools: Vec::new(),
                extra_disallowed_tools: Vec::new(),
                report_only: false,
            },
        })
    }

    pub async fn launch_and_converse(
        &self,
        params: LaunchAndConverseParams,
        author: Option<ClientRef>,
    ) -> Result<()> {
        let (launch, first) = params.into_parts();
        let session_id = first.session_id.clone();
        let driver = self.launch_driver(launch).await?;
        let Some((mut rx, followup)) = self.send_first_turn(&driver, first, &author).await? else {
            return Ok(());
        };
        if let Some(turn) =
            followup::await_followup(&session_id, driver.as_ref(), &mut rx, followup).await
        {
            self.send_followup(&session_id, turn, author).await;
        }
        Ok(())
    }

    /// Launches and sends the first turn before returning, so a waiter never sees the session idle before it; only the follow-up runs on.
    pub(crate) async fn start_detached(
        &self,
        params: LaunchAndConverseParams,
        author: Option<ClientRef>,
    ) -> Result<()> {
        let (launch, first) = params.into_parts();
        let session = launch.session.clone();
        let driver = match self.launch_driver(launch).await {
            Ok(driver) => driver,
            Err(e) => {
                self.discard_unlaunched(&session).await;
                return Err(e);
            }
        };
        let Some((mut rx, followup)) = self.send_first_turn(&driver, first, &author).await? else {
            return Ok(());
        };
        let runtime = self.runtime.handle.clone();
        tokio::spawn(async move {
            let Some(turn) =
                followup::await_followup(&session.id, driver.as_ref(), &mut rx, followup).await
            else {
                return;
            };
            if let Some(runtime) = runtime.upgrade() {
                runtime
                    .sessions()
                    .send_followup(&session.id, turn, author)
                    .await;
            }
        });
        Ok(())
    }

    /// The follow-up's receiver is subscribed before the send, so the first turn's end is never missed.
    async fn send_first_turn(
        &self,
        driver: &Arc<dyn AgentDriver>,
        first: FirstTurn,
        author: &Option<ClientRef>,
    ) -> Result<Option<(broadcast::Receiver<SessionEvent>, Followup)>> {
        let FirstTurn {
            session_id,
            wire_prompt,
            stage,
            prompt_name,
            client_message_id,
            followup,
        } = first;
        let op_lock = self.op_lock(&session_id).await;
        let turns = followup.as_ref().map(|_| driver.subscribe());
        let first_send = {
            let _guard = op_lock.lock().await;
            let ctx = SendContext {
                base_rev: None,
                author: author.clone(),
            };
            let nonce = self
                .reserve_turn(&session_id, client_message_id.as_deref(), &ctx)
                .await?;
            if matches!(followup, Some(Followup::CodexExecute(_))) {
                driver.set_collab_mode(Some(CollabMode::Plan));
            }
            driver.send_user_turn(
                &wire_prompt,
                Some(stage.as_str()),
                prompt_name.as_deref(),
                nonce.as_deref().or(client_message_id.as_deref()),
            )
        };
        first_send?;
        Ok(turns.zip(followup))
    }

    async fn send_followup(&self, session_id: &str, turn: FollowupTurn, author: Option<ClientRef>) {
        let sent = self
            .send_app_turn(
                session_id,
                AppTurn {
                    text: turn.prompt,
                    prompt_name: None,
                    author,
                    nonce: uuid::Uuid::new_v4().to_string(),
                    collab_mode: turn.collab_mode,
                },
            )
            .await;
        if let Err(e) = sent {
            tracing::warn!("session {session_id}: the follow-up was not sent: {e}");
        }
    }

    pub async fn launch(&self, params: LaunchSessionParams) -> Result<()> {
        self.launch_driver(params).await.map(|_| ())
    }

    /// For a caller holding the op lock: launches only when no harness is live, as `ResumeSession` does; `None` means one was.
    pub(crate) async fn launch_if_exited_held(
        &self,
        params: LaunchSessionParams,
        _held: &OpGuard<'_>,
    ) -> Result<Option<Arc<dyn AgentDriver>>> {
        if self.is_live(&params.session.id).await {
            return Ok(None);
        }
        self.launch_driver(params).await.map(Some)
    }

    pub(crate) async fn launch_driver(
        &self,
        params: LaunchSessionParams,
    ) -> Result<Arc<dyn AgentDriver>> {
        let session = &params.session;
        let session_id = session.id.clone();
        let resume = params.resume;

        if resume {
            let db = self.runtime.db.clone();
            let id = session_id.clone();
            let _ = blocking(move || db.set_session_state(&id, SessionState::Running)).await;
        }

        let described = format!(
            "{:?} {:?} (model {}, effort {}, resume {resume})",
            session.harness,
            session.stage,
            params.model.as_deref().unwrap_or("default"),
            params.effort.as_deref().unwrap_or("default"),
        );
        let policy = launch_policy(session.harness, session.stage, params.permission_mode);
        // opencode's one shared server has no per-session identity to attach a token to.
        let identity = match session.harness {
            Harness::Opencode => None,
            Harness::Claude | Harness::Codex => {
                let control = self.runtime.agent_control();
                let (parent, depth) = control.session_lineage(session).await;
                control
                    .identity_env(AgentIdentity {
                        caller: AgentCaller::Session {
                            session_id: session_id.clone(),
                        },
                        workstream_id: session.workstream_id.clone(),
                        project_path: params.project_path.clone(),
                        depth,
                        parent,
                        launch_mode: Some(policy.permission_mode()),
                        report_only: params.report_only,
                        harness: Some(session.harness),
                    })
                    .await
            }
        };
        let (identity_env, token) = identity.unzip();
        let spawned = self
            .spawn_driver(&params, policy, identity_env.unwrap_or_default())
            .await;
        let (driver, rx, launched_mode) = match spawned {
            Ok(spawned) => spawned,
            Err(e) => {
                tracing::error!("session {session_id}: {described} failed to start: {e}");
                // Resume marked the row running before the spawn; put it back.
                if resume {
                    let db = self.runtime.db.clone();
                    let id = session_id.clone();
                    let previous = session.state;
                    let _ = blocking(move || db.set_session_state(&id, previous)).await;
                }
                return Err(e);
            }
        };
        tracing::info!(
            "session {session_id} (workstream {}): {described} started as pid {:?} in {}, mode {launched_mode:?}",
            session.workstream_id,
            driver.pid(),
            session.cwd
        );

        if let Some(pid) = driver.pid() {
            let db = self.runtime.db.clone();
            let sid = session_id.clone();
            if let Err(e) = blocking(move || db.record_orphan(pid as i64, &sid)).await {
                tracing::warn!("session {session_id}: could not record pid {pid}: {e}");
            }
        }

        self.runtime
            .session_registry
            .insert(session_id.clone(), driver.clone(), launched_mode)
            .await;

        let ingress = self.runtime.session_actors.ingress(&session_id).await;

        let has_history = if resume {
            let db = self.runtime.db.clone();
            let id = session_id.clone();
            blocking(move || db.next_session_seq(&id))
                .await
                .map(|seq| seq > 0)
                .unwrap_or(false)
        } else {
            false
        };

        let workflow = WorkflowChannel::new();
        self.runtime
            .session_actors
            .set_workflow(&session_id, workflow.clone())
            .await?;

        if has_history {
            if params.boundary_on_relaunch {
                let quiet = !self.runtime.take_cut_off(&session_id);
                let _ = ingress.send(SessionEvent::Relaunched { quiet }).await;
            }
            workflow_bridge::recover_workflow_runs(&self.runtime.db, &session_id, &workflow).await;
        }

        spawn_relay(
            rx,
            ingress,
            self.runtime.session_registry.clone(),
            session_id,
            token,
        );

        Ok(driver)
    }

    async fn spawn_driver(
        &self,
        params: &LaunchSessionParams,
        policy: LaunchPolicy,
        identity_env: Vec<(String, String)>,
    ) -> Result<(
        Arc<dyn AgentDriver>,
        broadcast::Receiver<SessionEvent>,
        Option<PermissionMode>,
    )> {
        let LaunchSessionParams {
            session,
            project_path,
            resume,
            max_budget_usd,
            extra_allowed_tools,
            extra_disallowed_tools,
            report_only,
            ..
        } = params;
        let (model, effort, resume) = (params.model.clone(), params.effort.clone(), *resume);
        // Empty at `off`: no identity, so no MCP server either.
        let endpoint = self
            .runtime
            .agent_control_endpoint
            .as_ref()
            .filter(|_| !identity_env.is_empty());
        match session.harness {
            Harness::Opencode => {
                if model
                    .as_deref()
                    .is_some_and(|model| !opencode_model_is_supported(model))
                {
                    return Err(CoreError::OpencodeProviderUnsupported);
                }
                let server = self.runtime.providers().opencode_server().await?;
                if let Some(model) = model.as_deref() {
                    if server.knows_model(model).await == Some(false) {
                        return Err(CoreError::OpencodeModelUnavailable {
                            model: model.to_string(),
                        });
                    }
                }
                let (driver, rx) = OpenCodeDriver::spawn(server, &session.cwd, model)
                    .await
                    .map_err(|detail| CoreError::Opencode { detail })?;
                let driver: Arc<dyn AgentDriver> = driver;
                Ok((driver, rx, None))
            }
            Harness::Codex => {
                let bin = self.runtime.binaries.require(BinaryKind::Codex).await?;
                let add_dirs = self.thoughts_add_dirs(project_path).await?;
                let LaunchPolicy::Codex {
                    permission_mode,
                    posture,
                } = policy
                else {
                    unreachable!("codex harness yields a codex launch policy")
                };

                let mut extra_args = Vec::new();
                if let Some(endpoint) = endpoint {
                    if self.codex_supports_mcp().await {
                        let read_only = permission_mode != PermissionMode::BypassPermissions;
                        match inject::codex_mcp_args(&endpoint.ctl, &identity_env, read_only) {
                            Some(args) => extra_args = args,
                            None => tracing::info!(
                                "codex session {} gets no MCP server: a value cannot be a TOML literal",
                                session.id
                            ),
                        }
                    }
                }
                let mut env = self.runtime.child_env().await;
                env.extend(identity_env);
                let mut skill_roots = skills::skill_roots(
                    Some(std::path::Path::new(&session.cwd)),
                    &self.runtime.skills().skill_home(),
                );
                if let Some(plugin) = endpoint.and_then(|endpoint| endpoint.plugin_dir.as_ref()) {
                    skill_roots.push(plugin.join("skills"));
                }

                let developer_instructions = self.codex_developer_instructions(session).await;

                let resume_thread = resume
                    .then(|| session.provider_session_id.clone())
                    .flatten();
                if resume && resume_thread.is_none() {
                    tracing::warn!(
                        "codex session {} has no recorded thread id; starting a new thread \
                         (the model will not have the replayed history)",
                        session.id
                    );
                }

                let prior_user_turns = if resume_thread.is_some() {
                    let db = self.runtime.db.clone();
                    let sid = session.id.clone();
                    blocking(move || db.count_user_messages(&sid)).await?
                } else {
                    0
                };

                let (driver, rx) = CodexDriver::spawn(CodexSpawnConfig {
                    codex_bin: bin,
                    cwd: PathBuf::from(&session.cwd),
                    posture,
                    add_dirs,
                    skill_roots,
                    resume: resume_thread,
                    prior_user_turns,
                    model,
                    effort,
                    ephemeral: false,
                    env,
                    developer_instructions,
                    extra_args,
                })
                .await?;
                // codex 0.156+ resumes a thread in its saved plan mode, and a
                // relaunch never runs phase two to leave it.
                if driver.observed_collab_mode().as_deref() == Some("plan") {
                    driver.set_collab_mode(Some(CollabMode::Default));
                }
                let driver: Arc<dyn AgentDriver> = Arc::new(driver);
                Ok((driver, rx, Some(permission_mode)))
            }
            Harness::Claude => {
                let claude = self.runtime.binaries.require(BinaryKind::Claude).await?;
                let add_dirs = self.thoughts_add_dirs(project_path).await?;
                let LaunchPolicy::Claude {
                    permission_mode,
                    mut allowed_tools,
                    mut disallowed_tools,
                    agent,
                    review_notes,
                } = policy
                else {
                    unreachable!("claude harness yields a claude launch policy")
                };
                let settings = self.runtime.settings.get().await;
                let library = &settings.agents;
                let drafts: Vec<String> = agents::list_agents(
                    Some(std::path::Path::new(project_path)),
                    Some(&self.runtime.environment.home_dir),
                    None,
                )
                .into_iter()
                .filter(|a| {
                    library
                        .overlays
                        .get(&a.path)
                        .is_some_and(|o| o.status == "draft")
                })
                .map(|a| a.name)
                .collect();
                disallowed_tools.extend(agents::agent_deny_rules(&drafts));
                // The app owns goals: no native goal can be proposed in its sessions.
                disallowed_tools.push("ProposeGoal".into());
                if !*report_only {
                    allowed_tools.extend(pipeline::thoughts_edit_rules(session.stage, &add_dirs));
                }
                allowed_tools.extend(extra_allowed_tools.iter().cloned());
                disallowed_tools.extend(extra_disallowed_tools.iter().cloned());
                let mcp_config = endpoint.map(|endpoint| {
                    allowed_tools.extend(inject::mcp_allow_rules());
                    inject::claude_mcp_config(&endpoint.ctl, &identity_env).to_string()
                });

                let path = self.runtime.binaries.augmented_path().await;
                let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
                let claude_cfg = settings.providers.claude;
                env.extend(crate::runtime::providers::claude_auth_env(&claude_cfg));
                // A daemonized server has no SSH_AUTH_SOCK of its own; without this an
                // agent's own `git push` fails where a terminal's succeeds (ADR-0009 §7).
                env.extend(self.runtime.environment.overlay_env());
                env.extend(identity_env);

                let (driver, rx) = ClaudeDriver::spawn(SpawnConfig {
                    claude_bin: claude,
                    cwd: PathBuf::from(&session.cwd),
                    session_id: session.id.clone(),
                    permission_mode,
                    allowed_tools,
                    disallowed_tools,
                    add_dirs,
                    resume: resume.then(|| session.id.clone()),
                    fork: false,
                    max_budget_usd: *max_budget_usd,
                    model,
                    effort,
                    chrome: claude_cfg.chrome_enabled,
                    agent,
                    append_system_prompt: review_notes
                        .then(|| pipeline::review_notes_instruction(&session.workstream_id)),
                    env,
                    claude_config_dir: Some(self.runtime.paths.claude_config_dir.clone()),
                    mcp_config,
                    plugin_dir: endpoint.and_then(|endpoint| endpoint.plugin_dir.clone()),
                })?;
                let driver: Arc<dyn AgentDriver> = Arc::new(driver);
                Ok((driver, rx, Some(permission_mode)))
            }
        }
    }

    /// The MCP `-c` keys were probed on 0.164; nothing proves older codex accepts them.
    async fn codex_supports_mcp(&self) -> bool {
        let status = self.runtime.binaries.status(BinaryKind::Codex).await;
        let version = status.version.as_deref().and_then(parse_semver);
        let supported = version.is_some_and(|v| v >= inject::CODEX_HOOKS_MIN);
        if !supported {
            tracing::info!(
                "codex {} gets no MCP server: it predates 0.164",
                status.version.as_deref().unwrap_or("of unknown version")
            );
        }
        supported
    }

    pub async fn rewind(&self, request: RewindSessionRequest) -> Result<RewindSessionOutcome> {
        let RewindSessionRequest {
            session,
            workstream,
            user_turn,
            permission_mode,
        } = request;
        let session_id = session.id.clone();

        let op = self.op_lock(&session_id).await;
        let _op_guard = op.lock().await;

        let cutoff = {
            let db = self.runtime.db.clone();
            let id = session_id.clone();
            let n = user_turn as usize;
            blocking(move || db.nth_user_message_seq(&id, n)).await?
        }
        .ok_or(CoreError::UserTurnNotFound {
            session_id: session_id.clone(),
            user_turn,
        })?;

        let live = self
            .runtime
            .session_registry
            .get(&session_id)
            .await
            .map(|driver| (driver, user_turn as usize));

        let outcome = self
            .runtime
            .session_actors
            .rewind(&session_id, cutoff, live)
            .await?;
        tracing::debug!(
            session_id = %session_id,
            epoch = outcome.epoch,
            restart_seq = outcome.restart_seq,
            live_rewound = outcome.live_rewound,
            "session rewound"
        );

        if outcome.live_rewound {
            return Ok(RewindSessionOutcome {
                session,
                live_rewound: true,
            });
        }

        self.kill_and_await_exit(&session_id).await;

        let launched = self
            .launch(LaunchSessionParams {
                session: session.clone(),
                project_path: workstream.project_path,
                model: None,
                effort: None,
                permission_mode,
                resume: true,
                boundary_on_relaunch: false,
                max_budget_usd: None,
                extra_allowed_tools: Vec::new(),
                extra_disallowed_tools: Vec::new(),
                report_only: false,
            })
            .await;

        if let Err(e) = launched {
            let _ = self
                .runtime
                .session_actors
                .restore_after_failed_relaunch(&session_id, cutoff, outcome.removed)
                .await;
            return Err(e);
        }

        Ok(RewindSessionOutcome {
            session,
            live_rewound: false,
        })
    }
}

fn spawn_relay(
    mut rx: broadcast::Receiver<SessionEvent>,
    ingress: mpsc::Sender<SessionEvent>,
    registry: Arc<super::registry::SessionRegistry>,
    session_id: String,
    token: Option<TokenGuard>,
) {
    tokio::spawn(async move {
        // Revoked when the relay ends, not when this function returns.
        let _token = token;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let terminal = matches!(event, SessionEvent::Exited { .. });
                    // Keeps a resume asking for the new mode from relaunching.
                    if let SessionEvent::PermissionModeChanged { mode } = event {
                        registry.set_permission_mode(&session_id, mode).await;
                    }
                    if ingress.send(event).await.is_err() {
                        break;
                    }
                    if terminal {
                        registry.remove(&session_id).await;
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("session {session_id} relay lagged {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
