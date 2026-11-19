use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::protocol::{decode_line, Incoming};
use crate::error::{CoreError, Result};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CodexAuthStatus {
    pub method: Option<String>,
    pub email: Option<String>,
    pub plan: Option<String>,
}

impl CodexAuthStatus {
    pub fn signed_in(&self) -> bool {
        self.method.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CodexModel {
    pub id: String,
    pub display_name: String,
    pub supported_efforts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CodexProbe {
    pub auth: CodexAuthStatus,
    pub models: Vec<CodexModel>,
}

pub async fn probe(bin: &Path, path_env: &OsString) -> Result<CodexProbe> {
    let replies = ask(
        bin,
        path_env,
        &[("account/read", json!({})), ("model/list", json!({}))],
    )
    .await?;
    Ok(CodexProbe {
        auth: parse_auth(&replies[0]),
        models: parse_models(&replies[1]),
    })
}

fn parse_auth(result: &Value) -> CodexAuthStatus {
    let account = result.get("account").filter(|a| !a.is_null());
    let field = |k: &str| {
        account
            .and_then(|a| a.get(k))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    CodexAuthStatus {
        method: field("type"),
        email: field("email"),
        plan: field("planType"),
    }
}

pub(crate) fn parse_models(result: &Value) -> Vec<CodexModel> {
    result
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|m| m.get("hidden").and_then(Value::as_bool) != Some(true))
                .filter_map(|m| {
                    let id = m.get("id").and_then(Value::as_str)?.to_string();
                    Some(CodexModel {
                        display_name: m
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or(&id)
                            .to_string(),
                        supported_efforts: m
                            .get("supportedReasoningEfforts")
                            .and_then(Value::as_array)
                            .map(|efforts| {
                                efforts
                                    .iter()
                                    .filter_map(|e| {
                                        e.get("reasoningEffort")
                                            .and_then(Value::as_str)
                                            .or_else(|| e.as_str())
                                            .map(str::to_string)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn ask(bin: &Path, path_env: &OsString, calls: &[(&str, Value)]) -> Result<Vec<Value>> {
    let label = calls
        .iter()
        .map(|(m, _)| *m)
        .collect::<Vec<_>>()
        .join(" + ");
    tokio::time::timeout(PROBE_TIMEOUT, ask_inner(bin, path_env, calls))
        .await
        .unwrap_or_else(|_| {
            Err(CoreError::ExecTimeout {
                program: format!("codex app-server {label}"),
                seconds: PROBE_TIMEOUT.as_secs(),
            })
        })
}

async fn ask_inner(bin: &Path, path_env: &OsString, calls: &[(&str, Value)]) -> Result<Vec<Value>> {
    let _startup = super::startup_lock().lock().await;

    let mut cmd = Command::new(bin);
    crate::process::appimage::scrub_tokio(&mut cmd);
    cmd.args(["app-server", "--stdio"])
        .env("PATH", path_env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CoreError::BinaryMissing {
                name: "codex".into(),
            }
        } else {
            CoreError::Io(e)
        }
    })?;
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");

    let out = exchange(stdin, stdout, calls).await;

    let _ = child.kill().await;
    out
}

async fn exchange(
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    calls: &[(&str, Value)],
) -> Result<Vec<Value>> {
    let mut script = format!(
        "{}\n{}\n",
        json!({
            "id": 1,
            "method": "initialize",
            "params": { "clientInfo": {
                "name": "hyprlayer",
                "title": "Hyprlayer",
                "version": env!("CARGO_PKG_VERSION"),
            } }
        }),
        json!({ "method": "initialized", "params": {} }),
    );
    for (i, (method, params)) in calls.iter().enumerate() {
        script.push_str(&format!(
            "{}\n",
            json!({ "id": i + 2, "method": method, "params": params })
        ));
    }
    stdin.write_all(script.as_bytes()).await?;
    stdin.flush().await?;

    let mut out: Vec<Option<Value>> = vec![None; calls.len()];
    let mut remaining = calls.len();
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match decode_line(&line) {
            Some(Incoming::Response {
                id: 1,
                result: Err(e),
            }) => {
                return Err(CoreError::OutputParse {
                    program: "codex".into(),
                    detail: format!("initialize: {e}"),
                });
            }
            Some(Incoming::Response { id, result }) if id >= 2 => {
                let idx = (id - 2) as usize;
                let Some(slot) = out.get_mut(idx) else {
                    continue;
                };
                if slot.is_some() {
                    continue;
                }
                let (method, _) = calls[idx];
                *slot = Some(result.map_err(|e| CoreError::OutputParse {
                    program: "codex".into(),
                    detail: format!("{method}: {e}"),
                })?);
                remaining -= 1;
                if remaining == 0 {
                    return Ok(out.into_iter().map(Option::unwrap).collect());
                }
            }
            _ => continue,
        }
    }

    let missing = calls
        .iter()
        .zip(&out)
        .filter(|(_, slot)| slot.is_none())
        .map(|((m, _), _)| *m)
        .collect::<Vec<_>>()
        .join(", ");
    Err(CoreError::OutputParse {
        program: "codex".into(),
        detail: format!("app-server exited without answering: {missing}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_models_keeps_visible_rows_with_their_efforts() {
        let body = json!({
            "data": [
                {
                    "id": "gpt-5.4-codex",
                    "model": "gpt-5.4-codex",
                    "displayName": "GPT-5.4 Codex",
                    "description": "Fast agentic coding",
                    "hidden": false,
                    "isDefault": true,
                    "defaultReasoningEffort": "medium",
                    "supportedReasoningEfforts": [
                        { "reasoningEffort": "low", "description": "" },
                        { "reasoningEffort": "medium", "description": "" },
                        { "reasoningEffort": "high", "description": "" }
                    ]
                },
                { "id": "internal-eval", "displayName": "Internal", "hidden": true }
            ],
            "nextCursor": null
        });
        let models = parse_models(&body);
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id, "gpt-5.4-codex");
        assert_eq!(m.display_name, "GPT-5.4 Codex");
        assert_eq!(m.supported_efforts, ["low", "medium", "high"]);
    }

    #[test]
    fn parse_models_tolerates_a_missing_or_odd_body() {
        assert!(parse_models(&json!({})).is_empty());
        assert!(parse_models(&json!({ "data": [] })).is_empty());
        assert!(parse_models(&json!({ "data": [{ "displayName": "x" }] })).is_empty());
        let m = parse_models(&json!({ "data": [{ "id": "gpt-5.4" }] }));
        assert_eq!(m[0].display_name, "gpt-5.4");
        assert!(m[0].supported_efforts.is_empty());
    }
}
