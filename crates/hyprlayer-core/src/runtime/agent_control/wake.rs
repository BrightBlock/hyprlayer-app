use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use hyprlayer_models::{
    AgentCaller, ClientRef, PrWakeUntil, PtyAgentState, RuntimeEvent, SessionState, WaitUntil,
    Wake, WakeCondition,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, oneshot};
use tokio::time::{Instant, MissedTickBehavior};

use super::spawn::{
    observe_pty, paste, paste_refusal, quiet_spell, session_label, Observed, PROMPT_ACK,
};
use super::{blocking, denied, AgentControlService, AgentIdentity};
use crate::error::{CoreError, Result};
use crate::gitops;
use crate::pr::{ChecksState, PrLookup, PrState, PrStatus};
use crate::process::binaries::BinaryKind;
use crate::runtime::pr_watcher::{cached_age, cached_lookup, PrCache};
use crate::runtime::sessions::SendContext;
use crate::runtime::workstreams::{current_pr_ref, pr_dir};
use crate::runtime::HyprlayerRuntime;
use crate::store::{now_ms, Workstream};
use crate::worktree;

const MAX_PENDING: usize = 8;
const MAX_WAIT_SECS: u64 = 86_400;
const DEFAULT_TIMEOUT_MS: u32 = 3_600_000;
const POLL: Duration = Duration::from_secs(60);
// ponytail: a fixed retry while an idle PTY caller refuses a paste; no event says when its input clears.
const RECHECK: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct Wakes {
    by_caller: Mutex<HashMap<String, Vec<Pending>>>,
}

struct Pending {
    wake: Wake,
    _cancel: oneshot::Sender<()>,
}

impl Wakes {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Vec<Pending>>> {
        self.by_caller
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, caller: &str, wake: Wake, cancel: oneshot::Sender<()>) -> Result<()> {
        let mut pending = self.lock();
        let mine = pending.entry(caller.to_string()).or_default();
        if mine.len() >= MAX_PENDING {
            return Err(CoreError::AgentLimitReached {
                detail: format!(
                    "this agent already has {MAX_PENDING} pending wakes, the most it may hold; cancel one first"
                ),
            });
        }
        mine.push(Pending {
            wake,
            _cancel: cancel,
        });
        Ok(())
    }

    fn list(&self, caller: &str) -> Vec<Wake> {
        self.lock()
            .get(caller)
            .map(|mine| mine.iter().map(|p| p.wake.clone()).collect())
            .unwrap_or_default()
    }

    /// Dropping the entry's sender is what stops its waiter.
    fn take(&self, caller: &str, id: &str) -> bool {
        let mut pending = self.lock();
        let Some(mine) = pending.get_mut(caller) else {
            return false;
        };
        let before = mine.len();
        mine.retain(|p| p.wake.id != id);
        let taken = mine.len() < before;
        if mine.is_empty() {
            pending.remove(caller);
        }
        taken
    }

    fn clear(&self, caller: &str) {
        self.lock().remove(caller);
    }
}

