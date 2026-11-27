//! Port of `src/features/session/transcript.ts`: one session's items and status, folded from its events.
//! Mutates in place; `reduce_event` returns whether the TS reducer would have returned a new view.

use std::collections::HashSet;

use hyprlayer_core::models::session::{
    ClientRef, ExitOutcome, Question, QuestionOption, QuestionSubject, RelaunchReason, TurnStatus,
};
use hyprlayer_core::{SessionEvent, SessionState};
use serde::Serialize;

use super::text::{
    compact_summary_body, is_interrupt_notice, js_trim, native_continuation, skill_body_name,
    split_inline_thinking, NativeKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Deny,
    // The ask died with its harness process, so no answer can reach it.
    Expired,
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attachment {
    pub path: String,
    pub name: String,
    pub kind: AttachmentKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub request_id: String,
    pub questions: Vec<Question>,
    pub subject: Option<QuestionSubject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPermission {
    pub request_id: String,
    pub tool_name: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ItemKind {
    User {
        text: String,
        stage: Option<String>,
        attachments: Vec<Attachment>,
        created_at: Option<f64>,
        prompt_name: Option<String>,
        client_message_id: Option<String>,
        author: Option<ClientRef>,
    },
    Assistant {
        text: String,
        streaming: bool,
        #[serde(skip_serializing_if = "is_false")]
        auth_required: bool,
        parent_id: Option<String>,
    },
    Thinking {
        text: String,
        parent_id: Option<String>,
    },
    Skill {
        name: String,
        body: String,
        parent_id: Option<String>,
    },
    Tool {
        tool_use_id: String,
        name: String,
        input: String,
        result: Option<ToolResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        skill_body: Option<String>,
        parent_id: Option<String>,
    },
    Permission {
        request_id: String,
        tool_name: String,
        input: String,
        tool_use_id: Option<String>,
        decision: Option<PermissionDecision>,
    },
    Error {
        detail: String,
        raw: String,
    },
    Compact {
        trigger: String,
        pre_tokens: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    QueuedCommand {
        command_uuid: String,
        label: String,
        detail: Option<String>,
    },
    TaskUpdate {
        task_id: String,
        tool_use_id: Option<String>,
        status: String,
    },
    NativeContinuation {
        reason: String,
        native: NativeKind,
    },
    Relaunched {
        quiet: bool,
        reason: Option<RelaunchReason>,
    },
    Rewound,
    Exit {
        outcome: ExitOutcome,
        code: Option<i32>,
        stderr_tail: String,
    },
    Interrupted,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl Serialize for NativeKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            NativeKind::Hook => "hook",
            NativeKind::Goal => "goal",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptItem {
    pub id: String,
    /// The log row the item came from; absent for live-only and optimistic items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    #[serde(flatten)]
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub items: Vec<TranscriptItem>,
    pub counter: u64,
    pub state: SessionState,
    pub streaming: bool,
    pub cost_usd: Option<f64>,
    pub num_turns: Option<u32>,
    pub context_tokens: Option<u32>,
    pub context_window: Option<u32>,
    pub model: Option<String>,
    #[serde(rename = "claudeSessionId")]
    pub provider_session_id: Option<String>,
    pub mcp_servers: Vec<String>,
    pub pending_permission: Option<PendingPermission>,
    pub pending_question: Option<PendingQuestion>,
    /// Asked while `pending_question` was open; shown in turn.
    pub queued_questions: Vec<PendingQuestion>,
}

impl SessionView {
    pub fn new(state: SessionState) -> Self {
        Self {
            items: Vec::new(),
            counter: 0,
            state,
            streaming: false,
            cost_usd: None,
            num_turns: None,
            context_tokens: None,
            context_window: None,
            model: None,
            provider_session_id: None,
            mcp_servers: Vec::new(),
            pending_permission: None,
            pending_question: None,
            queued_questions: Vec::new(),
        }
    }
}

pub fn is_terminal(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Completed
            | SessionState::Interrupted
            | SessionState::Crashed
            | SessionState::Killed
            | SessionState::SpawnFailed
    )
}

pub fn has_pending_permission(view: &SessionView) -> bool {
    view.items
        .iter()
        .any(|it| matches!(it.kind, ItemKind::Permission { decision: None, .. }))
}

pub fn parked_state(view: &SessionView) -> SessionState {
    if is_terminal(view.state) {
        view.state
    } else if view.pending_question.is_some() {
        SessionState::AwaitingQuestion
    } else if has_pending_permission(view) {
        SessionState::AwaitingPermission
    } else {
        SessionState::Running
    }
}

pub fn restored_view(view: SessionView, live: bool) -> SessionView {
    if is_terminal(view.state) {
        return SessionView {
            streaming: false,
            ..view
        };
    }
    let mut settled = if live {
        view
    } else {
        expire_pending_prompts(view)
    };
    if settled.pending_question.is_some() || has_pending_permission(&settled) {
        settled.state = parked_state(&settled);
    } else {
        settled.state = SessionState::Idle;
        settled.pending_permission = None;
        settled.pending_question = None;
        settled.queued_questions.clear();
    }
    settled.streaming = false;
    settled
}

// JS truthiness of an optional string id.
fn truthy(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.is_empty())
}

pub fn parse_questions(questions: &[Question]) -> Vec<Question> {
    questions
        .iter()
        .filter(|q| !q.question.is_empty())
        .map(|q| Question {
            options: q
                .options
                .iter()
                .filter(|o| !o.label.is_empty())
                .map(|o| QuestionOption {
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect(),
            ..q.clone()
        })
        .collect()
}

fn open_questions(view: &SessionView) -> Vec<PendingQuestion> {
    view.pending_question
        .iter()
        .chain(view.queued_questions.iter())
        .cloned()
        .collect()
}

fn set_questions(view: &mut SessionView, questions: Vec<PendingQuestion>) {
    let mut it = questions.into_iter();
    view.pending_question = it.next();
    view.queued_questions = it.collect();
}

// Claude asks after streaming the call, so the oldest unanswered AskUserQuestion no open question claims is this one.
fn question_tool_use(view: &SessionView) -> Option<String> {
    let claimed: HashSet<&str> = view
        .pending_question
        .iter()
        .chain(view.queued_questions.iter())
        .filter_map(|q| q.tool_use_id.as_deref())
        .collect();
    view.items.iter().find_map(|it| match &it.kind {
        ItemKind::Tool {
            name,
            result: None,
            tool_use_id,
            ..
        } if name == "AskUserQuestion" && !claimed.contains(tool_use_id.as_str()) => {
            Some(tool_use_id.clone())
        }
        _ => None,
    })
}

// Its card's decision was local only; a restore would offer it again, and turnDone would park on it.
fn settle_subagent_ask(view: &mut SessionView, tool_use_id: &str) {
    let Some(i) = view.items.iter().position(|it| {
        matches!(&it.kind, ItemKind::Permission { decision: None, tool_use_id: Some(t), .. } if t == tool_use_id)
    }) else {
        return;
    };
    let ItemKind::Permission {
        request_id,
        decision,
        ..
    } = &mut view.items[i].kind
    else {
        return;
    };
    *decision = Some(PermissionDecision::Allow);
    let request_id = request_id.clone();
    if view
        .pending_permission
        .as_ref()
        .is_some_and(|p| p.request_id == request_id)
    {
        view.pending_permission = None;
    }
    view.state = parked_state(view);
}

fn settle_answered_prompt(view: &mut SessionView, tool_name: &str, tool_use_id: &str) {
    let mut questions = open_questions(view);
    let mut answered = false;

    if tool_name == "AskUserQuestion" {
        let own = questions
            .iter()
            .position(|q| q.tool_use_id.as_deref() == Some(tool_use_id));
        // A question from an older fold claims no call; the one on screen is the best guess.
        let at = own.or_else(|| questions.iter().position(|q| q.tool_use_id.is_none()));
        if let Some(at) = at {
            questions.remove(at);
            answered = true;
        }
    } else {
        for it in view.items.iter_mut().rev() {
            let ItemKind::Permission {
                request_id,
                tool_name: ask_tool,
                tool_use_id: ask_use,
                decision,
                ..
            } = &mut it.kind
            else {
                continue;
            };
            if decision.is_some() {
                continue;
            }
            // Asks from older servers carry no tool use id; the name is all they have.
            let mismatch = if truthy(ask_use) {
                ask_use.as_deref() != Some(tool_use_id)
            } else {
                ask_tool != tool_name
            };
            if mismatch {
                continue;
            }
            *decision = Some(PermissionDecision::Allow);
            if view
                .pending_permission
                .as_ref()
                .is_some_and(|p| &p.request_id == request_id)
            {
                view.pending_permission = None;
            }
            answered = true;
            break;
        }
    }

    set_questions(view, questions);
    // A result after turnDone is a stopped call settling; only an answered prompt resumes the turn.
    view.streaming = view.streaming || answered;
    view.state = parked_state(view);
}

fn open_assistant_index(items: &[TranscriptItem]) -> Option<usize> {
    items.iter().rposition(|it| {
        matches!(&it.kind, ItemKind::Assistant { streaming: true, parent_id, .. } if !truthy(parent_id))
    })
}

fn streamed_assistant_index(items: &[TranscriptItem], text: &str) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    for (i, it) in items.iter().enumerate().rev() {
        match &it.kind {
            ItemKind::Tool { .. } | ItemKind::Thinking { .. } => continue,
            ItemKind::Assistant {
                text: held,
                parent_id,
                ..
            } if !truthy(parent_id) && !held.is_empty() && text.starts_with(held.as_str()) => {
                return Some(i)
            }
            _ => return None,
        }
    }
    None
}

const SKILL_LAUNCH_RESULT: &str = "Launching skill:";

fn first_skill_tool_awaiting_body(items: &[TranscriptItem], parent: Option<&str>) -> Option<usize> {
    items.iter().position(|it| {
        matches!(&it.kind, ItemKind::Tool { name, result: Some(r), skill_body: None, parent_id, .. }
            if name == "Skill" && !r.is_error && r.content.starts_with(SKILL_LAUNCH_RESULT) && parent_id.as_deref() == parent)
    })
}

struct Seam {
    label: &'static str,
    detail: Option<String>,
}

fn queued_command_seam(items: &[TranscriptItem]) -> Seam {
    let generic = Seam {
        label: "Queued command",
        detail: None,
    };
    let Some((name, input)) = items.iter().rev().find_map(|it| match &it.kind {
        ItemKind::Tool { name, input, .. } => Some((name, input)),
        _ => None,
    }) else {
        return generic;
    };
    if name != "ScheduleWakeup" {
        return generic;
    }
    let parsed = serde_json::from_str::<serde_json::Value>(input).ok();
    let obj = parsed.as_ref().and_then(|v| v.as_object());
    let reason = obj.and_then(|o| {
        o.get("reason")
            .filter(|v| !v.is_null())
            .or_else(|| o.get("prompt"))
    });
    let detail = reason
        .and_then(|v| v.as_str())
        .map(js_trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Seam {
        label: "Scheduled wakeup",
        detail,
    }
}

fn close_open_assistant(items: &mut [TranscriptItem]) {
    if let Some(i) = open_assistant_index(items) {
        if let ItemKind::Assistant { streaming, .. } = &mut items[i].kind {
            *streaming = false;
        }
    }
}

fn append(view: &mut SessionView, kind: ItemKind, close_assistant: bool, seq: Option<i64>) {
    if close_assistant {
        close_open_assistant(&mut view.items);
    }
    view.items.push(TranscriptItem {
        id: format!("i{}", view.counter),
        seq,
        kind,
    });
    view.counter += 1;
}

fn is_subagent_tool(items: &[TranscriptItem], tool_use_id: Option<&str>) -> bool {
    tool_use_id.is_some_and(|id| {
        !id.is_empty()
            && items.iter().any(|it| {
                matches!(&it.kind, ItemKind::Tool { tool_use_id: t, parent_id, .. } if t == id && truthy(parent_id))
            })
    })
}

fn event_parent(event: &SessionEvent) -> Option<&str> {
    match event {
        SessionEvent::PartialText { parent_id, .. }
        | SessionEvent::AssistantText { parent_id, .. }
        | SessionEvent::Thinking { parent_id, .. }
        | SessionEvent::ToolUse { parent_id, .. }
        | SessionEvent::ToolResult { parent_id, .. } => {
            parent_id.as_deref().filter(|p| !p.is_empty())
        }
        _ => None,
    }
}

// The text may have merged into a streamed block, so the flag goes on whichever item holds it now.
fn flag_auth_text(view: &mut SessionView) -> bool {
    for it in view.items.iter_mut().rev() {
        if let ItemKind::Assistant {
            parent_id,
            auth_required,
            ..
        } = &mut it.kind
        {
            if !truthy(parent_id) {
                *auth_required = true;
                return true;
            }
        }
    }
    false
}

/// Folds one event into the view; `true` when the TS reducer returned a new view object.
pub fn reduce_event(view: &mut SessionView, event: &SessionEvent, seq: Option<i64>) -> bool {
    let was_streaming = view.streaming;
    let mut changed = reduce_inner(view, event, seq);
    // A background subagent's events keep arriving after the launching turn's
    // own turnDone; letting one raise `streaming` pins the session at "working".
    if event_parent(event).is_some() && view.streaming && !was_streaming {
        view.streaming = false;
    }
    if let SessionEvent::AssistantText {
        auth_required: true,
        parent_id,
        ..
    } = event
    {
        if !truthy(parent_id) && flag_auth_text(view) {
            changed = true;
        }
    }
    if view.state == SessionState::Starting && view.streaming {
        view.state = SessionState::Running;
        changed = true;
    }
    changed
}

fn reduce_inner(view: &mut SessionView, event: &SessionEvent, seq: Option<i64>) -> bool {
    match event {
        SessionEvent::Init {
            provider_session_id,
            model,
            mcp_servers,
        } => {
            // A new id is a new conversation (e.g. /clear): its fill is unknown until reported.
            let fresh = view
                .provider_session_id
                .as_deref()
                .is_some_and(|v| !v.is_empty())
                && !provider_session_id.is_empty()
                && view.provider_session_id.as_deref() != Some(provider_session_id.as_str());
            view.state = SessionState::Running;
            if !provider_session_id.is_empty() {
                view.provider_session_id = Some(provider_session_id.clone());
            }
            let new_model = model.as_deref().filter(|m| !m.is_empty());
            if new_model.is_some() && model != &view.model {
                view.context_window = None;
            }
            if model.is_some() {
                view.model = model.clone();
            }
            if let Some(servers) = mcp_servers {
                view.mcp_servers = servers.clone();
            }
            if fresh {
                view.context_tokens = None;
            }
            true
        }

        SessionEvent::UserMessage {
            text,
            stage,
            prompt_name,
            client_message_id,
            author,
        } => {
            // The sender already shows this turn optimistically; only its own echo is dropped.
            let id = client_message_id.as_deref().filter(|i| !i.is_empty());
            if let Some(id) = id {
                let echoed = view.items.iter().any(
                    |it| matches!(&it.kind, ItemKind::User { client_message_id: Some(c), .. } if c == id),
                );
                if echoed {
                    // A retry replays its stored first turn with no optimistic add; the echo is when it starts.
                    let changed = !view.streaming;
                    view.streaming = true;
                    return changed;
                }
            }
            append(
                view,
                ItemKind::User {
                    text: text.clone(),
                    stage: stage.clone(),
                    attachments: Vec::new(),
                    created_at: None,
                    prompt_name: prompt_name.clone(),
                    client_message_id: client_message_id.clone(),
                    author: author.clone(),
                },
                true,
                seq,
            );
            view.streaming = true;
            true
        }

        SessionEvent::PartialText { text, parent_id } => {
            if truthy(parent_id) || text.is_empty() {
                return false;
            }
            if let Some(i) = open_assistant_index(&view.items) {
                if let ItemKind::Assistant { text: held, .. } = &mut view.items[i].kind {
                    held.push_str(text);
                }
            } else {
                append(
                    view,
                    ItemKind::Assistant {
                        text: text.clone(),
                        streaming: true,
                        auth_required: false,
                        parent_id: None,
                    },
                    false,
                    seq,
                );
            }
            view.streaming = true;
            true
        }

        SessionEvent::AssistantText {
            text,
            parent_id,
            injected,
            ..
        } => reduce_assistant_text(view, text, parent_id, *injected, seq),

        SessionEvent::Thinking { text, parent_id } => {
            if text.is_empty() {
                return false;
            }
            append(
                view,
                ItemKind::Thinking {
                    text: text.clone(),
                    parent_id: parent_id.clone(),
                },
                !truthy(parent_id),
                seq,
            );
            view.streaming = true;
            true
        }

        SessionEvent::ToolUse {
            id,
            name,
            input,
            parent_id,
        } => {
            append(
                view,
                ItemKind::Tool {
                    tool_use_id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    result: None,
                    skill_body: None,
                    parent_id: parent_id.clone(),
                },
                !truthy(parent_id),
                seq,
            );
            view.streaming = true;
            true
        }

        SessionEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
            parent_id,
        } => {
            let hit = view.items.iter().rposition(|it| {
                matches!(&it.kind, ItemKind::Tool { tool_use_id: t, result: None, .. } if t == tool_use_id)
            });
            let Some(i) = hit else {
                append(
                    view,
                    ItemKind::Tool {
                        tool_use_id: tool_use_id.clone(),
                        name: "result".into(),
                        input: String::new(),
                        result: Some(ToolResult {
                            content: content.clone(),
                            is_error: *is_error,
                        }),
                        skill_body: None,
                        parent_id: parent_id.clone(),
                    },
                    false,
                    seq,
                );
                view.streaming = true;
                return true;
            };
            let ItemKind::Tool { result, name, .. } = &mut view.items[i].kind else {
                return true;
            };
            *result = Some(ToolResult {
                content: content.clone(),
                is_error: *is_error,
            });
            let name = name.clone();
            if truthy(parent_id) {
                settle_subagent_ask(view, tool_use_id);
            } else {
                settle_answered_prompt(view, &name, tool_use_id);
            }
            true
        }

        SessionEvent::PermissionAsk {
            request_id,
            tool_name,
            input,
            tool_use_id,
        } => {
            let subagent = is_subagent_tool(&view.items, tool_use_id.as_deref());
            append(
                view,
                ItemKind::Permission {
                    request_id: request_id.clone(),
                    tool_name: tool_name.clone(),
                    input: input.clone(),
                    tool_use_id: tool_use_id.clone(),
                    decision: None,
                },
                true,
                seq,
            );
            view.state = SessionState::AwaitingPermission;
            view.streaming = view.streaming || !subagent;
            view.pending_permission = Some(PendingPermission {
                request_id: request_id.clone(),
                tool_name: tool_name.clone(),
                input: input.clone(),
            });
            true
        }

        SessionEvent::QuestionAsk {
            request_id,
            questions,
            subject,
        } => {
            let asked = PendingQuestion {
                request_id: request_id.clone(),
                questions: parse_questions(questions),
                subject: subject.clone(),
                tool_use_id: question_tool_use(view),
            };
            let mut all: Vec<PendingQuestion> = open_questions(view)
                .into_iter()
                .filter(|q| &q.request_id != request_id)
                .collect();
            all.push(asked);
            set_questions(view, all);
            close_open_assistant(&mut view.items);
            view.state = SessionState::AwaitingQuestion;
            view.streaming = true;
            true
        }

        SessionEvent::PermissionResolved {
            request_id,
            allow,
            withdrawn,
        } => {
            let decision = if *withdrawn {
                PermissionDecision::Withdrawn
            } else if *allow {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Deny
            };
            close_ask(view, request_id, decision);
            true
        }

        SessionEvent::QuestionAnswered { request_id } => close_question(view, request_id),

        SessionEvent::Usage { context_tokens, .. } => {
            view.context_tokens = Some(*context_tokens);
            true
        }

        SessionEvent::ContextWindow { tokens } => {
            view.context_window = Some(*tokens);
            true
        }

        SessionEvent::TurnDone {
            cost_usd,
            num_turns,
            context_tokens,
            turn_status,
            ..
        } => {
            settle_reload(&mut view.items);
            close_open_assistant(&mut view.items);
            view.state = parked_state(view);
            view.streaming = false;
            view.cost_usd = cost_usd.or(view.cost_usd);
            view.num_turns = num_turns.or(view.num_turns);
            view.context_tokens = context_tokens.or(view.context_tokens);
            if turn_status.as_deref().map(TurnStatus::parse) == Some(TurnStatus::Interrupted) {
                append(view, ItemKind::Interrupted, false, seq);
            }
            true
        }

        SessionEvent::Exited {
            outcome,
            code,
            stderr_tail,
        } => {
            close_open_assistant(&mut view.items);
            expire_pending_in_place(view);
            if *outcome == ExitOutcome::Detached {
                view.streaming = false;
                view.state = SessionState::Idle;
                return true;
            }
            append(
                view,
                ItemKind::Exit {
                    outcome: *outcome,
                    code: *code,
                    stderr_tail: stderr_tail.clone(),
                },
                true,
                seq,
            );
            view.state = outcome.state();
            view.streaming = false;
            view.pending_permission = None;
            view.pending_question = None;
            view.queued_questions.clear();
            true
        }

        SessionEvent::ProtocolError { detail, raw } => {
            append(
                view,
                ItemKind::Error {
                    detail: detail.clone(),
                    raw: raw.clone(),
                },
                false,
                seq,
            );
            true
        }

        SessionEvent::CommandLifecycle {
            command_uuid,
            state,
        } => {
            if state != "started" {
                return false;
            }
            let Seam { label, detail } = queued_command_seam(&view.items);
            append(
                view,
                ItemKind::QueuedCommand {
                    command_uuid: command_uuid.clone(),
                    label: label.into(),
                    detail,
                },
                true,
                seq,
            );
            true
        }

        SessionEvent::CompactBoundary {
            trigger,
            pre_tokens,
        } => {
            // The pre-compaction fill is stale; a usage after the boundary restores it.
            view.context_tokens = None;
            append(
                view,
                ItemKind::Compact {
                    trigger: trigger.clone(),
                    pre_tokens: *pre_tokens,
                    summary: None,
                },
                true,
                seq,
            );
            true
        }

        SessionEvent::Relaunched { quiet, reason } => {
            // A kill that a relaunch resumed from isn't the session's end.
            if matches!(
                view.items.last().map(|it| &it.kind),
                Some(ItemKind::Exit {
                    outcome: ExitOutcome::Killed,
                    ..
                })
            ) {
                view.items.pop();
            }
            append(
                view,
                ItemKind::Relaunched {
                    quiet: *quiet,
                    reason: *reason,
                },
                false,
                seq,
            );
            true
        }

        SessionEvent::Rewound {} => {
            append(view, ItemKind::Rewound, false, seq);
            true
        }

        SessionEvent::BackgroundTaskUpdate {
            task_id,
            tool_use_id,
            status,
        } => {
            append(
                view,
                ItemKind::TaskUpdate {
                    task_id: task_id.clone(),
                    tool_use_id: tool_use_id.clone(),
                    status: status.clone(),
                },
                false,
                seq,
            );
            true
        }

        SessionEvent::PermissionModeChanged { .. } | SessionEvent::PlanModeChanged { .. } => false,
    }
}

fn reduce_assistant_text(
    view: &mut SessionView,
    text: &str,
    parent_id: &Option<String>,
    injected: bool,
    seq: Option<i64>,
) -> bool {
    if is_interrupt_notice(text) {
        return false;
    }
    let child = truthy(parent_id);
    // Ahead of the skill fold, which would take a hook line as a prefix-less skill body.
    if let Some(c) = injected.then(|| native_continuation(text)).flatten() {
        append(
            view,
            ItemKind::NativeContinuation {
                reason: c.reason,
                native: c.native,
            },
            true,
            seq,
        );
        return true;
    }
    // Ahead of the skill fold: the summary arrives injected too.
    if let Some(summary) = (!child).then(|| compact_summary_body(text)).flatten() {
        // The open block is dropped only when a divider takes the summary; it is no divider, so indices hold.
        let latest = view
            .items
            .iter()
            .rposition(|it| matches!(it.kind, ItemKind::Compact { .. }));
        if let Some(at) = latest {
            if let ItemKind::Compact {
                summary: slot @ None,
                ..
            } = &mut view.items[at].kind
            {
                *slot = Some(summary.to_string());
                if let Some(open) = open_assistant_index(&view.items) {
                    view.items.remove(open);
                }
                view.streaming = true;
                return true;
            }
        }
    }
    let skill = skill_body_name(text);
    if skill.is_some() || injected {
        if let Some(i) = first_skill_tool_awaiting_body(&view.items, parent_id.as_deref()) {
            if let ItemKind::Tool { skill_body, .. } = &mut view.items[i].kind {
                *skill_body = Some(text.to_string());
            }
            view.streaming = true;
            return true;
        }
    }
    if let Some(name) = skill {
        let make = ItemKind::Skill {
            name,
            body: text.to_string(),
            parent_id: parent_id.clone(),
        };
        if !child {
            if let Some(open) = open_assistant_index(&view.items) {
                // Takes the open block's place and id; the row's seq is not stamped.
                view.items[open].kind = make;
                view.items[open].seq = None;
                view.streaming = true;
                return true;
            }
        }
        append(view, make, !child, seq);
        view.streaming = true;
        return true;
    }
    // Any other injected note (image sizes, resume nudges, hook feedback) is for the model.
    if injected {
        return false;
    }
    if child {
        if text.is_empty() {
            return false;
        }
        append(
            view,
            ItemKind::Assistant {
                text: text.into(),
                streaming: false,
                auth_required: false,
                parent_id: parent_id.clone(),
            },
            false,
            seq,
        );
        view.streaming = true;
        return true;
    }
    let (reasoning, answer) = split_inline_thinking(text);
    if let Some(reasoning) = reasoning {
        let open = open_assistant_index(&view.items);
        let at = open.or_else(|| streamed_assistant_index(&view.items, text));
        let mut built = vec![TranscriptItem {
            id: format!("i{}", view.counter),
            seq,
            kind: ItemKind::Thinking {
                text: reasoning,
                parent_id: None,
            },
        }];
        if !answer.is_empty() {
            built.push(TranscriptItem {
                id: format!("i{}", view.counter + 1),
                seq,
                kind: ItemKind::Assistant {
                    text: answer,
                    streaming: false,
                    auth_required: false,
                    parent_id: None,
                },
            });
        }
        view.counter += built.len() as u64;
        match at {
            Some(at) => {
                view.items.splice(at..=at, built);
            }
            None => view.items.extend(built),
        }
        view.streaming = true;
        return true;
    }

    if let Some(idx) = open_assistant_index(&view.items) {
        if let ItemKind::Assistant {
            text: held,
            streaming,
            ..
        } = &mut view.items[idx].kind
        {
            if !text.is_empty() {
                *held = text.to_string();
            }
            *streaming = false;
        }
        view.streaming = true;
        return true;
    }
    if text.is_empty() {
        return false;
    }
    if let Some(i) = streamed_assistant_index(&view.items, text) {
        if let ItemKind::Assistant {
            text: held,
            streaming,
            ..
        } = &mut view.items[i].kind
        {
            *held = text.to_string();
            *streaming = false;
        }
        view.streaming = true;
        return true;
    }
    append(
        view,
        ItemKind::Assistant {
            text: text.into(),
            streaming: false,
            auth_required: false,
            parent_id: None,
        },
        false,
        seq,
    );
    view.streaming = true;
    true
}

// A reload's notice holds until the turn waiting on it writes; one that wrote nothing ends it here.
fn settle_reload(items: &mut [TranscriptItem]) {
    if let Some(TranscriptItem {
        kind: ItemKind::Relaunched { reason, .. },
        ..
    }) = items.last_mut()
    {
        *reason = None;
    }
}

pub fn add_user_message(
    view: &mut SessionView,
    text: &str,
    stage: Option<&str>,
    attachments: Vec<Attachment>,
    created_at: Option<f64>,
    prompt_name: Option<&str>,
    client_message_id: Option<&str>,
) {
    append(
        view,
        ItemKind::User {
            text: text.into(),
            stage: stage.map(Into::into),
            attachments,
            created_at,
            prompt_name: prompt_name.map(Into::into),
            client_message_id: client_message_id.map(Into::into),
            author: None,
        },
        true,
        None,
    );
    view.streaming = true;
}

pub fn resolve_permission(view: &mut SessionView, request_id: &str, decision: PermissionDecision) {
    // A background subagent asks after its launching turn ended; answering it doesn't reopen that turn.
    let subagent = view.items.iter().find_map(|it| match &it.kind {
        ItemKind::Permission {
            request_id: r,
            tool_use_id,
            ..
        } if r == request_id => Some(is_subagent_tool(&view.items, tool_use_id.as_deref())),
        _ => None,
    });
    let streaming = view.streaming || !subagent.unwrap_or(false);
    close_ask(view, request_id, decision);
    view.streaming = streaming;
}

pub fn resolve_question(view: &mut SessionView, request_id: &str) {
    if close_question(view, request_id) {
        view.streaming = true;
    }
}

// A logged answer leaves `streaming` alone: the ask kept it up while the turn ran.
fn close_ask(view: &mut SessionView, request_id: &str, decision: PermissionDecision) {
    for it in &mut view.items {
        if let ItemKind::Permission {
            request_id: r,
            decision: d,
            ..
        } = &mut it.kind
        {
            if r == request_id && matches!(d, None | Some(PermissionDecision::Withdrawn)) {
                *d = Some(decision);
            }
        }
    }
    if view
        .pending_permission
        .as_ref()
        .is_some_and(|p| p.request_id == request_id)
    {
        view.pending_permission = None;
    }
    view.state = parked_state(view);
}

fn close_question(view: &mut SessionView, request_id: &str) -> bool {
    let questions = open_questions(view);
    if !questions.iter().any(|q| q.request_id == request_id) {
        return false;
    }
    set_questions(
        view,
        questions
            .into_iter()
            .filter(|q| q.request_id != request_id)
            .collect(),
    );
    view.state = parked_state(view);
    true
}

/// Closes every ask outside `open`: the asks a live harness said it waits on, and any asked since.
pub fn settle_unwaited_asks(view: &mut SessionView, open: &HashSet<String>) {
    // No row says how it closed; its tool's row shows what ran.
    for it in &mut view.items {
        if let ItemKind::Permission {
            request_id,
            decision: d @ None,
            ..
        } = &mut it.kind
        {
            if !open.contains(request_id) {
                *d = Some(PermissionDecision::Allow);
            }
        }
    }
    let kept = open_questions(view)
        .into_iter()
        .filter(|q| open.contains(&q.request_id))
        .collect();
    set_questions(view, kept);
    if view
        .pending_permission
        .as_ref()
        .is_some_and(|p| !open.contains(&p.request_id))
    {
        view.pending_permission = None;
    }
    view.state = parked_state(view);
}

fn expire_pending_in_place(view: &mut SessionView) {
    for it in &mut view.items {
        if let ItemKind::Permission {
            decision: d @ None, ..
        } = &mut it.kind
        {
            *d = Some(PermissionDecision::Expired);
        }
    }
    // A question has no transcript card of its own, so it leaves one behind.
    for question in open_questions(view) {
        let input = serde_json::json!({ "questions": question.questions }).to_string();
        view.items.push(TranscriptItem {
            id: format!("i{}", view.counter),
            seq: None,
            kind: ItemKind::Permission {
                request_id: question.request_id,
                tool_name: "AskUserQuestion".into(),
                input,
                tool_use_id: None,
                decision: Some(PermissionDecision::Expired),
            },
        });
        view.counter += 1;
    }
    view.pending_permission = None;
    view.pending_question = None;
    view.queued_questions.clear();
}

fn expire_pending_prompts(mut view: SessionView) -> SessionView {
    expire_pending_in_place(&mut view);
    view
}

pub fn cancel_pending_prompts(view: &mut SessionView) {
    for it in &mut view.items {
        if let ItemKind::Permission {
            decision: d @ None, ..
        } = &mut it.kind
        {
            *d = Some(PermissionDecision::Deny);
        }
    }
    view.pending_permission = None;
    view.pending_question = None;
    view.queued_questions.clear();
}
