//! Caller identity and the control-level policy for agents that call back into the runtime.

pub(crate) mod codex_remote;
pub mod inject;
pub(crate) mod links;
pub mod pty_agents;
pub mod pty_controls;
mod rollout;
pub mod spawn;
pub mod tokens;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_models::goal::GoalCheck;
use hyprlayer_models::{
    AgentCaller, AgentControlLevel, AgentSignal, AgentStatusReport, AgentWhoami, Harness,
    PermissionMode, RevealTarget, RuntimeEvent, Session, SessionOrigin, UiRequest,
};

use super::HyprlayerRuntime;
use crate::error::{CoreError, Result};
use pty_agents::PtySnapshot;
pub use tokens::{AgentIdentity, AgentTokens, TokenGuard};

/// How far a descendant check climbs; lineage is far shallower than this.
const LINEAGE_WALK: usize = 64;

/// Every var a spawn's identity sets; a server spawn scrubs them so it never re-exports an outer agent's.
pub const AGENT_ENV: [&str; 6] = [
    "HYPRLAYER_CTL",
    "HYPRLAYER_SOCKET",
    "HYPRLAYER_AGENT_TOKEN",
    "HYPRLAYER_WORKSTREAM_ID",
    "HYPRLAYER_SESSION_ID",
    "HYPRLAYER_TERMINAL_ID",
];

/// How an agent process reaches this server: the binary to run and the socket to dial.
#[derive(Debug, Clone)]
pub struct AgentControlEndpoint {
    pub ctl: PathBuf,
    pub socket: PathBuf,
    /// The orchestration plugin the server wrote; `None` when it could not, so launches omit it.
    pub plugin_dir: Option<PathBuf>,
    /// A private dir short enough for a PTY codex's app-server socket; `None` keeps codex embedded.
    pub runtime_dir: Option<PathBuf>,
}

pub fn effective_level(
    level: AgentControlLevel,
    mode: Option<PermissionMode>,
) -> AgentControlLevel {
    match (level, mode) {
        (AgentControlLevel::Off, _) => AgentControlLevel::Off,
        // Bypass already removed the human; read-only would only stall it.
        (_, Some(PermissionMode::BypassPermissions)) => AgentControlLevel::Full,
        (level, _) => level,
    }
}

pub struct AgentControlService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn agent_control(&self) -> AgentControlService<'_> {
        AgentControlService { runtime: self }
    }
}

