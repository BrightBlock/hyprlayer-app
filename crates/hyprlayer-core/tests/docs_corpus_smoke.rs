use std::path::PathBuf;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::docs::frontmatter::{self, FmError, FmPatch, FmValue};
use hyprlayer_core::docs::parts::{compose, split, FrontmatterEdit};
use hyprlayer_core::index::thoughts_roots;

async fn corpus_roots() -> Vec<PathBuf> {
    let from_config = match dirs::config_dir() {
        Some(dir) => {
            let service = ConfigService::new(dir.join("hyprlayer").join("config.json"));
            match service.load().await {
                Ok(loaded) => thoughts_roots(&loaded.config)
                    .into_iter()
                    .map(|r| r.root)
                    .filter(|p| p.is_dir())
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
        None => Vec::new(),
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
    eprintln!("corpus: {} files", files.len());
    assert!(
        files.len() >= 10,
        "expected a real corpus, got {}",
        files.len()
    );
    for path in &files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let parsed = frontmatter::parse(&content);
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
    let mut refused: Vec<&PathBuf> = Vec::new();
    for path in &files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let out = match frontmatter::patch_frontmatter(&content, &patch) {
            Ok(out) => out,
            Err(FmError::Malformed) => {
                refused.push(path);
                continue;
            }
            Err(e) => panic!("patch failed for {}: {e}", path.display()),
        };
        let before = frontmatter::parse(&content);
        let after = frontmatter::parse(&out);
        assert_eq!(
            after.fm.status.as_deref(),
            Some("archived"),
            "{}",
            path.display()
        );
        assert_eq!(
            &out[after.body_offset..],
            &content[before.body_offset..],
            "body changed for {}",
            path.display()
        );
    }
    eprintln!("refused as malformed: {} files", refused.len());
    for path in &refused {
        eprintln!("  {}", path.display());
    }
}

#[tokio::test]
async fn corpus_split_and_empty_save_is_identity() {
    let files = corpus_files().await;
    if files.is_empty() {
        eprintln!("corpus not present; skipping");
        return;
    }

    let no_op = FrontmatterEdit::Patch(FmPatch::default());
    let mut fm_errors: Vec<PathBuf> = Vec::new();

    for path in &files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let parts = split(&content);
        if parts.fm_error.is_some() {
            fm_errors.push(path.clone());
        }

        let body = parts.body(&content).to_string();
        let out = compose(&content, &body, &no_op)
            .unwrap_or_else(|e| panic!("compose (no-op) failed for {}: {e}", path.display()));
        assert_eq!(out, content, "no-op compose mutated {}", path.display());

        let lf_body = body.replace("\r\n", "\n");
        let out = compose(&content, &lf_body, &no_op).unwrap_or_else(|e| {
            panic!(
                "compose (LF-normalised body) failed for {}: {e}",
                path.display()
            )
        });
        assert_eq!(
            out,
            content,
            "LF-normalised no-op compose mutated {}",
            path.display()
        );
    }

    eprintln!("fm_error: {} files", fm_errors.len());
    for path in &fm_errors {
        eprintln!("  {}", path.display());
    }
}
