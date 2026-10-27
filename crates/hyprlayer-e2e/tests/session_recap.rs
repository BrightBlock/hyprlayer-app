//! A recap covers only a session's last turn, comes from the one-shot model, and
//! makes no model call once recaps are turned off.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::settings::AppSettings;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
const STUB_RECAP: &str = "You asked for a count, and the counter is now 2.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_recap() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("session_recap", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let recap_log = sandbox.root.join("goal-control/recap.log");

    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Recaps").await;
    let ws = text(&view, "/item/id");

    j.step("two turns finish");
    let (session_id, mut events) = start_session(&mut j, &client, &ws).await;
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session_id.clone(),
            text: "E2E_RECAP_SECOND_TURN".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let second = turn_done(&mut events).await;
    j.guard(
        "the second turn finishes",
        second,
        json!({ "turnDone": second }),
    );

    j.step("the recap covers only the last turn");
    let recap = j
        .call(
            &client,
            Request::RecapSession {
                session_id: session_id.clone(),
            },
        )
        .await
        .expect("recap_session");
    j.guard(
        "the recap is the model's reply",
        recap == json!(STUB_RECAP),
        json!({ "recap": recap }),
    );
    let prompt = std::fs::read_to_string(&recap_log).unwrap_or_default();
    let asked_last = prompt.contains("User asked: E2E_RECAP_SECOND_TURN");
    let ends_on_reply = prompt.contains("Agent: counter is 2\n</turn>");
    let skips_first = !prompt.contains("User asked: hello") && !prompt.contains("counter is 1");
    j.guard(
        "the prompt holds the last turn's ask and final reply, and nothing from the first turn",
        asked_last && ends_on_reply && skips_first,
        json!({
            "asksLastTurn": asked_last,
            "endsOnFinalReply": ends_on_reply,
            "skipsFirstTurn": skips_first,
        }),
    );

    j.step("turning recaps off");
    let current = j
        .call(&client, Request::GetAppSettings {})
        .await
        .expect("get_app_settings");
    let mut settings: AppSettings =
        serde_json::from_value(current).expect("decode the current settings");
    settings.recaps.enabled = false;
    let saved = j
        .call(&client, Request::SetAppSettings { settings })
        .await
        .expect("set_app_settings");
    j.guard(
        "the setting is stored",
        saved["recaps"]["enabled"] == json!(false),
        json!({ "recaps": saved["recaps"] }),
    );
    let calls_before = recap_calls(&recap_log);
    let off = j
        .call(&client, Request::RecapSession { session_id })
        .await
        .expect("recap_session");
    let calls_after = recap_calls(&recap_log);
    j.guard(
        "a recap with recaps off is empty and calls no model",
        off.is_null() && calls_after == calls_before,
        json!({ "recap": off, "modelCalls": calls_after - calls_before }),
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

/// A session whose first turn has finished, with its event stream still open.
async fn start_session(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
) -> (String, EventStream<SessionEvent>) {
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
    j.alias(&session.id, "$S1");
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    turn_done(&mut events).await;
    (session.id, events)
}

async fn turn_done(events: &mut EventStream<SessionEvent>) -> bool {
    tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(event, SessionEvent::TurnDone { .. }) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

fn recap_calls(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .map(|s| s.matches("\n---\n").count())
        .unwrap_or(0)
}
