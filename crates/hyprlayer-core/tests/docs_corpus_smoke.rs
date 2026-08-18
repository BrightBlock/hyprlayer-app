//! Smoke test against the real thoughts corpus, when present on this machine.
//! Skips silently elsewhere (CI) — the in-module unit tests carry coverage.
//!
//! The corpus location comes from this machine's hyprlayer config (every
//! filesystem-backed root it configures), falling back to `~/thoughts` when
//! there is no usable config. It is deliberately *not* a hardcoded path: one
//! used to live here, and because a missing corpus skips silently, it made
//! this file a no-op on every machine but the one it was written on.

use std::path::PathBuf;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::docs::frontmatter::{self, FmPatch, FmValue};
use hyprlayer_core::index::thoughts_roots;

/// Filesystem thoughts roots to smoke-test: whatever the real config points
/// at, or `~/thoughts` when this machine has no usable config. Empty when
/// neither exists, which the tests treat as "skip".
async fn corpus_roots() -> Vec<PathBuf> {
    let from_config = match ConfigService::with_default_path() {
        Ok(service) => match service.load().await {
            Ok(loaded) => thoughts_roots(&loaded.config)
                .into_iter()
                .map(|r| r.root)
                .filter(|p| p.is_dir())
                .collect(),
            // An absent or unparseable config is a "no corpus here" signal,
            // not a failure of what this file tests.
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if !from_config.is_empty() {
        return from_config;
    }
    dirs::home_dir()
        .map(|home| home.join("thoughts"))
        .filter(|p| p.is_dir())
        .into_iter()
        .collect()
}

async fn corpus_files() -> Vec<PathBuf> {
    corpus_roots()
        .await
        .into_iter()
        .flat_map(|root| {
            walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .map(|e| e.into_path())
        })
        .collect()
}

#[tokio::test]
async fn corpus_parses_and_noop_patch_is_identity() {
    let files = corpus_files().await;
    if files.is_empty() {
        eprintln!("corpus not present; skipping");
        return;
    }
    // Say the size out loud: a silent skip is indistinguishable from a pass
    // in the summary line, which is how the old hardcoded path went unnoticed.
    eprintln!("corpus: {} files", files.len());
    assert!(
        files.len() >= 10,
        "expected a real corpus, got {}",
        files.len()
    );
    for path in &files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue; // non-UTF8 etc. — out of scope
        };
        let parsed = frontmatter::parse(&content); // must never panic
        let _ = &parsed.fm;

        let out = frontmatter::patch_frontmatter(&content, &FmPatch::default())
            .unwrap_or_else(|e| panic!("noop patch failed for {}: {e}", path.display()));
        assert_eq!(out, content, "noop patch mutated {}", path.display());
    }
}

#[tokio::test]
async fn corpus_status_patch_only_touches_frontmatter() {
    let files = corpus_files().await;
    if files.is_empty() {
        eprintln!("corpus not present; skipping");
        return;
    }
    let patch = FmPatch {
        set: vec![("status".to_string(), FmValue::Str("archived".to_string()))],
        remove: vec![],
    };
    for path in &files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let out = frontmatter::patch_frontmatter(&content, &patch)
            .unwrap_or_else(|e| panic!("patch failed for {}: {e}", path.display()));
        let before = frontmatter::parse(&content);
        let after = frontmatter::parse(&out);
        assert_eq!(
            after.fm.status.as_deref(),
            Some("archived"),
            "{}",
            path.display()
        );
        // Body bytes are never modified.
        assert_eq!(
            &out[after.body_offset..],
            &content[before.body_offset..],
            "body changed for {}",
            path.display()
        );
    }
}
