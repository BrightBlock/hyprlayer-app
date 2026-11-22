use hyprlayer_app_lib::commands::sessions::SessionLogEntryDto;
use hyprlayer_core::agent::Harness;
use hyprlayer_core::models::session::{SessionEvent, SessionLogEntry};
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::store::session_event_codec::{decode_raw_event, encode_raw_event};
use hyprlayer_core::store::{Db, NewSession, NewWorkstream};
use hyprlayer_core::store::{StoredEvent, WorktreeMode};
use serde::Deserialize;
use serde_json::Value;

const SESSION_ID: &str = "s-legacy";

const V0_7_SESSION_EVENTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../crates/hyprlayer-core/tests/fixtures/compat/v0_7/session-events.jsonl"
);

// The shape real v0.7 rows carry, verified against a populated database: `questions`
// is a JSON *string* holding objects keyed header/multiSelect/options/question.
const REALISTIC_LEGACY_QUESTION: &str = r#"{"kind":"questionAsk","requestId":"call_abc","questions":"[{\"header\":\"Primary pain\",\"multiSelect\":false,\"options\":[{\"label\":\"Streaming latency\",\"description\":\"Keep token streaming responsive.\"},{\"label\":\"Startup time\",\"description\":\"Open a transcript faster.\"}],\"question\":\"Which pain matters most?\"}]"}"#;

#[derive(Debug, Deserialize)]
struct FixtureRow {
    seq: f64,
    kind: String,
    payload: String,
    #[serde(rename = "createdAt")]
    created_at: f64,
}

fn fixture_rows() -> Vec<FixtureRow> {
    let raw = std::fs::read_to_string(V0_7_SESSION_EVENTS)
        .expect("the core crate's v0.7 session-event fixture is readable");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture row is valid JSON"))
        .collect()
}

fn seeded_db() -> Db {
    let db = Db::open_in_memory().expect("in-memory database");
    db.insert_workstream(NewWorkstream {
        project_path: "/p".into(),
        title: "legacy".into(),
        slug: "legacy".into(),
        branch: "hyprlayer/legacy".into(),
        worktree_path: "/wt".into(),
        base_branch: "main".into(),
        worktree_mode: WorktreeMode::Isolated,
    })
    .expect("insert workstream");
    let ws = db
        .list_active_workstreams()
        .expect("list workstreams")
        .pop()
        .expect("one workstream");
    db.insert_session(NewSession {
        id: SESSION_ID.into(),
        workstream_id: ws.id,
        stage: Stage::General,
        harness: Harness::Claude,
        cwd: "/wt".into(),
        first_input: None,
    })
    .expect("insert session");

    let mut rows: Vec<StoredEvent> = fixture_rows()
        .into_iter()
        .map(|row| StoredEvent {
            seq: row.seq,
            kind: row.kind,
            payload: row.payload,
            created_at: row.created_at,
        })
        .collect();
    rows.push(StoredEvent {
        seq: rows.len() as f64,
        kind: "questionAsk".into(),
        payload: REALISTIC_LEGACY_QUESTION.into(),
        created_at: 1_700_000_002_300.0,
    });
    db.restore_session_events(SESSION_ID, &rows)
        .expect("seed the legacy rows verbatim");
    db
}

fn dto_log(db: &Db) -> Vec<SessionLogEntryDto> {
    db.session_log(SESSION_ID)
        .expect("read the session log")
        .into_iter()
        .map(SessionLogEntryDto::from)
        .collect()
}

