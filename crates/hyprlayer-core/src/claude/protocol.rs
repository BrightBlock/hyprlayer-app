//! The claude 2.1.x headless `stream-json` wire protocol.
//!
//! Shapes verified against the installed `claude` binary (2.1.173). Parsing is
//! deliberately tolerant: every stdout line is read as a `serde_json::Value`
//! first, then dispatched on its `type` field, so an unrecognized event kind
//! degrades to [`ParsedLine::Unknown`] (→ a `ProtocolError` session event) and
//! never crashes the reader. This is the single most important property for
//! surviving a future claude release that adds event kinds.

use serde_json::{json, Value};

/// One parsed line of claude stdout. The variants cover only what the driver
/// acts on; everything else is `Unknown` (kept raw for diagnostics).
#[derive(Debug, Clone)]
pub enum ParsedLine {
    /// `{"type":"system","subtype":"init",...}` — session is up; carries the
    /// claude-assigned session id, model, and tool/skill inventory.
    Init {
        claude_session_id: Option<String>,
        model: Option<String>,
        tools: Vec<String>,
    },
    /// Other `system` events (status heartbeats, etc.) — informational.
    System { subtype: String },
    /// A completed assistant turn (`type:"assistant"`), already de-chunked.
    /// `context_tokens` is the per-message context fill (for the live meter —
    /// it updates each step of a multi-tool turn, not just at the final result).
    /// `parent_id` is the `Task` tool-use id when this is a subagent's message.
    Assistant {
        blocks: Vec<Block>,
        context_tokens: Option<u32>,
        parent_id: Option<String>,
    },
    /// A user turn echoed back — in agent runs this carries tool results.
    User {
        blocks: Vec<Block>,
        parent_id: Option<String>,
    },
    /// A partial-message chunk (`type:"stream_event"`); only text deltas are
    /// surfaced, for live streaming. Everything else here is ignored (the
    /// completed `assistant` turn is the source of truth).
    PartialText {
        text: String,
        parent_id: Option<String>,
    },
    /// A permission prompt: claude asks whether a tool may run.
    CanUseTool {
        request_id: String,
        tool_name: String,
        input: Value,
    },
    /// Terminal `{"type":"result",...}` — carries cumulative cost/usage.
    Result {
        is_error: bool,
        total_cost_usd: Option<f64>,
        num_turns: Option<u32>,
        stop_reason: Option<String>,
        /// Input tokens in context for the final turn (incl. cache) — the
        /// "context window fill" the UI's context meter shows.
        context_tokens: Option<u32>,
    },
    /// `{"type":"system","subtype":"compact_boundary",...}` — claude auto- or
    /// manually compacted the conversation; the next assistant turn runs on the
    /// post-compact summary. Surfaced so a silent compaction stretch is visible
    /// in the transcript instead of looking like a stuck turn.
    CompactBoundary {
        /// "auto" (claude triggered it) or "manual" (a `/compact` slash command).
        /// Unrecognized values pass through verbatim.
        trigger: String,
        /// Token count just before the compact ran, when reported.
        pre_tokens: Option<u32>,
    },
    /// `{"type":"system","subtype":"task_notification",...}` — a background shell
    /// task (spawned by `Bash{run_in_background:true}`) reached a terminal state.
    /// claude emits this ASYNCHRONOUSLY as the task finishes, even when the agent
    /// never polled it (the common fire-and-forget case) — realizing the spawn
    /// result's promise that "you will be notified when it completes". It's the
    /// authoritative "this background shell is done" signal the UI needs to drop a
    /// finished task from its live indicator. `status` is "completed" or "failed"
    /// at terminal; `tool_use_id` is the spawning `Bash` call (the UI's direct
    /// transcript key), `task_id` the background shell id.
    TaskNotification {
        task_id: String,
        tool_use_id: Option<String>,
        status: String,
    },
    /// A line we don't model — kept verbatim so the driver can log it.
    Unknown { raw: String },
}

/// A content block inside an assistant or user message.
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
    /// A block kind we don't model (kept so callers can render generically).
    Other {
        kind: String,
    },
}

