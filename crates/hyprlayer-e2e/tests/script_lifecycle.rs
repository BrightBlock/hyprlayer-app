//! A long-running Run script: it keeps going after the client that started it
//! goes quiet, its output is readable without reattaching, stopping it is
//! observable, and the server stops tracking it. `workstream_lifecycle` covers
//! the one-shot Setup and Archive scripts; this covers the one that stays up.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::repo_settings::ScriptSettings;
use hyprlayer_core::ScriptKind;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, read_until, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};

const IO: Duration = Duration::from_secs(20);

/// Prints a ready marker, then a heartbeat, so the scenario can tell "still
/// running" from "exited cleanly at the right moment".
const RUN_SCRIPT: &str = "printf 'RUN_READY\\n'; while :; do printf 'TICK\\n'; sleep 1; done";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn script_lifecycle() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("script_lifecycle", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("configure the scripts");
    j.call(
        &client,
        Request::SetScripts {
            project_path: project.clone(),
            scripts: ScriptSettings {
                setup: String::new(),
                run: RUN_SCRIPT.into(),
                archive: String::new(),
            },
        },
    )
    .await
    .expect("set_scripts");
    let scripts = j
        .call(
            &client,
            Request::GetScripts {
                project_path: project.clone(),
            },
        )
        .await
        .expect("get_scripts");
    j.guard(
        "the run script is stored against the project",
        text(&scripts, "/run") == RUN_SCRIPT,
        json!({ "configured": text(&scripts, "/run") == RUN_SCRIPT }),
    );

    let view = create_workstream(&mut j, &client, &project, "Script work").await;
    let ws = text(&view, "/item/id");

    j.step("start the long-running script");
    let (terminal, _stream_id, mut events) = client
        .script_run_stream(ws.clone(), ScriptKind::Run, 80, 24)
        .await
        .expect("script_run");
    j.alias(&terminal, "$SCRIPT_TERMINAL");
    let ready = read_until(&mut events, "RUN_READY", IO).await;
    j.guard(
        "the script runs and reaches its ready marker",
        ready.contains("RUN_READY"),
        json!({ "ready": ready.contains("RUN_READY") }),
    );
    let running = j
        .call(&client, Request::ListRunningScripts {})
        .await
        .expect("list_running_scripts");
    j.guard(
        "the server lists it as running",
        ids(&running).contains(&ws),
        json!({ "running": ids(&running).len() }),
    );
    let runtime = j
        .call(
            &client,
            Request::GetScriptRuntime {
                workstream_id: ws.clone(),
            },
        )
        .await
        .expect("get_script_runtime");
    j.guard(
        "the runtime names the terminal the script is on",
        text(&runtime, "/runTerminalId") == terminal,
        json!({
            "terminal_matches": text(&runtime, "/runTerminalId") == terminal,
            "statuses": runtime.pointer("/statuses").and_then(Value::as_array).map(|s| s.len()),
        }),
    );

    let started = events_until(&mut events_rx, IO, |e| {
        matches!(e, RuntimeEvent::ScriptRunChanged { workstream_id, .. } if *workstream_id == ws)
    })
    .await;
    j.guard(
        "starting the script announces a run-phase change",
        started
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ScriptRunChanged { workstream_id, .. } if *workstream_id == ws)),
        json!({ "phases": started.iter().filter_map(|e| match e {
            RuntimeEvent::ScriptRunChanged { phase, .. } => Some(format!("{phase:?}")),
            _ => None,
        }).collect::<Vec<_>>() }),
    );

    j.step("the stream is not the script's lifetime");
    // Dropping the stream is what closing the dock does; the script is not the
    // client's to end.
    drop(events);
    let still = j
        .call(&client, Request::ListRunningScripts {})
        .await
        .expect("list_running_scripts");
    j.guard(
        "dropping the stream does not stop the script",
        ids(&still).contains(&ws),
        json!({ "running": ids(&still).len() }),
    );
    let while_running = j
        .call(
            &client,
            Request::GetScriptOutput {
                workstream_id: ws.clone(),
                kind: ScriptKind::Run,
            },
        )
        .await
        .expect("get_script_output");
    // Not an oversight: `set_output` runs on `TerminalEvent::Exited`, from the
    // pty scrollback, so the buffer is the post-mortem record and the stream is
    // the only live view.
    j.guard(
        "a running script has no buffered output yet",
        while_running.is_null(),
        json!({ "buffered": !while_running.is_null() }),
    );

    j.step("stop it");
    j.call(
        &client,
        Request::ScriptStop {
            workstream_id: ws.clone(),
        },
    )
    .await
    .expect("script_stop");
    let stopped = wait_gone(&client, &ws).await;
    j.guard(
        "stopping takes it off the running list",
        stopped,
        json!({ "stopped": stopped }),
    );
    // Leaving the running list and buffering the scrollback are separate
    // steps: `script_stop` drops the registration, and `set_output` only runs
    // when the forwarder sees the pty's `Exited`.
    let ended = events_until(&mut events_rx, IO, |e| {
        matches!(e, RuntimeEvent::ScriptRunChanged { workstream_id, .. } if *workstream_id == ws)
    })
    .await;
    j.guard(
        "stopping announces a terminal run phase",
        ended
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ScriptRunChanged { .. })),
        json!({ "phases": ended.iter().filter_map(|e| match e {
            RuntimeEvent::ScriptRunChanged { phase, .. } => Some(format!("{phase:?}")),
            _ => None,
        }).collect::<Vec<_>>() }),
    );
    let after_exit = wait_output(&client, &ws).await;
    j.guard(
        "the exit buffers what the script printed",
        after_exit.contains("RUN_READY"),
        json!({ "has_marker": after_exit.contains("RUN_READY") }),
    );

    j.step("clear the buffer");
    j.call(
        &client,
        Request::ScriptClearOutput {
            workstream_id: ws.clone(),
            kind: ScriptKind::Run,
            terminal_id: None,
        },
    )
    .await
    .expect("script_clear_output");
    let cleared = j
        .call(
            &client,
            Request::GetScriptOutput {
                workstream_id: ws,
                kind: ScriptKind::Run,
            },
        )
        .await
        .expect("get_script_output");
    j.guard(
        "clearing drops the buffered record",
        cleared.is_null(),
        json!({ "buffered": !cleared.is_null() }),
    );

    j.step("shutdown");
    let down = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        down.status.success() && !down.still_listening,
        json!({ "exit": down.status.code(), "listening": down.still_listening }),
    );

    j.finish();
}

async fn wait_gone(client: &hyprlayer_server::client::RpcClient, ws: &str) -> bool {
    for _ in 0..40 {
        let running = client
            .list_running_scripts()
            .await
            .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        if !ids(&running).contains(&ws.to_string()) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

async fn wait_output(client: &hyprlayer_server::client::RpcClient, ws: &str) -> String {
    for _ in 0..40 {
        if let Ok(Some(out)) = client
            .get_script_output(ws.to_string(), ScriptKind::Run)
            .await
        {
            if !out.is_empty() {
                return out;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    String::new()
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