#[test]
fn a_legacy_row_reaches_the_binding_shape_with_typed_questions() {
    let log = dto_log(&seeded_db());
    let realistic = log.last().expect("the log is not empty");
    let wire: Value = serde_json::to_value(realistic).expect("the DTO serializes");

    assert_eq!(
        wire.as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>()),
        Some(vec![
            "createdAt".to_string(),
            "event".to_string(),
            "kind".to_string(),
            "seq".to_string(),
        ]),
        "SessionLogEntryDto drifted from its generated binding shape"
    );
    assert_eq!(wire["seq"], (fixture_rows().len()) as f64);
    assert_eq!(wire["createdAt"], 1_700_000_002_300.0);
    assert_eq!(wire["kind"], "questionAsk");
    assert_eq!(wire["event"]["kind"], "questionAsk");
    assert_eq!(wire["event"]["requestId"], "call_abc");

    let questions = wire["event"]["questions"]
        .as_array()
        .expect("questions reach the frontend as a typed array");
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0]["question"], "Which pain matters most?");
    assert_eq!(questions[0]["header"], "Primary pain");
    assert_eq!(questions[0]["multiSelect"], false);
    assert_eq!(questions[0]["options"][1]["label"], "Startup time");
    assert_eq!(
        questions[0]["options"][1]["description"],
        "Open a transcript faster."
    );
}

#[test]
fn the_log_keeps_every_stored_row_except_the_one_naming_no_known_variant() {
    let db = seeded_db();
    let stored = db.session_events(SESSION_ID).expect("raw rows");
    let log = dto_log(&db);

    assert_eq!(log.len(), stored.len() - 1);
    assert!(
        !log.iter()
            .any(|entry| entry.kind == "toolPermissionEscalated"),
        "a future kind should be skipped, not surfaced"
    );
    assert!(
        stored
            .iter()
            .any(|row| row.kind == "toolPermissionEscalated"),
        "the skipped row is still on disk for a rollback to read"
    );
}

#[test]
fn missing_and_malformed_question_fields_survive_as_empty_typed_lists() {
    let log = dto_log(&seeded_db());
    let malformed: Vec<_> = log
        .iter()
        .filter(|entry| matches!(&entry.event, SessionEvent::QuestionAsk { request_id, .. } if request_id.starts_with("q-")))
        .collect();

    assert_eq!(malformed.len(), 4, "the fixture's four question probes");
    for entry in malformed {
        match &entry.event {
            SessionEvent::QuestionAsk {
                request_id,
                questions,
                ..
            } => assert!(
                questions.is_empty(),
                "{request_id} should normalize to an empty question list"
            ),
            other => panic!("expected QuestionAsk, got {other:?}"),
        }
    }
}

#[test]
fn a_legacy_row_preserves_its_stored_kind_even_when_the_event_disagrees() {
    let entry = SessionLogEntryDto::from(
        SessionLogEntry::try_from(StoredEvent {
            seq: 7.0,
            kind: "legacyAlias".into(),
            payload: r#"{"kind":"relaunched"}"#.into(),
            created_at: 1_700_000_000_000.0,
        })
        .expect("a known variant under a legacy kind still decodes"),
    );

    assert_eq!(entry.kind, "legacyAlias");
    assert_eq!(entry.event.kind(), "relaunched");
}

#[test]
fn a_round_trip_through_the_codec_leaves_questions_string_encoded_on_disk() {
    let event = decode_raw_event(REALISTIC_LEGACY_QUESTION).expect("legacy row decodes");
    let re_encoded = encode_raw_event(&event).expect("event encodes");
    let raw: Value = serde_json::from_str(&re_encoded).expect("the encoded row is JSON");

    assert!(
        raw["questions"].is_string(),
        "a rollback to a pre-typed-question build must still read this row"
    );
    assert_eq!(decode_raw_event(&re_encoded).unwrap(), event);
}

#[test]
fn the_init_row_keeps_its_legacy_wire_key() {
    let log = dto_log(&seeded_db());
    let init = log.first().expect("the log starts with init");
    let wire = serde_json::to_value(init).expect("the DTO serializes");

    assert_eq!(wire["event"]["claudeSessionId"], "claude-legacy-abc123");
    assert!(wire["event"].get("providerSessionId").is_none());
}
