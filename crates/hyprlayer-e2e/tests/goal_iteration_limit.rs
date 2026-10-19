//! The limits of a goal: a spec the server cannot run is refused before anything
//! changes, the iteration cap ends a goal and can be raised, a goal that makes no
//! progress fails instead of looping, and an agent that commits every iteration
//! is not mistaken for one.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckOutcome, GoalDetail, GoalEndReason, GoalLimits, GoalSource, GoalSpec,
    GoalStatus,
};
use hyprlayer_core::runtime::goals::{GoalPatch, GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
const NEVER: &str = r#"echo "not yet"; false"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_iteration_limit() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_iteration_limit", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Goal limits").await;
    let ws = text(&view, "/item/id");
    let worktree = PathBuf::from(text(&view, "/item/worktreePath"));

    j.step("invalid specs are refused");
    let invalid = new_session(&mut j, &client, &sandbox, &ws, "$SESSION_INVALID", &[]).await;
    let refused = [
        ("no checks", edit(&invalid, |s| s.checks.clear())),
        (
            "no iterations",
            edit(&invalid, |s| s.limits.max_iterations = 0),
        ),
        (
            "under a minute of active time",
            edit(&invalid, |s| s.limits.max_active_secs = 30),
        ),
        (
            "Claude without a spend cap",
            edit(&invalid, |s| s.limits.max_cost_usd = None),
        ),
        (
            "a new Plan session",
            StartGoalRequest {
                target: GoalTarget::NewSession {
                    workstream_id: ws.clone(),
                    stage: Stage::Plan,
                    harness: Some(Harness::Claude),
                    model: None,
                    effort: None,
                    permission_mode: None,
                    attached_docs: vec![],
                },
                spec: spec("E2E_GOAL_INVALID", "true", 3),
            },
        ),
    ];
    let mut codes = Vec::new();
    for (label, request) in refused {
        let reply = j.call(&client, Request::StartGoal { request }).await;
        codes.push(json!({ "case": label, "code": reply.err().map(|e| e.code) }));
    }
    j.guard(
        "every one is goal_invalid",
        codes.iter().all(|c| c["code"] == "goal_invalid"),
        json!({ "refused": codes }),
    );
    let listed = client
        .list_goals(Some(ws.clone()), false)
        .await
        .expect("list_goals");
    j.guard(
        "none of them left a goal behind",
        listed.is_empty(),
        json!({ "goals": listed.len() }),
    );
    let before = client
        .list_sessions(ws.clone())
        .await
        .expect("list_sessions")
        .len();
    // An unsupported OpenCode provider fails at launch, after the session row exists.
    let unlaunched = j
        .call(
            &client,
            Request::StartGoal {
                request: StartGoalRequest {
                    target: GoalTarget::NewSession {
                        workstream_id: ws.clone(),
                        stage: Stage::General,
                        harness: Some(Harness::Opencode),
                        model: Some("github-copilot/gpt-4o".into()),
                        effort: None,
                        permission_mode: None,
                        attached_docs: vec![],
                    },
                    spec: spec("E2E_GOAL_UNLAUNCHED", "true", 3),
                },
            },
        )
        .await;
    let after = client
        .list_sessions(ws.clone())
        .await
        .expect("list_sessions")
        .len();
    j.guard(
        "a new session that cannot launch is refused and leaves no session behind",
        unlaunched.is_err() && after == before,
        json!({ "refused": unlaunched.is_err(), "sessionsAdded": after.saturating_sub(before) }),
    );

    j.step("the iteration cap ends the goal");
    let capped = new_session(&mut j, &client, &sandbox, &ws, "$SESSION_CAP", &[]).await;
    let goal = start(
        &mut j,
        &client,
        &capped,
        spec("E2E_GOAL_CAP", NEVER, 3),
        "$GOAL_CAP",
    )
    .await;
    let ended = wait_ended(&client, &goal.id).await;
    j.guard(
        "it ends exhausted on the iteration limit after three iterations",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && ended.goal.iteration == 3
            && ended.iterations.len() == 3,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
            "iterations": ended.iterations.len(),
        }),
    );
    j.guard(
        "its checks failed every time",
        ended
            .iterations
            .iter()
            .all(|i| i.outcome == Some(GoalCheckOutcome::Fail)),
        json!({ "outcomes": outcomes(&ended) }),
    );

    j.step("extend and resume");
    let raised = act(
        &mut j,
        &client,
        Request::UpdateGoal {
            goal_id: goal.id.clone(),
            patch: raise_to(4),
        },
    )
    .await
    .expect("update_goal");
    j.guard(
        "raising the limit changes the cap and resumes nothing",
        raised.status == GoalStatus::Exhausted && raised.limits.max_iterations == 4,
        json!({
            "status": raised.status.as_str(),
            "maxIterations": raised.limits.max_iterations,
        }),
    );
    let resumed = act(
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
        "resuming runs one more iteration and stops at the new cap",
        resumed.status == GoalStatus::Running
            && ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && ended.goal.iteration == 4
            && ended.iterations.len() == 4,
        json!({
            "resumedStatus": resumed.status.as_str(),
            "status": ended.goal.status.as_str(),
            "iteration": ended.goal.iteration,
            "iterations": ended.iterations.len(),
        }),
    );

    j.step("extend refuses while another goal is live");
    let other = start(
        &mut j,
        &client,
        &capped,
        spec("E2E_GOAL_LIVE", NEVER, 50),
        "$GOAL_LIVE",
    )
    .await;
    act(
        &mut j,
        &client,
        Request::PauseGoal {
            goal_id: other.id.clone(),
        },
    )
    .await
    .expect("pause_goal");
    let paused = wait_goal(&client, &other.id, |d| d.goal.status != GoalStatus::Running).await;
    act(
        &mut j,
        &client,
        Request::UpdateGoal {
            goal_id: goal.id.clone(),
            patch: raise_to(5),
        },
    )
    .await
    .expect("update_goal while another goal is live");
    let refused = act(
        &mut j,
        &client,
        Request::ResumeGoal {
            goal_id: goal.id.clone(),
        },
    )
    .await;
    let still = client.get_goal(goal.id.clone()).await.expect("get_goal");
    j.guard(
        "the first goal cannot resume past the live one",
        paused.goal.status == GoalStatus::Paused
            && refused.as_ref().err().map(|e| e.code.as_str()) == Some("goal_active")
            && still.goal.status == GoalStatus::Exhausted,
        json!({
            "liveGoal": paused.goal.status.as_str(),
            "code": refused.err().map(|e| e.code),
            "firstGoal": still.goal.status.as_str(),
        }),
    );
    act(
        &mut j,
        &client,
        Request::StopGoal {
            goal_id: other.id.clone(),
        },
    )
    .await
    .expect("stop_goal");

    j.step("a stalled goal fails");
    let stalled = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_STALL",
        &[("idle-turn", "")],
    )
    .await;
    let goal = start(
        &mut j,
        &client,
        &stalled,
        spec("E2E_GOAL_STALL", NEVER, 10),
        "$GOAL_STALL",
    )
    .await;
    let ended = wait_ended(&client, &goal.id).await;
    j.guard(
        "three iterations that change nothing end failed as stalled",
        ended.goal.status == GoalStatus::Failed
            && ended.goal.end_reason == Some(GoalEndReason::Stalled)
            && ended.goal.iteration == 3,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
        }),
    );

    j.step("a committing agent is not stalled");
    let committing = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$SESSION_COMMIT",
        &[("commit", "")],
    )
    .await;
    let before = commits(&sandbox, &worktree);
    // Earlier steps leave an untracked counter.txt here, so the diff is constant rather than empty.
    let diff_before = diff_of(&worktree);
    let goal = start(
        &mut j,
        &client,
        &committing,
        spec("E2E_GOAL_COMMIT", NEVER, 4),
        "$GOAL_COMMIT",
    )
    .await;
    let ended = wait_ended(&client, &goal.id).await;
    let added = commits(&sandbox, &worktree) - before;
    j.guard(
        "it runs to the iteration limit and is never failed as stalled",
        ended.goal.status == GoalStatus::Exhausted
            && ended.goal.end_reason == Some(GoalEndReason::IterationLimit)
            && ended.goal.iteration == 4,
        json!({
            "status": ended.goal.status.as_str(),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": ended.goal.iteration,
        }),
    );
    j.guard(
        "HEAD advanced on every iteration",
        added == 4,
        json!({ "commitsAdded": added }),
    );
    let hashes: std::collections::HashSet<&str> = ended
        .iterations
        .iter()
        .filter_map(|iteration| iteration.progress_hash.as_deref())
        .collect();
    let diff_unchanged = diff_before.is_some() && diff_of(&worktree) == diff_before;
    j.guard(
        "the diff never changes, yet the progress hash moves on every iteration",
        diff_unchanged && hashes.len() == 4,
        json!({
            "diffUnchanged": diff_unchanged,
            "distinctProgressHashes": hashes.len(),
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
        }],
        limits: limits(max_iterations),
    }
}