impl AgentControlService<'_> {
    /// The orchestration plugin claude and codex launches get; `None` at `off` or with none written.
    pub async fn launch_plugin_dir(&self) -> Option<PathBuf> {
        let dir = self
            .runtime
            .agent_control_endpoint
            .as_ref()?
            .plugin_dir
            .clone()?;
        let level = self.runtime.settings.get().await.agent_control.level;
        (level != AgentControlLevel::Off).then_some(dir)
    }

    /// The `HYPRLAYER_*` env for one spawn plus the guard that revokes its token; `None` with no endpoint or at `off`.
    pub async fn identity_env(
        &self,
        identity: AgentIdentity,
    ) -> Option<(Vec<(String, String)>, TokenGuard)> {
        let endpoint = self.runtime.agent_control_endpoint.as_ref()?;
        let level = self.runtime.settings.get().await.agent_control.level;
        if effective_level(level, identity.launch_mode) == AgentControlLevel::Off {
            return None;
        }
        let (caller_key, caller_id) = match &identity.caller {
            AgentCaller::Session { session_id } => (AGENT_ENV[4], session_id.clone()),
            AgentCaller::Terminal { terminal_id } => (AGENT_ENV[5], terminal_id.clone()),
        };
        let workstream_id = identity.workstream_id.clone();
        let guard = self.runtime.agent_tokens.mint(identity);
        let [ctl, socket, token, workstream, ..] = AGENT_ENV;
        let env = vec![
            (ctl.to_string(), endpoint.ctl.to_string_lossy().into_owned()),
            (
                socket.to_string(),
                endpoint.socket.to_string_lossy().into_owned(),
            ),
            (token.to_string(), guard.token().to_string()),
            (workstream.to_string(), workstream_id),
            (caller_key.to_string(), caller_id),
        ];
        Some((env, guard))
    }

    /// A session's spawning parent and its depth: one per agent hop up to a session a person started.
    pub async fn session_lineage(&self, session: &Session) -> (Option<AgentCaller>, u32) {
        let links = self.runtime.agent_links.clone();
        let parent = match session.spawn_parent() {
            Some(session_id) => Some(AgentCaller::Session {
                session_id: session_id.to_string(),
            }),
            None => links.get(&session.id).map(|link| link.parent),
        };
        let max_depth = self.runtime.settings.get().await.agent_control.max_depth;
        // Walking past the cap only proves what the cap already refuses.
        let cap = max_depth.saturating_add(1);
        let (db, start) = (self.runtime.db.clone(), session.clone());
        let walked = blocking(move || {
            let (mut depth, mut current) = (0, start);
            while depth < cap {
                let Some(parent_id) = current.spawn_parent().map(str::to_string) else {
                    // The topmost row a terminal spawned carries the terminal's depth too.
                    if let Some(link) = links.get(&current.id) {
                        depth += link.depth;
                    } else if current.origin == SessionOrigin::Agent {
                        // Its terminal parent is gone with a restart: fail closed at the limit.
                        depth = depth.saturating_add(1).max(max_depth);
                    }
                    break;
                };
                depth += 1;
                match db.get_session(&parent_id)? {
                    Some(row) => current = row,
                    None => {
                        // A deleted parent's own depth is unknowable.
                        depth = depth.max(max_depth);
                        break;
                    }
                }
            }
            Ok(depth)
        })
        .await;
        let depth = walked.unwrap_or_else(|e| {
            tracing::warn!("session {}: its lineage is unreadable: {e}", session.id);
            cap
        });
        (parent, depth)
    }

    /// The live spawn a token speaks for.
    pub fn identify(&self, token: &str) -> Result<AgentIdentity> {
        self.runtime
            .agent_tokens
            .lookup(token)
            .ok_or_else(|| CoreError::AgentControlDenied {
                detail: "the agent that held this token has exited".into(),
            })
    }

    async fn mode_of(&self, identity: &AgentIdentity) -> Option<PermissionMode> {
        match &identity.caller {
            AgentCaller::Session { session_id } => self
                .runtime
                .session_registry
                .permission_mode(session_id)
                .await
                .or(identity.launch_mode),
            AgentCaller::Terminal { .. } => identity.launch_mode,
        }
    }

    /// Resolved per call, so a session switched into or out of bypass changes level at once.
    pub async fn level_of(&self, identity: &AgentIdentity) -> AgentControlLevel {
        let level = self.runtime.settings.get().await.agent_control.level;
        effective_level(level, self.mode_of(identity).await)
    }

    /// Mutating verbs: `full`, not report-only, and never a codex session whose MCP calls go unasked.
    pub async fn require_full(&self, identity: &AgentIdentity) -> Result<()> {
        let mode = self.mode_of(identity).await;
        let level = self.runtime.settings.get().await.agent_control.level;
        if effective_level(level, mode) != AgentControlLevel::Full {
            return Err(denied("this needs agent control at Full"));
        }
        if identity.report_only {
            return Err(denied("a report-only session cannot change anything"));
        }
        // codex approves every hyprlayer tool unasked, so only bypass, which asks nothing anyway, may mutate.
        if identity.harness == Some(Harness::Codex)
            && mode != Some(PermissionMode::BypassPermissions)
        {
            return Err(denied("a codex session changes things only in bypass mode"));
        }
        Ok(())
    }

    /// A command check is a shell on the server, which only a bypass caller already has.
    pub async fn check_goal_checks(
        &self,
        identity: &AgentIdentity,
        checks: &[GoalCheck],
    ) -> Result<()> {
        let shell = checks
            .iter()
            .any(|c| matches!(c, GoalCheck::Command { .. }));
        if shell && self.mode_of(identity).await != Some(PermissionMode::BypassPermissions) {
            return Err(denied(
                "a goal's command check runs a shell, so only a bypass agent may set one",
            ));
        }
        Ok(())
    }

    /// Refuses unless `target_id`, a session or terminal, was spawned at any depth by the caller (or is it, with `or_self`).
    pub async fn check_descendant(
        &self,
        identity: &AgentIdentity,
        target_id: &str,
        or_self: bool,
    ) -> Result<()> {
        let me = identity.caller.id().to_string();
        if or_self && me == target_id {
            return Ok(());
        }
        let (db, links, target) = (
            self.runtime.db.clone(),
            self.runtime.agent_links.clone(),
            target_id.to_string(),
        );
        let descends = blocking(move || {
            let mut current = target;
            for _ in 0..LINEAGE_WALK {
                let parent = match links.get(&current) {
                    Some(link) => Some(link.parent.id().to_string()),
                    None => db
                        .get_session(&current)?
                        .and_then(|s| s.spawn_parent().map(str::to_string)),
                };
                match parent {
                    Some(parent) if parent == me => return Ok(true),
                    Some(parent) => current = parent,
                    None => return Ok(false),
                }
            }
            Ok(false)
        })
        .await?;
        if descends {
            Ok(())
        } else {
            Err(denied("that agent was not spawned by this one"))
        }
    }

    /// Live agents the caller spawned directly: persisted session children plus in-memory links.
    async fn live_children(&self, identity: &AgentIdentity) -> Result<usize> {
        let live = self.runtime.session_registry.live_ids().await;
        // A session's link outlives its harness; a PTY's ends with it.
        let linked = self
            .runtime
            .agent_links
            .children_of(identity.caller.id())
            .into_iter()
            .filter(|id| live.contains(id) || self.runtime.pty_agents.snapshot(id).is_some())
            .count();
        let AgentCaller::Session { session_id: me } = &identity.caller else {
            return Ok(linked);
        };
        let (db, me) = (self.runtime.db.clone(), me.clone());
        let persisted = blocking(move || {
            let mut count = 0;
            for id in live {
                if db
                    .get_session(&id)?
                    .is_some_and(|s| s.spawn_parent() == Some(me.as_str()))
                {
                    count += 1;
                }
            }
            Ok(count)
        })
        .await?;
        Ok(persisted + linked)
    }

    /// A person spoke to `target_id`: its agent turns count from zero again.
    pub fn note_human_turn(&self, target_id: &str) {
        self.runtime.agent_hops.reset(target_id);
    }

    async fn max_hops(&self) -> u32 {
        self.runtime
            .settings
            .get()
            .await
            .agent_control
            .max_agent_hops
    }

    /// Refuses an agent turn into `target_id` past the limit, telling the user once; `record_hop` counts a delivered one.
    async fn admit_hop(&self, target_id: &str, workstream_id: &str, label: &str) -> Result<()> {
        let max = self.max_hops().await;
        match self.runtime.agent_hops.admit(target_id, max) {
            Hop::Within => return Ok(()),
            Hop::Refused { first: true } => {
                let _ = self.runtime.events.send(RuntimeEvent::UiRequested {
                    request: UiRequest::Notify {
                        workstream_id: workstream_id.to_string(),
                        title: "An agent needs you".into(),
                        body: format!(
                            "{label} got {max} agent messages in a row. Reply to it to let agents continue."
                        ),
                        // Not an ask: no stream fold raises it, so every desktop must.
                        session_id: None,
                        terminal_id: None,
                    },
                });
            }
            Hop::Refused { first: false } => {}
        }
        Err(CoreError::AgentLimitReached {
            detail: format!(
                "{label} got {max} agent messages with no human turn between (Max agent hops)"
            ),
        })
    }

    /// Counts only a delivered turn, so a failed send to a dead target costs nothing.
    async fn record_hop(&self, target_id: &str) {
        let max = self.max_hops().await;
        self.runtime.agent_hops.record(target_id, max);
    }

    pub async fn whoami(&self, identity: &AgentIdentity) -> AgentWhoami {
        let permission_mode = self.mode_of(identity).await;
        let level = self.runtime.settings.get().await.agent_control.level;
        AgentWhoami {
            caller: identity.caller.clone(),
            workstream_id: identity.workstream_id.clone(),
            project_path: identity.project_path.clone(),
            level: effective_level(level, permission_mode),
            permission_mode,
            depth: identity.depth,
            parent: identity.parent.clone(),
            report_only: identity.report_only,
        }
    }

    pub fn notify(&self, identity: &AgentIdentity, title: String, body: String) {
        let _ = self.runtime.events.send(RuntimeEvent::UiRequested {
            request: UiRequest::Notify {
                workstream_id: identity.workstream_id.clone(),
                title,
                body,
                session_id: None,
                terminal_id: None,
            },
        });
    }

    pub fn request_attention(&self, identity: &AgentIdentity, reason: String) {
        self.notify(identity, "An agent needs you".into(), reason);
    }

    /// The caller's project: its workstreams (archived too) and its thoughts mapping.
    pub async fn project_scope(&self, identity: &AgentIdentity) -> Result<ProjectScope> {
        let db = self.runtime.db.clone();
        let project = identity.project_path.clone();
        let workstream_ids = blocking(move || db.list_workstreams(&project, true))
            .await?
            .into_iter()
            .map(|ws| ws.id)
            .collect();
        // A missing or unreadable CLI config only costs the doc mapping: mapped docs then fail closed.
        let thoughts_project = match self
            .runtime
            .settings()
            .effective_config_for(&identity.project_path)
            .await
        {
            Ok(effective) => effective.mapped_name,
            Err(e) => {
                tracing::debug!("agent scope has no thoughts mapping: {e}");
                None
            }
        };
        Ok(ProjectScope {
            workstream_ids,
            thoughts_project,
        })
    }

    pub fn check_project(&self, identity: &AgentIdentity, project_path: &str) -> Result<()> {
        in_project(identity, project_path)
    }

    pub async fn check_workstream(&self, identity: &AgentIdentity, id: &str) -> Result<()> {
        let db = self.runtime.db.clone();
        let owned = id.to_string();
        let ws = blocking(move || db.get_workstream(&owned))
            .await?
            .ok_or_else(|| CoreError::WorkstreamNotFound { id: id.to_string() })?;
        in_project(identity, &ws.project_path)
    }

    pub async fn check_session(&self, identity: &AgentIdentity, id: &str) -> Result<()> {
        let db = self.runtime.db.clone();
        let owned = id.to_string();
        let session = blocking(move || db.get_session(&owned))
            .await?
            .ok_or_else(|| CoreError::SessionNotFound {
                session_id: id.to_string(),
            })?;
        self.check_workstream(identity, &session.workstream_id)
            .await
    }

    pub async fn check_goal(&self, identity: &AgentIdentity, id: &str) -> Result<()> {
        let db = self.runtime.db.clone();
        let owned = id.to_string();
        let goal =
            blocking(move || db.goal(&owned))
                .await?
                .ok_or_else(|| CoreError::GoalNotFound {
                    goal_id: id.to_string(),
                })?;
        self.check_session(identity, &goal.goal.session_id).await
    }

    /// Indexed thoughts docs only: an unindexed id is a bare path, which would read any file.
    /// A write, attaching included, takes a doc mapped to the caller's project; attached docs only read.
    pub async fn check_doc(&self, identity: &AgentIdentity, id: &str, write: bool) -> Result<()> {
        let index = &self.runtime.index;
        let Some((resolved, meta)) = index
            .resolve_id(id)
            .and_then(|r| index.get(&r).map(|meta| (r, meta)))
        else {
            return Err(denied("that is not an indexed thoughts document"));
        };
        let scope = self.project_scope(identity).await?;
        let mapped = meta
            .project
            .as_deref()
            .is_some_and(|project| scope.thoughts_project.as_deref() == Some(project));
        if mapped || (!write && meta.project.is_none()) {
            return Ok(());
        }
        if write {
            return Err(denied(
                "an agent writes only documents mapped to its own project",
            ));
        }
        let db = self.runtime.db.clone();
        let attached = blocking(move || {
            for ws in &scope.workstream_ids {
                if db
                    .workstream_docs(ws)?
                    .iter()
                    .any(|d| index_path_eq(&d.doc_path, &resolved))
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await?;
        if attached {
            Ok(())
        } else {
            Err(denied("that document belongs to another project"))
        }
    }

    /// A PTY agent this server launched, in the caller's project; plain shells and scripts never pass.
    pub async fn check_terminal(&self, identity: &AgentIdentity, terminal_id: &str) -> Result<()> {
        self.pty_target(identity, terminal_id).await.map(|_| ())
    }

    async fn pty_target(&self, identity: &AgentIdentity, terminal_id: &str) -> Result<PtySnapshot> {
        let Some(snapshot) = self.runtime.pty_agents.snapshot(terminal_id) else {
            return Err(denied("that is not a running PTY agent"));
        };
        self.check_workstream(identity, &snapshot.agent.workstream_id)
            .await?;
        Ok(snapshot)
    }

    /// A goal on the caller's own session or on one it spawned.
    pub async fn check_own_goal(&self, identity: &AgentIdentity, goal_id: &str) -> Result<()> {
        let db = self.runtime.db.clone();
        let owned = goal_id.to_string();
        let goal =
            blocking(move || db.goal(&owned))
                .await?
                .ok_or_else(|| CoreError::GoalNotFound {
                    goal_id: goal_id.to_string(),
                })?;
        self.check_session(identity, &goal.goal.session_id).await?;
        self.check_descendant(identity, &goal.goal.session_id, true)
            .await
    }

    /// Opens a doc, file or diff in the caller's workstream on every attached desktop, in the background.
    pub fn reveal(&self, identity: &AgentIdentity, target: RevealTarget) {
        let _ = self.runtime.events.send(RuntimeEvent::UiRequested {
            request: UiRequest::Reveal {
                workstream_id: identity.workstream_id.clone(),
                target,
            },
        });
    }

    /// Folds a hook signal into its PTY agent; an unknown or revoked token is ignored.
    pub fn report_signal(&self, token: &str, signal: &AgentSignal) {
        match self.runtime.agent_tokens.lookup(token).map(|i| i.caller) {
            Some(AgentCaller::Terminal { terminal_id }) => {
                self.runtime.pty_agents.apply(&terminal_id, signal);
                if let Some(prompt) = signal.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
                    self.title_terminal(&terminal_id, prompt);
                }
            }
            Some(AgentCaller::Session { .. }) => {
                tracing::debug!(
                    "ignored a {} hook signal from a composer session",
                    signal.event
                )
            }
            None => tracing::debug!(
                "ignored a {} hook signal: its token is unknown or revoked",
                signal.event
            ),
        }
    }

    /// A claude statusLine run for its PTY agent; an unknown or revoked token is ignored.
    pub fn report_status(&self, token: &str, report: AgentStatusReport) {
        match self.runtime.agent_tokens.lookup(token).map(|i| i.caller) {
            Some(AgentCaller::Terminal { terminal_id }) => {
                self.runtime.pty_agents.report_status(&terminal_id, report)
            }
            _ => tracing::debug!("ignored a status report: its token names no PTY agent"),
        }
    }

    /// Whether the bar is switching this token's claude right now, so its `PreModelSwitch` hook skips the dialog.
    pub fn model_switch_pending(&self, token: &str) -> bool {
        match self.runtime.agent_tokens.lookup(token).map(|i| i.caller) {
            Some(AgentCaller::Terminal { terminal_id }) => {
                self.runtime.pty_agents.switch_pending(&terminal_id)
            }
            _ => false,
        }
    }

    /// The composer's title generator on a prompt, with one retry, until a title is stored; a
    /// failed or empty one leaves the next prompt to try again.
    fn title_terminal(&self, terminal_id: &str, prompt: &str) {
        let (runtime, id, prompt) = (
            self.runtime.handle.clone(),
            terminal_id.to_string(),
            prompt.to_string(),
        );
        tokio::spawn(async move {
            let Some(rt) = runtime.upgrade() else {
                return;
            };
            // Off claims nothing, so turning titles on later titles the next prompt.
            if !rt.settings.get().await.tab_titles.enabled || !rt.pty_agents.claim_title(&id) {
                return;
            }
            drop(rt);
            for _ in 0..2 {
                let Some(rt) = runtime.upgrade() else {
                    return;
                };
                if !rt.pty_agents.take_title_try(&id) {
                    break;
                }
                match rt.providers().suggest_title(&prompt).await {
                    Ok(Some(title)) => return rt.pty_agents.set_title(&id, title),
                    Ok(None) => break,
                    Err(e) => tracing::info!("terminal {id}: no title generated: {e}"),
                }
            }
            if let Some(rt) = runtime.upgrade() {
                rt.pty_agents.release_title(&id);
            }
        });
    }
}

/// Agent turns into each target since its last human turn; in memory, so a restart clears it.
#[derive(Default)]
pub(crate) struct AgentHops {
    counts: Mutex<HashMap<String, u32>>,
}

enum Hop {
    Within,
    /// `first` only for the refusal that crossed the limit.
    Refused {
        first: bool,
    },
}

impl AgentHops {
    fn admit(&self, target_id: &str, max: u32) -> Hop {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let count = counts.entry(target_id.to_string()).or_insert(0);
        if *count < max {
            return Hop::Within;
        }
        let first = *count == max;
        *count = max.saturating_add(1);
        Hop::Refused { first }
    }

    /// Capped at `max`, so racing sends still leave the crossing refusal to tell the user.
    fn record(&self, target_id: &str, max: u32) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let count = counts.entry(target_id.to_string()).or_insert(0);
        *count = count.saturating_add(1).min(max);
    }

    fn reset(&self, target_id: &str) {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(target_id);
    }
}

