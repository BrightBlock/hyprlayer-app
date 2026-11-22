//! Two devices on one server, each watching a worktree: each hears the
//! workstream it watches and nothing else, a shared watch survives one device
//! moving off it, and a device leaving releases only its own watches.
#![cfg(unix)]

use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, eventually, json, text, Journey, Request, RuntimeEvent, Sandbox, Server,
};
use hyprlayer_server::client::RpcClient;
use tokio::sync::mpsc::Receiver;

const HEARD: Duration = Duration::from_secs(20);
/// Every audience's copy of an event is queued at once, so once one device has
/// heard it, any copy another device was sent arrives well within this.
const SETTLE: Duration = Duration::from_secs(1);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worktree_watch_per_connection() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("worktree_watch_per_connection", &sandbox);
    let server = Server::start(&sandbox).await;
    let desktop = server.client();
    let mut desktop_rx = desktop.take_events().expect("the desktop's runtime events");
    let (phone, phone_socket) = killable(server.socket()).await;
    let mut phone_rx = phone.take_events().expect("the phone's runtime events");

    let project = onboard_and_import(&mut j, &desktop, &sandbox, &repo).await;
    let a_view = create_workstream(&mut j, &desktop, &project, "Desktop work").await;
    let b_view = create_workstream(&mut j, &desktop, &project, "Phone work").await;
    let (a, a_path) = (
        text(&a_view, "/item/id"),
        text(&a_view, "/item/worktreePath"),
    );
    let (b, b_path) = (
        text(&b_view, "/item/id"),
        text(&b_view, "/item/worktreePath"),
    );
    j.alias(&a, "$WS_A");
    j.alias(&b, "$WS_B");

    j.step("each device hears the workstream it watches, and only that one");
    watch(&mut j, &desktop, Some(&a)).await;
    watch(&mut j, &phone, Some(&b)).await;
    settle(&mut [&mut desktop_rx, &mut phone_rx]).await;
    touch(&a_path, "one.txt");
    touch(&b_path, "one.txt");
    let desktop_heard = heard(&mut desktop_rx, &a).await;
    let phone_heard = heard(&mut phone_rx, &b).await;
    let desktop_heard = desktop_heard.or(&drain(&mut desktop_rx).await);
    let phone_heard = phone_heard.or(&drain(&mut phone_rx).await);
    j.guard(
        "the desktop still hears A after the phone watches B",
        desktop_heard.has(&a),
        json!({ "desktop_heard_a": desktop_heard.has(&a) }),
    );
    j.guard(
        "the phone hears B, which it watches",
        phone_heard.has(&b),
        json!({ "phone_heard_b": phone_heard.has(&b) }),
    );
    j.guard(
        "neither device hears the workstream only the other watches",
        !desktop_heard.has(&b) && !phone_heard.has(&a),
        json!({ "desktop_heard_b": desktop_heard.has(&b), "phone_heard_a": phone_heard.has(&a) }),
    );

    j.step("a shared watch survives one device moving off it");
    watch(&mut j, &phone, Some(&a)).await;
    settle(&mut [&mut desktop_rx, &mut phone_rx]).await;
    touch(&a_path, "two.txt");
    let both = heard(&mut desktop_rx, &a).await.has(&a) && heard(&mut phone_rx, &a).await.has(&a);
    j.guard(
        "both devices hear A while both watch it",
        both,
        json!({ "both_heard_a": both }),
    );
    watch(&mut j, &phone, Some(&b)).await;
    settle(&mut [&mut desktop_rx, &mut phone_rx]).await;
    touch(&a_path, "three.txt");
    let desktop_heard = heard(&mut desktop_rx, &a).await;
    let phone_heard = drain(&mut phone_rx).await;
    j.guard(
        "the desktop still hears A after the phone moves to B",
        desktop_heard.has(&a),
        json!({ "desktop_heard_a": desktop_heard.has(&a) }),
    );
    j.guard(
        "the phone stops hearing A once it moves off it",
        !phone_heard.has(&a),
        json!({ "phone_heard_a": phone_heard.has(&a) }),
    );

    j.step("a device leaving releases only its own watches");
    // The phone's move off B above logged one already.
    let released = format!("worktree watch on {b}: released");
    let releases = || sandbox.server_log().matches(&released).count();
    let before = releases();
    phone_socket
        .shutdown(Shutdown::Both)
        .expect("close the phone's socket");
    let phone_gone =
        tokio::time::timeout(HEARD, async { while phone_rx.recv().await.is_some() {} })
            .await
            .is_ok();
    j.guard(
        "the phone sees its connection end",
        phone_gone,
        json!({ "phone_gone": phone_gone }),
    );
    let b_released = eventually(HEARD, || releases() > before).await;
    j.guard(
        "the server releases B, which only the phone watched",
        b_released,
        json!({ "b_released": b_released }),
    );
    settle(&mut [&mut desktop_rx]).await;
    touch(&a_path, "four.txt");
    let desktop_heard = heard(&mut desktop_rx, &a).await;
    j.guard(
        "the desktop still hears A after the phone disconnects",
        desktop_heard.has(&a),
        json!({ "desktop_heard_a": desktop_heard.has(&a) }),
    );

    j.step("a device that stops watching hears nothing");
    let laptop = server.connect().await;
    let mut laptop_rx = laptop.take_events().expect("the laptop's runtime events");
    watch(&mut j, &laptop, Some(&a)).await;
    watch(&mut j, &desktop, None).await;
    settle(&mut [&mut desktop_rx, &mut laptop_rx]).await;
    touch(&a_path, "five.txt");
    let laptop_heard = heard(&mut laptop_rx, &a).await;
    let desktop_heard = drain(&mut desktop_rx).await;
    j.guard(
        "a fresh connection watching A hears it",
        laptop_heard.has(&a),
        json!({ "laptop_heard_a": laptop_heard.has(&a) }),
    );
    j.guard(
        "the desktop hears nothing once it unwatches",
        desktop_heard.0.is_empty(),
        json!({ "desktop_heard": desktop_heard.0.len() }),
    );

    j.step("a terminal push reaches the watching device");
    sandbox.git(
        &sandbox.root,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch",
            "main",
            "origin.git",
        ],
    );
    let origin = sandbox.root.join("origin.git").display().to_string();
    sandbox.git(&repo, &["remote", "add", "origin", &origin]);
    settle(&mut [&mut laptop_rx]).await;
    sandbox.git(Path::new(&a_path), &["push", "--quiet", "origin", "HEAD"]);
    let laptop_heard = heard(&mut laptop_rx, &a).await;
    j.guard(
        "a push from A's worktree, which changes only the shared refs, reaches the device watching A",
        laptop_heard.has(&a),
        json!({ "laptop_heard_a": laptop_heard.has(&a) }),
    );

    j.finish();
}

