//! Bursts of large reads and an upload, then the server's memory: it comes back near where it
//! started, and its peak stays within a bound of what a burst carried. Locally, and on a Linux
//! host over the relay, where glibc held a burst's high-water (ADR-0009).
#![cfg(unix)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_core::WorktreeMode;
use hyprlayer_e2e::containers::RemoteHost;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::RpcClient;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const FILES: usize = 8;
/// Just under `worktree_read_file`'s 2 MiB cap.
const FILE_BYTES: usize = 1_990_000;
const ATTACHMENTS: usize = 3;
const ATTACHMENT_BYTES: usize = 16 * MIB as usize;
/// Later bursts reuse what an allocator kept, or show it still growing.
const BURSTS: usize = 3;
const SETTLE: Duration = Duration::from_secs(3);
const IO: Duration = Duration::from_secs(120);
/// What one burst carries, before base64: the files, the attachments read and one uploaded.
const PAYLOAD: u64 = (FILES * FILE_BYTES + (ATTACHMENTS + 1) * ATTACHMENT_BYTES) as u64;
/// Above where the server started, after the last burst.
const KEPT_LIMIT: u64 = 24 * MIB;
/// A burst's peak above where it started, against `PAYLOAD`.
const PEAK_RATIO: f64 = 1.5;

/// Where the server runs, and how what the OS charges it is read.
enum Process<'a> {
    Local(u32),
    Remote(&'a RemoteHost, String),
}

impl Process<'_> {
    fn charged(&self) -> u64 {
        match self {
            Process::Local(pid) => charged(*pid),
            Process::Remote(host, pid) => proc_status(host, pid, "VmRSS"),
        }
    }

    /// The most charged while `burst` runs: Linux's own high-water mark there, else sampled.
    async fn peak_during(&self, burst: impl std::future::Future<Output = bool>) -> (bool, u64) {
        match self {
            Process::Local(pid) => {
                let pid = *pid;
                let done = Arc::new(AtomicBool::new(false));
                let peak = Arc::new(AtomicU64::new(0));
                let sampler = {
                    let (done, peak) = (done.clone(), peak.clone());
                    std::thread::spawn(move || {
                        while !done.load(Relaxed) {
                            peak.fetch_max(charged(pid), Relaxed);
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    })
                };
                let ok = burst.await;
                done.store(true, Relaxed);
                sampler.join().expect("the sampler");
                (ok, peak.load(Relaxed))
            }
            Process::Remote(host, pid) => {
                host.exec(&format!("echo 5 > /proc/{pid}/clear_refs"));
                let ok = burst.await;
                (ok, proc_status(host, pid, "VmHWM"))
            }
        }
    }
}

/// macOS's phys_footprint, what Activity Monitor and jetsam count; its RSS also counts pages
/// malloc has handed back as reusable.
#[cfg(target_os = "macos")]
fn charged(pid: u32) -> u64 {
    // SAFETY: a zeroed v4 record is valid, and the call writes at most one.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let read = unsafe {
        libc::proc_pid_rusage(
            pid as i32,
            libc::RUSAGE_INFO_V4,
            (&mut info as *mut libc::rusage_info_v4).cast(),
        )
    };
    if read == 0 {
        info.ri_phys_footprint
    } else {
        0
    }
}

#[cfg(not(target_os = "macos"))]
fn charged(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| vm_field(&status, "VmRSS"))
        .unwrap_or(0)
}

fn ps_rss(pid: u32) -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        * KIB
}

fn vm_field(status: &str, field: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with(field))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * KIB)
}

fn proc_status(host: &RemoteHost, pid: &str, field: &str) -> u64 {
    vm_field(&host.exec(&format!("cat /proc/{pid}/status")), field).unwrap_or(0)
}

/// The workstream a burst reads from, the paths it reads, and what it uploads.
struct Burst {
    ws: String,
    files: Vec<String>,
    attachments: Vec<String>,
    upload: Arc<String>,
}

impl Burst {
    fn new(ws: String, attachments: Vec<String>) -> Self {
        Self {
            ws,
            files: (0..FILES).map(|i| format!("big-{i}.txt")).collect(),
            attachments,
            upload: Arc::new(B64.encode(blob(ATTACHMENT_BYTES))),
        }
    }

    /// Every read and the upload at once; `true` when each came back whole.
    async fn run(&self, client: &Arc<RpcClient>) -> bool {
        let mut calls = tokio::task::JoinSet::new();
        for path in &self.files {
            let (client, ws, path) = (client.clone(), self.ws.clone(), path.clone());
            calls.spawn(async move {
                client
                    .worktree_read_file(ws, path)
                    .await
                    .is_ok_and(|file| file.content.len() == FILE_BYTES)
            });
        }
        for path in &self.attachments {
            let (client, path) = (client.clone(), path.clone());
            calls.spawn(async move {
                client
                    .read_attachment(path)
                    .await
                    .is_ok_and(|data| data.len() == ATTACHMENT_BYTES.div_ceil(3) * 4)
            });
        }
        let (client, ws, upload) = (client.clone(), self.ws.clone(), self.upload.clone());
        calls.spawn(async move {
            client
                .save_attachment(ws, "upload.bin".into(), upload.to_string())
                .await
                .is_ok()
        });
        tokio::time::timeout(IO, async {
            let mut ok = true;
            while let Some(call) = calls.join_next().await {
                ok &= call.unwrap_or(false);
            }
            ok
        })
        .await
        .unwrap_or(false)
    }
}

