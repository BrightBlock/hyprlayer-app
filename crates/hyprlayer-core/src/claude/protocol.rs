use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub enum ParsedLine {
    Init {
        claude_session_id: Option<String>,
        model: Option<String>,
        tools: Vec<String>,
    },
    System {
        subtype: String,
    },
    Assistant {
        blocks: Vec<Block>,
        context_tokens: Option<u32>,
        parent_id: Option<String>,
    },
    User {
        blocks: Vec<Block>,
        parent_id: Option<String>,
        uuid: Option<String>,
        is_replay: bool,
        is_synthetic: bool,
    },
    PartialText {
        text: String,
        parent_id: Option<String>,
    },
    CanUseTool {
        request_id: String,
        tool_name: String,
        input: Value,
    },
    Result {
        is_error: bool,
        total_cost_usd: Option<f64>,
        num_turns: Option<u32>,
        stop_reason: Option<String>,
        context_tokens: Option<u32>,
    },
    CompactBoundary {
        trigger: String,
        pre_tokens: Option<u32>,
    },
    TaskNotification {
        task_id: String,
        tool_use_id: Option<String>,
        status: String,
    },
    CommandLifecycle {
        command_uuid: String,
        state: String,
    },
    ControlResponse {
        request_id: String,
        payload: Value,
    },
    Unknown {
        raw: String,
    },
}

#[derive(Debug, Clone)]
pub enum Block {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Other {
        kind: String,
    },
}

pub fn parse_line(line: &str) -> ParsedLine {
    let trimmed = line.trim();
    let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
        return ParsedLine::Unknown {
            raw: trimmed.to_string(),
        };
    };
    match v.get("type").and_then(Value::as_str) {
        Some("system") => {
            let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
            match subtype {
                "init" => ParsedLine::Init {
                    claude_session_id: str_field(&v, "session_id"),
                    model: str_field(&v, "model"),
                    tools: string_list(v.get("tools")),
                },
                "compact_boundary" => {
                    let meta = v.get("compact_metadata");
                    let trigger = meta
                        .and_then(|m| m.get("trigger"))
                        .and_then(Value::as_str)
                        .unwrap_or("auto")
                        .to_string();
                    let pre_tokens = meta
                        .and_then(|m| m.get("pre_tokens"))
                        .and_then(Value::as_u64)
                        .map(|n| n.min(u32::MAX as u64) as u32);
                    ParsedLine::CompactBoundary {
                        trigger,
                        pre_tokens,
                    }
                }
                "task_notification" => match str_field(&v, "task_id") {
                    Some(task_id) => ParsedLine::TaskNotification {
                        task_id,
                        tool_use_id: str_field(&v, "tool_use_id"),
                        status: str_field(&v, "status").unwrap_or_default(),
                    },
                    None => ParsedLine::System {
                        subtype: "task_notification".into(),
                    },
                },
                other => ParsedLine::System {
                    subtype: other.to_string(),
                },
            }
        }
        Some("queue-operation") => parse_queue_operation(&v),
        Some("assistant") => ParsedLine::Assistant {
            blocks: parse_blocks(v.get("message")),
            context_tokens: context_tokens(v.get("message").and_then(|m| m.get("usage"))),
            parent_id: str_field(&v, "parent_tool_use_id"),
        },
        Some("user") => ParsedLine::User {
            blocks: parse_blocks(v.get("message")),
            parent_id: str_field(&v, "parent_tool_use_id"),
            uuid: str_field(&v, "uuid"),
            is_replay: v.get("isReplay").and_then(Value::as_bool).unwrap_or(false),
            is_synthetic: v
                .get("isSynthetic")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        Some("stream_event") => parse_stream_event(&v),
        Some("result") => ParsedLine::Result {
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            total_cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
            num_turns: v.get("num_turns").and_then(Value::as_u64).map(|n| n as u32),
            stop_reason: str_field(&v, "stop_reason"),
            context_tokens: context_tokens(v.get("usage")),
        },
        Some("control_request") => parse_control_request(&v),
        Some("control_response") => parse_control_response(&v),
        Some("control_cancel_request") => ParsedLine::System {
            subtype: "control_cancel_request".into(),
        },
        Some("rate_limit_event") => ParsedLine::System {
            subtype: "rate_limit".into(),
        },
        Some("tool_progress") => ParsedLine::System {
            subtype: "tool_progress".into(),
        },
        Some("command_lifecycle") => match str_field(&v, "command_uuid") {
            Some(command_uuid) => ParsedLine::CommandLifecycle {
                command_uuid,
                state: str_field(&v, "state").unwrap_or_default(),
            },
            None => ParsedLine::System {
                subtype: "command_lifecycle".into(),
            },
        },
        _ => ParsedLine::Unknown {
            raw: trimmed.to_string(),
        },
    }
}

