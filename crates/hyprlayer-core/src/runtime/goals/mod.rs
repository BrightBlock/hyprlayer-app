//! Goals: the service the RPC layer calls, and the registry of the driver tasks
//! (`sessions::goal_loop`) that run them.

pub(crate) mod checks;
pub(crate) mod decide;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex, OwnedMutexGuard};

use hyprlayer_models::goal::{
    Goal, GoalCheck, GoalCheckKind, GoalDetail, GoalEndReason, GoalLimits, GoalPauseReason,
    GoalSource, GoalSpec, GoalStatus, GoalStep,
};
use hyprlayer_models::{ClientRef, RuntimeEvent};

use crate::agent::{AgentDriver, Harness};
use crate::claude::PermissionMode;
use crate::docs::frontmatter;
use crate::docs::plan_outline::{plan_outline, BoxSection, PlanOutline, PlanPhase};
use crate::error::{CoreError, Result};
use crate::pipeline::{fix_findings_prompt, goal_check_kind, Stage};
use crate::repo_settings;
use crate::runtime::sessions::goal_loop::{self, StartMode};
use crate::runtime::sessions::{LaunchSessionParams, SessionLineage, StartStageSessionRequest};
use crate::settings::GoalSettings;
use crate::store::{now_ms, ControlRequest, FieldOp, GoalRow, GoalStateUpdate, NewGoal};

use super::shutdown::{self, TrackedTask};
use super::HyprlayerRuntime;

const MAX_ITERATIONS: u32 = 200;
const MIN_ACTIVE_SECS: u32 = 60;
const MAX_ACTIVE_SECS: u32 = 172_800;
const REASON_CHARS: usize = 200;
const WORKSTREAM_STOP_WAIT: Duration = Duration::from_secs(5);
const FINISHING_WAIT: Duration = Duration::from_secs(2);
/// A remote host may have no desktop to send one.
pub(crate) const OVERLAY_WAIT: Duration = Duration::from_secs(30);
pub(crate) const MAX_RESTART_RESUMES: u32 = 3;
const DERIVED_TIMEOUT_SECS: u32 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StartGoalRequest {
    pub target: GoalTarget,
    pub spec: GoalSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum GoalTarget {
    Session {
        session_id: String,
    },
    NewSession {
        workstream_id: String,
        stage: Stage,
        #[serde(default)]
        harness: Option<Harness>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        permission_mode: Option<PermissionMode>,
        #[serde(default)]
        attached_docs: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GoalPatch {
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub work_prompt: Option<String>,
    #[serde(default)]
    pub checks: Option<Vec<GoalCheck>>,
    #[serde(default)]
    pub limits: Option<GoalLimits>,
}

/// A source to propose a goal from. `phases` is the plan's scope: empty for the whole plan,
/// one id for a phase, several for a range.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DeriveGoalRequest {
    pub workstream_id: String,
    pub source: GoalSource,
    #[serde(default)]
    pub harness: Option<Harness>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub plan_path: Option<String>,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub research_path: Option<String>,
    #[serde(default)]
    pub findings: Vec<String>,
}

/// What a runner is told to do; it applies each at its next safe point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunnerControl {
    Pause,
    Stop(GoalEndReason),
    Reload,
    UserInterrupt,
}

/// How a goal's run ends; a met goal is `End(ChecksPassed)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Conclusion {
    End(GoalEndReason),
    Pause(GoalPauseReason, Option<String>),
}

impl Conclusion {
    pub(crate) fn update(&self) -> GoalStateUpdate {
        match self {
            Conclusion::End(reason) => GoalStateUpdate {
                status: FieldOp::Set(reason.status()),
                step: FieldOp::Clear,
                pause_reason: FieldOp::Clear,
                end_reason: FieldOp::Set(*reason),
                ended_at: FieldOp::Set(now_ms()),
                control_request: FieldOp::Clear,
                ..Default::default()
            },
            Conclusion::Pause(reason, text) => GoalStateUpdate {
                status: FieldOp::Set(GoalStatus::Paused),
                step: FieldOp::Clear,
                pause_reason: FieldOp::Set(*reason),
                end_reason: FieldOp::Clear,
                last_reason: text.clone().map(FieldOp::Set).unwrap_or_default(),
                control_request: FieldOp::Clear,
                ..Default::default()
            },
        }
    }
}

