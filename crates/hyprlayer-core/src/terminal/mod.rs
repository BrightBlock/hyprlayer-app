//! PTY session abstraction for the Ghostty-style terminal panel.
//!
//! [`PtySession`] spawns the user's shell on a real PTY in the worktree
//! directory, streams output over a [`tokio::sync::broadcast`] channel with a
//! bounded scrollback ring, and accepts input / resize / kill.
//!
//! The reader loop runs on a dedicated [`std::thread`] because
//! `portable-pty`'s reader is a blocking `std::io::Read` handle — it must not
//! block a tokio task. Everything else (broadcast, scrollback) is shared-safe.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use base64::Engine as _;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Max scrollback bytes retained for re-attach replay (drops oldest first).
const SCROLLBACK_CAP: usize = 256 * 1024;
/// Upper bound on a single coalesced IPC event. Only a genuine flood (a build,
/// `cat` of a big file) — where reads queue up faster than we drain them — gets
/// batched this far; interactive output flushes immediately (see the coalescer).
const FLUSH_BYTES: usize = 64 * 1024;
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// One streamed terminal event. Output is base64 because PTY output is binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TerminalEvent {
    Output { data: String },
    Exited { code: Option<i32> },
}

/// Configuration for spawning a new [`PtySession`].
pub struct SpawnTerminal {
    pub shell: PathBuf,
    /// Additional arguments passed to the program (empty for an interactive
    /// shell, non-empty when running a specific command such as `["ai",
    /// "configure"]` via `spawn_cli_terminal`).
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub env: Vec<(String, String)>,
    pub workstream_id: String,
}

/// A live PTY session: shell process + broadcast output + bounded scrollback.
///
/// A waiter thread owns the `Child`, `wait()`s on it for the real exit code,
/// hands that to the reader thread, and then drops the master. The reader
/// thread streams output and emits the final `Exited`. We keep only a cloned
/// [`ChildKiller`] here so `kill()` never contends with that wait.
pub struct PtySession {
    /// `Option` so the waiter can drop the master once the child exits — on
    /// Windows that tears down the lingering ConPTY conhost, which is the only
    /// thing that unblocks the reader's blocking read (it never sees EOF on its
    /// own there). Shared with the waiter thread.
    master: Arc<StdMutex<Option<Box<dyn MasterPty + Send>>>>,
    killer: StdMutex<Box<dyn ChildKiller + Send + Sync>>,
    writer: StdMutex<Box<dyn Write + Send>>,
    tx: broadcast::Sender<TerminalEvent>,
    scrollback: Arc<StdMutex<VecDeque<u8>>>,
    /// The shell's pid, captured at spawn. On Unix the PTY child is a session
    /// leader (portable-pty `setsid`s it), so this doubles as its process-group
    /// id — `kill()` SIGKILLs the whole group to reap children the shell spawned.
    #[cfg(unix)]
    pid: Option<u32>,
    pub workstream_id: String,
}

impl PtySession {
    /// Spawn a shell in the given worktree directory and start the reader thread.
    pub fn spawn(cfg: SpawnTerminal) -> std::io::Result<Arc<Self>> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: cfg.rows,
                cols: cfg.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(to_io)?;

        let mut cmd = CommandBuilder::new(&cfg.shell);
        for arg in &cfg.args {
            cmd.arg(arg);
        }
        cmd.cwd(&cfg.cwd);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(to_io)?;
        drop(pair.slave); // so the master sees EOF when the shell exits
        let killer = child.clone_killer();
        // Capture the pid before `child` moves to the waiter thread, so `kill()`
        // can signal the child's whole process group.
        #[cfg(unix)]
        let pid = child.process_id();

        let reader = pair.master.try_clone_reader().map_err(to_io)?;
        let writer = pair.master.take_writer().map_err(to_io)?;
        let (tx, _rx) = broadcast::channel(2048);
        let scrollback = Arc::new(StdMutex::new(VecDeque::with_capacity(SCROLLBACK_CAP)));
        let master = Arc::new(StdMutex::new(Some(pair.master)));

        let session = Arc::new(Self {
            master: master.clone(),
            killer: StdMutex::new(killer),
            writer: StdMutex::new(writer),
            tx: tx.clone(),
            scrollback: scrollback.clone(),
            #[cfg(unix)]
            pid,
            workstream_id: cfg.workstream_id,
        });

        // The reader and the child are reaped on separate threads. The reader
        // can't double as the reaper: on Windows the ConPTY conhost keeps the
        // read pipe open after the child exits, so a read-until-EOF loop would
        // block forever and never surface the exit. Instead the waiter reaps the
        // child for its code and the reader emits `Exited` once it has both
        // drained to EOF and received that code — keeping the final output ahead
        // of the exit event (the property the Unix tests rely on).
        let (code_tx, code_rx) = std::sync::mpsc::channel::<Option<i32>>();

