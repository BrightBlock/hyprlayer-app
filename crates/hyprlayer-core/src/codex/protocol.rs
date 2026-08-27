use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::claude::driver::PermissionMode;
use crate::claude::events::SessionEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub fn decode_line(line: &str) -> Option<Incoming> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    let obj = v.as_object()?;
    let method = obj.get("method").and_then(Value::as_str);

    match (method, obj.get("id")) {
        (Some(m), Some(id)) => Some(Incoming::ServerRequest {
            id: id.clone(),
            method: m.to_string(),
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(m), None) => Some(Incoming::Notification {
            method: m.to_string(),
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let id = id.as_u64()?;
            let result = match obj.get("error") {
                Some(e) => Err(e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("codex request failed")
                    .to_string()),
                None => Ok(obj.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Response { id, result })
        }
        (None, None) => None,
    }
}

pub fn approval_posture(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Default => ("untrusted", "workspace-write"),
        PermissionMode::AcceptEdits => ("never", "workspace-write"),
        PermissionMode::Plan => ("on-request", "read-only"),
        PermissionMode::BypassPermissions => ("never", "danger-full-access"),
    }
}

/// Codex's native guardrail primitive. Unlike Claude, codex has no per-command
/// allow/deny; a stage's intent is lowered onto this sandbox + approval posture.
/// `network_access` only bites under `workspace-write` (the other sandboxes fix
/// network on their own).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexPosture {
    pub approval_policy: &'static str,
    pub sandbox: &'static str,
    pub network_access: bool,
}

impl CodexPosture {
    pub fn from_permission_mode(mode: PermissionMode) -> Self {
        let (approval_policy, sandbox) = approval_posture(mode);
        Self {
            approval_policy,
            sandbox,
            network_access: sandbox == "workspace-write",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollabMode {
    Plan,
    Default,
}

impl CollabMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CollabMode::Plan => "plan",
            CollabMode::Default => "default",
        }
    }
}

#[cfg(test)]
const VALID_APPROVAL_POLICIES: [&str; 3] = ["untrusted", "on-request", "never"];
#[cfg(test)]
const VALID_SANDBOX_MODES: [&str; 3] = ["read-only", "workspace-write", "danger-full-access"];

#[derive(Debug, Clone)]
pub struct PendingQuestion {
    pub rpc_id: Value,
    pub by_text: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalKind {
    CommandExecution,
    FileChange,
    Permissions { requested: Value },
    Legacy,
}

#[derive(Default)]
pub struct Translator {
    root_thread_id: Option<String>,
    root_rpc: Option<u64>,
    pub active_turn: Option<String>,
    pub requested_turn: Option<String>,
    started_tools: HashSet<String>,
    // Child transcript events stay hidden, but file-change approvals omit the
    // proposed patch and need the preceding item/started payload for context.
    child_file_change_inputs: HashMap<(String, String), Map<String, Value>>,
    context_window_sent: bool,
    pub pending_approvals: HashMap<String, (Value, ApprovalKind)>,
    pub pending_questions: HashMap<String, PendingQuestion>,
    pub user_turns: usize,
    pub collab_mode: Option<String>,
}

impl Translator {
    pub fn resumed(prior_user_turns: usize) -> Self {
        Self {
            user_turns: prior_user_turns,
            ..Self::default()
        }
    }

    pub fn set_root_thread_id(&mut self, thread_id: String) {
        self.root_thread_id = Some(thread_id);
    }

    // codex replays thread history right behind a resume response, so the read
    // loop arms the root filter itself before decoding the next line — the
    // request task's await lands too late.
    pub fn expect_root_thread_from(&mut self, rpc_id: u64) {
        self.root_rpc = Some(rpc_id);
    }

    pub fn observe_response(&mut self, rpc_id: u64, result: &Result<Value, String>) {
        if self.root_rpc != Some(rpc_id) {
            return;
        }
        self.root_rpc = None;
        if let Ok(value) = result {
            if let Some(thread_id) = value
                .get("thread")
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
            {
                self.set_root_thread_id(thread_id.to_string());
            }
        }
    }

    pub fn accepts_notification(&self, p: &Value) -> bool {
        let Some(root_thread_id) = self.root_thread_id.as_deref() else {
            return true;
        };
        match p.get("threadId") {
            None => true,
            Some(Value::String(thread_id)) => thread_id == root_thread_id,
            Some(_) => false,
        }
    }

    pub(crate) fn is_child_thread(&self, p: &Value) -> bool {
        let Some(root_thread_id) = self.root_thread_id.as_deref() else {
            return false;
        };
        p.get("threadId")
            .and_then(Value::as_str)
            .is_some_and(|thread_id| thread_id != root_thread_id)
    }

    fn observe_child_notification(&mut self, method: &str, p: &Value) {
        let Some(thread_id) = p.get("threadId").and_then(Value::as_str) else {
            return;
        };
        match method {
            "item/started" => {
                let item = p.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) != Some("fileChange") {
                    return;
                }
                let Some((item_id, _, Value::Object(input))) = tool_call(item) else {
                    return;
                };
                self.child_file_change_inputs
                    .insert((thread_id.to_string(), item_id), input);
            }
            "item/completed" => {
                let item_id = p.get("itemId").and_then(Value::as_str).or_else(|| {
                    p.get("item")
                        .and_then(|item| item.get("id"))
                        .and_then(Value::as_str)
                });
                if let Some(item_id) = item_id {
                    self.child_file_change_inputs
                        .remove(&(thread_id.to_string(), item_id.to_string()));
                }
            }
            "turn/completed" | "thread/closed" => {
                self.child_file_change_inputs
                    .retain(|(cached_thread_id, _), _| cached_thread_id != thread_id);
            }
            _ => {}
        }
    }

    fn take_child_file_change_input(&mut self, p: &Value) -> Option<Map<String, Value>> {
        if !self.is_child_thread(p) {
            return None;
        }
        let thread_id = p.get("threadId").and_then(Value::as_str)?;
        let item_id = p.get("itemId").and_then(Value::as_str)?;
        self.child_file_change_inputs
            .remove(&(thread_id.to_string(), item_id.to_string()))
    }