impl ControlRequest {
    pub(crate) fn conclusion(self) -> Conclusion {
        match self {
            ControlRequest::Pause => Conclusion::Pause(GoalPauseReason::User, None),
            ControlRequest::Stop(reason) => Conclusion::End(reason),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

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

fn invalid(detail: impl Into<String>) -> CoreError {
    CoreError::GoalInvalid {
        detail: detail.into(),
    }
}

fn announced_reason(reason: Option<&str>) -> Option<String> {
    reason.map(|r| r.chars().take(REASON_CHARS).collect())
}

pub(crate) fn announce(events: &tokio::sync::broadcast::Sender<RuntimeEvent>, goal: &Goal) {
    let _ = events.send(RuntimeEvent::GoalChanged {
        goal_id: goal.id.clone(),
        session_id: goal.session_id.clone(),
        workstream_id: goal.workstream_id.clone(),
        status: goal.status,
        step: goal.step,
        pause_reason: goal.pause_reason,
        end_reason: goal.end_reason,
        reason: announced_reason(goal.last_reason.as_deref()),
        iteration: goal.iteration,
        max_iterations: goal.limits.max_iterations,
    });
}

fn validate_spec(spec: &GoalSpec, harness: Harness) -> Result<()> {
    if !spec.checks.iter().any(|check| {
        matches!(
            goal_check_kind(check),
            GoalCheckKind::Command | GoalCheckKind::Evaluator | GoalCheckKind::Validation
        )
    }) {
        return Err(invalid(
            "A goal needs at least one command, evaluator or validation check.",
        ));
    }
    for check in &spec.checks {
        match check {
            GoalCheck::Command {
                command,
                timeout_secs,
                cwd,
                ..
            } => {
                if command.trim().is_empty() {
                    return Err(invalid("A command check needs a command."));
                }
                if *timeout_secs == 0 {
                    return Err(invalid(
                        "A command check needs a timeout of a second or more.",
                    ));
                }
                if checks::run_dir(cwd.as_deref()).is_some_and(|dir| !checks::stays_inside(dir)) {
                    return Err(invalid(
                        "A command check's directory must be a relative path inside the worktree.",
                    ));
                }
            }
            GoalCheck::PlanBoxes { plan_path, .. } => {
                if plan_path.trim().is_empty() {
                    return Err(invalid("A plan boxes check needs a plan."));
                }
            }
            GoalCheck::Evaluator { .. } => {}
            GoalCheck::Validation { plan_path } => {
                if plan_path.trim().is_empty() {
                    return Err(invalid("A validation check needs a plan."));
                }
            }
        }
    }
    let limits = &spec.limits;
    if !(1..=MAX_ITERATIONS).contains(&limits.max_iterations) {
        return Err(invalid(format!(
            "Iterations must be between 1 and {MAX_ITERATIONS}."
        )));
    }
    if !(MIN_ACTIVE_SECS..=MAX_ACTIVE_SECS).contains(&limits.max_active_secs) {
        return Err(invalid(format!(
            "Active time must be between {MIN_ACTIVE_SECS} seconds and {MAX_ACTIVE_SECS} seconds."
        )));
    }
    match harness {
        Harness::Claude | Harness::Opencode => {
            if !limits.max_cost_usd.is_some_and(|cost| cost > 0.0) {
                return Err(invalid(
                    "A goal on this harness needs a spend cap in dollars.",
                ));
            }
        }
        Harness::Codex => {
            if !limits.max_tokens.is_some_and(|tokens| tokens > 0) {
                return Err(invalid("A goal on Codex needs a token cap."));
            }
        }
    }
    Ok(())
}

fn limit_raised(goal: &Goal) -> bool {
    let limits = &goal.limits;
    match goal.end_reason {
        Some(GoalEndReason::IterationLimit) => limits.max_iterations > goal.iteration,
        Some(GoalEndReason::BudgetLimit) => limits
            .max_cost_usd
            .is_some_and(|cap| cap > goal.spent_cost_usd),
        Some(GoalEndReason::TokenLimit) => {
            limits.max_tokens.is_some_and(|cap| cap > goal.spent_tokens)
        }
        Some(GoalEndReason::TimeLimit) => {
            u64::from(limits.max_active_secs) * 1000 > u64::from(goal.active_ms)
        }
        _ => false,
    }
}

fn running_again() -> GoalStateUpdate {
    GoalStateUpdate {
        status: FieldOp::Set(GoalStatus::Running),
        step: FieldOp::Set(GoalStep::Working),
        pause_reason: FieldOp::Clear,
        end_reason: FieldOp::Clear,
        ended_at: FieldOp::Clear,
        // The crash-loop guards count automatic retries; a person's resume starts them over.
        harness_crashes: FieldOp::Set(0),
        restart_resumes: FieldOp::Set(0),
        ..Default::default()
    }
}

/// One driver task per running goal, found by goal id.
#[derive(Default)]
pub(crate) struct GoalRunners {
    runtime: OnceLock<Weak<HyprlayerRuntime>>,
    live: Mutex<HashMap<String, Runner>>,
    generations: AtomicU64,
    /// Holds a service transition's read-check-write apart from a runner's final write.
    transitions: Arc<AsyncMutex<()>>,
}

struct Runner {
    generation: u64,
    control: mpsc::UnboundedSender<RunnerControl>,
    finishing: Arc<AtomicBool>,
    /// Closes when the runner's task ends; nothing is ever sent on it.
    exited: watch::Receiver<()>,
    task: TrackedTask,
}

pub(crate) enum Delivery {
    Delivered,
    /// The runner is writing its last state; a control would be lost.
    Finishing,
    NoRunner,
}

/// What a runner holds of its own registry entry.
pub(crate) struct Lease {
    runners: Arc<GoalRunners>,
    finishing: Arc<AtomicBool>,
}

impl Lease {
    /// Marked under the registry lock, so a racing `spawn` either sees it or already chained.
    pub(crate) fn finishing(&self) {
        let _live = lock(&self.runners.live);
        self.finishing.store(true, Ordering::SeqCst);
    }

    /// Owned, so the runner can keep writing through `&mut self` while it holds it.
    pub(crate) async fn transition(&self) -> OwnedMutexGuard<()> {
        self.runners.transitions.clone().lock_owned().await
    }
}

/// Removes the runner's entry when its task ends, however it ends.
struct Registration {
    runners: Arc<GoalRunners>,
    goal_id: String,
    generation: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut live = lock(&self.runners.live);
        if live
            .get(&self.goal_id)
            .is_some_and(|runner| runner.generation == self.generation)
        {
            live.remove(&self.goal_id);
        }
    }
}

impl GoalRunners {
    /// A `&HyprlayerRuntime` cannot mint a `Weak`, so `start()` hands one over.
    pub(crate) fn bind(&self, runtime: Weak<HyprlayerRuntime>) {
        let _ = self.runtime.set(runtime);
    }

    /// A live runner is left alone; a finishing one is followed by a new runner that waits for its exit.
    /// `owned` is a harness launched with the goal's budget cap, which the runner then owns.
    pub(crate) fn spawn(
        self: &Arc<Self>,
        goal_id: &str,
        owned: Option<Weak<dyn AgentDriver>>,
        mode: StartMode,
    ) -> Result<()> {
        let runtime = self.runtime.get().ok_or(CoreError::RuntimeStopped)?.clone();
        let mut live = lock(&self.live);
        let earlier = match live.get(goal_id) {
            Some(runner) if !runner.finishing.load(Ordering::SeqCst) => return Ok(()),
            Some(runner) => Some(runner.exited.clone()),
            None => None,
        };
        let generation = self.generations.fetch_add(1, Ordering::SeqCst);
        let (control, inbox) = mpsc::unbounded_channel();
        let finishing = Arc::new(AtomicBool::new(false));
        let (done, exited) = watch::channel(());
        let lease = Lease {
            runners: self.clone(),
            finishing: finishing.clone(),
        };
        let registration = Registration {
            runners: self.clone(),
            goal_id: goal_id.to_string(),
            generation,
        };
        let id = goal_id.to_string();
        let task = TrackedTask::spawn(move |stop| async move {
            let _done = done;
            let _registration = registration;
            if let Some(mut earlier) = earlier {
                while earlier.changed().await.is_ok() {}
            }
            goal_loop::run(runtime, id, inbox, stop, lease, owned, mode).await;
        });
        live.insert(
            goal_id.to_string(),
            Runner {
                generation,
                control,
                finishing,
                exited,
                task,
            },
        );
        Ok(())
    }

    fn has_runner(&self, goal_id: &str) -> bool {
        lock(&self.live).contains_key(goal_id)
    }

    pub(crate) fn send(&self, goal_id: &str, control: RunnerControl) -> Delivery {
        let live = lock(&self.live);
        match live.get(goal_id) {
            None => Delivery::NoRunner,
            Some(runner) if runner.finishing.load(Ordering::SeqCst) => Delivery::Finishing,
            Some(runner) => match runner.control.send(control) {
                Ok(()) => Delivery::Delivered,
                Err(_) => Delivery::NoRunner,
            },
        }
    }

    pub(crate) async fn wait_exit(&self, goal_id: &str, timeout: Duration) {
        let exited = lock(&self.live).get(goal_id).map(|r| r.exited.clone());
        let Some(mut exited) = exited else {
            return;
        };
        let _ =
            tokio::time::timeout(timeout, async { while exited.changed().await.is_ok() {} }).await;
    }

    /// Rows are left as they are: the next open resumes or pauses them.
    pub(crate) async fn stop_all(&self) {
        let runners: Vec<Runner> = lock(&self.live).drain().map(|(_, r)| r).collect();
        let mut stopping = tokio::task::JoinSet::new();
        for Runner { task, .. } in runners {
            stopping.spawn(task.stop(shutdown::PER_TASK_TIMEOUT));
        }
        while stopping.join_next().await.is_some() {}
    }
}

/// Re-arms the goals a previous server left running, once a client's environment is back.
pub(crate) async fn resume_running(runners: &Arc<GoalRunners>, runtime: &Weak<HyprlayerRuntime>) {
    let Some(rt) = runtime.upgrade() else {
        return;
    };
    let mut arrived = rt.overlay_arrived.subscribe();
    let db = rt.db.clone();
    drop(rt);
    match blocking(move || db.goals_with_status(GoalStatus::Running)).await {
        Ok(running) if running.is_empty() => return,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("goals a previous run left running were not read: {e}");
            return;
        }
    }
    // Without it a relaunched harness and its checks miss the client's SSH_AUTH_SOCK and LANG.
    if tokio::time::timeout(OVERLAY_WAIT, arrived.wait_for(|arrived| *arrived))
        .await
        .is_err()
    {
        tracing::info!("no client environment within {OVERLAY_WAIT:?}; resuming goals without it");
    }
    let Some(rt) = runtime.upgrade() else {
        return;
    };
    let db = rt.db.clone();
    let running = match blocking(move || db.goals_with_status(GoalStatus::Running)).await {
        Ok(running) => running,
        Err(e) => {
            tracing::warn!("goals a previous run left running were not read: {e}");
            return;
        }
    };
    for row in running {
        let id = row.goal.id.clone();
        if let Err(e) = resume_one(runners, &rt, &id).await {
            tracing::warn!("goal {id}: not resumed after the restart: {e}");
        }
    }
}

async fn resume_one(
    runners: &Arc<GoalRunners>,
    rt: &HyprlayerRuntime,
    goal_id: &str,
) -> Result<()> {
    let step = {
        let _transition = runners.transitions.lock().await;
        let (db, id) = (rt.db.clone(), goal_id.to_string());
        let Some(row) = blocking(move || db.goal(&id)).await? else {
            return Ok(());
        };
        // A client's resume may have got here first.
        if row.goal.status != GoalStatus::Running || runners.has_runner(goal_id) {
            return Ok(());
        }
        let resumes = row.restart_resumes.saturating_add(1);
        // A pause or stop already accepted concludes as soon as the runner opens.
        let looping = resumes > MAX_RESTART_RESUMES && row.control_request.is_none();
        let update = if looping {
            let reason =
                format!("The server restarted {resumes} times before an iteration finished");
            GoalStateUpdate {
                restart_resumes: FieldOp::Set(resumes),
                ..Conclusion::Pause(GoalPauseReason::RestartLoop, Some(reason)).update()
            }
        } else {
            GoalStateUpdate {
                step: FieldOp::Set(GoalStep::Resuming),
                restart_resumes: FieldOp::Set(resumes),
                ..Default::default()
            }
        };
        let (db, id) = (rt.db.clone(), goal_id.to_string());
        let Some(written) = blocking(move || db.update_goal_state(&id, update)).await? else {
            return Ok(());
        };
        announce(&rt.events, &written.goal);
        if looping {
            tracing::info!("goal {goal_id}: pauses as restart_loop after {resumes} restarts");
            return Ok(());
        }
        tracing::info!(
            "goal {goal_id}: resuming after a server restart ({resumes} of {MAX_RESTART_RESUMES})"
        );
        row.goal.step
    };
    runners.spawn(goal_id, None, StartMode::Resume { step })
}

pub struct GoalService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn goals(&self) -> GoalService<'_> {
        GoalService { runtime: self }
    }
}

