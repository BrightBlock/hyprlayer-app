//! A goal's spend is capped: the budget ends it at a boundary and counts only its
//! own turns, the `--max-budget-usd` backstop on a harness the goal launched trips
//! mid-turn, and that cap is lifted when the goal lets the session go, except for
//! a session the user killed.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalDetail, GoalEndReason, GoalLimits, GoalPauseReason, GoalSource, GoalSpec,
    GoalStatus,
};
use hyprlayer_core::runtime::goals::{GoalPatch, GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, SessionLogEntry, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{clear, control, control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
const GOAL_PROMPT: &str = "hyprlayer:goal";
const NEVER: &str = r#"echo "not yet"; false"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_budget() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_budget", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let controls = sandbox.root.join("goal-control");

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Goal budget").await;
    let ws = text(&view, "/item/id");

    j.step("a session the goal did not spawn has no cap");
    let earlier = new_session(&mut j, &client, &sandbox, &ws, "$SESSION_BOUNDARY", &[]).await;
    let goal = start(
        &mut j,
        &client,
        &earlier,
        spec("E2E_GOAL_BOUNDARY", NEVER, 10, 1.0),
        "$GOAL_BOUNDARY",
    )
    .await;
    let launched = launches(&controls, &earlier);
    let first = launched.first().map(String::as_str).unwrap_or_default();
    j.guard(
        "its one launch denies ProposeGoal and carries no budget cap",
        launched.len() == 1 && denies(first, "ProposeGoal") && cap_of(first).is_none(),
        json!({
            "launches": launched.len(),
            "deniesProposeGoal": denies(first, "ProposeGoal"),
            "cap": cap_of(first),
        }),
    );

    j.step("the boundary budget ends the goal");
    let ended = wait_ended(&client, &goal.id).await;
    let launched = launches(&controls, &earlier);
    j.guard(
        "it ends exhausted on the budget at iteration 3",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::BudgetLimit)
            && ended.goal.iteration == 3,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
        }),
    );
    j.guard(
        "it spent $1.20 on its three 40-cent iterations; the session's earlier turn is not counted",
        cents(ended.goal.spent_cost_usd) == 120,
        json!({ "spentCents": cents(ended.goal.spent_cost_usd) }),
    );
    let capped = launched
        .iter()
        .filter(|line| cap_of(line).is_some())
        .count();
    j.guard(
        "no launch carried a budget cap",
        capped == 0,
        json!({ "launches": launched.len(), "capped": capped }),
    );

    j.step("turns during a pause do not count");
    let paused = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_PAUSE",
        &[("hold", "2")],
    )
    .await;
    let goal = start(
        &mut j,
        &client,
        &paused,
        spec("E2E_GOAL_PAUSE", NEVER, 2, 10.0),
        "$GOAL_PAUSE",
    )
    .await;
    wait_sent(&client, &goal.id, 1).await;
    act(
        &mut j,
        &client,
        Request::PauseGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("pause_goal");
    control(&sandbox, "release", "");
    let after_one = wait_ended(&client, &goal.id).await;
    j.guard(
        "the goal pauses after iteration 1 with $0.40 spent",
        after_one.goal.status == GoalStatus::Paused
            && after_one.goal.pause_reason == Some(GoalPauseReason::User)
            && after_one.goal.iteration == 1
            && cents(after_one.goal.spent_cost_usd) == 40,
        json!({
            "status": after_one.goal.status.as_str(),
            "pauseReason": after_one.goal.pause_reason.map(GoalPauseReason::as_str),
            "iteration": after_one.goal.iteration,
            "spentCents": cents(after_one.goal.spent_cost_usd),
        }),
    );
    send(&mut j, &client, &paused, "E2E_USER_WHILE_PAUSED").await;
    let reply = wait_reply(&client, &paused, "E2E_USER_WHILE_PAUSED").await;
    act(
        &mut j,
        &client,
        Request::ResumeGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("resume_goal");
    let ended = wait_ended(&client, &goal.id).await;
    j.guard(
        "the user's 40-cent turn is not counted: iteration 2 brings the spend to $0.80",
        reply.done
            && ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && ended.goal.iteration == 2
            && cents(ended.goal.spent_cost_usd) == 80,
        json!({
            "userTurnEnded": reply.done,
            "status": ended.goal.status.as_str(),
            "iteration": ended.goal.iteration,
            "spentCents": cents(ended.goal.spent_cost_usd),
        }),
    );

    j.step("the backstop trips mid-turn");
    control(&sandbox, "cost", "70");
    let goal = start_new(
        &mut j,
        &client,
        &ws,
        spec("E2E_GOAL_TRIP", NEVER, 10, 1.0),
        "$GOAL_TRIP",
        "$SESSION_TRIP",
    )
    .await;
    let tripped = goal.session_id.clone();
    let ended = wait_ended(&client, &goal.id).await;
    let launched = launches(&controls, &tripped);
    let first = launched.first().map(String::as_str).unwrap_or_default();
    j.guard(
        "its launch carries the goal's one-dollar cap",
        cap_of(first) == Some("1"),
        json!({ "cap": cap_of(first) }),
    );
    let statuses = turn_statuses(&ended);
    let trip = statuses
        .iter()
        .position(|status| *status == Some("budget_exhausted"));
    j.guard(
        "iteration 2's turn trips the backstop",
        statuses == [Some("completed"), Some("budget_exhausted")],
        // Goldens never record a raw turn status, so the evidence states the facts instead.
        json!({
            "tripIteration": trip.map(|index| index + 1),
            "earlierTurnsCompleted": trip
                .is_some_and(|index| statuses[..index].iter().all(|s| *s == Some("completed"))),
        }),
    );
    j.guard(
        "the goal ends exhausted on the budget at iteration 2",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::BudgetLimit)
            && ended.goal.iteration == 2,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
            "spentCents": cents(ended.goal.spent_cost_usd),
        }),
    );

    j.step("the cap is lifted when the runner exits");
    let lifted = eventually(IO, || launches(&controls, &tripped).len() >= 2).await;
    let launched = launches(&controls, &tripped);
    let newest = launched.last().map(String::as_str).unwrap_or_default();
    send(&mut j, &client, &tripped, "E2E_AFTER_THE_END").await;
    let reply = wait_reply(&client, &tripped, "E2E_AFTER_THE_END").await;
    j.guard(
        "after the goal ends, its session is relaunched with --resume and without the cap",
        lifted && resumes(newest, &tripped) && cap_of(newest).is_none(),
        json!({
            "relaunched": lifted,
            "resumes": resumes(newest, &tripped),
            "cap": cap_of(newest),
        }),
    );
    j.guard(
        "a message after the end gets a normal reply",
        reply.normal(),
        json!({ "normalReply": reply.normal() }),
    );
    clear(&sandbox, "cost");
    control(&sandbox, "hold", "1");
    let goal = start_new(
        &mut j,
        &client,
        &ws,
        spec("E2E_GOAL_LIFT", NEVER, 10, 10.0),
        "$GOAL_LIFT",
        "$SESSION_LIFT",
    )
    .await;
    let lift = goal.session_id.clone();
    wait_sent(&client, &goal.id, 1).await;
    act(
        &mut j,
        &client,
        Request::PauseGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("pause_goal");
    control(&sandbox, "release", "");
    let paused = wait_ended(&client, &goal.id).await;
    clear(&sandbox, "hold");
    let lifted = eventually(IO, || launches(&controls, &lift).len() >= 2).await;
    let launched = launches(&controls, &lift);
    let first = launched.first().map(String::as_str).unwrap_or_default();
    let newest = launched.last().map(String::as_str).unwrap_or_default();
    send(&mut j, &client, &lift, "E2E_DURING_THE_PAUSE").await;
    let reply = wait_reply(&client, &lift, "E2E_DURING_THE_PAUSE").await;
    j.guard(
        "after a pause, the capped launch is replaced by a --resume without the cap",
        paused.goal.status == GoalStatus::Paused
            && cap_of(first) == Some("10")
            && lifted
            && resumes(newest, &lift)
            && cap_of(newest).is_none(),
        json!({
            "status": paused.goal.status.as_str(),
            "firstCap": cap_of(first),
            "relaunched": lifted,
            "resumes": resumes(newest, &lift),
            "cap": cap_of(newest),
        }),
    );
    j.guard(
        "a message during the pause gets a normal reply",
        reply.normal(),
        json!({ "normalReply": reply.normal() }),
    );

    j.step("extend while the cap is lifted");
    control(&sandbox, "slow-exit", "");
    control(&sandbox, "hold", "1");
    let goal = start_new(
        &mut j,
        &client,
        &ws,
        spec("E2E_GOAL_EXTEND", NEVER, 2, 10.0),
        "$GOAL_EXTEND",
        "$SESSION_EXTEND",
    )
    .await;
    let extend = goal.session_id.clone();
    wait_sent(&client, &goal.id, 1).await;
    // Attached to the live capped harness, as an open desktop tab is.
    let (_, _, _, mut watching) = client
        .resume_session_stream(extend.clone(), None, None, None, None)
        .await
        .expect("resume_session_stream");
    clear(&sandbox, "hold");
    control(&sandbox, "release", "");
    let at_cap = wait_ended(&client, &goal.id).await;
    // Read at launch, so only the first harness exits slowly.
    clear(&sandbox, "slow-exit");
    let raised = act(
        &mut j,
        &client,
        Request::UpdateGoal {
            goal_id: goal.id.clone(),
            patch: raise_to(3),
        },
    )
    .await;
    let launched_at_resume = launches(&controls, &extend).len();
    let resumed = act(
        &mut j,
        &client,
        Request::ResumeGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await;
    j.guard(
        "the goal ends at its cap of 2, and a raise and a resume land while the lift waits on the exit",
        at_cap.goal.status == GoalStatus::Exhausted
            && at_cap.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && at_cap.goal.iteration == 2
            && raised
                .as_ref()
                .is_ok_and(|goal| goal.limits.max_iterations == 3)
            && resumed
                .as_ref()
                .is_ok_and(|goal| goal.status == GoalStatus::Running)
            && launched_at_resume == 1,
        json!({
            "status": at_cap.goal.status.as_str(),
            "endReason": at_cap.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": at_cap.goal.iteration,
            "raised": raised.as_ref().ok().map(|goal| goal.limits.max_iterations),
            "resumed": resumed.as_ref().ok().map(|goal| goal.status.as_str()),
            "launchesAtResume": launched_at_resume,
        }),
    );
    let ended = wait_ended(&client, &goal.id).await;
    let log = session_log(&client, &extend).await;
    let sent = goal_messages(&log);
    let third_cents = sent
        .get(2)
        .and_then(|at| turn_cost_after(&log, *at))
        .map(cents);
    let received = goal_lines(&controls.join(&extend).join("stdin.log"), "E2E_GOAL_EXTEND");
    let launched = launches(&controls, &extend);
    let newest = launched.last().map(String::as_str).unwrap_or_default();
    // Cost is per process, so a fresh harness reports its first turn alone.
    j.guard(
        "iteration 3 is sent once, to the relaunched harness",
        sent.len() == 3
            && received == 3
            && launched.len() == 2
            && resumes(newest, &extend)
            && cap_of(newest).is_none()
            && third_cents == Some(40),
        json!({
            "goalMessages": sent.len(),
            "receivedByTheHarness": received,
            "launches": launched.len(),
            "resumes": resumes(newest, &extend),
            "cap": cap_of(newest),
            "iteration3CostCents": third_cents,
        }),
    );
    let streamed = drain(&mut watching).await;
    let exited = streamed
        .iter()
        .any(|e| matches!(e, SessionEvent::Exited { .. }));
    let relaunched = streamed
        .iter()
        .any(|e| matches!(e, SessionEvent::Relaunched {}));
    let goal_turns = streamed
        .iter()
        .filter(|e| matches!(e, SessionEvent::UserMessage { prompt_name: Some(name), .. } if name == GOAL_PROMPT))
        .count();
    let last_done = streamed
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e,
                SessionEvent::UserMessage { .. } | SessionEvent::TurnDone { .. }
            )
        })
        .is_some_and(|e| matches!(e, SessionEvent::TurnDone { .. }));
    j.guard(
        "a client attached before the lift keeps its stream: it sees the relaunch and iteration 3's turn",
        !exited && relaunched && goal_turns == 2 && last_done,
        json!({
            "exited": exited,
            "relaunched": relaunched,
            "goalTurnsAfterAttach": goal_turns,
            "lastTurnDone": last_done,
        }),
    );
    j.guard(
        "the goal ends exhausted at its new cap of 3",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && ended.goal.iteration == 3,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
        }),
    );

    j.step("a killed session is not relaunched to lift the cap");
    control(&sandbox, "hold", "1");
    let goal = start_new(
        &mut j,
        &client,
        &ws,
        spec("E2E_GOAL_KILL", NEVER, 10, 10.0),
        "$GOAL_KILL",
        "$SESSION_KILL",
    )
    .await;
    let killed = goal.session_id.clone();
    let stdin = controls.join(&killed).join("stdin.log");
    let holding = eventually(IO, || goal_lines(&stdin, "E2E_GOAL_KILL") >= 1).await;
    j.call(
        &client,
        Request::KillSession {
            session_id: killed.clone(),
        },
    )
    .await
    .expect("kill_session");
    let ended = wait_ended(&client, &goal.id).await;
    clear(&sandbox, "hold");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let launched = launches(&controls, &killed);
    let first = launched.first().map(String::as_str).unwrap_or_default();
    j.guard(
        "the kill ends the goal with its session",
        holding
            && ended.goal.status == GoalStatus::Stopped
            && ended.goal.end_reason == Some(GoalEndReason::SessionEnded),
        json!({
            "iterationWasRunning": holding,
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
        }),
    );
    j.guard(
        "its launch carried the cap, and nothing relaunched it after the kill",
        launched.len() == 1 && cap_of(first) == Some("10"),
        json!({ "launches": launched.len(), "cap": cap_of(first) }),
    );

    j.step("a crash past the budget ends the goal");
    // Stub turn 1 is the session's own; iteration 2 (turn 3) reports its cost, then exits.
    let crashed = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_CRASH",
        &[("crash-after", "3")],
    )
    .await;
    let goal = start(
        &mut j,
        &client,
        &crashed,
        spec("E2E_GOAL_CRASH_SPENT", NEVER, 10, 0.5),
        "$GOAL_CRASH_SPENT",
    )
    .await;
    let ended = wait_ended(&client, &goal.id).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let launched = launches(&controls, &crashed);
    j.guard(
        "the crashed iteration's spend reaches the cap, so the goal ends at iteration 2",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::BudgetLimit)
            && ended.goal.iteration == 2,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
            "spentCents": cents(ended.goal.spent_cost_usd),
        }),
    );
    j.guard(
        "nothing relaunched the crashed session with a spent budget",
        launched.len() == 1,
        json!({ "launches": launched.len(), "caps": launched.iter().map(|l| cap_of(l)).collect::<Vec<_>>() }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn limits(max_iterations: u32, max_cost_usd: f64) -> GoalLimits {
    GoalLimits {
        max_iterations,
        max_cost_usd: Some(max_cost_usd),
        max_tokens: None,
        max_active_secs: 3600,
    }
}

fn spec(condition: &str, command: &str, max_iterations: u32, max_cost_usd: f64) -> GoalSpec {
    GoalSpec {
        condition: condition.into(),
        work_prompt: "Work toward the goal.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![GoalCheck::Command {
            command: command.into(),
            timeout_secs: 60,
            label: None,
        }],
        limits: limits(max_iterations, max_cost_usd),
    }
}