fn parse_stream_event(v: &Value) -> ParsedLine {
    let event = v.get("event");
    let text = event
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("content_block_delta"))
        .and_then(|e| e.get("delta"))
        .filter(|d| d.get("type").and_then(Value::as_str) == Some("text_delta"))
        .and_then(|d| d.get("text"))
        .and_then(Value::as_str);
    match text {
        Some(text) => ParsedLine::PartialText {
            text: text.to_string(),
            parent_id: str_field(v, "parent_tool_use_id"),
        },
        None => ParsedLine::System {
            subtype: "stream_event".into(),
        },
    }
}

fn request_field<'a>(request: &'a Value, key: &str) -> Option<&'a str> {
    request.get(key).and_then(Value::as_str)
}

fn parse_queue_operation(v: &Value) -> ParsedLine {
    let content = v.get("content").and_then(Value::as_str).unwrap_or("");
    if content.contains("<task-notification>") {
        if let Some(task_id) = xml_tag(content, "task-id") {
            return ParsedLine::TaskNotification {
                task_id: task_id.to_string(),
                tool_use_id: xml_tag(content, "tool-use-id").map(str::to_string),
                status: xml_tag(content, "status")
                    .map(str::to_string)
                    .unwrap_or_default(),
            };
        }
    }
    ParsedLine::System {
        subtype: "queue-operation".into(),
    }
}

fn xml_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(s[start..end].trim())
}

fn parse_control_response(v: &Value) -> ParsedLine {
    let resp = v.get("response");
    match resp
        .and_then(|r| r.get("request_id"))
        .and_then(Value::as_str)
    {
        Some(id) => ParsedLine::ControlResponse {
            request_id: id.to_string(),
            payload: resp
                .and_then(|r| r.get("response"))
                .cloned()
                .unwrap_or(Value::Null),
        },
        None => ParsedLine::System {
            subtype: "control_response".into(),
        },
    }
}

fn parse_control_request(v: &Value) -> ParsedLine {
    parse_can_use_tool(v).unwrap_or_else(|| ParsedLine::Unknown { raw: v.to_string() })
}

fn parse_can_use_tool(v: &Value) -> Option<ParsedLine> {
    let request = v.get("request")?;
    (request_field(request, "subtype") == Some("can_use_tool")).then_some(())?;
    Some(ParsedLine::CanUseTool {
        request_id: str_field(v, "request_id")?,
        tool_name: request_field(request, "tool_name")?.to_string(),
        input: request.get("input").cloned().unwrap_or(Value::Null),
    })
}

fn parse_blocks(message: Option<&Value>) -> Vec<Block> {
    let Some(content) = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    content
        .iter()
        .map(|b| match b.get("type").and_then(Value::as_str) {
            Some("text") => Block::Text {
                text: b
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
            Some("thinking") => Block::Thinking {
                text: b
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
            Some("tool_use") => Block::ToolUse {
                id: str_field(b, "id").unwrap_or_default(),
                name: str_field(b, "name").unwrap_or_default(),
                input: b.get("input").cloned().unwrap_or(Value::Null),
            },
            Some("tool_result") => Block::ToolResult {
                tool_use_id: str_field(b, "tool_use_id").unwrap_or_default(),
                content: tool_result_text(b.get("content")),
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            },
            Some(other) => Block::Other {
                kind: other.to_string(),
            },
            None => Block::Other {
                kind: "unknown".into(),
            },
        })
        .collect()
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn context_tokens(usage: Option<&Value>) -> Option<u32> {
    let usage = usage?;
    let source = usage
        .get("iterations")
        .and_then(Value::as_array)
        .and_then(|iters| iters.last())
        .unwrap_or(usage);
    let field = |key: &str| source.get(key).and_then(Value::as_u64).unwrap_or(0);
    let total = field("input_tokens")
        + field("cache_read_input_tokens")
        + field("cache_creation_input_tokens");
    (total > 0).then_some(total.min(u32::MAX as u64) as u32)
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn user_turn_json(session_id: &str, text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "parent_tool_use_id": null,
        "session_id": session_id,
    })
    .to_string()
}

pub fn permission_response_json(
    request_id: &str,
    allow: bool,
    updated_input: Option<Value>,
    message: Option<&str>,
) -> String {
    let inner = if allow {
        json!({
            "behavior": "allow",
            "updatedInput": updated_input.unwrap_or_else(|| json!({})),
        })
    } else {
        json!({
            "behavior": "deny",
            "message": message.unwrap_or("Denied by the user."),
        })
    };
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": inner,
        },
    })
    .to_string()
}

