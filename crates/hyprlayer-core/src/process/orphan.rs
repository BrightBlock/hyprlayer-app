//! Reaping a long-lived child that outlived the app.
//!
//! `spawn_supervised` keeps a child tree tied to this process on Windows through a
//! kill-on-close job object, and `kill_on_drop` covers the ordinary quit path
//! everywhere. Neither runs after a `SIGKILL` or a crash, so on unix a server the
//! app spawned is reparented to init and survives. Recording its pid lets the next
//! run finish the job.

use std::path::Path;

struct Record {
    pid: u32,
    program: String,
}

/// Note the pid of a child that must not outlive the app. `program` is matched
/// against the live process's command line before anything is signalled, so a
/// reused pid belonging to an unrelated process is left alone.
pub fn record(pid_file: &Path, pid: u32, program: &str) {
    if let Some(parent) = pid_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(pid_file, format!("{pid}\n{program}")) {
        tracing::warn!("could not record orphan pid at {}: {e}", pid_file.display());
    }
}

/// Forget a child that was shut down through the normal path.
pub fn clear(pid_file: &Path) {
    let _ = std::fs::remove_file(pid_file);
}

/// Kill a child recorded by a previous run, if it is still the process we
/// recorded. Returns the pid that was signalled.
pub fn reap(pid_file: &Path) -> Option<u32> {
    reap_with(pid_file, process_cmdline, kill_group)
}

fn reap_with(
    pid_file: &Path,
    cmdline: impl Fn(u32) -> Option<String>,
    kill: impl Fn(u32),
) -> Option<u32> {
    let record = read_record(pid_file);
    // Drop the record either way: a pid we decline to kill is not worth
    // re-examining on every future launch.
    clear(pid_file);
    let record = record?;
    if record.pid <= 1 {
        return None;
    }
    let live = cmdline(record.pid)?;
    if !live.contains(&record.program) {
        tracing::info!(
            "orphan pid {} is now an unrelated process; not signalling it",
            record.pid
        );
        return None;
    }
    tracing::warn!(
        "reaping orphan process {} ({}) left by a previous run",
        record.pid,
        record.program
    );
    kill(record.pid);
    Some(record.pid)
}

fn read_record(pid_file: &Path) -> Option<Record> {
    let raw = std::fs::read_to_string(pid_file).ok()?;
    let (pid, program) = raw.split_once('\n')?;
    let pid: u32 = pid.trim().parse().ok()?;
    let program = program.trim();
    if program.is_empty() {
        return None;
    }
    Some(Record {
        pid,
        program: program.to_string(),
    })
}

