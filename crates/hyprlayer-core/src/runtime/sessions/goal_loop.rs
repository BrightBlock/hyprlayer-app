//! The goal driver: a watcher that reads one iteration out of a session's
//! events, and the runner that sends iterations, runs checks and decides what follows.

use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, Notify};
use tokio::time::Instant;
use xxhash_rust::xxh3::Xxh3;

use hyprlayer_models::goal::{
    Goal, GoalCheckOutcome, GoalCheckResult, GoalEndReason, GoalIterationNote, GoalPauseReason,
    GoalSpec, GoalStatus, GoalStep,
};
use hyprlayer_models::RuntimeEvent;

use crate::agent::{AgentDriver, Harness, UsageSoFar};
use crate::claude::{ExitOutcome, PermissionMode, SessionEvent, TurnStatus};
use crate::error::Result;
use crate::pipeline::{goal_iteration_prompt, implement_notes_suffix, GOAL_PROMPT_NAME};
use crate::runtime::goals::checks::{run_checks, CheckContext, CheckRun};
use crate::runtime::goals::decide::{
    decide_boundary, remaining_active, spend_limit, spend_reached, stall_count_after,
    token_cap_reached, usage_delta, Boundary, BoundaryInput, StallKey,
};
use crate::runtime::goals::{announce, Conclusion, Lease, RunnerControl};
use crate::runtime::HyprlayerRuntime;
use crate::store::{
    ControlRequest, Db, FieldOp, GoalRow, GoalStateUpdate, IterationEnd as ClosedIteration,
    Session, Workstream,
};

use super::delivery::{LiveReceiver, SessionStreamError};
use super::service::{AppTurn, OpGuard};
use super::{LaunchSessionParams, SessionActors, SessionService};

/// Quiet time after a turn ends before the iteration counts as over.
pub(crate) const QUIET: Duration = Duration::from_secs(2);
/// How long a harness gets to end its turn after an interrupt before it is killed.
const KILL_GRACE: Duration = Duration::from_secs(10);
/// How long a relaunched codex gets to re-report its thread total.
const RESTORE_WAIT: Duration = Duration::from_secs(2);
const RESTORE_POLL: Duration = Duration::from_millis(50);
/// Crashes before an iteration finishes that pause the goal instead of relaunching.
const HARNESS_CRASH_LIMIT: u32 = 2;
const QUESTION_TOOL: &str = "AskUserQuestion";
/// The active-time cap of a clock stopped on an ask: never, in practice.
const STOPPED_CAP: Duration = Duration::from_secs(365 * 24 * 3600);

/// How a runner picks the goal up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartMode {
    /// A start, or a resume by a client.
    Normal,
    /// After a server restart; `step` is the one the goal was on when it went down.
    Resume { step: Option<GoalStep> },
}

/// Reports whether the session's actor is between turns with no send waiting to land.
type Idle<'a> = &'a (dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Sync);

pub(crate) fn idle_probe(
    actors: Arc<SessionActors>,
    session_id: String,
) -> impl Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Sync {
    move || {
        let (actors, session_id) = (actors.clone(), session_id.clone());
        Box::pin(async move { is_idle(&actors, &session_id).await })
    }
}

/// A session's events as the runner reads them, across lag and harness restarts.
pub struct Watch {
    actors: Arc<SessionActors>,
    session_id: String,
    live: LiveReceiver,
    replay: VecDeque<crate::store::SessionLogEntry>,
    last_seq: Option<i64>,
    lagged: Option<u64>,
}

impl Watch {
    /// Reads from now on: the history the attachment replays is not an iteration's.
    pub(crate) async fn attach(actors: Arc<SessionActors>, session_id: &str) -> Result<Watch> {
        let attachment = actors.attach(session_id, true).await?;
        Ok(Watch {
            session_id: session_id.to_string(),
            live: attachment.live,
            replay: VecDeque::new(),
            last_seq: attachment.replay.last().map(|entry| entry.seq),
            lagged: None,
            actors,
        })
    }

