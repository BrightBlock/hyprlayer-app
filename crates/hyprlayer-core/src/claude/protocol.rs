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
        /// The message's own `uuid`, when present. Maps a human turn to claude's
        /// internal id, which `rewind_conversation` targets.
        uuid: Option<String>,
        /// `isReplay:true` — a replay echo of a prompt we sent on stdin (emitted
        /// under `--replay-user-messages`). Not new transcript content, so the reader
        /// suppresses it; it never renders as assistant text.
        is_replay: bool,
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
    /// A background shell task (spawned by `Bash{run_in_background:true}`) reached
    /// a terminal state. claude emits this ASYNCHRONOUSLY as the task finishes,
    /// even when the agent never polled it (the common fire-and-forget case) —
    /// realizing the spawn result's promise that "you will be notified when it
    /// completes". It's the authoritative "this background shell is done" signal
    /// the UI needs to drop a finished task from its live indicator. `status` is
    /// "completed" or "failed" at terminal; `tool_use_id` is the spawning `Bash`
    /// call (the UI's direct transcript key), `task_id` the background shell id.
    ///
    /// Two wire shapes, both handled: the older flat
    /// `{"type":"system","subtype":"task_notification",...}` line, and (claude
    /// 2.1.195+) a `{"type":"queue-operation",...,"content":"<task-notification>…"}`
    /// line carrying the same fields inside a `<task-notification>` XML block.
    TaskNotification {
        task_id: String,
        tool_use_id: Option<String>,
        status: String,
    },
    /// A reply to one of our own control requests (`interrupt`,
    /// `rewind_conversation`, …). We act only on the ones we're awaiting (keyed by
    /// `request_id`); the rest — e.g. interrupt acks — are dropped. `payload` is
    /// the innermost result object (for rewind: `{rewound, prefillText, error?}`).
    ControlResponse {
        request_id: String,
        payload: Value,
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
        // claude 2.1.195+ delivers the background-task completion notification as
        // a `queue-operation` line carrying a `<task-notification>` XML block,
        // instead of the flat `system`/`task_notification` line older builds used
        // (handled above). Same payload, different envelope — see
        // `parse_queue_operation`.
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
        // A reply to one of our control requests. `rewind_conversation` awaits its
        // reply (keyed by request_id); other acks (interrupt) route to no waiter
        // and are dropped. Not transcript content, so never a transcript event.
        Some("control_response") => parse_control_response(&v),
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

/// A `{"type":"queue-operation",…,"content":"<task-notification>…"}` line.
/// claude 2.1.195 delivers the async background-task completion notification this
/// way — a queued out-of-band event whose `content` holds a flat
/// `<task-notification>` XML block — rather than the `system`/`task_notification`
/// JSON line earlier builds emitted. When that block is present we pull the same
/// fields and emit `TaskNotification` (so the UI drops the finished task); any
/// other queue-operation is a known-but-inert line, kept as informational
/// `System` rather than a scary `Unknown`/protocol error.
fn parse_queue_operation(v: &Value) -> ParsedLine {
    let content = v.get("content").and_then(Value::as_str).unwrap_or("");
    if content.contains("<task-notification>") {
        if let Some(task_id) = xml_tag(content, "task-id") {
            return ParsedLine::TaskNotification {
                task_id: task_id.to_string(),
                tool_use_id: xml_tag(content, "tool-use-id").map(str::to_string),
                status: xml_tag(content, "status").map(str::to_string).unwrap_or_default(),
            };
        }
    }
    ParsedLine::System {
        subtype: "queue-operation".into(),
    }
}

/// Text of the first `<tag>…</tag>` in `s`, trimmed. The task-notification block
/// is flat (no nesting or attributes), so a plain find/slice is enough.
fn xml_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(s[start..end].trim())
}

/// A `{"type":"control_response","response":{"subtype":..,"request_id":..,
/// "response":{..}}}` reply. We key on the inner `request_id` and surface the
/// innermost result object as `payload` (for `rewind_conversation`:
/// `{rewound, prefillText, precedingAssistantUuid, error?}`). A reply we can't
/// key (no request_id) degrades to an ignored `System` line.
fn parse_control_response(v: &Value) -> ParsedLine {
    let resp = v.get("response");
    match resp.and_then(|r| r.get("request_id")).and_then(Value::as_str) {
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

    /// claude 2.1.195 delivers the completion notification as a `queue-operation`
    /// whose `content` is a `<task-notification>` XML block (verified live against
    /// session 88c21580). It must parse to the SAME `TaskNotification` the flat
    /// `system` shape did, or finished background tasks linger in the live
    /// indicator forever. Line shape is verbatim from a real session.
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
                assert_eq!(tool_use_id.as_deref(), Some("toolu_01KXEEjFzSxWBxnYuvAz9pdG"));
                assert_eq!(status, "completed");
            }
            other => panic!("expected TaskNotification, got {other:?}"),
        }
    }

    /// A `failed` background command over the queue-operation envelope reports the
    /// terminal status the same way — the UI still drops it.
    #[test]
    fn task_notification_via_queue_operation_failed_parses() {
        let line = r#"{"type":"queue-operation","content":"<task-notification>\n<task-id>bzx</task-id>\n<tool-use-id>toolu_9</tool-use-id>\n<status>failed</status>\n</task-notification>"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::TaskNotification { status, .. } if status == "failed"
        ));
    }

    /// A queue-operation that isn't a task notification (e.g. a queued user
    /// message) is a known-but-inert line — informational `System`, not a scary
    /// `Unknown`/protocol error.
    #[test]
    fn non_task_queue_operation_is_system() {
        let line = r#"{"type":"queue-operation","operation":"add","content":"some queued prompt"}"#;
        assert!(matches!(
            parse_line(line),
            ParsedLine::System { subtype } if subtype == "queue-operation"
        ));
    }

    /// A `--replay-user-messages` echo carries `isReplay:true` and the turn's own
    /// `uuid` — the id the driver captures to target a later `rewind_conversation`.
    #[test]
    fn replay_user_echo_carries_uuid_and_flag() {
        let line = r#"{"type":"user","isReplay":true,"uuid":"d2052f84-2034-4399-81e7-bd28d28bc691","message":{"role":"user","content":"Reply READY"},"parent_tool_use_id":null,"session_id":"s"}"#;
        match parse_line(line) {
            ParsedLine::User {
                uuid, is_replay, ..
            } => {
                assert!(is_replay);
                assert_eq!(uuid.as_deref(), Some("d2052f84-2034-4399-81e7-bd28d28bc691"));
            }
            other => panic!("expected a User line, got {other:?}"),
        }
    }

    /// A normal agent `user` line (tool results) is NOT a replay echo, so the
    /// driver leaves it alone (it still emits tool-result events).
    #[test]
    fn tool_result_user_line_is_not_a_replay() {
        let line = r#"{"type":"user","uuid":"u1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]},"session_id":"s"}"#;
        match parse_line(line) {
            ParsedLine::User { is_replay, .. } => assert!(!is_replay),
            other => panic!("expected a User line, got {other:?}"),
        }
    }

    /// A `control_response` to a request we're awaiting parses to `ControlResponse`
    /// keyed by the inner `request_id`, surfacing the innermost result as payload
    /// (rewind's `{rewound, prefillText, …}`).
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

    /// A logical refusal ("stale target") still arrives as outer `subtype:success`
    /// with the reason in the inner `error` — the driver reads `rewound:false`.
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
