//! ADR-0009's promise: terminals outlive the client that opened them, and the
//! profile's state outlives the server process.
#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    eventually, json, keystrokes, read_until, text, Journey, Request, Sandbox, Server,
};
use hyprlayer_server::client::RpcClient;

const IO: Duration = Duration::from_secs(15);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_outlives_the_client() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("outlives_the_client", &sandbox);

    j.step("set up a workstream");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Long running").await;
    let id = text(&view, "/item/id");
    let worktree = PathBuf::from(text(&view, "/item/worktreePath"));
    j.guard(
        "the workstream exists before any disconnect",
        worktree.is_dir(),
        json!({ "worktree": worktree }),
    );

    j.step("open a terminal from a client, then drop that client");
    let (terminal, before) = on_a_client_that_goes_away(sandbox.socket(), id.clone()).await;
    j.alias(&terminal, "$TERMINAL");
    j.guard(
        "the first client's terminal echoes its input",
        before.contains("BEFORE_DROP"),
        json!({ "marker": before.contains("BEFORE_DROP") }),
    );
    let closed = eventually(IO, || opener_closed(&sandbox.server_log())).await;
    j.guard(
        "the server sees the first client's connection close",
        closed,
        json!({ "closed": closed }),
    );

    j.step("reattach from a new client");
    let attached = client
        .terminal_attach_stream(terminal.clone())
        .await
        .expect("terminal_attach");
    j.guard(
        "the terminal survives its client's disconnect",
        attached.is_some(),
        json!({ "attached": attached.is_some() }),
    );
    let (_, mut events) = attached.expect("a live terminal");
    let replay = read_until(&mut events, "BEFORE_DROP", IO).await;
    j.guard(
        "reattaching replays the scrollback",
        replay.contains("BEFORE_DROP"),
        json!({ "replayed_marker": replay.contains("BEFORE_DROP") }),
    );
    j.call(
        &client,
        Request::TerminalWrite {
            terminal_id: terminal.clone(),
            data: keystrokes("printf 'AFTER_%s\\n' ATTACH\n"),
        },
    )
    .await
    .expect("terminal_write");
    let after = read_until(&mut events, "AFTER_ATTACH", IO).await;
    j.guard(
        "the reattached terminal accepts input",
        after.contains("AFTER_ATTACH"),
        json!({ "marker": after.contains("AFTER_ATTACH") }),
    );

    j.step("restart the server");
    let stopped = server.shutdown(&client).await;
    drop(events);
    j.guard(
        "the first server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    j.guard(
        "a second server binds the same profile",
        true,
        json!({ "socket": server.socket() }),
    );
    let listed = j
        .call(
            &client,
            Request::ListWorkstreams {
                project_path: project.clone(),
                include_archived: false,
            },
        )
        .await
        .expect("list_workstreams");
    let ids: Vec<String> = listed
        .as_array()
        .map(|items| items.iter().map(|w| text(w, "/item/id")).collect())
        .unwrap_or_default();
    j.guard(
        "the workstream survives the restart",
        ids.contains(&id) && worktree.is_dir(),
        json!({ "ids": ids, "worktree_exists": worktree.is_dir() }),
    );
    let reattach = client
        .terminal_attach_stream(terminal.clone())
        .await
        .expect("terminal_attach");
    j.guard(
        "the old terminal is reported gone rather than hanging",
        reattach.is_none(),
        json!({ "attached": reattach.is_some() }),
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

/// Whether the connection that opened the terminal has logged its close.
fn opener_closed(log: &str) -> bool {
    let opener = log.lines().find_map(|line| {
        let rest = &line[line.find("conn ")? + 5..];
        let (id, event) = rest.split_once(':')?;
        event
            .contains("terminal_open opened stream")
            .then(|| id.to_string())
    });
    opener.is_some_and(|id| log.contains(&format!("conn {id}: closed")))
}

/// Opens a terminal, round-trips a marker, then drops the client's whole
/// runtime: dropping an `RpcClient` alone leaves its reader holding the socket
/// open, so only ending every task that owns a half really disconnects.
async fn on_a_client_that_goes_away(socket: PathBuf, workstream: String) -> (String, String) {
    tokio::task::spawn_blocking(move || {
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("build the first client's runtime");
            let outcome = runtime.block_on(async {
                let client = RpcClient::connect_socket(&socket)
                    .await
                    .expect("connect the first client");
                let (terminal, _, mut events) = client
                    .terminal_open_stream(workstream, 80, 24)
                    .await
                    .expect("terminal_open");
                client
                    .terminal_write(terminal.clone(), keystrokes("printf 'BEFORE_%s\\n' DROP\n"))
                    .await
                    .expect("terminal_write");
                let output = read_until(&mut events, "BEFORE_DROP", IO).await;
                (terminal, output)
            });
            runtime.shutdown_timeout(Duration::from_secs(5));
            outcome
        })
        .join()
        .expect("the first client's thread")
    })
    .await
    .expect("join the first client")
}
