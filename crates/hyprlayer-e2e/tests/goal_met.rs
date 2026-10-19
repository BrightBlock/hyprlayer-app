//! A goal drives one session through iterations until its checks pass: a native
//! Stop-hook loop or an extra empty result inside a turn is still one iteration,
//! the device that started it may leave, and only clients that announced
//! `GoalChanged` hear about it.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckOutcome, GoalDetail, GoalEndReason, GoalLimits, GoalSource, GoalSpec,
    GoalStatus, GoalStep,
};
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
const GOAL_PROMPT: &str = "hyprlayer:goal";

/// A connection with its own runtime and thread, so it can really leave:
/// dropping an `RpcClient` alone leaves its reader holding the socket open.
struct Departing {
    client: Arc<RpcClient>,
    leave: std::sync::mpsc::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl Departing {
    fn connect(socket: PathBuf) -> Self {
        let (client_tx, client_rx) = std::sync::mpsc::channel();
        let (leave, left) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("build the departing client's runtime");
            let client = runtime
                .block_on(RpcClient::connect_socket(&socket))
                .expect("connect the departing client");
            client_tx.send(client).expect("hand the client back");
            let _ = left.recv();
            runtime.shutdown_timeout(Duration::from_secs(5));
        });
        let client = client_rx.recv().expect("the departing client connects");
        Self {
            client,
            leave,
            thread,
        }
    }

    async fn leave(self) {
        let Self {
            client,
            leave,
            thread,
        } = self;
        drop(client);
        let _ = leave.send(());
        tokio::task::spawn_blocking(move || thread.join().expect("the departing client's thread"))
            .await
            .expect("join the departing client");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_met() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("goal_met", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let socket = server.socket().to_path_buf();
    let a = tokio::task::spawn_blocking(move || Departing::connect(socket))
        .await
        .expect("connect A");
    let b = server.connect().await;
    let mut b_events = b.take_events().expect("B's runtime events");
    let mut c_events = c.take_events().expect("C's runtime events");

    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Goal met").await;
    let ws = text(&view, "/item/id");

    let session_kinds: Vec<&str> = TARGETED_EVENTS
        .into_iter()
        .filter(|kind| *kind != "GoalChanged")
        .collect();
    j.call(&c, announce("client-c", "device-c", &TARGETED_EVENTS))
        .await
        .expect("C announces everything");
    j.call(&b, announce("client-b", "device-b", &session_kinds))
        .await
        .expect("B announces the session kinds only");
    j.call(
        &a.client,
        announce("client-a", "device-a", &TARGETED_EVENTS),
    )
    .await
    .expect("A announces everything");

    let (session, _prompt, _stream_id, mut events) = c
        .start_stage_session_stream(
            ws.clone(),
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
    control_for(&sandbox, &sid, "idle-turn", "1");
    control_for(&sandbox, &sid, "stop-hook", "3");
    control_for(&sandbox, &sid, "bg-result", "3");
    j.call(
        &c,
        Request::LaunchAndConverse {
            session_id: sid.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    drop(events);

    j.step("start a goal on a live session");
    let started = j
        .call(
            &a.client,
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
    a.leave().await;
    let author = goal.author.as_ref().map(|a| a.client_id.clone());
    j.guard(
        "the goal starts running, attributed to client-a",
        goal.status == GoalStatus::Running && author.as_deref() == Some("client-a"),
        json!({ "status": goal.status.as_str(), "author": author }),
    );
    let brief = session_goal(&c, &ws, &sid).await;
    j.guard(
        "the session carries the goal's brief while it runs",
        brief == Some(GoalStatus::Running),
        json!({ "status": brief.map(GoalStatus::as_str) }),
    );

    let detail = wait_goal(&c, &goal.id, |d| d.goal.status != GoalStatus::Running).await;
    let log = c
        .get_session_log(sid.clone())
        .await
        .expect("get_session_log");
    let sent = goal_messages(&log);

    j.step("iterations end at the right boundary");
    j.guard(
        "each iteration is one message under the goal prompt name",
        sent.len() == 3,
        json!({ "goalMessages": sent.len() }),
    );
    let hook_texts = log
        .iter()
        .filter(|e| {
            matches!(&e.event, SessionEvent::AssistantText { text, injected: true, .. }
                if text.starts_with("Stop hook feedback:"))
        })
        .count();
    j.guard(
        "the native Stop-hook feedback is one injected text",
        hook_texts == 1,
        json!({ "injectedTexts": hook_texts }),
    );
    let turn_dones_in_second = match (sent.get(1), sent.get(2)) {
        (Some(second), Some(third)) => log[second.at..third.at]
            .iter()
            .filter(|e| matches!(e.event, SessionEvent::TurnDone { .. }))
            .count(),
        _ => 0,
    };
    j.guard(
        "the extra empty result lands inside the second iteration",
        turn_dones_in_second == 2 && detail.iterations.len() == 3,
        json!({
            "turnDonesInIteration2": turn_dones_in_second,
            "iterations": detail.iterations.len(),
        }),
    );

    j.step("the goal meets its condition");
    let outcomes: Vec<Option<&str>> = detail
        .iterations
        .iter()
        .map(|i| i.outcome.map(GoalCheckOutcome::as_str))
        .collect();
    j.guard(
        "it ends met on its checks at the third iteration",
        detail.goal.status == GoalStatus::Met
            && detail.goal.end_reason == Some(GoalEndReason::ChecksPassed)
            && detail.goal.iteration == 3,
        json!({
            "status": detail.goal.status.as_str(),
            "endReason": detail.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": detail.goal.iteration,
        }),
    );
    j.guard(
        "the checks fail twice, then pass",
        outcomes == [Some("fail"), Some("fail"), Some("pass")],
        json!({ "outcomes": outcomes }),
    );
    let second = sent.get(1).map(|s| s.text.as_str()).unwrap_or_default();
    j.guard(
        "the second iteration carries the first check's output",
        second.contains("did not pass") && second.contains("counter is 1"),
        json!({
            "saysDidNotPass": second.contains("did not pass"),
            "carriesOutput": second.contains("counter is 1"),
        }),
    );
    let spent_cents = (detail.goal.spent_cost_usd * 100.0).round() as i64;
    j.guard(
        "spend counts the goal's own turns, not the session's earlier one",
        spent_cents == 120,
        json!({ "spentCents": spent_cents }),
    );
    let seen = events_until(&mut c_events, IO, |e| {
        matches!(
            e,
            RuntimeEvent::GoalChanged {
                status: GoalStatus::Met,
                ..
            }
        )
    })
    .await;
    let changes = goal_changes(&seen, &goal.id);
    let checked_each = (1..=3).all(|n| changes.contains(&format!("running/checking/{n}")));
    j.guard(
        "C hears the goal start, check every iteration and meet",
        changes.first().map(String::as_str) == Some("running/working/0")
            && checked_each
            && changes.last().is_some_and(|c| c.starts_with("met/")),
        json!({ "changes": changes }),
    );
    let brief_after = session_goal(&c, &ws, &sid).await;
    j.guard(
        "the brief is gone once the goal has ended",
        brief_after.is_none(),
        json!({ "status": brief_after.map(GoalStatus::as_str) }),
    );

    j.step("an unannounced client hears nothing");
    let b_seen = events_until(&mut b_events, Duration::from_millis(500), |_| false).await;
    let b_goal_events = b_seen
        .iter()
        .filter(|e| matches!(e, RuntimeEvent::GoalChanged { .. }))
        .count();
    let b_live = b_seen
        .iter()
        .any(|e| matches!(e, RuntimeEvent::SessionLifecycle { .. }));
    j.guard(
        "B, which never announced GoalChanged, still hears the session but no goal event",
        b_goal_events == 0 && b_live,
        json!({ "goalEvents": b_goal_events, "heardTheSession": b_live }),
    );

    j.step("iterations stay attributed to the starter");
    let authors: Vec<Option<&str>> = sent.iter().map(|s| s.author.as_deref()).collect();
    j.guard(
        "every goal message is client-a's, though client-a left",
        authors.len() == 3 && authors.iter().all(|a| *a == Some("client-a")),
        json!({ "authors": authors }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn spec() -> GoalSpec {
    GoalSpec {
        condition: "E2E_GOAL_MET".into(),
        work_prompt: "Raise the counter to 3.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![GoalCheck::Command {
            command: r#"c=$(cat counter.txt); echo "counter is $c"; test "$c" = 3"#.into(),
            timeout_secs: 60,
            label: None,
        }],
        limits: GoalLimits {
            max_iterations: 6,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 3600,
        },
    }
}

fn announce(client_id: &str, device: &str, capabilities: &[&str]) -> Request {
    Request::AnnounceClient {
        client_id: client_id.into(),
        device: device.into(),
        capabilities: capabilities.iter().map(|k| k.to_string()).collect(),
    }
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

async fn session_goal(client: &RpcClient, ws: &str, session_id: &str) -> Option<GoalStatus> {
    client
        .list_sessions(ws.to_string())
        .await
        .expect("list_sessions")
        .into_iter()
        .find(|s| s.id == session_id)
        .and_then(|s| s.goal)
        .map(|brief| brief.status)
}

struct Sent {
    at: usize,
    text: String,
    author: Option<String>,
}

/// The log's user messages sent under the goal prompt name, with their row index.
fn goal_messages(log: &[SessionLogEntry]) -> Vec<Sent> {
    log.iter()
        .enumerate()
        .filter_map(|(at, entry)| match &entry.event {
            SessionEvent::UserMessage {
                text,
                prompt_name: Some(name),
                author,
                ..
            } if name == GOAL_PROMPT => Some(Sent {
                at,
                text: text.clone(),
                author: author.as_ref().map(|a| a.client_id.clone()),
            }),
            _ => None,
        })
        .collect()
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