    /// `None` once the session's event source is gone. Safe to drop mid-wait: a lag is recovered on the next call.
    pub(crate) async fn next(&mut self) -> Option<SessionEvent> {
        loop {
            if let Some(entry) = self.replay.pop_front() {
                self.last_seq = Some(entry.seq);
                return Some(entry.event);
            }
            if let Some(epoch) = self.lagged {
                let resumed = self
                    .actors
                    .attach_after(&self.session_id, epoch, self.last_seq, true)
                    .await
                    .ok()?;
                self.live = resumed.live;
                self.replay = resumed.replay.into();
                self.lagged = None;
                continue;
            }
            match self.live.recv_item().await {
                Some(Ok(item)) => {
                    if let Some(seq) = item.seq {
                        self.last_seq = Some(seq);
                    }
                    return Some(item.event);
                }
                Some(Err(SessionStreamError::Lagged { epoch, .. })) => self.lagged = Some(epoch),
                None => return None,
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IterationEnd {
    pub last_text: Option<String>,
    pub turn_status: Option<String>,
    /// The harness's cumulative cost at the last turn end.
    pub cost_usd: Option<f64>,
    /// Codex's thread total at the last usage report.
    pub thread_tokens: Option<u32>,
    pub steered: bool,
    pub exited: Option<ExitOutcome>,
}

/// Codex's mid-turn cap: the goal's spend before the iteration, the thread total at its send, and the cap.
#[derive(Debug, Clone, Copy)]
struct TokenCap {
    spent: u32,
    baseline: u32,
    max: u32,
}

/// One iteration's state, kept apart from the future that advances it so a caller can stop polling and come back.
pub(crate) struct IterationWatch {
    /// `None` waits out a turn that is not ours.
    nonce: Option<String>,
    quiet: Duration,
    echoed: bool,
    deadline: Option<Instant>,
    token_cap: Option<TokenCap>,
    /// A usage report reached `token_cap`; the iteration is cut short.
    capped: bool,
    /// Our turn is parked on a permission or question ask.
    waiting: bool,
    /// The subagent call the pending ask came from; `None` is the top level.
    ask_scope: Option<String>,
    /// Tool calls with no result yet: id, name, and the subagent call they run under.
    open_tools: Vec<(String, String, Option<String>)>,
    change: Option<Progress>,
    end: IterationEnd,
}

/// Why `IterationWatch::finish` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Progress {
    Ended,
    /// Our turn asks a person; carries the tool or the question.
    Asked(String),
    Answered,
}

impl IterationWatch {
    pub(crate) fn new(nonce: &str, quiet: Duration) -> Self {
        Self {
            nonce: Some(nonce.to_string()),
            quiet,
            echoed: false,
            deadline: None,
            token_cap: None,
            capped: false,
            waiting: false,
            ask_scope: None,
            open_tools: Vec::new(),
            change: None,
            end: IterationEnd::default(),
        }
    }

    pub(crate) fn foreign(quiet: Duration) -> Self {
        Self {
            nonce: None,
            quiet,
            echoed: true,
            deadline: Some(Instant::now() + quiet),
            token_cap: None,
            capped: false,
            waiting: false,
            ask_scope: None,
            open_tools: Vec::new(),
            change: None,
            end: IterationEnd::default(),
        }
    }

    pub(crate) fn into_end(self) -> IterationEnd {
        self.end
    }

    pub(crate) fn partial(&self) -> IterationEnd {
        self.end.clone()
    }

    fn activity(&mut self) {
        // Our own iteration waits for its next `TurnDone`; someone else's turn only for quiet.
        self.deadline = self.nonce.is_none().then(|| Instant::now() + self.quiet);
    }

    fn turn_item(&mut self) {
        self.activity();
        self.answered();
    }

    fn asked(&mut self, what: String) {
        if self.nonce.is_some() && !self.waiting {
            self.waiting = true;
            self.change = Some(Progress::Asked(what));
        }
    }

    fn answered(&mut self) {
        if self.waiting {
            self.waiting = false;
            self.change = Some(Progress::Answered);
        }
    }

    /// A parallel subagent's items say nothing about an ask made elsewhere.
    fn item_from(&mut self, parent_id: Option<&str>) {
        self.activity();
        if parent_id == self.ask_scope.as_deref() {
            self.answered();
        }
    }

    /// The subagent call behind an ask, read off the newest open call of the asking tool.
    fn scope_of(&self, tool_name: &str) -> Option<String> {
        self.open_tools
            .iter()
            .rev()
            .find(|(_, name, _)| name == tool_name)
            .and_then(|(.., parent)| parent.clone())
    }

    fn apply(&mut self, event: SessionEvent) {
        if let SessionEvent::UserMessage {
            client_message_id, ..
        } = &event
        {
            let ours = self.nonce.is_some() && client_message_id == &self.nonce;
            if !self.echoed {
                self.echoed = ours;
            } else if !ours {
                self.end.steered |= self.nonce.is_some();
                self.turn_item();
            }
            return;
        }
        if !self.echoed {
            return;
        }
        match event {
            SessionEvent::AssistantText {
                text,
                parent_id,
                injected,
                ..
            } => {
                if parent_id.is_none() && !injected {
                    self.end.last_text = Some(text);
                }
                self.item_from(parent_id.as_deref());
            }
            // A parallel call streams its next `tool_use` while an ask is still pending.
            SessionEvent::ToolUse {
                id,
                name,
                parent_id,
                ..
            } => {
                self.open_tools.push((id, name, parent_id));
                self.activity();
            }
            // Asking tools run serially, so a result after an ask means it was resolved.
            SessionEvent::ToolResult {
                tool_use_id,
                parent_id,
                ..
            } => {
                self.open_tools.retain(|(id, ..)| *id != tool_use_id);
                self.item_from(parent_id.as_deref());
            }
            SessionEvent::PermissionResolved { .. } | SessionEvent::QuestionAnswered { .. } => {
                self.answered();
            }
            SessionEvent::PermissionAsk { tool_name, .. } => {
                self.turn_item();
                self.ask_scope = self.scope_of(&tool_name);
                self.asked(tool_name);
            }
            SessionEvent::QuestionAsk { questions, .. } => {
                self.turn_item();
                self.ask_scope = self.scope_of(QUESTION_TOOL);
                let question = questions.into_iter().next().map(|q| q.question);
                self.asked(question.unwrap_or_default());
            }
            SessionEvent::Usage {
                thread_tokens: Some(total),
                ..
            } => {
                self.end.thread_tokens = Some(total);
                self.capped |= self
                    .token_cap
                    .is_some_and(|cap| token_cap_reached(cap.spent, cap.baseline, total, cap.max));
            }
            SessionEvent::TurnDone {
                cost_usd,
                turn_status,
                thread_tokens,
                ..
            } => {
                if turn_status.is_some() {
                    self.end.turn_status = turn_status;
                }
                if cost_usd.is_some() {
                    self.end.cost_usd = cost_usd;
                }
                if thread_tokens.is_some() {
                    self.end.thread_tokens = thread_tokens;
                }
                self.deadline = Some(Instant::now() + self.quiet);
                self.answered();
            }
            SessionEvent::Exited { outcome, .. } => self.end.exited = Some(outcome),
            _ => {}
        }
    }

    /// Returns once the iteration is over (the harness left, a usage report reached the token cap, or the quiet window closed on an idle actor), or when our turn starts or stops waiting on a person.
    pub(crate) async fn finish(&mut self, watch: &mut Watch, idle: Idle<'_>) -> Progress {
        while self.end.exited.is_none() && !self.capped {
            if let Some(change) = self.change.take() {
                return change;
            }
            let deadline = self.deadline;
            tokio::select! {
                biased;
                event = watch.next() => match event {
                    Some(event) => self.apply(event),
                    None => self.end.exited = Some(ExitOutcome::Detached),
                },
                () = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    if idle().await {
                        return Progress::Ended;
                    }
                    self.deadline = Some(Instant::now() + self.quiet);
                }
            }
        }
        Progress::Ended
    }
}

/// Ends after our echo, a `TurnDone` and `quiet` with no turn item while the actor is idle.
#[cfg(any(test, feature = "test-support"))]
pub async fn watch_iteration(
    watch: &mut Watch,
    nonce: &str,
    quiet: Duration,
    idle: Idle<'_>,
) -> IterationEnd {
    let mut state = IterationWatch::new(nonce, quiet);
    while state.finish(watch, idle).await != Progress::Ended {}
    state.into_end()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Escalation {
    Ended {
        turn_status: Option<String>,
        cost_usd: Option<f64>,
        thread_tokens: Option<u32>,
    },
    Exited,
    Killed,
}

/// Interrupts at once and kills if no `TurnDone` arrives within `grace`; returns without waiting for the exit.
pub async fn interrupt_then_kill(
    driver: &dyn AgentDriver,
    watch: &mut Watch,
    grace: Duration,
) -> Escalation {
    if let Err(e) = driver.interrupt() {
        tracing::warn!("the interrupt was not delivered: {e}");
    }
    let deadline = Instant::now() + grace;
    loop {
        tokio::select! {
            biased;
            event = watch.next() => match event {
                Some(SessionEvent::TurnDone { turn_status, cost_usd, thread_tokens, .. }) => {
                    return Escalation::Ended { turn_status, cost_usd, thread_tokens };
                }
                Some(SessionEvent::Exited { .. }) | None => return Escalation::Exited,
                Some(_) => {}
            },
            () = tokio::time::sleep_until(deadline) => {
                driver.kill();
                return Escalation::Killed;
            }
        }
    }
}

/// The run is over: its last state is written, the runtime is gone, or it was stopped for shutdown.
struct Over;
type Step<T> = std::result::Result<T, Over>;

enum Signal {
    Control(RunnerControl),
    Cap,
    Stopped,
}

enum Reaction {
    Continue,
    Abort(Conclusion),
    Quit,
}

/// Waits for a control, the active-time cap or the runtime's stop; a closed control channel never fires.
async fn next_signal(
    control: &mut mpsc::UnboundedReceiver<RunnerControl>,
    cap_at: Instant,
    stop: &Notify,
) -> Signal {
    tokio::select! {
        biased;
        () = stop.notified() => Signal::Stopped,
        () = tokio::time::sleep_until(cap_at) => Signal::Cap,
        received = control.recv() => match received {
            Some(control) => Signal::Control(control),
            None => std::future::pending().await,
        },
    }
}

#[derive(Default)]
struct Pending {
    pause: bool,
    interrupted: bool,
    reload: bool,
}

/// What the last finished check said, for the stall rule.
struct Verdict {
    progress_hash: Option<String>,
    outcome: GoalCheckOutcome,
    reason: Option<String>,
}

impl Verdict {
    fn key(&self) -> StallKey<'_> {
        StallKey {
            progress_hash: self.progress_hash.as_deref(),
            outcome: self.outcome,
            reason: self.reason.as_deref(),
        }
    }
}

/// Active time: `base_ms` plus the run since `since`; `None` while a person is asked.
struct Clock {
    base_ms: u32,
    since: Option<Instant>,
}

impl Clock {
    fn active_ms(&self) -> u32 {
        let elapsed = self.since.map_or(0, |since| {
            u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX)
        });
        self.base_ms.saturating_add(elapsed)
    }

    fn stop(&mut self) {
        self.base_ms = self.active_ms();
        self.since = None;
    }

    fn start(&mut self) {
        self.since.get_or_insert_with(Instant::now);
    }

    fn cap_at(&self, max_active_secs: u32) -> Instant {
        match self.since {
            Some(since) => since + remaining_active(max_active_secs, self.base_ms),
            None => Instant::now() + STOPPED_CAP,
        }
    }
}

struct Sent {
    n: u32,
    nonce: String,
}

/// What the boundary found once it held the session's op lock.
enum Gate {
    Sent(Sent),
    Busy,
    Pause,
    End(GoalEndReason),
    RelaunchFailed(String),
}

struct Run {
    runtime: Weak<HyprlayerRuntime>,
    db: Arc<Db>,
    actors: Arc<SessionActors>,
    events: broadcast::Sender<RuntimeEvent>,
    lease: Lease,
    control: mpsc::UnboundedReceiver<RunnerControl>,
    stop: Arc<Notify>,
    row: GoalRow,
    session: Session,
    workstream: Workstream,
    watch: Watch,
    clock: Clock,
    cap_at: Instant,
    pending: Pending,
    /// An iteration's turn is open.
    in_flight: bool,
    feedback: Vec<GoalCheckResult>,
    verdict: Option<Verdict>,
    /// The last iteration's closing text, for the evaluator; a recheck reuses it.
    last_text: Option<String>,
    cut_off: bool,
    /// The iteration a previous run left open.
    dangling: Option<u32>,
    /// The claude harness this run launched with the goal's budget cap.
    owned: Option<Weak<dyn AgentDriver>>,
    /// The session's last known mode; the registry forgets it once the harness exits.
    mode: Option<PermissionMode>,
    concluded: Option<Conclusion>,
}

/// Drives one goal until it pauses or ends. It is the only writer of the goal's state columns while it runs.
pub(crate) async fn run(
    runtime: Weak<HyprlayerRuntime>,
    goal_id: String,
    control: mpsc::UnboundedReceiver<RunnerControl>,
    stop: Arc<Notify>,
    lease: Lease,
    owned: Option<Weak<dyn AgentDriver>>,
    mode: StartMode,
) {
    let Ok(mut run) = Run::open(runtime, goal_id, control, stop, lease, owned).await else {
        return;
    };
    let started = match (run.row.control_request, mode) {
        (Some(request), _) => {
            let _ = run.close_cut_off(None).await;
            Err(run.conclude(request.conclusion()).await)
        }
        (None, StartMode::Normal) => {
            // `GOAL_CUT_OFF` names a server restart; a crash's cut-off note is not one.
            run.cut_off = false;
            run.close_cut_off(None).await
        }
        (None, StartMode::Resume { step }) => run.resume(step).await,
    };
    if started.is_ok() {
        let _ = run.drive().await;
    }
    run.lift_cap().await;
}

impl Run {
    /// A goal that is not `running` has nothing to drive: a resume that lost a race ends here.
    async fn open(
        runtime: Weak<HyprlayerRuntime>,
        goal_id: String,
        control: mpsc::UnboundedReceiver<RunnerControl>,
        stop: Arc<Notify>,
        lease: Lease,
        owned: Option<Weak<dyn AgentDriver>>,
    ) -> Step<Run> {
        let rt = runtime.upgrade().ok_or(Over)?;
        let db = rt.db.clone();
        let actors = rt.session_actors.clone();
        let events = rt.events.clone();

        let row = {
            // A resume writes `running` under this lock and spawns after it, so it either sees us finishing or we see it running.
            let _transition = lease.transition().await;
            let id = goal_id.clone();
            let row = read(&db, move |db| db.goal(&id)).await?.ok_or(Over)?;
            if row.goal.status != GoalStatus::Running {
                lease.finishing();
                return Err(Over);
            }
            row
        };
        let session = rt
            .sessions()
            .get(&row.goal.session_id)
            .await
            .map_err(|e| warn_over(&goal_id, e))?
            .ok_or(Over)?;
        let workstream = rt
            .workstreams()
            .find(&session.workstream_id)
            .await
            .map_err(|e| warn_over(&goal_id, e))?;
        // With no harness live, the mode the session last asked for.
        let mode = rt
            .sessions()
            .permission_mode(&session.id)
            .await
            .or(session.requested_permission_mode);
        drop(rt);
        let watch = Watch::attach(actors.clone(), &session.id)
            .await
            .map_err(|e| warn_over(&goal_id, e))?;

        let id = goal_id.clone();
        let iterations = read(&db, move |db| db.goal_iterations(&id)).await?;
        let last = iterations.last();
        let cut_off =
            last.is_some_and(|i| i.ended_at.is_none() || i.note == Some(GoalIterationNote::CutOff));
        let dangling = last.filter(|i| i.ended_at.is_none()).map(|i| i.n);
        let checked = iterations.iter().rev().find(|i| i.outcome.is_some());
        let feedback = checked.map(|i| i.results.clone()).unwrap_or_default();
        let verdict = checked.and_then(|i| {
            Some(Verdict {
                progress_hash: i.progress_hash.clone(),
                outcome: i.outcome?,
                reason: i.reason.clone(),
            })
        });
        let sent = last.map_or(0, |i| i.n);

        tracing::info!(
            "goal {goal_id}: its run starts after iteration {} of {}",
            row.goal.iteration,
            row.goal.limits.max_iterations
        );
        let clock = Clock {
            base_ms: row.goal.active_ms,
            since: Some(Instant::now()),
        };
        let cap_at = clock.cap_at(row.goal.limits.max_active_secs);
        let mut run = Run {
            runtime,
            db,
            actors,
            events,
            lease,
            control,
            stop,
            row,
            session,
            workstream,
            watch,
            clock,
            cap_at,
            pending: Pending::default(),
            in_flight: false,
            feedback,
            verdict,
            last_text: None,
            cut_off,
            dangling,
            owned,
            mode,
            concluded: None,
        };
        // A crash between inserting an iteration and counting it must not make the next one collide.
        if sent > run.row.goal.iteration {
            run.write(GoalStateUpdate {
                iteration: FieldOp::Set(sent),
                ..Default::default()
            })
            .await?;
        }
        Ok(run)
    }

