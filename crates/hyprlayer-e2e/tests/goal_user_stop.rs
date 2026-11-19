//! A person stays in charge of a goal: pausing waits for the running iteration,
//! a message or a client restart takes the session back, stopping interrupts and
//! kills only a harness that ignores it, and archiving what a goal runs on ends
//! the goal instead of leaving it to run on nothing (a killed session: goal_budget).
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalDetail, GoalEndReason, GoalIterationNote, GoalLimits, GoalPauseReason,
    GoalSource, GoalSpec, GoalStatus, GoalStep,
};
use hyprlayer_core::runtime::goals::{GoalPatch, GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, SessionLogEntry, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{control, control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
const GOAL_PROMPT: &str = "hyprlayer:goal";
const NEVER: &str = r#"echo "not yet"; false"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_user_stop() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_user_stop", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let other = server.connect().await;
    let controls = sandbox.root.join("goal-control");

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Goal stops").await;
    let ws = text(&view, "/item/id");

    j.step("start on a busy session waits for its turn");
    let busy = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_BUSY",
        &[("hold", "1")],
        false,
    )
    .await;
    let busy_runs = controls.join("busy-runs.log");
    let check = format!("echo run >> '{}'; true", busy_runs.display());
    let goal = start(
        &mut j,
        &client,
        &busy,
        spec("E2E_GOAL_BUSY", &check, 3),
        "$GOAL_BUSY",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let waiting = client.get_goal(goal.id.clone()).await.expect("get_goal");
    let log = session_log(&client, &busy).await;
    j.guard(
        "the goal waits for the user's turn: no message sent, no check run",
        waiting.goal.status == GoalStatus::Running
            && waiting.goal.iteration == 0
            && goal_sent(&log) == 0
            && lines(&busy_runs) == 0,
        json!({
            "iteration": waiting.goal.iteration,
            "goalMessages": goal_sent(&log),
            "checksRun": lines(&busy_runs),
        }),
    );
    control(&sandbox, "release", "");
    let ended = wait_ended(&client, &goal.id).await;
    let log = session_log(&client, &busy).await;
    j.guard(
        "iteration 1 follows that turn's result, and the goal meets there, never at iteration 0",
        ended.goal.status == GoalStatus::Met
            && ended.goal.iteration == 1
            && first_turn_precedes_goal(&log)
            && lines(&busy_runs) == 1,
        json!({
            "status": ended.goal.status.as_str(),
            "iteration": ended.goal.iteration,
            "afterTheTurn": first_turn_precedes_goal(&log),
            "checksRun": lines(&busy_runs),
        }),
    );

    j.step("pause waits for the iteration");
    let pause = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_PAUSE",
        &[("hold", "2")],
        true,
    )
    .await;
    let pause_log = controls.join(&pause).join("stdin.log");
    let goal = start(
        &mut j,
        &client,
        &pause,
        spec("E2E_GOAL_PAUSE", NEVER, 20),
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
    let pausing = wait_goal(&client, &goal.id, |d| {
        d.goal.step == Some(GoalStep::Pausing)
    })
    .await;
    control(&sandbox, "release", "");
    let paused = wait_goal(&client, &goal.id, |d| d.goal.status != GoalStatus::Running).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let later = client.get_goal(goal.id.clone()).await.expect("get_goal");
    j.guard(
        "the goal reads pausing while the turn runs, and nothing interrupts it",
        pausing.goal.step == Some(GoalStep::Pausing) && interrupts(&pause_log) == 0,
        json!({
            "step": pausing.goal.step.map(GoalStep::as_str),
            "interrupts": interrupts(&pause_log),
        }),
    );
    j.guard(
        "it pauses at iteration 1 once the checks have run, and starts no second iteration",
        paused.goal.status == GoalStatus::Paused
            && paused.goal.pause_reason == Some(GoalPauseReason::User)
            && paused.goal.iteration == 1
            && paused.goal.last_outcome.is_some()
            && later.goal.iteration == 1
            && later.goal.status == GoalStatus::Paused,
        json!({
            "status": paused.goal.status.as_str(),
            "pauseReason": paused.goal.pause_reason.map(GoalPauseReason::as_str),
            "iteration": paused.goal.iteration,
            "checked": paused.goal.last_outcome.is_some(),
            "iterationThreeSecondsLater": later.goal.iteration,
        }),
    );

    j.step("resume continues");
    let resumed = act(
        &mut j,
        &client,
        Request::ResumeGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("resume_goal");
    let advanced = wait_sent(&client, &goal.id, 2).await;
    j.guard(
        "the next iteration follows",
        resumed.status == GoalStatus::Running
            && advanced.goal.status == GoalStatus::Running
            && advanced.iterations.len() >= 2,
        json!({
            "resumedStatus": resumed.status.as_str(),
            "status": advanced.goal.status.as_str(),
            "reachedIteration2": advanced.iterations.len() >= 2,
        }),
    );

    j.step("two resumes start one runner");
    act(
        &mut j,
        &client,
        Request::PauseGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("pause_goal");
    let paused = wait_goal(&client, &goal.id, |d| d.goal.status == GoalStatus::Paused).await;
    let at = paused.goal.iteration;
    act(
        &mut j,
        &client,
        Request::UpdateGoal {
            goal_id: goal.id.clone(),
            patch: raise_to(at + 1),
        },
    )
    .await
    .expect("update_goal");
    let before = goal_lines(&pause_log, "E2E_GOAL_PAUSE");
    let (from_client, from_other) = tokio::join!(
        client.resume_goal(goal.id.clone()),
        other.resume_goal(goal.id.clone())
    );
    let ended = wait_ended(&client, &goal.id).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let added = goal_lines(&pause_log, "E2E_GOAL_PAUSE") - before;
    j.guard(
        "both resumes are accepted and the harness gets exactly one more goal message",
        from_client.is_ok() && from_other.is_ok() && added == 1,
        json!({
            "firstAccepted": from_client.is_ok(),
            "secondAccepted": from_other.is_ok(),
            "goalMessagesAdded": added,
        }),
    );
    j.guard(
        "that one iteration ends the goal at the cap",
        ended.goal.status == GoalStatus::Exhausted && ended.goal.iteration == at + 1,
        json!({
            "status": ended.goal.status.as_str(),
            "iterationsAfterPause": ended.goal.iteration - at,
        }),
    );

    j.step("a message during checks re-runs them");
    let checks = new_session(&mut j, &client, &sandbox, &ws, "$SESSION_CHECKS", &[], true).await;
    let runs = controls.join("runs.log");
    let blocked = blocking(&runs, &controls.join("check-release"));
    let goal = start(
        &mut j,
        &client,
        &checks,
        spec("E2E_GOAL_CHECKS", &blocked, 1),
        "$GOAL_CHECKS",
    )
    .await;
    let first_run = eventually(IO, || lines(&runs) >= 1).await;
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: checks.clone(),
            text: "E2E_USER_DURING_CHECKS".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let rerun = eventually(IO, || lines(&runs) >= 2).await;
    let log = session_log(&client, &checks).await;
    j.guard(
        "the checks start again only after the user's turn has finished",
        first_run && rerun && turn_finished_after(&log, "E2E_USER_DURING_CHECKS"),
        json!({
            "firstRun": first_run,
            "rerun": rerun,
            "userTurnFinished": turn_finished_after(&log, "E2E_USER_DURING_CHECKS"),
        }),
    );
    control(&sandbox, "check-release", "");
    let ended = wait_ended(&client, &goal.id).await;
    j.guard(
        "the check ran twice and the message did not count as an iteration",
        lines(&runs) == 2 && ended.goal.iteration == 1 && ended.iterations.len() == 1,
        json!({
            "checksRun": lines(&runs),
            "iteration": ended.goal.iteration,
            "iterations": ended.iterations.len(),
        }),
    );

    j.step("a second goal is refused");
    let occupied = new_session(&mut j, &client, &sandbox, &ws, "$SESSION_SECOND", &[], true).await;
    let live = start(
        &mut j,
        &client,
        &occupied,
        spec("E2E_GOAL_SECOND", NEVER, 50),
        "$GOAL_SECOND",
    )
    .await;
    act(
        &mut j,
        &client,
        Request::PauseGoal {
            goal_id: live.id.clone(),
        },
    )
    .await
    .expect("pause_goal");
    wait_goal(&client, &live.id, |d| d.goal.status == GoalStatus::Paused).await;
    let again = act(
        &mut j,
        &client,
        Request::StartGoal {
            request: request_for(&occupied, spec("E2E_GOAL_AGAIN", NEVER, 3)),
        },
    )
    .await;
    j.guard(
        "a session with a live goal refuses another",
        again.as_ref().err().map(|e| e.code.as_str()) == Some("goal_active"),
        json!({ "code": again.err().map(|e| e.code) }),
    );
    let race = new_session(&mut j, &client, &sandbox, &ws, "$SESSION_RACE", &[], true).await;
    let (x, y) = tokio::join!(
        client.start_goal(request_for(&race, spec("E2E_GOAL_RACE_A", NEVER, 50))),
        other.start_goal(request_for(&race, spec("E2E_GOAL_RACE_B", NEVER, 50)))
    );
    let refused = [&x, &y]
        .into_iter()
        .filter_map(|r| r.as_ref().err().map(|e| e.code.as_str()))
        .collect::<Vec<_>>();
    let winners = [x.is_ok(), y.is_ok()].into_iter().filter(|w| *w).count();
    j.guard(
        "two starts at once on another session: exactly one wins",
        winners == 1 && refused == ["goal_active"],
        json!({ "winners": winners, "refusedWith": refused }),
    );
    if let Some(winner) = x.ok().or(y.ok()) {
        act(
            &mut j,
            &client,
            Request::StopGoal {
                goal_id: winner.id.clone(),
            },
        )
        .await
        .expect("stop_goal");
        wait_goal(&client, &winner.id, |d| {
            d.goal.status == GoalStatus::Stopped
        })
        .await;
    }

    j.step("archive and delete are refused");
    let archive = j
        .call(
            &client,
            Request::SetSessionArchived {
                session_id: occupied.clone(),
                archived: true,
            },
        )
        .await;
    let delete = j
        .call(
            &client,
            Request::DeleteSession {
                session_id: occupied.clone(),
            },
        )
        .await;
    j.guard(
        "a session with a live goal can be neither archived nor deleted",
        archive.as_ref().err().map(|e| e.code.as_str()) == Some("goal_active")
            && delete.as_ref().err().map(|e| e.code.as_str()) == Some("goal_active"),
        json!({
            "archive": archive.err().map(|e| e.code),
            "delete": delete.err().map(|e| e.code),
        }),
    );
    let closed = act(
        &mut j,
        &client,
        Request::StopGoal {
            goal_id: live.id.clone(),
        },
    )
    .await
    .expect("stop_goal");
    let freed = j
        .call(
            &client,
            Request::SetSessionArchived {
                session_id: occupied.clone(),
                archived: true,
            },
        )
        .await;
    j.guard(
        "stopping the paused goal frees the session",
        closed.status == GoalStatus::Stopped && freed.is_ok(),
        json!({
            "status": closed.status.as_str(),
            "archivedAfterStop": freed.is_ok(),
        }),
    );

    j.step("NEEDS-HUMAN pauses the goal");
    let reply = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_REPLY",
        &[("needs-human", "2")],
        true,
    )
    .await;
    let reply_runs = controls.join("reply-runs.log");
    let check = format!("echo run >> '{}'; false", reply_runs.display());
    let goal = start(
        &mut j,
        &client,
        &reply,
        spec("E2E_GOAL_REPLY", &check, 2),
        "$GOAL_REPLY",
    )
    .await;
    let asked = wait_ended(&client, &goal.id).await;
    j.guard(
        "the agent's question pauses the goal after the checks have run",
        asked.goal.status == GoalStatus::Paused
            && asked.goal.pause_reason == Some(GoalPauseReason::NeedsHuman)
            && asked.goal.last_reason.as_deref() == Some("which database?")
            && asked.goal.iteration == 1
            && lines(&reply_runs) == 1,
        json!({
            "status": asked.goal.status.as_str(),
            "pauseReason": asked.goal.pause_reason.map(GoalPauseReason::as_str),
            "reason": asked.goal.last_reason,
            "iteration": asked.goal.iteration,
            "checksRun": lines(&reply_runs),
        }),
    );

    j.step("reply and resume waits for the reply");
    control_for(&sandbox, &reply, "hold", "3");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: reply.clone(),
            text: "E2E_REPLY use sqlite".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let resumed = act(
        &mut j,
        &client,
        Request::ResumeGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("resume_goal");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let waiting = client.get_goal(goal.id.clone()).await.expect("get_goal");
    let log = session_log(&client, &reply).await;
    j.guard(
        "while the reply's turn runs nothing is sent and no check runs",
        resumed.status == GoalStatus::Running
            && waiting.goal.status == GoalStatus::Running
            && goal_sent(&log) == 1
            && lines(&reply_runs) == 1,
        json!({
            "status": waiting.goal.status.as_str(),
            "goalMessages": goal_sent(&log),
            "checksRun": lines(&reply_runs),
        }),
    );
    control(&sandbox, "release", "");
    let ended = wait_ended(&client, &goal.id).await;
    let log = session_log(&client, &reply).await;
    j.guard(
        "the checks run again after the reply, then the next iteration follows",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.iteration == 2
            && goal_sent(&log) == 2
            && lines(&reply_runs) == 3,
        json!({
            "status": ended.goal.status.as_str(),
            "iteration": ended.goal.iteration,
            "goalMessages": goal_sent(&log),
            "checksRun": lines(&reply_runs),
        }),
    );

    j.step("take over pauses the goal");
    let takeover = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_TAKEOVER",
        &[("slow", "2")],
        true,
    )
    .await;
    let takeover_log = controls.join(&takeover).join("stdin.log");
    let goal = start(
        &mut j,
        &client,
        &takeover,
        spec("E2E_GOAL_TAKEOVER", NEVER, 20),
        "$GOAL_TAKEOVER",
    )
    .await;
    wait_sent(&client, &goal.id, 1).await;
    j.call(
        &client,
        Request::InterruptSession {
            session_id: takeover.clone(),
        },
    )
    .await
    .expect("interrupt_session");
    let ended = wait_ended(&client, &goal.id).await;
    j.guard(
        "the user's interrupt pauses the goal and marks the iteration interrupted",
        ended.goal.status == GoalStatus::Paused
            && ended.goal.pause_reason == Some(GoalPauseReason::User)
            && note_of(&ended, 0) == Some(GoalIterationNote::Interrupted)
            && interrupts(&takeover_log) == 1,
        json!({
            "status": ended.goal.status.as_str(),
            "pauseReason": ended.goal.pause_reason.map(GoalPauseReason::as_str),
            "note": note_of(&ended, 0).map(GoalIterationNote::as_str),
            "interrupts": interrupts(&takeover_log),
        }),
    );

    j.step("a client restart pauses the goal");
    let restart = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_RESTART",
        &[("slow", "2")],
        true,
    )
    .await;
    let goal = start(
        &mut j,
        &client,
        &restart,
        spec("E2E_GOAL_RESTART", NEVER, 20),
        "$GOAL_RESTART",
    )
    .await;
    wait_sent(&client, &goal.id, 1).await;
    let restarted = client
        .restart_session_stream(restart.clone(), None, None, None)
        .await
        .expect("restart_session");
    let ended = wait_ended(&client, &goal.id).await;
    drop(restarted);
    j.guard(
        "restarting the session pauses the goal and marks the iteration interrupted",
        ended.goal.status == GoalStatus::Paused
            && ended.goal.pause_reason == Some(GoalPauseReason::User)
            && note_of(&ended, 0) == Some(GoalIterationNote::Interrupted),
        json!({
            "status": ended.goal.status.as_str(),
            "pauseReason": ended.goal.pause_reason.map(GoalPauseReason::as_str),
            "note": note_of(&ended, 0).map(GoalIterationNote::as_str),
        }),
    );

    j.step("stop interrupts");
    let stop = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_STOP",
        &[("slow", "2")],
        true,
    )
    .await;
    let stop_dir = controls.join(&stop);
    let goal = start(
        &mut j,
        &client,
        &stop,
        spec("E2E_GOAL_STOP", NEVER, 20),
        "$GOAL_STOP",
    )
    .await;
    wait_sent(&client, &goal.id, 1).await;
    let stopping = act(
        &mut j,
        &client,
        Request::StopGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("stop_goal");
    let ended = wait_ended(&client, &goal.id).await;
    j.guard(
        "stopping answers stopping, then ends the goal as stopped by the user",
        stopping.step == Some(GoalStep::Stopping)
            && ended.goal.status == GoalStatus::Stopped
            && ended.goal.end_reason == Some(GoalEndReason::StoppedByUser),
        json!({
            "answeredStep": stopping.step.map(GoalStep::as_str),
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
        }),
    );
    j.guard(
        "the harness got one interrupt and is still running",
        interrupts(&stop_dir.join("stdin.log")) == 1 && alive(&stop_dir.join("pid")),
        json!({
            "interrupts": interrupts(&stop_dir.join("stdin.log")),
            "alive": alive(&stop_dir.join("pid")),
        }),
    );

    j.step("stop kills a harness that ignores the interrupt");
    let deaf = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_DEAF",
        &[("slow", "2"), ("deaf", "2")],
        true,
    )
    .await;
    let deaf_dir = controls.join(&deaf);
    let goal = start(
        &mut j,
        &client,
        &deaf,
        spec("E2E_GOAL_DEAF", NEVER, 20),
        "$GOAL_DEAF",
    )
    .await;
    wait_sent(&client, &goal.id, 1).await;
    let asked = Instant::now();
    act(
        &mut j,
        &client,
        Request::StopGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await
    .expect("stop_goal");
    let ended = wait_ended(&client, &goal.id).await;
    let waited = asked.elapsed();
    let gone = eventually(IO, || !alive(&deaf_dir.join("pid"))).await;
    j.guard(
        "the goal ends stopped by the user",
        ended.goal.status == GoalStatus::Stopped
            && ended.goal.end_reason == Some(GoalEndReason::StoppedByUser),
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
        }),
    );
    j.guard(
        "the harness is killed only after the grace period",
        waited >= Duration::from_secs(9) && gone,
        json!({ "waitedForTheGrace": waited >= Duration::from_secs(9), "harnessGone": gone }),
    );
    j.guard(
        "it was asked once and never sent anything after",
        interrupts(&deaf_dir.join("stdin.log")) == 1
            && last_line_is_interrupt(&deaf_dir.join("stdin.log")),
        json!({
            "interrupts": interrupts(&deaf_dir.join("stdin.log")),
            "interruptWasLast": last_line_is_interrupt(&deaf_dir.join("stdin.log")),
        }),
    );

    j.step("archiving the workstream stops its goal");
    let view = create_workstream(&mut j, &client, &project, "Goal archived").await;
    let archived_ws = text(&view, "/item/id");
    let archived = new_session(
        &mut j,
        &client,
        &sandbox,
        &archived_ws,
        "$SESSION_ARCHIVE",
        &[],
        true,
    )
    .await;
    let archive_runs = controls.join("archive-runs.log");
    let check_pid = controls.join("archive-check.pid");
    let blocked = format!(
        "echo $$ > '{}'; {}",
        check_pid.display(),
        blocking(&archive_runs, &controls.join("archive-release"))
    );
    let goal = start(
        &mut j,
        &client,
        &archived,
        spec("E2E_GOAL_ARCHIVE", &blocked, 5),
        "$GOAL_ARCHIVE",
    )
    .await;
    let running = eventually(IO, || lines(&archive_runs) >= 1).await;
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: archived_ws.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let ended = wait_ended(&client, &goal.id).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    j.guard(
        "the goal ends stopped with the workstream, never paused",
        running
            && ended.goal.status == GoalStatus::Stopped
            && ended.goal.end_reason == Some(GoalEndReason::WorkstreamArchived),
        json!({
            "checkWasRunning": running,
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
        }),
    );
    // A missing pid file would read as a dead process, so the pid must have been recorded.
    let pid_recorded = std::fs::read_to_string(&check_pid).is_ok_and(|pid| !pid.trim().is_empty());
    let check_killed =
        pid_recorded && eventually(Duration::from_secs(2), || !alive(&check_pid)).await;
    j.guard(
        "the check in flight was killed and not run again",
        check_killed && lines(&archive_runs) == 1,
        json!({
            "checkKilled": check_killed,
            "checksRun": lines(&archive_runs),
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

fn limits(max_iterations: u32) -> GoalLimits {
    GoalLimits {
        max_iterations,
        max_cost_usd: Some(10.0),
        max_tokens: None,
        max_active_secs: 3600,
    }
}

fn spec(condition: &str, command: &str, max_iterations: u32) -> GoalSpec {
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
            cwd: None,
            phases: vec![],
        }],
        limits: limits(max_iterations),
    }
}