impl GoalService<'_> {
    pub async fn start(
        &self,
        request: StartGoalRequest,
        author: Option<ClientRef>,
    ) -> Result<Goal> {
        self.start_with(request, author, SessionLineage::goal())
            .await
    }

    /// `lineage` is what a `NewSession` target's row records; an agent's goal names the agent.
    pub async fn start_with(
        &self,
        request: StartGoalRequest,
        author: Option<ClientRef>,
        lineage: SessionLineage,
    ) -> Result<Goal> {
        ensure_running(self.runtime)?;
        let StartGoalRequest { target, spec } = request;
        let sessions = self.runtime.sessions();
        let (session, fresh, owned) =
            match target {
                GoalTarget::Session { session_id } => {
                    let session = sessions.get(&session_id).await?.ok_or_else(|| {
                        CoreError::SessionNotFound {
                            session_id: session_id.clone(),
                        }
                    })?;
                    validate_spec(&spec, session.harness)?;
                    if self.live_for_session(&session_id).await?.is_some() {
                        return Err(CoreError::GoalActive { session_id });
                    }
                    (session, false, None)
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
                    if !matches!(stage, Stage::General | Stage::Research | Stage::Implement) {
                        return Err(invalid(
                            "A goal can start a general, research or implement session.",
                        ));
                    }
                    validate_spec(&spec, harness.unwrap_or_default())?;
                    let workstream = self.runtime.workstreams().find(&workstream_id).await?;
                    let project_path = workstream.project_path.clone();
                    // The docs only activate the plan: iteration 1 is the goal prompt, which names it.
                    let started = sessions
                        .prepare_start_with(
                            StartStageSessionRequest {
                                workstream,
                                stage,
                                input: spec.work_prompt.clone(),
                                attached_docs,
                                harness,
                                model: model.clone(),
                                effort: effort.clone(),
                                permission_mode,
                                prompt_name: None,
                                client_message_id: None,
                                parent_session_id: None,
                            },
                            lineage,
                        )
                        .await?;
                    let cap = spec
                        .limits
                        .max_cost_usd
                        .filter(|_| started.session.harness == Harness::Claude);
                    let launched = sessions
                        .launch_driver(LaunchSessionParams {
                            session: started.session.clone(),
                            project_path,
                            model,
                            effort,
                            permission_mode,
                            resume: false,
                            boundary_on_relaunch: false,
                            max_budget_usd: cap,
                            extra_allowed_tools: Vec::new(),
                            extra_disallowed_tools: Vec::new(),
                            report_only: false,
                        })
                        .await;
                    let driver = match launched {
                        Ok(driver) => driver,
                        Err(e) => {
                            sessions.discard_unlaunched(&started.session).await;
                            return Err(e);
                        }
                    };
                    let owned = cap.map(|_| Arc::downgrade(&driver));
                    (started.session, true, owned)
                }
            };

        let new = NewGoal {
            session_id: session.id.clone(),
            workstream_id: session.workstream_id.clone(),
            spec,
            author,
        };
        let db = self.runtime.db.clone();
        let inserted = match blocking(move || db.insert_goal(new)).await {
            Ok(row) => row,
            Err(e) => {
                if fresh {
                    let _ = sessions.kill(&session.id).await;
                }
                return Err(e);
            }
        };
        self.announce(&inserted.goal);
        self.runtime
            .goal_runners
            .spawn(&inserted.goal.id, owned, StartMode::Normal)?;
        Ok(inserted.goal)
    }

    /// A running goal pauses once its iteration and checks are done.
    pub async fn pause(&self, goal_id: &str) -> Result<Goal> {
        ensure_running(self.runtime)?;
        for _ in 0..2 {
            if let Some(goal) = self.try_pause(goal_id).await? {
                return Ok(goal);
            }
            self.runtime
                .goal_runners
                .wait_exit(goal_id, FINISHING_WAIT)
                .await;
        }
        self.load(goal_id).await.map(|row| row.goal)
    }

    /// Paused goals run again; so do exhausted ones whose hit limit was raised and a running one with no runner.
    pub async fn resume(&self, goal_id: &str) -> Result<Goal> {
        ensure_running(self.runtime)?;
        let runners = &self.runtime.goal_runners;
        let goal = {
            let _transition = runners.transitions.lock().await;
            let row = self.load(goal_id).await?;
            match row.goal.status {
                GoalStatus::Running => row.goal,
                GoalStatus::Paused => self.write_running(goal_id).await?,
                GoalStatus::Exhausted => {
                    if !limit_raised(&row.goal) {
                        return Err(CoreError::GoalFinished {
                            goal_id: goal_id.to_string(),
                        });
                    }
                    if self.live_for_session(&row.goal.session_id).await?.is_some() {
                        return Err(CoreError::GoalActive {
                            session_id: row.goal.session_id,
                        });
                    }
                    self.write_running(goal_id).await?
                }
                _ => {
                    return Err(CoreError::GoalFinished {
                        goal_id: goal_id.to_string(),
                    })
                }
            }
        };
        runners.spawn(goal_id, None, StartMode::Normal)?;
        Ok(goal)
    }

    /// A running goal's runner interrupts its turn; any other live goal is ended here.
    pub async fn stop(&self, goal_id: &str) -> Result<Goal> {
        ensure_running(self.runtime)?;
        let row = self.load(goal_id).await?;
        if !row.goal.status.is_live() {
            return Err(CoreError::GoalFinished {
                goal_id: goal_id.to_string(),
            });
        }
        self.end(goal_id, GoalEndReason::StoppedByUser).await
    }

    /// Applies at the runner's next boundary; the running iteration is untouched.
    pub async fn update(&self, goal_id: &str, patch: GoalPatch) -> Result<Goal> {
        ensure_running(self.runtime)?;
        let runners = &self.runtime.goal_runners;
        let goal = {
            let _transition = runners.transitions.lock().await;
            let row = self.load(goal_id).await?;
            let goal = row.goal;
            if !matches!(
                goal.status,
                GoalStatus::Running | GoalStatus::Paused | GoalStatus::Exhausted
            ) {
                return Err(CoreError::GoalFinished {
                    goal_id: goal_id.to_string(),
                });
            }
            let session = self
                .runtime
                .sessions()
                .get(&goal.session_id)
                .await?
                .ok_or_else(|| CoreError::SessionNotFound {
                    session_id: goal.session_id.clone(),
                })?;
            let GoalPatch {
                condition,
                work_prompt,
                checks,
                limits,
            } = patch;
            let merged = GoalSpec {
                condition: condition.unwrap_or(goal.condition),
                work_prompt: work_prompt.unwrap_or(goal.work_prompt),
                source: goal.source,
                source_ref: goal.source_ref,
                source_phase: goal.source_phase,
                checks: checks.unwrap_or(goal.checks),
                limits: limits.unwrap_or(goal.limits),
            };
            validate_spec(&merged, session.harness)?;
            let (db, id) = (self.runtime.db.clone(), goal_id.to_string());
            let written = blocking(move || {
                db.update_goal_spec(
                    &id,
                    &merged.condition,
                    &merged.work_prompt,
                    &merged.checks,
                    &merged.limits,
                )
            })
            .await?;
            written
                .ok_or_else(|| CoreError::GoalNotFound {
                    goal_id: goal_id.to_string(),
                })?
                .goal
        };
        let _ = runners.send(goal_id, RunnerControl::Reload);
        self.announce(&goal);
        Ok(goal)
    }

    pub async fn get(&self, goal_id: &str) -> Result<GoalDetail> {
        ensure_running(self.runtime)?;
        let row = self.load(goal_id).await?;
        let (db, id) = (self.runtime.db.clone(), goal_id.to_string());
        let iterations = blocking(move || db.goal_iterations(&id)).await?;
        Ok(GoalDetail {
            goal: row.goal,
            iterations,
        })
    }

    /// Newest first.
    pub async fn list(&self, workstream_id: Option<&str>, active_only: bool) -> Result<Vec<Goal>> {
        ensure_running(self.runtime)?;
        let (db, ws) = (self.runtime.db.clone(), workstream_id.map(str::to_string));
        blocking(move || db.list_goals(ws.as_deref(), active_only)).await
    }

    /// Proposes a goal for a source; nothing is stored. Plan commands and the condition are copied
    /// here, so a later edit to the plan does not move a goal started from it.
    pub async fn derive(&self, request: DeriveGoalRequest) -> Result<GoalSpec> {
        ensure_running(self.runtime)?;
        let workstream = self
            .runtime
            .workstreams()
            .find(&request.workstream_id)
            .await?;
        let settings = self.runtime.settings.get().await;
        let harness = request
            .harness
            .or_else(|| {
                let default = settings.providers.default_harness.as_deref();
                default.map(Harness::parse)
            })
            .unwrap_or_default();
        let limits = derived_limits(&settings.goals, harness);
        let worktree = PathBuf::from(&workstream.worktree_path);
        let evaluator = GoalCheck::Evaluator { model: None };

        match request.source {
            GoalSource::Composer => {
                let text = required(request.text, "A goal from the composer needs its text.")?;
                let project = PathBuf::from(&workstream.project_path);
                let repo_checks = match blocking(move || repo_settings::read(&project)).await {
                    Ok(settings) => settings.goals.checks,
                    Err(e) => {
                        tracing::warn!("the repo's goal checks were not read: {e}");
                        Vec::new()
                    }
                };
                let mut checks = command_checks(repo_checks.into_iter().map(ProposedCommand::bare));
                checks.push(evaluator);
                Ok(GoalSpec {
                    condition: text.clone(),
                    work_prompt: text,
                    source: GoalSource::Composer,
                    source_ref: None,
                    source_phase: None,
                    checks,
                    limits,
                })
            }
            GoalSource::Plan | GoalSource::PlanPhase => {
                let plan_path = required(request.plan_path, "A goal from a plan needs the plan.")?;
                let outline = read_outline(&worktree, &plan_path).await?;
                let scope = PlanScope::of(&outline, &request.phases)?;
                let criteria = Criteria::of(&scope.phases);
                let title = plan_title(&outline, &plan_path);
                let mut condition = format!(
                    "{} of {title} {} automated criteria pass",
                    scope.subject(),
                    scope.predicate(),
                );
                condition.push_str(&criteria.prose_suffix());
                let mut checks = command_checks(criteria.commands);
                if scope.ids.is_empty() {
                    checks.push(GoalCheck::PlanBoxes {
                        plan_path: plan_path.clone(),
                        phase: None,
                    });
                }
                checks.extend(scope.ids.iter().map(|id| GoalCheck::PlanBoxes {
                    plan_path: plan_path.clone(),
                    phase: Some(id.clone()),
                }));
                if !criteria.prose.is_empty() {
                    checks.push(evaluator);
                }
                Ok(GoalSpec {
                    condition,
                    work_prompt: format!("/implement_plan {plan_path} {}", scope.instruction()),
                    source: scope.source(),
                    source_ref: Some(plan_path),
                    source_phase: scope.label(),
                    checks,
                    limits,
                })
            }
            GoalSource::Research => {
                let research_path = required(
                    request.research_path,
                    "A goal from research needs the research document.",
                )?;
                let path = worktree.join(&research_path);
                let content = blocking(move || Ok(std::fs::read_to_string(path)?)).await?;
                let question = research_question(&content)
                    .or_else(|| file_stem(&research_path))
                    .ok_or_else(|| invalid("The research document has no question."))?;
                Ok(GoalSpec {
                    condition: format!("A research document answers: {question}"),
                    work_prompt: format!("/research_codebase {question}"),
                    source: GoalSource::Research,
                    source_ref: Some(research_path),
                    source_phase: None,
                    checks: vec![evaluator],
                    limits,
                })
            }
            GoalSource::Validation => {
                if request.findings.is_empty() {
                    return Err(invalid("A goal from validation needs its findings."));
                }
                let mut checks = Vec::new();
                let mut source_phase = None;
                if let Some(plan_path) = request.plan_path.as_deref() {
                    let outline = read_outline(&worktree, plan_path).await?;
                    let scope = PlanScope::of(&outline, &request.phases)?;
                    checks = command_checks(Criteria::of(&scope.phases).commands);
                    source_phase = scope.label();
                }
                checks.push(evaluator);
                Ok(GoalSpec {
                    condition: format!(
                        "These validation findings are resolved and the plan's automated checks pass: {}",
                        request.findings.join("; ")
                    ),
                    work_prompt: fix_findings_prompt(&request.findings),
                    source: GoalSource::Validation,
                    source_ref: request.plan_path,
                    source_phase,
                    checks,
                    limits,
                })
            }
        }
    }

    /// Ends the workstream's live goals and waits for their runners, so none sees the archive's kills.
    pub(crate) async fn stop_for_workstream(&self, workstream_id: &str) {
        let (db, ws) = (self.runtime.db.clone(), workstream_id.to_string());
        let goals = match blocking(move || db.list_goals(Some(&ws), true)).await {
            Ok(goals) => goals,
            Err(e) => {
                tracing::warn!("workstream {workstream_id}: its goals were not read: {e}");
                return;
            }
        };
        for goal in &goals {
            if let Err(e) = self.end(&goal.id, GoalEndReason::WorkstreamArchived).await {
                tracing::warn!("goal {}: not stopped for the archive: {e}", goal.id);
            }
        }
        let deadline = tokio::time::Instant::now() + WORKSTREAM_STOP_WAIT;
        for goal in &goals {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            self.runtime
                .goal_runners
                .wait_exit(&goal.id, remaining)
                .await;
        }
    }

    pub(crate) async fn note_user_interrupt(&self, session_id: &str) {
        if let Some(goal) = self.running_goal(session_id).await {
            let _ = self
                .runtime
                .goal_runners
                .send(&goal.id, RunnerControl::UserInterrupt);
        }
    }

    pub(crate) async fn note_session_killed(&self, session_id: &str) {
        let Some(goal) = self.live_goal(session_id).await else {
            return;
        };
        if let Err(e) = self.end(&goal.id, GoalEndReason::SessionEnded).await {
            tracing::warn!("goal {}: not stopped with its session: {e}", goal.id);
        }
    }

    async fn load(&self, goal_id: &str) -> Result<GoalRow> {
        let (db, id) = (self.runtime.db.clone(), goal_id.to_string());
        blocking(move || db.goal(&id))
            .await?
            .ok_or_else(|| CoreError::GoalNotFound {
                goal_id: goal_id.to_string(),
            })
    }

    async fn live_for_session(&self, session_id: &str) -> Result<Option<GoalRow>> {
        let (db, id) = (self.runtime.db.clone(), session_id.to_string());
        blocking(move || db.live_goal_for_session(&id)).await
    }

    async fn live_goal(&self, session_id: &str) -> Option<Goal> {
        match self.live_for_session(session_id).await {
            Ok(row) => row.map(|r| r.goal),
            Err(e) => {
                tracing::warn!("session {session_id}: its goal was not read: {e}");
                None
            }
        }
    }

    async fn running_goal(&self, session_id: &str) -> Option<Goal> {
        self.live_goal(session_id)
            .await
            .filter(|goal| goal.status == GoalStatus::Running)
    }

    /// Tells a running goal's runner to stop, or ends the goal here when none runs it.
    async fn end(&self, goal_id: &str, reason: GoalEndReason) -> Result<Goal> {
        for _ in 0..2 {
            if let Some(goal) = self.try_end(goal_id, reason).await? {
                return Ok(goal);
            }
            self.runtime
                .goal_runners
                .wait_exit(goal_id, FINISHING_WAIT)
                .await;
        }
        self.load(goal_id).await.map(|row| row.goal)
    }

    /// `None` when the runner is writing its last state and the caller should wait it out.
    async fn try_pause(&self, goal_id: &str) -> Result<Option<Goal>> {
        let runners = &self.runtime.goal_runners;
        let _transition = runners.transitions.lock().await;
        let row = self.load(goal_id).await?;
        match row.goal.status {
            GoalStatus::Paused => Ok(Some(row.goal)),
            GoalStatus::Running => {
                let stopping = matches!(row.control_request, Some(ControlRequest::Stop(_)));
                match runners.send(goal_id, RunnerControl::Pause) {
                    Delivery::Delivered if stopping => {
                        Ok(Some(with_step(row.goal, GoalStep::Stopping)))
                    }
                    Delivery::Delivered => {
                        let goal = self.write_request(goal_id, ControlRequest::Pause).await?;
                        if matches!(row.goal.step, Some(GoalStep::Working | GoalStep::Waiting)) {
                            Ok(Some(with_step(goal, GoalStep::Pausing)))
                        } else {
                            Ok(Some(goal))
                        }
                    }
                    Delivery::NoRunner => {
                        let request = row.control_request.unwrap_or(ControlRequest::Pause);
                        self.write_conclusion(goal_id, request.conclusion())
                            .await
                            .map(Some)
                    }
                    Delivery::Finishing => Ok(None),
                }
            }
            _ => Err(CoreError::GoalFinished {
                goal_id: goal_id.to_string(),
            }),
        }
    }

    async fn try_end(&self, goal_id: &str, reason: GoalEndReason) -> Result<Option<Goal>> {
        let runners = &self.runtime.goal_runners;
        let _transition = runners.transitions.lock().await;
        let row = self.load(goal_id).await?;
        if !row.goal.status.is_live() {
            return Ok(Some(row.goal));
        }
        if row.goal.status == GoalStatus::Running {
            match runners.send(goal_id, RunnerControl::Stop(reason)) {
                Delivery::Delivered => {
                    let goal = self
                        .write_request(goal_id, ControlRequest::Stop(reason))
                        .await?;
                    return Ok(Some(with_step(goal, GoalStep::Stopping)));
                }
                Delivery::Finishing => return Ok(None),
                Delivery::NoRunner => {}
            }
        }
        self.write_conclusion(goal_id, Conclusion::End(reason))
            .await
            .map(Some)
    }

    async fn write_conclusion(&self, goal_id: &str, how: Conclusion) -> Result<Goal> {
        let goal = self.write(goal_id, how.update()).await?;
        self.announce(&goal);
        Ok(goal)
    }

    async fn write_request(&self, goal_id: &str, request: ControlRequest) -> Result<Goal> {
        let update = GoalStateUpdate {
            control_request: FieldOp::Set(request),
            ..Default::default()
        };
        self.write(goal_id, update).await
    }

    async fn write_running(&self, goal_id: &str) -> Result<Goal> {
        let goal = self.write(goal_id, running_again()).await?;
        self.announce(&goal);
        Ok(goal)
    }

    async fn write(&self, goal_id: &str, update: GoalStateUpdate) -> Result<Goal> {
        let (db, id) = (self.runtime.db.clone(), goal_id.to_string());
        blocking(move || db.update_goal_state(&id, update))
            .await?
            .map(|row| row.goal)
            .ok_or_else(|| CoreError::GoalNotFound {
                goal_id: goal_id.to_string(),
            })
    }

    fn announce(&self, goal: &Goal) {
        announce(&self.runtime.events, goal);
    }
}