fn raise_to(max_iterations: u32) -> GoalPatch {
    GoalPatch {
        condition: None,
        work_prompt: None,
        checks: None,
        limits: Some(limits(max_iterations, 10.0)),
    }
}

/// A started session whose first turn has finished; `controls` are its per-session stub controls.
async fn new_session(
    j: &mut Journey,
    client: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
    alias: &str,
    controls: &[(&str, &str)],
) -> String {
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "hello".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, alias);
    for (name, contents) in controls {
        control_for(sandbox, &session.id, name, contents);
    }
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    session.id
}

async fn start(
    j: &mut Journey,
    client: &RpcClient,
    session_id: &str,
    spec: GoalSpec,
    alias: &str,
) -> Goal {
    let goal = act(
        j,
        client,
        Request::StartGoal {
            request: StartGoalRequest {
                target: GoalTarget::Session {
                    session_id: session_id.into(),
                },
                spec,
            },
        },
    )
    .await
    .expect("start_goal");
    j.alias(&goal.id, alias);
    goal
}

/// A goal that launches its own General session, so its runner owns that harness.
async fn start_new(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    spec: GoalSpec,
    goal_alias: &str,
    session_alias: &str,
) -> Goal {
    let goal = act(
        j,
        client,
        Request::StartGoal {
            request: StartGoalRequest {
                target: GoalTarget::NewSession {
                    workstream_id: ws.into(),
                    stage: Stage::General,
                    harness: Some(Harness::Claude),
                    model: None,
                    effort: None,
                    permission_mode: None,
                    attached_docs: vec![],
                },
                spec,
            },
        },
    )
    .await
    .expect("start_goal");
    j.alias(&goal.id, goal_alias);
    j.alias(&goal.session_id, session_alias);
    goal
}

