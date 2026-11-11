//! `/compact` and `/clear` change the context without a usage report. The client
//! gets what claude does say: the post-compaction size and summary, or the new
//! conversation's id, so the meter never keeps a fill from before.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::EventStream;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const SUMMARY_START: &str = "This session is being continued from a previous conversation";
const CLEARED_SESSION: &str = "ce1e46b2-9aa9-4b44-ac07-f58408bcc1da";

/// Answers `/compact` and `/clear` with `<sandbox>/<command>.jsonl` and any other
/// turn with one reply that fills 30,096 tokens of context.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    --resume) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
    *'"/compact"'*) sed "s/SESSION_ID/$sid/g" "$(dirname "$0")/../compact.jsonl" ;;
    *'"/clear"'*) sed "s/SESSION_ID/$sid/g" "$(dirname "$0")/../clear.jsonl" ;;
    *)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY"}],"usage":{"input_tokens":2,"cache_creation_input_tokens":4609,"cache_read_input_tokens":25485,"output_tokens":7}},"session_id":"%s"}\n' "$sid"
      printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY","session_id":"%s","num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":2,"cache_creation_input_tokens":4609,"cache_read_input_tokens":25485,"output_tokens":7}}\n' "$sid"
      ;;
  esac
done
"#;

/// claude 2.1.299's reply to `/compact`, its session id, summary and init swapped
/// for the stub's. The result reports zero usage, so only the boundary's
/// `post_tokens` says how full the context is; the summary is a plain string.
const COMPACT: &str = r##"{"type":"system","subtype":"status","status":"compacting","session_id":"SESSION_ID","uuid":"2f49302a-920c-41ae-b770-b4d1b332cda2"}
{"type":"system","subtype":"status","status":null,"compact_result":"success","session_id":"SESSION_ID","uuid":"3bfba262-725c-454a-832e-8e744db1558a"}
{"type":"system","subtype":"init","session_id":"SESSION_ID","model":"claude-e2e","permissionMode":"default","tools":[]}
{"type":"system","subtype":"compact_boundary","session_id":"SESSION_ID","uuid":"0c02c80e-805c-411e-b383-3d342de1643b","compact_metadata":{"trigger":"manual","pre_tokens":30103,"post_tokens":1374,"cumulative_dropped_tokens":28729,"duration_ms":1245,"preserved_segment":{"head_uuid":"007c5162-b04e-4dcc-b564-04e2e45aacb5","anchor_uuid":"e1568905-7c7f-4c42-8977-803dc416490c","tail_uuid":"007c5162-b04e-4dcc-b564-04e2e45aacb5"},"preserved_messages":{"anchor_uuid":"e1568905-7c7f-4c42-8977-803dc416490c","uuids":["007c5162-b04e-4dcc-b564-04e2e45aacb5"],"all_uuids":["007c5162-b04e-4dcc-b564-04e2e45aacb5"]}},"logical_parent_uuid":"007c5162-b04e-4dcc-b564-04e2e45aacb5"}
{"type":"user","message":{"role":"user","content":"This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\nE2E_REPLY\nContinue the conversation from where it left off without asking the user any further questions. Resume directly — do not acknowledge the summary, do not recap what was happening, do not preface with \"I'll continue\" or similar. Pick up the last task as if the break never happened.\n\nThe messages after this summary are the most recent messages from before compaction, kept verbatim. The summary was written without seeing them, so something it says has not happened yet may already have happened in them."},"session_id":"SESSION_ID","parent_tool_use_id":null,"uuid":"e1568905-7c7f-4c42-8977-803dc416490c","timestamp":"2026-10-16T15:50:38.262Z","isReplay":false,"isSynthetic":true}
{"type":"user","message":{"role":"user","content":"<local-command-stdout>Compacted </local-command-stdout>"},"session_id":"SESSION_ID","parent_tool_use_id":null,"uuid":"bb7ff4f2-0ebb-447b-b073-d44e9bf67481","timestamp":"2026-10-16T15:50:38.368Z","isReplay":true}
{"is_error":false,"duration_api_ms":0,"num_turns":0,"stop_reason":null,"session_id":"SESSION_ID","total_cost_usd":0.0041025950000000005,"usage":{"output_tokens_details":{"thinking_tokens":0},"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard","fallback_credit":null},"modelUsage":{"claude-haiku-5-5":{"inputTokens":3224,"outputTokens":38,"cacheReadInputTokens":73337,"cacheCreationInputTokens":16851,"webSearchRequests":0,"costUSD":0.0041025950000000005,"contextWindow":1000000,"maxOutputTokens":128000,"thinkingTokens":0,"canonicalModel":"claude-haiku-5-5","provider":"firstParty","costBasis":"list"}},"permission_denials":[],"fast_mode_state":"off","fast_mode_disabled_reason":"sdk_opt_in_required","subagent_stats":{"spawned":0,"requested":{"background":0,"foreground":0,"unset":0},"started_in_background":0,"max_depth":0,"spawned_by_subagents":0,"completed":0,"failed":0,"killed":{"parent":0,"user":0,"system":0},"refused":{"depth_limit":0,"concurrency_limit":0,"budget":0},"by_type":{}},"safety_stops":0,"subtype":"success","result":"","user_message_uuid":"5004ab34-2096-463c-b5c5-df9c1521a5aa","user_message_uuids":["5004ab34-2096-463c-b5c5-df9c1521a5aa"],"local_command":"compact","type":"result","duration_ms":1257,"uuid":"f937661b-a8d6-4446-a21d-5d7f18a27d63","queued_turn_count":0,"result_index":2}
"##;