impl AgentControlService<'_> {
    pub async fn wake_when(
        &self,
        identity: &AgentIdentity,
        condition: WakeCondition,
        timeout_ms: Option<u32>,
    ) -> Result<Wake> {
        if let AgentCaller::Terminal { terminal_id } = &identity.caller {
            let hooked = self
                .runtime
                .pty_agents
                .snapshot(terminal_id)
                .is_some_and(|s| s.agent.hooked);
            self.check_unhooked_input(identity, hooked).await?;
        }
        let (started, registered) = (Instant::now(), now_ms());
        let deadline_ms = deadline(&condition, timeout_ms, registered)?;
        // Before the baseline is read, so no change after it is missed.
        let events = self.runtime.events.subscribe();
        let watch = self.watch_for(identity, &condition).await?;
        let wake = Wake {
            id: uuid::Uuid::new_v4().to_string(),
            condition,
            deadline_ms,
        };
        let (cancel, cancelled) = oneshot::channel();
        self.runtime
            .wakes
            .add(identity.caller.id(), wake.clone(), cancel)?;
        let waited = Duration::from_millis((deadline_ms - registered) as u64);
        let waiter = Waiter {
            runtime: self.runtime.handle.clone(),
            caller: identity.caller.clone(),
            project: identity.project_path.clone(),
            wake: wake.clone(),
            deadline: started + waited,
            waited_secs: waited.as_secs(),
            watch,
            events,
        };
        tokio::spawn(waiter.run(cancelled));
        Ok(wake)
    }

    pub fn list_wakes(&self, identity: &AgentIdentity) -> Vec<Wake> {
        self.runtime.wakes.list(identity.caller.id())
    }

    pub fn cancel_wake(&self, identity: &AgentIdentity, wake_id: &str) -> Result<()> {
        if self.runtime.wakes.take(identity.caller.id(), wake_id) {
            Ok(())
        } else {
            Err(invalid(format!("this agent has no pending wake {wake_id}")))
        }
    }

    async fn watch_for(
        &self,
        identity: &AgentIdentity,
        condition: &WakeCondition,
    ) -> Result<Watch> {
        let rt = self.runtime;
        match condition {
            WakeCondition::After { .. } | WakeCondition::At { .. } => Ok(Watch::Timer),
            WakeCondition::Ref { branch, remote } => {
                let remote = remote.clone().unwrap_or_else(|| "origin".into());
                let repo = PathBuf::from(&identity.project_path);
                let local = check_ref(&repo, &remote, branch).await?;
                let baseline = match ls_remote(rt, &repo, &remote, branch).await {
                    Ok(tip) => Baseline::Tip(tip),
                    Err(e) => {
                        tracing::debug!("wake on {remote}/{branch}: ls-remote failed, so the next one sets the baseline: {e}");
                        Baseline::Unread
                    }
                };
                Ok(Watch::Ref(RefWatch {
                    repo,
                    remote,
                    branch: branch.clone(),
                    baseline,
                    local,
                }))
            }
            WakeCondition::Pr {
                workstream_id,
                until,
            } => {
                self.check_workstream(identity, workstream_id).await?;
                Ok(Watch::Pr {
                    workstream_id: workstream_id.clone(),
                    until: *until,
                })
            }
            WakeCondition::Session { session_id, until } => {
                self.check_session(identity, session_id).await?;
                let target = Agent::Session(session_id.clone());
                let now = observe_agent(rt, &target).await;
                Ok(Watch::Agent {
                    target,
                    until: *until,
                    now,
                })
            }
            WakeCondition::Terminal { terminal_id, until } => {
                if !self.pty_target(identity, terminal_id).await?.agent.hooked {
                    return Err(denied(
                        "that agent's engine reports no state to wait on; read its terminal instead",
                    ));
                }
                let target = Agent::Terminal(terminal_id.clone());
                let now = observe_agent(rt, &target).await;
                Ok(Watch::Agent {
                    target,
                    until: *until,
                    now,
                })
            }
            WakeCondition::AnyAgent { until } => {
                let mut projects = HashMap::new();
                let seen = seed(
                    rt,
                    &identity.project_path,
                    identity.caller.id(),
                    &mut projects,
                )
                .await?;
                Ok(Watch::AnyAgent {
                    until: *until,
                    seen,
                    projects,
                })
            }
        }
    }
}

fn deadline(condition: &WakeCondition, timeout_ms: Option<u32>, now: f64) -> Result<f64> {
    let max_ms = (MAX_WAIT_SECS * 1000) as f64;
    match condition {
        WakeCondition::After { secs } if (1..=MAX_WAIT_SECS).contains(&u64::from(*secs)) => {
            Ok(now + f64::from(*secs) * 1000.0)
        }
        WakeCondition::After { .. } => {
            Err(invalid(format!("`after` takes 1 to {MAX_WAIT_SECS} secs")))
        }
        WakeCondition::At { unix_secs } => {
            let at = *unix_secs as f64 * 1000.0;
            if at > now && at - now <= max_ms {
                Ok(at)
            } else {
                Err(invalid(format!(
                    "`at` takes a unix time in the next {MAX_WAIT_SECS} s"
                )))
            }
        }
        _ => Ok(now + f64::from(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)).clamp(1000.0, max_ms)),
    }
}

/// The ref the event path reads; refuses a remote that isn't configured, so neither a URL nor
/// an option reaches `git ls-remote`.
async fn check_ref(repo: &Path, remote: &str, branch: &str) -> Result<Option<String>> {
    let (dir, remote, branch) = (repo.to_path_buf(), remote.to_string(), branch.to_string());
    blocking(move || {
        let repo = gitops::open_repo(&dir)
            .map_err(|_| invalid("this project is not a git repository".into()))?;
        if repo.find_remote(&remote).is_err() {
            return Err(invalid(format!("{remote} is not a remote of this project")));
        }
        if !git2::Reference::is_valid_name(&format!("refs/heads/{branch}")) {
            return Err(invalid(format!("{branch} is not a branch name")));
        }
        Ok(gitops::remote_tip(&dir, &remote, &branch))
    })
    .await
}

async fn ls_remote(
    rt: &HyprlayerRuntime,
    repo: &Path,
    remote: &str,
    branch: &str,
) -> Result<Option<String>> {
    let git = rt.binaries.require(BinaryKind::Git).await?;
    let overlay = rt.child_env().await;
    worktree::ls_remote_unattended(&git, repo, remote, branch, &overlay).await
}

