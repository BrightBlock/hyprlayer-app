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
                assert_eq!(context_tokens, Some(24130), "context fill from usage");
            }
            ParsedLine::Unknown { .. } => unknown += 1,
            _ => {}
        }
    }

    assert!(saw_init, "transcript has a system/init line");
    assert!(saw_assistant_text, "transcript has assistant text");
    assert!(saw_result, "transcript has a terminal result");
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
    let sub = r#"{"type":"assistant","parent_tool_use_id":"toolu_task1","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
    match parse_line(sub) {
        ParsedLine::Assistant { parent_id, .. } => {
            assert_eq!(parent_id.as_deref(), Some("toolu_task1"));
        }
        other => panic!("expected Assistant, got {other:?}"),
    }
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
fn malformed_permission_request_is_unknown_not_an_empty_card() {
    let no_id =
        r#"{"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#;
    let no_tool =
        r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool"}}"#;
    for line in [no_id, no_tool] {
        assert!(
            matches!(parse_line(line), ParsedLine::Unknown { .. }),
            "expected Unknown for malformed line: {line}"
        );
    }
    let no_input = r#"{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"TodoWrite"}}"#;
    match parse_line(no_input) {
        ParsedLine::CanUseTool { input, .. } => assert!(input.is_null()),
        other => panic!("expected CanUseTool, got {other:?}"),
    }
}

#[test]
fn parses_compact_boundary_with_metadata() {
    let auto = r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"auto","pre_tokens":182431},"session_id":"s1"}"#;
    match parse_line(auto) {
        ParsedLine::CompactBoundary {
            trigger,
            pre_tokens,
        } => {
            assert_eq!(trigger, "auto");
            assert_eq!(pre_tokens, Some(182_431));
        }
        other => panic!("expected CompactBoundary, got {other:?}"),
    }

    let manual =
        r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual"}}"#;
    match parse_line(manual) {
        ParsedLine::CompactBoundary {
            trigger,
            pre_tokens,
        } => {
            assert_eq!(trigger, "manual");
            assert_eq!(pre_tokens, None);
        }
        other => panic!("expected CompactBoundary, got {other:?}"),
    }

    let bare = r#"{"type":"system","subtype":"compact_boundary"}"#;
    match parse_line(bare) {
        ParsedLine::CompactBoundary {
            trigger,
            pre_tokens,
        } => {
            assert_eq!(trigger, "auto");
            assert_eq!(pre_tokens, None);
        }
        other => panic!("expected CompactBoundary, got {other:?}"),
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

    let allow: Value = serde_json::from_str(&permission_response_json(
        "r2",
        true,
        Some(serde_json::json!({ "command": "ls" })),
        None,
    ))
    .unwrap();
    assert_eq!(allow["response"]["response"]["behavior"], "allow");
    assert_eq!(
        allow["response"]["response"]["updatedInput"]["command"],
        "ls"
    );
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