    /// Relaunches first, then picks up from the step the goal went down on.
    async fn resume(&mut self, step: Option<GoalStep>) -> Step<()> {
        self.relaunch_now("The agent did not relaunch after a server restart")
            .await?;
        let restored = self.restored_tokens().await;
        self.close_cut_off(restored).await?;
        match step {
            Some(GoalStep::Checking) if self.row.goal.iteration > 0 => {
                let n = self.row.goal.iteration;
                let id = self.row.goal.id.clone();
                let iterations = self.db(move |db| db.goal_iterations(&id)).await?;
                let end = IterationEnd {
                    turn_status: iterations
                        .iter()
                        .find(|i| i.n == n)
                        .and_then(|i| i.turn_status.clone()),
                    ..IterationEnd::default()
                };
                self.judge(n, &end).await
            }
            Some(GoalStep::Pausing) => Err(self
                .conclude(Conclusion::Pause(GoalPauseReason::User, None))
                .await),
            Some(GoalStep::Stopping) => Err(self
                .conclude(Conclusion::End(GoalEndReason::StoppedByUser))
                .await),
            _ => Ok(()),
        }
    }

    /// Relaunches the gone harness under the session's op lock; a failure pauses the goal, saying `why`.
    async fn relaunch_now(&mut self, why: &str) -> Step<()> {
        let relaunched = {
            let rt = self.runtime()?;
            let sessions = rt.sessions();
            let op = sessions.op_lock(&self.session.id).await;
            let guard = op.lock().await;
            self.relaunch(&sessions, &guard, true).await?
        };
        match relaunched {
            Ok(()) => Ok(()),
            Err(e) => Err(self
                .conclude(Conclusion::Pause(
                    GoalPauseReason::RelaunchFailed,
                    Some(format!("{why}: {e}")),
                ))
                .await),
        }
    }