/// The step a control answers with before the runner writes it.
fn with_step(mut goal: Goal, step: GoalStep) -> Goal {
    goal.step = Some(step);
    goal
}

fn required(value: Option<String>, missing: &str) -> Result<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid(missing))
}

/// Settings' defaults in the harness's unit: dollars on Claude and OpenCode, tokens on Codex.
fn derived_limits(goals: &GoalSettings, harness: Harness) -> GoalLimits {
    let codex = harness == Harness::Codex;
    GoalLimits {
        max_iterations: goals.max_iterations,
        max_cost_usd: (!codex).then_some(goals.max_cost_usd),
        max_tokens: codex.then_some(goals.max_tokens),
        max_active_secs: goals.max_active_secs,
    }
}

/// A plan or repo command before it becomes a check.
struct ProposedCommand {
    command: String,
    label: Option<String>,
    cwd: Option<String>,
    phase: Option<String>,
}

impl ProposedCommand {
    fn bare(command: String) -> Self {
        Self {
            command,
            label: None,
            cwd: None,
            phase: None,
        }
    }
}

/// One Command check per distinct directory and command, in the order given; a repeat adds its phase.
fn command_checks(proposed: impl IntoIterator<Item = ProposedCommand>) -> Vec<GoalCheck> {
    let mut checks: Vec<GoalCheck> = Vec::new();
    for next in proposed {
        if next.command.trim().is_empty() {
            continue;
        }
        let seen = checks.iter_mut().find_map(|check| match check {
            GoalCheck::Command {
                command,
                cwd,
                phases,
                ..
            } if *command == next.command && *cwd == next.cwd => Some(phases),
            _ => None,
        });
        match seen {
            Some(phases) => {
                if let Some(phase) = next.phase.filter(|phase| !phases.contains(phase)) {
                    phases.push(phase);
                }
            }
            None => checks.push(GoalCheck::Command {
                command: next.command,
                timeout_secs: DERIVED_TIMEOUT_SECS,
                label: next.label,
                cwd: next.cwd,
                phases: next.phase.into_iter().collect(),
            }),
        }
    }
    checks
}

