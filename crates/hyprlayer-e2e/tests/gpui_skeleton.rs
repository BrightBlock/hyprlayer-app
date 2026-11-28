//! The GPUI client's skeleton against the real server: the desktop's entities in a
//! `TestAppContext`, attached through a proxy that can stall the server's half of the link, so
//! the event gap and the server kill are both real. Needs `--features gpui`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext};
use hyprlayer_client::hosts::Target;
use hyprlayer_client::paths::AppPaths;
use hyprlayer_desktop::config::Config;
use hyprlayer_desktop::desktop::Desktop;
use hyprlayer_desktop::io::Io;
use hyprlayer_desktop::shell;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Sandbox, Server, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

const SETTLE: Duration = Duration::from_secs(30);
/// Past the server's 256-frame queue plus a Linux socket buffer (~208 KiB) of ~150-byte events.
const FLOOD_EVENTS: usize = 4000;
const FLOOD_WORKERS: usize = 16;

/// Forwards each desktop connection to the server's socket; `stalled` stops the server's half.
struct Proxy {
    stalled: Arc<AtomicBool>,
}

impl Proxy {
    fn start(io: &Io, listen: &Path, upstream: PathBuf) -> Self {
        std::fs::create_dir_all(listen.parent().expect("socket dir"))
            .expect("create the proxy dir");
        let listener = io
            .handle()
            .block_on(async { tokio::net::UnixListener::bind(listen) })
            .expect("bind the proxy socket");
        let stalled = Arc::new(AtomicBool::new(false));
        let pipes = stalled.clone();
        io.handle().spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                // With the server down, dropping `down` is the refused connect the desktop sees.
                if let Ok(up) = UnixStream::connect(&upstream).await {
                    tokio::spawn(pipe(down, up, pipes.clone()));
                }
            }
        });
        Self { stalled }
    }
}

async fn pipe(down: UnixStream, up: UnixStream, stalled: Arc<AtomicBool>) {
    let (mut down_rd, mut down_wr) = down.into_split();
    let (mut up_rd, mut up_wr) = up.into_split();
    let to_server = async {
        let _ = tokio::io::copy(&mut down_rd, &mut up_wr).await;
    };
    let to_desktop = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            while stalled.load(Relaxed) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            match up_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) if down_wr.write_all(&buf[..n]).await.is_err() => break,
                Ok(_) => {}
            }
        }
    };
    tokio::select! {
        () = to_server => {}
        () = to_desktop => {}
    }
}

/// Runs the foreground until `done` holds or `within` passes, and returns the last dump. The
/// io thread wakes GPUI tasks from outside the test thread, so this polls rather than parks.
fn wait(
    cx: &mut TestAppContext,
    desktop: &Entity<Desktop>,
    within: Duration,
    done: impl Fn(&Value) -> bool,
) -> (bool, Value) {
    let deadline = Instant::now() + within;
    loop {
        cx.run_until_parked();
        let dump = cx.read(|cx| desktop.read(cx).dump(cx));
        if done(&dump) {
            return (true, dump);
        }
        if Instant::now() >= deadline {
            return (false, dump);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn state(dump: &Value) -> &str {
    dump["connection"]["state"].as_str().unwrap_or_default()
}

fn labels(dump: &Value, list: &str) -> Vec<String> {
    dump["lists"][list]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["label"].as_str().map(str::to_string))
        .collect()
}

fn keys(dump: &Value, list: &str) -> Vec<String> {
    dump["lists"][list]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["key"].as_str().map(str::to_string))
        .collect()
}

fn painted(dump: &Value, id: &str) -> bool {
    dump["painted"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|p| p.as_str() == Some(id)))
}

fn fetches(dump: &Value) -> [u64; 3] {
    ["projects", "workstreams", "sessions"].map(|list| dump["fetches"][list].as_u64().unwrap_or(0))
}

fn count(dump: &Value, key: &str) -> u64 {
    dump[key].as_u64().unwrap_or(0)
}

