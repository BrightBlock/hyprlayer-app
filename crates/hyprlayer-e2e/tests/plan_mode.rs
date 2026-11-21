//! A harness that leaves plan mode on its own: claude's approved ExitPlanMode
//! reaches the client as a mode change, stays out of the stored log, and the
//! server adopts it so a resume in the new mode does not relaunch. A resume
//! never relaunches; restart does.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, PermissionMode, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Logs each session spawn's `--permission-mode` to `<sandbox>/spawns.log`
/// (the binary probe passes none, so it is not logged). An
/// `EXIT_PLAN` turn asks to run ExitPlanMode and, once allowed, sends the
/// `status` line claude 2.1.281 sends at that moment, verbatim. An in-place
/// `set_permission_mode` is answered with a `status` line naming the new mode.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
mode=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    --resume) sid="$2"; shift 2 ;;
    --permission-mode) mode="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$mode" ] && printf 'spawn %s\n' "$mode" >> "$(dirname "$0")/../spawns.log"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","permissionMode":"%s","tools":[]}\n' "$sid" "$mode"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
    *set_permission_mode*)
      to=$(printf '%s' "$line" | sed -n 's/.*"mode":"\([A-Za-z]*\)".*/\1/p')
      printf '{"type":"system","subtype":"status","status":null,"permissionMode":"%s","session_id":"%s"}\n' "$to" "$sid"
      continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *EXIT_PLAN*)
      printf '{"type":"control_request","request_id":"plan-%s","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","input":{"plan":"Create hello.txt"},"tool_use_id":"toolu_plan","requires_user_interaction":true}}\n' "$turn"
      IFS= read -r response || break
      case "$response" in
        *'"allow"'*)
          printf '{"type":"system","subtype":"status","status":null,"permissionMode":"default","uuid":"39419917-7c47-4a09-bb3b-077ccac12720","session_id":"%s"}\n' "$sid"
          ;;
      esac
      ;;
  esac
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_mode() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let spawns_log = sandbox.root.join("spawns.log");
    let spawns = || -> Vec<String> {
        std::fs::read_to_string(&spawns_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    };
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("plan_mode", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Plan work").await;
    let ws = text(&view, "/item/id");

    j.step("launch a session in plan mode");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "E2E_PROMPT_ONE".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode: Some(PermissionMode::Plan),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$SESSION");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let first = drain_turn(&mut events).await;
    j.guard(
        "the harness is launched in plan mode",
        spawns() == ["spawn plan"],
        json!({ "spawns": spawns() }),
    );
    j.guard(
        "a turn with no mode change reports none",
        !first.iter().any(is_mode_event),
        json!({ "mode_events": first.iter().filter(|e| is_mode_event(e)).count() }),
    );

    j.step("the agent exits plan mode");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "EXIT_PLAN now".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let asked = recv_until(&mut events, |e| {
        matches!(e, SessionEvent::PermissionAsk { .. })
    })
    .await;
    let request_id = asked.iter().find_map(|e| match e {
        SessionEvent::PermissionAsk {
            request_id,
            tool_name,
            ..
        } if tool_name == "ExitPlanMode" => Some(request_id.clone()),
        _ => None,
    });
    j.guard(
        "ExitPlanMode asks the user first",
        request_id.is_some(),
        json!({ "asked": request_id.is_some() }),
    );
    j.call(
        &client,
        Request::RespondPermission {
            session_id: session.id.clone(),
            request_id: request_id.unwrap_or_default(),
            allow: true,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
    let exited = drain_turn(&mut events).await;
    let modes: Vec<PermissionMode> = exited
        .iter()
        .filter_map(|e| match e {
            SessionEvent::PermissionModeChanged { mode } => Some(*mode),
            _ => None,
        })
        .collect();
    j.guard(
        "the client hears the harness leave plan mode",
        modes == [PermissionMode::Default],
        json!({ "modes": modes }),
    );
    let recorded = requested_mode(&mut j, &client, &ws, &session.id).await;
    j.guard(
        "the row records the mode the harness left plan mode for",
        recorded == "default",
        json!({ "mode": recorded }),
    );

    j.step("the mode change is live state, not history");
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let kinds = kinds_of(&log);
    j.guard(
        "the stored log carries no mode change",
        !kinds.iter().any(|k| k == "permissionModeChanged")
            && kinds.iter().any(|k| k == "turnDone"),
        json!({ "kinds": kinds }),
    );

    j.step("resume in the mode the harness chose");
    drop(events);
    let resumed = client
        .resume_session_stream(
            session.id.clone(),
            None,
            None,
            Some(PermissionMode::Default),
            None,
        )
        .await;
    j.guard(
        "the resume is accepted",
        resumed.is_ok(),
        json!({ "resumed": resumed.is_ok() }),
    );
    let mut events = resumed.expect("a resumed session").3;
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "E2E_PROMPT_THREE".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    // A relaunched stub restarts its turn count, so it would never say REPLY_3.
    let third = recv_until(
        &mut events,
        |e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_3")),
    )
    .await;
    j.guard(
        "it attaches to the running harness instead of relaunching",
        spawns() == ["spawn plan"]
            && third
                .iter()
                .any(|e| matches!(e, SessionEvent::AssistantText { text, .. } if text.contains("E2E_REPLY_3"))),
        json!({ "spawns": spawns() }),
    );
    drop(events);

    j.step("a different mode attaches to the running harness");
    let attached = client
        .resume_session_stream(
            session.id.clone(),
            None,
            None,
            Some(PermissionMode::AcceptEdits),
            None,
        )
        .await;
    j.guard(
        "the resume is accepted",
        attached.is_ok(),
        json!({ "resumed": attached.is_ok() }),
    );
    let running = match &attached {
        Ok((_, mode, ..)) => *mode,
        Err(_) => None,
    };
    j.guard(
        "the resume reports the harness's own mode",
        running == Some(PermissionMode::Default),
        json!({ "running": running }),
    );
    let relaunched = eventually(Duration::from_secs(2), || spawns().len() > 1).await;
    j.guard(
        "no relaunch",
        !relaunched && spawns() == ["spawn plan"],
        json!({ "spawns": spawns() }),
    );
    drop(attached);

    j.step("restart applies a different mode");
    let restarted = client
        .restart_session_stream(
            session.id.clone(),
            None,
            None,
            Some(PermissionMode::AcceptEdits),
        )
        .await;
    j.guard(
        "the restart is accepted",
        restarted.is_ok(),
        json!({ "restarted": restarted.is_ok() }),
    );
    // The stub logs its spawn from its own process, after the restart returns.
    let _ = tokio::time::timeout(IO, async {
        while spawns().len() < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    j.guard(
        "the harness is relaunched in the requested mode",
        spawns() == ["spawn plan", "spawn acceptEdits"],
        json!({ "spawns": spawns() }),
    );

    j.step("an in-place switch to Auto stays Auto");
    let (auto, _prompt, _stream_id, mut auto_events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "E2E_PROMPT_AUTO".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session (auto)");
    j.alias(&auto.id, "$AUTO");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: auto.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse (auto)");
    drain_turn(&mut auto_events).await;
    // Each switch is followed by a turn; the stub answers a switch before it reads the turn.
    let switch = |mode: Option<PermissionMode>| Request::SetSessionPermissionMode {
        session_id: auto.id.clone(),
        mode,
    };
    let turn = |text: &str| Request::SendUserMessage {
        session_id: auto.id.clone(),
        text: text.into(),
        attached_docs: vec![],
        prompt_name: None,
        client_message_id: None,
        base_rev: None,
    };
    j.call(&client, switch(Some(PermissionMode::AcceptEdits)))
        .await
        .expect("set_session_permission_mode (acceptEdits)");
    j.call(&client, turn("after accept edits"))
        .await
        .expect("send_user_message");
    let heard_edits = mode_changes(&drain_turn(&mut auto_events).await);
    let edits = requested_mode(&mut j, &client, &ws, &auto.id).await;
    j.guard(
        "a switch to Accept edits is heard and recorded",
        heard_edits == [PermissionMode::AcceptEdits] && edits == "acceptEdits",
        json!({ "heard": heard_edits, "mode": edits }),
    );
    j.call(&client, switch(None))
        .await
        .expect("set_session_permission_mode (auto)");
    j.call(&client, turn("after auto"))
        .await
        .expect("send_user_message");
    let heard_auto = mode_changes(&drain_turn(&mut auto_events).await);
    let back = requested_mode(&mut j, &client, &ws, &auto.id).await;
    j.guard(
        "the harness's report of Auto's own mode leaves the row on Auto and reaches no client",
        heard_auto.is_empty() && back.is_null(),
        json!({ "heard": heard_auto, "mode": back }),
    );
    drop(auto_events);

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn mode_changes(events: &[SessionEvent]) -> Vec<PermissionMode> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::PermissionModeChanged { mode } => Some(*mode),
            _ => None,
        })
        .collect()
}

/// The session row's `requestedPermissionMode`; null is Auto.
async fn requested_mode(j: &mut Journey, client: &RpcClient, ws: &str, id: &str) -> Value {
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

fn is_mode_event(e: &SessionEvent) -> bool {
    matches!(
        e,
        SessionEvent::PermissionModeChanged { .. } | SessionEvent::PlanModeChanged { .. }
    )
}

async fn drain_turn(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    recv_until(events, |e| matches!(e, SessionEvent::TurnDone { .. })).await
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

fn kinds_of(log: &Value) -> Vec<String> {
    log.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.pointer("/event/kind").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