/// The workstream ids of the `WorktreeChanged` events a device heard.
struct Heard(Vec<String>);

impl Heard {
    fn has(&self, id: &str) -> bool {
        self.0.iter().any(|seen| seen == id)
    }

    fn or(mut self, more: &Heard) -> Heard {
        self.0.extend(more.0.iter().cloned());
        self
    }
}

fn worktree_changes(events: Vec<RuntimeEvent>) -> Heard {
    Heard(
        events
            .into_iter()
            .filter_map(|e| match e {
                RuntimeEvent::WorktreeChanged { workstream_id } => Some(workstream_id),
                _ => None,
            })
            .collect(),
    )
}

async fn heard(rx: &mut Receiver<RuntimeEvent>, id: &str) -> Heard {
    worktree_changes(
        events_until(
            rx,
            HEARD,
            |e| matches!(e, RuntimeEvent::WorktreeChanged { workstream_id } if workstream_id == id),
        )
        .await,
    )
}

async fn drain(rx: &mut Receiver<RuntimeEvent>) -> Heard {
    worktree_changes(events_until(rx, SETTLE, |_| false).await)
}

/// Lets a new watch arm and a previous write's events land, then discards them.
async fn settle(rxs: &mut [&mut Receiver<RuntimeEvent>]) {
    tokio::time::sleep(SETTLE).await;
    for rx in rxs {
        while rx.try_recv().is_ok() {}
    }
}

/// The request an older desktop sends too: one workstream, or none.
async fn watch(j: &mut Journey, client: &RpcClient, id: Option<&str>) {
    j.call(
        client,
        Request::WatchWorktree {
            workstream_id: id.map(str::to_string),
        },
    )
    .await
    .expect("watch_worktree");
}

fn touch(worktree: &str, name: &str) {
    std::fs::write(Path::new(worktree).join(name), "x\n").expect("write into the worktree");
}

/// A connection the test can drop from under its client, the way a phone loses its link.
async fn killable(socket: &Path) -> (Arc<RpcClient>, UnixStream) {
    let stream = UnixStream::connect(socket).expect("connect to the sandbox socket");
    let kill = stream.try_clone().expect("clone the socket");
    stream.set_nonblocking(true).expect("nonblocking socket");
    let stream = tokio::net::UnixStream::from_std(stream).expect("register the socket");
    let client = RpcClient::connect(stream).await.expect("handshake");
    (client, kill)
}
