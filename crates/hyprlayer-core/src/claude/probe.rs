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

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

const REQUEST_ID: &str = "hyprlayer-list-models";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeModel {
    pub flag: Option<String>,
    pub resolved_model: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    pub supported_effort_levels: Vec<String>,
    pub supports_fast_mode: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProbe {
    pub models: Vec<ClaudeModel>,
}

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
    crate::process::appimage::scrub_tokio(&mut cmd);
    cmd.arg("-p")
        .args(["--input-format", "stream-json"])
        .args(["--output-format", "stream-json"])
        .arg("--verbose")
        .current_dir(cwd)
        .env("PATH", path_env)
        .env("CLAUDE_CODE_ENTRYPOINT", "hyprlayer-app")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
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

async fn exchange(
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
) -> Result<Vec<ClaudeModel>> {
    let line = control_request_json(REQUEST_ID, "list_models", json!({}));
    stdin.write_all(format!("{line}\n").as_bytes()).await?;
    stdin.flush().await?;

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

fn control_response_for(
    line: &str,
    request_id: &str,
) -> Option<std::result::Result<Value, String>> {
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

pub fn neutral_cwd(home: Option<&Path>) -> PathBuf {
    home.map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let default = &models[0];
        assert_eq!(default.flag, None);
        assert_eq!(default.resolved_model.as_deref(), Some("claude-opus-5[1m]"));
        assert_eq!(
            default.supported_effort_levels,
            ["low", "medium", "high", "xhigh", "max"]
        );
        assert!(default.supports_fast_mode);
        assert!(!default.disabled);

        assert_eq!(models[1].flag.as_deref(), Some("claude-fable-5[1m]"));
        assert!(!models[1].supports_fast_mode);

        assert!(models[2].supported_effort_levels.is_empty());
    }

    #[test]
    fn tolerates_a_missing_or_odd_body() {
        assert!(parse_models(&json!({})).is_empty());
        assert!(parse_models(&json!({ "models": [] })).is_empty());
        assert!(parse_models(&json!({ "models": [{ "displayName": "x" }] })).is_empty());
        let m = parse_models(&json!({ "models": [{ "value": "sonnet" }] }));
        assert_eq!(m[0].display_name, "sonnet");
        assert_eq!(m[0].description, None);
    }

    #[test]
    fn picks_our_control_response_out_of_the_stream() {
        let other = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        assert!(control_response_for(other, REQUEST_ID).is_none());
        assert!(control_response_for("not json", REQUEST_ID).is_none());

        let mismatched = r#"{"type":"control_response","response":{"subtype":"success","request_id":"other","response":{}}}"#;
        assert!(control_response_for(mismatched, REQUEST_ID).is_none());

        let ours = format!(
            r#"{{"type":"control_response","response":{{"subtype":"success","request_id":"{REQUEST_ID}","response":{{"models":[]}}}}}}"#
        );
        assert!(matches!(
            control_response_for(&ours, REQUEST_ID),
            Some(Ok(_))
        ));
    }

    #[test]
    fn surfaces_a_refusal_instead_of_waiting_for_a_success() {
        let err = format!(
            r#"{{"type":"control_response","response":{{"subtype":"error","request_id":"{REQUEST_ID}","error":"Unknown subtype"}}}}"#
        );
        assert_eq!(
            control_response_for(&err, REQUEST_ID),
            Some(Err("Unknown subtype".to_string()))
        );
    }
}