    /// `thread/resume` re-reports codex's thread total shortly after a relaunch; other harnesses have none.
    async fn restored_tokens(&self) -> Option<u32> {
        if self.session.harness != Harness::Codex {
            return None;
        }
        let deadline = Instant::now() + RESTORE_WAIT;
        loop {
            let reading = {
                let rt = self.runtime().ok()?;
                let driver = rt.sessions().live_driver(&self.session.id).await;
                driver.and_then(|driver| driver.usage_so_far().thread_tokens)
            };
            if reading.is_some() || Instant::now() >= deadline {
                return reading;
            }
            tokio::time::sleep(RESTORE_POLL).await;
        }
    }

    /// Closes the iteration a previous run left open. Its cost is unknown; codex's `restored` total prices its tokens.
    async fn close_cut_off(&mut self, restored: Option<u32>) -> Step<()> {
        let Some(n) = self.dangling.take() else {
            return Ok(());
        };
        let (tokens, baseline) = usage_delta(self.row.token_baseline.unwrap_or(0), restored);
        let closed = ClosedIteration {
            turn_status: None,
            cost_usd: None,
            tokens: restored.map(|_| tokens),
            note: Some(GoalIterationNote::CutOff),
        };
        let id = self.row.goal.id.clone();
        self.db(move |db| db.finish_iteration(&id, n, &closed))
            .await?;
        if restored.is_none() {
            return Ok(());
        }
        self.write(GoalStateUpdate {
            spent_tokens: FieldOp::Set(self.row.goal.spent_tokens.saturating_add(tokens)),
            token_baseline: FieldOp::Set(baseline),
            ..Default::default()
        })
        .await
    }

    async fn drive(&mut self) -> Step<()> {
        loop {
            let sent = self.boundary().await?;
            let end = self.working(&sent).await?;
            self.after(&sent, end).await?;
        }
    }

    fn runtime(&self) -> Step<Arc<HyprlayerRuntime>> {
        self.runtime.upgrade().ok_or(Over)
    }

    async fn boundary(&mut self) -> Step<Sent> {
        loop {
            self.drain_controls().await?;
            if self.pending.reload {
                self.reload_spec().await?;
            }
            let gate = self.gate().await?;
            match gate {
                Gate::Sent(sent) => return Ok(sent),
                Gate::Pause => {
                    return Err(self
                        .conclude(Conclusion::Pause(GoalPauseReason::User, None))
                        .await)
                }
                Gate::End(reason) => return Err(self.conclude(Conclusion::End(reason)).await),
                Gate::RelaunchFailed(detail) => {
                    return Err(self
                        .conclude(Conclusion::Pause(
                            GoalPauseReason::RelaunchFailed,
                            Some(detail),
                        ))
                        .await)
                }
                Gate::Busy => {
                    self.wait_idle().await?;
                    if self.row.goal.iteration > 0 {
                        self.recheck().await?;
                    }
                }
            }
        }
    }

    /// Under the session's op lock: relaunch a gone harness, then send if the session is idle.
    async fn gate(&mut self) -> Step<Gate> {
        // Nothing was sent, so there is no turn to wait for.
        if self.pending.pause && self.row.goal.iteration == 0 {
            return Ok(Gate::Pause);
        }
        // A cap lowered below the spend while paused; no turn or capped launch may follow.
        if spend_reached(&self.row.goal, self.session.harness) {
            return Ok(Gate::End(spend_limit(self.session.harness)));
        }
        // A restart that cut off the last iteration reaches here without a judge.
        if self.row.goal.iteration >= self.row.goal.limits.max_iterations {
            return Ok(Gate::End(GoalEndReason::IterationLimit));
        }
        let rt = self.runtime()?;
        let sessions = rt.sessions();
        let session_id = self.session.id.clone();
        let op = sessions.op_lock(&session_id).await;
        let guard = op.lock().await;
        if !sessions.is_live(&session_id).await {
            if self.pending.pause {
                return Ok(Gate::Pause);
            }
            if let Err(e) = self.relaunch(&sessions, &guard, true).await? {
                return Ok(Gate::RelaunchFailed(e.to_string()));
            }
        }
        if !is_idle(&self.actors, &session_id).await {
            return Ok(Gate::Busy);
        }
        if self.pending.pause {
            return Ok(Gate::Pause);
        }
        // Only on an idle session: the swap's exit would read as a client's restart while the run waits.
        if self.carries_foreign_cap(&sessions).await {
            self.remember_mode(&sessions).await;
            sessions.kill_for_relaunch(&session_id).await;
            if let Err(e) = self.relaunch(&sessions, &guard, true).await? {
                sessions.end_kept_streams(&session_id).await;
                return Ok(Gate::RelaunchFailed(e.to_string()));
            }
        }
        self.send(&sessions, &guard).await.map(Gate::Sent)
    }

    /// On claude the harness gets the goal's remaining budget as its cap, which this run then owns.
    async fn relaunch(
        &mut self,
        sessions: &SessionService<'_>,
        guard: &OpGuard<'_>,
        boundary_on_relaunch: bool,
    ) -> Step<Result<()>> {
        let Some(session) = sessions.get(&self.session.id).await.ok().flatten() else {
            return Err(Over);
        };
        let goal = &self.row.goal;
        let cap = goal
            .limits
            .max_cost_usd
            .filter(|_| session.harness == Harness::Claude)
            .map(|max| max - goal.spent_cost_usd)
            // Spent already: the boundary ends the goal before any send, so no cap is needed.
            .filter(|remaining| *remaining > 0.0);
        let params = LaunchSessionParams {
            session,
            project_path: self.workstream.project_path.clone(),
            model: None,
            effort: None,
            permission_mode: self.mode,
            resume: true,
            boundary_on_relaunch,
            max_budget_usd: cap,
            extra_allowed_tools: Vec::new(),
            extra_disallowed_tools: Vec::new(),
            report_only: false,
        };
        let launched = match sessions.launch_if_exited_held(params, guard).await {
            Ok(launched) => launched,
            Err(e) => return Ok(Err(e)),
        };
        if let Some(driver) = launched.filter(|_| cap.is_some()) {
            self.owned = Some(Arc::downgrade(&driver));
        }
        Ok(Ok(()))
    }

