//! One-off backfill for PR records that predate the merge-time promotion in
//! `src-tauri/src/pr_watch.rs`.
//!
//! The watcher only polls *active* workstreams, so records whose workstream was
//! archived long ago can never self-heal. This walks a `prs/` directory and runs
//! the same `pr_record::promote_merged_record` the watcher calls, for exactly
//! the PR numbers fed on stdin (one per line — the caller decides which PRs are
//! merged; this does not talk to GitHub).
//!
//! Usage:
//!   printf '%s\n' 216 215 | cargo run -p hyprlayer-core --example backfill_pr_records -- <prs-dir> [--apply]
//!
//! Without `--apply` it is a dry run and writes nothing.

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

use hyprlayer_core::docs::pr_record;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: backfill_pr_records <prs-dir> [--apply]  (PR numbers on stdin)");
        std::process::exit(2);
    };
    let apply = args.any(|a| a == "--apply");

    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .expect("read PR numbers from stdin");
    let merged: HashSet<u32> = raw.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    assert!(!merged.is_empty(), "no PR numbers on stdin");

    let mut promoted = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    entries.sort();

    for path in entries {
        let Some(number) = path
            .file_name()
            .and_then(|f| f.to_str())
            .and_then(|f| f.split('_').next())
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if !merged.contains(&number) {
            continue;
        }

        if apply {
            match pr_record::promote_merged_record(&path) {
                Ok(true) => {
                    promoted += 1;
                    println!("promoted  #{number}");
                }
                Ok(false) => skipped += 1,
                Err(e) => {
                    failed += 1;
                    eprintln!("FAILED    #{number}: {e}");
                }
            }
        } else {
            // Dry run: same decision, no write.
            let content = std::fs::read_to_string(&path).expect("read record");
            match pr_record::promote_merged(&content) {
                Ok(Some(_)) => {
                    promoted += 1;
                    println!("would promote  #{number}");
                }
                Ok(None) => skipped += 1,
                Err(e) => {
                    failed += 1;
                    eprintln!("FAILED    #{number}: {e}");
                }
            }
        }
    }

    println!(
        "\n{} {promoted}, left alone {skipped}, failed {failed}",
        if apply { "promoted" } else { "would promote" }
    );
}
