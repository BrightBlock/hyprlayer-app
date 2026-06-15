//! Smoke test against the real thoughts corpus, when present on this machine.
//! Skips silently elsewhere (CI) — the in-module unit tests carry coverage.

use std::path::Path;

use hyprlayer_core::docs::frontmatter::{self, FmPatch, FmValue};

const CORPUS: &str = "/Users/jt/thoughts";

fn corpus_files() -> Vec<std::path::PathBuf> {
    if !Path::new(CORPUS).is_dir() {
        return Vec::new();
    }
    walkdir::WalkDir::new(CORPUS)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.into_path())
        .collect()
}

#[test]
fn corpus_parses_and_noop_patch_is_identity() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("corpus not present; skipping");
        return;
    }
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

#[test]
fn corpus_status_patch_only_touches_frontmatter() {
    let files = corpus_files();
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
