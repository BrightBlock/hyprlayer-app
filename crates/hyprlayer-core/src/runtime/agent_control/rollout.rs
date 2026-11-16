//! A codex rollout read forward from where the last read stopped: the effort, plan mode and token
//! counts its hooks never carry.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use hyprlayer_models::{PtyAgentContext, PtyAgentStatus};
use memchr::memmem;
use serde::Deserialize;

/// Only these records are parsed; every other line is skipped unread.
const MARKERS: [&[u8]; 3] = [
    b"\"turn_context\"",
    b"\"thread_settings_applied\"",
    b"\"token_count\"",
];

/// One agent's read position; a new path (a resumed or cleared session) starts from the top.
#[derive(Default)]
pub(crate) struct RolloutTail {
    path: Option<PathBuf>,
    offset: u64,
}

/// What the lines since the last read reported; `None` leaves that field as it was.
#[derive(Default)]
pub(crate) struct RolloutDelta {
    model: Option<String>,
    effort: Option<String>,
    planning: Option<bool>,
    context: Option<Option<PtyAgentContext>>,
}

impl RolloutDelta {
    pub(crate) fn apply(self, status: &mut PtyAgentStatus) {
        if let Some(model) = self.model {
            status.model = Some(model);
        }
        if let Some(effort) = self.effort {
            status.effort = Some(effort);
        }
        if let Some(planning) = self.planning {
            status.planning = planning;
        }
        if let Some(context) = self.context {
            status.context = context;
        }
    }

    fn fold(&mut self, record: Record) {
        let payload = record.payload;
        let mut settings = |model: Option<String>, effort: Option<String>, mode: Option<Mode>| {
            self.model = model.or(self.model.take());
            self.effort = effort.or(self.effort.take());
            if let Some(mode) = mode.and_then(|m| m.mode) {
                self.planning = Some(mode == "plan");
            }
        };
        match (record.kind.as_str(), payload.kind.as_deref()) {
            ("turn_context", _) => {
                settings(payload.model, payload.effort, payload.collaboration_mode)
            }
            ("event_msg", Some("thread_settings_applied")) => {
                if let Some(t) = payload.thread_settings {
                    settings(t.model, t.reasoning_effort, t.collaboration_mode);
                }
            }
            // A null `info` means codex has nothing to show yet.
            ("event_msg", Some("token_count")) => {
                self.context = Some(payload.info.and_then(|info| {
                    let tokens = info.last_token_usage?.total_tokens;
                    let window = info.model_context_window?;
                    Some(PtyAgentContext {
                        tokens,
                        window,
                        percent: Some(codex_used_percent(tokens, window)),
                    })
                }));
            }
            _ => {}
        }
    }
}

impl RolloutTail {
    /// Reads whole lines past the last read; a half-written last line waits for the next call.
    pub(crate) fn read(&mut self, path: &Path) -> std::io::Result<RolloutDelta> {
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_path_buf());
            self.offset = 0;
        }
        let mut file = std::fs::File::open(path)?;
        // Rewritten shorter, the offset would sit past its end for good.
        if file.metadata()?.len() < self.offset {
            self.offset = 0;
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut delta = RolloutDelta::default();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 || line.last() != Some(&b'\n') {
                return Ok(delta);
            }
            self.offset += n as u64;
            if !MARKERS.iter().any(|m| memmem::find(&line, m).is_some()) {
                continue;
            }
            if let Ok(record) = serde_json::from_slice::<Record>(&line) {
                delta.fold(record);
            }
        }
    }
}

#[derive(Deserialize)]
struct Record {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    payload: Payload,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    collaboration_mode: Option<Mode>,
    thread_settings: Option<ThreadSettings>,
    info: Option<TokenInfo>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Mode {
    mode: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThreadSettings {
    model: Option<String>,
    reasoning_effort: Option<String>,
    collaboration_mode: Option<Mode>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TokenInfo {
    last_token_usage: Option<TokenUsage>,
    model_context_window: Option<u32>,
}

#[derive(Deserialize)]
struct TokenUsage {
    total_tokens: u32,
}

/// codex's own "% context left" (`TokenUsage::percent_of_context_window_remaining`), as used:
/// it sets aside a fixed baseline for the system prompt, so raw used/window never matches its footer.
pub(crate) fn codex_used_percent(tokens: u32, window: u32) -> u32 {
    const BASELINE: f64 = 12_000.0;
    let window = f64::from(window);
    if window <= BASELINE {
        return 100;
    }
    let effective = window - BASELINE;
    let used = (f64::from(tokens) - BASELINE).max(0.0);
    let left = ((effective - used).max(0.0) / effective * 100.0)
        .clamp(0.0, 100.0)
        .round();
    100 - left as u32
}
