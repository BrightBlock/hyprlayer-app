//! Smoke tests against the real machine state (hyprlayer config + binaries).
//! Ignored by default — they require a configured hyprlayer install. Run with:
//!   cargo test -p hyprlayer-core --test real_machine_smoke -- --ignored --nocapture

use std::collections::HashMap;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryResolver, BinaryState};

/// A resolved binary spawns directly, whatever it was installed as.
///
/// Worth pinning because the opposite looks true: `CreateProcessW` genuinely
/// cannot execute a batch file, and npm installs a script-`bin` CLI (OpenCode's
/// and Codex's both are) as `<name>.cmd` with NO `.exe`. But Rust's
/// `std::process::Command` routes `.bat`/`.cmd` through `cmd.exe` itself, with
/// better escaping than a hand-rolled wrapper — so the app needs no shim
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
        .expect("a resolved binary must be spawnable as-is — if this fails on a \
                 .cmd shim, the app needs its own cmd.exe wrapper again");
    println!("exit={:?} out={:?}", out.status.code(), String::from_utf8_lossy(&out.stdout).trim());
    assert!(out.status.success(), "`opencode --version` should exit 0");
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

/// Whether the app would show Codex as installed — the exact resolver the
/// Harnesses tab reads, not a `which` approximation.
///
/// Windows is the case worth running: the official installer drops a real
/// `codex.exe` in a per-user `Programs` dir on the User PATH, while `npm i -g`
/// drops only a `.cmd` shim (its npm `bin` is a JS wrapper). Those resolve and
/// spawn differently, so run this from the SAME session the app launches from —
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
