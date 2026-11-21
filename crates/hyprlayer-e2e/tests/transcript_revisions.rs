//! Sessions carry a persisted `rev`: every committed turn advances it, a
//! catch-up read pages through history at any revision, and a send naming a
//! stale `base_rev` is refused before it ever reaches the harness.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_proto::StaleTranscript;
use hyprlayer_server::client::EventStream;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
/// A pagination bug that never reports `remaining: 0` must fail loudly, not hang.
const MAX_PAGES: usize = 20;

/// `session_lifecycle.rs`'s script plus an `E2E_TOOL` branch: a `tool_use`
/// block and a user `tool_result` line (shape: `claude/protocol.rs:612`)
/// before the standard reply.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
log="$(dirname "$0")/../harness.log"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *ASK_QUESTION*)
      printf '{"type":"control_request","request_id":"q-%s","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Ship it?","header":"Ship","multiSelect":false,"options":[{"label":"Yes","description":"ship now"},{"label":"No","description":"hold"}]}]}}}\n' "$turn"
      IFS= read -r response || break
      printf '%s\n' "$response" >> "$log"
      ;;
    *ASK_PERMISSION*)
      printf '{"type":"control_request","request_id":"req-%s","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}\n' "$turn"
      IFS= read -r response || break
      printf '%s\n' "$response" >> "$log"
      ;;
    *E2E_TOOL*)
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_e2e_%s","name":"Bash","input":{"command":"echo hi"}}]},"session_id":"%s"}\n' "$turn" "$sid"
      printf '{"type":"user","uuid":"u-tool-%s","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_e2e_%s","content":"ok"}]},"session_id":"%s"}\n' "$turn" "$turn" "$sid"
      ;;
  esac
  printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"E2E_PARTIAL_%s"}},"session_id":"%s","parent_tool_use_id":null}\n' "$turn" "$sid"
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7},"modelUsage":{"claude-e2e":{"contextWindow":200000}}}\n' "$turn" "$sid" "$turn"
done
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transcript_revisions() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let harness_log = sandbox.root.join("harness.log");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("transcript_revisions", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Transcript revisions").await;
    let ws = text(&view, "/item/id");

    j.step("A starts a session");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "E2E_PROMPT_ONE".into(),
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
    drain_turn(&mut events).await;
    let turns = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 100,
            },
        )
        .await
        .expect("list_session_turns");
    j.guard(
        "a first turn is rev 2 (user message, reply)",
        rev(&turns) == 2
            && reset_rev(&turns) == 0
            && in_flight(&turns) == 0
            && remaining(&turns) == 0,
        json!({ "kinds": kinds_of(&turns) }),
    );

    j.step("a tool turn commits three revisions");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "E2E_TOOL".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: Some(2),
        },
    )
    .await
    .expect("send_user_message (tool turn)");
    drain_turn(&mut events).await;
    let turns = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 100,
            },
        )
        .await
        .expect("list_session_turns");
    j.guard(
        "user, tool result and reply each advance rev; the tool call does not",
        rev(&turns) == 5,
        json!({ "rev": rev(&turns) }),
    );

    j.step("the catch-up read pages and repeats");
    let full = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 100,
            },
        )
        .await
        .expect("list_session_turns (full)");
    let full_seqs = seqs_of(&full);
    let mut cursor = LogCursor::Empty;
    let mut pages: Vec<Value> = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = j
            .call(
                &client,
                Request::ListSessionTurns {
                    session_id: session.id.clone(),
                    cursor,
                    limit: 3,
                },
            )
            .await
            .expect("list_session_turns (page)");
        let done = remaining(&page) == 0;
        let next = next_cursor_of(&page);
        pages.push(page);
        if done {
            break;
        }
        cursor = next;
    }
    let paged_seqs: Vec<i64> = pages.iter().flat_map(seqs_of).collect();
    let pages_match_full_read = paged_seqs == full_seqs;
    j.guard(
        "pages concatenate to the full log",
        pages_match_full_read,
        json!({ "pages": pages.len(), "matches": pages_match_full_read }),
    );
    let each_page_resumes = pages.iter().all(page_cursor_matches_its_last_row);
    j.guard(
        "each page resumes at its own last row",
        each_page_resumes,
        json!({ "pages": pages.len(), "eachPageResumes": each_page_resumes }),
    );
    let repeat = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 3,
            },
        )
        .await
        .expect("list_session_turns (repeat)");
    let identical = pages.first().is_some_and(|first| *first == repeat);
    j.guard(
        "the same read twice returns the same page",
        identical,
        json!({ "identical": identical }),
    );

    j.step("a stale send never reaches the harness");
    let refused = j
        .call(
            &client,
            Request::SendUserMessage {
                session_id: session.id.clone(),
                text: "E2E_STALE".into(),
                attached_docs: vec![],
                prompt_name: None,
                client_message_id: None,
                base_rev: Some(2),
            },
        )
        .await;
    let detail = refused.as_ref().err().and_then(|e| e.stale_transcript);
    j.guard(
        "refused with stale_transcript",
        refused.as_ref().err().map(|e| e.code.as_str()) == Some("stale_transcript")
            && detail
                == Some(StaleTranscript {
                    rev: 5,
                    committed_rev: 5,
                    missing_turns: 3,
                }),
        json!({
            "code": refused.as_ref().err().map(|e| e.code.clone()),
            "rev": detail.map(|d| d.rev),
            "committedRev": detail.map(|d| d.committed_rev),
            "missingTurns": detail.map(|d| d.missing_turns),
        }),
    );
    let seen = std::fs::read_to_string(&harness_log).unwrap_or_default();
    j.guard(
        "the harness never saw it",
        seen.contains("E2E_TOOL") && !seen.contains("E2E_STALE"),
        json!({ "harnessSawStale": seen.contains("E2E_STALE") }),
    );
    let turns = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 100,
            },
        )
        .await
        .expect("list_session_turns");
    j.guard(
        "rev is unchanged",
        rev(&turns) == 5,
        json!({ "rev": rev(&turns) }),
    );

    j.step("a current send and a legacy send are accepted");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: "E2E_PROMPT_TWO".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: Some(5),
        },
    )
    .await
    .expect("send_user_message (current rev)");
    drain_turn(&mut events).await;
    let turns = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 100,
            },
        )
        .await
        .expect("list_session_turns");
    j.guard(
        "a checked send at the current rev is accepted and commits",
        rev(&turns) == 7,
        json!({ "rev": rev(&turns) }),
    );
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
    .expect("send_user_message (legacy)");
    drain_turn(&mut events).await;
    let turns = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::Empty,
                limit: 100,
            },
        )
        .await
        .expect("list_session_turns");
    j.guard(
        "a legacy send with no base_rev is accepted unchecked",
        rev(&turns) == 9,
        json!({ "rev": rev(&turns) }),
    );

    j.step("a rewritten cursor is refused");
    let (seq, created_at) = match next_cursor_of(&turns) {
        LogCursor::After { seq, created_at } => (seq, created_at),
        other => panic!("a non-empty log has a last row, not {other:?}"),
    };
    let refused = j
        .call(
            &client,
            Request::ListSessionTurns {
                session_id: session.id.clone(),
                cursor: LogCursor::After {
                    seq,
                    created_at: created_at + 1.0,
                },
                limit: 100,
            },
        )
        .await;
    j.guard(
        "a cursor whose row was rewritten is refused",
        matches!(&refused, Err(e) if e.code == "session_epoch_advanced"),
        json!({ "code": refused.as_ref().err().map(|e| e.code.clone()) }),
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

fn rev(turns: &Value) -> u64 {
    turns.pointer("/rev").and_then(Value::as_u64).unwrap_or(0)
}

fn reset_rev(turns: &Value) -> u64 {
    turns
        .pointer("/resetRev")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn in_flight(turns: &Value) -> u64 {
    turns
        .pointer("/inFlight")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn remaining(turns: &Value) -> u64 {
    turns
        .pointer("/remaining")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn kinds_of(turns: &Value) -> Vec<String> {
    turns
        .pointer("/entries")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.pointer("/event/kind").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn seqs_of(turns: &Value) -> Vec<i64> {
    turns
        .pointer("/entries")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.pointer("/seq").and_then(Value::as_i64))
                .collect()
        })
        .unwrap_or_default()
}

fn next_cursor_of(turns: &Value) -> LogCursor {
    turns
        .pointer("/nextCursor")
        .cloned()
        .and_then(|c| serde_json::from_value(c).ok())
        .unwrap_or(LogCursor::Empty)
}

/// Whether a page's `nextCursor` names the last row that page actually returned.
fn page_cursor_matches_its_last_row(page: &Value) -> bool {
    let Some(last) = page
        .pointer("/entries")
        .and_then(Value::as_array)
        .and_then(|rows| rows.last())
    else {
        return matches!(next_cursor_of(page), LogCursor::Empty);
    };
    let seq = last.pointer("/seq").and_then(Value::as_i64);
    let created_at = last.pointer("/createdAt").and_then(Value::as_f64);
    match next_cursor_of(page) {
        LogCursor::After {
            seq: cursor_seq,
            created_at: cursor_created_at,
        } => Some(cursor_seq) == seq && Some(cursor_created_at) == created_at,
        _ => false,
    }
}
