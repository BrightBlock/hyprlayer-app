//! A required tool installed outside the app is found on its own: nobody
//! presses Re-detect or restarts anything, even where no focus change is ever
//! reported (Hyprland delivers none to WebKitGTK).
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_e2e::{events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server};
use hyprlayer_server::client::RpcClient;

/// A few of the server's checks for a missing tool, then a full reprobe.
const FOUND: Duration = Duration::from_secs(30);
/// More than one check interval, so a spurious event would show up.
const QUIET: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_installed_while_away() {
    let mut sandbox = Sandbox::new();
    // A login shell that adds to PATH sends its own BinariesChanged at startup, whenever it finishes.
    sandbox.set_env("SHELL", "/usr/bin/false");
    let mut j = Journey::new("tool_installed_while_away", &sandbox);

    j.step("start without claude");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events = client.take_events().expect("the client's runtime events");
    let claude = claude_state(&mut j, &client).await;
    j.guard(
        "claude is missing",
        claude == "missing",
        json!({ "claude": claude }),
    );

    j.step("install claude without the window ever losing focus");
    sandbox.install_absent("claude", "#!/bin/sh\nexit 0\n");
    let heard = events_until(&mut events, FOUND, is_binaries_changed).await;
    j.guard(
        "the client hears the binaries changed",
        heard.iter().any(is_binaries_changed),
        json!({ "heard": heard.iter().any(is_binaries_changed) }),
    );
    let claude = claude_state(&mut j, &client).await;
    j.guard(
        "claude is found without a redetect",
        claude == "ok",
        json!({ "claude": claude }),
    );

    j.step("with nothing missing, the server stays quiet, even on a focus change");
    foreground(&mut j, &client, false).await;
    foreground(&mut j, &client, true).await;
    let heard = events_until(&mut events, QUIET, is_binaries_changed).await;
    j.guard(
        "no binaries event",
        !heard.iter().any(is_binaries_changed),
        json!({ "heard": heard.iter().any(is_binaries_changed) }),
    );

    j.finish();
}

async fn foreground(j: &mut Journey, client: &RpcClient, foreground: bool) {
    j.call(client, Request::SetForeground { foreground })
        .await
        .expect("set_foreground");
}

async fn claude_state(j: &mut Journey, client: &RpcClient) -> String {
    let bootstrap = j
        .call(client, Request::GetBootstrap {})
        .await
        .expect("get_bootstrap");
    bootstrap["binaries"]
        .as_array()
        .and_then(|all| all.iter().find(|s| text(s, "/name") == "claude"))
        .map(|s| text(s, "/state/state"))
        .unwrap_or_default()
}

fn is_binaries_changed(event: &RuntimeEvent) -> bool {
    matches!(event, RuntimeEvent::BinariesChanged)
}
