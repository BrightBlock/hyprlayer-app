//! Two parallel calls to one MCP tool each ask for permission: every ask names
//! its own tool use, live and on replay, and answering one leaves the other
//! waiting. Init carries the MCP servers' real names, which tool ids mangle.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// On a `PARALLEL_ASKS` turn, calls one tool twice, asks for both, and returns
/// each call's result only once its own ask is answered.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[],"mcp_servers":[{"name":"claude.ai Atlassian Rovo","status":"connected","source":"claudeai"}]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *PARALLEL_ASKS*)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_a","name":"mcp__claude_ai_Atlassian_Rovo__getJiraIssue","input":{"key":"A"}},{"type":"tool_use","id":"toolu_b","name":"mcp__claude_ai_Atlassian_Rovo__getJiraIssue","input":{"key":"B"}}]},"session_id":"%s"}\n' "$sid"
      for t in a b; do
        printf '{"type":"control_request","request_id":"req-%s","request":{"subtype":"can_use_tool","tool_name":"mcp__claude_ai_Atlassian_Rovo__getJiraIssue","input":{"key":"%s"},"tool_use_id":"toolu_%s"}}\n' "$t" "$t" "$t"
      done
      for _ in 1 2; do
        IFS= read -r response || exit 0
        case "$response" in
          *'"req-a"'*) t=a ;;
          *) t=b ;;
        esac
        printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_%s","content":"issue %s","is_error":false}]},"session_id":"%s"}\n' "$t" "$t" "$sid"
      done
      ;;
  esac
  printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":%s,"total_cost_usd":0,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_permission_asks() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("parallel_permission_asks", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Parallel asks").await;
    let ws = text(&view, "/item/id");

    j.step("two calls to one tool ask at once");
    let (session, mut events) = start_session(&mut j, &client, &ws, "PARALLEL_ASKS").await;
    j.alias(&session, "$SESSION");
    let seen = recv_until(
        &mut events,
        |e| matches!(e, SessionEvent::PermissionAsk { request_id, .. } if request_id == "req-b"),
    )
    .await;
    let servers = seen.iter().find_map(|e| match e {
        SessionEvent::Init { mcp_servers, .. } => Some(json!(mcp_servers)),
        _ => None,
    });
    j.guard(
        "init names the MCP servers as claude lists them",
        servers == Some(json!(["claude.ai Atlassian Rovo"])),
        json!({ "servers": servers }),
    );
    let asks = asks_of(&seen);
    j.guard(
        "each ask names its own tool use",
        asks == json!([["req-a", "toolu_a"], ["req-b", "toolu_b"]]),
        json!({ "asks": asks }),
    );

    j.step("answer one ask");
    respond(&mut j, &client, &session, "req-a").await;
    let after = recv_until(&mut events, |e| {
        matches!(e, SessionEvent::ToolResult { .. })
    })
    .await;
    let states = session_state(&mut j, &client, &ws, &session).await;
    j.guard(
        "only the answered call returns; the session still waits on the other",
        results_of(&after) == json!(["toolu_a"]) && states == "awaiting_permission",
        json!({ "results": results_of(&after), "state": states }),
    );

    j.step("answer the other ask");
    respond(&mut j, &client, &session, "req-b").await;
    let after = recv_until(&mut events, |e| matches!(e, SessionEvent::TurnDone { .. })).await;
    j.guard(
        "the second call returns and the turn ends",
        results_of(&after) == json!(["toolu_b"])
            && after
                .iter()
                .any(|e| matches!(e, SessionEvent::TurnDone { .. })),
        json!({ "results": results_of(&after) }),
    );

    j.step("replay the log");
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let replayed: Vec<Value> = log
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("event"))
        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("permissionAsk"))
        .map(|e| json!([e["requestId"], e["toolUseId"]]))
        .collect();
    j.guard(
        "the stored asks keep their tool use ids",
        Value::Array(replayed.clone()) == asks,
        json!({ "asks": replayed }),
    );

    j.step("shutdown");
    drop(events);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn asks_of(events: &[SessionEvent]) -> Value {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::PermissionAsk {
                request_id,
                tool_use_id,
                ..
            } => Some(json!([request_id, tool_use_id])),
            _ => None,
        })
        .collect()
}

fn results_of(events: &[SessionEvent]) -> Value {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolResult { tool_use_id, .. } => Some(json!(tool_use_id)),
            _ => None,
        })
        .collect()
}

async fn respond(j: &mut Journey, client: &RpcClient, session: &str, request_id: &str) {
    j.call(
        client,
        Request::RespondPermission {
            session_id: session.to_string(),
            request_id: request_id.into(),
            allow: true,
            message: None,
        },
    )
    .await
    .expect("respond_permission");
}

async fn start_session(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    input: &str,
) -> (String, EventStream<SessionEvent>) {
    let (session, _prompt, _stream_id, events) = client
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            input.into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    (session.id, events)
}

async fn session_state(j: &mut Journey, client: &RpcClient, ws: &str, session: &str) -> String {
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
        .into_iter()
        .flatten()
        .find(|r| text(r, "/id") == session)
        .and_then(|r| r.get("state"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
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