async fn read_outline(worktree: &Path, plan_path: &str) -> Result<PlanOutline> {
    let path = worktree.join(plan_path);
    let content = blocking(move || Ok(std::fs::read_to_string(path)?)).await?;
    Ok(plan_outline(&content))
}

fn plan_title(outline: &PlanOutline, plan_path: &str) -> String {
    outline
        .title
        .clone()
        .or_else(|| file_stem(plan_path))
        .unwrap_or_else(|| "the plan".to_string())
}

fn file_stem(path: &str) -> Option<String> {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// The phases a plan goal covers; `ids` is empty for the whole plan.
struct PlanScope<'a> {
    ids: Vec<String>,
    phases: Vec<&'a PlanPhase>,
}

impl<'a> PlanScope<'a> {
    fn of(outline: &'a PlanOutline, ids: &[String]) -> Result<Self> {
        if ids.is_empty() {
            return Ok(Self {
                ids: Vec::new(),
                phases: outline.phases.iter().collect(),
            });
        }
        let phases = ids
            .iter()
            .map(|id| {
                outline
                    .phases
                    .iter()
                    .find(|phase| phase.id.eq_ignore_ascii_case(id.trim()))
                    .ok_or_else(|| invalid(format!("The plan has no phase {id}.")))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            ids: phases.iter().map(|phase| phase.id.clone()).collect(),
            phases,
        })
    }