#[cfg(target_os = "linux")]
fn process_cmdline(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .map(String::from_utf8_lossy)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(target_os = "macos")]
fn process_cmdline(pid: u32) -> Option<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let cmd = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!cmd.is_empty()).then_some(cmd)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cmdline(_pid: u32) -> Option<String> {
    None
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    // Matches `SupervisedChild::kill_blocking`: the server is spawned into its own
    // process group, so signalling the group takes its children with it.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hyprlayer-orphan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn killed(f: impl FnOnce(&dyn Fn(u32))) -> Vec<u32> {
        let seen = RefCell::new(Vec::new());
        f(&|pid| seen.borrow_mut().push(pid));
        seen.into_inner()
    }

    #[test]
    fn round_trips_a_record() {
        let dir = temp_dir("roundtrip");
        let file = dir.join("server.pid");
        record(&file, 4321, "/usr/bin/opencode serve");
        let got = read_record(&file).unwrap();
        assert_eq!(got.pid, 4321);
        assert_eq!(got.program, "/usr/bin/opencode serve");
    }

    #[test]
    fn creates_the_parent_directory() {
        let dir = temp_dir("mkdir");
        let file = dir.join("nested").join("server.pid");
        record(&file, 10, "x");
        assert!(file.exists());
    }

    #[test]
    fn kills_a_matching_process_and_removes_the_file() {
        let dir = temp_dir("match");
        let file = dir.join("server.pid");
        record(&file, 4321, "opencode serve");
        let seen = killed(|kill| {
            let out = reap_with(&file, |_| Some("/opt/opencode serve --port 1".into()), kill);
            assert_eq!(out, Some(4321));
        });
        assert_eq!(seen, vec![4321]);
        assert!(!file.exists(), "record must not survive a reap");
    }

    #[test]
    fn spares_a_reused_pid_running_something_else() {
        let dir = temp_dir("reuse");
        let file = dir.join("server.pid");
        record(&file, 4321, "opencode serve");
        let seen = killed(|kill| {
            assert_eq!(
                reap_with(&file, |_| Some("/usr/bin/postgres -D /var".into()), kill),
                None
            );
        });
        assert!(seen.is_empty(), "must not signal an unrelated process");
        assert!(!file.exists());
    }

    #[test]
    fn spares_an_interactive_session_of_the_same_binary() {
        let dir = temp_dir("interactive");
        let file = dir.join("server.pid");
        record(&file, 4321, "/opt/opencode serve");
        let seen = killed(|kill| {
            assert_eq!(
                reap_with(&file, |_| Some("/opt/opencode".into()), kill),
                None
            );
        });
        assert!(seen.is_empty(), "a bare `opencode` TUI is not our server");
    }

    // The program string `OpenCodeServer::start` records, matched against a
    // command line copied from a real orphaned server on Linux.
    #[test]
    fn matches_a_real_opencode_server_command_line() {
        let dir = temp_dir("real-shape");
        let file = dir.join("server.pid");
        let bin = "/home/user/.local/share/mise/installs/opencode/latest/opencode";
        record(&file, 4321, &format!("{bin} serve"));
        let live = format!("{bin} serve --hostname 127.0.0.1 --port 39833 ");
        let seen = killed(|kill| {
            assert_eq!(reap_with(&file, |_| Some(live.clone()), kill), Some(4321));
        });
        assert_eq!(seen, vec![4321]);
    }

    #[test]
    fn spares_a_dead_pid() {
        let dir = temp_dir("dead");
        let file = dir.join("server.pid");
        record(&file, 4321, "opencode serve");
        let seen = killed(|kill| {
            assert_eq!(reap_with(&file, |_| None, kill), None);
        });
        assert!(seen.is_empty());
        assert!(!file.exists());
    }

    #[test]
    fn does_nothing_without_a_record() {
        let dir = temp_dir("missing");
        let file = dir.join("server.pid");
        let seen = killed(|kill| {
            assert_eq!(
                reap_with(&file, |_| Some("opencode serve".into()), kill),
                None
            );
        });
        assert!(seen.is_empty());
    }

    #[test]
    fn ignores_a_malformed_record() {
        let dir = temp_dir("malformed");
        for body in ["", "not-a-pid\nopencode", "4321", "4321\n", "\nopencode"] {
            let file = dir.join("server.pid");
            std::fs::write(&file, body).unwrap();
            let seen = killed(|kill| {
                assert_eq!(
                    reap_with(&file, |_| Some("opencode serve".into()), kill),
                    None,
                    "body {body:?} must not reap"
                );
            });
            assert!(seen.is_empty(), "body {body:?} must not signal");
        }
    }

    #[test]
    fn never_signals_a_process_group_that_could_be_the_whole_session() {
        let dir = temp_dir("pid1");
        for pid in [0, 1] {
            let file = dir.join("server.pid");
            record(&file, pid, "opencode serve");
            let seen = killed(|kill| {
                assert_eq!(
                    reap_with(&file, |_| Some("opencode serve".into()), kill),
                    None
                );
            });
            assert!(seen.is_empty(), "pid {pid} must never be signalled");
        }
    }

    #[test]
    fn clear_removes_the_record() {
        let dir = temp_dir("clear");
        let file = dir.join("server.pid");
        record(&file, 99, "opencode serve");
        clear(&file);
        assert!(!file.exists());
        clear(&file);
    }

    // The tests above inject `cmdline` and `kill`, so they never exercise
    // /proc parsing or the signal itself. These two drive a real process
    // through the real `reap`.
    #[cfg(unix)]
    mod real_process {
        use super::*;
        use std::os::unix::process::CommandExt;
        use std::process::{Child, Command};
        use std::time::{Duration, Instant};

        // Between `spawn` and `exec` the child still reports the parent's argv,
        // so wait for the exec to land before treating the pid as identifiable.
        // The real reap runs on a later app launch, where this never races.
        fn spawn_sleeper() -> Child {
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            cmd.process_group(0);
            let mut child = cmd.spawn().expect("spawn sleep");
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut identifiable = false;
            while Instant::now() < deadline {
                if process_cmdline(child.id()).is_some_and(|c| c.contains("sleep 30")) {
                    identifiable = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !identifiable {
                let _ = child.kill();
                let _ = child.wait();
                panic!("sleep never became identifiable through /proc");
            }
            child
        }

        fn exited_within(child: &mut Child, within: Duration) -> bool {
            let deadline = Instant::now() + within;
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => return true,
                    Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                    Err(_) => return false,
                }
            }
            false
        }

        #[test]
        fn kills_a_real_recorded_process() {
            let dir = temp_dir("real-kill");
            let file = dir.join("server.pid");
            let mut child = spawn_sleeper();
            let pid = child.id();

            record(&file, pid, "sleep 30");
            assert_eq!(reap(&file), Some(pid));

            assert!(
                exited_within(&mut child, Duration::from_secs(5)),
                "recorded process should have been signalled"
            );
            assert!(!file.exists());
        }

        #[test]
        fn leaves_a_real_process_whose_cmdline_does_not_match() {
            let dir = temp_dir("real-spare");
            let file = dir.join("server.pid");
            let mut child = spawn_sleeper();
            let pid = child.id();

            record(&file, pid, "definitely-not-what-is-running");
            assert_eq!(reap(&file), None, "identity guard must refuse the kill");

            assert!(
                !exited_within(&mut child, Duration::from_millis(500)),
                "a pid whose cmdline no longer matches must survive"
            );

            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
