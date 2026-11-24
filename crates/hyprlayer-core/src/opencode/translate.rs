//! OpenCode 2's `/api/event` payloads, and the stored messages a read-back replays, as
//! `SessionEvent`s. Everything here is idempotent per text part and tool call, so an event a
//! read-back already showed is not shown again when it arrives late.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::claude::SessionEvent;
use crate::models::session::{Question, QuestionAnswers, QuestionOption};

#[derive(Clone, Debug)]
pub(crate) struct OcEvent {
    pub kind: String,
    pub data: Value,
}

impl OcEvent {
    #[cfg(test)]
    pub fn new(kind: &str, data: Value) -> Self {
        Self {
            kind: kind.to_string(),
            data,
        }
    }

    /// The session an event belongs to; a form names it inside the form.
    pub fn session(&self) -> Option<&str> {
        self.data
            .get("sessionID")
            .or_else(|| self.data.pointer("/form/sessionID"))
            .and_then(Value::as_str)
    }

    pub fn str(&self, key: &str) -> &str {
        self.data
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    }
}

/// One SSE record; `: heartbeat` comments carry no data and yield nothing.
pub(crate) fn parse_sse_record(record: &str) -> Option<OcEvent> {
    let mut data = String::new();
    for line in record.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    let mut v: Value = serde_json::from_str(&data).ok()?;
    let kind = v.get("type")?.as_str()?.to_string();
    Some(OcEvent {
        kind,
        data: v.get_mut("data").map(Value::take).unwrap_or(Value::Null),
    })
}

/// How an execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Succeeded,
    Failed,
    Interrupted,
}