    fn source(&self) -> GoalSource {
        if self.ids.is_empty() {
            GoalSource::Plan
        } else {
            GoalSource::PlanPhase
        }
    }

    /// `N`, or `N-M` for a range.
    fn label(&self) -> Option<String> {
        match self.ids.as_slice() {
            [] => None,
            [one] => Some(one.clone()),
            [first, .., last] => Some(format!("{first}-{last}")),
        }
    }

    fn instruction(&self) -> String {
        match (self.ids.len(), self.label()) {
            (_, None) => "all remaining phases consecutively".to_string(),
            (1, Some(id)) => format!("phase {id}"),
            (_, Some(range)) => format!("phases {range} consecutively"),
        }
    }

    fn subject(&self) -> String {
        match (self.ids.len(), self.label()) {
            (_, None) => "Every phase".to_string(),
            (1, Some(id)) => format!("Phase {id}"),
            (_, Some(range)) => format!("Phases {}", range.replacen('-', "–", 1)),
        }
    }

    /// The predicate after the subject: a range is plural.
    fn predicate(&self) -> &'static str {
        if self.ids.len() > 1 {
            "are implemented and their"
        } else {
            "is implemented and its"
        }
    }
}

/// A scope's automated criteria: the ones with a fence or ending in a backtick span are commands,
/// the rest prose.
struct Criteria {
    commands: Vec<ProposedCommand>,
    prose: Vec<String>,
}