/// A request for a session's goal with one thing wrong with its spec.
fn edit(session_id: &str, change: impl FnOnce(&mut GoalSpec)) -> StartGoalRequest {
    let mut spec = spec("E2E_GOAL_INVALID", "true", 3);
    change(&mut spec);
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

async fn act(j: &mut Journey, client: &RpcClient, request: Request) -> Result<Goal, WireError> {
    j.call(client, request)
        .await
        .map(|value| serde_json::from_value(value).expect("the command answers a goal"))
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

fn outcomes(detail: &GoalDetail) -> Vec<Option<&'static str>> {
    detail
        .iterations
        .iter()
        .map(|i| i.outcome.map(GoalCheckOutcome::as_str))
        .collect()
}

fn commits(sandbox: &Sandbox, worktree: &Path) -> u32 {
    sandbox
        .git(worktree, &["rev-list", "--count", "HEAD"])
        .parse()
        .expect("a commit count")
}

/// The diff exactly as the runner hashes it.
fn diff_of(worktree: &Path) -> Option<Vec<(String, String, String)>> {
    let files = hyprlayer_core::gitops::worktree_diff(worktree).ok()?;
    Some(
        files
            .into_iter()
            .map(|file| (file.path, file.change, file.patch))
            .collect(),
    )
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
