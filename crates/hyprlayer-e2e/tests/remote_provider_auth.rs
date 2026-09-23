//! Provider credentials set through a relay belong to the host that runs the
//! agents, not to the client that typed them. Until 2026-09-19 the OpenCode
//! auth and config files were read and written on the client's own disk, so a
//! key added in Settings never reached the remote server (ADR-0008).
//!
//! This covers the RPC path only. The bug lived in the desktop shell, which
//! bypassed these commands entirely; no harness that speaks RPC directly can
//! reproduce it. What is locked here is that the commands exist and resolve
//! their paths on the server.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_e2e::containers::RemoteHost;
use hyprlayer_e2e::{eventually, json, Journey, Request, Sandbox, Value};

const IO: Duration = Duration::from_secs(20);
const SERVE: &str = "hyprlayer-server serve";
const AUTH: &str = "$HOME/.local/share/opencode/auth.json";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Docker: cargo test -p hyprlayer-e2e -- --ignored"]
async fn remote_provider_auth() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("remote_provider_auth", &sandbox);
    let host = RemoteHost::start(&sandbox);

    j.step("a fresh host holds no credentials");
    let relay = host.relay().await;
    let listed = j
        .call(&relay.client, Request::OpencodeAuthList {})
        .await
        .expect("opencode_auth_list");
    j.guard(
        "a fresh host lists no providers",
        providers(&listed).is_empty(),
        json!({ "providers": providers(&listed) }),
    );
    j.guard(
        "no auth file exists on the host yet",
        host_missing(&host, AUTH),
        json!({ "host_auth": host.exec(&exists(AUTH)) }),
    );

    j.step("set a key through the relay");
    let set = j
        .call(
            &relay.client,
            Request::OpencodeSetProviderKey {
                provider: "anthropic".into(),
                key: "sk-e2e-not-a-real-key".into(),
            },
        )
        .await
        .expect("opencode_set_provider_key");
    j.guard(
        "the response lists the provider it just stored",
        providers(&set) == vec!["anthropic".to_string()],
        json!({ "providers": providers(&set) }),
    );
    j.guard(
        "the key lands on the host that runs the agents",
        host.exec(&format!("cat {AUTH} 2>/dev/null"))
            .contains("sk-e2e-not-a-real-key"),
        json!({ "host_auth": host.exec(&exists(AUTH)) }),
    );
    j.guard(
        "the host file is not world-readable",
        host.exec(&format!("stat -c %a {AUTH} 2>/dev/null")).trim() == "600",
        json!({ "mode": host.exec(&format!("stat -c %a {AUTH} 2>/dev/null")).trim() }),
    );

    j.step("refuse an unsupported provider");
    let refused = j
        .call(
            &relay.client,
            Request::OpencodeSetProviderKey {
                provider: "github-copilot".into(),
                key: "sk-e2e-copilot".into(),
            },
        )
        .await;
    let evidence = match &refused {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "an unsupported provider is refused",
        refused.is_err(),
        evidence,
    );
    let after_refusal = host.exec(&format!("cat {AUTH} 2>/dev/null"));
    j.guard(
        "a refused provider writes nothing",
        !after_refusal.contains("github-copilot") && !after_refusal.contains("sk-e2e-copilot"),
        json!({ "mentions_refused_provider": after_refusal.contains("github-copilot") }),
    );

    j.step("the key outlives the server");
    let before = host.pids(SERVE);
    host.kill(SERVE);
    let dead = eventually(IO, || host.pids(SERVE).is_empty()).await;
    j.guard(
        "the host's server is gone",
        dead,
        json!({ "servers": host.pids(SERVE).len() }),
    );
    relay.hang_up().await;
    let relay = host.relay().await;
    j.guard(
        "the next relay starts a fresh server",
        host.pids(SERVE) != before,
        json!({ "new_process": host.pids(SERVE) != before }),
    );
    let survived = j
        .call(&relay.client, Request::OpencodeAuthList {})
        .await
        .expect("opencode_auth_list");
    j.guard(
        "the credential survives the server it was set on",
        providers(&survived) == vec!["anthropic".to_string()],
        json!({ "providers": providers(&survived) }),
    );

    j.step("remove it");
    let removed = j
        .call(
            &relay.client,
            Request::OpencodeRemoveProvider {
                provider: "anthropic".into(),
            },
        )
        .await
        .expect("opencode_remove_provider");
    j.guard(
        "removal empties the list",
        providers(&removed).is_empty(),
        json!({ "providers": providers(&removed) }),
    );
    j.guard(
        "removal clears the key on the host",
        !host
            .exec(&format!("cat {AUTH} 2>/dev/null"))
            .contains("sk-e2e-not-a-real-key"),
        json!({ "host_auth": host.exec(&format!("cat {AUTH} 2>/dev/null")).trim() }),
    );

    j.step("shutdown");
    relay
        .client
        .request_shutdown()
        .await
        .expect("request a shutdown");
    let stopped = eventually(IO, || host.pids(SERVE).is_empty()).await;
    j.guard(
        "shutdown stops the host's server",
        stopped,
        json!({ "servers": host.pids(SERVE).len() }),
    );
    relay.hang_up().await;
    host.save_server_logs();

    j.finish();
}

fn providers(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|e| e.get("provider").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn exists(path: &str) -> String {
    format!("test -f {path} && echo yes || echo no")
}

fn host_missing(host: &RemoteHost, path: &str) -> bool {
    host.exec(&exists(path)).trim() == "no"
}