/// claude 2.1.299's reply to `/clear`, its old session id and init swapped for
/// the stub's: a new session id and a zero-usage result, no fill.
const CLEAR: &str = r##"{"type":"conversation_reset","new_conversation_id":"5e552698-74c6-40ea-875d-3cc1a132a7db","uuid":"5e552698-74c6-40ea-875d-3cc1a132a7db","trigger":"clear","user_message_uuid":"bce71726-c29b-4b6c-b7cc-f715c813be32","timestamp":"2026-10-16T19:32:34.488Z","session_id":"SESSION_ID"}
{"type":"system","subtype":"init","session_id":"ce1e46b2-9aa9-4b44-ac07-f58408bcc1da","model":"claude-e2e","permissionMode":"default","tools":[]}
{"is_error":false,"duration_api_ms":0,"num_turns":0,"stop_reason":null,"session_id":"ce1e46b2-9aa9-4b44-ac07-f58408bcc1da","total_cost_usd":0,"usage":{"output_tokens_details":{"thinking_tokens":0},"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard","fallback_credit":null},"modelUsage":{},"permission_denials":[],"fast_mode_state":"off","fast_mode_disabled_reason":"sdk_opt_in_required","subagent_stats":{"spawned":0,"requested":{"background":0,"foreground":0,"unset":0},"started_in_background":0,"max_depth":0,"spawned_by_subagents":0,"completed":0,"failed":0,"killed":{"parent":0,"user":0,"system":0},"refused":{"depth_limit":0,"concurrency_limit":0,"budget":0},"by_type":{}},"safety_stops":0,"subtype":"success","result":"","user_message_uuid":"bce71726-c29b-4b6c-b7cc-f715c813be32","user_message_uuids":["bce71726-c29b-4b6c-b7cc-f715c813be32"],"local_command":"clear","type":"result","duration_ms":155,"uuid":"20847017-66d6-481c-92cb-f7eb4b922ece","queued_turn_count":0,"result_index":2}
"##;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_resets() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    std::fs::write(sandbox.root.join("compact.jsonl"), COMPACT).expect("stage the /compact reply");
    std::fs::write(sandbox.root.join("clear.jsonl"), CLEAR).expect("stage the /clear reply");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("context_resets", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Compaction").await;
    let ws = text(&view, "/item/id");

    j.step("a turn fills the context");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws,
            Stage::General,
            "E2E_PROMPT".into(),
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
    let turn = drain_turn(&mut events).await;
    j.guard(
        "the turn reports its fill",
        turn.iter().any(|e| {
            matches!(
                e,
                SessionEvent::TurnDone {
                    context_tokens: Some(30_096),
                    ..
                }
            )
        }),
        json!({ "events": shape(&turn) }),
    );

    j.step("a manual /compact");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "/compact".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let compacted = drain_turn(&mut events).await;
    j.guard(
        "the boundary records the size it compacted",
        compacted.iter().any(|e| {
            matches!(
                e,
                SessionEvent::CompactBoundary {
                    trigger,
                    pre_tokens: Some(30_103),
                } if trigger == "manual"
            )
        }),
        json!({ "events": shape(&compacted) }),
    );
    // The turn ends with no usage of its own, so this is the fill the meter keeps.
    let after = compacted
        .iter()
        .skip_while(|e| !matches!(e, SessionEvent::CompactBoundary { .. }))
        .find_map(|e| match e {
            SessionEvent::Usage { context_tokens, .. } => Some(*context_tokens),
            _ => None,
        });
    j.guard(
        "the fill after the boundary is claude's post-compaction size",
        after == Some(1_374),
        json!({ "events": shape(&compacted) }),
    );
    let texts: Vec<&SessionEvent> = compacted
        .iter()
        .filter(|e| matches!(e, SessionEvent::AssistantText { .. }))
        .collect();
    j.guard(
        "the summary reaches the client as injected text, and nothing else does",
        matches!(
            texts.as_slice(),
            [SessionEvent::AssistantText {
                text,
                parent_id: None,
                injected: true,
                auth_required: false,
            }] if text.starts_with(SUMMARY_START)
        ),
        json!({ "events": shape(&compacted) }),
    );

    j.step("a /clear");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "/clear".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let cleared = drain_turn(&mut events).await;
    // A new id is a new conversation: the client drops the old fill until one is reported.
    j.guard(
        "the client hears the new conversation's id, and no fill",
        cleared.iter().any(|e| {
            matches!(e, SessionEvent::Init { provider_session_id, .. } if provider_session_id == CLEARED_SESSION)
        }) && !cleared
            .iter()
            .any(|e| matches!(e, SessionEvent::Usage { .. })),
        json!({ "events": shape(&cleared) }),
    );
    j.guard(
        "claude's conversation_reset line raises no protocol error",
        !cleared
            .iter()
            .any(|e| matches!(e, SessionEvent::ProtocolError { .. })),
        json!({ "events": shape(&cleared) }),
    );

    j.step("the stored log replays both");
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
        "the post-compaction fill and the summary follow the boundary",
        order
            .windows(3)
            .any(|w| w == ["compactBoundary", "usage:1374", "assistantText:injected"]),
        json!({ "order": order }),
    );
    j.guard(
        "nothing stored is a protocol error",
        !order.iter().any(|k| k == "protocolError"),
        json!({ "order": order }),
    );

    j.finish();
}

