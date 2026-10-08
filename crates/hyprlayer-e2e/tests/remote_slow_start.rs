//! A host whose server starts slower than the relay used to wait: a debug
//! build hashes its own ~300 MB binary before it binds, as a large profile's
//! migrations or a slow disk would.
#![cfg(unix)]

use std::time::{Duration, Instant};

use hyprlayer_e2e::containers::{Build, RemoteHost};
use hyprlayer_e2e::{eventually, json, Journey, Sandbox};

const SERVE: &str = "hyprlayer-server serve";
const IO: Duration = Duration::from_secs(60);
/// Well inside the relay's 30s listen timeout: a failure reported sooner was
/// seen as the server exiting, not waited out.
const PROMPT: Duration = Duration::from_secs(25);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Docker: cargo test -p hyprlayer-e2e -- --ignored, or the release gate"]
async fn remote_slow_start() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("remote_slow_start", &sandbox);
    let host = RemoteHost::start_with(&sandbox, Build::Debug);

    j.step("race two clients against a cold start");
    let (first, second) = tokio::join!(host.try_relay(), host.try_relay());
    j.event("first relay", json!({ "error": first.as_ref().err() }));
    j.event("second relay", json!({ "error": second.as_ref().err() }));
    j.guard(
        "both clients get through a slow cold start",
        first.is_ok() && second.is_ok(),
        json!({ "first": first.is_ok(), "second": second.is_ok() }),
    );
    let settled = eventually(IO, || host.pids(SERVE).len() == 1).await;
    j.guard(
        "one server serves them both",
        settled,
        json!({ "servers": host.pids(SERVE).len() }),
    );
    for relay in [first, second].into_iter().flatten() {
        relay.hang_up().await;
    }

    j.step("fail to start");
    host.kill(SERVE);
    let stopped = eventually(IO, || host.pids(SERVE).is_empty()).await;
    assert!(stopped, "the first server did not stop");
    host.exec("chmod 000 \"$HOME/.local/share/ai.brightblock.hyprlayer/hyprlayer.db\"");
    let started = Instant::now();
    let failed = host.try_relay().await;
    let elapsed = started.elapsed();
    let error = failed.as_ref().err().cloned().unwrap_or_default();
    j.event("failed relay", json!({ "error": error }));
    j.guard(
        "a server that cannot start is reported as having exited",
        failed.is_err() && error.contains("exited during startup"),
        json!({ "says_exited": error.contains("exited during startup") }),
    );
    j.guard(
        "the report carries the server's own reason",
        error.contains("the server said:") && error.contains("hyprlayer.db"),
        json!({ "names_the_database": error.contains("hyprlayer.db") }),
    );
    j.guard(
        "the failure is reported without waiting out the timeout",
        elapsed < PROMPT,
        json!({ "within_25s": elapsed < PROMPT }),
    );
    host.save_server_logs();

    j.finish();
}