        // Raw reads flow reader-thread → coalescer-thread over this channel. The
        // reader can only do blocking `read`s (it can't also wait on a timer), so
        // a second thread does the time-based batching.
        let (chunk_tx, chunk_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        // Reader: portable-pty's reader is a blocking io::Read, so it gets its own
        // thread. It just hands each read to the coalescer and exits (dropping
        // `chunk_tx`, which the coalescer observes as EOF) once the PTY closes.
        let mut reader = reader;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    // If the coalescer is gone there's no one to stream to; stop.
                    Ok(n) if chunk_tx.send(buf[..n].to_vec()).is_err() => break,
                    Ok(_) => {}
                }
            }
        });

        // Coalescer: flush each read straight through, but fold in any reads that
        // are ALREADY queued (a flood) up to `FLUSH_BYTES`, so a chatty program
        // still collapses into a handful of IPC events without ever delaying the
        // interactive path. It appends each batch to the scrollback ring and
        // broadcasts it under the same lock, so a concurrent
        // `subscribe_with_scrollback` sees every batch in exactly one of
        // {snapshot, live stream} — never both (duplicated) nor neither (lost).
        // It blocks on `recv()` when idle, so an open-but-quiet terminal costs no
        // wakeups.
        let reader_tx = tx.clone();
        std::thread::spawn(move || {
            let mut pending: Vec<u8> = Vec::with_capacity(FLUSH_BYTES);
            let flush = |pending: &mut Vec<u8>| {
                if pending.is_empty() {
                    return;
                }
                let encoded = B64.encode(&pending);
                let mut sb = scrollback.lock().unwrap();
                sb.extend(pending.iter().copied());
                let overflow = sb.len().saturating_sub(SCROLLBACK_CAP);
                if overflow > 0 {
                    sb.drain(0..overflow);
                }
                // send() errs only if there are no receivers — fine. Held under
                // the scrollback lock so it can't interleave with a snapshot.
                let _ = reader_tx.send(TerminalEvent::Output { data: encoded });
                drop(sb);
                pending.clear();
            };
            loop {
                // Block for the next read; a closed channel means the reader is
                // done, so drop out and emit the exit below.
                let first = match chunk_rx.recv() {
                    Ok(c) => c,
                    Err(_) => break,
                };
                pending.extend_from_slice(&first);
                // Drain whatever ELSE is already queued (a genuine flood, where
                // reads pile up faster than we encode+broadcast) up to the byte
                // cap — but DON'T wait for more. Interactive output (a keystroke
                // echo, a prompt redraw on wrap) has nothing else queued, so it
                // flushes on the same tick it arrives.
                //
                // This eager flush matters for rendering, not just latency: a
                // WKWebView only re-composites the xterm canvas when it paints,
                // and xterm paints once per write. Coalescing a wrap's redraw into
                // a single delayed write produced a single paint that WKWebView
                // would leave un-presented until the next "real" event (Enter,
                // resize) — the terminal went black mid-line. Writing each read
                // through keeps the paint cadence high enough that every update is
                // presented, matching a no-coalescing reference renderer.
                let mut disconnected = false;
                while pending.len() < FLUSH_BYTES {
                    match chunk_rx.try_recv() {
                        Ok(c) => pending.extend_from_slice(&c),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                flush(&mut pending);
                if disconnected {
                    break;
                }
            }
            // EOF: the tail is already flushed above; emit the exit once the
            // waiter has the code, keeping final output ahead of the exit event.
            let code = code_rx.recv().unwrap_or(None);
            let _ = reader_tx.send(TerminalEvent::Exited { code });
        });

        // Waiter: reap the child (no zombie) for its real exit code, hand it to
        // the reader, then drop the master. Dropping the master closes a
        // lingering ConPTY so the reader's blocked read returns EOF on Windows;
        // on Unix the reader already EOF'd when the child closed the slave, so
        // the drop is a harmless no-op.
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            let _ = code_tx.send(code);
            if let Ok(mut guard) = master.lock() {
                let _ = guard.take();
            }
        });

        Ok(session)
    }

    /// Subscribe to the live output broadcast.
    pub fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.tx.subscribe()
    }

    /// Atomically snapshot the retained scrollback and subscribe to live output,
    /// returning both. Taken together under the scrollback lock — which the
    /// coalescer thread also holds while appending each output batch to the ring
    /// and broadcasting it — this closes the gap where a batch emitted between a
    /// separate snapshot and subscribe would be in neither (lost) or both
    /// (duplicated). The snapshot is the base64 scrollback for replay; live
    /// output follows on the receiver.
    pub fn subscribe_with_scrollback(&self) -> (broadcast::Receiver<TerminalEvent>, String) {
        let sb = self.scrollback.lock().unwrap();
        let rx = self.tx.subscribe();
        let snapshot = B64.encode(sb.iter().copied().collect::<Vec<u8>>());
        (rx, snapshot)
    }

    /// Snapshot the retained scrollback as raw bytes (no subscribe). Used to
    /// capture a script run's output for the dock's read-only "last run" view.
    pub fn scrollback_bytes(&self) -> Vec<u8> {
        self.scrollback.lock().unwrap().iter().copied().collect()
    }

    /// Write raw bytes (keyboard input) to the PTY master.
    pub fn write_input(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()
    }

    /// Notify the PTY of a terminal-size change. A no-op once the child has
    /// exited and the waiter has dropped the master.
    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        match self.master.lock().unwrap().as_ref() {
            Some(master) => master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(to_io),
            None => Ok(()),
        }
    }

    /// Kill the shell and everything it spawned. On Unix the child is a session
    /// leader, so we SIGKILL its whole process group — otherwise a foreground
    /// child (e.g. `npm run dev`) or a `&` job is orphaned and survives Stop /
    /// terminal close / archive teardown, leaking the process and its port. The
    /// waiter thread then reaps the shell, the reader emits the final `Exited`,
    /// and the master is dropped.
    pub fn kill(&self) {
        // Group kill first (children), then the leader as a fallback. getpgid
        // confirms the leader is still alive before signalling, narrowing the
        // window where a reaped-then-reused pid could be hit.
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
            if pgid > 0 {
                unsafe { libc::killpg(pgid, libc::SIGKILL) };
            }
        }
        let _ = self.killer.lock().unwrap().kill();
    }
}

