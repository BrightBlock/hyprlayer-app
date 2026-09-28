use std::path::PathBuf;
use std::sync::Arc;

use hyprlayer_models::session::QuestionAnswers;
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
use crate::process::binaries::BinaryKind;
use crate::runtime::HyprlayerRuntime;
use crate::skills;
use crate::store::{LogCursor, NewSession, Session, SessionLogEntry, Workstream};
use hyprlayer_models::RuntimeEvent;
use serde::{Deserialize, Serialize};

use super::followup::{self, Followup};
use super::workflow_bridge::{self, WorkflowChannel};
use super::SessionAttachment;

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
}

pub struct LaunchSessionParams {
    pub session: Session,
    pub project_path: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub resume: bool,
    pub boundary_on_relaunch: bool,
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
    ) -> Result<()> {
        let op = self.op_lock(session_id).await;
        let _op_guard = op.lock().await;

        let prompt = pipeline::append_docs(text.to_string(), attached_docs);
        let driver = self.require_live(session_id).await?;
        // A session someone is still talking to belongs back in the list.
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        blocking(move || db.set_session_archived(&id, false)).await?;
        driver.send_user_turn(&prompt, None, prompt_name, client_message_id)
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
        self.require_live(session_id).await?.interrupt()
    }

    pub async fn kill(&self, session_id: &str) -> Result<()> {
        tracing::info!("session {session_id}: kill requested");
        self.require_live(session_id).await?.kill();
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
        self.announce(session);
        Ok(())
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

    pub async fn prepare_start(
        &self,
        request: StartStageSessionRequest,
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
        } = request;

        let harness = harness.unwrap_or_default();
        let first_input = Some(input.trim().to_string()).filter(|t| !t.is_empty());
        let bodies = self.action_prompt_bodies(stage, &workstream).await;
        let prompt =
            pipeline::compose_prompt(stage, &input, &workstream.base_branch, &bodies, harness);
        let wire_prompt = pipeline::append_docs(prompt.clone(), &attached_docs);

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
            blocking(move || db.insert_session(new)).await?
        };
        self.runtime.session_titler.enqueue(&session.id, true);

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
            },
        })
    }

    pub async fn launch_and_converse(&self, params: LaunchAndConverseParams) -> Result<()> {
        let LaunchAndConverseParams {
            session,
            project_path,
            wire_prompt,
            stage,
            prompt_name,
            client_message_id,
            model,
            effort,
            permission_mode,
            bodies,
        } = params;

        let followup = followup::followup_for(session.harness, stage, &bodies);
        let session_id = session.id.clone();

        let driver = self
            .launch_driver(LaunchSessionParams {
                session,
                project_path,
                model,
                effort,
                permission_mode,
                resume: false,
                boundary_on_relaunch: false,
            })
            .await?;

        let op_lock = self.op_lock(&session_id).await;
        let mut turns = followup.as_ref().map(|_| driver.subscribe());
        let first_send = {
            let _guard = op_lock.lock().await;
            if matches!(followup, Some(Followup::CodexExecute(_))) {
                driver.set_collab_mode(Some(CollabMode::Plan));
            }
            driver.send_user_turn(
                &wire_prompt,
                Some(stage.as_str()),
                prompt_name.as_deref(),
                client_message_id.as_deref(),
            )
        };
        first_send?;
        if let (Some(rx), Some(followup)) = (turns.as_mut(), followup) {
            followup::run_followup(&session_id, driver.as_ref(), rx, followup, op_lock).await;
        }
        Ok(())
    }

    pub async fn launch(&self, params: LaunchSessionParams) -> Result<()> {
        self.launch_driver(params).await.map(|_| ())
    }

    async fn launch_driver(&self, params: LaunchSessionParams) -> Result<Arc<dyn AgentDriver>> {
        let LaunchSessionParams {
            session,
            project_path,
            model,
            effort,
            permission_mode,
            resume,
            boundary_on_relaunch,
        } = params;
        let session_id = session.id.clone();

        if resume {
            let db = self.runtime.db.clone();
            let id = session_id.clone();
            let _ = blocking(move || db.set_session_state(&id, SessionState::Running)).await;
        }

        let described = format!(
            "{:?} {:?} (model {}, effort {}, resume {resume})",
            session.harness,
            session.stage,
            model.as_deref().unwrap_or("default"),
            effort.as_deref().unwrap_or("default"),
        );
        let spawned = self
            .spawn_driver(
                &session,
                &project_path,
                model,
                effort,
                permission_mode,
                resume,
            )
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
            if boundary_on_relaunch {
                let _ = ingress.send(SessionEvent::Relaunched {}).await;
            }
            workflow_bridge::recover_workflow_runs(&self.runtime.db, &session_id, &workflow).await;
        }

        spawn_relay(
            rx,
            ingress,
            self.runtime.session_registry.clone(),
            session_id,
        );

        Ok(driver)
    }

    async fn spawn_driver(
        &self,
        session: &Session,
        project_path: &str,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
        resume: bool,
    ) -> Result<(
        Arc<dyn AgentDriver>,
        broadcast::Receiver<SessionEvent>,
        Option<PermissionMode>,
    )> {
        let policy = launch_policy(session.harness, session.stage, permission_mode);
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

                let path = self.runtime.binaries.augmented_path().await;
                let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
                env.extend(self.runtime.environment.overlay_env());

                let developer_instructions = self.memory_instructions(&session.workstream_id).await;

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
                    skill_roots: skills::skill_roots(
                        Some(std::path::Path::new(&session.cwd)),
                        Some(&self.runtime.environment.home_dir),
                    ),
                    resume: resume_thread,
                    prior_user_turns,
                    model,
                    effort,
                    ephemeral: false,
                    env,
                    developer_instructions,
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
                    allowed_tools,
                    mut disallowed_tools,
                    agent,
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

                let path = self.runtime.binaries.augmented_path().await;
                let mut env = vec![("PATH".to_string(), path.to_string_lossy().into_owned())];
                let claude_cfg = settings.providers.claude;
                env.extend(crate::runtime::providers::claude_auth_env(&claude_cfg));
                // A daemonized server has no SSH_AUTH_SOCK of its own; without this an
                // agent's own `git push` fails where a terminal's succeeds (ADR-0009 §7).
                env.extend(self.runtime.environment.overlay_env());

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
                    max_budget_usd: None,
                    model,
                    effort,
                    chrome: claude_cfg.chrome_enabled,
                    agent,
                    env,
                    claude_config_dir: Some(self.runtime.paths.claude_config_dir.clone()),
                })?;
                let driver: Arc<dyn AgentDriver> = Arc::new(driver);
                Ok((driver, rx, Some(permission_mode)))
            }
        }
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
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let terminal = matches!(event, SessionEvent::Exited { .. });
                    // Keeps a resume asking for the new mode from relaunching.
                    if let SessionEvent::PermissionModeChanged { mode } = event {
                        registry.set_permission_mode(&session_id, mode).await;
                    }
                    if ingress.send(event).await.is_err() {
                        return;
                    }
                    if terminal {
                        registry.remove(&session_id).await;
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("session {session_id} relay lagged {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}
