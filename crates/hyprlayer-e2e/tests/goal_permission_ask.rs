//! A goal whose iteration asks for permission or a question waits for a person:
//! the step reads `waiting`, its active-time clock stops, every announced device
//! hears what it waits on, and the answer from any device lets the iteration end.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckOutcome, GoalDetail, GoalEndReason, GoalLimits, GoalSource, GoalSpec,
    GoalStatus, GoalStep,
};
use hyprlayer_core::models::session::QuestionAnswers;
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, SessionLogEntry, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
/// Long enough that a running clock would show it past the 2 s quiet window.
const HOLD: Duration = Duration::from_secs(4);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_permission_ask() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_permission_ask", &sandbox);
    let server = Server::start(&sandbox).await;
    let a = server.client();
    let b = server.connect().await;
    let mut b_events = b.take_events().expect("B's runtime events");

    let project = onboard_and_import(&mut j, &a, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &a, &project, "Goal asks").await;
    let ws = text(&view, "/item/id");
    j.call(&a, announce("client-a", "device-a"))
        .await
        .expect("A announces");
    j.call(&b, announce("client-b", "device-b"))
        .await
        .expect("B announces");

    let (session, _prompt, _stream_id, mut events) = a
        .start_stage_session_stream(
            ws,
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
    let sid = session.id.clone();
    j.alias(&sid, "$SESSION");
    // Stub turn 1 is the session's own; iteration n is turn n + 1.
    control_for(&sandbox, &sid, "ask", "2");
    control_for(&sandbox, &sid, "ask-question", "3");
    j.call(
        &a,
        Request::LaunchAndConverse {
            session_id: sid.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    drop(events);

    j.step("an ask moves the goal to waiting");
    let started = j
        .call(
            &a,
            Request::StartGoal {
                request: StartGoalRequest {
                    target: GoalTarget::Session {
                        session_id: sid.clone(),
                    },
                    spec: spec(),
                },
            },
        )
        .await
        .expect("start_goal");
    let goal: Goal = serde_json::from_value(started).expect("start_goal answers a goal");
    j.alias(&goal.id, "$GOAL");
    let mut heard = events_until(&mut b_events, IO, |e| is_waiting(e, &goal.id)).await;
    let waited_on = waiting_reason(&heard, &goal.id);
    j.guard(
        "B hears the goal wait on the Bash permission",
        waited_on.as_deref() == Some("Bash"),
        json!({ "reason": waited_on }),
    );
    let first = wait_goal(&a, &goal.id, |d| d.goal.step == Some(GoalStep::Waiting)).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let second = a.get_goal(goal.id.clone()).await.expect("get_goal");
    j.guard(
        "the step is waiting and active time stands still across 3 s",
        first.goal.step == Some(GoalStep::Waiting)
            && second.goal.step == Some(GoalStep::Waiting)
            && first.goal.active_ms == second.goal.active_ms,
        json!({
            "step": second.goal.step.map(GoalStep::as_str),
            "activeMsEqual": first.goal.active_ms == second.goal.active_ms,
        }),
    );

    j.step("another device answers");
    tokio::time::sleep(HOLD.saturating_sub(Duration::from_secs(3))).await;
    let asked_at = second.goal.active_ms;
    let request_id = pending_permission(&b, &sid).await;
    j.call(
        &b,
        Request::RespondPermission {
            session_id: sid.clone(),
            request_id: request_id.clone().unwrap_or_default(),
            allow: true,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
    heard.extend(
        events_until(&mut b_events, IO, |e| {
            matches!(e, RuntimeEvent::GoalChanged { goal_id, step: Some(GoalStep::Checking), iteration: 1, .. } if *goal_id == goal.id)
        })
        .await,
    );
    let changes = goal_changes(&heard, &goal.id);
    let expected = [
        "running/working/1",
        "running/waiting/1",
        "running/working/1",
        "running/checking/1",
    ];
    j.guard(
        "B's answer moves the goal back to working, then the iteration ends and checks run",
        request_id.is_some() && in_order(&changes, &expected),
        json!({ "answered": request_id.is_some(), "changes": changes }),
    );
    let checked = wait_goal(&a, &goal.id, |d| {
        d.iterations.first().is_some_and(|i| i.outcome.is_some())
    })
    .await;
    // The quiet window after the answer counts, the time spent waiting does not.
    let grew = checked.goal.active_ms.saturating_sub(asked_at);
    let clock_stopped = u128::from(grew) < HOLD.as_millis();
    j.guard(
        "the time spent waiting is not counted as active time",
        clock_stopped,
        json!({ "clockStoppedWhileWaiting": clock_stopped }),
    );
    let outcome = checked.iterations.first().and_then(|i| i.outcome);
    j.guard(
        "iteration 1's checks ran after the answer",
        outcome == Some(GoalCheckOutcome::Fail),
        json!({ "outcome1": outcome.map(GoalCheckOutcome::as_str) }),
    );

    j.step("a question waits too");
    let heard = events_until(&mut b_events, IO, |e| is_waiting(e, &goal.id)).await;
    let waited_on = waiting_reason(&heard, &goal.id);
    let waiting = wait_goal(&a, &goal.id, |d| d.goal.step == Some(GoalStep::Waiting)).await;
    j.guard(
        "iteration 2 waits on the question, and B hears its text",
        waiting.goal.step == Some(GoalStep::Waiting)
            && waiting.goal.iteration == 2
            && waited_on.as_deref() == Some("Ship it?"),
        json!({
            "step": waiting.goal.step.map(GoalStep::as_str),
            "iteration": waiting.goal.iteration,
            "reason": waited_on,
        }),
    );
    let question_id = pending_question(&a, &sid).await;
    j.call(
        &a,
        Request::AnswerQuestion {
            session_id: sid,
            request_id: question_id.clone().unwrap_or_default(),
            answers: QuestionAnswers(BTreeMap::from([("Ship".to_string(), "Yes".to_string())])),
        },
    )
    .await
    .expect("answer_question");
    let after = events_until(&mut b_events, IO, |e| {
        matches!(e, RuntimeEvent::GoalChanged { goal_id, status: GoalStatus::Met, .. } if *goal_id == goal.id)
    })
    .await;
    let changes = goal_changes(&after, &goal.id);
    j.guard(
        "A's answer moves the goal back to working",
        question_id.is_some() && changes.first().map(String::as_str) == Some("running/working/2"),
        json!({ "answered": question_id.is_some(), "changes": changes }),
    );
    let done = wait_goal(&a, &goal.id, |d| d.goal.status != GoalStatus::Running).await;
    j.guard(
        "the goal meets at iteration 2",
        done.goal.status == GoalStatus::Met
            && done.goal.end_reason == Some(GoalEndReason::ChecksPassed)
            && done.goal.iteration == 2,
        json!({
            "status": done.goal.status.as_str(),
            "endReason": done.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": done.goal.iteration,
        }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&a).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn spec() -> GoalSpec {
    GoalSpec {
        condition: "E2E_GOAL_ASKS".into(),
        work_prompt: "Raise the counter to 3.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![GoalCheck::Command {
            command: r#"test "$(cat counter.txt)" -ge 3"#.into(),
            timeout_secs: 60,
            label: None,
            cwd: None,
            phases: vec![],
        }],
        limits: GoalLimits {
            max_iterations: 5,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 3600,
        },
    }
}

fn announce(client_id: &str, device: &str) -> Request {
    Request::AnnounceClient {
        client_id: client_id.into(),
        device: device.into(),
        capabilities: TARGETED_EVENTS.iter().map(|k| k.to_string()).collect(),
    }
}

fn is_waiting(event: &RuntimeEvent, goal: &str) -> bool {
    matches!(event, RuntimeEvent::GoalChanged { goal_id, step: Some(GoalStep::Waiting), .. } if goal_id == goal)
}

fn waiting_reason(events: &[RuntimeEvent], goal: &str) -> Option<String> {
    events.iter().rev().find_map(|e| match e {
        RuntimeEvent::GoalChanged {
            goal_id,
            step: Some(GoalStep::Waiting),
            reason,
            ..
        } if goal_id == goal => reason.clone(),
        _ => None,
    })
}

/// `expected` appears in `changes` in this order, other changes between them allowed.
fn in_order(changes: &[String], expected: &[&str]) -> bool {
    let mut rest = changes.iter();
    expected
        .iter()
        .all(|want| rest.any(|change| change == want))
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

async fn session_log(client: &RpcClient, session_id: &str) -> Vec<SessionLogEntry> {
    client
        .get_session_log(session_id.to_string())
        .await
        .expect("get_session_log")
}

/// Read from the persisted log, as a device that did not start the turn would.
async fn pending_permission(client: &RpcClient, session_id: &str) -> Option<String> {
    session_log(client, session_id)
        .await
        .iter()
        .rev()
        .find_map(|e| match &e.event {
            SessionEvent::PermissionAsk { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
}

async fn pending_question(client: &RpcClient, session_id: &str) -> Option<String> {
    session_log(client, session_id)
        .await
        .iter()
        .rev()
        .find_map(|e| match &e.event {
            SessionEvent::QuestionAsk { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
}

/// One `status/step/iteration` per change of this goal, repeats collapsed.
fn goal_changes(events: &[RuntimeEvent], goal_id: &str) -> Vec<String> {
    let mut changes: Vec<String> = Vec::new();
    for event in events {
        let RuntimeEvent::GoalChanged {
            goal_id: id,
            status,
            step,
            iteration,
            ..
        } = event
        else {
            continue;
        };
        if id != goal_id {
            continue;
        }
        let change = format!(
            "{}/{}/{iteration}",
            status.as_str(),
            step.map_or("-", GoalStep::as_str)
        );
        if changes.last() != Some(&change) {
            changes.push(change);
        }
    }
    changes
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