#[derive(Debug, Clone)]
enum Agent {
    Session(String),
    Terminal(String),
}

impl Agent {
    fn id(&self) -> &str {
        match self {
            Agent::Session(id) | Agent::Terminal(id) => id,
        }
    }
}

enum Watch {
    Timer,
    Ref(RefWatch),
    Pr {
        workstream_id: String,
        until: PrWakeUntil,
    },
    Agent {
        target: Agent,
        until: WaitUntil,
        now: Observed,
    },
    AnyAgent {
        until: WaitUntil,
        seen: HashMap<String, Observed>,
        /// Workstream id → whether it is in the caller's project.
        projects: HashMap<String, bool>,
    },
}

impl Watch {
    fn first_poll(&self) -> Option<Duration> {
        match self {
            Watch::Ref(_) => Some(POLL),
            Watch::Pr { .. } => Some(Duration::ZERO),
            Watch::Timer | Watch::Agent { .. } | Watch::AnyAgent { .. } => None,
        }
    }

    /// Already true at the start: a session or terminal that is there already, or a cached PR.
    async fn holds(&self, rt: &HyprlayerRuntime) -> Option<String> {
        match self {
            Watch::Pr {
                workstream_id,
                until,
            } => cached_pr(rt, workstream_id, *until).await,
            Watch::Agent { target, until, now } if now.reaches(*until) => {
                Some(agent_report(rt, target, *now).await)
            }
            _ => None,
        }
    }

    async fn poll(&mut self, rt: &HyprlayerRuntime) -> Option<String> {
        match self {
            // ponytail: one ls-remote per ref waiter per minute; share per remote if waiters multiply
            Watch::Ref(watch) => {
                match ls_remote(rt, &watch.repo, &watch.remote, &watch.branch).await {
                    Ok(tip) => watch.remote_tip(tip),
                    Err(e) => {
                        tracing::debug!(
                            "wake on {}/{}: ls-remote failed: {e}",
                            watch.remote,
                            watch.branch
                        );
                        None
                    }
                }
            }
            Watch::Pr {
                workstream_id,
                until,
            } => {
                let cache = &rt.watchers.pr_cache;
                if cached_age(cache, workstream_id).is_some_and(|age| age < POLL) {
                    return None;
                }
                let item = match rt.workstreams().find(workstream_id).await {
                    Ok(item) => item,
                    Err(e) => {
                        tracing::debug!("wake on {workstream_id}'s PR: no refresh: {e}");
                        return None;
                    }
                };
                let lookup = rt.workstreams().refresh_pr_in_background(&item).await?;
                pr_report(&item, &lookup, *until).await
            }
            Watch::Timer | Watch::Agent { .. } | Watch::AnyAgent { .. } => None,
        }
    }

    async fn on_event(
        &mut self,
        rt: &HyprlayerRuntime,
        project: &str,
        me: &str,
        event: &RuntimeEvent,
    ) -> Option<String> {
        match self {
            Watch::Timer => None,
            Watch::Ref(watch) => match event {
                RuntimeEvent::WorktreeChanged { .. } => watch.reread().await,
                _ => None,
            },
            Watch::Pr {
                workstream_id,
                until,
            } => match event {
                RuntimeEvent::PrStatusChanged {
                    workstream_id: changed,
                    ..
                } if changed == workstream_id => cached_pr(rt, workstream_id, *until).await,
                _ => None,
            },
            Watch::Agent { target, until, now } => {
                let (agent, _, next) =
                    observed(rt, event, |id| (id == target.id()).then_some(*now))?;
                if agent.id() != target.id() {
                    return None;
                }
                *now = next;
                if next.reaches(*until) {
                    Some(agent_report(rt, target, next).await)
                } else {
                    None
                }
            }
            Watch::AnyAgent {
                until,
                seen,
                projects,
            } => {
                let (agent, workstream_id, next) = observed(rt, event, |id| seen.get(id).copied())?;
                if agent.id() == me || !in_project(rt, projects, workstream_id, project).await {
                    return None;
                }
                let before = seen
                    .insert(agent.id().to_string(), next)
                    .unwrap_or(Observed::Working);
                if !before.reaches(*until) && next.reaches(*until) {
                    Some(agent_report(rt, &agent, next).await)
                } else {
                    None
                }
            }
        }
    }

