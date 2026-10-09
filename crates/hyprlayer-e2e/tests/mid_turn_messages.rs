//! A message sent while a turn runs joins that turn at the harness's next step,
//! as claude's own CLI does: it interrupts nothing, its echo lands where the
//! model read it, and one still waiting can be withdrawn.
//!
//! The harness is a stub speaking claude's queue protocol (`uuid` in, then
//! `command_lifecycle` queued/started/completed out), not the real binary;
//! `claude_driver_smoke` covers that against the real one.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const QUIET: Duration = Duration::from_millis(800);

/// Every line it reads is appended to `<sandbox>/harness.log`. A turn whose
/// text carries `E2E_STEER` or `E2E_WITHDRAW` starts a tool and reads the next
/// line while it "runs", the way claude queues input that arrives mid-turn.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
log="$(dirname "$0")/../harness.log"
field() { printf '%s' "$1" | sed -n "s/.*\"$2\":\"\([^\"]*\)\".*/\1/p"; }
lifecycle() { printf '{"type":"command_lifecycle","command_uuid":"%s","state":"%s","uuid":"lc-%s","session_id":"%s"}\n' "$1" "$2" "$2" "$sid"; }
init() { printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"; }
reply() { printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"%s"}]},"session_id":"%s"}\n' "$1" "$sid"; }
result() { printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1}}\n' "$sid"; }
tool_use() { printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"%s","name":"Bash","input":{"command":"sleep 8"}}]},"session_id":"%s"}\n' "$1" "$sid"; }
tool_result() { printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"%s","content":"slept","is_error":false}]},"session_id":"%s"}\n' "$1" "$sid"; }
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  uuid=$(field "$line" uuid)
  lifecycle "$uuid" queued
  lifecycle "$uuid" started
  init
  case "$line" in
    *E2E_STEER*)
      tool_use toolu_steer
      IFS= read -r steer || break
      printf '%s\n' "$steer" >> "$log"
      next=$(field "$steer" uuid)
      lifecycle "$next" queued
      tool_result toolu_steer
      lifecycle "$next" started
      reply "E2E_HEARD_STEER"
      result
      lifecycle "$next" completed
      ;;
    *E2E_WITHDRAW*)
      tool_use toolu_hold
      IFS= read -r steer || break
      printf '%s\n' "$steer" >> "$log"
      next=$(field "$steer" uuid)
      lifecycle "$next" queued
      IFS= read -r cancel || break
      printf '%s\n' "$cancel" >> "$log"
      lifecycle "$next" cancelled
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":{"cancelled":true}}}\n' "$(field "$cancel" request_id)"
      tool_result toolu_hold
      reply "E2E_HEARD_NOTHING"
      result
      ;;
    *)
      reply "E2E_REPLY"
      result
      ;;
  esac
  lifecycle "$uuid" completed
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mid_turn_messages() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let harness_log = sandbox.root.join("harness.log");
    let harness_saw = |needle: &str| {
        std::fs::read_to_string(&harness_log)
            .unwrap_or_default()
            .contains(needle)
    };
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("mid_turn_messages", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Mid-turn messages").await;
    let ws = text(&view, "/item/id");

    j.step("an idle send opens its turn at once");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "E2E_HELLO".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
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
    let hello = recv_until(&mut events, is_turn_done).await;
    j.guard(
        "the first message echoes and its turn completes",
        echo_index(&hello, None).is_some() && hello.iter().any(is_turn_done),
        json!({ "events": shape(&hello) }),
    );
    j.guard(
        "the harness's lifecycle frames for the app's own messages never reach the client",
        !hello.iter().any(is_lifecycle),
        json!({ "events": shape(&hello) }),
    );

    j.step("a message sent mid-turn joins the running turn");
    send(
        &mut j,
        &client,
        &session.id,
        "E2E_STEER run the slow tool",
        "steer-turn",
    )
    .await;
    let running = recv_until(&mut events, |e| matches!(e, SessionEvent::ToolUse { .. })).await;
    j.guard(
        "the turn's tool is running when the next message is sent",
        running
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { .. })),
        json!({ "events": shape(&running) }),
    );
    send(
        &mut j,
        &client,
        &session.id,
        "E2E_MIDTURN also mention lemons",
        "mid-turn",
    )
    .await;
    let turn = recv_until(&mut events, is_turn_done).await;
    let after = quiet(&mut events).await;
    let mid = echo_index(&turn, Some("mid-turn"));
    let result = turn
        .iter()
        .position(|e| matches!(e, SessionEvent::ToolResult { .. }));
    j.guard(
        "its echo waits for the running tool's result, where the harness took it in",
        matches!((result, mid), (Some(r), Some(m)) if r < m),
        json!({ "events": shape(&turn) }),
    );
    j.guard(
        "the reply that follows heard it",
        turn.iter().any(
            |e| matches!(e, SessionEvent::AssistantText { text, .. } if text == "E2E_HEARD_STEER"),
        ),
        json!({ "events": shape(&turn) }),
    );
    j.guard(
        "it joins the running turn instead of opening its own",
        turn.iter().filter(|e| is_turn_done(e)).count() == 1 && after.is_empty(),
        json!({ "turn": shape(&turn), "after": shape(&after) }),
    );
    j.guard(
        "the running tool was not interrupted",
        turn.iter().any(|e| {
            matches!(
                e,
                SessionEvent::ToolResult {
                    is_error: false,
                    ..
                }
            )
        }),
        json!({ "events": shape(&turn) }),
    );
    j.guard(
        "the harness received it while the tool ran",
        harness_saw("E2E_MIDTURN"),
        json!({ "harness_saw": harness_saw("E2E_MIDTURN") }),
    );

    j.step("the stored transcript keeps the harness's order");
    let log = j
        .call(
            &client,
            Request::GetSessionLog {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("get_session_log");
    let order = stored_order(&log);
    j.guard(
        "tool result, then the mid-turn message, then the reply",
        order.windows(3).any(|w| {
            w == [
                "toolResult",
                "userMessage:mid-turn",
                "assistantText:E2E_HEARD_STEER",
            ]
        }),
        json!({ "order": order }),
    );
    j.guard(
        "no queued-command seam was stored for the app's own messages",
        !order.iter().any(|k| k.starts_with("commandLifecycle")),
        json!({ "order": order }),
    );

    j.step("a waiting message can be withdrawn");
    send(
        &mut j,
        &client,
        &session.id,
        "E2E_WITHDRAW hold the tool",
        "withdraw-turn",
    )
    .await;
    recv_until(&mut events, |e| matches!(e, SessionEvent::ToolUse { .. })).await;
    send(
        &mut j,
        &client,
        &session.id,
        "E2E_LATE never mind",
        "withdrawn",
    )
    .await;
    let withdrawn = j
        .call(
            &client,
            Request::CancelQueuedMessage {
                session_id: session.id.clone(),
                client_message_id: "withdrawn".into(),
            },
        )
        .await
        .expect("cancel_queued_message");
    let turn = recv_until(&mut events, is_turn_done).await;
    j.guard(
        "the harness confirms it was withdrawn",
        withdrawn == Value::Bool(true),
        json!({ "withdrawn": withdrawn }),
    );
    j.guard(
        "a withdrawn message never reaches the transcript",
        echo_index(&turn, Some("withdrawn")).is_none(),
        json!({ "events": shape(&turn) }),
    );
    j.guard(
        "a withdrawal the user asked for is not handed back to resend",
        !turn.iter().any(is_lifecycle),
        json!({ "events": shape(&turn) }),
    );
    let gone = j
        .call(
            &client,
            Request::CancelQueuedMessage {
                session_id: session.id.clone(),
                client_message_id: "mid-turn".into(),
            },
        )
        .await
        .expect("cancel_queued_message");
    j.guard(
        "a message already taken in cannot be withdrawn",
        gone == Value::Bool(false),
        json!({ "withdrawn": gone }),
    );

    j.finish();
}

async fn send(j: &mut Journey, client: &RpcClient, session_id: &str, text: &str, id: &str) {
    j.call(
        client,
        Request::SendUserMessage {
            session_id: session_id.to_string(),
            text: text.into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: Some(id.into()),
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
}

fn is_turn_done(e: &SessionEvent) -> bool {
    matches!(e, SessionEvent::TurnDone { .. })
}

fn is_lifecycle(e: &SessionEvent) -> bool {
    matches!(e, SessionEvent::CommandLifecycle { .. })
}

fn echo_index(events: &[SessionEvent], id: Option<&str>) -> Option<usize> {
    events.iter().position(|e| {
        matches!(e, SessionEvent::UserMessage { client_message_id, .. } if client_message_id.as_deref() == id)
    })
}

/// Kinds in arrival order, user messages tagged with their id; partial text is noise.
fn shape(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|e| !matches!(e, SessionEvent::PartialText { .. }))
        .map(|e| match e {
            SessionEvent::UserMessage {
                client_message_id, ..
            } => format!(
                "userMessage:{}",
                client_message_id.as_deref().unwrap_or("-")
            ),
            SessionEvent::AssistantText { text, .. } => format!("assistantText:{text}"),
            other => other.kind().to_string(),
        })
        .collect()
}

fn stored_order(log: &Value) -> Vec<String> {
    log.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let event = r.get("event")?;
                    let kind = event.get("kind").and_then(Value::as_str)?;
                    Some(match kind {
                        "userMessage" => format!(
                            "userMessage:{}",
                            event
                                .get("clientMessageId")
                                .and_then(Value::as_str)
                                .unwrap_or("-")
                        ),
                        "assistantText" => format!(
                            "assistantText:{}",
                            event
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                        ),
                        other => other.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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

/// What else arrives once the turn has ended: a second turn would show here.
async fn quiet(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(QUIET, events.recv()).await {
        seen.push(event);
    }
    seen
}
