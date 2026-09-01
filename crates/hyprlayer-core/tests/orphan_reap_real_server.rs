//! Reaping a real `opencode serve` that the app's cleanup never ran on.
//!
//! The unit tests in `process::orphan` drive the decision with injected seams.
//! This drives the real thing: a real server, spawned the way
//! `OpenCodeServer::start` spawns it, left behind the way a `SIGKILL` of the app
//! leaves it, then reaped through the real command-line lookup and the real
//! signal.
//!
//! Ignored by default — it needs the opencode binary on the machine and it
//! spawns a server. Run with:
//!   cargo test -p hyprlayer-core --test orphan_reap_real_server -- --ignored --nocapture
//!
//! Unix only: `process_group` and the signal are unix concepts, and Windows
//! keeps its children in a kill-on-close job object rather than reaping them.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use hyprlayer_core::process::{orphan, spawn_supervised, SupervisedChild};

/// `OPENCODE_BIN` if set, else whatever `opencode` resolves to on PATH. No
/// install layout is assumed, so this works wherever the binary happens to live.
fn opencode_bin() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("OPENCODE_BIN").map(PathBuf::from) {
        return explicit.is_file().then_some(explicit);
    }
    let out = std::process::Command::new("sh")
        .args(["-c", "command -v opencode"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    path.is_file().then_some(path)
}

/// Waiting on the handle is the portable way to observe the child: it behaves
/// the same on every unix and it reaps, where polling `/proc` is Linux-only and
/// would see a zombie's entry outlive the process it describes.
///
/// The handle is only ever watched, never used to kill, so the process is left
/// exactly as an app that died without running its cleanup leaves it. A real
/// orphan is additionally reparented to init, which the reaper does not care
/// about — it signals by pid.
async fn exited_within(child: &mut SupervisedChild, within: Duration) -> bool {
    tokio::time::timeout(within, child.wait()).await.is_ok()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hyprlayer-reap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spawns a real opencode server"]
async fn reaps_a_real_opencode_server_left_behind() {
    let Some(bin) = opencode_bin() else {
        panic!("opencode not found on PATH; set OPENCODE_BIN to run this test");
    };

    let dir = temp_dir("e2e");
    let pid_file = dir.join("opencode-server.pid");
    let port: u16 = 46613;

    // The argv `OpenCodeServer::start` builds, and the record it derives from it.
    let serve_args = [
        "serve".to_string(),
        "--hostname".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    let program = format!(
        "{} {}",
        bin.file_name().and_then(|n| n.to_str()).unwrap(),
        serve_args.join(" ")
    );

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(&serve_args)
        .env("XDG_DATA_HOME", dir.join("data"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.process_group(0);

    let mut child = spawn_supervised(&mut cmd).expect("spawn opencode serve");
    let pid = child.id().expect("child pid");
    orphan::record(&pid_file, pid, &program);

    assert!(
        !exited_within(&mut child, Duration::from_millis(1500)).await,
        "server should still be running before we reap it"
    );

    let reaped = orphan::reap(&pid_file);

    assert_eq!(reaped, Some(pid), "reap must report the pid it signalled");
    assert!(
        exited_within(&mut child, Duration::from_secs(10)).await,
        "the real server must actually die, not merely be signalled"
    );
    assert!(!pid_file.exists(), "the record must not survive the reap");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spawns a real process"]
async fn spares_a_real_process_that_is_not_what_was_recorded() {
    let dir = temp_dir("spare");
    let pid_file = dir.join("opencode-server.pid");

    let mut cmd = tokio::process::Command::new("sleep");
    cmd.arg("30").stdin(Stdio::null()).stdout(Stdio::null());
    cmd.process_group(0);
    let mut child = spawn_supervised(&mut cmd).expect("spawn sleep");
    let pid = child.id().expect("child pid");

    // A pid the record claims is our server, running something else entirely.
    orphan::record(
        &pid_file,
        pid,
        "opencode serve --hostname 127.0.0.1 --port 1",
    );

    let reaped = orphan::reap(&pid_file);

    assert_eq!(reaped, None, "must not signal an unrelated process");
    assert!(
        !exited_within(&mut child, Duration::from_secs(2)).await,
        "the bystander must still be running"
    );
    assert!(!pid_file.exists());

    child.kill_blocking();
    let _ = child.wait().await;
    let _ = std::fs::remove_dir_all(&dir);
}