fn request_for(session_id: &str, spec: GoalSpec) -> StartGoalRequest {
    StartGoalRequest {
        target: GoalTarget::Session {
            session_id: session_id.into(),
        },
        spec,
    }
}

fn raise_to(max_iterations: u32) -> GoalPatch {
    GoalPatch {
        condition: None,
        work_prompt: None,
        checks: None,
        limits: Some(limits(max_iterations)),
    }
}

/// Appends a line to `runs`, then waits up to 20 s for `release` before failing.
fn blocking(runs: &Path, release: &Path) -> String {
    format!(
        r#"echo run >> '{}'; i=0; while [ ! -e '{}' ] && [ "$i" -lt 200 ]; do sleep 0.1; i=$((i + 1)); done; false"#,
        runs.display(),
        release.display()
    )
}

/// A started session; `finished` waits for its first turn, which a `hold` control may keep open.
async fn new_session(
    j: &mut Journey,
    client: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
    alias: &str,
    stub_controls: &[(&str, &str)],
    finished: bool,
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
    for (name, contents) in stub_controls {
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
    if finished {
        recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    }
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
            request: request_for(session_id, spec),
        },
    )
    .await
    .expect("start_goal");
    j.alias(&goal.id, alias);
    goal
}

async fn act(j: &mut Journey, client: &RpcClient, request: Request) -> Result<Goal, WireError> {
    j.call(client, request)
        .await
        .map(|value| serde_json::from_value(value).expect("the command answers a goal"))
}