impl Criteria {
    fn of(phases: &[&PlanPhase]) -> Self {
        let mut criteria = Criteria {
            commands: Vec::new(),
            prose: Vec::new(),
        };
        let automated = phases.iter().flat_map(|phase| {
            phase
                .boxes
                .iter()
                .filter(|b| b.section == BoxSection::Automated)
                .map(|b| (&phase.id, b))
        });
        for (phase, criterion) in automated {
            let Some(command) = criterion.command() else {
                criteria.prose.push(criterion.text.clone());
                continue;
            };
            // A fence leaves the sentence whole; a span is cut out of it.
            let label = match &criterion.block {
                Some(_) => criterion.text.clone(),
                None => criterion
                    .text
                    .rfind(&format!("`{command}`"))
                    .map(|at| {
                        let close = at + command.len() + 2;
                        format!("{}{}", &criterion.text[..at], &criterion.text[close..])
                    })
                    .unwrap_or_default(),
            };
            let label = label.replace('`', "");
            let label = label.trim().trim_end_matches(':').trim();
            criteria.commands.push(ProposedCommand {
                command: command.to_string(),
                label: (!label.is_empty()).then(|| label.to_string()),
                cwd: criterion.dir.clone(),
                phase: Some(phase.clone()),
            });
        }
        criteria
    }

