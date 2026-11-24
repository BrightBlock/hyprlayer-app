//! A client attaching while a text block streams sees what the block already
//! said: partial text is never stored, so the replay alone would start it mid-sentence.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::EventStream;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const QUIET: Duration = Duration::from_millis(800);

/// Streams two deltas, holds the block open until `<sandbox>/release` exists, then finishes it.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
gate="$(dirname "$0")/../release"
delta() { printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"%s"}},"session_id":"%s","parent_tool_use_id":null}\n' "$1" "$sid"; }
while IFS= read -r line; do
  printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
  delta "Rivers carve "
  delta "their patience "
  # Left in place: the title request runs this script too and waits on the same gate.
  while [ ! -f "$gate" ]; do sleep 0.05; done
  delta "into stone."
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Rivers carve their patience into stone."}]},"session_id":"%s"}\n' "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1}}\n' "$sid"
done
"#;

const PREFIX: &str = "Rivers carve their patience ";
const FULL: &str = "Rivers carve their patience into stone.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_attach() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let gate = sandbox.root.join("release");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("streaming_attach", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Streaming attach").await;
    let ws = text(&view, "/item/id");

    j.step("a turn streams half a text block and holds it open");
    let (session, _prompt, _stream_id, mut first) = client
        .start_stage_session_stream(
            ws,
            Stage::General,
            "E2E_STREAM".into(),
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
    let mut first_seen = recv_until(&mut first, |seen| partial_text(seen) == PREFIX).await;
    j.guard(
        "the first client has the open block's prefix",
        partial_text(&first_seen) == PREFIX,
        json!({ "partial": partial_text(&first_seen) }),
    );

    j.step("a client resuming mid-block gets the prefix after the replay");
    let (_, _, _, mut late) = client
        .resume_session_stream(session.id.clone(), None, None, None, Some(LogCursor::Empty))
        .await
        .expect("resume_session");
    // The block is held open, so any partial text here can only be the seed.
    let late_before = recv_until(&mut late, |seen| partial_text(seen) == PREFIX).await;
    let replayed_user = late_before
        .iter()
        .position(|e| matches!(e, SessionEvent::UserMessage { .. }));
    let first_partial = late_before
        .iter()
        .position(|e| matches!(e, SessionEvent::PartialText { .. }));
    j.guard(
        "the streamed prefix arrives before any new delta",
        partial_text(&late_before) == PREFIX,
        json!({ "partial": partial_text(&late_before) }),
    );
    j.guard(
        "the prefix follows the replayed history",
        matches!((replayed_user, first_partial), (Some(u), Some(p)) if u < p),
        json!({ "events": shape(&late_before) }),
    );

    j.step("the block finishes on both streams");
    std::fs::write(&gate, "").expect("release the block");
    let late_rest = recv_until(&mut late, |seen| seen.iter().any(is_turn_done)).await;
    first_seen.extend(recv_until(&mut first, |seen| seen.iter().any(is_turn_done)).await);
    let late_all: Vec<_> = late_before.iter().chain(&late_rest).cloned().collect();
    j.guard(
        "the late client's deltas add up to the saved text, nothing doubled",
        partial_text(&late_all) == FULL,
        json!({ "partial": partial_text(&late_all) }),
    );
    j.guard(
        "the first client's deltas are untouched by the late attach",
        partial_text(&first_seen) == FULL,
        json!({ "partial": partial_text(&first_seen) }),
    );
    j.guard(
        "both clients receive the saved text",
        assistant_text(&late_all) == Some(FULL) && assistant_text(&first_seen) == Some(FULL),
        json!({ "late": shape(&late_all), "first": shape(&first_seen) }),
    );

    j.step("a client attaching after the block was saved gets no partial text");
    let (_, _, _, mut after) = client
        .resume_session_stream(session.id.clone(), None, None, None, Some(LogCursor::Empty))
        .await
        .expect("resume_session");
    let after_seen = quiet(&mut after).await;
    j.guard(
        "a saved block is not sent again as partial text",
        !after_seen
            .iter()
            .any(|e| matches!(e, SessionEvent::PartialText { .. })),
        json!({ "events": shape(&after_seen) }),
    );

    j.finish();
}

fn partial_text(events: &[SessionEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::PartialText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn assistant_text(events: &[SessionEvent]) -> Option<&str> {
    events.iter().find_map(|e| match e {
        SessionEvent::AssistantText { text, .. } => Some(text.as_str()),
        _ => None,
    })
}

fn is_turn_done(e: &SessionEvent) -> bool {
    matches!(e, SessionEvent::TurnDone { .. })
}

fn shape(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            SessionEvent::PartialText { text, .. } => format!("partialText:{text}"),
            SessionEvent::AssistantText { text, .. } => format!("assistantText:{text}"),
            other => other.kind().to_string(),
        })
        .collect()
}

async fn recv_until(
    events: &mut EventStream<SessionEvent>,
    stop: impl Fn(&[SessionEvent]) -> bool,
) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            seen.push(event);
            if stop(&seen) {
                return;
            }
        }
    })
    .await;
    seen
}

async fn quiet(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(QUIET, events.recv()).await {
        seen.push(event);
    }
    seen
}
