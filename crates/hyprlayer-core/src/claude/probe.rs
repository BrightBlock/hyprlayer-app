//! One-shot `list_models` query for the model picker.
//!
//! The CLI answers a `list_models` control request with the same catalog its own
//! `/model` picker renders — provider, settings cascade, and enforcement policy
//! already applied. That makes it the authority on which models this install can
//! actually run, so the composer prefers it over the static table in
//! `src/lib/domain/models.ts`.
//!
//! The catalog is narrower than what `--model` accepts: legacy ids (Opus 4.7,
//! Sonnet 4.6, …) are absent from it yet still launch, and the CLI validates
//! `--model` lazily, so nothing here can confirm them. The frontend keeps its
//! static rows for those.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::protocol::control_request_json;
use crate::error::{CoreError, Result};

/// A probe is one round-trip against a freshly spawned CLI; a wedged child must
/// not hang the composer. Startup is ~1s in practice.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Our control-request id. Only one is in flight, so it needn't be unique.
const REQUEST_ID: &str = "hyprlayer-list-models";

/// One selectable model, as the CLI's own picker sees it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeModel {
    /// The `--model` value, or `None` for the "default" row (which launches with
    /// no `--model` at all). Mirrors `ModelOption.flag` on the frontend.
    pub flag: Option<String>,
    /// Canonical id the row resolves to, e.g. `claude-opus-5[1m]`. Keys the
    /// Models-API context-window lookup once the `[1m]` suffix is stripped.
    pub resolved_model: Option<String>,
    /// Picker label, e.g. `Opus (1M context)`.
    pub display_name: String,
    pub description: Option<String>,
    /// Effort tiers this model accepts. Empty means it takes no `--effort` at
    /// all (Haiku), which is different from "we don't know".
    pub supported_effort_levels: Vec<String>,
    pub supports_fast_mode: bool,
    /// Listed but not selectable — org policy can restrict a model.
    pub disabled: bool,
}

/// The CLI's selectable model catalog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProbe {
    /// Empty only if the CLI answered with an empty catalog; a probe that could
    /// not run at all is an `Err`, so the two stay distinguishable.
    pub models: Vec<ClaudeModel>,
}

/// Ask a short-lived `claude` for its model catalog.
///
/// `cwd` should be a directory the user already trusts (their home): the CLI
/// layers project settings from its working directory, and an untrusted repo can
/// prompt. `env` carries API-key/Bedrock/Vertex auth, which decides the catalog —
/// the same vars a session spawn gets.
pub async fn probe(
    bin: &Path,
    path_env: &OsString,
    cwd: &Path,
    env: &[(String, String)],
) -> Result<ClaudeProbe> {
    let models = tokio::time::timeout(PROBE_TIMEOUT, ask(bin, path_env, cwd, env))
        .await
        .unwrap_or_else(|_| {
            Err(CoreError::ExecTimeout {
                program: "claude list_models".into(),
                seconds: PROBE_TIMEOUT.as_secs(),
            })
        })?;
    Ok(ClaudeProbe { models })
}

async fn ask(
    bin: &Path,
    path_env: &OsString,
    cwd: &Path,
    env: &[(String, String)],
) -> Result<Vec<ClaudeModel>> {
    let mut cmd = Command::new(bin);
    cmd.arg("-p")
        .args(["--input-format", "stream-json"])
        .args(["--output-format", "stream-json"])
        .arg("--verbose")
        .current_dir(cwd)
        .env("PATH", path_env)
        .env("CLAUDE_CODE_ENTRYPOINT", "hyprlayer-app")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Discarded, not piped: nothing reads stderr, and a chatty child would
        // fill the pipe buffer, block on write, and never answer.
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CoreError::BinaryMissing {
                name: "claude".into(),
            }
        } else {
            CoreError::Io(e)
        }
    })?;
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let out = exchange(stdin, stdout).await;
    let _ = child.kill().await;
    out
}

/// Write the request, read until the matching `control_response`. Split out so
/// the caller can reap the child on every exit path.
async fn exchange(
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
) -> Result<Vec<ClaudeModel>> {
    let line = control_request_json(REQUEST_ID, "list_models", json!({}));
    stdin.write_all(format!("{line}\n").as_bytes()).await?;
    stdin.flush().await?;
    // stdin stays open until the answer lands — EOF means "shut down", and the
    // CLI can win that race and exit before replying.

    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(response) = control_response_for(&line, REQUEST_ID) else {
            continue;
        };
        return match response {
            Ok(body) => Ok(parse_models(&body)),
            Err(detail) => Err(CoreError::OutputParse {
                program: "claude".into(),
                detail: format!("list_models: {detail}"),
            }),
        };
    }
    Err(CoreError::OutputParse {
        program: "claude".into(),
        detail: "exited without answering list_models".into(),
    })
}

/// Pick our `control_response` out of the stream. `Ok(body)` is the success
/// payload, `Err(msg)` the CLI's refusal; `None` for every other line (the
/// stream also carries `system`/`assistant` traffic).
fn control_response_for(line: &str, request_id: &str) -> Option<std::result::Result<Value, String>> {
    let v: Value = serde_json::from_str(line).ok()?;
    (v.get("type").and_then(Value::as_str) == Some("control_response")).then_some(())?;
    let response = v.get("response")?;
    (response.get("request_id").and_then(Value::as_str) == Some(request_id)).then_some(())?;
    if response.get("subtype").and_then(Value::as_str) == Some("error") {
        return Some(Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string()));
    }
    Some(Ok(response.get("response").cloned().unwrap_or(Value::Null)))
}

