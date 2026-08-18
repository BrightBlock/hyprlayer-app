//! Smoke tests against the real machine state (hyprlayer config + binaries).
//! Ignored by default, they require a configured hyprlayer install. Run with:
//!   cargo test -p hyprlayer-core --test real_machine_smoke -- --ignored --nocapture

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryResolver, BinaryState};
use hyprlayer_core::process::spawn_supervised;

/// A resolved binary spawns directly, whatever it was installed as.
///
/// Worth pinning because the opposite looks true: `CreateProcessW` genuinely
/// cannot execute a batch file, and npm installs a script-`bin` CLI (OpenCode's
/// and Codex's both are) as `<name>.cmd` with NO `.exe`. But Rust's
/// `std::process::Command` routes `.bat`/`.cmd` through `cmd.exe` itself, with
/// better escaping than a hand-rolled wrapper, so the app needs no shim
/// handling of its own. Verified on Windows against `opencode.cmd`.
#[tokio::test]
#[ignore = "requires opencode installed; run in the session the app launches from"]
async fn a_resolved_binary_spawns_directly() {
    let resolver = BinaryResolver::initialize(HashMap::new());
    resolver.enrich_with_login_shell().await;
    let status = resolver.status(BinaryKind::OpenCode).await;
    let path = status.path.clone().expect("opencode should resolve");
    println!("opencode: {path}");

    let out = tokio::process::Command::new(&path)
        .arg("--version")
        .output()
        .await
        .expect(
            "a resolved binary must be spawnable as-is. If this fails on a \
                 .cmd shim, the app needs its own cmd.exe wrapper again",
        );
    println!(
        "exit={:?} out={:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim()
    );
    assert!(out.status.success(), "`opencode --version` should exit 0");
}

