//! A running goal outlives its server: a clean stop or a crash leaves the row
//! running, and the next server reaps the harness the last one left behind,
//! relaunches the session once a client's environment is back, and continues
//! where it was cut off. A server that keeps dying on the goal pauses it.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalDetail, GoalIterationNote, GoalLimits, GoalPauseReason, GoalSource,
    GoalSpec, GoalStatus,
};
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, SessionLogEntry, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{clear, control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const GOAL_WAIT: Duration = Duration::from_secs(90);
const GOAL_PROMPT: &str = "hyprlayer:goal";
const CUT_OFF: &str = "Your previous turn was cut off by a server restart.";
const DISPLAY: &str = ":goal-e2e";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_server_restart() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_server_restart", &sandbox);
    let controls = sandbox.root.join("goal-control");
    let (mut server, mut client) = boot(&mut j, &sandbox, None).await;

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let clean = create_workstream(&mut j, &client, &project, "Restart clean").await;
    let clean_ws = text(&clean, "/item/id");
    let clean_tree = PathBuf::from(text(&clean, "/item/worktreePath"));
    let crash = create_workstream(&mut j, &client, &project, "Restart crash").await;
    let ws = text(&crash, "/item/id");

    j.step("a clean restart resumes the goal");
    let s1 = new_session(
        &mut j,
        &client,
        &sandbox,
        &clean_ws,
        "$S1",
        &[("hold", "2")],
    )
    .await;
    let g1 = start(
        &mut j,
        &client,
        &s1,
        spec("E2E_RESTART_CLEAN", r#"test "$(cat counter.txt)" -ge 2"#),
        "$G1",
    )
    .await;
    let held = eventually(IO, || turns(&controls, &s1) == 2).await;
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the server stops cleanly while iteration 1 is held",
        held && stopped.status.success(),
        json!({ "held": held, "exit": stopped.status.code() }),
    );
    (server, client) = boot(&mut j, &sandbox, None).await;
    let done = wait_ended(&client, &g1.id).await;
    let log = session_log(&client, &s1).await;
    let sent = goal_texts(&log);
    j.guard(
        "iteration 1 is closed as cut off",
        note_of(&done, 0) == Some(GoalIterationNote::CutOff),
        json!({ "note1": note_of(&done, 0).map(GoalIterationNote::as_str) }),
    );
    let second_cut_off = sent.get(1).is_some_and(|t| t.starts_with(CUT_OFF));
    j.guard(
        "iteration 2's prompt opens by saying the last turn was cut off",
        sent.len() == 2 && second_cut_off,
        json!({ "goalMessages": sent.len(), "opensWithCutOff": second_cut_off }),
    );
    let relaunched = last_line(&controls.join(&s1).join("argv.log")).contains("--resume");
    j.guard(
        "the session is relaunched with --resume",
        relaunched,
        json!({ "resumed": relaunched }),
    );
    let counter = std::fs::read_to_string(clean_tree.join("counter.txt")).unwrap_or_default();
    j.guard(
        "the goal is met at iteration 2",
        done.goal.status == GoalStatus::Met && done.goal.iteration == 2,
        json!({
            "status": done.goal.status.as_str(),
            "iteration": done.goal.iteration,
            "counter": counter.trim(),
        }),
    );

    j.step("a crash mid-iteration reaps the orphan and resumes");
    let s2 = new_session(
        &mut j,
        &client,
        &sandbox,
        &ws,
        "$S2",
        &[("slow", "2"), ("linger", "")],
    )
    .await;
    let g2 = start(
        &mut j,
        &client,
        &s2,
        spec("E2E_RESTART_CRASH", "true"),
        "$G2",
    )
    .await;
    let in_slow = wait_working(&client, &s2, 1).await;
    let pid = read_pid(&controls, &s2);
    let gone = crash_server(server).await;
    j.guard(
        "the crashed server stops listening while the stub is mid-turn",
        in_slow && gone,
        json!({ "midTurn": in_slow, "gone": gone }),
    );
    (server, client) = boot(&mut j, &sandbox, None).await;
    let reaped = eventually(Duration::from_secs(5), || !alive(&pid)).await;
    j.guard(
        "the lingering stub is gone within 5 s of the restart",
        reaped,
        json!({ "reaped": reaped }),
    );
    let named = sandbox
        .server_log()
        .contains(&format!("reaping goal orphan pid {pid} (session {s2})"));
    j.guard(
        "the server log names the reap of that pid",
        named,
        json!({ "logNamesTheReap": named }),
    );
    let done = wait_ended(&client, &g2.id).await;
    j.guard(
        "the cut-off iteration is closed and a new one is sent",
        done.iterations.len() == 2
            && note_of(&done, 0) == Some(GoalIterationNote::CutOff)
            && done.goal.status == GoalStatus::Met,
        json!({
            "iterations": done.iterations.len(),
            "note1": note_of(&done, 0).map(GoalIterationNote::as_str),
            "status": done.goal.status.as_str(),
        }),
    );

    j.step("a crash mid-check re-runs the checks");
    let s3 = new_session(&mut j, &client, &sandbox, &ws, "$S3", &[]).await;
    let runs = controls.join("mid-check-runs.log");
    let check = format!(
        r#"echo run >> '{0}'; [ "$(wc -l < '{0}')" -gt 1 ] || sleep 20; true"#,
        runs.display()
    );
    let g3 = start(
        &mut j,
        &client,
        &s3,
        spec("E2E_RESTART_CHECK", &check),
        "$G3",
    )
    .await;
    let checking = eventually(IO, || lines(&runs) == 1).await;
    let gone = crash_server(server).await;
    j.guard(
        "the server dies while the check runs",
        checking && gone,
        json!({ "checking": checking, "gone": gone }),
    );
    (server, client) = boot(&mut j, &sandbox, None).await;
    let done = wait_ended(&client, &g3.id).await;
    j.guard(
        "the check runs again and the iteration is not re-sent",
        lines(&runs) == 2
            && done.goal.iteration == 1
            && done.iterations.len() == 1
            && done.goal.status == GoalStatus::Met,
        json!({
            "checkRuns": lines(&runs),
            "iteration": done.goal.iteration,
            "iterations": done.iterations.len(),
            "status": done.goal.status.as_str(),
        }),
    );

    j.step("a clean stop kills the check in flight");
    let s4 = new_session(&mut j, &client, &sandbox, &ws, "$S4", &[]).await;
    let runs = controls.join("clean-stop-runs.log");
    let check_pid = controls.join("check.pid");
    let check = format!(
        r#"echo run >> '{0}'; if [ ! -e '{1}' ]; then echo $$ > '{1}'; sleep 30; fi; true"#,
        runs.display(),
        check_pid.display()
    );
    let g4 = start(
        &mut j,
        &client,
        &s4,
        spec("E2E_RESTART_STOP", &check),
        "$G4",
    )
    .await;
    let checking = eventually(IO, || !read_file(&check_pid).is_empty()).await;
    let shell = read_file(&check_pid);
    let stopped = server.shutdown(&client).await;
    let killed = eventually(Duration::from_secs(2), || !alive(&shell)).await;
    j.guard(
        "the check's shell is gone within 2 s of the shutdown",
        checking && stopped.status.success() && killed,
        json!({ "checking": checking, "exit": stopped.status.code(), "killed": killed }),
    );
    (server, client) = boot(&mut j, &sandbox, None).await;
    let done = wait_ended(&client, &g4.id).await;
    j.guard(
        "after the restart the check runs again",
        lines(&runs) == 2 && done.goal.status == GoalStatus::Met,
        json!({ "checkRuns": lines(&runs), "status": done.goal.status.as_str() }),
    );

    j.step("the resume waits for the overlay");
    let s5 = new_session(&mut j, &client, &sandbox, &ws, "$S5", &[("hold", "2")]).await;
    let g5 = start(&mut j, &client, &s5, spec("E2E_RESTART_ENV", "true"), "$G5").await;
    let held = eventually(IO, || turns(&controls, &s5) == 2).await;
    let stopped = server.shutdown(&client).await;
    (server, client) = boot(&mut j, &sandbox, Some(Duration::from_secs(2))).await;
    let done = wait_ended(&client, &g5.id).await;
    // Only capped launches are the goal's: the cap lift relaunches uncapped after the goal ends, racing this read.
    let argv_log = read_file(&controls.join(&s5).join("argv.log"));
    let env_log = read_file(&controls.join(&s5).join("env.log"));
    let resumes: Vec<&str> = argv_log
        .lines()
        .zip(env_log.lines())
        .filter(|(argv, _)| argv.contains("--max-budget-usd"))
        .map(|(_, env)| env)
        .collect();
    let with_overlay = resumes == [format!("DISPLAY={DISPLAY}").as_str()];
    j.guard(
        "the relaunched stub sees the overlay that arrived 2 s after the restart",
        held && stopped.status.success() && with_overlay,
        json!({
            "held": held,
            "goalLaunches": resumes.len(),
            "relaunchHasTheOverlay": with_overlay,
        }),
    );
    j.guard(
        "the goal then meets",
        done.goal.status == GoalStatus::Met,
        json!({ "status": done.goal.status.as_str() }),
    );

    j.step("a restart loop pauses");
    let s6 = new_session(&mut j, &client, &sandbox, &ws, "$S6", &[]).await;
    control_for(&sandbox, &s6, "slow", "");
    control_for(&sandbox, &s6, "linger", "");
    let g6 = start(
        &mut j,
        &client,
        &s6,
        spec("E2E_RESTART_LOOP", "true"),
        "$G6",
    )
    .await;
    let mut cycles = Vec::new();
    for k in 1..=4 {
        let in_slow = wait_working(&client, &s6, k).await;
        let pid = read_pid(&controls, &s6);
        let gone = crash_server(server).await;
        (server, client) = boot(&mut j, &sandbox, None).await;
        let reaped = eventually(Duration::from_secs(5), || !alive(&pid)).await;
        cycles.push(json!({ "midTurn": in_slow, "gone": gone, "reaped": reaped }));
    }
    let every_cycle = cycles
        .iter()
        .all(|c| c.as_object().is_some_and(|c| c.values().all(|v| v == true)));
    j.guard(
        "four crashes, each mid-iteration, each orphan reaped",
        every_cycle,
        json!({ "cycles": cycles }),
    );
    let done = wait_ended(&client, &g6.id).await;
    let cut_offs = done
        .iterations
        .iter()
        .filter(|i| i.note == Some(GoalIterationNote::CutOff))
        .count();
    j.guard(
        "three resumes each cut off an iteration and sent the next",
        done.iterations.len() == 4 && cut_offs == 3,
        json!({ "iterations": done.iterations.len(), "cutOff": cut_offs }),
    );
    j.guard(
        "the fourth restart pauses the goal as a restart loop",
        done.goal.status == GoalStatus::Paused
            && done.goal.pause_reason == Some(GoalPauseReason::RestartLoop),
        json!({
            "status": done.goal.status.as_str(),
            "pauseReason": done.goal.pause_reason.map(GoalPauseReason::as_str),
            "reason": done.goal.last_reason,
        }),
    );

    j.step("a resume after a restart loop gets a fresh allowance");
    j.call(
        &client,
        Request::ResumeGoal {
            goal_id: g6.id.clone(),
        },
    )
    .await
    .expect("resume_goal");
    let in_slow = wait_working(&client, &s6, 5).await;
    let pid = read_pid(&controls, &s6);
    let gone = crash_server(server).await;
    (server, client) = boot(&mut j, &sandbox, None).await;
    let reaped = eventually(Duration::from_secs(5), || !alive(&pid)).await;
    let resent = wait_working(&client, &s6, 6).await;
    let after = client.get_goal(g6.id.clone()).await.expect("get_goal");
    j.guard(
        "one restart after the user's resume resumes the goal instead of pausing it",
        in_slow && gone && reaped && resent && after.goal.status == GoalStatus::Running,
        json!({
            "midTurn": in_slow,
            "gone": gone,
            "reaped": reaped,
            "nextSent": resent,
            "status": after.goal.status.as_str(),
            "pauseReason": after.goal.pause_reason.map(GoalPauseReason::as_str),
        }),
    );
    clear(&sandbox, &format!("{s6}/linger"));
    j.call(
        &client,
        Request::StopGoal {
            goal_id: g6.id.clone(),
        },
    )
    .await
    .expect("stop_goal");
    wait_ended(&client, &g6.id).await;

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

/// A fresh server and client; the client sends its overlay at once, as the desktop does at connect.
async fn boot(
    j: &mut Journey,
    sandbox: &Sandbox,
    overlay_after: Option<Duration>,
) -> (Server, Arc<RpcClient>) {
    let server = Server::start(sandbox).await;
    let client = server.client();
    if let Some(wait) = overlay_after {
        tokio::time::sleep(wait).await;
    }
    j.call(
        &client,
        Request::SetEnvOverlay {
            vars: BTreeMap::from([("DISPLAY".to_string(), DISPLAY.to_string())]),
        },
    )
    .await
    .expect("set_env_overlay");
    (server, client)
}

/// SIGKILL through `kill_on_drop`, so the server writes nothing on its way out.
async fn crash_server(server: Server) -> bool {
    let socket = server.socket().to_path_buf();
    drop(server);
    let deadline = Instant::now() + IO;
    loop {
        if RpcClient::connect_socket(&socket).await.is_err() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn spec(condition: &str, command: &str) -> GoalSpec {
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
        limits: GoalLimits {
            max_iterations: 10,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 3600,
        },
    }
}

/// A started session whose first turn has finished; `stub_controls` apply from its launch.
async fn new_session(
    j: &mut Journey,
    client: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
    alias: &str,
    stub_controls: &[(&str, &str)],
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

async fn wait_ended(client: &RpcClient, goal_id: &str) -> GoalDetail {
    let deadline = tokio::time::Instant::now() + GOAL_WAIT;
    loop {
        let detail = client
            .get_goal(goal_id.to_string())
            .await
            .expect("get_goal");
        if detail.goal.status != GoalStatus::Running || tokio::time::Instant::now() >= deadline {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Until the stub has answered `count` slow turns with "working", so a kill finds it mid-turn.
async fn wait_working(client: &RpcClient, session_id: &str, count: usize) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    loop {
        let log = session_log(client, session_id).await;
        let working = log
            .iter()
            .filter(|e| matches!(&e.event, SessionEvent::AssistantText { text, .. } if text == "working"))
            .count();
        if working >= count {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
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

fn note_of(detail: &GoalDetail, index: usize) -> Option<GoalIterationNote> {
    detail.iterations.get(index).and_then(|i| i.note)
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_pid(controls: &Path, session_id: &str) -> String {
    read_file(&controls.join(session_id).join("pid"))
}

fn turns(controls: &Path, session_id: &str) -> u32 {
    read_file(&controls.join(session_id).join("turns"))
        .parse()
        .unwrap_or(0)
}

fn last_line(path: &Path) -> String {
    read_file(path)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

fn lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|t| t.lines().count())
        .unwrap_or(0)
}

fn alive(pid: &str) -> bool {
    !pid.is_empty()
        && std::process::Command::new("kill")
            .args(["-0", pid])
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
