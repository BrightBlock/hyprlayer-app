//! Smoke tests against the real machine state (hyprlayer config + binaries).
//! Ignored by default — they require a configured hyprlayer install. Run with:
//!   cargo test -p hyprlayer-core --test real_machine_smoke -- --ignored --nocapture

use std::collections::HashMap;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryResolver, BinaryState};

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