pub fn control_request_json(request_id: &str, subtype: &str, extra: Value) -> String {
    let mut request = json!({ "subtype": subtype });
    if let Value::Object(map) = extra {
        for (k, v) in map {
            request[k] = v;
        }
    }
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_notification_completed_parses() {
        let line = r#"{"type":"system","subtype":"task_notification","task_id":"bxmc1fof8","tool_use_id":"toolu_01HgG","status":"completed","output_file":"/tmp/x.output","summary":"Background command \"Sleep\" completed (exit code 0)","session_id":"s"}"#;
        match parse_line(line) {
            ParsedLine::TaskNotification {
                task_id,
                tool_use_id,
                status,
            } => {
                assert_eq!(task_id, "bxmc1fof8");
                assert_eq!(tool_use_id.as_deref(), Some("toolu_01HgG"));
                assert_eq!(status, "completed");
            }
            other => panic!("expected TaskNotification, got {other:?}"),
        }
    }

    #[test]
    fn task_notification_failed_parses() {
        let line = r#"{"type":"system","subtype":"task_notification","task_id":"bzxqr580k","tool_use_id":"toolu_01W","status":"failed","summary":"… failed with exit code 7","session_id":"s"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::TaskNotification { status, .. } if status == "failed"
        ));
    }

    #[test]
    fn task_notification_without_task_id_falls_back_to_system() {
        let line = r#"{"type":"system","subtype":"task_notification","status":"completed"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "task_notification"
        ));
    }

    #[test]
    fn other_task_lifecycle_lines_stay_system() {
        for subtype in ["task_started", "task_updated"] {
            let line = format!(r#"{{"type":"system","subtype":"{subtype}","task_id":"x"}}"#);
            assert!(matches!(
                parse_line(&line),
                ParsedLine::System { subtype: s } if s == subtype
            ));
        }
    }

    #[test]
    fn task_notification_via_queue_operation_parses() {
        let line = r#"{"type":"queue-operation","operation":"add","timestamp":"2026-06-27T19:00:00Z","sessionId":"s","content":"<task-notification>\n<task-id>b9vew92w3</task-id>\n<tool-use-id>toolu_01KXEEjFzSxWBxnYuvAz9pdG</tool-use-id>\n<output-file>/tmp/b9vew92w3.output</output-file>\n<status>completed</status>\n<summary>Background command \"Run NSIS build\" completed (exit code 0)</summary>\n</task-notification>"}"#;
        match parse_line(line) {
            ParsedLine::TaskNotification {
                task_id,
                tool_use_id,
                status,
            } => {
                assert_eq!(task_id, "b9vew92w3");
                assert_eq!(
                    tool_use_id.as_deref(),
                    Some("toolu_01KXEEjFzSxWBxnYuvAz9pdG")
                );
                assert_eq!(status, "completed");
            }
            other => panic!("expected TaskNotification, got {other:?}"),
        }
    }

    #[test]
    fn task_notification_via_queue_operation_failed_parses() {
        let line = r#"{"type":"queue-operation","content":"<task-notification>\n<task-id>bzx</task-id>\n<tool-use-id>toolu_9</tool-use-id>\n<status>failed</status>\n</task-notification>"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::TaskNotification { status, .. } if status == "failed"
        ));
    }

    #[test]
    fn non_task_queue_operation_is_system() {
        let line = r#"{"type":"queue-operation","operation":"add","content":"some queued prompt"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "queue-operation"
        ));
    }

    #[test]
    fn command_lifecycle_pair_parses_and_shares_a_command_uuid() {
        let started = r#"{"type":"command_lifecycle","command_uuid":"cd3eda46-298e-48ba-881f-e90649d1f690","state":"started","uuid":"05a09b7c-a67c-4abf-b43e-35e5a50ea970","session_id":"a762f910-ec07-4eec-b25f-2197e96b4cd6"}"#;
        let completed = r#"{"type":"command_lifecycle","command_uuid":"cd3eda46-298e-48ba-881f-e90649d1f690","state":"completed","uuid":"815094ae-9e6b-45e6-9be8-a4e663cc7d94","session_id":"a762f910-ec07-4eec-b25f-2197e96b4cd6"}"#;
        let parsed: Vec<(String, String)> = [started, completed]
            .iter()
            .map(|line| match parse_line(line) {
                ParsedLine::CommandLifecycle {
                    command_uuid,
                    state,
                } => (command_uuid, state),
                other => panic!("expected CommandLifecycle, got {other:?}"),
            })
            .collect();
        assert_eq!(parsed[0].0, "cd3eda46-298e-48ba-881f-e90649d1f690");
        assert_eq!(parsed[0].0, parsed[1].0, "the pair shares one command_uuid");
        assert_eq!(parsed[0].1, "started");
        assert_eq!(parsed[1].1, "completed");
    }

    #[test]
    fn command_lifecycle_cancelled_passes_through() {
        let line =
            r#"{"type":"command_lifecycle","command_uuid":"c1","state":"cancelled","uuid":"u1"}"#;
        assert!(
            matches!(parse_line(line), ParsedLine::CommandLifecycle { state, .. } if state == "cancelled")
        );
    }

    #[test]
    fn command_lifecycle_without_command_uuid_falls_back_to_system() {
        let line = r#"{"type":"command_lifecycle","state":"started","uuid":"u1"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "command_lifecycle"
        ));
    }

    #[test]
    fn tool_progress_heartbeat_is_system() {
        let line = r#"{"type":"tool_progress","tool_use_id":"toolu_018Z7VFXLKyjbchmrLXh2zN4-heartbeat-0","tool_name":"Bash","parent_tool_use_id":"toolu_018Z7VFXLKyjbchmrLXh2zN4","elapsed_time_seconds":30,"heartbeat":true,"session_id":"a762f910-ec07-4eec-b25f-2197e96b4cd6","uuid":"4b112374-593d-4406-aa8b-cba5534f0813"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "tool_progress"
        ));
    }

    #[test]
    fn tool_progress_without_heartbeat_flag_is_still_system() {
        let line = r#"{"type":"tool_progress","tool_use_id":"toolu_01A","tool_name":"Bash","subagent_retry":{"attempt":1},"elapsed_time_seconds":90,"uuid":"u1"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "tool_progress"
        ));
    }

    #[test]
    fn control_cancel_request_is_system() {
        let line = r#"{"type":"control_cancel_request","request_id":"365bf5ed-5f59-4b2e-8b6b-c0a731c086b0"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "control_cancel_request"
        ));
    }

    #[test]
    fn replay_user_echo_carries_uuid_and_flag() {
        let line = r#"{"type":"user","isReplay":true,"uuid":"d2052f84-2034-4399-81e7-bd28d28bc691","message":{"role":"user","content":"Reply READY"},"parent_tool_use_id":null,"session_id":"s"}"#;
        match parse_line(line) {
            ParsedLine::User {
                uuid, is_replay, ..
            } => {
                assert!(is_replay);
                assert_eq!(
                    uuid.as_deref(),
                    Some("d2052f84-2034-4399-81e7-bd28d28bc691")
                );
            }
            other => panic!("expected a User line, got {other:?}"),
        }
    }

    #[test]
    fn injected_skill_body_is_synthetic() {
        let line = r#"{"type":"user","parent_tool_use_id":null,"uuid":"u1","isSynthetic":true,"message":{"role":"user","content":[{"type":"text","text":"Approach this as the design lead"}]},"session_id":"s"}"#;
        match parse_line(line) {
            ParsedLine::User { is_synthetic, .. } => assert!(is_synthetic),
            other => panic!("expected a User line, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_user_line_is_not_a_replay() {
        let line = r#"{"type":"user","uuid":"u1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]},"session_id":"s"}"#;
        match parse_line(line) {
            ParsedLine::User { is_replay, .. } => assert!(!is_replay),
            other => panic!("expected a User line, got {other:?}"),
        }
    }

    #[test]
    fn control_response_routes_by_request_id() {
        let line = r#"{"type":"control_response","response":{"subtype":"success","request_id":"app-s-3","response":{"rewound":true,"prefillText":"Reply THIRD","precedingAssistantUuid":"a1"}}}"#;
        match parse_line(line) {
            ParsedLine::ControlResponse {
                request_id,
                payload,
            } => {
                assert_eq!(request_id, "app-s-3");
                assert_eq!(payload.get("rewound").and_then(Value::as_bool), Some(true));
                assert_eq!(
                    payload.get("prefillText").and_then(Value::as_str),
                    Some("Reply THIRD")
                );
            }
            other => panic!("expected ControlResponse, got {other:?}"),
        }
    }

    #[test]
    fn control_response_surfaces_refusal_error() {
        let line = r#"{"type":"control_response","response":{"subtype":"success","request_id":"app-s-4","response":{"rewound":false,"prefillText":null,"error":"stale target"}}}"#;
        match parse_line(line) {
            ParsedLine::ControlResponse { payload, .. } => {
                assert_eq!(payload.get("rewound").and_then(Value::as_bool), Some(false));
                assert_eq!(
                    payload.get("error").and_then(Value::as_str),
                    Some("stale target")
                );
            }
            other => panic!("expected ControlResponse, got {other:?}"),
        }
    }
}
