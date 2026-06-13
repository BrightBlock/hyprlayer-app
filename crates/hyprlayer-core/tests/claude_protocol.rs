//! Protocol parsing against a recorded claude 2.1.173 `stream-json` transcript.
//! Locks the observed payload shapes so a future claude release that changes
//! them fails loudly here instead of silently in the driver.

use hyprlayer_core::claude::protocol::{parse_line, Block, ParsedLine};

const TRANSCRIPT: &str = include_str!("fixtures/claude_stream_basic.ndjson");

#[test]
fn parses_a_real_transcript_into_known_events() {
    let mut saw_init = false;
    let mut saw_assistant_text = false;
    let mut saw_result = false;
    let mut unknown = 0;

    for line in TRANSCRIPT.lines().filter(|l| !l.trim().is_empty()) {
        match parse_line(line) {
            ParsedLine::Init {
                claude_session_id,
                model,
                ..
            } => {
                saw_init = true;
                assert!(claude_session_id.is_some(), "init carries a session id");
                // init reports the resolved model, possibly with a variant
                // suffix (e.g. "claude-opus-4-8[1m]").
                assert!(
                    model.as_deref().unwrap_or("").starts_with("claude-"),
                    "init carries a model id, got {model:?}"
                );
            }
            ParsedLine::Assistant { blocks } => {
                for b in blocks {
                    if let Block::Text { text } = b {
                        assert!(!text.is_empty());
                        saw_assistant_text = true;
                    }
                }
            }
            ParsedLine::Result {
                is_error,
                total_cost_usd,
                num_turns,
                ..
            } => {
                saw_result = true;
                assert!(!is_error);
                assert!(total_cost_usd.unwrap_or(0.0) > 0.0, "result carries cost");
                assert_eq!(num_turns, Some(1));
            }
            ParsedLine::Unknown { .. } => unknown += 1,
            _ => {}
        }
    }

    assert!(saw_init, "transcript has a system/init line");
    assert!(saw_assistant_text, "transcript has assistant text");
    assert!(saw_result, "transcript has a terminal result");
    // Every line in a real transcript should map to a known kind (rate-limit
    // and partial stream events included) — nothing should be Unknown.
    assert_eq!(
        unknown, 0,
        "every transcript line should parse to a known kind"
    );
}

#[test]
fn unparseable_lines_never_panic() {
    assert!(matches!(
        parse_line("not json at all"),
        ParsedLine::Unknown { .. }
    ));
    assert!(matches!(parse_line("{}"), ParsedLine::Unknown { .. }));
    assert!(matches!(
        parse_line(r#"{"type":"brand_new_event_kind","x":1}"#),
        ParsedLine::Unknown { .. }
    ));
}

#[test]
fn parses_a_permission_request() {
    let line = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}"#;
    match parse_line(line) {
        ParsedLine::CanUseTool {
            request_id,
            tool_name,
            input,
        } => {
            assert_eq!(request_id, "req-1");
            assert_eq!(tool_name, "Bash");
            assert_eq!(input["command"], "ls");
        }
        other => panic!("expected CanUseTool, got {other:?}"),
    }
}

#[test]
fn outbound_shapes_are_well_formed() {
    use hyprlayer_core::claude::protocol::{
        control_request_json, permission_response_json, user_turn_json,
    };
    use serde_json::Value;

    let turn: Value =
        serde_json::from_str(&user_turn_json("sid", "/research_codebase hi")).unwrap();
    assert_eq!(turn["type"], "user");
    assert_eq!(turn["message"]["role"], "user");
    assert_eq!(turn["message"]["content"], "/research_codebase hi");
    assert_eq!(turn["session_id"], "sid");

    let deny: Value =
        serde_json::from_str(&permission_response_json("r1", false, Some("nope"))).unwrap();
    assert_eq!(deny["type"], "control_response");
    assert_eq!(deny["response"]["request_id"], "r1");
    assert_eq!(deny["response"]["response"]["behavior"], "deny");
    assert_eq!(deny["response"]["response"]["message"], "nope");

    let allow: Value = serde_json::from_str(&permission_response_json("r2", true, None)).unwrap();
    assert_eq!(allow["response"]["response"]["behavior"], "allow");
    assert!(allow["response"]["response"].get("message").is_none());

    let interrupt: Value = serde_json::from_str(&control_request_json(
        "c1",
        "interrupt",
        serde_json::json!({}),
    ))
    .unwrap();
    assert_eq!(interrupt["type"], "control_request");
    assert_eq!(interrupt["request"]["subtype"], "interrupt");
}
