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

/// The close is only observable through the server's log, so this allows for
/// the client's teardown, the server noticing, and `non_blocking`'s worker
/// flushing. It was briefly 60s while the intermittent failure below was
/// mistaken for slowness; it is not.
const LOGGED: Duration = Duration::from_secs(15);

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

    j.step("open a terminal from a client, then drop that client");
    let (terminal, before) = on_a_client_that_goes_away(sandbox.socket(), id.clone()).await;
    j.alias(&terminal, "$TERMINAL");
    j.guard(
        "the first client's terminal echoes its input",
        before.contains("BEFORE_DROP"),
        json!({ "marker": before.contains("BEFORE_DROP") }),
    );
    let closed = eventually(LOGGED, || opener_closed(&sandbox.server_log())).await;
    // On failure only, so the golden keeps no per-run connection ids.
    let evidence = if closed {
        json!({ "closed": true })
    } else {
        let log = sandbox.server_log();
        json!({
            "closed": false,
            "opener": opener_conn(&log),
            "end_lines": conn_lines(&log, ": closed"),
            "dropped_lines": conn_lines(&log, ": dropped"),
            "conn_lines": conn_lines(&log, "conn "),
        })
    };
    j.guard(
        "the server sees the first client's connection close",
        closed,
        evidence,
    );

    j.step("reattach from a new client");
    let attached = client
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach");
    j.guard(
        "the terminal survives its client's disconnect",
        attached.is_some(),
        json!({ "attached": attached.is_some() }),
    );
    let mut events = attached.expect("a live terminal").events;
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

    j.step("resize and close the terminal");
    j.call(
        &client,
        Request::TerminalResize {
            terminal_id: terminal.clone(),
            cols: 120,
            rows: 40,
        },
    )
    .await
    .expect("terminal_resize");
    j.call(
        &client,
        Request::TerminalWrite {
            terminal_id: terminal.clone(),
            // `stty size` rather than `tput cols`: the sandbox exports no TERM.
            // The trailing marker is what to wait for — the shell echoes the
            // command line first, so waiting on "SIZE=" matches the echo.
            data: keystrokes(
                "printf 'SIZE=%s\\n' \"$(stty size | tr ' ' x)\"; printf 'DONE_%s\\n' RESIZE\n",
            ),
        },
    )
    .await
    .expect("terminal_write");
    let sized = read_until(&mut events, "DONE_RESIZE", IO).await;
    let reported = sized
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("SIZE="))
        .map(str::to_string);
    j.guard(
        "a resize reaches the pty, not just the server's bookkeeping",
        reported.as_deref() == Some("40x120"),
        json!({ "reported": reported }),
    );
    let closed_terminal = j
        .call(
            &client,
            Request::TerminalClose {
                terminal_id: terminal.clone(),
            },
        )
        .await;
    j.guard(
        "closing a terminal is accepted",
        closed_terminal.is_ok(),
        json!({ "closed": closed_terminal.is_ok() }),
    );
    let gone = client
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach");
    j.guard(
        "a closed terminal cannot be reattached",
        gone.is_none(),
        json!({ "attached": gone.is_some() }),
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
        .terminal_attach_stream(terminal.clone(), None)
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

/// Whether the connection that opened the terminal has logged its end.
///
/// `handle_connection` spells that two ways (`lib.rs`): a clean end logs
/// "closed after", and an end that surfaced an I/O error logs "dropped after".
/// Tearing a client's runtime down abruptly produces either, depending on
/// whether the server's read sees EOF or an error first — so matching only
/// "closed" failed intermittently, and more often the busier the machine.
fn opener_closed(log: &str) -> bool {
    opener_conn(log).is_some_and(|id| {
        log.contains(&format!("conn {id}: closed")) || log.contains(&format!("conn {id}: dropped"))
    })
}

/// The connection id that opened the terminal, if the log names one.
fn opener_conn(log: &str) -> Option<String> {
    log.lines().find_map(|line| {
        let rest = &line[line.find("conn ")? + 5..];
        let (id, event) = rest.split_once(':')?;
        event
            .contains("terminal_open opened stream")
            .then(|| id.to_string())
    })
}

/// The tail of the log lines mentioning `needle`, for a failure to explain
/// itself without a kept sandbox.
fn conn_lines(log: &str, needle: &str) -> Vec<String> {
    let all: Vec<String> = log
        .lines()
        .filter(|l| l.contains(needle))
        .map(|l| l.trim().to_string())
        .collect();
    all.into_iter().rev().take(20).collect()
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