    /// Another goal's cap, left on the harness while that goal's lift waited for the session to go idle.
    async fn carries_foreign_cap(&self, sessions: &SessionService<'_>) -> bool {
        let Some(live) = sessions.live_driver(&self.session.id).await else {
            return false;
        };
        let ours = self
            .owned
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|owned| Arc::ptr_eq(&owned, &live));
        live.budget_cap().is_some() && !ours
    }

    async fn remember_mode(&mut self, sessions: &SessionService<'_>) {
        if let Some(mode) = sessions.permission_mode(&self.session.id).await {
            self.mode = Some(mode);
        }
    }

    async fn send(&mut self, sessions: &SessionService<'_>, guard: &OpGuard<'_>) -> Step<Sent> {
        self.remember_mode(sessions).await;
        let goal_id = self.row.goal.id.clone();
        let n = self.row.goal.iteration + 1;
        let nonce = uuid::Uuid::new_v4().to_string();
        let usage = sessions
            .live_driver(&self.session.id)
            .await
            .map(|driver| driver.usage_so_far())
            .unwrap_or_default();
        let (id, message_id) = (goal_id.clone(), nonce.clone());
        self.db(move |db| db.insert_iteration(&id, n, &message_id))
            .await?;

        let notes = (n == 1)
            .then(|| {
                implement_notes_suffix(
                    self.session.harness,
                    self.session.stage,
                    &self.session.workstream_id,
                )
            })
            .flatten();
        let text = goal_iteration_prompt(
            &spec_of(&self.row.goal),
            n,
            &self.feedback,
            self.cut_off,
            notes.as_deref(),
        );
        let sent = sessions
            .send_app_turn_held(
                &self.session.id,
                AppTurn {
                    text: &text,
                    prompt_name: Some(GOAL_PROMPT_NAME),
                    author: self.row.goal.author.clone(),
                    nonce: nonce.clone(),
                    collab_mode: None,
                },
                guard,
            )
            .await;
        if let Err(e) = sent {
            let id = goal_id;
            self.db(move |db| db.delete_iteration(&id, n)).await?;
            let detail = format!("The harness did not take iteration {n}: {e}");
            return Err(self
                .conclude(Conclusion::Pause(
                    GoalPauseReason::HarnessCrashes,
                    Some(detail),
                ))
                .await);
        }
        self.cut_off = false;
        self.write(GoalStateUpdate {
            iteration: FieldOp::Set(n),
            step: FieldOp::Set(GoalStep::Working),
            cost_baseline_usd: FieldOp::Set(usage.cost_usd.unwrap_or(0.0)),
            // `thread/resume` restores the thread total, possibly after this send, so a relaunch keeps the old baseline.
            token_baseline: usage.thread_tokens.map(FieldOp::Set).unwrap_or_default(),
            ..Default::default()
        })
        .await?;
        Ok(Sent { n, nonce })
    }

    async fn working(&mut self, sent: &Sent) -> Step<IterationEnd> {
        self.in_flight = true;
        let mut state = IterationWatch::new(&sent.nonce, QUIET);
        state.token_cap = self.token_cap();
        let idle = self.idle_probe();
        loop {
            tokio::select! {
                biased;
                signal = next_signal(&mut self.control, self.cap_at, &self.stop) => {
                    match self.react(signal).await? {
                        Reaction::Continue => {}
                        Reaction::Abort(how) => {
                            return Err(self.abort_working(sent, state.partial(), how).await);
                        }
                        Reaction::Quit => return Err(Over),
                    }
                }
                progress = state.finish(&mut self.watch, &idle) => match progress {
                    Progress::Ended => break,
                    Progress::Asked(what) => self.wait_on(what).await?,
                    Progress::Answered => self.back_to(GoalStep::Working).await?,
                },
            }
        }
        // An iteration that ended mid-ask (a crash, a token cap) leaves the clock to run again.
        self.restart_clock();
        if state.capped {
            let how = Conclusion::End(GoalEndReason::TokenLimit);
            return Err(self.abort_working(sent, state.into_end(), how).await);
        }
        self.in_flight = false;
        Ok(state.into_end())
    }

    /// An ask stops the active-time clock; the event names what it waits on, which the row does not keep.
    async fn wait_on(&mut self, what: String) -> Step<()> {
        self.clock.stop();
        self.cap_at = self.clock.cap_at(self.row.goal.limits.max_active_secs);
        // A pause requested before the ask keeps its step.
        let step = matches!(
            self.row.goal.step,
            Some(GoalStep::Working | GoalStep::Checking)
        )
        .then_some(GoalStep::Waiting);
        self.store(GoalStateUpdate {
            step: step.map(FieldOp::Set).unwrap_or_default(),
            ..Default::default()
        })
        .await?;
        let mut goal = self.row.goal.clone();
        goal.last_reason = Some(what);
        announce(&self.events, &goal);
        Ok(())
    }

    /// An answered ask restarts the clock and puts the goal back on `step`.
    async fn back_to(&mut self, step: GoalStep) -> Step<()> {
        self.restart_clock();
        if self.row.goal.step != Some(GoalStep::Waiting) {
            return Ok(());
        }
        self.write(GoalStateUpdate {
            step: FieldOp::Set(step),
            ..Default::default()
        })
        .await
    }

    fn restart_clock(&mut self) {
        self.clock.start();
        self.cap_at = self.clock.cap_at(self.row.goal.limits.max_active_secs);
    }

    fn token_cap(&self) -> Option<TokenCap> {
        let max = self
            .row
            .goal
            .limits
            .max_tokens
            .filter(|_| self.session.harness == Harness::Codex)?;
        Some(TokenCap {
            spent: self.row.goal.spent_tokens,
            baseline: self.row.token_baseline.unwrap_or(0),
            max,
        })
    }

    /// A stop or a cap during a turn interrupts it, kills after the grace, and ends the run.
    async fn abort_working(&mut self, sent: &Sent, mut end: IterationEnd, how: Conclusion) -> Over {
        self.in_flight = false;
        let ours = matches!(
            how,
            Conclusion::End(
                GoalEndReason::StoppedByUser | GoalEndReason::TimeLimit | GoalEndReason::TokenLimit
            )
        );
        if ours && !is_idle(&self.actors, &self.session.id).await {
            let driver = match self.runtime() {
                Ok(rt) => rt.sessions().live_driver(&self.session.id).await,
                Err(over) => return over,
            };
            if let Some(driver) = driver {
                let escalation = tokio::select! {
                    escalation = interrupt_then_kill(&*driver, &mut self.watch, KILL_GRACE) => escalation,
                    () = self.stop.notified() => return Over,
                };
                if let Escalation::Ended {
                    turn_status,
                    cost_usd,
                    thread_tokens,
                } = escalation
                {
                    end.turn_status = turn_status.or(end.turn_status);
                    end.cost_usd = cost_usd.or(end.cost_usd);
                    end.thread_tokens = thread_tokens.or(end.thread_tokens);
                }
            }
        }
        let _ = self
            .close_iteration(sent.n, &end, Some(GoalIterationNote::Interrupted), None)
            .await;
        self.conclude(how).await
    }