    pub fn notification(&mut self, method: &str, p: &Value) -> Vec<SessionEvent> {
        if !self.accepts_notification(p) {
            self.observe_child_notification(method, p);
            if method == "error" {
                if let Some(msg) = error_text(p) {
                    return vec![SessionEvent::AssistantText {
                        text: format!("⚠ Codex sub-agent error: {msg}"),
                        parent_id: None,
                        injected: false,
                    }];
                }
            }
            return Vec::new();
        }
        match method {
            "turn/started" => {
                self.active_turn = p
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Vec::new()
            }
            "turn/completed" => {
                self.child_file_change_inputs.clear();
                self.active_turn = None;
                self.requested_turn = None;
                let raw_status = p
                    .get("turn")
                    .and_then(|t| t.get("status"))
                    .and_then(Value::as_str);
                let status = raw_status.unwrap_or("completed");
                let mut out = Vec::new();
                if status == "failed" {
                    if let Some(msg) = p
                        .get("turn")
                        .and_then(|t| t.get("error"))
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                    {
                        out.push(SessionEvent::AssistantText {
                            text: format!("⚠ Codex error: {msg}"),
                            parent_id: None,
                            injected: false,
                        });
                    }
                }
                out.push(SessionEvent::TurnDone {
                    cost_usd: None,
                    num_turns: None,
                    context_tokens: None,
                    turn_status: raw_status.map(str::to_string),
                });
                out
            }
            "item/agentMessage/delta" => {
                let delta = p.get("delta").and_then(Value::as_str).unwrap_or_default();
                if delta.is_empty() {
                    Vec::new()
                } else {
                    vec![SessionEvent::PartialText {
                        text: delta.to_string(),
                        parent_id: None,
                    }]
                }
            }
            "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => Vec::new(),
            "item/commandExecution/outputDelta" => Vec::new(),
            "item/started" => self.item_started(p.get("item").unwrap_or(&Value::Null)),
            "item/completed" => self.item_completed(p.get("item").unwrap_or(&Value::Null)),
            "turn/plan/updated" => plan_events(p),
            "thread/tokenUsage/updated" => self.token_usage(p),
            "error" => match error_text(p) {
                Some(msg) => vec![
                    SessionEvent::AssistantText {
                        text: format!("⚠ Codex error: {msg}"),
                        parent_id: None,
                        injected: false,
                    },
                    SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                        turn_status: Some("failed".into()),
                    },
                ],
                None => Vec::new(),
            },
            "thread/settings/updated" => {
                self.collab_mode = p
                    .get("threadSettings")
                    .and_then(|s| s.get("collaborationMode"))
                    .and_then(|c| c.get("mode"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn server_request(&mut self, id: &Value, method: &str, p: &Value) -> Vec<SessionEvent> {
        // Parallel child questions cannot share the root UI's single question
        // slot. Returning no event makes the driver answer this request empty.
        if method == "item/tool/requestUserInput" && self.is_child_thread(p) {
            return Vec::new();
        }
        match method {
            "item/commandExecution/requestApproval" => {
                let request_id = approval_request_id(p, id);
                self.pending_approvals.insert(
                    request_id.clone(),
                    (id.clone(), ApprovalKind::CommandExecution),
                );
                let mut input = Map::new();
                if let Some(c) = p.get("command") {
                    input.insert("command".into(), c.clone());
                }
                if let Some(c) = p.get("cwd") {
                    input.insert("cwd".into(), c.clone());
                }
                if let Some(r) = p.get("reason").filter(|r| !r.is_null()) {
                    input.insert("reason".into(), r.clone());
                }
                vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name: "Bash".into(),
                    input: Value::Object(input).to_string(),
                }]
            }
            "item/fileChange/requestApproval" => {
                let request_id = approval_request_id(p, id);
                self.pending_approvals
                    .insert(request_id.clone(), (id.clone(), ApprovalKind::FileChange));
                let mut input = self.take_child_file_change_input(p).unwrap_or_default();
                if let Some(r) = p.get("reason").filter(|r| !r.is_null()) {
                    input.insert("reason".into(), r.clone());
                }
                if let Some(r) = p.get("grantRoot").filter(|r| !r.is_null()) {
                    input.insert("grant_root".into(), r.clone());
                }
                vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name: "Edit".into(),
                    input: Value::Object(input).to_string(),
                }]
            }
            "item/permissions/requestApproval" => {
                let request_id = approval_request_id(p, id);
                let requested = p.get("permissions").cloned().unwrap_or(json!({}));
                self.pending_approvals.insert(
                    request_id.clone(),
                    (
                        id.clone(),
                        ApprovalKind::Permissions {
                            requested: requested.clone(),
                        },
                    ),
                );
                vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name: "Permissions".into(),
                    input: json!({
                        "cwd": p.get("cwd").cloned().unwrap_or(Value::Null),
                        "reason": p.get("reason").cloned().unwrap_or(Value::Null),
                        "permissions": requested,
                    })
                    .to_string(),
                }]
            }
            "execCommandApproval" | "applyPatchApproval" => {
                let request_id = p
                    .get("callId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| id.to_string());
                self.pending_approvals
                    .insert(request_id.clone(), (id.clone(), ApprovalKind::Legacy));
                let (tool_name, input) = if method == "execCommandApproval" {
                    (
                        "Bash",
                        json!({
                            "command": join_argv(p.get("command")),
                            "cwd": p.get("cwd").cloned().unwrap_or(Value::Null),
                        }),
                    )
                } else {
                    (
                        "Edit",
                        json!({ "file_changes": p.get("fileChanges").cloned().unwrap_or(json!({})) }),
                    )
                };
                vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name: tool_name.into(),
                    input: input.to_string(),
                }]
            }
            "item/tool/requestUserInput" => {
                let request_id = approval_request_id(p, id);
                let questions = p
                    .get("questions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut by_text = HashMap::new();
                let ui: Vec<Value> = questions
                    .iter()
                    .map(|q| {
                        let text = q
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(qid) = q.get("id").and_then(Value::as_str) {
                            by_text.insert(text.to_string(), qid.to_string());
                        }
                        json!({
                            "question": text,
                            "header": q.get("header").and_then(Value::as_str).unwrap_or_default(),
                            "multiSelect": false,
                            "options": q
                                .get("options")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default(),
                        })
                    })
                    .collect();
                self.pending_questions.insert(
                    request_id.clone(),
                    PendingQuestion {
                        rpc_id: id.clone(),
                        by_text,
                    },
                );
                vec![SessionEvent::QuestionAsk {
                    request_id,
                    questions: Value::Array(ui).to_string(),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn token_usage(&mut self, p: &Value) -> Vec<SessionEvent> {
        let usage = p.get("tokenUsage");
        let mut out = Vec::new();
        if !self.context_window_sent {
            if let Some(w) = usage
                .and_then(|u| u.get("modelContextWindow"))
                .and_then(Value::as_u64)
                .filter(|w| *w > 0)
            {
                self.context_window_sent = true;
                out.push(SessionEvent::ContextWindow {
                    tokens: w.min(u32::MAX as u64) as u32,
                });
            }
        }
        if let Some(ctx) = usage
            .and_then(|u| u.get("last"))
            .and_then(|l| l.get("inputTokens"))
            .and_then(Value::as_u64)
        {
            out.push(SessionEvent::Usage {
                context_tokens: ctx.min(u32::MAX as u64) as u32,
            });
        }
        out
    }

    fn item_started(&mut self, item: &Value) -> Vec<SessionEvent> {
        let Some((id, name, input)) = tool_call(item) else {
            return Vec::new();
        };
        if !self.started_tools.insert(id.clone()) {
            return Vec::new();
        }
        vec![SessionEvent::ToolUse {
            id,
            name,
            input: input.to_string(),
            parent_id: None,
        }]
    }

    fn item_completed(&mut self, item: &Value) -> Vec<SessionEvent> {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match kind {
            "userMessage" => {
                self.user_turns += 1;
                Vec::new()
            }
            "agentMessage" => vec![SessionEvent::AssistantText {
                text: item
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parent_id: None,
                injected: false,
            }],
            "reasoning" => {
                let text = join_strings(item.get("content"))
                    .filter(|s| !s.is_empty())
                    .or_else(|| join_strings(item.get("summary")))
                    .unwrap_or_default();
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![SessionEvent::Thinking {
                        text,
                        parent_id: None,
                    }]
                }
            }
            "contextCompaction" => vec![SessionEvent::CompactBoundary {
                trigger: "auto".into(),
                pre_tokens: None,
            }],
            _ => {
                let Some((tool_id, name, input)) = tool_call(item) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                if self.started_tools.insert(tool_id.clone()) {
                    out.push(SessionEvent::ToolUse {
                        id: tool_id.clone(),
                        name,
                        input: input.to_string(),
                        parent_id: None,
                    });
                }
                let (content, is_error) = tool_result(item);
                out.push(SessionEvent::ToolResult {
                    tool_use_id: if tool_id.is_empty() { id } else { tool_id },
                    content,
                    is_error,
                    parent_id: None,
                });
                out
            }
        }
    }
}

fn approval_request_id(p: &Value, rpc_id: &Value) -> String {
    if let Some(approval) = p.get("approvalId").and_then(Value::as_str) {
        return approval.to_string();
    }
    let item = p.get("itemId").and_then(Value::as_str);
    let thread = p.get("threadId").and_then(Value::as_str);
    match (thread, item) {
        // Item ids are per-thread counters; scope them so parallel sub-agent
        // approvals cannot collide in pending_approvals.
        (Some(thread), Some(item)) => format!("{thread}:{item}"),
        (None, Some(item)) => item.to_string(),
        _ => rpc_id.to_string(),
    }
}

fn error_text(p: &Value) -> Option<String> {
    if p.get("willRetry").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    Some(
        p.get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Codex reported an error")
            .to_string(),
    )
}

pub fn approval_reply(kind: &ApprovalKind, allow: bool, message: Option<&str>) -> Value {
    match kind {
        ApprovalKind::CommandExecution | ApprovalKind::FileChange => {
            json!({ "decision": if allow { "accept" } else { "decline" } })
        }
        ApprovalKind::Permissions { requested } => {
            json!({
                "permissions": if allow { requested.clone() } else { json!({}) },
                "scope": "turn",
            })
        }
        ApprovalKind::Legacy => {
            if allow {
                json!({ "decision": "approved" })
            } else {
                json!({ "decision": { "denied": { "rejection": message.unwrap_or("Denied by the user") } } })
            }
        }
    }
}

pub fn question_reply(pending: &PendingQuestion, answers: &Value) -> Value {
    let mut out = Map::new();
    if let Some(obj) = answers.as_object() {
        for (text, chosen) in obj {
            let Some(qid) = pending.by_text.get(text) else {
                continue;
            };
            let labels: Vec<String> = chosen
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_string()])
                .unwrap_or_default();
            out.insert(qid.clone(), json!({ "answers": labels }));
        }
    }
    json!({ "answers": Value::Object(out) })
}