    /// After a lag: a target is observed afresh, and edges across the project are let go.
    async fn resync(&mut self, rt: &HyprlayerRuntime, project: &str, me: &str) -> Option<String> {
        match self {
            Watch::Timer => None,
            Watch::Ref(watch) => watch.reread().await,
            Watch::Agent { target, now, .. } => {
                *now = observe_agent(rt, target).await;
                self.holds(rt).await
            }
            Watch::Pr { .. } => self.holds(rt).await,
            Watch::AnyAgent { seen, projects, .. } => {
                match seed(rt, project, me, projects).await {
                    Ok(fresh) => *seen = fresh,
                    Err(e) => {
                        tracing::warn!("wake on any agent: re-reading the project failed: {e}")
                    }
                }
                None
            }
        }
    }
}

struct RefWatch {
    repo: PathBuf,
    remote: String,
    branch: String,
    baseline: Baseline,
    /// The local tracking ref as last read: a fetch catching up to the baseline is no move.
    local: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Baseline {
    /// No `ls-remote` has answered yet. The local tracking ref can lag the remote, so it never stands in.
    Unread,
    /// The remote's tip when the wait began; `None` while the branch is not there.
    Tip(Option<String>),
}

impl RefWatch {
    async fn reread(&mut self) -> Option<String> {
        let (dir, remote, branch) = (self.repo.clone(), self.remote.clone(), self.branch.clone());
        let now = blocking(move || Ok(gitops::remote_tip(&dir, &remote, &branch)))
            .await
            .ok()?;
        if now == self.local {
            return None;
        }
        self.local = now.clone();
        self.moved(now)
    }

    fn remote_tip(&mut self, tip: Option<String>) -> Option<String> {
        if self.baseline == Baseline::Unread {
            self.baseline = Baseline::Tip(tip);
            return None;
        }
        self.moved(tip)
    }

    fn moved(&self, now: Option<String>) -> Option<String> {
        let Baseline::Tip(baseline) = &self.baseline else {
            return None;
        };
        let name = format!("{}/{}", self.remote, self.branch);
        match (baseline, &now) {
            (was, now) if was == now => None,
            (Some(was), Some(now)) => Some(format!("{name} moved {} → {}", short(was), short(now))),
            (None, Some(now)) => Some(format!("{name} appeared at {}", short(now))),
            (Some(was), None) => Some(format!("{name} was deleted; it was at {}", short(was))),
            (None, None) => None,
        }
    }
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(7)]
}

async fn cached_pr(
    rt: &HyprlayerRuntime,
    workstream_id: &str,
    until: PrWakeUntil,
) -> Option<String> {
    let item = rt.workstreams().find(workstream_id).await.ok()?;
    let lookup = cached_for(&rt.watchers.pr_cache, &item)?;
    pr_report(&item, &lookup, until).await
}

/// Only an entry for the branch the workstream is on now: one from an earlier branch is another PR.
fn cached_for(cache: &PrCache, item: &Workstream) -> Option<PrLookup> {
    cached_lookup(cache, &item.id, &current_pr_ref(item)?)
}

async fn pr_report(item: &Workstream, lookup: &PrLookup, until: PrWakeUntil) -> Option<String> {
    let pr = lookup.known()?;
    let holds = match until {
        PrWakeUntil::Opened => true,
        PrWakeUntil::Merged => pr.state == PrState::Merged,
        PrWakeUntil::Closed => pr.state != PrState::Open,
        PrWakeUntil::ChecksDone => {
            matches!(pr.checks.state, ChecksState::Passing | ChecksState::Failing)
                && on_pushed_head(item, &pr.head_ref_oid).await
        }
    };
    holds.then(|| describe_pr(pr))
}

/// A cached PR can predate the branch's last push, and its checks then belong to the old head.
async fn on_pushed_head(item: &Workstream, head: &str) -> bool {
    let Some(branch) = current_pr_ref(item) else {
        return true;
    };
    let (dir, base) = (pr_dir(item).to_path_buf(), item.base_branch.clone());
    let pushed = blocking(move || {
        let repo = gitops::open_repo(&dir)?;
        Ok(gitops::pushed_ref(&repo, &branch, &base)
            .and_then(|r| r.target())
            .map(|tip| tip.to_string()))
    })
    .await;
    pushed.ok().flatten().is_none_or(|tip| tip == head)
}

fn describe_pr(pr: &PrStatus) -> String {
    let state = match pr.state {
        PrState::Open => "open",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
    };
    let checks = match pr.checks.state {
        ChecksState::None => "no checks",
        ChecksState::Pending => "checks pending",
        ChecksState::Passing => "checks passing",
        ChecksState::Failing => "checks failing",
    };
    format!(
        "PR #{} \"{}\" is {state}, {checks} — {}",
        pr.number, pr.title, pr.url
    )
}