/// Incompressible, as an image is.
fn blob(len: usize) -> Vec<u8> {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// Source text, with the quotes and newlines JSON escapes.
const LINE: &str = "fn step() -> u64 { 42 } // a \"quoted\" word";

fn text_file() -> String {
    let line = format!("{LINE}\n");
    let mut text = line.repeat(FILE_BYTES / line.len() + 1);
    text.truncate(FILE_BYTES);
    text
}

async fn measure(j: &mut Journey, process: &Process<'_>, client: &Arc<RpcClient>, burst: &Burst) {
    tokio::time::sleep(SETTLE).await;
    let start = process.charged();
    let (mut all_ok, mut worst_peak, mut kept) = (true, 0, 0);
    let mut rounds = Vec::new();
    for _ in 0..BURSTS {
        let before = process.charged();
        let (ok, peak) = process.peak_during(burst.run(client)).await;
        tokio::time::sleep(SETTLE).await;
        let after = process.charged();
        all_ok &= ok;
        worst_peak = worst_peak.max(peak.saturating_sub(before));
        kept = after.saturating_sub(start);
        let mut round = json!({ "before": mib(before), "peak": mib(peak), "after": mib(after) });
        if let Process::Local(pid) = process {
            round["ps_rss_after"] = json!(mib(ps_rss(*pid)));
        }
        rounds.push(round);
    }
    j.note(
        "server_memory_mib",
        json!({ "payload": mib(PAYLOAD), "start": mib(start), "bursts": rounds }),
    );
    j.guard("every read and upload comes back whole", all_ok, json!({}));
    j.guard(
        "the server comes back near where it started",
        kept <= KEPT_LIMIT,
        json!({ "bursts": BURSTS, "limit_mib": mib(KEPT_LIMIT) }),
    );
    j.guard(
        "a burst's peak stays within a bound of what it carried",
        worst_peak as f64 <= PEAK_RATIO * PAYLOAD as f64,
        json!({ "payload_mib": mib(PAYLOAD), "ratio": PEAK_RATIO }),
    );
}

fn mib(bytes: u64) -> f64 {
    (bytes as f64 / MIB as f64 * 10.0).round() / 10.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_memory() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("server_memory", &sandbox);
    let server = Server::start(&sandbox).await;
    let project = onboard_and_import(&mut j, &server.client(), &sandbox, &repo).await;
    let view = create_workstream(&mut j, &server.client(), &project, "Memory").await;
    let worktree = std::path::PathBuf::from(text(&view, "/item/worktreePath"));
    let content = text_file();
    for i in 0..FILES {
        std::fs::write(worktree.join(format!("big-{i}.txt")), &content).expect("write a file");
    }
    // Written in place rather than uploaded, so the setup leaves no high-water of its own.
    let dir = worktree.join(".hyprlayer").join("attachments");
    std::fs::create_dir_all(&dir).expect("create the attachments dir");
    let attachments = (0..ATTACHMENTS)
        .map(|i| {
            let path = dir.join(format!("held-{i}.bin"));
            std::fs::write(&path, blob(ATTACHMENT_BYTES)).expect("write an attachment");
            path.display().to_string()
        })
        .collect();
    let burst = Burst::new(text(&view, "/item/id"), attachments);
    let client = server.connect().await;

    j.step("bursts of large reads and an upload");
    measure(&mut j, &Process::Local(server.pid()), &client, &burst).await;
    j.finish();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs Docker: cargo test -p hyprlayer-e2e -- --ignored, or the release gate"]
async fn remote_server_memory() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("remote_server_memory", &sandbox);
    let host = RemoteHost::start(&sandbox);
    host.exec("mkdir -p notes && printf 'remote\\n' > notes/README");
    let relay = host.relay().await;
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
    let view = j
        .call(
            &relay.client,
            Request::CreateWorkstream {
                project_path: text(&project, "/path"),
                title: "Memory".into(),
                base: None,
                mode: WorktreeMode::Folder,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let ws = text(&view, "/item/id");
    j.alias(&ws, "$WS");
    let worktree = text(&view, "/item/worktreePath");
    host.exec(&format!(
        "cd '{worktree}' && yes '{line}' | head -c {FILE_BYTES} > big-0.txt \
         && for i in $(seq 1 {last}); do cp big-0.txt big-$i.txt; done \
         && mkdir -p .hyprlayer/attachments && for i in $(seq 0 {last_held}); do \
         head -c {ATTACHMENT_BYTES} /dev/urandom > .hyprlayer/attachments/held-$i.bin; done",
        line = LINE,
        last = FILES - 1,
        last_held = ATTACHMENTS - 1,
    ));
    let attachments = (0..ATTACHMENTS)
        .map(|i| format!("{worktree}/.hyprlayer/attachments/held-{i}.bin"))
        .collect();
    let burst = Burst::new(ws, attachments);
    let pid = host
        .pids("hyprlayer-server serve")
        .into_iter()
        .next()
        .expect("the relay started a server");

    j.step("bursts of large reads and an upload over the relay");
    measure(&mut j, &Process::Remote(&host, pid), &relay.client, &burst).await;
    relay.hang_up().await;
    host.save_server_logs();
    j.finish();
}