/// One spawn at a time per caller, so concurrent spawns cannot all pass its live-children count.
#[derive(Default)]
pub(crate) struct SpawnLocks {
    by_caller: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl SpawnLocks {
    async fn acquire(&self, caller_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self
                .by_caller
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Only the map holds an idle caller's lock.
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
            locks.entry(caller_id.to_string()).or_default().clone()
        };
        lock.lock_owned().await
    }
}

/// What a list verb keeps for an agent: rows in its project, docs mapped to it or to none.
pub struct ProjectScope {
    pub workstream_ids: HashSet<String>,
    pub thoughts_project: Option<String>,
}

impl ProjectScope {
    /// Shared and global docs carry no project and stay readable.
    pub fn allows_doc(&self, project: Option<&str>) -> bool {
        project.is_none_or(|p| self.thoughts_project.as_deref() == Some(p))
    }
}

fn in_project(identity: &AgentIdentity, project_path: &str) -> Result<()> {
    if project_path == identity.project_path {
        Ok(())
    } else {
        Err(denied("that belongs to another project"))
    }
}

fn denied(detail: &str) -> CoreError {
    CoreError::AgentControlDenied {
        detail: detail.into(),
    }
}

fn index_path_eq(doc_path: &str, resolved: &str) -> bool {
    doc_path == resolved
        || std::fs::canonicalize(doc_path).is_ok_and(|p| p.to_string_lossy() == resolved)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}