/// Parse one stdout line. Never fails: a malformed or unrecognized line maps to
/// [`ParsedLine::Unknown`].
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
                // The background-task lifecycle notification: claude fires it on
                // its own when a `Bash{run_in_background}` shell finishes (see the
                // variant doc). We key on `task_id`; without one there's nothing
                // actionable, so it stays an informational `System` line.
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
        Some("assistant") => ParsedLine::Assistant {
            blocks: parse_blocks(v.get("message")),
            context_tokens: context_tokens(v.get("message").and_then(|m| m.get("usage"))),
            parent_id: str_field(&v, "parent_tool_use_id"),
        },
        Some("user") => ParsedLine::User {
            blocks: parse_blocks(v.get("message")),
            parent_id: str_field(&v, "parent_tool_use_id"),
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
        // Our own control requests (interrupt / set_permission_mode) get acked
        // with a `control_response`. It's bookkeeping, not transcript content —
        // ignore it rather than surfacing a scary "unrecognized line" warning.
        Some("control_response") => ParsedLine::System {
            subtype: "control_response".into(),
        },
        // Known-but-ignored informational lines (don't surface as errors).
        Some("rate_limit_event") => ParsedLine::System {
            subtype: "rate_limit".into(),
        },
        // Any genuinely unrecognized kind: keep raw for the log.
        _ => ParsedLine::Unknown {
            raw: trimmed.to_string(),
        },
    }
}

fn parse_stream_event(v: &Value) -> ParsedLine {
    // Only a `content_block_delta` carrying a `text_delta` matters for live
    // streaming; anything else is an ignored heartbeat. Walk the shape with
    // combinators and bail to `System` the moment a step doesn't match.
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

fn parse_control_request(v: &Value) -> ParsedLine {
    // Only `can_use_tool` is modelled; anything else — or a malformed line
    // missing the `request_id`/`tool_name` we'd need to reply — is kept raw so
    // it surfaces as a protocol error rather than an unanswerable permission card.
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

/// A tool_result's `content` is either a string or an array of text blocks.
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

/// Context-window fill from a `usage` object: the input tokens in context for
/// the request (fresh input + both cache tiers; output is generation, excluded).
///
/// A turn's top-level `usage` SUMS every internal iteration (each tool step
/// re-reads the cached context), so summing it over a long turn balloons far
/// past the real window. The actual fill is the LAST iteration's context, so we
/// read `usage.iterations[last]` when present (the `result` event), else the
/// object itself (a single assistant message). None if there's no usage.
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

/* --------------------------- Outbound (stdin) ---------------------------- */

/// A user turn, sent over stdin as one NDJSON line. The first prompt and every
/// subsequent turn use this shape.
pub fn user_turn_json(session_id: &str, text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "parent_tool_use_id": null,
        "session_id": session_id,
    })
    .to_string()
}

/// Reply to a `can_use_tool` control request. claude's permission-result schema
/// requires `updatedInput` on an allow (the tool input to run, which we echo
/// back unchanged) and a `message` on a deny — omitting either is a ZodError
/// that fails the tool. `updated_input` falls back to `{}` only if we somehow
/// lost the cached input.
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

/// An app-initiated control request (interrupt / set_permission_mode).
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

    /// A background-task completion notification (verified against claude 2.1.x:
    /// fire `sleep N && …` with `run_in_background`, don't poll, and this lands
    /// asynchronously the moment the shell exits) parses to `TaskNotification`
    /// carrying the shell id, the spawning Bash tool-use id, and the status.
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

    /// A failed background command reports `status:"failed"` (same shape) — the
    /// UI drops it from the live set just the same (it's no longer running).
    #[test]
    fn task_notification_failed_parses() {
        let line = r#"{"type":"system","subtype":"task_notification","task_id":"bzxqr580k","tool_use_id":"toolu_01W","status":"failed","summary":"… failed with exit code 7","session_id":"s"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::TaskNotification { status, .. } if status == "failed"
        ));
    }

    /// Defensive: a malformed notification missing `task_id` has nothing to key
    /// on, so it degrades to an informational `System` line rather than emitting
    /// an unactionable task event.
    #[test]
    fn task_notification_without_task_id_falls_back_to_system() {
        let line = r#"{"type":"system","subtype":"task_notification","status":"completed"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "task_notification"
        ));
    }

    /// The sibling lifecycle lines (`task_started` / `task_updated`) are not
    /// modelled — the terminal `task_notification` is the single signal we act
    /// on — so they stay generic `System` lines and drive nothing.
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
}
