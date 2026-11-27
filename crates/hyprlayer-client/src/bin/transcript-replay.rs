//! `transcript-replay [--perf <log> <items>] | <name>=<log.jsonl>... --out <dir>`: the Rust half of scripts/transcript-diff.

use std::path::PathBuf;
use std::time::Instant;

use hyprlayer_client::session::replay::{fold_log, replay};
use hyprlayer_core::store::session_event_codec::decode_raw_event;
use hyprlayer_core::SessionLogEntry;

// Rows go through the server's own decoder, as they do before reaching a client.
fn read_log(path: &str) -> Vec<SessionLogEntry> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(n, line)| {
            let at = format!("{path} line {}", n + 1);
            let row: serde_json::Value =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("{at}: {e}"));
            SessionLogEntry {
                seq: row["seq"]
                    .as_i64()
                    .unwrap_or_else(|| panic!("{at}: no seq")),
                kind: row["kind"].as_str().unwrap_or_default().to_string(),
                created_at: row["createdAt"].as_f64().unwrap_or_default(),
                event: decode_raw_event(&row["event"].to_string())
                    .unwrap_or_else(|e| panic!("{at}: {e}")),
            }
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--perf") {
        let entries = read_log(&args[1]);
        let target: usize = args[2].parse().expect("item count");
        let mut cut = None;
        fold_log(&entries, |i, view| {
            if cut.is_none() && view.items.len() >= target {
                cut = Some(i + 1);
            }
        });
        let cut = cut.unwrap_or(entries.len());
        let started = Instant::now();
        let view = fold_log(&entries[..cut], |_, _| {});
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{}",
            serde_json::json!({ "events": cut, "items": view.items.len(), "ms": ms })
        );
        return;
    }
    let out_at = args.iter().position(|a| a == "--out").expect("--out <dir>");
    let out = PathBuf::from(&args[out_at + 1]);
    std::fs::create_dir_all(&out).expect("create out dir");
    for arg in args.iter().take(out_at) {
        let (name, path) = arg.split_once('=').expect("name=path");
        let result = replay(&read_log(path));
        let file = std::fs::File::create(out.join(format!("{name}.json"))).expect("create output");
        serde_json::to_writer(std::io::BufWriter::new(file), &result).expect("write output");
    }
}
