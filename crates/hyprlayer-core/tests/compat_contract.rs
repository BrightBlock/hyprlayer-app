use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hyprlayer_core::claude::SessionEvent;
use hyprlayer_core::settings::SettingsStore;
use hyprlayer_core::store::session_event_codec::decode_raw_event;
use hyprlayer_models::session::Question;
use serde::Deserialize;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compat/v0_7")
}

#[derive(Debug, Deserialize)]
struct StoredEventRow {
    seq: f64,
    kind: String,
    payload: String,
}

fn load_rows() -> Vec<StoredEventRow> {
    let raw = std::fs::read_to_string(fixture_dir().join("session-events.jsonl"))
        .expect("session-events.jsonl fixture is readable");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture row is valid JSON"))
        .collect()
}

#[derive(Clone, Copy)]
enum Decode {
    Ok(&'static str),
    Err,
}

fn expected_decodes() -> Vec<(i64, Decode)> {
    use Decode::{Err as Fails, Ok as Decodes};
    vec![
        (0, Decodes("init")),
        (1, Decodes("userMessage")),
        (2, Decodes("partialText")),
        (3, Decodes("assistantText")),
        (4, Decodes("thinking")),
        (5, Decodes("toolUse")),
        (6, Decodes("toolResult")),
        (7, Decodes("permissionAsk")),
        (8, Decodes("questionAsk")),
        (9, Decodes("usage")),
        (10, Decodes("contextWindow")),
        (11, Decodes("turnDone")),
        (12, Decodes("compactBoundary")),
        (13, Decodes("commandLifecycle")),
        (14, Decodes("backgroundTaskUpdate")),
        (15, Decodes("relaunched")),
        (16, Decodes("rewound")),
        (17, Decodes("protocolError")),
        (18, Decodes("exited")),
        (19, Decodes("questionAsk")),
        (20, Decodes("questionAsk")),
        (21, Decodes("questionAsk")),
        (22, Fails),
        (23, Decodes("questionAsk")),
    ]
}

#[test]
fn the_codec_decodes_every_pinned_v0_7_row_except_the_unknown_kind() {
    let rows = load_rows();
    let expected = expected_decodes();
    assert_eq!(
        rows.len(),
        expected.len(),
        "session-events.jsonl grew or shrank without updating expected_decodes()"
    );

    for (row, (seq, expectation)) in rows.iter().zip(expected) {
        assert_eq!(row.seq as i64, seq, "fixture row order drifted");
        let decoded = decode_raw_event(&row.payload);
        match expectation {
            Decode::Ok(kind) => {
                let event =
                    decoded.unwrap_or_else(|e| panic!("seq {seq} ({kind}) should decode: {e}"));
                assert_eq!(event.kind(), kind, "seq {seq}");
            }
            Decode::Err => {
                let err = decoded
                    .err()
                    .unwrap_or_else(|| panic!("seq {seq} names no known variant"));
                assert_eq!(err.code(), "session_event_decode", "seq {seq}");
            }
        }
    }
}

#[test]
fn init_row_carries_the_provider_session_id_under_its_legacy_wire_name() {
    let rows = load_rows();
    let row = &rows[0];
    assert!(row.payload.contains("\"claudeSessionId\""));
    match decode_raw_event(&row.payload).unwrap() {
        SessionEvent::Init {
            provider_session_id,
            model,
            mcp_servers,
        } => {
            assert_eq!(provider_session_id, "claude-legacy-abc123");
            assert_eq!(model.as_deref(), Some("claude-opus-4-6"));
            assert_eq!(mcp_servers, None);
        }
        other => panic!("expected Init, got {other:?}"),
    }
}

#[test]
fn every_stored_question_row_is_still_string_encoded_on_disk() {
    for row in load_rows().iter().filter(|row| row.kind == "questionAsk") {
        let raw: serde_json::Value = serde_json::from_str(&row.payload).unwrap();
        match raw.get("questions") {
            None => {}
            Some(questions) => assert!(
                questions.is_string(),
                "seq {} stopped being string encoded, which breaks the rollback contract",
                row.seq
            ),
        }
    }
}

// Sampled from the shape every one of 236 stored rows uses: a `questions` JSON string
// whose entries carry question/header/multiSelect/options[{label,description}].
#[test]
fn a_realistic_legacy_question_string_is_decoded_into_the_typed_list() {
    let questions = questions_of(&load_rows()[23].payload);
    assert_eq!(questions.len(), 2, "both stored questions survive");

    let first = &questions[0];
    assert_eq!(first.question, "Which branch should this work start from?");
    assert_eq!(first.header, "Branch base");
    assert!(!first.multi_select);
    assert_eq!(first.options.len(), 2);
    assert_eq!(first.options[0].label, "main");
    assert_eq!(
        first.options[0].description,
        "Branch from the default branch."
    );

    assert!(questions[1].multi_select, "multiSelect survives the decode");
    assert_eq!(questions[1].options.len(), 3);
}

// This row's entry is keyed `label`, not `question`, so it filters out — the same
// result the frontend's tolerant parser has always produced for it.
#[test]
fn a_question_entry_without_question_text_filters_out() {
    assert_eq!(questions_of(&load_rows()[8].payload).len(), 0);
}

#[test]
fn malformed_question_payloads_normalize_instead_of_dropping_the_row() {
    let rows = load_rows();
    for index in [19, 20, 21] {
        let row = &rows[index];
        assert!(
            questions_of(&row.payload).is_empty(),
            "seq {} should normalize to an empty question list",
            row.seq
        );
    }
}

fn questions_of(payload: &str) -> Vec<Question> {
    match decode_raw_event(payload).unwrap() {
        SessionEvent::QuestionAsk { questions, .. } => questions,
        other => panic!("expected QuestionAsk, got {other:?}"),
    }
}

#[tokio::test]
async fn settings_fixture_loads_through_the_settings_store() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture_dir().join("settings.json"),
        dir.path().join("settings.json"),
    )
    .expect("stage the settings fixture");

    let settings = SettingsStore::load(dir.path().to_path_buf()).get().await;

    assert_eq!(
        settings.worktree_root.as_deref(),
        Some("/Users/jt/hyprlayer/worktrees")
    );
    assert_eq!(settings.git.branch_prefix.as_deref(), Some("jt/"));
    assert_eq!(settings.prompts.custom.len(), 1);
    assert!(
        dir.path().join("settings.json").is_file(),
        "not moved aside"
    );
}

#[test]
fn generated_bindings_match_the_pinned_v0_7_excerpts() {
    let bindings_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/lib/bindings.ts");
    let bindings = std::fs::read_to_string(&bindings_path)
        .expect("src/lib/bindings.ts is readable from the workspace root");
    let raw = std::fs::read_to_string(fixture_dir().join("bindings-contract.json"))
        .expect("bindings-contract.json fixture is readable");
    let pinned: BTreeMap<String, String> =
        serde_json::from_str(&raw).expect("bindings-contract.json is valid JSON");

    for (name, expected) in pinned {
        assert!(
            bindings.contains(&expected),
            "generated binding for {name} drifted from the pinned v0.7 excerpt"
        );
    }
}