async fn act(j: &mut Journey, client: &RpcClient, request: Request) -> Result<Goal, WireError> {
    j.call(client, request)
        .await
        .map(|value| serde_json::from_value(value).expect("the command answers a goal"))
}

async fn send(j: &mut Journey, client: &RpcClient, session_id: &str, text: &str) {
    j.call(
        client,
        Request::SendUserMessage {
            session_id: session_id.into(),
            text: text.into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
}

/// Until the goal has sent its `n`th iteration, which a `hold` turn then keeps open.
async fn wait_sent(client: &RpcClient, goal_id: &str, n: usize) -> GoalDetail {
    wait_goal(client, goal_id, |d| d.iterations.len() >= n).await
}

async fn wait_ended(client: &RpcClient, goal_id: &str) -> GoalDetail {
    wait_goal(client, goal_id, |d| d.goal.status != GoalStatus::Running).await
}

/// Polls until `done` holds or the wait runs out; the last reading is returned either way.
async fn wait_goal(
    client: &RpcClient,
    goal_id: &str,
    done: impl Fn(&GoalDetail) -> bool,
) -> GoalDetail {
    let deadline = tokio::time::Instant::now() + GOAL_WAIT;
    loop {
        let detail = client
            .get_goal(goal_id.to_string())
            .await
            .expect("get_goal");
        if done(&detail) || tokio::time::Instant::now() >= deadline {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn session_log(client: &RpcClient, session_id: &str) -> Vec<SessionLogEntry> {
    client
        .get_session_log(session_id.to_string())
        .await
        .expect("get_session_log")
}

#[derive(Default)]
struct Reply {
    done: bool,
    text: Option<String>,
    turn_status: Option<String>,
}

impl Reply {
    /// The stub's ordinary turn, not a sticky budget refusal.
    fn normal(&self) -> bool {
        self.turn_status.as_deref() == Some("completed")
            && self
                .text
                .as_deref()
                .is_some_and(|text| text.starts_with("counter is"))
    }
}

/// Polls the log until the user message holding `needle` has a `TurnDone` after it.
async fn wait_reply(client: &RpcClient, session_id: &str, needle: &str) -> Reply {
    let deadline = tokio::time::Instant::now() + IO;
    loop {
        let reply = reply_to(&session_log(client, session_id).await, needle);
        if reply.done || tokio::time::Instant::now() >= deadline {
            return reply;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn reply_to(log: &[SessionLogEntry], needle: &str) -> Reply {
    let mut reply = Reply::default();
    let Some(at) = log.iter().position(
        |e| matches!(&e.event, SessionEvent::UserMessage { text, .. } if text.contains(needle)),
    ) else {
        return reply;
    };
    for entry in &log[at + 1..] {
        match &entry.event {
            SessionEvent::AssistantText {
                text,
                parent_id: None,
                injected: false,
            } => reply.text = Some(text.clone()),
            SessionEvent::TurnDone { turn_status, .. } => {
                reply.done = true;
                reply.turn_status = turn_status.clone();
                break;
            }
            _ => {}
        }
    }
    reply
}

/// The log rows of the user messages sent under the goal prompt name.
fn goal_messages(log: &[SessionLogEntry]) -> Vec<usize> {
    log.iter()
        .enumerate()
        .filter(|(_, entry)| {
            matches!(&entry.event, SessionEvent::UserMessage { prompt_name: Some(name), .. } if name == GOAL_PROMPT)
        })
        .map(|(at, _)| at)
        .collect()
}

/// The cost the first `TurnDone` after row `at` reports.
fn turn_cost_after(log: &[SessionLogEntry], at: usize) -> Option<f64> {
    log[at..].iter().find_map(|entry| match &entry.event {
        SessionEvent::TurnDone { cost_usd, .. } => *cost_usd,
        _ => None,
    })
}

fn turn_statuses(detail: &GoalDetail) -> Vec<Option<&str>> {
    detail
        .iterations
        .iter()
        .map(|iteration| iteration.turn_status.as_deref())
        .collect()
}

/// Each argv the stub was launched with for this session, oldest first.
fn launches(controls: &Path, session_id: &str) -> Vec<String> {
    std::fs::read_to_string(controls.join(session_id).join("argv.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Argv lines are space-joined, so a flag's value is the token after it.
fn flag_value<'a>(line: &'a str, flag: &str) -> Option<&'a str> {
    let mut tokens = line.split(' ');
    tokens.find(|token| *token == flag)?;
    tokens.next()
}

fn cap_of(line: &str) -> Option<&str> {
    flag_value(line, "--max-budget-usd")
}

fn resumes(line: &str, session_id: &str) -> bool {
    flag_value(line, "--resume") == Some(session_id)
}

/// `--disallowedTools` is one space-joined argument, so its rules run to the next flag.
fn denies(line: &str, tool: &str) -> bool {
    line.split(' ')
        .skip_while(|token| *token != "--disallowedTools")
        .skip(1)
        .take_while(|token| !token.starts_with("--"))
        .any(|token| token == tool)
}

/// The harness's stdin lines that are user messages carrying `condition`: every goal message does.
fn goal_lines(stdin_log: &Path, condition: &str) -> usize {
    std::fs::read_to_string(stdin_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(r#""type":"user""#) && l.contains(condition))
        .count()
}

fn cents(usd: f64) -> i64 {
    (usd * 100.0).round() as i64
}

/// What the stream has delivered so far, ending at its close or a quiet second.
async fn drain(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
        seen.push(event);
    }
    seen
}

async fn recv_until(
    events: &mut EventStream<SessionEvent>,
    stop: impl Fn(&SessionEvent) -> bool,
) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            let done = stop(&event);
            seen.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    seen
}
