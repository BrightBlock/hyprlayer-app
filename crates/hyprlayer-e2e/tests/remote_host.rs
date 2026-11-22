//! The remote-host path over real SSH against a Linux container: the relay
//! starts the server, work happens on the host, and neither a dropped link nor
//! a killed server loses what the host should keep.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::runtime::projects::ThoughtsSource;
use hyprlayer_core::WorktreeMode;
use hyprlayer_e2e::containers::RemoteHost;
use hyprlayer_e2e::{
    eventually, json, keystrokes, read_until, text, Journey, Request, Sandbox, Value,
};

const IO: Duration = Duration::from_secs(20);
const SERVE: &str = "hyprlayer-server serve";
const RELAY: &str = "hyprlayer-server relay";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Docker: cargo test -p hyprlayer-e2e -- --ignored, or the release gate"]
async fn remote_host() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("remote_host", &sandbox);
    let host = RemoteHost::start(&sandbox);
    host.exec("mkdir -p notes && printf 'remote\\n' > notes/README");

    j.step("connect through the relay");
    let relay = host.relay().await;
    let servers = host.pids(SERVE);
    j.guard(
        "the relay starts a server when none is running",
        servers.len() == 1,
        json!({ "servers": servers.len() }),
    );
    j.guard(
        "the handshake survives the SSH hop",
        relay.client.server_version() == hyprlayer_proto::PROTOCOL_VERSION,
        json!({ "protocol": relay.client.server_version() }),
    );
    j.note("server", json!({ "version": relay.client.server_semver() }));

    j.step("work on the host");
    let bootstrap = j
        .call(&relay.client, Request::GetBootstrap {})
        .await
        .expect("get_bootstrap");
    j.guard(
        "the client sees the host's profile",
        text(&bootstrap, "/default_repos_root") == "/home/e2e/hyprlayer/repos",
        json!({ "default_repos_root": bootstrap["default_repos_root"] }),
    );
    let project = j
        .call(
            &relay.client,
            Request::ImportLocalFolder {
                path: "/home/e2e/notes".into(),
                directory_name: None,
            },
        )
        .await
        .expect("import_local_folder");
    let project_path = text(&project, "/path");
    let view = j
        .call(
            &relay.client,
            Request::CreateWorkstream {
                project_path: project_path.clone(),
                title: "Remote work".into(),
                base: None,
                mode: WorktreeMode::Folder,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let id = text(&view, "/item/id");
    let slug = text(&view, "/item/slug");
    j.alias(&id, "$WS");
    if let Some((stem, _)) = slug.rsplit_once('-') {
        j.alias(&slug, format!("{stem}-$SUFFIX"));
    }
    let listed = j
        .call(
            &relay.client,
            Request::ListWorkstreams {
                project_path: project_path.clone(),
                include_archived: false,
            },
        )
        .await;
    let evidence = match &listed {
        Ok(items) => json!({ "ids": ids_of(items) }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "a folder project lists its workstreams before onboarding",
        listed.is_ok_and(|items| ids_of(&items).contains(&id)),
        evidence,
    );
    j.call(
        &relay.client,
        Request::InitThoughtsRepo {
            user: "e2e".into(),
            source: ThoughtsSource::Create {
                path: "~/thoughts".into(),
            },
        },
    )
    .await
    .expect("init_thoughts_repo");
    let onboarded = host.exec("test -d \"$HOME/thoughts/.git\" && echo yes || echo no");
    j.guard(
        "onboarding creates the thoughts repository on the host",
        onboarded == "yes",
        json!({ "thoughts_repo": onboarded }),
    );
    let (terminal, _, mut events) = relay
        .client
        .terminal_open_stream(id.clone(), 80, 24)
        .await
        .expect("terminal_open");
    j.alias(&terminal, "$TERMINAL");
    j.call(
        &relay.client,
        Request::TerminalWrite {
            terminal_id: terminal.clone(),
            data: keystrokes(
                "printf '%s\\n%s\\n' \"$(uname -s)\" \"$PWD\" > \"$HOME/terminal-where\"; printf 'E2E_%s\\n' OK\n",
            ),
        },
    )
    .await
    .expect("terminal_write");
    read_until(&mut events, "E2E_OK", IO).await;
    let terminal_where = host.exec("cat \"$HOME/terminal-where\" 2>/dev/null || true");
    let mut lines = terminal_where.lines().map(str::to_string);
    let (os, cwd) = (lines.next(), lines.next());
    j.guard(
        "the terminal runs on the host, in the folder",
        os.as_deref() == Some("Linux") && cwd.as_deref() == Some("/home/e2e/notes"),
        json!({ "os": os, "cwd": cwd }),
    );

    j.step("drop the SSH connection");
    let server_before = host.pids(SERVE);
    relay.hang_up().await;
    drop(events);
    let relay_gone = eventually(IO, || host.pids(RELAY).is_empty()).await;
    j.guard(
        "the host's relay exits with its SSH session",
        relay_gone,
        json!({ "relays_left": host.pids(RELAY).len() }),
    );
    let server_after = host.pids(SERVE);
    j.guard(
        "the host's server keeps running",
        server_after.len() == 1 && server_after == server_before,
        json!({ "servers": server_after.len(), "same_process": server_after == server_before }),
    );
    let said = host.relay_stderr(1);
    j.guard(
        "a fresh host's first relay says nothing on stderr",
        said.trim().is_empty(),
        json!({ "stderr": said.trim() }),
    );

    j.step("reconnect over a new SSH session");
    let relay = host.relay().await;
    let attached = relay
        .client
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach");
    j.guard(
        "the terminal survives the dropped connection",
        attached.is_some(),
        json!({ "attached": attached.is_some() }),
    );
    let mut events = attached.expect("a live terminal").events;
    let replay = read_until(&mut events, "E2E_OK", IO).await;
    j.guard(
        "reattaching replays the scrollback",
        replay.contains("E2E_OK"),
        json!({ "replayed_marker": replay.contains("E2E_OK") }),
    );
    j.call(
        &relay.client,
        Request::TerminalWrite {
            terminal_id: terminal.clone(),
            data: keystrokes("printf 'AFTER_%s\\n' RECONNECT\n"),
        },
    )
    .await
    .expect("terminal_write");
    let after = read_until(&mut events, "AFTER_RECONNECT", IO).await;
    j.guard(
        "the terminal takes input after reconnecting",
        after.contains("AFTER_RECONNECT"),
        json!({ "marker": after.contains("AFTER_RECONNECT") }),
    );
    drop(events);

    j.step("kill the host's server");
    let killed = host.pids(SERVE);
    host.kill(SERVE);
    let dead = eventually(IO, || host.pids(SERVE).is_empty()).await;
    j.guard(
        "the host's server is gone",
        dead,
        json!({ "servers": host.pids(SERVE).len() }),
    );
    let mut stale = relay;
    let ended = tokio::time::timeout(IO, async { while stale.events.recv().await.is_some() {} })
        .await
        .is_ok();
    j.guard(
        "the client sees its connection end",
        ended,
        json!({ "ended": ended }),
    );
    stale.hang_up().await;
    let relay = host.relay().await;
    let respawned = host.pids(SERVE);
    j.guard(
        "the next relay starts a fresh server",
        respawned.len() == 1 && respawned != killed,
        json!({ "servers": respawned.len(), "new_process": respawned != killed }),
    );
    let listed = j
        .call(
            &relay.client,
            Request::ListWorkstreams {
                project_path: project_path.clone(),
                include_archived: false,
            },
        )
        .await
        .expect("list_workstreams");
    let ids = ids_of(&listed);
    j.guard(
        "the workstream survives the server's death",
        ids.contains(&id),
        json!({ "ids": ids }),
    );
    let reattach = relay
        .client
        .terminal_attach_stream(terminal.clone(), None)
        .await
        .expect("terminal_attach");
    j.guard(
        "the dead server's terminal is reported gone",
        reattach.is_none(),
        json!({ "attached": reattach.is_some() }),
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

fn ids_of(workstreams: &Value) -> Vec<String> {
    workstreams
        .as_array()
        .map(|items| items.iter().map(|w| text(w, "/item/id")).collect())
        .unwrap_or_default()
}