async fn drain_turn(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            let done = matches!(event, SessionEvent::TurnDone { .. });
            seen.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    seen
}

/// Kinds in arrival order, each fill tagged with its size; partial text is noise.
fn shape(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|e| !matches!(e, SessionEvent::PartialText { .. }))
        .map(|e| match e {
            SessionEvent::Init {
                provider_session_id,
                ..
            } if provider_session_id == CLEARED_SESSION => "init:cleared".into(),
            SessionEvent::AssistantText { injected: true, .. } => "assistantText:injected".into(),
            SessionEvent::Usage { context_tokens, .. } => format!("usage:{context_tokens}"),
            SessionEvent::TurnDone { context_tokens, .. } => match context_tokens {
                Some(t) => format!("turnDone:{t}"),
                None => "turnDone:-".into(),
            },
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
                        "init"
                            if event.get("claudeSessionId").and_then(Value::as_str)
                                == Some(CLEARED_SESSION) =>
                        {
                            "init:cleared".into()
                        }
                        "assistantText" if event.get("injected") == Some(&Value::Bool(true)) => {
                            "assistantText:injected".into()
                        }
                        "usage" => format!(
                            "usage:{}",
                            event
                                .get("contextTokens")
                                .and_then(Value::as_u64)
                                .unwrap_or_default()
                        ),
                        other => other.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
