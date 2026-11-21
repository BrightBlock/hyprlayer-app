//! A goal survives its harness going down once: a crash mid-iteration relaunches
//! the session and the goal continues, while a second crash with no finished
//! iteration between, two failed turns, or a relaunch that cannot start pause it.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckOutcome, GoalDetail, GoalEndReason, GoalIterationNote, GoalLimits,
    GoalPauseReason, GoalSource, GoalSpec, GoalStatus, GoalStep,
};
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, PermissionMode, SessionEvent, SessionLogEntry, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{control, control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{
    events_until, eventually, json, text, Journey, Request, RuntimeEvent, Sandbox, Server,
};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
const GOAL_PROMPT: &str = "hyprlayer:goal";
const CUT_OFF: &str = "Your previous turn was cut off by a server restart.";

/// Moves the staged harness aside; dropping it puts the harness back, even when a guard panics.
struct Unstaged {
    staged: PathBuf,
    aside: PathBuf,
}

impl Unstaged {
    fn new(staged: &Path) -> Self {
        let aside = staged.with_extension("aside");
        std::fs::rename(staged, &aside).expect("move the staged harness aside");
        Self {
            staged: staged.to_path_buf(),
            aside,
        }
    }
}

impl Drop for Unstaged {
    fn drop(&mut self) {
        let _ = std::fs::rename(&self.aside, &self.staged);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_harness_crash() {
    let sandbox = Sandbox::new();
    let staged = sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_harness_crash", &sandbox);
    let controls = sandbox.root.join("goal-control");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut runtime_events = client.take_events().expect("the client's runtime events");

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Harness crash").await;
    let ws = text(&view, "/item/id");
    j.call(
        &client,
        Request::AnnounceClient {
            client_id: "client-a".into(),
            device: "device-a".into(),
            capabilities: TARGETED_EVENTS.iter().map(|k| k.to_string()).collect(),
        },
    )
    .await
    .expect("announce_client");

    j.step("two failed turns pause the goal");
    let s1 = new_session(&mut j, &client, &ws, "$S1", None).await;
    control_for(&sandbox, &s1, "fail-turn", "");
    let g1 = start(&mut j, &client, &s1, "E2E_CRASH_FAILED_TURNS", "$G1").await;
    let done = wait_goal(&client, &g1.id, |d| d.goal.status != GoalStatus::Running).await;
    j.guard(
        "the goal pauses as turn failures after its second iteration",
        done.goal.status == GoalStatus::Paused
            && done.goal.pause_reason == Some(GoalPauseReason::TurnFailures)
            && done.goal.iteration == 2,
        json!({
            "status": done.goal.status.as_str(),
            "pauseReason": done.goal.pause_reason.map(GoalPauseReason::as_str),
            "iteration": done.goal.iteration,
        }),
    );

    j.step("a crash relaunches and continues");
    let s2 = new_session(
        &mut j,
        &client,
        &ws,
        "$S2",
        Some(PermissionMode::AcceptEdits),
    )
    .await;
    // Stub turn 1 is the session's own; iteration n is turn n + 1.
    control_for(&sandbox, &s2, "crash-once", "2");
    control_for(&sandbox, &s2, "hold", "4");
    let g2 = start(&mut j, &client, &s2, "E2E_CRASH_ONCE", "$G2").await;
    let heard = events_until(&mut runtime_events, GOAL_WAIT, |e| {
        matches!(e, RuntimeEvent::GoalChanged { goal_id, step: Some(GoalStep::Checking), iteration: 2, .. } if *goal_id == g2.id)
    })
    .await;
    let changes = goal_changes(&heard, &g2.id);
    let resumed_then_sent = in_order(
        &changes,
        &[
            "running/working/1",
            "running/resuming/1",
            "running/working/2",
            "running/checking/2",
        ],
    );
    j.guard(
        "the crash moves the goal to resuming, and iteration 2 is sent and checked",
        resumed_then_sent,
        json!({ "changes": changes }),
    );
    let launches = read_file(&controls.join(&s2).join("argv.log"));
    let relaunched = launches.lines().count() == 2
        && launches
            .lines()
            .last()
            .is_some_and(|l| l.contains("--resume"));
    j.guard(
        "the session is relaunched once, with --resume",
        relaunched,
        json!({ "relaunchedWithResume": relaunched }),
    );
    let modes: Vec<&str> = launches
        .lines()
        .filter_map(|l| {
            l.split(' ')
                .skip_while(|t| *t != "--permission-mode")
                .nth(1)
        })
        .collect();
    j.guard(
        "the relaunch keeps the session's permission mode",
        modes == ["acceptEdits", "acceptEdits"],
        json!({ "modes": modes }),
    );
    let after_crash = wait_goal(&client, &g2.id, |d| {
        d.iterations.get(1).is_some_and(|i| i.outcome.is_some())
    })
    .await;
    let crashed_note = note_of(&after_crash, 0);
    let crashed_checked = after_crash.iterations.first().and_then(|i| i.outcome);
    let next_outcome = after_crash.iterations.get(1).and_then(|i| i.outcome);
    j.guard(
        "the crashed iteration closes cut off with no checks, and the next one finishes its checks",
        crashed_note == Some(GoalIterationNote::CutOff)
            && crashed_checked.is_none()
            && next_outcome == Some(GoalCheckOutcome::Fail),
        json!({
            "note1": crashed_note.map(GoalIterationNote::as_str),
            "checked1": crashed_checked.is_some(),
            "outcome2": next_outcome.map(GoalCheckOutcome::as_str),
        }),
    );
    let sent = goal_texts(&session_log(&client, &s2).await);
    let names_restart = sent.get(1).is_some_and(|t| t.starts_with(CUT_OFF));
    j.guard(
        "iteration 2's prompt does not claim a server restart",
        sent.len() >= 2 && !names_restart,
        json!({ "sent2": sent.len() >= 2, "namesARestart": names_restart }),
    );

    j.step("a second crash pauses");
    let held = eventually(IO, || turns(&controls, &s2) == 4).await;
    control_for(&sandbox, &s2, "crash", "");
    control(&sandbox, "release", "");
    let done = wait_goal(&client, &g2.id, |d| d.goal.status != GoalStatus::Running).await;
    let notes: Vec<Option<&str>> = done
        .iterations
        .iter()
        .map(|i| i.note.map(GoalIterationNote::as_str))
        .collect();
    j.guard(
        "iterations 3 and 4 both crash",
        held && notes == [Some("cut_off"), None, Some("cut_off"), Some("cut_off")],
        json!({ "held": held, "notes": notes }),
    );
    // Pausing at 3 would mean iteration 2 finishing did not reset the count.
    j.guard(
        "the second crash in a row pauses the goal at iteration 4",
        done.goal.status == GoalStatus::Paused
            && done.goal.pause_reason == Some(GoalPauseReason::HarnessCrashes)
            && done.goal.iteration == 4,
        json!({
            "status": done.goal.status.as_str(),
            "pauseReason": done.goal.pause_reason.map(GoalPauseReason::as_str),
            "iteration": done.goal.iteration,
        }),
    );

    j.step("a failed relaunch pauses");
    let unstaged = Unstaged::new(&staged);
    j.call(
        &client,
        Request::ResumeGoal {
            goal_id: g2.id.clone(),
        },
    )
    .await
    .expect("resume_goal");
    let failed = wait_goal(&client, &g2.id, |d| {
        d.goal.pause_reason == Some(GoalPauseReason::RelaunchFailed)
    })
    .await;
    drop(unstaged);
    let explained = failed
        .goal
        .last_reason
        .as_deref()
        .is_some_and(|r| !r.is_empty());
    j.guard(
        "the resume cannot relaunch the session and pauses with the error",
        failed.goal.status == GoalStatus::Paused
            && failed.goal.pause_reason == Some(GoalPauseReason::RelaunchFailed)
            && explained
            && failed.goal.iteration == 4,
        json!({
            "status": failed.goal.status.as_str(),
            "pauseReason": failed.goal.pause_reason.map(GoalPauseReason::as_str),
            "hasReason": explained,
            "iteration": failed.goal.iteration,
        }),
    );

    j.step("a resume after crashes does not claim a server restart");
    // `crash` is still set; the resume's fresh crash allowance relaunches after iteration 5, and 6 pauses.
    j.call(
        &client,
        Request::ResumeGoal {
            goal_id: g2.id.clone(),
        },
    )
    .await
    .expect("resume_goal");
    let resumed = wait_goal(&client, &g2.id, |d| d.goal.status != GoalStatus::Running).await;
    let sent = goal_texts(&session_log(&client, &s2).await);
    let fifth = sent.get(4).map(String::as_str).unwrap_or_default();
    j.guard(
        "the iteration a user's resume sends carries no cut-off sentence",
        sent.len() == 6 && !fifth.contains(CUT_OFF) && resumed.goal.iteration == 6,
        json!({
            "goalMessages": sent.len(),
            "namesARestart": fifth.contains(CUT_OFF),
            "iteration": resumed.goal.iteration,
        }),
    );

    j.step("a crash on the last iteration does not exceed the cap");
    let s3 = new_session(&mut j, &client, &ws, "$S3", None).await;
    control_for(&sandbox, &s3, "crash-once", "2");
    let g3 = start_with(
        &mut j,
        &client,
        &s3,
        GoalSpec {
            limits: GoalLimits {
                max_iterations: 1,
                ..spec("E2E_CRASH_AT_CAP").limits
            },
            ..spec("E2E_CRASH_AT_CAP")
        },
        "$G3",
    )
    .await;
    let done = wait_goal(&client, &g3.id, |d| d.goal.status != GoalStatus::Running).await;
    let stdin_log = controls.join(&s3).join("stdin.log");
    // Long enough for a wrongly sent iteration 2 to reach the relaunched stub.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let delivered = goal_lines(&stdin_log, "E2E_CRASH_AT_CAP");
    j.guard(
        "the goal stops at its iteration limit and the stub got one goal message",
        done.goal.status == GoalStatus::Exhausted
            && done.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && done.goal.iteration == 1
            && delivered == 1,
        json!({
            "status": done.goal.status.as_str(),
            "endReason": done.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": done.goal.iteration,
            "goalMessagesDelivered": delivered,
        }),
    );

    j.step("a goal's relaunch leaves the session's requested mode as it was");
    let s4 = new_session(&mut j, &client, &ws, "$S4", None).await;
    control_for(&sandbox, &s4, "crash-once", "2");
    let g4 = start_with(
        &mut j,
        &client,
        &s4,
        GoalSpec {
            limits: GoalLimits {
                max_iterations: 2,
                ..spec("E2E_CRASH_AUTO").limits
            },
            ..spec("E2E_CRASH_AUTO")
        },
        "$G4",
    )
    .await;
    let done = wait_goal(&client, &g4.id, |d| d.goal.status != GoalStatus::Running).await;
    let launches = read_file(&controls.join(&s4).join("argv.log"))
        .lines()
        .count();
    let auto = requested_mode(&mut j, &client, &ws, &s4).await;
    let explicit = requested_mode(&mut j, &client, &ws, &s2).await;
    j.guard(
        "an Auto session the goal relaunched still asks for no mode",
        done.goal.iteration == 2 && launches == 2 && auto.is_null(),
        json!({ "iteration": done.goal.iteration, "launches": launches, "mode": auto }),
    );
    // The resumed goal relaunched it after its harness was gone, with no live mode to copy.
    let s2_modes: Vec<String> = read_file(&controls.join(&s2).join("argv.log"))
        .lines()
        .filter_map(|l| {
            l.split(' ')
                .skip_while(|t| *t != "--permission-mode")
                .nth(1)
                .map(str::to_string)
        })
        .collect();
    j.guard(
        "an Accept edits session the goal relaunched still asks for it, and runs it",
        explicit == "acceptEdits" && s2_modes.iter().all(|m| m == "acceptEdits"),
        json!({ "mode": explicit, "launchedModes": s2_modes }),
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

fn spec(condition: &str) -> GoalSpec {
    GoalSpec {
        condition: condition.into(),
        work_prompt: "Work toward the goal.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![GoalCheck::Command {
            command: "false".into(),
            timeout_secs: 60,
            label: None,
            cwd: None,
            phases: vec![],
        }],
        limits: GoalLimits {
            max_iterations: 10,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 3600,
        },
    }
}

/// A started session whose first turn has finished.
async fn new_session(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    alias: &str,
    permission_mode: Option<PermissionMode>,
) -> String {
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "hello".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode,
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
    condition: &str,
    alias: &str,
) -> Goal {
    start_with(j, client, session_id, spec(condition), alias).await
}

async fn start_with(
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

async fn session_log(client: &RpcClient, session_id: &str) -> Vec<SessionLogEntry> {
    client
        .get_session_log(session_id.to_string())
        .await
        .expect("get_session_log")
}

fn goal_texts(log: &[SessionLogEntry]) -> Vec<String> {
    log.iter()
        .filter_map(|e| match &e.event {
            SessionEvent::UserMessage {
                text,
                prompt_name: Some(name),
                ..
            } if name == GOAL_PROMPT => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Goal messages the stub read, across its relaunches.
fn goal_lines(stdin_log: &Path, condition: &str) -> usize {
    std::fs::read_to_string(stdin_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(r#""type":"user""#) && l.contains(condition))
        .count()
}

fn note_of(detail: &GoalDetail, index: usize) -> Option<GoalIterationNote> {
    detail.iterations.get(index).and_then(|i| i.note)
}

/// The session row's `requestedPermissionMode`; null is Auto.
async fn requested_mode(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    id: &str,
) -> serde_json::Value {
    let sessions = j
        .call(
            client,
            Request::ListSessions {
                workstream_id: ws.to_string(),
            },
        )
        .await
        .expect("list_sessions");
    sessions
        .as_array()
        .and_then(|all| all.iter().find(|s| s["id"] == id))
        .map(|s| s["requestedPermissionMode"].clone())
        .unwrap_or_default()
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn turns(controls: &Path, session_id: &str) -> u32 {
    read_file(&controls.join(session_id).join("turns"))
        .parse()
        .unwrap_or(0)
}

/// `expected` appears in `changes` in this order, other changes between them allowed.
fn in_order(changes: &[String], expected: &[&str]) -> bool {
    let mut rest = changes.iter();
    expected
        .iter()
        .all(|want| rest.any(|change| change == want))
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
