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
            ParsedLine::Assistant { blocks, .. } => {
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
                context_tokens,
                ..
            } => {
                saw_result = true;
                assert!(!is_error);
                assert!(total_cost_usd.unwrap_or(0.0) > 0.0, "result carries cost");
                assert_eq!(num_turns, Some(1));
                // input_tokens (3890) + cache_creation (20240) + cache_read (0).
                assert_eq!(context_tokens, Some(24130), "context fill from usage");
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
fn context_fill_is_the_last_iteration_not_the_summed_total() {
    // A multi-step turn: top-level usage SUMS every iteration (cache_read
    // balloons to ~907k); the real window fill is the last iteration (114k).
    let line = r#"{"type":"result","is_error":false,"num_turns":1,"total_cost_usd":1.0,"usage":{"input_tokens":5000,"cache_read_input_tokens":900000,"cache_creation_input_tokens":2000,"iterations":[{"input_tokens":2000,"cache_read_input_tokens":0,"cache_creation_input_tokens":20000},{"input_tokens":1000,"cache_read_input_tokens":110000,"cache_creation_input_tokens":3000}]}}"#;
    match parse_line(line) {
        ParsedLine::Result { context_tokens, .. } => {
            assert_eq!(context_tokens, Some(114_000), "last iteration, not the sum");
        }
        other => panic!("expected Result, got {other:?}"),
    }
}

#[test]
fn carries_parent_tool_use_id_for_subagents() {
    // A subagent's assistant line is tagged with the spawning Task's tool-use id.
    let sub = r#"{"type":"assistant","parent_tool_use_id":"toolu_task1","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
    match parse_line(sub) {
        ParsedLine::Assistant { parent_id, .. } => {
            assert_eq!(parent_id.as_deref(), Some("toolu_task1"));
        }
        other => panic!("expected Assistant, got {other:?}"),
    }
    // A main-thread line has none.
    let main = r#"{"type":"assistant","parent_tool_use_id":null,"message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
    match parse_line(main) {
        ParsedLine::Assistant { parent_id, .. } => assert_eq!(parent_id, None),
        other => panic!("expected Assistant, got {other:?}"),
    }
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
        serde_json::from_str(&permission_response_json("r1", false, None, Some("nope"))).unwrap();
    assert_eq!(deny["type"], "control_response");
    assert_eq!(deny["response"]["request_id"], "r1");
    assert_eq!(deny["response"]["response"]["behavior"], "deny");
    assert_eq!(deny["response"]["response"]["message"], "nope");

    // Allow echoes the tool input back as updatedInput (required by claude).
    let allow: Value = serde_json::from_str(&permission_response_json(
        "r2",
        true,
        Some(serde_json::json!({ "command": "ls" })),
        None,
    ))
    .unwrap();
    assert_eq!(allow["response"]["response"]["behavior"], "allow");
    assert_eq!(allow["response"]["response"]["updatedInput"]["command"], "ls");
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