/// Pure parse of a `list_models` body. Split from the process work so it stays
/// testable against a captured payload.
fn parse_models(body: &Value) -> Vec<ClaudeModel> {
    body.get("models")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|m| {
                    let value = m.get("value").and_then(Value::as_str)?;
                    let str_field = |k: &str| {
                        m.get(k)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    };
                    Some(ClaudeModel {
                        // The CLI spells the no-`--model` row "default"; the app
                        // models that as a null flag.
                        flag: (value != "default").then(|| value.to_string()),
                        resolved_model: str_field("resolvedModel"),
                        display_name: str_field("displayName").unwrap_or_else(|| value.to_string()),
                        description: str_field("description"),
                        supported_effort_levels: m
                            .get("supportedEffortLevels")
                            .and_then(Value::as_array)
                            .map(|levels| {
                                levels
                                    .iter()
                                    .filter_map(|l| l.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        supports_fast_mode: m
                            .get("supportsFastMode")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        disabled: m.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where to run the probe: the user's home, else the system temp dir. Never a
/// repo — project settings would colour the catalog and can prompt for trust.
pub fn neutral_cwd() -> PathBuf {
    dirs::home_dir().unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from `claude` 2.1.219, trimmed to three rows.
    fn captured() -> Value {
        json!({
            "models": [
                {
                    "value": "default",
                    "resolvedModel": "claude-opus-5[1m]",
                    "displayName": "Default (recommended)",
                    "description": "Opus 5 with 1M context · Best for everyday, complex tasks",
                    "supportsEffort": true,
                    "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"],
                    "supportsAdaptiveThinking": true,
                    "supportsFastMode": true,
                    "supportsAutoMode": true
                },
                {
                    "value": "claude-fable-5[1m]",
                    "resolvedModel": "claude-fable-5",
                    "displayName": "Fable",
                    "description": "Fable 5 · Most capable for your hardest and longest-running tasks",
                    "supportsEffort": true,
                    "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"],
                    "supportsAdaptiveThinking": true,
                    "supportsAutoMode": true
                },
                {
                    "value": "haiku",
                    "resolvedModel": "claude-haiku-4-5-20251001",
                    "displayName": "Haiku",
                    "description": "Haiku 4.5 · Fastest for quick answers"
                }
            ]
        })
    }

    #[test]
    fn parses_the_captured_catalog() {
        let models = parse_models(&captured());
        assert_eq!(models.len(), 3);

        // The "default" row is the no-`--model` launch, so its flag is null.
        let default = &models[0];
        assert_eq!(default.flag, None);
        assert_eq!(default.resolved_model.as_deref(), Some("claude-opus-5[1m]"));
        assert_eq!(default.supported_effort_levels, ["low", "medium", "high", "xhigh", "max"]);
        assert!(default.supports_fast_mode);
        assert!(!default.disabled);

        assert_eq!(models[1].flag.as_deref(), Some("claude-fable-5[1m]"));
        // Fast mode is Opus-only; an absent flag is false, not unknown.
        assert!(!models[1].supports_fast_mode);

        // Haiku reports no effort support at all — an empty list, which the
        // composer must render as "no effort picker" rather than "all tiers".
        assert!(models[2].supported_effort_levels.is_empty());
    }

    #[test]
    fn tolerates_a_missing_or_odd_body() {
        assert!(parse_models(&json!({})).is_empty());
        assert!(parse_models(&json!({ "models": [] })).is_empty());
        // A row with no value can't be launched — drop it rather than emit a blank.
        assert!(parse_models(&json!({ "models": [{ "displayName": "x" }] })).is_empty());
        // No displayName falls back to the value, never an empty label.
        let m = parse_models(&json!({ "models": [{ "value": "sonnet" }] }));
        assert_eq!(m[0].display_name, "sonnet");
        assert_eq!(m[0].description, None);
    }

    #[test]
    fn picks_our_control_response_out_of_the_stream() {
        let other = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        assert!(control_response_for(other, REQUEST_ID).is_none());
        assert!(control_response_for("not json", REQUEST_ID).is_none());

        // A response to somebody else's request must not be mistaken for ours.
        let mismatched =
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"other","response":{}}}"#;
        assert!(control_response_for(mismatched, REQUEST_ID).is_none());

        let ours = format!(
            r#"{{"type":"control_response","response":{{"subtype":"success","request_id":"{REQUEST_ID}","response":{{"models":[]}}}}}}"#
        );
        assert!(matches!(control_response_for(&ours, REQUEST_ID), Some(Ok(_))));
    }

    #[test]
    fn surfaces_a_refusal_instead_of_waiting_for_a_success() {
        // An older CLI that doesn't know `list_models` errors rather than
        // answering; that must not read as an empty catalog.
        let err = format!(
            r#"{{"type":"control_response","response":{{"subtype":"error","request_id":"{REQUEST_ID}","error":"Unknown subtype"}}}}"#
        );
        assert_eq!(
            control_response_for(&err, REQUEST_ID),
            Some(Err("Unknown subtype".to_string()))
        );
    }
}