/// Until the goal has sent its `n`th iteration, which a `slow` or `hold` turn then keeps open.
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

fn is_goal_message(event: &SessionEvent) -> bool {
    matches!(event, SessionEvent::UserMessage { prompt_name: Some(name), .. } if name == GOAL_PROMPT)
}

fn goal_sent(log: &[SessionLogEntry]) -> usize {
    log.iter().filter(|e| is_goal_message(&e.event)).count()
}

fn first_turn_precedes_goal(log: &[SessionLogEntry]) -> bool {
    let turn_done = log
        .iter()
        .position(|e| matches!(e.event, SessionEvent::TurnDone { .. }));
    let goal = log.iter().position(|e| is_goal_message(&e.event));
    matches!((turn_done, goal), (Some(t), Some(g)) if t < g)
}

/// Whether the user message containing `needle` has a `TurnDone` after it.
fn turn_finished_after(log: &[SessionLogEntry], needle: &str) -> bool {
    let sent = log.iter().position(
        |e| matches!(&e.event, SessionEvent::UserMessage { text, .. } if text.contains(needle)),
    );
    sent.is_some_and(|at| {
        log[at..]
            .iter()
            .any(|e| matches!(e.event, SessionEvent::TurnDone { .. }))
    })
}

fn note_of(detail: &GoalDetail, index: usize) -> Option<GoalIterationNote> {
    detail.iterations.get(index).and_then(|i| i.note)
}

fn lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|t| t.lines().count())
        .unwrap_or(0)
}

/// The harness's stdin lines that are user messages carrying `condition`: every goal message does.
fn goal_lines(stdin_log: &Path, condition: &str) -> usize {
    std::fs::read_to_string(stdin_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(r#""type":"user""#) && l.contains(condition))
        .count()
}

fn interrupts(stdin_log: &Path) -> usize {
    std::fs::read_to_string(stdin_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(r#""subtype":"interrupt""#))
        .count()
}

fn last_line_is_interrupt(stdin_log: &Path) -> bool {
    std::fs::read_to_string(stdin_log)
        .unwrap_or_default()
        .lines()
        .last()
        .is_some_and(|l| l.contains(r#""subtype":"interrupt""#))
}

fn alive(pid_file: &Path) -> bool {
    let pid = std::fs::read_to_string(pid_file).unwrap_or_default();
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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
