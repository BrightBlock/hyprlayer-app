//! Two clients attached to one server merge their environment overlays per
//! key: a remote client's nonexistent `SSH_AUTH_SOCK` never replaces a working
//! one, and every other key still takes the newest attach's value.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::net::UnixListener;
use std::time::Duration;

use hyprlayer_e2e::{json, read_until, Journey, Request, Sandbox, Server};

const IO: Duration = Duration::from_secs(20);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_env_overlay() {
    let sandbox = Sandbox::new();
    // Short, so the bound path fits macOS's 104-byte sun_path.
    let agent_sock = sandbox.root.join("agent.sock");
    let _agent = UnixListener::bind(&agent_sock).expect("bind agent.sock");
    let missing_sock = sandbox.root.join("missing.sock");

    let mut j = Journey::new("shared_env_overlay", &sandbox);
    let server = Server::start(&sandbox).await;
    let a = server.client();
    let b = server.connect().await;

    j.step("two clients attach with different overlays");
    j.call(
        &a,
        Request::SetEnvOverlay {
            vars: BTreeMap::from([
                (
                    "SSH_AUTH_SOCK".to_string(),
                    agent_sock.display().to_string(),
                ),
                ("DISPLAY".to_string(), ":a".to_string()),
            ]),
        },
    )
    .await
    .expect("set_env_overlay (a)");
    j.call(
        &b,
        Request::SetEnvOverlay {
            vars: BTreeMap::from([
                (
                    "SSH_AUTH_SOCK".to_string(),
                    missing_sock.display().to_string(),
                ),
                ("DISPLAY".to_string(), ":b".to_string()),
            ]),
        },
    )
    .await
    .expect("set_env_overlay (b)");
    let (_, _, mut events) = a
        .spawn_shell_terminal_stream(
            "printf 'sock=%s display=%s|end\\n' \"$SSH_AUTH_SOCK\" \"$DISPLAY\"".into(),
            80,
            24,
        )
        .await
        .expect("spawn a shell terminal");
    let seen = read_until(&mut events, "|end", IO).await;
    j.guard(
        "a nonexistent agent socket never replaces a working one",
        seen.contains(&format!("sock={}", agent_sock.display())),
        json!({ "seen": seen }),
    );
    j.guard(
        "a later attach still wins the keys it sends",
        seen.contains("display=:b"),
        json!({ "seen": seen }),
    );

    j.step("a client with no agent keeps the one that works");
    j.call(
        &b,
        Request::SetEnvOverlay {
            vars: BTreeMap::from([("TERM".to_string(), "xterm-256color".to_string())]),
        },
    )
    .await
    .expect("set_env_overlay (b, remote-shaped)");
    let (_, _, mut events) = a
        .spawn_shell_terminal_stream(
            "printf 'sock=%s display=%s|end\\n' \"$SSH_AUTH_SOCK\" \"$DISPLAY\"".into(),
            80,
            24,
        )
        .await
        .expect("spawn a shell terminal");
    let seen = read_until(&mut events, "|end", IO).await;
    j.guard(
        "the working agent socket is still there",
        seen.contains(&format!("sock={}", agent_sock.display())),
        json!({ "seen": seen }),
    );
    j.guard(
        "the last DISPLAY sent is still there",
        seen.contains("display=:b"),
        json!({ "seen": seen }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&a).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}