fn plan_events(p: &Value) -> Vec<SessionEvent> {
    let steps = p
        .get("plan")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if steps.is_empty() {
        return Vec::new();
    }
    let todos: Vec<Value> = steps
        .iter()
        .map(|s| {
            json!({
                "content": s.get("step").and_then(Value::as_str).unwrap_or_default(),
                "status": match s.get("status").and_then(Value::as_str).unwrap_or("pending") {
                    "inProgress" => "in_progress",
                    other => other,
                },
            })
        })
        .collect();
    let id = format!(
        "plan-{}",
        p.get("turnId").and_then(Value::as_str).unwrap_or("turn")
    );
    vec![
        SessionEvent::ToolUse {
            id: id.clone(),
            name: "TodoWrite".into(),
            input: json!({ "todos": todos }).to_string(),
            parent_id: None,
        },
        SessionEvent::ToolResult {
            tool_use_id: id,
            content: "Plan updated".into(),
            is_error: false,
            parent_id: None,
        },
    ]
}

fn tool_call(item: &Value) -> Option<(String, String, Value)> {
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    match item.get("type").and_then(Value::as_str)? {
        "commandExecution" => Some((
            id,
            "Bash".into(),
            json!({
                "command": item.get("command").and_then(Value::as_str).unwrap_or_default(),
                "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
            }),
        )),
        "fileChange" => {
            let changes = item.get("changes").and_then(Value::as_array)?;
            Some(file_change_tool(id, changes))
        }
        "mcpToolCall" => {
            let server = item
                .get("server")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or_default();
            Some((
                id,
                format!("mcp__{server}__{tool}"),
                item.get("arguments").cloned().unwrap_or(json!({})),
            ))
        }
        "dynamicToolCall" => Some((
            id,
            item.get("tool")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string(),
            item.get("arguments").cloned().unwrap_or(json!({})),
        )),
        "webSearch" => Some((
            id,
            "WebSearch".into(),
            json!({
                "query": item
                    .get("action")
                    .and_then(|a| a.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            }),
        )),
        "imageView" => Some((
            id,
            "Read".into(),
            json!({ "file_path": item.get("path").cloned().unwrap_or(Value::Null) }),
        )),
        _ => None,
    }
}

fn file_change_tool(id: String, changes: &[Value]) -> (String, String, Value) {
    let path_of = |c: &Value| {
        c.get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let first_path = changes.first().map(path_of).unwrap_or_default();

    if changes.len() == 1 {
        let c = &changes[0];
        let diff = c.get("diff").and_then(Value::as_str).unwrap_or_default();
        let (old, new) = split_unified_diff(diff);
        let kind = c
            .get("kind")
            .and_then(|k| k.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("update");
        if kind == "add" {
            return (
                id,
                "Write".into(),
                json!({ "file_path": first_path, "content": new }),
            );
        }
        return (
            id,
            "Edit".into(),
            json!({ "file_path": first_path, "old_string": old, "new_string": new }),
        );
    }

    let edits: Vec<Value> = changes
        .iter()
        .map(|c| {
            let (old, new) =
                split_unified_diff(c.get("diff").and_then(Value::as_str).unwrap_or_default());
            json!({ "file_path": path_of(c), "old_string": old, "new_string": new })
        })
        .collect();
    (
        id,
        "MultiEdit".into(),
        json!({ "file_path": first_path, "edits": edits }),
    )
}

fn split_unified_diff(diff: &str) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    let hunked = diff.lines().any(|l| l.starts_with("@@"));
    let body = diff.lines().skip_while(|l| hunked && !l.starts_with("@@"));
    for line in body {
        if line.starts_with("@@") {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                new.push_str(&line[1..]);
                new.push('\n');
            }
            Some(b'-') => {
                old.push_str(&line[1..]);
                old.push('\n');
            }
            _ => {
                let body = line.strip_prefix(' ').unwrap_or(line);
                old.push_str(body);
                old.push('\n');
                new.push_str(body);
                new.push('\n');
            }
        }
    }
    (old, new)
}

fn tool_result(item: &Value) -> (String, bool) {
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let failed = matches!(status, "failed" | "declined");
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "commandExecution" => {
            let out = item
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let code = item.get("exitCode").and_then(Value::as_i64);
            let is_error = failed || code.is_some_and(|c| c != 0);
            let text = if out.is_empty() && is_error {
                match code {
                    Some(c) => format!("exited with {c}"),
                    None => "command failed".to_string(),
                }
            } else {
                out
            };
            (text, is_error)
        }
        "fileChange" => {
            let n = item
                .get("changes")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let text = if failed {
                "patch not applied".to_string()
            } else {
                format!("{n} file{} changed", if n == 1 { "" } else { "s" })
            };
            (text, failed)
        }
        "mcpToolCall" => {
            if let Some(err) = item
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
            {
                return (err.to_string(), true);
            }
            (value_to_text(item.get("result")), failed)
        }
        _ => (value_to_text(item.get("result")), failed),
    }
}