#[test]
fn gpui_skeleton() {
    let io = Io::start();
    let sandbox = Sandbox::new();
    let mut j = Journey::new("gpui_skeleton", &sandbox);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut server = io.handle().block_on(Server::start(&sandbox));
    let client = server.client();
    let project = io
        .handle()
        .block_on(onboard_and_import(&mut j, &client, &sandbox, &repo));

    let proxy_dir = sandbox.root.join("proxy");
    let proxy = Proxy::start(
        &io,
        &hyprlayer_server::socket::bind_path(&proxy_dir),
        sandbox.socket(),
    );
    let desktop_dir = sandbox.root.join("desktop");
    let paths = AppPaths {
        config: desktop_dir.join("config"),
        data: desktop_dir.join("data"),
        log: desktop_dir.join("logs"),
    };
    let config = Config::new(
        "ai.brightblock.hyprlayer.e2e".into(),
        paths,
        Some(Target::LocalProfile(proxy_dir)),
    )
    .expect("prepare the desktop's profile");

    let mut cx = TestAppContext::single();
    // The io thread wakes foreground tasks, which the test scheduler allows only once parking is.
    cx.executor().allow_parking();
    cx.update(shell::init);
    let window = cx.add_window(|window, cx| Desktop::new(config, io.clone(), window, cx));
    let desktop = window.root(&mut cx).expect("the window's root view");

    j.step("the desktop attaches and lists the imported project");
    let (listed, dump) = wait(&mut cx, &desktop, SETTLE, |d| {
        state(d) == "connected" && keys(d, "projects").contains(&project)
    });
    j.guard(
        "it attaches to the running server through hyprlayer-client",
        state(&dump) == "connected",
        json!({ "connection": dump["connection"] }),
    );
    j.guard(
        "the server answered the announce",
        dump["transcriptSync"] == json!(true),
        json!({ "transcriptSync": dump["transcriptSync"] }),
    );
    j.guard(
        "the project is listed",
        listed,
        json!({ "projects": dump["lists"]["projects"] }),
    );
    let row = format!("projects:{project}");
    j.guard(
        "its row is painted",
        painted(&dump, &row),
        json!({ "row": row }),
    );

    j.step("a WorkstreamsChanged re-reads the workstreams list");
    let before = fetches(&dump);
    let created =
        io.handle()
            .block_on(create_workstream(&mut j, &client, &project, "GPUI refresh"));
    let ws = text(&created, "/item/id");
    let (refreshed, dump) = wait(&mut cx, &desktop, SETTLE, |d| {
        labels(d, "workstreams").contains(&"GPUI refresh".to_string())
    });
    j.guard(
        "the new workstream is listed",
        refreshed,
        json!({ "workstreams": labels(&dump, "workstreams") }),
    );
    j.guard(
        "by a re-read the event asked for",
        fetches(&dump)[1] > before[1],
        json!({}),
    );
    let row = format!("workstreams:{ws}");
    j.guard(
        "its row is painted",
        painted(&dump, &row),
        json!({ "row": row }),
    );

    j.step("a gap in runtime events re-reads every list");
    let (before, gaps, connects) = (
        fetches(&dump),
        count(&dump, "gaps"),
        count(&dump, "connects"),
    );
    proxy.stalled.store(true, Relaxed);
    let flood = server_flood(&io, &server, &project);
    proxy.stalled.store(false, Relaxed);
    let (reread, dump) = wait(&mut cx, &desktop, SETTLE, |d| {
        count(d, "gaps") > gaps && fetches(d).iter().zip(before).all(|(now, was)| *now > was)
    });
    j.note(
        "gap",
        json!({ "events_sent": flood, "fetches_before": before, "fetches_after": fetches(&dump) }),
    );
    j.guard(
        "the stalled link drops events, and the desktop hears the gap",
        count(&dump, "gaps") > gaps,
        json!({ "flooded_with": "LeadQueueChanged, which no list listens to" }),
    );
    j.guard("every list is read again after it", reread, json!({}));
    j.guard(
        "over the same connection",
        count(&dump, "connects") == connects && state(&dump) == "connected",
        json!({ "connection": dump["connection"] }),
    );

    j.step("the desktop shows recovery when its server is killed, then reconnects");
    let connects = count(&dump, "connects");
    let killed = unsafe { libc::kill(server.pid() as i32, libc::SIGKILL) } == 0;
    let reaped = io
        .handle()
        .block_on(server.exited(Duration::from_secs(10)))
        .is_some();
    let (recovering, dump) = wait(&mut cx, &desktop, SETTLE, |d| {
        state(d) == "lost" && painted(d, "recovery")
    });
    j.guard("the server is gone", killed && reaped, json!({}));
    j.guard(
        "the recovery screen replaces the lists",
        recovering && painted(&dump, "recovery-retry") && !painted(&dump, "projects"),
        json!({ "state": state(&dump) }),
    );
    j.guard(
        "no list from the lost connection is reported as current",
        ["projects", "workstreams", "sessions"]
            .iter()
            .all(|list| dump["lists"][list].is_null()),
        json!({ "lists": dump["lists"] }),
    );
    let lost_at = Instant::now();
    drop(server);
    let server = io.handle().block_on(Server::start(&sandbox));
    let (back, dump) = wait(&mut cx, &desktop, Duration::from_secs(60), |d| {
        state(d) == "connected"
            && count(d, "connects") > connects
            && keys(d, "projects").contains(&project)
            && labels(d, "workstreams").contains(&"GPUI refresh".to_string())
            && !painted(d, "recovery")
    });
    j.note(
        "reconnect",
        json!({ "seconds_after_kill": lost_at.elapsed().as_secs_f64(), "connects": count(&dump, "connects") }),
    );
    j.guard(
        "it reconnects on its own pacing",
        state(&dump) == "connected" && count(&dump, "connects") > connects,
        json!({ "connection": dump["connection"] }),
    );
    j.guard(
        "the lists are read again on the new connection",
        back,
        json!({ "projects": keys(&dump, "projects"), "workstreams": labels(&dump, "workstreams") }),
    );
    j.guard(
        "the recovery screen is gone",
        !painted(&dump, "recovery") && painted(&dump, "projects"),
        json!({}),
    );

    cx.update(|cx| cx.quit());
    drop(server);
    j.finish();
}

/// `reorder_lead_queue` emits one `LeadQueueChanged` per call from another connection, which
/// never announced and so is never sent them itself.
fn server_flood(io: &Io, server: &Server, project: &str) -> usize {
    io.handle().block_on(async {
        let flooder = server.connect().await;
        let workers: Vec<_> = (0..FLOOD_WORKERS)
            .map(|_| {
                let (flooder, project) = (flooder.clone(), project.to_string());
                tokio::spawn(async move {
                    let mut sent = 0;
                    for _ in 0..FLOOD_EVENTS / FLOOD_WORKERS {
                        if flooder
                            .reorder_lead_queue(project.clone(), Vec::new())
                            .await
                            .is_ok()
                        {
                            sent += 1;
                        }
                    }
                    sent
                })
            })
            .collect();
        let mut sent = 0;
        for worker in workers {
            sent += worker.await.unwrap_or(0);
        }
        sent
    })
}