/// A supervised `opencode serve` must not survive the handle that owns it.
///
/// Windows is the case this exists for. npm installs OpenCode as `opencode.cmd`
/// with no `.exe` on PATH, and `std::process::Command` runs `.cmd` through
/// `cmd.exe`, so the pid we hold is the wrapper and the server is its child.
/// Verified on nexus-win: `kill_on_drop` reaped the wrapper and left the server
/// listening indefinitely, which is why the spawn goes through a job object now.
/// On Unix the tree is one process deep and this pins the same invariant.
///
/// The "came up" assertion is load-bearing: without it a server that never
/// started would make the teardown check pass for the wrong reason.
#[tokio::test]
#[ignore = "spawns a real `opencode serve`; run in the session the app launches from"]
async fn opencode_serve_dies_with_its_handle() {
    let resolver = BinaryResolver::initialize(HashMap::new());
    resolver.enrich_with_login_shell().await;
    let bin = resolver
        .status(BinaryKind::OpenCode)
        .await
        .path
        .expect("opencode should resolve");

    let port = free_port();
    let data_dir = tempfile::tempdir().expect("temp data home");

    // The app's spawn shape (src-tauri/src/opencode.rs), against a throwaway
    // data home so this never touches the user's OpenCode state.
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ])
    .env("XDG_DATA_HOME", data_dir.path())
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        // Inherent on tokio's Command; no CommandExt import needed.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = spawn_supervised(&mut cmd).expect("spawn opencode serve");
    println!("port {port}, held pid {:?}", child.id());

    assert!(
        port_settles(port, true, Duration::from_secs(30)).await,
        "`opencode serve` never listened on {port}; the teardown check below \
         would pass vacuously"
    );

    drop(child);

    assert!(
        port_settles(port, false, Duration::from_secs(15)).await,
        "`opencode serve` is still listening on {port} after its handle dropped \
         the process tree outlived us. On Windows this means the job object \
         did not take: we killed the `cmd.exe` shim and the server underneath \
         it is still running."
    );
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Poll until `port` is open (`want_open`) or closed, within `budget`.
/// Blocking connect on purpose, a loopback attempt resolves immediately either
/// way, and `tokio`'s `net` feature is not enabled for this crate on its own.
async fn port_settles(port: u16, want_open: bool, budget: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let open = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok();
        if open == want_open {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
#[ignore = "requires a configured hyprlayer install"]
async fn real_config_parses_and_lists_projects() {
    let service = ConfigService::with_default_path().expect("config dir");
    let loaded = service.load().await.expect("config should parse");
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .expect("thoughts configured");

    println!("user: {}", thoughts.user);
    println!("backend: {:?}", thoughts.backend.kind());
    println!("projects: {}", thoughts.repo_mappings.len());
    assert!(!thoughts.user.is_empty());
    assert!(thoughts.is_configured());
}

/// Whether the app would show Codex as installed, the exact resolver the
/// Harnesses tab reads, not a `which` approximation.
///
/// Windows is the case worth running: the official installer drops a real
/// `codex.exe` in a per-user `Programs` dir on the User PATH, while `npm i -g`
/// drops only a `.cmd` shim (its npm `bin` is a JS wrapper). Those resolve and
/// spawn differently, so run this from the SAME session the app launches from,
/// an SSH logon session has a different view of per-user install dirs and will
/// report a false negative.
#[tokio::test]
#[ignore = "requires codex installed; run in the session the app launches from"]
async fn real_codex_resolves() {
    let resolver = BinaryResolver::initialize(HashMap::new());
    resolver.enrich_with_login_shell().await;

    let status = resolver.status(BinaryKind::Codex).await;
    println!("codex: {:?}", status.state);
    println!("  path:    {:?}", status.path);
    println!("  version: {:?}", status.version);
    assert_eq!(
        status.state,
        BinaryState::Ok,
        "codex should resolve AND answer --version; a `.cmd` shim that can't be \
         spawned surfaces here as a non-Ok state"
    );
}

#[tokio::test]
#[ignore = "requires hyprlayer/git/claude on PATH"]
async fn real_binaries_resolve() {
    let resolver = BinaryResolver::initialize(HashMap::new());
    resolver.enrich_with_login_shell().await;

    for kind in [BinaryKind::Hyprlayer, BinaryKind::Git, BinaryKind::Claude] {
        let status = resolver.status(kind).await;
        println!("{}: {:?} {:?}", status.name, status.state, status.path);
        assert_eq!(
            status.state,
            BinaryState::Ok,
            "{} should be ok",
            status.name
        );
    }
}

#[tokio::test]
#[ignore = "requires a configured hyprlayer install with mapped repos"]
async fn real_storage_info_round_trips() {
    let service = ConfigService::with_default_path().expect("config dir");
    let loaded = service.load().await.expect("config should parse");
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .expect("thoughts configured");
    let repo = thoughts
        .repo_mappings
        .keys()
        .find(|p| std::path::Path::new(p.as_str()).is_dir())
        .expect("at least one mapped repo exists on disk");

    let resolver = BinaryResolver::initialize(HashMap::new());
    resolver.enrich_with_login_shell().await;

    let info = HyprCli::new(&resolver)
        .storage_info(std::path::Path::new(repo))
        .await
        .expect("storage info should parse");
    println!("repo: {repo}");
    println!("mapped: {:?} backend: {}", info.mapped_name, info.backend);
    println!("schema fields: {}", info.schema.len());
    assert!(info.mapped_name.is_some());
    assert_eq!(info.schema.len(), 10, "thought schema has 10 fields");
}

/// A root the app scaffolds must be one the real CLI adopts, the contract
/// "Add project" depends on, pinned against the installed binary because the
/// rule lives in the CLI, not here.
#[tokio::test]
#[ignore = "requires the hyprlayer CLI installed; run with --ignored"]
async fn scaffolded_root_is_accepted_by_the_real_cli() {
    let git = which::which("git").expect("git installed");
    let hyprlayer = which::which("hyprlayer").expect("hyprlayer CLI installed");
    let tmp = tempfile::tempdir().unwrap();

    // The thoughts root, shaped by the app exactly as the wizard would.
    let root = tmp.path().join("thoughts");
    hyprlayer_core::thoughts_repo::scaffold(&git, &root, "tester", "repos", "global")
        .await
        .expect("scaffold should succeed");

    // A throwaway project to run `thoughts init` from.
    let code = tmp.path().join("proj");
    std::fs::create_dir_all(&code).unwrap();
    for args in [
        vec!["init", "-b", "main", code.to_str().unwrap()],
        vec![
            "-C",
            code.to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-m",
            "init",
            "-c",
            "commit.gpgsign=false",
        ],
    ] {
        std::process::Command::new(&git)
            .args(&args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
    }

    // An isolated config so the run can't touch the developer's real one.
    let cfg = tmp.path().join("config.json");
    std::fs::write(
        &cfg,
        serde_json::json!({
            "version": 4,
            "disableUpdateCheck": true,
            "thoughts": {
                "backend": {
                    "kind": "git",
                    "thoughtsRepo": root.to_str().unwrap(),
                    "reposDir": "repos",
                    "globalDir": "global",
                },
                "profiles": {}, "currentProfile": null, "repoMappings": {}, "user": "tester",
            },
            "ai": {"agentTool": "claude"},
            "telemetry": {"mode": "off"},
        })
        .to_string(),
    )
    .unwrap();

    let out = std::process::Command::new(&hyprlayer)
        .current_dir(&code)
        .args([
            "thoughts",
            "init",
            "--yes",
            "--directory",
            "Proj",
            "--config-file",
            cfg.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .output()
        .expect("hyprlayer should run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("exit={:?}\nstderr: {stderr}", out.status.code());

    assert!(
        out.status.success(),
        "the CLI must adopt an app-scaffolded root, but refused it:\n{stderr}"
    );
    assert!(
        code.join("thoughts").join("global").exists(),
        "the CLI should have linked thoughts/global into the scaffolded root"
    );
    assert!(
        root.join("repos").join("Proj").is_dir(),
        "project tree created"
    );
}

/// The model-catalog probe drives a REAL `claude`, end to end.
///
/// The parse is unit-tested off a captured payload; what this pins is the spawn
/// path, args, neutral cwd, and holding stdin open until the reply lands. Get
/// any of those wrong and the probe times out instead of failing loudly, which
/// unit tests can't catch. Costs no tokens: no user message is ever sent.
#[tokio::test]
#[ignore = "spawns a real `claude`; run in the session the app launches from"]
async fn claude_answers_the_model_catalog_probe() {
    let resolver = BinaryResolver::initialize(HashMap::new());
    resolver.enrich_with_login_shell().await;
    let bin = resolver
        .status(BinaryKind::Claude)
        .await
        .path
        .expect("claude should resolve");
    let path_env = resolver.augmented_path().await;

    let probe = hyprlayer_core::claude::probe::probe(
        std::path::Path::new(&bin),
        &path_env,
        &hyprlayer_core::claude::probe::neutral_cwd(),
        &[],
    )
    .await
    .expect("claude should answer list_models");

    for m in &probe.models {
        println!(
            "{:?} -> {:?} [{}] efforts={:?} fast={}",
            m.flag,
            m.resolved_model,
            m.display_name,
            m.supported_effort_levels,
            m.supports_fast_mode
        );
    }
    assert!(!probe.models.is_empty(), "a signed-in CLI offers models");
    // Exactly one row launches with no `--model`; the composer's default entry
    // maps onto it, so a second (or zero) would break that mapping.
    assert_eq!(
        probe.models.iter().filter(|m| m.flag.is_none()).count(),
        1,
        "expected exactly one default (no --model) row"
    );
}
