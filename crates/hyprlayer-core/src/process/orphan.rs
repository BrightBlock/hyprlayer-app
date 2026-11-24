//! Reaping a long-lived child that outlived the app.
//!
//! `spawn_supervised` keeps a child tree tied to this process on Windows through a
//! kill-on-close job object, and `kill_on_drop` covers the ordinary quit path
//! everywhere. Neither runs after a `SIGKILL` or a crash, so on unix a server the
//! app spawned is reparented to init and survives. Recording its pid lets the next
//! run finish the job.

use std::path::Path;

use crate::agent::Harness;

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
    reap_with(pid_file, cmdline, kill_group)
}

/// `reap`, with `SIGKILL`: a codex app-server mid-turn ignores `SIGTERM` and keeps its thread.
pub fn reap_now(pid_file: &Path) -> Option<u32> {
    reap_with(pid_file, cmdline, kill_group_now)
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

/// The executable a live pid is running, or `None` once it has exited.
pub fn running_program(pid: u32) -> Option<String> {
    let line = cmdline(pid)?;
    let program = line.split_whitespace().next()?;
    Some(
        std::path::Path::new(program)
            .file_name()
            .map_or(program.to_string(), |name| {
                name.to_string_lossy().into_owned()
            }),
    )
}

#[cfg(target_os = "linux")]
pub fn cmdline(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .map(String::from_utf8_lossy)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(target_os = "macos")]
pub fn cmdline(pid: u32) -> Option<String> {
    // `-ww` is belt and braces, not a fix: BSD ps can wrap to the width of an
    // attached terminal, and the recorded string ends in `--port <n>`, so a
    // truncated line would fail the match. Measured on macOS 26.6.2 it makes no
    // difference here — `output()` always pipes, and a 177-character command
    // line came back whole either way, even under COLUMNS=40.
    let out = std::process::Command::new("/bin/ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let cmd = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!cmd.is_empty()).then_some(cmd)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn cmdline(_pid: u32) -> Option<String> {
    None
}

/// `ps` reports whole seconds, so a start read back can trail its record by about one.
const START_SLACK_MS: f64 = 2_000.0;

/// Whether a process that started at `started_ms` can be the one recorded at `recorded_ms`:
/// a reused pid started after the recorded process was gone.
pub fn started_by(started_ms: Option<f64>, recorded_ms: f64) -> bool {
    started_ms.is_some_and(|started| started <= recorded_ms + START_SLACK_MS)
}

/// When a live pid started, in milliseconds since the epoch.
#[cfg(target_os = "linux")]
pub fn started_at_ms(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may hold spaces and parens; field 22, `starttime`, is the 20th after it.
    let ticks: f64 = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()?;
    let boot: f64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (hz > 0).then(|| (boot + ticks / hz as f64) * 1000.0)
}

#[cfg(target_os = "macos")]
pub fn started_at_ms(pid: u32) -> Option<f64> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "etime="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let elapsed = parse_etime(String::from_utf8_lossy(&out.stdout).trim())?;
    Some(crate::store::now_ms() - elapsed as f64 * 1000.0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn started_at_ms(_pid: u32) -> Option<f64> {
    None
}

/// Seconds from `ps`'s `[[dd-]hh:]mm:ss`.
#[cfg(any(test, target_os = "macos"))]
fn parse_etime(etime: &str) -> Option<u64> {
    let (days, clock) = match etime.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, etime),
    };
    let mut secs = 0u64;
    for part in clock.split(':') {
        secs = secs * 60 + part.parse::<u64>().ok()?;
    }
    Some(days * 86_400 + secs)
}

#[cfg(unix)]
pub fn kill_group(pid: u32) {
    // Matches `SupervisedChild::kill_blocking`: the server is spawned into its own
    // process group, so signalling the group takes its children with it.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
}

#[cfg(not(unix))]
pub fn kill_group(_pid: u32) {}

/// For a child that may ignore `SIGTERM` while it holds state the next run needs.
#[cfg(unix)]
pub fn kill_group_now(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
pub fn kill_group_now(_pid: u32) {}

#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub fn alive(_pid: u32) -> bool {
    false
}

/// Whether a live command line is a harness the session left behind.
pub fn harness_orphan_matches(cmdline: &str, harness: Harness, session_id: &str) -> bool {
    let mut tokens = cmdline.split_whitespace();
    match harness {
        // `--session-id` or `--resume` put it in argv; the program may be a `sh` stub.
        Harness::Claude => tokens.any(|token| token == session_id),
        // The native app-server outlives its wrapper and holds the thread. Its argv names no
        // thread, so only our goals-off flag tells it from another client's app-server.
        Harness::Codex => {
            tokens.any(|token| token.ends_with("codex"))
                && cmdline.contains("app-server")
                && cmdline
                    .split_whitespace()
                    .any(|token| token == crate::codex::driver::GOALS_DISABLED_CONFIG)
        }
        // Its per-profile server is reaped through `OPENCODE_PID_FILE`.
        Harness::Opencode => false,
    }
}

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

    // A version-manager shim and the install it execs into: different
    // directories, same file name. The literal paths carry no meaning — the
    // property under test is only that the process reports a path other than
    // the one that was spawned, which is what any shim does. Found for real
    // against a mise shim, where recording the spawned path matched nothing.
    const SHIM_BIN: &str = "/opt/toolchain/shims/opencode";
    const RESOLVED_BIN: &str = "/opt/toolchain/installs/opencode/1.2.3/opencode";
    const PORT: u16 = 46603;

    fn serve_args(port: u16) -> String {
        format!("serve --hostname 127.0.0.1 --port {port}")
    }

    /// What `/proc/<pid>/cmdline` reports once the shim has exec'd, joined the
    /// way `cmdline` joins it (trailing separator included).
    fn live_cmdline(port: u16) -> String {
        format!("{RESOLVED_BIN} {} ", serve_args(port))
    }

    /// What `OpenCodeServer::start` records: file name plus the spawned args.
    fn recorded(bin: &str, port: u16) -> String {
        let name = bin.rsplit('/').next().unwrap_or(bin);
        format!("{name} {}", serve_args(port))
    }

    #[test]
    fn matches_a_server_reached_through_a_shim() {
        let dir = temp_dir("shim");
        let file = dir.join("server.pid");
        record(&file, 4321, &recorded(SHIM_BIN, PORT));
        let seen = killed(|kill| {
            assert_eq!(
                reap_with(&file, |_| Some(live_cmdline(PORT)), kill),
                Some(4321)
            );
        });
        assert_eq!(seen, vec![4321]);
    }

    #[test]
    fn the_port_keeps_a_different_server_from_matching() {
        let dir = temp_dir("other-port");
        let file = dir.join("server.pid");
        record(&file, 4321, &recorded(SHIM_BIN, PORT));
        let seen = killed(|kill| {
            assert_eq!(
                reap_with(&file, |_| Some(live_cmdline(PORT + 1)), kill),
                None
            );
        });
        assert!(seen.is_empty(), "a server on another port is not ours");
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

    const SESSION: &str = "4f3c2a10-8b7e-4d21-9a55-0c1e2f3a4b5c";

    #[test]
    fn matches_a_claude_orphan_by_its_session_id() {
        let line = format!(
            "/bin/sh /opt/bin/claude -p --output-format stream-json --verbose --resume {SESSION} --permission-mode default"
        );
        assert!(harness_orphan_matches(&line, Harness::Claude, SESSION));
        let first = format!("/opt/bin/claude -p --session-id {SESSION}");
        assert!(harness_orphan_matches(&first, Harness::Claude, SESSION));
    }

    #[test]
    fn spares_a_reused_pid_without_the_session_id() {
        assert!(!harness_orphan_matches(
            "/usr/bin/postgres -D /var/lib/postgres",
            Harness::Claude,
            SESSION
        ));
        assert!(!harness_orphan_matches(
            "/usr/bin/postgres -D /var/lib/postgres",
            Harness::Codex,
            SESSION
        ));
    }

    #[test]
    fn spares_a_claude_process_for_another_session() {
        let other = "9d8e7f60-1a2b-4c3d-8e9f-a0b1c2d3e4f5";
        let line = format!("/opt/bin/claude -p --resume {other}");
        assert!(!harness_orphan_matches(&line, Harness::Claude, SESSION));
        // A longer id that merely contains ours is someone else's.
        let longer = format!("/opt/bin/claude -p --resume {SESSION}-fork");
        assert!(!harness_orphan_matches(&longer, Harness::Claude, SESSION));
    }

    #[test]
    fn matches_a_codex_app_server() {
        assert!(harness_orphan_matches(
            "node /opt/homebrew/bin/codex app-server -c features.goals=false",
            Harness::Codex,
            SESSION
        ));
        assert!(!harness_orphan_matches(
            "node /opt/homebrew/bin/codex exec hello",
            Harness::Codex,
            SESSION
        ));
        assert!(!harness_orphan_matches(
            "/opt/bin/opencode serve --port 4096",
            Harness::Opencode,
            SESSION
        ));
    }

    #[test]
    fn spares_a_codex_app_server_another_client_started() {
        assert!(!harness_orphan_matches(
            "/opt/homebrew/bin/codex app-server --stdio",
            Harness::Codex,
            SESSION
        ));
    }

    const RECORDED_MS: f64 = 1_760_000_000_000.0;

    #[test]
    fn spares_a_pid_that_started_after_its_record() {
        assert!(!started_by(Some(RECORDED_MS + 10_000.0), RECORDED_MS));
    }

    #[test]
    fn spares_a_pid_whose_start_is_unknown() {
        assert!(!started_by(None, RECORDED_MS));
    }

    #[test]
    fn keeps_a_pid_that_started_by_its_record() {
        assert!(started_by(Some(RECORDED_MS - 500.0), RECORDED_MS));
        // `ps` reports whole seconds, so a start read back can trail the record slightly.
        assert!(started_by(Some(RECORDED_MS + 1_500.0), RECORDED_MS));
    }

    #[test]
    fn parses_ps_elapsed_time() {
        assert_eq!(parse_etime("05:07"), Some(307));
        assert_eq!(parse_etime("02:05:07"), Some(7507));
        assert_eq!(parse_etime("3-02:05:07"), Some(3 * 86_400 + 7507));
        assert_eq!(parse_etime(""), None);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn reads_this_process_start_time() {
        let now = crate::store::now_ms();
        let started = started_at_ms(std::process::id()).expect("a live pid has a start time");
        assert!(started <= now + 1_000.0, "started {started}, now {now}");
        assert!(started > now - 3_600_000.0, "started {started}, now {now}");
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
                if cmdline(child.id()).is_some_and(|c| c.contains("sleep 30")) {
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