fn to_io(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pty_echo_roundtrip() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test".into(),
        })
        .expect("spawn pty");

        let mut rx = s.subscribe();
        s.write_input(b"printf MARKER_OK\n").unwrap();

        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Ok(TerminalEvent::Output { data })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
                if seen.contains("MARKER_OK") {
                    break;
                }
            }
        }
        assert!(seen.contains("MARKER_OK"), "got: {seen:?}");
        s.resize(100, 30).expect("resize ok");
        s.kill();
    }

    /// Mirrors the `terminal_open` path: subscribe via `subscribe_with_scrollback`
    /// (as `attach_stream` does) and confirm live output is delivered on that rx.
    /// Confirm that `SpawnTerminal::args` are forwarded: `/bin/echo hi` exits 0
    /// and its output appears on the channel.
    #[tokio::test]
    async fn spawn_with_args_runs_and_exits() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/echo"),
            args: vec!["ARGS_OK".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-args".into(),
        })
        .expect("spawn pty");

        let mut rx = s.subscribe();
        let mut seen = String::new();
        let mut exited = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(TerminalEvent::Output { data })) => {
                    seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
                }
                Ok(Ok(TerminalEvent::Exited { code })) => {
                    assert_eq!(code, Some(0), "echo should exit 0");
                    exited = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(seen.contains("ARGS_OK"), "got: {seen:?}");
        assert!(exited, "expected Exited event");
    }

    #[tokio::test]
    async fn open_path_delivers_live_output() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec![],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test".into(),
        })
        .expect("spawn pty");

        let (mut rx, snapshot) = s.subscribe_with_scrollback();
        let mut seen = String::from_utf8_lossy(&B64.decode(snapshot).unwrap()).into_owned();
        s.write_input(b"printf MARKER_OK\n").unwrap();

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !seen.contains("MARKER_OK") && tokio::time::Instant::now() < deadline {
            if let Ok(Ok(TerminalEvent::Output { data })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
            }
        }
        assert!(seen.contains("MARKER_OK"), "got: {seen:?}");
        s.kill();
    }

    /// `kill()` must reap the child's whole process group, not just the shell:
    /// a backgrounded grandchild (`sleep 60 &`) must be gone after kill().
    #[tokio::test]
    async fn kill_reaps_child_process_group() {
        let s = PtySession::spawn(SpawnTerminal {
            shell: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "sleep 60 & echo PID:$!; sleep 60".into()],
            cwd: std::env::temp_dir(),
            cols: 80,
            rows: 24,
            env: vec![],
            workstream_id: "test-group".into(),
        })
        .expect("spawn pty");

        // Read the backgrounded child's pid from the shell's output.
        let mut rx = s.subscribe();
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let child_pid: i32 = loop {
            assert!(tokio::time::Instant::now() < deadline, "never saw pid: {seen:?}");
            if let Ok(Ok(TerminalEvent::Output { data })) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                seen.push_str(&String::from_utf8_lossy(&B64.decode(data).unwrap()));
                if let Some(pid) = seen
                    .lines()
                    .find_map(|l| l.strip_prefix("PID:"))
                    .and_then(|p| p.trim().parse::<i32>().ok())
                {
                    break pid;
                }
            }
        };

        assert_eq!(
            unsafe { libc::kill(child_pid, 0) },
            0,
            "backgrounded child should be alive before kill()"
        );
        s.kill();

        // After the group kill the child must die (kill(pid, 0) -> ESRCH).
        let mut gone = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if unsafe { libc::kill(child_pid, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(gone, "backgrounded child {child_pid} survived kill()");
    }
}