    async fn after(&mut self, sent: &Sent, end: IterationEnd) -> Step<()> {
        let exited = end.exited;
        let note = match exited {
            Some(ExitOutcome::Detached) => {
                // A client restarting or rewinding the session takes it back.
                self.pending.pause = true;
                Some(GoalIterationNote::Interrupted)
            }
            Some(ExitOutcome::Killed) => Some(GoalIterationNote::Interrupted),
            Some(_) => Some(GoalIterationNote::CutOff),
            None if self.pending.interrupted => Some(GoalIterationNote::Interrupted),
            None if end.steered => Some(GoalIterationNote::Steered),
            None => None,
        };
        self.pending.interrupted = false;
        let proceeds = matches!(exited, None | Some(ExitOutcome::Detached));
        let step = proceeds.then_some(GoalStep::Checking);
        self.close_iteration(sent.n, &end, note, step).await?;
        match exited.map(exit_kind) {
            None | Some(Exit::TakenBack) => {}
            Some(Exit::Ended(how)) => return Err(self.conclude(how).await),
            // Not checked: the next iteration follows the relaunch.
            Some(Exit::Crashed(detail)) => return self.recover(detail).await,
        }
        self.judge(sent.n, &end).await
    }

    /// Relaunches after a crash mid-iteration. No cut-off line: `GOAL_CUT_OFF` names a server restart.
    async fn recover(&mut self, detail: String) -> Step<()> {
        // A limit outranks the crash, as at any boundary, and must end the goal before the relaunch.
        let limits = &self.row.goal.limits;
        if self.row.goal.iteration >= limits.max_iterations {
            return Err(self
                .conclude(Conclusion::End(GoalEndReason::IterationLimit))
                .await);
        }
        if spend_reached(&self.row.goal, self.session.harness) {
            let reason = spend_limit(self.session.harness);
            return Err(self.conclude(Conclusion::End(reason)).await);
        }
        if remaining_active(self.row.goal.limits.max_active_secs, self.clock.active_ms()).is_zero()
        {
            return Err(self
                .conclude(Conclusion::End(GoalEndReason::TimeLimit))
                .await);
        }
        if let Some(how) = self.count_crash(detail).await? {
            return Err(self.conclude(how).await);
        }
        // The boundary pauses on the gone harness without relaunching it.
        if self.pending.pause {
            return Ok(());
        }
        self.write(GoalStateUpdate {
            step: FieldOp::Set(GoalStep::Resuming),
            ..Default::default()
        })
        .await?;
        self.relaunch_now("The agent did not relaunch after it stopped unexpectedly")
            .await?;
        let Some(restored) = self.restored_tokens().await else {
            return Ok(());
        };
        let (tokens, baseline) = usage_delta(self.row.token_baseline.unwrap_or(0), Some(restored));
        self.write(GoalStateUpdate {
            spent_tokens: FieldOp::Set(self.row.goal.spent_tokens.saturating_add(tokens)),
            token_baseline: FieldOp::Set(baseline),
            ..Default::default()
        })
        .await
    }

    /// `Some` once crashes reach the limit with no finished iteration between them.
    async fn count_crash(&mut self, detail: String) -> Step<Option<Conclusion>> {
        let crashes = self.row.harness_crashes.saturating_add(1);
        if crashes >= HARNESS_CRASH_LIMIT {
            return Ok(Some(Conclusion::Pause(
                GoalPauseReason::HarnessCrashes,
                Some(detail),
            )));
        }
        self.write(GoalStateUpdate {
            harness_crashes: FieldOp::Set(crashes),
            ..Default::default()
        })
        .await?;
        Ok(None)
    }

    /// Checks iteration `n` and decides what follows it.
    async fn judge(&mut self, n: u32, end: &IterationEnd) -> Step<()> {
        let hash = {
            let worktree = self.workstream.worktree_path.clone();
            tokio::task::spawn_blocking(move || progress_hash(Path::new(&worktree)))
                .await
                .ok()
                .flatten()
        };
        self.last_text = end.last_text.clone();
        let run = self.check().await?;

        let outcome = run.outcome;
        let key = StallKey {
            progress_hash: hash.as_deref(),
            outcome,
            reason: run.reason.as_deref(),
        };
        let previous = self.verdict.as_ref().map(Verdict::key);
        let stall_count = stall_count_after(previous.as_ref(), &key, self.row.stall_count);
        let failed = end.turn_status.as_deref().map(TurnStatus::parse) == Some(TurnStatus::Failed);
        let turn_failures = if failed {
            self.row.turn_failures.saturating_add(1)
        } else {
            0
        };
        let check_errors = if outcome == GoalCheckOutcome::Error {
            self.row.check_errors.saturating_add(1)
        } else {
            0
        };
        // A finished iteration ends a crash or restart streak.
        let counters = GoalStateUpdate {
            stall_count: FieldOp::Set(stall_count),
            check_errors: FieldOp::Set(check_errors),
            turn_failures: FieldOp::Set(turn_failures),
            harness_crashes: FieldOp::Set(0),
            restart_resumes: FieldOp::Set(0),
            ..Default::default()
        };
        self.record(n, &run, hash.as_deref(), counters).await?;
        self.verdict = Some(Verdict {
            progress_hash: hash,
            outcome,
            reason: run.reason.clone(),
        });
        self.feedback = run.results;

        if self.pending.reload {
            self.reload_spec().await?;
        }
        let harness = self.session.harness;
        let decision = decide_boundary(&BoundaryInput {
            outcome,
            iteration: n,
            max_iterations: self.row.goal.limits.max_iterations,
            harness,
            spend_reached: spend_reached(&self.row.goal, harness),
            budget_tripped: end.turn_status.as_deref().map(TurnStatus::parse)
                == Some(TurnStatus::BudgetExhausted),
            check_errors: self.row.check_errors,
            stall_count,
            turn_failures,
            last_text: end.last_text.as_deref(),
            pause_requested: self.pending.pause,
        });
        self.follow(decision).await
    }

    /// Checks again after someone else's turn, and decides on the new result alone.
    async fn recheck(&mut self) -> Step<()> {
        let run = self.check().await?;
        let n = self.row.goal.iteration;
        let hash = self
            .verdict
            .as_ref()
            .and_then(|verdict| verdict.progress_hash.clone());
        self.record(n, &run, hash.as_deref(), GoalStateUpdate::default())
            .await?;
        self.verdict = Some(Verdict {
            progress_hash: hash,
            outcome: run.outcome,
            reason: run.reason.clone(),
        });
        self.feedback = run.results;
        if self.pending.reload {
            self.reload_spec().await?;
        }
        let harness = self.session.harness;
        let decision = decide_boundary(&BoundaryInput {
            outcome: run.outcome,
            iteration: n,
            max_iterations: self.row.goal.limits.max_iterations,
            harness,
            spend_reached: spend_reached(&self.row.goal, harness),
            budget_tripped: false,
            check_errors: self.row.check_errors,
            stall_count: self.row.stall_count,
            turn_failures: self.row.turn_failures,
            last_text: None,
            pause_requested: self.pending.pause,
        });
        self.follow(decision).await
    }

    async fn follow(&mut self, decision: Boundary) -> Step<()> {
        match decision {
            Boundary::Next => Ok(()),
            Boundary::Met => Err(self
                .conclude(Conclusion::End(GoalEndReason::ChecksPassed))
                .await),
            Boundary::End(reason) => Err(self.conclude(Conclusion::End(reason)).await),
            Boundary::Pause(reason, text) => {
                Err(self.conclude(Conclusion::Pause(reason, text)).await)
            }
        }
    }