fn lifecycle(state: SessionState) -> Option<Observed> {
    match state {
        SessionState::Completed
        | SessionState::Interrupted
        | SessionState::Killed
        | SessionState::SpawnFailed
        | SessionState::Crashed => Some(Observed::Exited),
        SessionState::AwaitingPermission | SessionState::AwaitingQuestion => {
            Some(Observed::NeedsYou)
        }
        SessionState::Running | SessionState::Starting => Some(Observed::Working),
        // Re-announced on a title change, so it says nothing about the turn.
        SessionState::Idle => None,
    }
}

/// The agent an event is about, its workstream, and where it now stands.
fn observed<'e>(
    rt: &HyprlayerRuntime,
    event: &'e RuntimeEvent,
    prior: impl FnOnce(&str) -> Option<Observed>,
) -> Option<(Agent, &'e str, Observed)> {
    match event {
        RuntimeEvent::SessionLifecycle {
            session_id,
            workstream_id,
            state,
        } => Some((
            Agent::Session(session_id.clone()),
            workstream_id,
            lifecycle(*state)?,
        )),
        RuntimeEvent::SessionRevised {
            session_id,
            workstream_id,
            working,
            ..
        } => {
            // An ask's own rows keep its turn working, and an exit's closing revision comes
            // before its lifecycle; only the turn's end or a relaunch's lifecycle moves either on.
            let now = match (prior(session_id), working) {
                (Some(Observed::Exited), _) => Observed::Exited,
                (Some(Observed::NeedsYou), true) => Observed::NeedsYou,
                (_, true) => Observed::Working,
                (_, false) => Observed::Idle,
            };
            Some((Agent::Session(session_id.clone()), workstream_id, now))
        }
        RuntimeEvent::PtyAgentChanged {
            terminal_id,
            workstream_id,
            ..
        } => Some((
            Agent::Terminal(terminal_id.clone()),
            workstream_id,
            pty_observed(rt, terminal_id),
        )),
        _ => None,
    }
}

fn pty_observed(rt: &HyprlayerRuntime, terminal_id: &str) -> Observed {
    match rt.pty_agents.snapshot(terminal_id) {
        None => Observed::Exited,
        Some(now) if now.prompt_owed => Observed::Working,
        Some(now) => observe_pty(now.agent.state),
    }
}

async fn observe_agent(rt: &HyprlayerRuntime, agent: &Agent) -> Observed {
    match agent {
        Agent::Session(id) => {
            let control = rt.agent_control();
            let asking = control.last_turn(id).await.is_ok_and(|turn| turn.asking);
            control.observe(id, asking).await
        }
        Agent::Terminal(id) => pty_observed(rt, id),
    }
}

/// The project's live agents but the caller, as they stand now.
async fn seed(
    rt: &HyprlayerRuntime,
    project: &str,
    me: &str,
    projects: &mut HashMap<String, bool>,
) -> Result<HashMap<String, Observed>> {
    let mut seen = HashMap::new();
    for active in rt.sessions().list_active().await? {
        let ours = active.workstream.project_path == project;
        projects.insert(active.workstream.id, ours);
        let target = Agent::Session(active.session.id);
        if ours && target.id() != me {
            let now = observe_agent(rt, &target).await;
            seen.insert(target.id().to_string(), now);
        }
    }
    for agent in rt.pty_agents.list(None) {
        if agent.hooked
            && agent.terminal_id != me
            && in_project(rt, projects, &agent.workstream_id, project).await
        {
            seen.insert(
                agent.terminal_id.clone(),
                pty_observed(rt, &agent.terminal_id),
            );
        }
    }
    Ok(seen)
}

async fn in_project(
    rt: &HyprlayerRuntime,
    projects: &mut HashMap<String, bool>,
    workstream_id: &str,
    project: &str,
) -> bool {
    if let Some(ours) = projects.get(workstream_id) {
        return *ours;
    }
    let (db, id) = (rt.db.clone(), workstream_id.to_string());
    let ours = blocking(move || db.get_workstream(&id))
        .await
        .ok()
        .flatten()
        .is_some_and(|ws| ws.project_path == project);
    projects.insert(workstream_id.to_string(), ours);
    ours
}

async fn agent_report(rt: &HyprlayerRuntime, agent: &Agent, now: Observed) -> String {
    let name = match agent {
        Agent::Session(id) => match rt.sessions().get(id).await {
            Ok(Some(session)) => format!("session {id} \"{}\"", session_label(&session)),
            _ => format!("session {id}"),
        },
        Agent::Terminal(id) => match rt.pty_agents.snapshot(id) {
            Some(s) => format!(
                "terminal {id} \"{}\"",
                s.agent.title.unwrap_or(s.agent.engine_id)
            ),
            None => format!("terminal {id}"),
        },
    };
    let state = match now {
        Observed::Working => "is working",
        Observed::NeedsYou => "needs you",
        Observed::Idle => "is idle",
        Observed::Exited => "has exited",
    };
    format!("{name} {state}")
}

