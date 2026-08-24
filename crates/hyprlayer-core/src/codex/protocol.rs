//! The `codex app-server` v2 wire protocol: JSON-RPC framing plus a stateful
//! translator from Codex notifications to harness-agnostic [`SessionEvent`]s.
//!
//! Shapes are generated from the CLI itself (`codex app-server generate-ts`,
//! codex-cli 0.145.0), regenerate and re-check them whenever
//! [`BinaryKind::Codex`]'s version floor moves. The methods actually handled are
//! the match arms in [`Translator::notification`] and
//! [`Translator::server_request`]; that's the list to read, not a copy of it.
//!
//! The one structural thing worth stating up front: server → client *requests*
//! (`{ method, id, params }`) MUST be answered or the turn blocks forever, which
//! is what makes interactive approvals possible here and impossible under
//! `codex exec`.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::claude::driver::PermissionMode;
use crate::claude::events::SessionEvent;

/// One decoded line of the app-server's stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A reply to a request we sent (`id` + `result`/`error`).
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    /// A server-initiated request we must answer on stdin.
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
    /// A one-way notification.
    Notification { method: String, params: Value },
}

/// Decode one stdout line. Returns `None` for blank lines and anything that
/// isn't a JSON object (the app-server occasionally logs, and a stray line must
/// never be fatal).
pub fn decode_line(line: &str) -> Option<Incoming> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    let obj = v.as_object()?;
    let method = obj.get("method").and_then(Value::as_str);

    match (method, obj.get("id")) {
        // A request from the server: has BOTH a method and an id.
        (Some(m), Some(id)) => Some(Incoming::ServerRequest {
            id: id.clone(),
            method: m.to_string(),
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(m), None) => Some(Incoming::Notification {
            method: m.to_string(),
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        }),
        // A reply to us: an id and no method. Ids we mint are always integers.
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

/// The Codex approval + sandbox posture for one of our permission modes.
///
/// Two independent axes: `approvalPolicy` decides whether Codex *asks*, `sandbox`
/// decides what a command may do without asking. `default` uses `untrusted` so Codex
/// asks before each command, the user then permits the host/network access the
/// sandbox would otherwise deny *silently* (Codex's own escalation is model-driven
/// and misses cases like a bare `ping`). `acceptEdits` never asks (contained to the
/// worktree); `plan` is read-only; only `bypass` drops the sandbox. The allow/deny
/// tool lists have no Codex analog, so this posture is the only guardrail.
///
/// The emitted values are pinned by the `VALID_*` test consts below; anything
/// outside them fails the whole `thread/start` rather than degrading.
pub fn approval_posture(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Default => ("untrusted", "workspace-write"),
        PermissionMode::AcceptEdits => ("never", "workspace-write"),
        PermissionMode::Plan => ("on-request", "read-only"),
        PermissionMode::BypassPermissions => ("never", "danger-full-access"),
    }
}

/// The generated `AskForApproval` / `SandboxMode` unions, minus `granular`,
/// a struct variant we don't emit as a bare string.
#[cfg(test)]
const VALID_APPROVAL_POLICIES: [&str; 3] = ["untrusted", "on-request", "never"];
#[cfg(test)]
const VALID_SANDBOX_MODES: [&str; 3] = ["read-only", "workspace-write", "danger-full-access"];

/// A pending question we surfaced as [`SessionEvent::QuestionAsk`], kept so the
/// user's `{question text → label}` answer map can be turned back into Codex's
/// `{question id → {answers:[label]}}` reply shape.
#[derive(Debug, Clone)]
pub struct PendingQuestion {
    /// JSON-RPC id of the originating server request.
    pub rpc_id: Value,
    /// `question text` → `question id`.
    pub by_text: HashMap<String, String>,
}

/// What the reader task should do with a server request, after the translator
/// has turned it into user-visible events.
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalKind {
    /// v2 command execution: reply `{ decision: "accept" | "decline" }`.
    CommandExecution,
    /// v2 file change: reply `{ decision: "accept" | "decline" }`.
    FileChange,
    /// v2 permission-profile grant: reply with the requested profile, or an
    /// empty one to refuse.
    Permissions { requested: Value },
    /// Legacy v1 exec/patch approval: reply `{ decision: "approved" | {denied} }`.
    Legacy,
}

/// Translates Codex's notification/request stream into [`SessionEvent`]s.
///
/// Stateful because Codex streams an item's content as deltas and only sends the
/// finished payload on `item/completed`, and because answering an approval or a
/// question later needs the JSON-RPC id that carried it. One translator lives for
/// the life of a session.
#[derive(Default)]
pub struct Translator {
    /// The turn currently in flight (`turn/started` → `turn/completed`), needed
    /// to address `turn/interrupt`.
    pub active_turn: Option<String>,
    /// The turn id from `turn/start`'s REPLY, which lands before `turn/started`.
    /// Interrupt falls back to it so Stop in that window still addresses a turn
    /// instead of silently doing nothing.
    pub requested_turn: Option<String>,
    /// Item ids we already emitted a `ToolUse` for, so `item/completed` doesn't
    /// double-announce a tool it also finishes.
    started_tools: HashSet<String>,
    /// `ContextWindow` is the meter's denominator and never changes mid-session;
    /// emit it once.
    context_window_sent: bool,
    /// Approval request id → how to answer it. Populated when we surface a
    /// `PermissionAsk`, drained by `respond_permission`.
    pub pending_approvals: HashMap<String, (Value, ApprovalKind)>,
    /// Question request id → the payload needed to build Codex's reply.
    pub pending_questions: HashMap<String, PendingQuestion>,
    /// How many user turns this thread has seen. `thread/rollback` drops N turns
    /// from the END, so rewinding to the k-th turn needs the total.
    pub user_turns: usize,
}

impl Translator {
    /// A translator for a thread that already has `prior_user_turns` logged.
    /// Rewind ordinals are session-global, so a resumed driver must start its
    /// count there rather than at 0 (see [`crate::codex::SpawnConfig`]).
    pub fn resumed(prior_user_turns: usize) -> Self {
        Self {
            user_turns: prior_user_turns,
            ..Self::default()
        }
    }

    /// Translate one notification. Unknown methods yield no events, Codex's
    /// notification surface is far wider than the transcript renders, and a new
    /// one must never be fatal.
    pub fn notification(&mut self, method: &str, p: &Value) -> Vec<SessionEvent> {
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
                self.active_turn = None;
                self.requested_turn = None;
                // Codex reports token usage on its own `thread/tokenUsage/updated`
                // notification, so the turn boundary carries no counts of its own.
                // Cost isn't reported at all (ChatGPT-plan sessions have no
                // per-turn price), hence `None` rather than a fabricated 0.
                let status = p
                    .get("turn")
                    .and_then(|t| t.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
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
            // Reasoning streams as deltas too, but there is no partial-thinking
            // event in our model (the UI shows a collapsed block). Drop them and
            // surface the whole thought once, on `item/completed`, the same
            // shape the OpenCode driver settled on. Emitting them as PartialText
            // would leak the model's private reasoning into visible prose.
            "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => Vec::new(),
            // Command output streams live but `item/completed` carries the whole
            // `aggregatedOutput`, which is what the ToolResult renders.
            "item/commandExecution/outputDelta" => Vec::new(),
            "item/started" => self.item_started(p.get("item").unwrap_or(&Value::Null)),
            "item/completed" => self.item_completed(p.get("item").unwrap_or(&Value::Null)),
            "turn/plan/updated" => plan_events(p),
            "thread/tokenUsage/updated" => self.token_usage(p),
            "error" => {
                // A retryable error is noise, Codex is about to try again and the
                // turn hasn't ended. Only surface terminal ones.
                if p.get("willRetry").and_then(Value::as_bool) == Some(true) {
                    return Vec::new();
                }
                let msg = p
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Codex reported an error");
                vec![
                    SessionEvent::AssistantText {
                        text: format!("⚠ Codex error: {msg}"),
                        parent_id: None,
                        injected: false,
                    },
                    SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                    },
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Translate a server-initiated request. Returns the events to broadcast;
    /// the reply itself is the reader task's job (it holds stdin), driven off
    /// `pending_approvals` / `pending_questions`.
    pub fn server_request(&mut self, id: &Value, method: &str, p: &Value) -> Vec<SessionEvent> {
        match method {
            "item/commandExecution/requestApproval" => {
                let request_id = approval_request_id(p);
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
                let request_id = approval_request_id(p);
                self.pending_approvals
                    .insert(request_id.clone(), (id.clone(), ApprovalKind::FileChange));
                let mut input = Map::new();
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
                let request_id = approval_request_id(p);
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
            // Legacy v1 approval shapes, still emitted by some code paths.
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
                let request_id = approval_request_id(p);
                let questions = p
                    .get("questions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut by_text = HashMap::new();
                // Re-shape into the `AskUserQuestion` payload the composer's
                // interactive picker already renders (Claude's vocabulary):
                // `[{question, header, multiSelect, options:[{label,description}]}]`.
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
        // The meter's numerator is how full the context is *now*, Codex's `last`
        // breakdown (this turn's input), not the cumulative `total`, which counts
        // every token ever sent and would run the meter past 100%.
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
            // We emit `UserMessage` ourselves in `send_user_turn` (so the turn is
            // persisted and replays); echoing Codex's copy would double it.
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
                // `content` is the full chain when the model exposes it; `summary`
                // is the redacted version most models return. Prefer content.
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
                // A tool that completed without ever starting (a fast call, or a
                // replayed item) still needs its card.
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

/// The id an approval/question request is answered by. Codex sends a distinct
/// `approvalId` only when several callbacks share one item (the zsh-exec bridge);
/// otherwise the item id addresses it.
fn approval_request_id(p: &Value) -> String {
    p.get("approvalId")
        .and_then(Value::as_str)
        .or_else(|| p.get("itemId").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// Build the Codex reply body for an answered permission prompt.
pub fn approval_reply(kind: &ApprovalKind, allow: bool, message: Option<&str>) -> Value {
    match kind {
        ApprovalKind::CommandExecution | ApprovalKind::FileChange => {
            json!({ "decision": if allow { "accept" } else { "decline" } })
        }
        ApprovalKind::Permissions { requested } => {
            // Granting hands back exactly what was asked for, scoped to this turn;
            // refusing hands back an empty profile (Codex's "nothing granted").
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

/// Build the Codex reply for an answered question. The UI answers
/// `{question text → "label"}` (multi-select labels comma-joined); Codex wants
/// `{question id → {answers:[label,…]}}`.
pub fn question_reply(pending: &PendingQuestion, answers: &Value) -> Value {
    let mut out = Map::new();
    if let Some(obj) = answers.as_object() {
        for (text, chosen) in obj {
            let Some(qid) = pending.by_text.get(text) else {
                continue;
            };
            // One answer, taken whole. We surface Codex questions with
            // `multiSelect: false`, so the UI never comma-joins picks here,
            // and splitting on ", " would shatter any label that contains one
            // ("Yes, deploy to prod") into two ids Codex doesn't recognize.
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

/// `turn/plan/updated` → the `TodoWrite` pair the transcript's plan panel reads.
/// Codex's plan is a whole-list snapshot, exactly like TodoWrite's `todos`, so
/// reusing that vocabulary means the existing plan UI works with no frontend
/// change (the same trick `normalize_tool` plays for OpenCode's file tools).
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
    // The pair shares one id so the transcript folds them into a single row. The
    // turn id keeps it stable across the many updates within one turn.
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

/// A Codex item rendered as a tool call, mapped onto the Claude-Code tool
/// vocabulary: `(id, name, input)`, or `None` for items that aren't tools.
///
/// The normalization is load-bearing, for the same reason it is in the OpenCode
/// driver: the transcript UI keys every display path, per-file chips, the
/// edit-diff preview, agent-doc detection, argument summaries, on Claude's
/// PascalCase names and snake_case keys (see `src/features/session/toolDisplay.ts`).
/// Without it, Codex's shell and patch calls fall through to the generic renderer
/// and collapse into an anonymous "N tool calls" group.
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

/// A `fileChange` item as an `Edit` / `Write` / `MultiEdit` call.
///
/// Codex describes an edit as a unified diff; the UI's inline diff preview wants
/// Claude's `old_string`/`new_string`. Reconstructing the two sides from the hunk
/// (context + `-` lines = old, context + `+` lines = new) is exactly what the
/// preview then re-diffs, so Codex patches render as real diffs rather than raw
/// patch text.
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

    // Several files in one patch. `MultiEdit` stacks its `edits` in the preview
    // (separated by an `@@` marker), which is the closest existing rendering.
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

/// Split a unified diff body into its before/after sides. Hunk headers are
/// dropped; a line with no marker is context and belongs to both.
fn split_unified_diff(diff: &str) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    // Header detection is POSITIONAL, not prefix-based. A body line may
    // legitimately start with `---` (removing a SQL comment or a `---`
    // frontmatter delimiter) or `+++` (adding `++count;`), and matching those
    // anywhere silently drops them. Codex also sends bare bodies with no `@@`
    // at all (single-hunk edits, added files), so "everything before the first
    // hunk marker is header" only applies when there IS one.
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

/// The result text + error flag for a completed tool item.
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

/// Best-effort string form of an arbitrary result value.
fn value_to_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Join a `string[]` field into one block, or `None` when absent.
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

/// The legacy approval shape sends argv; the UI shows a command line.
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
        // A reply to us: id, no method.
        assert_eq!(
            decode_line(r#"{"id":1,"result":{"ok":true}}"#),
            Some(Incoming::Response {
                id: 1,
                result: Ok(json!({ "ok": true }))
            })
        );
        // An error reply surfaces its message.
        assert_eq!(
            decode_line(r#"{"id":2,"error":{"code":-32603,"message":"boom"}}"#),
            Some(Incoming::Response {
                id: 2,
                result: Err("boom".into())
            })
        );
        // A server request: BOTH method and id, must not be read as a response.
        assert!(matches!(
            decode_line(r#"{"id":"a1","method":"execCommandApproval","params":{}}"#),
            Some(Incoming::ServerRequest { .. })
        ));
        // A notification: method, no id.
        assert!(matches!(
            decode_line(r#"{"method":"turn/started","params":{}}"#),
            Some(Incoming::Notification { .. })
        ));
        // Stray log lines are never fatal.
        assert_eq!(decode_line("listening on stdio"), None);
        assert_eq!(decode_line("   "), None);
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
        // Codex streams reasoning as `field`-less text deltas on their own
        // notification. Emitting those as PartialText would print the model's
        // private chain-of-thought into the transcript as prose, the same
        // regression the OpenCode translator guards against.
        let mut t = Translator::default();
        for method in [
            "item/reasoning/textDelta",
            "item/reasoning/summaryTextDelta",
        ] {
            let out = t.notification(method, &json!({ "itemId": "r1", "delta": "secret plan" }));
            assert!(out.is_empty(), "{method} leaked: {out:?}");
        }
        // The completed item surfaces it exactly once, as a collapsed block.
        let out = t.notification(
            "item/completed",
            &json!({ "item": { "type": "reasoning", "id": "r1", "summary": ["a"], "content": ["full thought"] } }),
        );
        assert_eq!(ev_names(&out), ["thinking"]);
        match &out[0] {
            // `content` wins over the redacted `summary` when present.
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
                // `command` is what toolSummary("Bash") renders.
                assert_eq!(parse(input)["command"], json!("ls -la"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }

        // Completion must NOT re-announce the tool, only close it.
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
        // Never started → the card is emitted alongside its result.
        assert_eq!(ev_names(&out), ["toolUse", "toolResult"]);
        match &out[1] {
            SessionEvent::ToolResult {
                is_error, content, ..
            } => {
                assert!(is_error);
                // Empty output would render as "(no output)", say why it failed.
                assert_eq!(content, "exited with 1");
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    /// Header detection has to be positional. A body line may legitimately start
    /// with `---` (a removed SQL comment or `---` frontmatter delimiter) or `+++`
    /// (adding `++count;`), prefix-matching those drops them from the
    /// reconstructed side, so the transcript's inline diff shows an edit that
    /// never happened and hides one that did.
    #[test]
    fn diff_body_lines_that_look_like_headers_are_kept() {
        let (old, new) = split_unified_diff(
            "--- a/x.sql\n+++ b/x.sql\n@@ -1,2 +1,2 @@\n keep\n--- old comment\n+++count;\n",
        );
        assert_eq!(old, "keep\n-- old comment\n");
        assert_eq!(new, "keep\n++count;\n");
    }

    /// Codex also sends bare bodies with no `@@` at all (single-hunk edits, added
    /// files), so "skip to the first hunk" must not eat the whole diff.
    #[test]
    fn a_diff_with_no_hunk_marker_is_all_body() {
        let (old, new) = split_unified_diff("-a\n+b\n");
        assert_eq!(old, "a\n");
        assert_eq!(new, "b\n");
    }

    /// Header detection has to be positional. Prefix-matching `---`/`+++` on
    /// every line drops real content, a removed SQL comment or `---` frontmatter
    /// delimiter, an added `++count;`, so the inline diff would show a change
    /// the user never made and hide one they did.
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

    /// Codex also sends bare bodies with no `@@` at all, so the positional rule
    /// must not swallow a whole header-less diff.
    #[test]
    fn a_header_less_diff_is_all_body() {
        let (old, new) = split_unified_diff("-a\n+b\n");
        assert_eq!(old, "a\n");
        assert_eq!(new, "b\n");
    }

    #[test]
    fn file_change_becomes_an_edit_with_reconstructed_strings() {
        // The UI's inline diff re-diffs old_string vs new_string, so a Codex
        // unified diff has to be split back into its two sides.
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
        // The driver emits UserMessage itself so the turn persists; echoing
        // Codex's copy would double-stamp the transcript. The count is what
        // `thread/rollback` (which drops N turns from the end) needs.
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
                // planTasks.ts keys on snake_case `in_progress`, not Codex's camel.
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
                // The meter's numerator is this turn's context fill, not the
                // cumulative total (which would run it past 100%).
                assert_eq!(*context_tokens, 120);
            }
            other => panic!("unexpected events {other:?}"),
        }
        // The denominator never changes, don't re-announce it.
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
    }

    /// `turn/completed` must clear the id recorded from `turn/start`'s reply too,
    /// or a Stop between turns would address a turn that already finished.
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
    }

    #[test]
    fn retryable_errors_are_not_surfaced() {
        // Codex retries transient failures itself; a banner per retry would read
        // as the turn dying when it hasn't.
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
        // Several callbacks can share one itemId (the zsh-exec bridge); the
        // distinct approvalId is what routes the answer back.
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
                // Reshaped into the AskUserQuestion payload the composer renders.
                assert_eq!(qs[0]["question"], json!("Which auth method?"));
                assert_eq!(qs[0]["header"], json!("Auth"));
                assert_eq!(qs[0]["options"][0]["label"], json!("OAuth"));
            }
            other => panic!("expected QuestionAsk, got {other:?}"),
        }
        // The UI answers by question TEXT; Codex is addressed by question ID.
        let pending = t.pending_questions.get("q1").expect("question is pending");
        assert_eq!(
            question_reply(pending, &json!({ "Which auth method?": "OAuth" })),
            json!({ "answers": { "qa": { "answers": ["OAuth"] } } })
        );
    }

    /// A label is one answer, taken whole. We surface Codex questions with
    /// `multiSelect: false`, so the UI never comma-joins picks, and splitting on
    /// ", " would shatter a legitimate label into ids Codex doesn't recognize,
    /// either wedging the turn or silently picking a different branch.
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
        // A dismissed question yields no answer rather than an empty string.
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
        // Plan is "look, don't touch": read-only sandbox.
        assert_eq!(
            approval_posture(PermissionMode::Plan),
            ("on-request", "read-only")
        );
        // Default asks before commands (untrusted); only Bypass drops the sandbox.
        assert_eq!(
            approval_posture(PermissionMode::BypassPermissions),
            ("never", "danger-full-access")
        );
    }

    /// A bad mapping doesn't degrade. It makes that permission mode
    /// unlaunchable, since `thread/start` rejects the whole request.
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
        // Codex's notification surface is far wider than we render; a new one
        // must never be fatal or produce a phantom event.
        let mut t = Translator::default();
        assert!(t.notification("thread/goal/updated", &json!({})).is_empty());
        assert!(t
            .notification(
                "item/completed",
                &json!({ "item": { "type": "sleep", "id": "s1" } })
            )
            .is_empty());
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("event payload is valid JSON")
    }
}