    /// Writes a check's verdict on iteration `n` and the goal, with the counters it moved.
    async fn record(
        &mut self,
        n: u32,
        run: &CheckRun,
        hash: Option<&str>,
        counters: GoalStateUpdate,
    ) -> Step<()> {
        let (id, hash) = (self.row.goal.id.clone(), hash.map(str::to_string));
        let (outcome, reason, results) = (run.outcome, run.reason.clone(), run.results.clone());
        self.db(move |db| {
            db.set_iteration_results(
                &id,
                n,
                outcome,
                reason.as_deref(),
                &results,
                hash.as_deref(),
            )
        })
        .await?;
        // A separate process: the turn's cost baseline is untouched.
        let spent = if run.cost_usd > 0.0 {
            FieldOp::Set(self.row.goal.spent_cost_usd + run.cost_usd)
        } else {
            FieldOp::Unchanged
        };
        self.write(GoalStateUpdate {
            spent_cost_usd: spent,
            last_outcome: FieldOp::Set(run.outcome),
            last_reason: run
                .reason
                .clone()
                .map(FieldOp::Set)
                .unwrap_or(FieldOp::Clear),
            ..counters
        })
        .await
    }

    /// Runs the goal's checks. Someone else's turn cancels them; they run again once it ends.
    async fn check(&mut self) -> Step<CheckRun> {
        loop {
            let rt = self.runtime()?;
            let (asks, mut asked) = mpsc::unbounded_channel();
            let context = CheckContext {
                workstream: self.workstream.clone(),
                checks: self.row.goal.checks.clone(),
                condition: self.row.goal.condition.clone(),
                last_text: self.last_text.clone(),
                spend_reached: spend_reached(&self.row.goal, self.session.harness),
                harness: self.session.harness,
                author: self.row.goal.author.clone(),
                asks,
            };
            let cancel = Notify::new();
            let mut checking = Box::pin(run_checks(&rt, &context, &cancel));
            let mut superseded = false;
            let mut watching = true;
            let finished = loop {
                tokio::select! {
                    biased;
                    signal = next_signal(&mut self.control, self.cap_at, &self.stop) => {
                        match self.react(signal).await? {
                            Reaction::Continue => {}
                            Reaction::Abort(how) => {
                                drop(checking);
                                return Err(self.conclude(how).await);
                            }
                            Reaction::Quit => return Err(Over),
                        }
                    }
                    run = &mut checking => break run,
                    Some(progress) = asked.recv() => match progress {
                        Progress::Asked(what) => self.wait_on(what).await?,
                        Progress::Answered | Progress::Ended => self.back_to(GoalStep::Checking).await?,
                    },
                    event = self.watch.next(), if watching => match event {
                        Some(SessionEvent::UserMessage { .. }
                            | SessionEvent::AssistantText { .. }
                            | SessionEvent::ToolUse { .. }
                            | SessionEvent::PermissionAsk { .. }
                            | SessionEvent::QuestionAsk { .. }) => {
                            if !superseded {
                                superseded = true;
                                cancel.notify_one();
                            }
                        }
                        Some(SessionEvent::Exited { outcome, .. }) => match exit_kind(outcome) {
                            Exit::TakenBack => self.pending.pause = true,
                            Exit::Ended(how) => {
                                drop(checking);
                                return Err(self.conclude(how).await);
                            }
                            // The checks go on; the next boundary relaunches it.
                            Exit::Crashed(detail) => {
                                if let Some(how) = self.count_crash(detail).await? {
                                    drop(checking);
                                    return Err(self.conclude(how).await);
                                }
                            }
                        },
                        Some(_) => {}
                        None => {
                            watching = false;
                            self.pending.pause = true;
                        }
                    },
                }
            };
            drop(checking);
            drop(rt);
            // A validate session cut off mid-ask leaves the goal waiting with its clock stopped.
            self.back_to(GoalStep::Checking).await?;
            if !superseded {
                return Ok(finished);
            }
            // Its result is dropped, but what its paid layers spent is not.
            if finished.cost_usd > 0.0 {
                self.write(GoalStateUpdate {
                    spent_cost_usd: FieldOp::Set(self.row.goal.spent_cost_usd + finished.cost_usd),
                    ..Default::default()
                })
                .await?;
            }
            self.wait_idle().await?;
        }
    }

    /// Waits until someone else's turn is over, and the actor idle, for a quiet while.
    async fn wait_idle(&mut self) -> Step<()> {
        let mut state = IterationWatch::foreign(QUIET);
        let idle = self.idle_probe();
        loop {
            tokio::select! {
                biased;
                signal = next_signal(&mut self.control, self.cap_at, &self.stop) => {
                    match self.react(signal).await? {
                        Reaction::Continue => {}
                        Reaction::Abort(how) => return Err(self.conclude(how).await),
                        Reaction::Quit => return Err(Over),
                    }
                }
                // Someone else's turn never reports an ask.
                _ = state.finish(&mut self.watch, &idle) => break,
            }
        }
        match state.into_end().exited {
            Some(outcome) => self.on_exit(outcome).await,
            None => Ok(()),
        }
    }

    /// The harness left between iterations; a crash is relaunched by the next boundary.
    async fn on_exit(&mut self, outcome: ExitOutcome) -> Step<()> {
        let how = match exit_kind(outcome) {
            Exit::TakenBack => {
                self.pending.pause = true;
                return Ok(());
            }
            Exit::Ended(how) => how,
            Exit::Crashed(detail) => match self.count_crash(detail).await? {
                Some(how) => how,
                None => return Ok(()),
            },
        };
        Err(self.conclude(how).await)
    }

    async fn drain_controls(&mut self) -> Step<()> {
        while let Ok(control) = self.control.try_recv() {
            match self.apply(control).await? {
                Reaction::Continue => {}
                Reaction::Abort(how) => return Err(self.conclude(how).await),
                Reaction::Quit => return Err(Over),
            }
        }
        Ok(())
    }

    async fn react(&mut self, signal: Signal) -> Step<Reaction> {
        match signal {
            Signal::Stopped => Ok(Reaction::Quit),
            Signal::Cap => Ok(Reaction::Abort(Conclusion::End(GoalEndReason::TimeLimit))),
            Signal::Control(control) => self.apply(control).await,
        }
    }

    async fn apply(&mut self, control: RunnerControl) -> Step<Reaction> {
        match control {
            RunnerControl::Pause => self.request_pause().await?,
            RunnerControl::UserInterrupt => {
                self.pending.interrupted |= self.in_flight;
                self.request_pause().await?;
            }
            RunnerControl::Reload => self.pending.reload = true,
            RunnerControl::Stop(reason) => {
                self.write(GoalStateUpdate {
                    step: FieldOp::Set(GoalStep::Stopping),
                    ..Default::default()
                })
                .await?;
                return Ok(Reaction::Abort(Conclusion::End(reason)));
            }
        }
        Ok(Reaction::Continue)
    }

    async fn request_pause(&mut self) -> Step<()> {
        self.pending.pause = true;
        if matches!(
            self.row.goal.step,
            Some(GoalStep::Working | GoalStep::Waiting)
        ) {
            self.write(GoalStateUpdate {
                step: FieldOp::Set(GoalStep::Pausing),
                ..Default::default()
            })
            .await?;
        }
        Ok(())
    }

    /// Takes an edit made since the run last looked: limits, checks and prompts apply from here.
    async fn reload_spec(&mut self) -> Step<()> {
        let id = self.row.goal.id.clone();
        let fresh = self.db(move |db| db.goal(&id)).await?.ok_or(Over)?;
        let goal = &mut self.row.goal;
        goal.condition = fresh.goal.condition;
        goal.work_prompt = fresh.goal.work_prompt;
        goal.checks = fresh.goal.checks;
        goal.limits = fresh.goal.limits;
        self.cap_at = self.clock.cap_at(self.row.goal.limits.max_active_secs);
        self.pending.reload = false;
        Ok(())
    }