fn summary(condition: &WakeCondition) -> String {
    let until = |until: &WaitUntil| match until {
        WaitUntil::Idle => "be idle",
        WaitUntil::NeedsYou => "need you",
        WaitUntil::Exited => "exit",
    };
    match condition {
        WakeCondition::After { secs } => format!("a {secs} s timer"),
        WakeCondition::At { unix_secs } => format!("unix time {unix_secs}"),
        WakeCondition::Ref { branch, remote } => {
            format!("{}/{branch} to move", remote.as_deref().unwrap_or("origin"))
        }
        WakeCondition::Pr {
            workstream_id,
            until,
        } => {
            let verb = match until {
                PrWakeUntil::Opened => "open",
                PrWakeUntil::Merged => "merge",
                PrWakeUntil::Closed => "close",
                PrWakeUntil::ChecksDone => "finish its checks",
            };
            format!("the PR of workstream {workstream_id} to {verb}")
        }
        WakeCondition::Session {
            session_id,
            until: u,
        } => format!("session {session_id} to {}", until(u)),
        WakeCondition::Terminal {
            terminal_id,
            until: u,
        } => format!("terminal {terminal_id} to {}", until(u)),
        WakeCondition::AnyAgent { until: u } => {
            format!("another agent in this project to {}", until(u))
        }
    }
}

enum Readiness {
    Ready,
    /// Idle, but its input would refuse a paste right now.
    Blocked,
    Busy,
    Gone,
}

fn pty_readiness(rt: &HyprlayerRuntime, terminal_id: &str) -> Readiness {
    let Some(now) = rt.pty_agents.snapshot(terminal_id) else {
        return Readiness::Gone;
    };
    if now.agent.state != PtyAgentState::Idle {
        Readiness::Busy
    } else if now.prompt_owed || paste_refusal(&rt.pty_agents, terminal_id).is_some() {
        Readiness::Blocked
    } else {
        Readiness::Ready
    }
}

fn author() -> ClientRef {
    ClientRef {
        client_id: format!("{}hyprlayer", ClientRef::AGENT_PREFIX),
        device: "hyprlayer".into(),
    }
}

struct Waiter {
    runtime: Weak<HyprlayerRuntime>,
    caller: AgentCaller,
    project: String,
    wake: Wake,
    deadline: Instant,
    waited_secs: u64,
    watch: Watch,
    events: broadcast::Receiver<RuntimeEvent>,
}

impl Waiter {
    async fn run(mut self, mut cancelled: oneshot::Receiver<()>) {
        let due = tokio::select! {
            _ = &mut cancelled => return,
            due = self.until_due() => due,
        };
        let Some(report) = due else {
            return self.clear();
        };
        let text = format!("[from hyprlayer · wake {}]\n\n{report}", self.wake.id);
        loop {
            let ready = tokio::select! {
                _ = &mut cancelled => return,
                ready = self.caller_ready() => ready,
            };
            if !ready {
                return self.clear();
            }
            if self.deliver(&text).await {
                return;
            }
        }
    }