fn value_to_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn join_strings(v: Option<&Value>) -> Option<String> {
    let arr = v?.as_array()?;
    Some(
        arr.iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
    )
}

fn join_argv(v: Option<&Value>) -> String {
    match v.and_then(Value::as_array) {
        Some(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev_names(events: &[SessionEvent]) -> Vec<&'static str> {
        events.iter().map(SessionEvent::kind).collect()
    }

    #[test]
    fn decode_distinguishes_response_request_and_notification() {
        assert_eq!(
            decode_line(r#"{"id":1,"result":{"ok":true}}"#),
            Some(Incoming::Response {
                id: 1,
                result: Ok(json!({ "ok": true }))
            })
        );
        assert_eq!(
            decode_line(r#"{"id":2,"error":{"code":-32603,"message":"boom"}}"#),
            Some(Incoming::Response {
                id: 2,
                result: Err("boom".into())
            })
        );
        assert!(matches!(
            decode_line(r#"{"id":"a1","method":"execCommandApproval","params":{}}"#),
            Some(Incoming::ServerRequest { .. })
        ));
        assert!(matches!(
            decode_line(r#"{"method":"turn/started","params":{}}"#),
            Some(Incoming::Notification { .. })
        ));
        assert_eq!(decode_line("listening on stdio"), None);
        assert_eq!(decode_line("   "), None);
    }

    #[test]
    fn notification_thread_filter_uses_the_schema_thread_id() {
        let mut t = Translator::default();
        let child = json!({ "threadId": "child-thread" });

        assert!(
            t.accepts_notification(&child),
            "notifications may arrive while thread/start is still handshaking"
        );

        t.set_root_thread_id("root-thread".into());
        assert!(t.accepts_notification(&json!({ "threadId": "root-thread" })));
        assert!(!t.accepts_notification(&child));
        assert!(
            t.accepts_notification(&json!({ "status": "ready" })),
            "connection-scoped notifications have no threadId"
        );
        assert!(!t.accepts_notification(&json!({ "threadId": null })));
    }

    #[test]
    fn child_thread_notifications_do_not_enter_the_root_transcript() {
        let mut t = Translator {
            active_turn: Some("root-turn".into()),
            requested_turn: Some("root-turn".into()),
            ..Default::default()
        };
        t.set_root_thread_id("root-thread".into());

        let child_notifications = [
            (
                "item/agentMessage/delta",
                json!({ "threadId": "child-thread", "turnId": "child-turn", "itemId": "i1", "delta": "child partial" }),
            ),
            (
                "item/completed",
                json!({ "threadId": "child-thread", "turnId": "child-turn", "item": {
                    "type": "agentMessage", "id": "i1", "text": "child final"
                } }),
            ),
            (
                "thread/tokenUsage/updated",
                json!({ "threadId": "child-thread", "turnId": "child-turn", "tokenUsage": {
                    "last": { "inputTokens": 12 }, "modelContextWindow": 1000
                } }),
            ),
            (
                "turn/completed",
                json!({ "threadId": "child-thread", "turn": {
                    "id": "child-turn", "status": "completed"
                } }),
            ),
        ];

        for (method, params) in child_notifications {
            assert!(
                t.notification(method, &params).is_empty(),
                "{method} leaked from the child thread"
            );
        }
        assert_eq!(t.active_turn.as_deref(), Some("root-turn"));
        assert_eq!(t.requested_turn.as_deref(), Some("root-turn"));

        assert!(t
            .notification(
                "error",
                &json!({ "threadId": "child-thread", "willRetry": true,
                    "error": { "message": "retrying" }
                }),
            )
            .is_empty());
        let child_error = t.notification(
            "error",
            &json!({ "threadId": "child-thread", "turnId": "child-turn", "willRetry": false,
                "error": { "message": "child failed" }
            }),
        );
        assert_eq!(ev_names(&child_error), ["assistantText"]);
        match &child_error[0] {
            SessionEvent::AssistantText { text, .. } => {
                assert!(text.contains("child failed"), "kept the message: {text}");
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }

        let root_text = t.notification(
            "item/completed",
            &json!({ "threadId": "root-thread", "turnId": "root-turn", "item": {
                "type": "agentMessage", "id": "i2", "text": "root final"
            } }),
        );
        assert_eq!(ev_names(&root_text), ["assistantText"]);
        let root_done = t.notification(
            "turn/completed",
            &json!({ "threadId": "root-thread", "turn": {
                "id": "root-turn", "status": "completed"
            } }),
        );
        assert_eq!(ev_names(&root_done), ["turnDone"]);
    }

    #[test]
    fn child_thread_server_requests_remain_routable() {
        let mut t = Translator::default();
        t.set_root_thread_id("root-thread".into());

        let out = t.server_request(
            &json!(21),
            "item/commandExecution/requestApproval",
            &json!({ "threadId": "child-thread", "turnId": "child-turn", "itemId": "child-command",
                "command": "touch child-output", "cwd": "/w"
            }),
        );

        assert_eq!(ev_names(&out), ["permissionAsk"]);
        assert!(t
            .pending_approvals
            .contains_key("child-thread:child-command"));
    }

    #[test]
    fn parallel_child_approvals_with_the_same_item_id_do_not_collide() {
        let mut t = Translator::default();
        t.set_root_thread_id("root-thread".into());

        for (rpc, thread) in [(51, "child-one"), (52, "child-two")] {
            let out = t.server_request(
                &json!(rpc),
                "item/commandExecution/requestApproval",
                &json!({ "threadId": thread, "turnId": "child-turn", "itemId": "item_1",
                    "command": "touch out", "cwd": "/w"
                }),
            );
            assert_eq!(ev_names(&out), ["permissionAsk"]);
        }

        assert_eq!(t.pending_approvals.len(), 2);
        assert!(t.pending_approvals.contains_key("child-one:item_1"));
        assert!(t.pending_approvals.contains_key("child-two:item_1"));
    }

    #[test]
    fn the_read_loop_arms_the_root_filter_from_the_handshake_response() {
        let mut t = Translator::default();
        t.expect_root_thread_from(7);

        t.observe_response(6, &Ok(json!({ "thread": { "id": "not-the-handshake" } })));
        assert!(t.accepts_notification(&json!({ "threadId": "child-thread" })));

        t.observe_response(7, &Ok(json!({ "thread": { "id": "root-thread" } })));
        assert!(!t.accepts_notification(&json!({ "threadId": "child-thread" })));
        assert!(t.accepts_notification(&json!({ "threadId": "root-thread" })));
    }

    #[test]
    fn child_file_change_approval_uses_the_cached_item_then_cleans_it() {
        let mut t = Translator::default();
        t.set_root_thread_id("root-thread".into());

        let started = json!({
            "threadId": "child-thread",
            "turnId": "child-turn",
            "item": {
                "type": "fileChange",
                "id": "child-edit",
                "status": "inProgress",
                "changes": [{
                    "path": "/w/src/lib.rs",
                    "kind": { "type": "update" },
                    "diff": "@@ -1 +1 @@\n-old line\n+new line\n"
                }]
            }
        });
        assert!(t.notification("item/started", &started).is_empty());
        assert_eq!(t.child_file_change_inputs.len(), 1);

        let out = t.server_request(
            &json!(22),
            "item/fileChange/requestApproval",
            &json!({
                "threadId": "child-thread",
                "turnId": "child-turn",
                "itemId": "child-edit",
                "reason": "Apply the requested correction"
            }),
        );
        match &out[0] {
            SessionEvent::PermissionAsk {
                request_id,
                tool_name,
                input,
            } => {
                assert_eq!(request_id, "child-thread:child-edit");
                assert_eq!(tool_name, "Edit");
                let input = parse(input);
                assert_eq!(input["file_path"], json!("/w/src/lib.rs"));
                assert_eq!(input["old_string"], json!("old line\n"));
                assert_eq!(input["new_string"], json!("new line\n"));
                assert_eq!(input["reason"], json!("Apply the requested correction"));
            }
            other => panic!("expected PermissionAsk, got {other:?}"),
        }
        assert!(t.child_file_change_inputs.is_empty());

        assert!(t.notification("item/started", &started).is_empty());
        assert_eq!(t.child_file_change_inputs.len(), 1);
        assert!(t
            .notification(
                "item/completed",
                &json!({
                    "threadId": "child-thread",
                    "turnId": "child-turn",
                    "item": { "type": "fileChange", "id": "child-edit", "changes": [], "status": "declined" }
                }),
            )
            .is_empty());
        assert!(t.child_file_change_inputs.is_empty());

        assert!(t.notification("item/started", &started).is_empty());
        assert!(t
            .notification(
                "turn/completed",
                &json!({
                    "threadId": "child-thread",
                    "turn": { "id": "child-turn", "status": "interrupted" }
                }),
            )
            .is_empty());
        assert!(t.child_file_change_inputs.is_empty());
    }

    #[test]
    fn child_questions_are_not_queued_but_root_questions_are_unchanged() {
        let mut t = Translator::default();
        t.set_root_thread_id("root-thread".into());
        let question = |thread_id: &str, item_id: &str| {
            json!({
                "threadId": thread_id,
                "turnId": format!("{thread_id}-turn"),
                "itemId": item_id,
                "questions": [{
                    "id": format!("{item_id}-choice"),
                    "header": "Choice",
                    "question": "Continue?",
                    "options": []
                }]
            })
        };

        assert!(t
            .server_request(
                &json!(31),
                "item/tool/requestUserInput",
                &question("child-one", "child-question-one"),
            )
            .is_empty());
        assert!(t
            .server_request(
                &json!(32),
                "item/tool/requestUserInput",
                &question("child-two", "child-question-two"),
            )
            .is_empty());
        assert!(t.pending_questions.is_empty());

        let out = t.server_request(
            &json!(33),
            "item/tool/requestUserInput",
            &question("root-thread", "root-question"),
        );
        assert_eq!(ev_names(&out), ["questionAsk"]);
        assert!(t
            .pending_questions
            .contains_key("root-thread:root-question"));
        assert_eq!(t.pending_questions.len(), 1);
    }

    #[test]
    fn agent_message_streams_then_finalizes() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/agentMessage/delta",
            &json!({ "itemId": "i1", "delta": "Hel" }),
        );
        assert_eq!(ev_names(&out), ["partialText"]);

        let out = t.notification(
            "item/completed",
            &json!({ "item": { "type": "agentMessage", "id": "i1", "text": "Hello" } }),
        );
        match &out[0] {
            SessionEvent::AssistantText { text, .. } => assert_eq!(text, "Hello"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn reasoning_never_leaks_as_visible_text() {
        let mut t = Translator::default();
        for method in [
            "item/reasoning/textDelta",
            "item/reasoning/summaryTextDelta",
        ] {
            let out = t.notification(method, &json!({ "itemId": "r1", "delta": "secret plan" }));
            assert!(out.is_empty(), "{method} leaked: {out:?}");
        }
        let out = t.notification(
            "item/completed",
            &json!({ "item": { "type": "reasoning", "id": "r1", "summary": ["a"], "content": ["full thought"] } }),
        );
        assert_eq!(ev_names(&out), ["thinking"]);
        match &out[0] {
            SessionEvent::Thinking { text, .. } => assert_eq!(text, "full thought"),
            other => panic!("expected Thinking, got {other:?}"),
        }
    }

    #[test]
    fn reasoning_falls_back_to_summary() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/completed",
            &json!({ "item": { "type": "reasoning", "id": "r1", "summary": ["thought a", "thought b"], "content": [] } }),
        );
        match &out[0] {
            SessionEvent::Thinking { text, .. } => assert_eq!(text, "thought a\nthought b"),
            other => panic!("expected Thinking, got {other:?}"),
        }
    }

    #[test]
    fn command_execution_maps_to_bash_with_output() {
        let mut t = Translator::default();
        let started = t.notification(
            "item/started",
            &json!({ "item": { "type": "commandExecution", "id": "c1", "command": "ls -la", "cwd": "/w" } }),
        );
        match &started[0] {
            SessionEvent::ToolUse {
                name, input, id, ..
            } => {
                assert_eq!(name, "Bash");
                assert_eq!(id, "c1");
                assert_eq!(parse(input)["command"], json!("ls -la"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }

        let done = t.notification(
            "item/completed",
            &json!({ "item": { "type": "commandExecution", "id": "c1", "command": "ls -la",
                               "status": "completed", "aggregatedOutput": "a.txt", "exitCode": 0 } }),
        );
        assert_eq!(ev_names(&done), ["toolResult"]);
        match &done[0] {
            SessionEvent::ToolResult {
                content,
                is_error,
                tool_use_id,
                ..
            } => {
                assert_eq!(tool_use_id, "c1");
                assert_eq!(content, "a.txt");
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn nonzero_exit_is_an_error_result() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/completed",
            &json!({ "item": { "type": "commandExecution", "id": "c9", "command": "false",
                               "status": "completed", "aggregatedOutput": "", "exitCode": 1 } }),
        );
        assert_eq!(ev_names(&out), ["toolUse", "toolResult"]);
        match &out[1] {
            SessionEvent::ToolResult {
                is_error, content, ..
            } => {
                assert!(is_error);
                assert_eq!(content, "exited with 1");
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn diff_body_lines_that_look_like_headers_are_kept() {
        let (old, new) = split_unified_diff(
            "--- a/x.sql\n+++ b/x.sql\n@@ -1,2 +1,2 @@\n keep\n--- old comment\n+++count;\n",
        );
        assert_eq!(old, "keep\n-- old comment\n");
        assert_eq!(new, "keep\n++count;\n");
    }

    #[test]
    fn a_diff_with_no_hunk_marker_is_all_body() {
        let (old, new) = split_unified_diff("-a\n+b\n");
        assert_eq!(old, "a\n");
        assert_eq!(new, "b\n");
    }

    #[test]
    fn diff_body_lines_that_look_like_headers_survive() {
        let (old, new) = split_unified_diff(
            "--- a/schema.sql\n\
             +++ b/schema.sql\n\
             @@ -1,3 +1,3 @@\n\
             \x20keep\n\
             --- seed data\n\
             +++count;\n",
        );
        assert_eq!(old, "keep\n-- seed data\n");
        assert_eq!(new, "keep\n++count;\n");
    }

    #[test]
    fn a_header_less_diff_is_all_body() {
        let (old, new) = split_unified_diff("-a\n+b\n");
        assert_eq!(old, "a\n");
        assert_eq!(new, "b\n");
    }

    #[test]
    fn file_change_becomes_an_edit_with_reconstructed_strings() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/started",
            &json!({ "item": { "type": "fileChange", "id": "f1", "changes": [
                { "path": "/w/a.rs", "kind": { "type": "update", "move_path": null },
                  "diff": "@@ -1,2 +1,2 @@\n keep\n-old line\n+new line\n" }
            ] } }),
        );
        match &out[0] {
            SessionEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "Edit");
                let v = parse(input);
                assert_eq!(v["file_path"], json!("/w/a.rs"));
                assert_eq!(v["old_string"], json!("keep\nold line\n"));
                assert_eq!(v["new_string"], json!("keep\nnew line\n"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn added_file_becomes_a_write() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/started",
            &json!({ "item": { "type": "fileChange", "id": "f2", "changes": [
                { "path": "/w/new.rs", "kind": { "type": "add" }, "diff": "+fn main() {}\n" }
            ] } }),
        );
        match &out[0] {
            SessionEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "Write");
                assert_eq!(parse(input)["file_path"], json!("/w/new.rs"));
                assert_eq!(parse(input)["content"], json!("fn main() {}\n"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn multi_file_patch_becomes_multiedit() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/started",
            &json!({ "item": { "type": "fileChange", "id": "f3", "changes": [
                { "path": "/w/a.rs", "kind": { "type": "update" }, "diff": "-a\n+b\n" },
                { "path": "/w/b.rs", "kind": { "type": "update" }, "diff": "-c\n+d\n" }
            ] } }),
        );
        match &out[0] {
            SessionEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "MultiEdit");
                let v = parse(input);
                assert_eq!(v["file_path"], json!("/w/a.rs"));
                assert_eq!(v["edits"].as_array().unwrap().len(), 2);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn mcp_tool_uses_the_claude_naming_convention() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/started",
            &json!({ "item": { "type": "mcpToolCall", "id": "m1", "server": "linear",
                               "tool": "create_issue", "arguments": { "title": "x" } } }),
        );
        match &out[0] {
            SessionEvent::ToolUse { name, .. } => assert_eq!(name, "mcp__linear__create_issue"),
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn user_message_is_counted_not_echoed() {
        let mut t = Translator::default();
        let out = t.notification(
            "item/completed",
            &json!({ "item": { "type": "userMessage", "id": "u1", "content": [] } }),
        );
        assert!(out.is_empty());
        assert_eq!(t.user_turns, 1);
    }

    #[test]
    fn plan_update_reuses_the_todowrite_vocabulary() {
        let mut t = Translator::default();
        let out = t.notification(
            "turn/plan/updated",
            &json!({ "turnId": "t1", "plan": [
                { "step": "Read the code", "status": "completed" },
                { "step": "Write the fix", "status": "inProgress" }
            ] }),
        );
        assert_eq!(ev_names(&out), ["toolUse", "toolResult"]);
        match &out[0] {
            SessionEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "TodoWrite");
                let todos = parse(input)["todos"].clone();
                assert_eq!(todos[0]["content"], json!("Read the code"));
                assert_eq!(todos[1]["status"], json!("in_progress"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn token_usage_sends_the_window_once_and_tracks_the_turn() {
        let mut t = Translator::default();
        let p = json!({ "tokenUsage": {
            "total": { "inputTokens": 900, "totalTokens": 1000 },
            "last": { "inputTokens": 120, "totalTokens": 150 },
            "modelContextWindow": 272000
        } });
        let out = t.notification("thread/tokenUsage/updated", &p);
        assert_eq!(ev_names(&out), ["contextWindow", "usage"]);
        match (&out[0], &out[1]) {
            (SessionEvent::ContextWindow { tokens }, SessionEvent::Usage { context_tokens }) => {
                assert_eq!(*tokens, 272_000);
                assert_eq!(*context_tokens, 120);
            }
            other => panic!("unexpected events {other:?}"),
        }
        let out = t.notification("thread/tokenUsage/updated", &p);
        assert_eq!(ev_names(&out), ["usage"]);
    }

    #[test]
    fn turn_lifecycle_tracks_the_active_turn() {
        let mut t = Translator::default();
        t.notification("turn/started", &json!({ "turn": { "id": "t7" } }));
        assert_eq!(t.active_turn.as_deref(), Some("t7"));
        let out = t.notification(
            "turn/completed",
            &json!({ "turn": { "id": "t7", "status": "completed" } }),
        );
        assert_eq!(ev_names(&out), ["turnDone"]);
        assert!(t.active_turn.is_none());
        match &out[0] {
            SessionEvent::TurnDone { turn_status, .. } => {
                assert_eq!(turn_status.as_deref(), Some("completed"))
            }
            other => panic!("expected TurnDone, got {other:?}"),
        }
    }

    #[test]
    fn turn_completion_clears_the_requested_turn_fallback() {
        let mut t = Translator {
            requested_turn: Some("t8".into()),
            ..Default::default()
        };
        t.notification(
            "turn/completed",
            &json!({ "turn": { "id": "t8", "status": "completed" } }),
        );
        assert!(t.requested_turn.is_none());
    }

    #[test]
    fn failed_turn_surfaces_its_error_then_ends() {
        let mut t = Translator::default();
        let out = t.notification(
            "turn/completed",
            &json!({ "turn": { "id": "t8", "status": "failed", "error": { "message": "rate limited" } } }),
        );
        assert_eq!(ev_names(&out), ["assistantText", "turnDone"]);
        match &out[1] {
            SessionEvent::TurnDone { turn_status, .. } => {
                assert_eq!(turn_status.as_deref(), Some("failed"))
            }
            other => panic!("expected TurnDone, got {other:?}"),
        }
    }

    #[test]
    fn a_completed_turn_with_no_status_leaves_turn_status_unset() {
        let mut t = Translator::default();
        let out = t.notification("turn/completed", &json!({ "turn": { "id": "t9" } }));
        match &out[0] {
            SessionEvent::TurnDone { turn_status, .. } => assert_eq!(*turn_status, None),
            other => panic!("expected TurnDone, got {other:?}"),
        }
    }

    #[test]
    fn retryable_errors_are_not_surfaced() {
        let mut t = Translator::default();
        let out = t.notification(
            "error",
            &json!({ "error": { "message": "stream reset" }, "willRetry": true }),
        );
        assert!(out.is_empty());

        let out = t.notification(
            "error",
            &json!({ "error": { "message": "no credit" }, "willRetry": false }),
        );
        assert_eq!(ev_names(&out), ["assistantText", "turnDone"]);
    }

    #[test]
    fn command_approval_becomes_a_permission_card() {
        let mut t = Translator::default();
        let out = t.server_request(
            &json!(11),
            "item/commandExecution/requestApproval",
            &json!({ "itemId": "c5", "command": "rm -rf build", "cwd": "/w", "reason": null }),
        );
        match &out[0] {
            SessionEvent::PermissionAsk {
                request_id,
                tool_name,
                input,
            } => {
                assert_eq!(request_id, "c5");
                assert_eq!(tool_name, "Bash");
                assert_eq!(parse(input)["command"], json!("rm -rf build"));
            }
            other => panic!("expected PermissionAsk, got {other:?}"),
        }
        let (rpc_id, kind) = t.pending_approvals.get("c5").expect("approval is pending");
        assert_eq!(rpc_id, &json!(11));
        assert_eq!(
            approval_reply(kind, true, None),
            json!({ "decision": "accept" })
        );
        assert_eq!(
            approval_reply(kind, false, None),
            json!({ "decision": "decline" })
        );
    }

    #[test]
    fn approval_id_wins_over_item_id_when_present() {
        let mut t = Translator::default();
        t.server_request(
            &json!(3),
            "item/commandExecution/requestApproval",
            &json!({ "itemId": "c1", "approvalId": "cb-9", "command": "ls" }),
        );
        assert!(t.pending_approvals.contains_key("cb-9"));
        assert!(!t.pending_approvals.contains_key("c1"));
    }

    #[test]
    fn legacy_approval_denial_carries_a_rejection() {
        let mut t = Translator::default();
        t.server_request(
            &json!(4),
            "execCommandApproval",
            &json!({ "callId": "call-1", "command": ["git", "push"], "cwd": "/w" }),
        );
        let (_, kind) = t.pending_approvals.get("call-1").expect("pending");
        assert_eq!(
            approval_reply(kind, true, None),
            json!({ "decision": "approved" })
        );
        assert_eq!(
            approval_reply(kind, false, Some("nope")),
            json!({ "decision": { "denied": { "rejection": "nope" } } })
        );
    }

    #[test]
    fn permission_grant_returns_the_requested_profile_or_nothing() {
        let mut t = Translator::default();
        let requested = json!({ "network": { "hosts": ["api.example.com"] } });
        t.server_request(
            &json!(5),
            "item/permissions/requestApproval",
            &json!({ "itemId": "p1", "cwd": "/w", "reason": "fetch", "permissions": requested }),
        );
        let (_, kind) = t.pending_approvals.get("p1").expect("pending");
        assert_eq!(
            approval_reply(kind, true, None),
            json!({ "permissions": requested, "scope": "turn" })
        );
        assert_eq!(
            approval_reply(kind, false, None),
            json!({ "permissions": {}, "scope": "turn" })
        );
    }

    #[test]
    fn user_input_request_round_trips_through_the_question_ui() {
        let mut t = Translator::default();
        let out = t.server_request(
            &json!(7),
            "item/tool/requestUserInput",
            &json!({ "itemId": "q1", "questions": [
                { "id": "qa", "header": "Auth", "question": "Which auth method?",
                  "isOther": false, "isSecret": false,
                  "options": [{ "label": "OAuth", "description": "..." }] }
            ] }),
        );
        match &out[0] {
            SessionEvent::QuestionAsk {
                request_id,
                questions,
            } => {
                assert_eq!(request_id, "q1");
                let qs = parse(questions);
                assert_eq!(qs[0]["question"], json!("Which auth method?"));
                assert_eq!(qs[0]["header"], json!("Auth"));
                assert_eq!(qs[0]["options"][0]["label"], json!("OAuth"));
            }
            other => panic!("expected QuestionAsk, got {other:?}"),
        }
        let pending = t.pending_questions.get("q1").expect("question is pending");
        assert_eq!(
            question_reply(pending, &json!({ "Which auth method?": "OAuth" })),
            json!({ "answers": { "qa": { "answers": ["OAuth"] } } })
        );
    }

    #[test]
    fn an_answer_label_containing_a_comma_round_trips_whole() {
        let mut t = Translator::default();
        t.server_request(
            &json!(8),
            "item/tool/requestUserInput",
            &json!({ "itemId": "q2", "questions": [
                { "id": "qb", "header": "Deploy", "question": "Which action?", "options": [] }
            ] }),
        );
        let pending = t.pending_questions.get("q2").unwrap();
        assert_eq!(
            question_reply(pending, &json!({ "Which action?": "Yes, deploy to prod" })),
            json!({ "answers": { "qb": { "answers": ["Yes, deploy to prod"] } } })
        );
        assert_eq!(
            question_reply(pending, &json!({ "Which action?": "" })),
            json!({ "answers": { "qb": { "answers": [] } } })
        );
    }

    #[test]
    fn permission_modes_map_to_codex_postures() {
        assert_eq!(
            approval_posture(PermissionMode::Default),
            ("untrusted", "workspace-write")
        );
        assert_eq!(
            approval_posture(PermissionMode::AcceptEdits),
            ("never", "workspace-write")
        );
        assert_eq!(
            approval_posture(PermissionMode::Plan),
            ("on-request", "read-only")
        );
        assert_eq!(
            approval_posture(PermissionMode::BypassPermissions),
            ("never", "danger-full-access")
        );
    }

    #[test]
    fn every_posture_uses_a_variant_codex_accepts() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::BypassPermissions,
        ] {
            let (approval, sandbox) = approval_posture(mode);
            assert!(
                VALID_APPROVAL_POLICIES.contains(&approval),
                "{mode:?} → approvalPolicy {approval:?} is not an AskForApproval variant"
            );
            assert!(
                VALID_SANDBOX_MODES.contains(&sandbox),
                "{mode:?} → sandbox {sandbox:?} is not a SandboxMode variant"
            );
        }
    }

    #[test]
    fn unknown_notifications_and_items_are_ignored() {
        let mut t = Translator::default();
        assert!(t.notification("thread/goal/updated", &json!({})).is_empty());
        assert!(t
            .notification(
                "item/completed",
                &json!({ "item": { "type": "sleep", "id": "s1" } })
            )
            .is_empty());
    }

    #[test]
    fn thread_settings_updated_records_the_observed_collab_mode() {
        let mut t = Translator::default();
        let out = t.notification(
            "thread/settings/updated",
            &json!({
                "threadId": "t1",
                "threadSettings": {
                    "model": "gpt-5.6-sol",
                    "collaborationMode": { "mode": "plan", "settings": { "model": "gpt-5.6-sol" } },
                },
            }),
        );
        assert!(out.is_empty());
        assert_eq!(t.collab_mode.as_deref(), Some("plan"));

        t.notification(
            "thread/settings/updated",
            &json!({ "threadSettings": { "collaborationMode": { "mode": "default" } } }),
        );
        assert_eq!(t.collab_mode.as_deref(), Some("default"));
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("event payload is valid JSON")
    }
}