    /// Adds the iteration's spend and writes its end. `step` moves the goal on in the same write.
    async fn close_iteration(
        &mut self,
        n: u32,
        end: &IterationEnd,
        note: Option<GoalIterationNote>,
        step: Option<GoalStep>,
    ) -> Step<()> {
        let so_far = match self.runtime() {
            Ok(rt) => rt
                .sessions()
                .live_driver(&self.session.id)
                .await
                .map(|driver| driver.usage_so_far())
                .unwrap_or_default(),
            Err(_) => UsageSoFar::default(),
        };
        let reading = end.cost_usd.or(so_far.cost_usd);
        let baseline = self.row.cost_baseline_usd.unwrap_or(0.0);
        let (delta, baseline) = usage_delta(baseline, reading);
        let tokens = end.thread_tokens.or(so_far.thread_tokens);
        let (token_delta, token_baseline) =
            usage_delta(self.row.token_baseline.unwrap_or(0), tokens);
        let closed = ClosedIteration {
            turn_status: end.turn_status.clone(),
            cost_usd: reading.map(|_| delta),
            tokens: tokens.map(|_| token_delta),
            note,
        };
        let id = self.row.goal.id.clone();
        self.db(move |db| db.finish_iteration(&id, n, &closed))
            .await?;
        let spent_tokens = self.row.goal.spent_tokens.saturating_add(token_delta);
        self.write(GoalStateUpdate {
            spent_cost_usd: FieldOp::Set(self.row.goal.spent_cost_usd + delta),
            cost_baseline_usd: FieldOp::Set(baseline),
            spent_tokens: tokens
                .map(|_| FieldOp::Set(spent_tokens))
                .unwrap_or_default(),
            token_baseline: tokens
                .map(|_| FieldOp::Set(token_baseline))
                .unwrap_or_default(),
            step: step.map(FieldOp::Set).unwrap_or_default(),
            ..Default::default()
        })
        .await
    }

    /// Writes how the run ends. A stop that arrived while the run was pausing wins over the pause.
    async fn conclude(&mut self, mut how: Conclusion) -> Over {
        self.lease.finishing();
        let _transition = self.lease.transition().await;
        // A stop may have gone to a runner chained after this one; the row has it either way.
        let id = self.row.goal.id.clone();
        let requested = self
            .db(move |db| db.goal(&id))
            .await
            .ok()
            .flatten()
            .and_then(|row| row.control_request);
        if let (Some(ControlRequest::Stop(reason)), Conclusion::Pause(..)) = (requested, &how) {
            how = Conclusion::End(reason);
        }
        tracing::info!("goal {}: {}", self.row.goal.id, describe(&how));
        let _ = self.write(how.update()).await;
        self.concluded = Some(how);
        Over
    }

    /// Claude's cap is sticky, so the harness this run capped is relaunched without it, unless it went down with the goal.
    async fn lift_cap(&self) {
        let Some(owned) = &self.owned else {
            return;
        };
        let kept_down = matches!(
            self.concluded,
            None | Some(Conclusion::End(
                GoalEndReason::SessionEnded | GoalEndReason::WorkstreamArchived
            ))
        );
        if kept_down {
            return;
        }
        let Ok(rt) = self.runtime() else {
            return;
        };
        let sessions = rt.sessions();
        tokio::select! {
            biased;
            () = self.stop.notified() => {}
            () = sessions.lift_budget_cap(&self.session.id, &self.row.goal.id, owned) => {}
        }
    }

    async fn write(&mut self, update: GoalStateUpdate) -> Step<()> {
        self.store(update).await?;
        announce(&self.events, &self.row.goal);
        Ok(())
    }

    /// `write` without the announcement.
    async fn store(&mut self, mut update: GoalStateUpdate) -> Step<()> {
        update.active_ms = FieldOp::Set(self.clock.active_ms());
        let id = self.row.goal.id.clone();
        match self.db(move |db| db.update_goal_state(&id, update)).await? {
            Some(row) => {
                self.row = row;
                Ok(())
            }
            None => Err(Over),
        }
    }

    async fn db<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Db) -> Result<T> + Send + 'static,
    ) -> Step<T> {
        read(&self.db, f).await
    }

    fn idle_probe(&self) -> impl Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Sync {
        idle_probe(self.actors.clone(), self.session.id.clone())
    }
}

async fn read<T: Send + 'static>(
    db: &Arc<Db>,
    f: impl FnOnce(&Db) -> Result<T> + Send + 'static,
) -> Step<T> {
    let db = db.clone();
    match tokio::task::spawn_blocking(move || f(&db)).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => {
            tracing::warn!("a goal's store read or write failed: {e}");
            Err(Over)
        }
        Err(e) => {
            tracing::warn!("a goal's store task failed: {e}");
            Err(Over)
        }
    }
}

fn warn_over(goal_id: &str, error: crate::error::CoreError) -> Over {
    tracing::warn!("goal {goal_id}: it cannot run: {error}");
    Over
}

/// Between turns, with no send waiting to land.
pub(crate) async fn is_idle(actors: &SessionActors, session_id: &str) -> bool {
    actors
        .rev_state(session_id, None)
        .await
        .is_none_or(|state| !state.working && state.in_flight == 0)
}

fn spec_of(goal: &Goal) -> GoalSpec {
    GoalSpec {
        condition: goal.condition.clone(),
        work_prompt: goal.work_prompt.clone(),
        source: goal.source,
        source_ref: goal.source_ref.clone(),
        source_phase: goal.source_phase.clone(),
        checks: goal.checks.clone(),
        limits: goal.limits.clone(),
    }
}

/// What the harness leaving means to a run that did not cause it.
enum Exit {
    /// A client detached it to relaunch: the run pauses, it does not end.
    TakenBack,
    Ended(Conclusion),
    Crashed(String),
}

fn exit_kind(outcome: ExitOutcome) -> Exit {
    match outcome {
        ExitOutcome::Detached => Exit::TakenBack,
        ExitOutcome::Killed => Exit::Ended(Conclusion::End(GoalEndReason::SessionEnded)),
        other => Exit::Crashed(format!(
            "The agent stopped unexpectedly ({}).",
            other.state().as_str()
        )),
    }
}

fn describe(how: &Conclusion) -> String {
    match how {
        Conclusion::End(reason) => format!("ends as {}", reason.as_str()),
        Conclusion::Pause(reason, _) => format!("pauses as {}", reason.as_str()),
    }
}

/// Hex of a 64-bit hash of HEAD's oid and the worktree diff; unreadable means no hash, so it never repeats.
fn progress_hash(worktree: &Path) -> Option<String> {
    let repo = crate::gitops::open_repo(worktree).ok()?;
    let head = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| oid.to_string())
        .unwrap_or_default();
    let files = crate::gitops::worktree_diff(worktree).ok()?;
    let mut hasher = Xxh3::new();
    hasher.update(head.as_bytes());
    for file in &files {
        for part in [&file.path, &file.change, &file.patch] {
            hasher.update(&[0]);
            hasher.update(part.as_bytes());
        }
    }
    Some(format!("{:016x}", hasher.digest()))
}
