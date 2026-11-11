use serde_json::Value;

use crate::error::{CoreError, Result};
use hyprlayer_models::session::{Question, QuestionOption, SessionEvent};

const QUESTION_ASK: &str = "questionAsk";
const QUESTIONS: &str = "questions";

pub fn decode_raw_event(payload: &str) -> Result<SessionEvent> {
    let mut value: Value = serde_json::from_str(payload).map_err(|e| decode_error(payload, e))?;
    if is_question_ask(&value) {
        let questions = questions_from_value(value.get(QUESTIONS));
        value[QUESTIONS] = serde_json::to_value(questions).unwrap_or_else(|_| Value::Array(vec![]));
    }
    serde_json::from_value(value).map_err(|e| decode_error(payload, e))
}

// Questions stay string-encoded on disk so a rollback to a pre-typed-question build
// still reads rows this one wrote.
pub fn encode_raw_event(event: &SessionEvent) -> Result<String> {
    let mut value = serde_json::to_value(event).map_err(|e| encode_error(event, e))?;
    if is_question_ask(&value) {
        value[QUESTIONS] = Value::String(value[QUESTIONS].to_string());
    }
    serde_json::to_string(&value).map_err(|e| encode_error(event, e))
}

pub fn questions_from_value(value: Option<&Value>) -> Vec<Question> {
    match value {
        Some(Value::Array(items)) => typed_questions(items),
        Some(Value::String(raw)) => match serde_json::from_str::<Value>(raw) {
            Ok(Value::Array(items)) => typed_questions(&items),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn typed_questions(items: &[Value]) -> Vec<Question> {
    items.iter().filter_map(question_from_value).collect()
}

fn question_from_value(value: &Value) -> Option<Question> {
    let question = str_field(value, "question")?;
    Some(Question {
        question,
        header: str_field(value, "header").unwrap_or_default(),
        multi_select: value.get("multiSelect") == Some(&Value::Bool(true)),
        options: value
            .get("options")
            .and_then(Value::as_array)
            .map(|options| options.iter().filter_map(option_from_value).collect())
            .unwrap_or_default(),
    })
}

fn option_from_value(value: &Value) -> Option<QuestionOption> {
    Some(QuestionOption {
        label: str_field(value, "label")?,
        description: str_field(value, "description").unwrap_or_default(),
    })
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn is_question_ask(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some(QUESTION_ASK)
}

fn decode_error(payload: &str, source: serde_json::Error) -> CoreError {
    CoreError::SessionEventDecode {
        detail: format!("{source} in {}", truncated(payload)),
    }
}

fn encode_error(event: &SessionEvent, source: serde_json::Error) -> CoreError {
    CoreError::SessionEventDecode {
        detail: format!("{source} while encoding a {} event", event.kind()),
    }
}

const DETAIL_CAP: usize = 200;

fn truncated(payload: &str) -> String {
    match payload.char_indices().nth(DETAIL_CAP) {
        Some((cut, _)) => format!("{}…", &payload[..cut]),
        None => payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprlayer_models::session::ExitOutcome;

    fn question(text: &str) -> Question {
        Question {
            question: text.into(),
            header: "Header".into(),
            multi_select: false,
            options: vec![QuestionOption {
                label: "Yes".into(),
                description: "Do it".into(),
            }],
        }
    }

    fn decoded_questions(payload: &str) -> Vec<Question> {
        match decode_raw_event(payload).unwrap() {
            SessionEvent::QuestionAsk { questions, .. } => questions,
            other => panic!("expected QuestionAsk, got {other:?}"),
        }
    }

    #[test]
    fn a_legacy_question_string_decodes_to_the_canonical_list() {
        let payload = r#"{"kind":"questionAsk","requestId":"q-1","questions":"[{\"header\":\"Header\",\"multiSelect\":false,\"options\":[{\"label\":\"Yes\",\"description\":\"Do it\"}],\"question\":\"Ship it?\"}]"}"#;
        assert_eq!(decoded_questions(payload), vec![question("Ship it?")]);
    }

    #[test]
    fn a_canonical_question_array_decodes_unchanged() {
        let payload = r#"{"kind":"questionAsk","requestId":"q-1","questions":[{"question":"Ship it?","header":"Header","multiSelect":false,"options":[{"label":"Yes","description":"Do it"}]}]}"#;
        assert_eq!(decoded_questions(payload), vec![question("Ship it?")]);
    }

    #[test]
    fn missing_malformed_and_wrongly_typed_questions_normalize_to_an_empty_list() {
        for payload in [
            r#"{"kind":"questionAsk","requestId":"q-1"}"#,
            r#"{"kind":"questionAsk","requestId":"q-1","questions":"not-valid-json{"}"#,
            r#"{"kind":"questionAsk","requestId":"q-1","questions":"{\"not\":\"an array\"}"}"#,
            r#"{"kind":"questionAsk","requestId":"q-1","questions":17}"#,
            r#"{"kind":"questionAsk","requestId":"q-1","questions":null}"#,
        ] {
            assert!(
                decoded_questions(payload).is_empty(),
                "should normalize to an empty list: {payload}"
            );
        }
    }

    #[test]
    fn entries_without_question_text_are_filtered_rather_than_failing_the_row() {
        let payload = r#"{"kind":"questionAsk","requestId":"q-4","questions":"[{\"unexpected\":\"shape\"},\"not-even-an-object\",{\"question\":\"Ship it?\",\"header\":\"Header\",\"options\":[{\"label\":\"Yes\",\"description\":\"Do it\"},{\"description\":\"no label\"}]}]"}"#;
        assert_eq!(decoded_questions(payload), vec![question("Ship it?")]);
    }

    #[test]
    fn a_row_naming_no_known_variant_is_a_typed_error() {
        for payload in [
            r#"{"kind":"toolPermissionEscalated","requestId":"future-1"}"#,
            r#"{"kind":"init"}"#,
            "not json at all",
        ] {
            let err = decode_raw_event(payload).unwrap_err();
            assert_eq!(err.code(), "session_event_decode", "{payload}");
        }
    }

    #[test]
    fn encoding_keeps_questions_string_encoded_for_rollback() {
        let event = SessionEvent::QuestionAsk {
            request_id: "q-1".into(),
            questions: vec![question("Ship it?")],
        };
        let raw = encode_raw_event(&event).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        let questions = value["questions"]
            .as_str()
            .expect("questions stay a JSON string on disk");
        assert!(serde_json::from_str::<Value>(questions).unwrap().is_array());
        assert_eq!(decode_raw_event(&raw).unwrap(), event);
    }

    #[test]
    fn every_other_variant_round_trips_through_the_codec() {
        for event in [
            SessionEvent::Init {
                provider_session_id: "abc".into(),
                model: Some("claude-opus-4-6".into()),
                mcp_servers: None,
            },
            SessionEvent::PartialText {
                text: "hi".into(),
                parent_id: None,
            },
            SessionEvent::Relaunched { quiet: true },
            SessionEvent::Exited {
                outcome: ExitOutcome::Completed,
                code: Some(0),
                stderr_tail: String::new(),
            },
        ] {
            let raw = encode_raw_event(&event).unwrap();
            assert_eq!(decode_raw_event(&raw).unwrap(), event, "{}", event.kind());
        }
    }

    #[test]
    fn a_truncated_payload_does_not_split_a_multibyte_character() {
        let payload = format!(r#"{{"kind":"nope","text":"{}"}}"#, "é".repeat(400));
        let err = decode_raw_event(&payload).unwrap_err();
        assert!(err.to_string().ends_with('…'));
    }
}