    /// The report once the condition holds or the deadline passes; `None` once the caller ends.
    async fn until_due(&mut self) -> Option<String> {
        let rt = self.runtime.upgrade()?;
        if let Some(report) = self.watch.holds(&rt).await {
            return Some(report);
        }
        drop(rt);
        let mut poll = self.watch.first_poll().map(|first| {
            let mut every = tokio::time::interval_at(Instant::now() + first, POLL);
            every.set_missed_tick_behavior(MissedTickBehavior::Delay);
            every
        });
        loop {
            let tick = async {
                match poll.as_mut() {
                    Some(every) => {
                        every.tick().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                () = tokio::time::sleep_until(self.deadline) => return Some(self.expired()),
                () = tick => {
                    let rt = self.runtime.upgrade()?;
                    if let Some(report) = self.watch.poll(&rt).await {
                        return Some(report);
                    }
                }
                event = self.events.recv() => {
                    let rt = self.runtime.upgrade()?;
                    let report = match event {
                        Ok(event) if self.caller_ended(&rt, &event) => return None,
                        Ok(event) => {
                            self.watch
                                .on_event(&rt, &self.project, self.caller.id(), &event)
                                .await
                        }
                        Err(RecvError::Lagged(_)) if self.caller_gone(&rt).await => return None,
                        Err(RecvError::Lagged(_)) => {
                            self.watch.resync(&rt, &self.project, self.caller.id()).await
                        }
                        Err(RecvError::Closed) => return None,
                    };
                    if report.is_some() {
                        return report;
                    }
                }
            }
        }
    }

    fn expired(&self) -> String {
        match &self.wake.condition {
            WakeCondition::After { secs } => format!("timer up: {secs} s passed"),
            WakeCondition::At { unix_secs } => format!("timer up: it is unix time {unix_secs}"),
            other => format!(
                "timed out after {} s waiting for {}",
                self.waited_secs,
                summary(other)
            ),
        }
    }

    fn caller_ended(&self, rt: &HyprlayerRuntime, event: &RuntimeEvent) -> bool {
        match (&self.caller, event) {
            (
                AgentCaller::Session { session_id },
                RuntimeEvent::SessionLifecycle {
                    session_id: id,
                    state,
                    ..
                },
            ) => id == session_id && lifecycle(*state) == Some(Observed::Exited),
            (
                AgentCaller::Terminal { terminal_id },
                RuntimeEvent::PtyAgentChanged {
                    terminal_id: id,
                    state: PtyAgentState::Exited,
                    ..
                },
            ) => id == terminal_id && rt.pty_agents.snapshot(id).is_none(),
            _ => false,
        }
    }

    async fn caller_gone(&self, rt: &HyprlayerRuntime) -> bool {
        match &self.caller {
            AgentCaller::Session { session_id } => !rt.sessions().is_live(session_id).await,
            AgentCaller::Terminal { terminal_id } => rt.pty_agents.snapshot(terminal_id).is_none(),
        }
    }

    /// Waits on the caller's own events until it could take the message; `false` once it ends.
    async fn caller_ready(&mut self) -> bool {
        match self.caller.clone() {
            AgentCaller::Session { session_id } => loop {
                let Some(rt) = self.runtime.upgrade() else {
                    return false;
                };
                match observe_agent(&rt, &Agent::Session(session_id.clone())).await {
                    Observed::Idle => return true,
                    Observed::Exited => return false,
                    Observed::Working | Observed::NeedsYou => {}
                }
                drop(rt);
                if !self.next_about_caller(None).await {
                    return false;
                }
            },
            AgentCaller::Terminal { terminal_id } => {
                let Some(snapshot) = self
                    .runtime
                    .upgrade()
                    .and_then(|rt| rt.pty_agents.snapshot(&terminal_id))
                else {
                    return false;
                };
                if !snapshot.agent.hooked {
                    // Only a bypass caller gets here: a quiet screen stands in for idle, as for a first prompt.
                    let mut reader = snapshot.session.subscribe();
                    drop(snapshot);
                    return quiet_spell(&mut reader).await;
                }
                drop(snapshot);
                loop {
                    let Some(rt) = self.runtime.upgrade() else {
                        return false;
                    };
                    let retry = match pty_readiness(&rt, &terminal_id) {
                        Readiness::Ready => return true,
                        Readiness::Gone => return false,
                        Readiness::Busy => None,
                        Readiness::Blocked => Some(RECHECK),
                    };
                    drop(rt);
                    if !self.next_about_caller(retry).await {
                        return false;
                    }
                }
            }
        }
    }

    /// `true` at the caller's next turn end or state change, or once `retry` passes.
    async fn next_about_caller(&mut self, retry: Option<Duration>) -> bool {
        let until = retry.map(|after| Instant::now() + after);
        loop {
            let event = match until {
                Some(until) => match tokio::time::timeout_at(until, self.events.recv()).await {
                    Ok(event) => event,
                    Err(_) => return true,
                },
                None => self.events.recv().await,
            };
            let Some(rt) = self.runtime.upgrade() else {
                return false;
            };
            match event {
                Ok(event) if self.caller_ended(&rt, &event) => return false,
                Ok(event) if self.about_caller(&event) => return true,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => return !self.caller_gone(&rt).await,
                Err(RecvError::Closed) => return false,
            }
        }
    }

    fn about_caller(&self, event: &RuntimeEvent) -> bool {
        let me = self.caller.id();
        match event {
            RuntimeEvent::SessionLifecycle { session_id, .. } => session_id == me,
            RuntimeEvent::SessionRevised {
                session_id,
                working: false,
                ..
            } => session_id == me,
            RuntimeEvent::PtyAgentChanged { terminal_id, .. } => terminal_id == me,
            _ => false,
        }
    }

    /// `false` when the caller moved on before the claim, so the wait goes on.
    async fn deliver(&self, text: &str) -> bool {
        let Some(rt) = self.runtime.upgrade() else {
            return true;
        };
        let control = rt.agent_control();
        let id = &self.wake.id;
        match &self.caller {
            AgentCaller::Session { session_id } => {
                if !rt.wakes.take(session_id, id) {
                    return true;
                }
                let Ok(Some(session)) = rt.sessions().get(session_id).await else {
                    return true;
                };
                let label = session_label(&session);
                if let Err(e) = control
                    .admit_hop(session_id, &session.workstream_id, &label)
                    .await
                {
                    tracing::warn!("wake {id}: dropped for session {session_id}: {e}");
                    return true;
                }
                let ctx = SendContext {
                    base_rev: None,
                    author: Some(author()),
                };
                match rt
                    .sessions()
                    .send(session_id, text, &[], None, None, ctx)
                    .await
                {
                    Ok(()) => control.record_hop(session_id).await,
                    Err(e) => {
                        tracing::warn!("wake {id}: not delivered to session {session_id}: {e}")
                    }
                }
            }
            AgentCaller::Terminal { terminal_id } => {
                let Some(snapshot) = rt.pty_agents.snapshot(terminal_id) else {
                    rt.wakes.clear(terminal_id);
                    return true;
                };
                let _input = snapshot.input.lock().await;
                // Again under the lock: a message or bar change that held it may have moved the agent on.
                if snapshot.agent.hooked
                    && !matches!(pty_readiness(&rt, terminal_id), Readiness::Ready)
                {
                    return false;
                }
                if !rt.wakes.take(terminal_id, id) {
                    return true;
                }
                if let Err(e) = control
                    .admit_hop(
                        terminal_id,
                        &snapshot.agent.workstream_id,
                        &snapshot.agent.engine_id,
                    )
                    .await
                {
                    tracing::warn!("wake {id}: dropped for terminal {terminal_id}: {e}");
                    return true;
                }
                let agents = &rt.pty_agents;
                agents.owe_prompt(terminal_id, PROMPT_ACK);
                match paste(agents, terminal_id, &snapshot.session, text).await {
                    Ok(()) => control.record_hop(terminal_id).await,
                    Err(e) => {
                        tracing::warn!("wake {id}: not delivered to terminal {terminal_id}: {e}")
                    }
                }
            }
        }
        true
    }

    fn clear(&self) {
        if let Some(rt) = self.runtime.upgrade() {
            rt.wakes.clear(self.caller.id());
        }
    }
}

fn invalid(detail: String) -> CoreError {
    CoreError::WakeInvalid { detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::{MergeState, PrChecks};
    use crate::runtime::pr_watcher::store_lookup;
    use crate::store::{Db, NewWorkstream, WorktreeMode};

    fn workstream(branch: &str) -> Workstream {
        Db::open_in_memory()
            .unwrap()
            .insert_workstream(NewWorkstream {
                project_path: "/repo".into(),
                title: "t".into(),
                slug: "t-1".into(),
                branch: branch.into(),
                worktree_path: "/wt".into(),
                base_branch: "origin/main".into(),
                worktree_mode: WorktreeMode::Isolated,
            })
            .unwrap()
    }

    fn merged(number: u32) -> PrLookup {
        PrLookup::Found {
            pr: PrStatus {
                number,
                url: String::new(),
                title: String::new(),
                state: PrState::Merged,
                is_draft: false,
                base_ref_name: String::new(),
                head_ref_oid: String::new(),
                merge_state: MergeState::Unknown,
                checks: PrChecks {
                    state: ChecksState::None,
                    total: 0,
                    passed: 0,
                    failed: 0,
                    pending: 0,
                    skipped: 0,
                    cancelled: 0,
                    list: Vec::new(),
                },
                review_decision: None,
                reviews: Vec::new(),
            },
        }
    }

    #[test]
    fn a_pr_cached_for_the_workstreams_earlier_branch_does_not_count() {
        let cache = PrCache::default();
        let item = workstream("second");
        let now = std::time::Instant::now;
        store_lookup(&cache, &item.id, "first", merged(1), true, now());
        assert_eq!(cached_for(&cache, &item), None);
        store_lookup(&cache, &item.id, "second", merged(2), true, now());
        assert_eq!(cached_for(&cache, &item), Some(merged(2)));
    }

    #[test]
    fn a_ref_reports_no_move_until_ls_remote_sets_its_baseline() {
        let (was, now) = ("a".repeat(40), "b".repeat(40));
        let mut watch = RefWatch {
            repo: PathBuf::new(),
            remote: "origin".into(),
            branch: "feat".into(),
            baseline: Baseline::Unread,
            local: None,
        };
        assert_eq!(watch.moved(Some(now.clone())), None);
        assert_eq!(watch.remote_tip(Some(was.clone())), None);
        assert_eq!(watch.remote_tip(Some(was)), None);
        assert_eq!(
            watch.remote_tip(Some(now)).as_deref(),
            Some("origin/feat moved aaaaaaa → bbbbbbb")
        );
    }
}