    fn prose_suffix(&self) -> String {
        if self.prose.is_empty() {
            return String::new();
        }
        format!(". Prose criteria: {}", self.prose.join("; "))
    }
}

/// Frontmatter `topic`, else the first paragraph under `## Research Question` or `## Question`, else the title.
fn research_question(content: &str) -> Option<String> {
    let parsed = frontmatter::parse(content);
    let topic = parsed.fm.extra.get("topic").map(|t| t.trim().to_string());
    let body = content.get(parsed.body_offset..).unwrap_or(content);
    topic
        .filter(|t| !t.is_empty())
        .or_else(|| first_paragraph_under(body, "research question"))
        .or_else(|| first_paragraph_under(body, "question"))
        .or_else(|| parsed.fm.title.filter(|t| !t.trim().is_empty()))
        .or_else(|| {
            body.lines()
                .find_map(|line| line.strip_prefix("# "))
                .map(|title| title.trim().to_string())
        })
}

fn first_paragraph_under(body: &str, heading: &str) -> Option<String> {
    let mut lines = body.lines().skip_while(|line| {
        line.strip_prefix("## ")
            .is_none_or(|h| !h.trim().eq_ignore_ascii_case(heading))
    });
    lines.next()?;
    let paragraph: Vec<&str> = lines
        .map(str::trim)
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    (!paragraph.is_empty()).then(|| paragraph.join(" "))
}