impl Outcome {
    pub fn of(kind: &str) -> Option<Self> {
        match kind {
            "session.execution.succeeded" => Some(Self::Succeeded),
            "session.execution.failed" => Some(Self::Failed),
            "session.execution.interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// The `outcome` of a stored `idle` message.
    pub fn of_idle(outcome: &str) -> Self {
        match outcome {
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            _ => Self::Succeeded,
        }
    }
}

#[derive(Default)]
pub(crate) struct Translator {
    tool_names: HashMap<String, String>,
    shown: HashSet<String>,
    /// A background subagent's session, and the call that launched it.
    tasks: HashMap<String, String>,
    pub cost_total: f64,
    pub context_tokens: Option<u32>,
    planning: bool,
}

impl Translator {
    /// The session's own content events: steps, text, reasoning, tools, usage, and a background
    /// subagent's result.
    pub fn content(&mut self, ev: &OcEvent) -> Vec<SessionEvent> {
        let d = &ev.data;
        let message = ev.str("assistantMessageID");
        let ordinal = d.get("ordinal").and_then(Value::as_u64).unwrap_or(0);
        match ev.kind.as_str() {
            "session.step.started" => self.agent(ev.str("agent")),
            "session.text.delta" => {
                let delta = ev.str("delta");
                if delta.is_empty() || self.shown.contains(&text_key(message, ordinal)) {
                    return Vec::new();
                }
                vec![SessionEvent::PartialText {
                    text: delta.to_string(),
                    parent_id: None,
                }]
            }
            "session.text.ended" => self.text(message, ordinal, ev.str("text")),
            "session.reasoning.ended" => self.reasoning(message, ordinal, ev.str("text")),
            "session.tool.input.started" => {
                self.tool_names
                    .insert(ev.str("id").to_string(), ev.str("name").to_string());
                Vec::new()
            }
            "session.tool.called" => {
                let id = ev.str("id");
                let name = self.tool_names.get(id).cloned().unwrap_or_default();
                self.tool_use(id, &name, d.get("input").cloned().unwrap_or(json!({})))
            }
            "session.tool.progress" => {
                self.note_task(ev.str("id"), d.get("metadata"));
                Vec::new()
            }
            "session.tool.success" => {
                self.note_task(ev.str("id"), d.get("metadata"));
                self.tool_result(ev.str("id"), content_text(d.get("content")), false)
            }
            "session.tool.failed" => {
                let error = d.pointer("/error/message").and_then(Value::as_str);
                let mut text = error.unwrap_or("the tool failed").to_string();
                let more = content_text(d.get("content"));
                if !more.is_empty() {
                    text = format!("{text}\n{more}");
                }
                self.tool_result(ev.str("id"), text, true)
            }
            "session.step.ended" => self.usage(d.get("cost"), d.get("tokens")),
            "session.inbox.enqueued" => self.background_result(d.get("item")),
            _ => Vec::new(),
        }
    }

    /// A stored assistant message, as a read-back after a gap in the stream sees it.
    pub fn replay(&mut self, message: &Value) -> Vec<SessionEvent> {
        let id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut out = self.agent(
            message
                .get("agent")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let (mut texts, mut thoughts) = (0, 0);
        for item in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    out.extend(self.text(id, texts, text));
                    texts += 1;
                }
                Some("reasoning") => {
                    out.extend(self.reasoning(id, thoughts, text));
                    thoughts += 1;
                }
                Some("tool") => {
                    let call = item.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                    self.tool_names.insert(call.to_string(), name.to_string());
                    let state = item.get("state").unwrap_or(&Value::Null);
                    let input = state.get("input").cloned().unwrap_or(json!({}));
                    out.extend(self.tool_use(call, name, input));
                    self.note_task(call, state.get("metadata"));
                    match state.get("status").and_then(Value::as_str) {
                        Some("completed") => out.extend(self.tool_result(
                            call,
                            content_text(state.get("content")),
                            false,
                        )),
                        Some("error") => {
                            let error = state.pointer("/error/message").and_then(Value::as_str);
                            let text = error.unwrap_or("the tool failed").to_string();
                            out.extend(self.tool_result(call, text, true));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn ended_as(&self, status: &str) -> SessionEvent {
        SessionEvent::TurnDone {
            cost_usd: Some(self.cost_total),
            num_turns: None,
            context_tokens: self.context_tokens,
            turn_status: Some(status.into()),
            thread_tokens: None,
        }
    }

    /// Every step names the agent answering it; `plan` is OpenCode's plan mode.
    fn agent(&mut self, agent: &str) -> Vec<SessionEvent> {
        if agent.is_empty() {
            return Vec::new();
        }
        let planning = agent == "plan";
        if planning == self.planning {
            return Vec::new();
        }
        self.planning = planning;
        vec![SessionEvent::PlanModeChanged { active: planning }]
    }

    fn text(&mut self, message: &str, ordinal: u64, text: &str) -> Vec<SessionEvent> {
        if text.is_empty() || !self.shown.insert(text_key(message, ordinal)) {
            return Vec::new();
        }
        vec![SessionEvent::AssistantText {
            text: text.to_string(),
            parent_id: None,
            injected: false,
            auth_required: false,
        }]
    }

    fn reasoning(&mut self, message: &str, ordinal: u64, text: &str) -> Vec<SessionEvent> {
        if text.is_empty() || !self.shown.insert(format!("r:{message}:{ordinal}")) {
            return Vec::new();
        }
        vec![SessionEvent::Thinking {
            text: text.to_string(),
            parent_id: None,
        }]
    }

    /// A question tool's ask is the form; its call is not shown as a tool.
    fn tool_use(&mut self, id: &str, name: &str, input: Value) -> Vec<SessionEvent> {
        if id.is_empty() || name == "question" || !self.shown.insert(format!("u:{id}")) {
            return Vec::new();
        }
        let (name, input) = normalize_tool(name, input);
        vec![SessionEvent::ToolUse {
            id: id.to_string(),
            name,
            input: input.to_string(),
            parent_id: None,
        }]
    }

    fn tool_result(&mut self, id: &str, content: String, is_error: bool) -> Vec<SessionEvent> {
        let question = self.tool_names.get(id).is_some_and(|n| n == "question");
        if id.is_empty() || question || !self.shown.insert(format!("o:{id}")) {
            return Vec::new();
        }
        vec![SessionEvent::ToolResult {
            tool_use_id: id.to_string(),
            content,
            is_error,
            parent_id: None,
        }]
    }

    fn note_task(&mut self, call: &str, metadata: Option<&Value>) {
        let child = metadata
            .and_then(|m| m.get("sessionID"))
            .and_then(Value::as_str);
        if let (Some(child), false) = (child, call.is_empty()) {
            self.tasks.insert(child.to_string(), call.to_string());
        }
    }

    fn usage(&mut self, cost: Option<&Value>, tokens: Option<&Value>) -> Vec<SessionEvent> {
        self.cost_total += cost.and_then(Value::as_f64).unwrap_or(0.0);
        let Some(tokens) = tokens else {
            return Vec::new();
        };
        let n = |path: &str| tokens.pointer(path).and_then(Value::as_f64).unwrap_or(0.0);
        let total =
            n("/input") + n("/output") + n("/reasoning") + n("/cache/read") + n("/cache/write");
        let context_tokens = total.min(u32::MAX as f64) as u32;
        self.context_tokens = Some(context_tokens);
        vec![SessionEvent::Usage {
            context_tokens,
            thread_tokens: None,
        }]
    }

    /// A subagent's round ended: OpenCode reports it to the parent's inbox as
    /// `<background-result kind="subagent" sessionID="…" state="…">`.
    fn background_result(&mut self, item: Option<&Value>) -> Vec<SessionEvent> {
        let Some(item) =
            item.filter(|i| i.get("type").and_then(Value::as_str) == Some("synthetic"))
        else {
            return Vec::new();
        };
        let text = item
            .pointer("/payload/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(head) = text
            .strip_prefix("<background-result")
            .and_then(|rest| rest.split('>').next())
        else {
            return Vec::new();
        };
        let Some(child) = attribute(head, "sessionID") else {
            return Vec::new();
        };
        let status = attribute(head, "state").unwrap_or("completed");
        vec![SessionEvent::BackgroundTaskUpdate {
            task_id: child.to_string(),
            tool_use_id: self.tasks.get(child).cloned(),
            status: status.to_string(),
        }]
    }
}

fn text_key(message: &str, ordinal: u64) -> String {
    format!("t:{message}:{ordinal}")
}

fn attribute<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    let start = head.find(&format!("{name}=\""))? + name.len() + 2;
    let len = head[start..].find('"')?;
    Some(&head[start..start + len])
}

fn content_text(content: Option<&Value>) -> String {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match item.get("type").and_then(Value::as_str) {
            Some("text") => item.get("text").and_then(Value::as_str).map(str::to_string),
            Some("file") => item.get("uri").and_then(Value::as_str).map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// OpenCode 2's tool names and input fields in the Claude vocabulary the transcript renders.
pub(crate) fn normalize_tool(tool: &str, mut input: Value) -> (String, Value) {
    fn rename(input: &mut Value, from: &str, to: &str) {
        if let Some(obj) = input.as_object_mut() {
            if let Some(v) = obj.remove(from) {
                obj.insert(to.to_string(), v);
            }
        }
    }
    let name = match tool {
        "read" => {
            rename(&mut input, "path", "file_path");
            "Read"
        }
        "write" => {
            rename(&mut input, "path", "file_path");
            "Write"
        }
        "edit" => {
            rename(&mut input, "path", "file_path");
            rename(&mut input, "oldString", "old_string");
            rename(&mut input, "newString", "new_string");
            "Edit"
        }
        "shell" => "Bash",
        "grep" => "Grep",
        "glob" => "Glob",
        "webfetch" => "WebFetch",
        "todowrite" => "TodoWrite",
        // OpenCode 2 runs a subagent in the background and reports its result to the parent's
        // inbox, as a backgrounded Claude agent does.
        "subagent" => {
            rename(&mut input, "agent", "subagent_type");
            if let Some(obj) = input.as_object_mut() {
                obj.insert("run_in_background".into(), Value::Bool(true));
            }
            "Task"
        }
        _ => return (tool.to_string(), input),
    };
    (name.to_string(), input)
}

/// What a permission ask shows: OpenCode's action and the resources it names.
pub(crate) fn permission_ask(data: &Value) -> SessionEvent {
    let action = data
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    SessionEvent::PermissionAsk {
        request_id: data
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tool_use_id: None,
        tool_name: if action.is_empty() { "tool" } else { action }.to_string(),
        input: json!({
            "patterns": data.get("resources").cloned().unwrap_or(json!([])),
            "metadata": data.get("metadata").cloned().unwrap_or(json!({})),
        })
        .to_string(),
    }
}

/// A form's field as the question the composer asks: its description when it has one.
fn field_question(field: &Value) -> String {
    let get = |key| {
        field
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    get("description")
        .or_else(|| get("title"))
        .or_else(|| get("key"))
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn form_ask(form: &Value) -> SessionEvent {
    let questions = form_fields(form)
        .iter()
        .map(|field| {
            let title = field
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let question = field_question(field);
            Question {
                header: if title == question {
                    String::new()
                } else {
                    title.to_string()
                },
                question,
                multi_select: field.get("type").and_then(Value::as_str) == Some("multiselect"),
                options: field
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(QuestionOption {
                            label: option.get("label").and_then(Value::as_str)?.to_string(),
                            description: option
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        })
                    })
                    .collect(),
            }
        })
        .collect();
    SessionEvent::QuestionAsk {
        request_id: form
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        questions,
        subject: None,
    }
}

pub(crate) fn form_fields(form: &Value) -> Vec<Value> {
    form.get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The composer's answers, keyed by question text, as the form's `answer`: an option's label
/// becomes its value, a multiselect's `, `-joined labels a list.
pub(crate) fn form_answer(fields: &[Value], answers: &QuestionAnswers) -> Value {
    let mut answer = serde_json::Map::new();
    for field in fields {
        let Some(key) = field.get("key").and_then(Value::as_str) else {
            continue;
        };
        let Some(given) = answers
            .get(&field_question(field))
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let value_of = |label: &str| {
            field
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|o| o.get("label").and_then(Value::as_str) == Some(label))
                .and_then(|o| o.get("value").and_then(Value::as_str))
                .unwrap_or(label)
                .to_string()
        };
        let value = match field.get("type").and_then(Value::as_str) {
            Some("multiselect") => json!(given.split(", ").map(value_of).collect::<Vec<_>>()),
            Some("boolean") => json!(matches!(
                given.to_ascii_lowercase().as_str(),
                "yes" | "true"
            )),
            Some("number" | "integer") => given
                .parse::<f64>()
                .map_or_else(|_| json!(given), |n| json!(n)),
            _ => json!(value_of(given)),
        };
        answer.insert(key.to_string(), value);
    }
    Value::Object(answer)
}

/// The transcript line for an execution that failed; a rejected key asks the user to sign in.
pub(crate) fn failure_text(error: Option<&Value>) -> SessionEvent {
    let message = error
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("OpenCode reported an error");
    let kind = error
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let status = error.and_then(|e| e.get("status")).and_then(Value::as_u64);
    SessionEvent::AssistantText {
        text: format!("⚠ OpenCode error: {message}"),
        parent_id: None,
        injected: false,
        auth_required: status == Some(401) || kind.contains("auth"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, data: Value) -> OcEvent {
        OcEvent::new(kind, data)
    }

    #[test]
    fn an_sse_record_unwraps_its_data_and_a_heartbeat_yields_nothing() {
        let record = "data: {\"id\":\"evt_1\",\"type\":\"session.execution.started\",\"data\":{\"sessionID\":\"ses_1\"},\"durable\":{\"seq\":2}}";
        let parsed = parse_sse_record(record).expect("an event");
        assert_eq!(parsed.kind, "session.execution.started");
        assert_eq!(parsed.session(), Some("ses_1"));
        assert!(parse_sse_record(": heartbeat\n").is_none());
        let form = ev(
            "form.created",
            json!({ "form": { "id": "frm_1", "sessionID": "ses_2" } }),
        );
        assert_eq!(form.session(), Some("ses_2"));
    }

    #[test]
    fn text_streams_as_deltas_and_lands_once_in_full() {
        let mut x = Translator::default();
        let data = |extra: Value| {
            let mut d = json!({ "sessionID": "s", "assistantMessageID": "msg_a", "ordinal": 0 });
            d.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            d
        };
        let delta = x.content(&ev("session.text.delta", data(json!({ "delta": "hel" }))));
        assert!(matches!(&delta[..], [SessionEvent::PartialText { text, .. }] if text == "hel"));
        let ended = ev("session.text.ended", data(json!({ "text": "hello" })));
        let full = x.content(&ended);
        assert!(matches!(&full[..], [SessionEvent::AssistantText { text, .. }] if text == "hello"));
        assert!(x.content(&ended).is_empty(), "a repeat is not shown again");
        let late = x.content(&ev("session.text.delta", data(json!({ "delta": "lo" }))));
        assert!(late.is_empty(), "a delta after the full text is dropped");

        let thought = ev("session.reasoning.ended", data(json!({ "text": "plan" })));
        let thinking = x.content(&thought);
        assert!(matches!(&thinking[..], [SessionEvent::Thinking { text, .. }] if text == "plan"));
    }

    #[test]
    fn tools_map_to_the_claude_vocabulary_with_their_results() {
        let mut x = Translator::default();
        fn call(x: &mut Translator, id: &str, name: &str, input: Value) -> Vec<SessionEvent> {
            let started = ev(
                "session.tool.input.started",
                json!({ "id": id, "name": name }),
            );
            let mut out = x.content(&started);
            out.extend(x.content(&ev(
                "session.tool.called",
                json!({ "id": id, "input": input }),
            )));
            out
        }
        let read = call(&mut x, "c1", "read", json!({ "path": "notes.txt" }));
        match &read[..] {
            [SessionEvent::ToolUse {
                id, name, input, ..
            }] => {
                assert_eq!((id.as_str(), name.as_str()), ("c1", "Read"));
                assert_eq!(
                    serde_json::from_str::<Value>(input).unwrap(),
                    json!({ "file_path": "notes.txt" })
                );
            }
            other => panic!("expected one ToolUse, got {other:?}"),
        }
        let ok = x.content(&ev(
            "session.tool.success",
            json!({ "id": "c1", "content": [{ "type": "text", "text": "1: hello" }] }),
        ));
        assert!(
            matches!(&ok[..], [SessionEvent::ToolResult { content, is_error: false, .. }] if content == "1: hello")
        );

        let shell = call(&mut x, "c2", "shell", json!({ "command": "ls" }));
        assert!(matches!(&shell[..], [SessionEvent::ToolUse { name, .. }] if name == "Bash"));
        let failed = x.content(&ev(
            "session.tool.failed",
            json!({ "id": "c2", "error": { "type": "aborted", "message": "Tool execution interrupted" } }),
        ));
        assert!(
            matches!(&failed[..], [SessionEvent::ToolResult { is_error: true, content, .. }] if content == "Tool execution interrupted")
        );

        let (name, input) = normalize_tool(
            "edit",
            json!({ "path": "a", "oldString": "x", "newString": "y" }),
        );
        assert_eq!(name, "Edit");
        assert_eq!(
            input,
            json!({ "file_path": "a", "old_string": "x", "new_string": "y" })
        );

        assert!(
            call(&mut x, "c3", "question", json!({ "questions": [] })).is_empty(),
            "a question is its form"
        );
        let answered = ev(
            "session.tool.success",
            json!({ "id": "c3", "content": [{ "type": "text", "text": "answered" }] }),
        );
        assert!(x.content(&answered).is_empty());

        assert_eq!(
            normalize_tool("some_mcp_tool", json!({ "a": 1 })),
            ("some_mcp_tool".into(), json!({ "a": 1 }))
        );
    }

    #[test]
    fn a_subagent_is_a_background_task_its_result_closes() {
        let mut x = Translator::default();
        x.content(&ev(
            "session.tool.input.started",
            json!({ "id": "c1", "name": "subagent" }),
        ));
        let launched = x.content(&ev(
            "session.tool.called",
            json!({ "id": "c1", "input": { "agent": "general", "description": "Read it", "prompt": "read" } }),
        ));
        match &launched[..] {
            [SessionEvent::ToolUse { name, input, .. }] => {
                assert_eq!(name, "Task");
                let input: Value = serde_json::from_str(input).unwrap();
                assert_eq!(input["subagent_type"], "general");
                assert_eq!(input["run_in_background"], true);
            }
            other => panic!("expected one ToolUse, got {other:?}"),
        }
        x.content(&ev(
            "session.tool.success",
            json!({ "id": "c1", "content": [{ "type": "text", "text": "working" }], "metadata": { "sessionID": "ses_child", "status": "running" } }),
        ));
        let result = x.content(&ev(
            "session.inbox.enqueued",
            json!({ "inboxID": "msg_x", "item": { "type": "synthetic", "payload": {
                "text": "<background-result kind=\"subagent\" sessionID=\"ses_child\" state=\"completed\" description=\"Read it\">\nhello\n</background-result>"
            } } }),
        ));
        match &result[..] {
            [SessionEvent::BackgroundTaskUpdate {
                task_id,
                tool_use_id,
                status,
            }] => {
                assert_eq!(task_id, "ses_child");
                assert_eq!(tool_use_id.as_deref(), Some("c1"));
                assert_eq!(status, "completed");
            }
            other => panic!("expected one BackgroundTaskUpdate, got {other:?}"),
        }
        let user = ev(
            "session.inbox.enqueued",
            json!({ "item": { "type": "user", "payload": { "text": "hi" } } }),
        );
        assert!(x.content(&user).is_empty());
    }

    #[test]
    fn steps_sum_cost_report_context_and_mark_plan_mode() {
        let mut x = Translator::default();
        let tokens = json!({ "input": 100, "output": 10, "reasoning": 5, "cache": { "read": 80, "write": 5 } });
        for cost in [0.25, 0.75] {
            let usage = x.content(&ev(
                "session.step.ended",
                json!({ "finish": "tool-calls", "cost": cost, "tokens": tokens }),
            ));
            assert!(matches!(
                &usage[..],
                [SessionEvent::Usage {
                    context_tokens: 200,
                    ..
                }]
            ));
        }
        assert!(
            matches!(x.ended_as("completed"), SessionEvent::TurnDone { cost_usd: Some(c), context_tokens: Some(200), .. } if c == 1.0)
        );
        let plan = x.content(&ev("session.step.started", json!({ "agent": "plan" })));
        assert!(matches!(
            &plan[..],
            [SessionEvent::PlanModeChanged { active: true }]
        ));
        assert!(x
            .content(&ev("session.step.started", json!({ "agent": "plan" })))
            .is_empty());
        let build = x.content(&ev("session.step.started", json!({ "agent": "build" })));
        assert!(matches!(
            &build[..],
            [SessionEvent::PlanModeChanged { active: false }]
        ));
    }

    #[test]
    fn a_replayed_message_shows_only_what_the_stream_did_not() {
        let mut x = Translator::default();
        x.content(&ev(
            "session.text.ended",
            json!({ "assistantMessageID": "msg_a", "ordinal": 0, "text": "first" }),
        ));
        let stored = json!({ "id": "msg_a", "type": "assistant", "agent": "build", "content": [
            { "type": "reasoning", "text": "think" },
            { "type": "text", "text": "first" },
            { "type": "tool", "id": "c1", "name": "read", "state": { "status": "completed", "input": { "path": "a" }, "content": [{ "type": "text", "text": "A" }] } },
            { "type": "text", "text": "second" },
        ] });
        let replayed = x.replay(&stored);
        let kinds: Vec<&str> = replayed
            .iter()
            .map(|e| match e {
                SessionEvent::Thinking { .. } => "thinking",
                SessionEvent::AssistantText { text, .. } if text == "second" => "second",
                SessionEvent::ToolUse { .. } => "use",
                SessionEvent::ToolResult { .. } => "result",
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(kinds, ["thinking", "use", "result", "second"]);
        assert!(
            x.replay(&stored).is_empty(),
            "a second read-back shows nothing"
        );
    }

    #[test]
    fn a_form_asks_its_fields_and_takes_answers_by_question() {
        let form = json!({ "id": "frm_1", "sessionID": "s", "title": "Questions", "fields": [
            { "key": "q0", "title": "Color", "description": "Which color?", "type": "string",
              "options": [{ "value": "r", "label": "Red" }, { "value": "b", "label": "Blue", "description": "cool" }] },
            { "key": "q1", "title": "Pick", "description": "Which ones?", "type": "multiselect",
              "options": [{ "value": "x", "label": "X" }, { "value": "y", "label": "Y" }] },
            { "key": "q2", "title": "Name", "type": "string" },
        ] });
        match form_ask(&form) {
            SessionEvent::QuestionAsk {
                request_id,
                questions,
                ..
            } => {
                assert_eq!(request_id, "frm_1");
                assert_eq!(questions[0].question, "Which color?");
                assert_eq!(questions[0].header, "Color");
                assert_eq!(questions[0].options[1].description, "cool");
                assert!(questions[1].multi_select);
                assert_eq!(
                    (questions[2].question.as_str(), questions[2].header.as_str()),
                    ("Name", "")
                );
            }
            other => panic!("expected a QuestionAsk, got {other:?}"),
        }
        let answers = QuestionAnswers(
            [
                ("Which color?", "Blue"),
                ("Which ones?", "X, Y"),
                ("Name", "Ada"),
            ]
            .into_iter()
            .map(|(q, a)| (q.to_string(), a.to_string()))
            .collect(),
        );
        assert_eq!(
            form_answer(&form_fields(&form), &answers),
            json!({ "q0": "b", "q1": ["x", "y"], "q2": "Ada" })
        );
    }

    #[test]
    fn a_permission_names_its_action_and_resources() {
        let ask = permission_ask(
            &json!({ "id": "per_1", "sessionID": "s", "action": "read", "resources": [".env"] }),
        );
        match ask {
            SessionEvent::PermissionAsk {
                request_id,
                tool_name,
                input,
                tool_use_id,
            } => {
                assert_eq!((request_id.as_str(), tool_name.as_str()), ("per_1", "read"));
                assert!(tool_use_id.is_none());
                assert_eq!(
                    serde_json::from_str::<Value>(&input).unwrap()["patterns"],
                    json!([".env"])
                );
            }
            other => panic!("expected a PermissionAsk, got {other:?}"),
        }
    }

    #[test]
    fn a_rejected_key_reads_as_a_sign_in_failure() {
        let auth = failure_text(Some(
            &json!({ "type": "provider.auth", "message": "bad key", "status": 401 }),
        ));
        assert!(
            matches!(auth, SessionEvent::AssistantText { auth_required: true, ref text, .. } if text == "⚠ OpenCode error: bad key")
        );
        let other = failure_text(Some(
            &json!({ "type": "provider.unknown-model", "message": "no model" }),
        ));
        assert!(matches!(
            other,
            SessionEvent::AssistantText {
                auth_required: false,
                ..
            }
        ));
    }
}
