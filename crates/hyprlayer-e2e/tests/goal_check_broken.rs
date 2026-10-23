//! A goal's checks can break rather than fail: a check that hangs is killed at
//! its timeout and keeps what it printed, a check that floods output is capped,
//! and three iterations in a row whose checks error fail the goal.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckOutcome, GoalDetail, GoalEndReason, GoalLimits, GoalSource, GoalSpec,
    GoalStatus,
};
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
const KEPT_TAIL_BYTES: usize = 8 * 1024;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_check_broken() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_check_broken", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Broken checks").await;
    let ws = text(&view, "/item/id");

    j.step("a timed-out check keeps its partial output");
    let s1 = new_session(&mut j, &client, &ws, "$S1").await;
    let hanging = start(
        &mut j,
        &client,
        &s1,
        spec("E2E_CHECK_HANGS", "echo partial-output; sleep 30", 2, 10),
        "$G1",
    )
    .await;
    let first = wait_goal(&client, &hanging.id, |d| {
        d.iterations.first().is_some_and(|i| i.outcome.is_some())
    })
    .await;
    let result = first
        .iterations
        .first()
        .and_then(|i| i.results.first())
        .cloned();
    let outcome = result.as_ref().map(|r| r.outcome);
    let reason = result.as_ref().and_then(|r| r.reason.clone());
    let kept = result
        .as_ref()
        .is_some_and(|r| r.output_tail.contains("partial-output"));
    j.guard(
        "the check errors as timed out, and its tail holds what it printed before the kill",
        outcome == Some(GoalCheckOutcome::Error)
            && reason.as_deref() == Some("timed out after 2 s")
            && kept,
        json!({
            "outcome": outcome.map(GoalCheckOutcome::as_str),
            "reason": reason,
            "tailHasPartialOutput": kept,
        }),
    );

    j.step("three errors fail the goal");
    let done = wait_goal(&client, &hanging.id, |d| {
        d.goal.status != GoalStatus::Running
    })
    .await;
    let outcomes: Vec<Option<&str>> = done
        .iterations
        .iter()
        .map(|i| i.outcome.map(GoalCheckOutcome::as_str))
        .collect();
    j.guard(
        "three iterations in a row error",
        outcomes == [Some("error"), Some("error"), Some("error")],
        json!({ "outcomes": outcomes }),
    );
    j.guard(
        "the goal fails as checks broken at iteration 3",
        done.goal.status == GoalStatus::Failed
            && done.goal.end_reason == Some(GoalEndReason::ChecksBroken)
            && done.goal.iteration == 3,
        json!({
            "status": done.goal.status.as_str(),
            "endReason": done.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": done.goal.iteration,
        }),
    );

    j.step("a flooding check is capped");
    let s2 = new_session(&mut j, &client, &ws, "$S2").await;
    let flooding = start(
        &mut j,
        &client,
        &s2,
        spec(
            "E2E_CHECK_FLOODS",
            "yes flood | head -c 2000000; false",
            60,
            1,
        ),
        "$G2",
    )
    .await;
    let done = wait_goal(&client, &flooding.id, |d| {
        d.goal.status != GoalStatus::Running
    })
    .await;
    let result = done
        .iterations
        .first()
        .and_then(|i| i.results.first())
        .cloned();
    let outcome = result.as_ref().map(|r| r.outcome);
    let tail = result.as_ref().map_or(0, |r| r.output_tail.len());
    let capped = tail > 0 && tail <= KEPT_TAIL_BYTES;
    j.guard(
        "the check fails with at most 8 KiB of its output kept",
        outcome == Some(GoalCheckOutcome::Fail) && capped,
        json!({ "outcome": outcome.map(GoalCheckOutcome::as_str), "tailCapped": capped }),
    );
    j.guard(
        "the one-iteration goal then stops at its limit",
        done.goal.status == GoalStatus::Exhausted
            && done.goal.end_reason == Some(GoalEndReason::IterationLimit),
        json!({
            "status": done.goal.status.as_str(),
            "endReason": done.goal.end_reason.map(GoalEndReason::as_str),
        }),
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

fn spec(condition: &str, command: &str, timeout_secs: u32, max_iterations: u32) -> GoalSpec {
    GoalSpec {
        condition: condition.into(),
        work_prompt: "Work toward the goal.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![GoalCheck::Command {
            command: command.into(),
            timeout_secs,
            label: None,
            cwd: None,
            phases: vec![],
        }],
        limits: GoalLimits {
            max_iterations,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 3600,
        },
    }
}

/// A started session whose first turn has finished.
async fn new_session(j: &mut Journey, client: &RpcClient, ws: &str, alias: &str) -> String {
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
    let value = j
        .call(
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
    let goal: Goal = serde_json::from_value(value).expect("start_goal answers a goal");
    j.alias(&goal.id, alias);
    goal
}

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
