//! `hyprlayer-server mcp [--read-only]`: the `hyprlayer` MCP server over stdio, newline-delimited JSON-RPC 2.0.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::models::session::QuestionAnswers;
use hyprlayer_core::runtime::agent_control::inject::READ_TOOLS;
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::runtime::reviews::AddReviewCommentRequest;
use hyprlayer_core::settings::GoalSettings;
use hyprlayer_core::{ScriptKind, Stage, WorktreeMode};
use hyprlayer_models::{
    AgentCaller, AgentControlLevel, AgentWhoami, GoalLimits, GoalSource, GoalSpec, MessageDelivery,
    PtyAgent, RevealTarget, ReviewSide, SessionEvent, SpawnPtyAgentRequest, SpawnSessionRequest,
    SpawnedAgent, WaitUntil, WakeCondition,
};
use hyprlayer_proto::WireError;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::client::{ConnectError, RpcClient};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// `tools/list` only asks the level; a hung server must not stall the agent's startup.
const LIST_PROBE: Duration = Duration::from_millis(500);
const DEFAULT_TURNS: usize = 20;
const DEFAULT_LINES: u32 = 100;
const DEFAULT_WAIT_SECS: u64 = 300;
/// The server caps a wait at this too; codex's tool timeout sits just above it.
const MAX_WAIT_SECS: u64 = 600;
const MAX_WAKE_SECS: u64 = 86_400;

/// Where the agent's identity says the server is; `None` outside a hyprlayer agent.
pub(crate) struct AgentEnv {
    socket: PathBuf,
    token: String,
}

impl AgentEnv {
    pub(crate) fn from_env() -> Option<Self> {
        let var = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        Some(Self {
            socket: PathBuf::from(var("HYPRLAYER_SOCKET")?),
            token: var("HYPRLAYER_AGENT_TOKEN")?,
        })
    }

    /// Connects, drops the event stream (an agent connection hears none) and announces.
    pub(crate) async fn connect(&self) -> Result<Agent, WireError> {
        let client = RpcClient::connect_socket(&self.socket)
            .await
            .map_err(|e| unreachable(&self.socket, e))?;
        drop(client.take_events());
        let whoami = client.announce_agent(self.token.clone()).await?;
        Ok(Agent {
            client,
            token: self.token.clone(),
            whoami,
        })
    }
}

fn unreachable(socket: &std::path::Path, e: ConnectError) -> WireError {
    WireError {
        code: "server_unreachable".into(),
        message: format!("no hyprlayer server at {} ({e})", socket.display()),
        remedy: Some("Open Hyprlayer, or restart the agent from it.".into()),
        stale_transcript: None,
    }
}

pub(crate) struct Agent {
    client: Arc<RpcClient>,
    token: String,
    pub(crate) whoami: AgentWhoami,
}

/// One tool, shared by `mcp` and `ctl`; `read_only` tools are the ones safe to auto-approve.
struct Tool {
    name: &'static str,
    description: &'static str,
    read_only: bool,
    schema: fn() -> Value,
}

fn no_args() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn workstream_arg() -> Value {
    json!({ "type": "object", "properties": {
        "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
    } })
}

fn session_arg() -> Value {
    json!({ "type": "object", "required": ["session_id"], "properties": {
        "session_id": { "type": "string" },
    } })
}

/// A composer session or a PTY agent: exactly one of the two ids.
fn agent_arg() -> Value {
    json!({ "type": "object", "properties": {
        "session_id": { "type": "string", "description": "A composer session; or give terminal_id." },
        "terminal_id": { "type": "string", "description": "A PTY agent; or give session_id." },
    } })
}

fn plan_args() -> Value {
    json!({ "type": "object", "required": ["plan"], "properties": {
        "plan": { "type": "string", "description": "The plan document's path, as list_docs returns it." },
        "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
        "harness": { "type": "string", "enum": ["claude", "codex", "opencode"], "description": "Only when the user names one: forces a composer session in that harness. Unset, the agent gets the user's default engine." },
        "model": { "type": "string" },
        "effort": { "type": "string" },
        "title": { "type": "string" },
    } })
}

fn queue_item_arg() -> Value {
    json!({ "type": "object", "required": ["id"], "properties": {
        "id": { "type": "string" },
    } })
}

fn wait_props() -> Value {
    json!({
        "until": { "type": "string", "enum": ["idle", "needs_you", "exited"],
            "description": "idle (the default) also stops at needs_you and exited; needs_you also stops at exited." },
        "timeout_sec": { "type": "integer", "minimum": 1, "maximum": MAX_WAIT_SECS,
            "description": "300 by default." },
    })
}

fn text_prop() -> Value {
    json!({
        "text": { "type": "string", "description": "Required: the message to send. The field is `text`; `message` is accepted too." },
        "delivery": { "type": "string", "enum": ["queue", "interrupt"], "description": "queue (the default) lands as a person's message sent mid-turn does: at the agent's next tool call or turn end. interrupt first stops the running turn, cutting off a running command, which can leave partial work such as half an install, then sends this as the agent's next turn. Use it for holds, stops and scope changes that must land before the current command finishes." },
    })
}

fn with_props(mut base: Value, extra: Value) -> Value {
    if let (Some(props), Value::Object(extra)) = (base["properties"].as_object_mut(), extra) {
        props.extend(extra);
    }
    base
}

const TOOLS: [Tool; 38] = [
    Tool {
        name: READ_TOOLS[0],
        description: "Who this agent is to hyprlayer: its session or terminal, workstream, project and control level.",
        read_only: true,
        schema: no_args,
    },
    Tool {
        name: READ_TOOLS[1],
        description: "The open workstreams in this project.",
        read_only: true,
        schema: no_args,
    },
    Tool {
        name: READ_TOOLS[2],
        description: "Live composer sessions and PTY agents in this project, and under stoppedPtyAgents the PTY agents a server restart or crash left exited, which message_agent resumes.",
        read_only: true,
        schema: no_args,
    },
    Tool {
        name: READ_TOOLS[3],
        description: "The latest user and assistant turns of a session in this project, or a PTY agent's screen.",
        read_only: true,
        schema: || {
            with_props(
                agent_arg(),
                json!({
                    "limit": { "type": "integer", "minimum": 1, "description": "Turns, or screen lines for a PTY agent; 20 turns or 100 lines by default." },
                }),
            )
        },
    },
    Tool {
        name: READ_TOOLS[4],
        description: "A workstream's thoughts documents: those attached to it (by role), those its agents wrote last (`written`), and those whose frontmatter names its branch (`branch`).",
        read_only: true,
        schema: workstream_arg,
    },
    Tool {
        name: READ_TOOLS[5],
        description: "A thoughts document's content, by the id or path list_docs returns.",
        read_only: true,
        schema: || {
            json!({ "type": "object", "required": ["id"], "properties": {
                "id": { "type": "string" },
            } })
        },
    },
    Tool {
        name: READ_TOOLS[6],
        description: "The pull request status of a workstream's branch: `kind` is `found` (with `pr`), `noPr`, `notGitHub` (the repository has no GitHub remote), `failed` (with `cause`, and the last known PR as `last`), or `unknown`.",
        read_only: true,
        schema: workstream_arg,
    },
    Tool {
        name: READ_TOOLS[7],
        description: "The git status of a workstream's worktree.",
        read_only: true,
        schema: workstream_arg,
    },
    Tool {
        name: READ_TOOLS[8],
        description: "The last output of a workstream's setup, run or archive script.",
        read_only: true,
        schema: || {
            json!({ "type": "object", "required": ["kind"], "properties": {
                "kind": { "type": "string", "enum": ["setup", "run", "archive"] },
                "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
            } })
        },
    },
    Tool {
        name: READ_TOOLS[9],
        description: "Shows the user a desktop notification. Say why in the body.",
        read_only: true,
        schema: || {
            json!({ "type": "object", "required": ["title", "body"], "properties": {
                "title": { "type": "string" },
                "body": { "type": "string" },
            } })
        },
    },
    Tool {
        name: READ_TOOLS[10],
        description: "Tells the user this agent needs them, and why.",
        read_only: true,
        schema: || {
            json!({ "type": "object", "required": ["reason"], "properties": {
                "reason": { "type": "string" },
            } })
        },
    },
    Tool {
        name: READ_TOOLS[11],
        description: "The last lines of a PTY agent's screen in this project, as plain text. TUI redraws repeat lines.",
        read_only: true,
        schema: || {
            json!({ "type": "object", "required": ["terminal_id"], "properties": {
                "terminal_id": { "type": "string" },
                "lines": { "type": "integer", "minimum": 1, "maximum": 500, "description": "100 by default." },
            } })
        },
    },
    Tool {
        name: READ_TOOLS[12],
        description: "Wakes this agent with a message from hyprlayer when a condition holds or its timeout passes; use it, then end your turn, rather than polling. With `cancel` it drops a pending wake; with neither it lists them. Every result lists `pending`.",
        read_only: true,
        schema: || {
            json!({ "type": "object", "properties": {
                "condition": { "type": "object", "description": "One of {kind:'after',secs}, {kind:'at',unixSecs}, {kind:'ref',branch,remote?} (the remote branch moves; origin by default), {kind:'pr',workstreamId,until:'opened'|'merged'|'closed'|'checks_done'}, {kind:'session',sessionId,until}, {kind:'terminal',terminalId,until}, or {kind:'any_agent',until} (another agent in this project gets there). until is idle, needs_you or exited." },
                "timeout_sec": { "type": "integer", "minimum": 1, "maximum": MAX_WAKE_SECS, "description": "When a condition that is not a timer gives up and wakes you as timed out; 3600 by default." },
                "cancel": { "type": "string", "description": "A pending wake's id." },
            } })
        },
    },
    Tool {
        name: "spawn_agent",
        description: "Starts a background agent in this project on the user's default engine: a PTY agent when the user runs app actions in a CLI, else a composer session. Leave harness unset unless the user names one. The result names a sessionId or a terminalId; wait on a terminalId with wake_when. It runs no higher than this agent's own permission mode.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "properties": {
                "prompt": { "type": "string", "description": "Required for a composer session; a PTY agent gets it once its TUI is ready." },
                "terminal_engine": { "type": "string", "description": "A multiplexer engine id, such as claude-code or codex, to start a PTY agent instead." },
                "bypass": { "type": "boolean", "description": "PTY agents only; needs this agent to be in bypass mode." },
                "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
                "stage": { "type": "string", "enum": ["general", "research", "plan", "implement", "validate", "commit", "ship", "revise"] },
                "harness": { "type": "string", "enum": ["claude", "codex", "opencode"], "description": "Only when the user names one: forces a composer session in that harness. Unset, the agent gets the user's default engine." },
                "model": { "type": "string" },
                "effort": { "type": "string" },
                "permission_mode": { "type": "string", "enum": ["plan", "dontAsk", "default", "acceptEdits", "auto", "bypassPermissions"], "description": "Defaults to this agent's own. Ranked as listed; a child runs no higher than this agent." },
                "attached_docs": { "type": "array", "items": { "type": "string" } },
                "title": { "type": "string" },
            } })
        },
    },
    Tool {
        name: "wait_agent",
        description: "Waits for a session or hooked PTY agent to finish its turn, ask for a person, or exit, and returns its last reply. Returns early as stalled when an idle agent does not move.",
        read_only: false,
        schema: || with_props(agent_arg(), wait_props()),
    },
    Tool {
        name: "message_agent",
        description: "Sends a message to a session in this project, or to a PTY agent this agent spawned, marked as from this agent. An exited PTY agent this agent spawned (list_agents' stoppedPtyAgents) resumes in place, in its own tab and conversation, with the message as its first turn. A message opening with a slash command to a PTY agent that is mid-turn, and any message sent after it, waits for the agent's next idle: the result is then `queued`.",
        read_only: false,
        schema: || {
            with_props(
                agent_arg(),
                text_prop(),
            )
        },
    },
    Tool {
        name: "prompt_and_wait",
        description: "message_agent, then wait_agent: use it rather than polling.",
        read_only: false,
        schema: || {
            with_props(
                with_props(
                    agent_arg(),
                    text_prop(),
                ),
                wait_props(),
            )
        },
    },
    Tool {
        name: "stop_agent",
        description: "Interrupts the turn of a session or PTY agent this agent spawned; a PTY agent gets Esc.",
        read_only: false,
        schema: agent_arg,
    },
    Tool {
        name: "kill_agent",
        description: "Ends a session or PTY agent this agent spawned.",
        read_only: false,
        schema: agent_arg,
    },
    Tool {
        name: "archive_agent",
        description: "Archives an idle session this agent spawned.",
        read_only: false,
        schema: session_arg,
    },
    Tool {
        name: "archive_workstream",
        description: "Archives another workstream in this project, as the user's archive does: its agents stop and its worktree moves aside. Only once the user has said to archive it. Never this agent's own workstream, main, or the Lead's. The result lists the workstream's active child workstreams (made by its subagents) with their changed files; they stay active unless include_children is true.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["workstream_id"], "properties": {
                "workstream_id": { "type": "string" },
                "include_children": { "type": "boolean", "description": "Also archive its active child workstreams, uncommitted changes and all, which move aside with them. Only once the user has said to." },
            } })
        },
    },
    Tool {
        name: "answer_question",
        description: "Answers a question a session this agent spawned is asking, by question text.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["session_id", "request_id", "answers"], "properties": {
                "session_id": { "type": "string" },
                "request_id": { "type": "string" },
                "answers": { "type": "object", "additionalProperties": { "type": "string" } },
            } })
        },
    },
    Tool {
        name: "set_agent_title",
        description: "Renames this agent's own session or one it spawned.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["title"], "properties": {
                "title": { "type": "string" },
                "session_id": { "type": "string", "description": "Defaults to this agent's own session." },
            } })
        },
    },
    Tool {
        name: "queue_add",
        description: "Adds planned work to the end of this project's Lead queue, which the user sees and can edit.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["title", "summary"], "properties": {
                "title": { "type": "string" },
                "summary": { "type": "string", "description": "One line: what the brief will ask for." },
            } })
        },
    },
    Tool {
        name: "queue_list",
        description: "Lists this project's Lead queue in order: queued and running items. The user may have removed or reordered items since you last read it.",
        read_only: false,
        schema: no_args,
    },
    Tool {
        name: "queue_start",
        description: "Marks a queued item running in the workstream it became. Refused when the item is no longer queued, for example because the user removed it.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["id", "workstream_id"], "properties": {
                "id": { "type": "string" },
                "workstream_id": { "type": "string" },
            } })
        },
    },
    Tool {
        name: "queue_done",
        description: "Marks a queue item done, which takes it off the queue.",
        read_only: false,
        schema: queue_item_arg,
    },
    Tool {
        name: "queue_remove",
        description: "Removes an item from the queue without running it.",
        read_only: false,
        schema: queue_item_arg,
    },
    Tool {
        name: "create_workstream",
        description: "Creates a workstream in this project; its setup script runs as it would for the user.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["title"], "properties": {
                "title": { "type": "string" },
                "base": { "type": "string", "description": "The branch to start from; the project's default otherwise." },
                "branch": { "type": "string" },
                "worktree_mode": { "type": "string", "enum": ["isolated", "existing"], "description": "isolated (the default) makes a new branch; existing checks out `branch`." },
            } })
        },
    },
    Tool {
        name: "approve_plan",
        description: "Approves a plan and starts a background implement agent that builds it; the result names a sessionId or a terminalId.",
        read_only: false,
        schema: || {
            with_props(
                plan_args(),
                json!({ "permission_mode": { "type": "string", "enum": ["plan", "default", "acceptEdits", "bypassPermissions"], "description": "Defaults to this agent's own." } }),
            )
        },
    },
    Tool {
        name: "validate_plan",
        description: "Starts a background validate agent against a plan; the result names a sessionId or a terminalId.",
        read_only: false,
        schema: plan_args,
    },
    Tool {
        name: "start_goal",
        description: "Starts a goal: work repeats until its checks pass or a limit is hit. It runs on this agent's own session, a session it spawned, or a new one.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["condition", "work_prompt", "checks"], "properties": {
                "condition": { "type": "string", "description": "What done means." },
                "work_prompt": { "type": "string", "description": "The prompt each iteration sends." },
                "checks": { "type": "array", "items": { "type": "object" }, "description": "At least one of {kind:'command',command,timeoutSecs} (bypass mode only), {kind:'evaluator'} or {kind:'validation',planPath}." },
                "limits": { "type": "object", "description": "{maxIterations,maxCostUsd,maxTokens,maxActiveSecs}; the app's defaults otherwise." },
                "session_id": { "type": "string", "description": "Defaults to this agent's own session." },
                "new_session": { "type": "object", "description": "Start a new session instead: {workstreamId?, stage: general|research|implement, harness?, model?, effort?, permissionMode?, attachedDocs?}." },
            } })
        },
    },
    Tool {
        name: "stop_goal",
        description: "Stops a goal on this agent's own session or one it spawned.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["goal_id"], "properties": {
                "goal_id": { "type": "string" },
            } })
        },
    },
    Tool {
        name: "run_script",
        description: "Runs a workstream's setup or run script in the background; read it with script_output.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["kind"], "properties": {
                "kind": { "type": "string", "enum": ["setup", "run"] },
                "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
            } })
        },
    },
    Tool {
        name: "stop_script",
        description: "Stops a workstream's running run script.",
        read_only: false,
        schema: workstream_arg,
    },
    Tool {
        name: "comment",
        description: "Adds a review comment on a changed file's line, or replies to one.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["path", "line", "body"], "properties": {
                "path": { "type": "string", "description": "Relative to the worktree." },
                "line": { "type": "integer", "minimum": 1 },
                "body": { "type": "string" },
                "side": { "type": "string", "enum": ["original", "modified"], "description": "modified by default." },
                "anchor_text": { "type": "string", "description": "The line's text, so the comment follows it through later edits." },
                "parent_id": { "type": "string", "description": "The comment this replies to." },
                "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
            } })
        },
    },
    Tool {
        name: "resolve_comment",
        description: "Resolves a review comment.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["comment_id"], "properties": {
                "comment_id": { "type": "string" },
                "workstream_id": { "type": "string", "description": "Defaults to this agent's workstream." },
            } })
        },
    },
    Tool {
        name: "reveal",
        description: "Opens a document, file or diff in a background tab of this agent's workstream on the user's desktops.",
        read_only: false,
        schema: || {
            json!({ "type": "object", "required": ["kind", "path"], "properties": {
                "kind": { "type": "string", "enum": ["doc", "file", "diff"] },
                "path": { "type": "string", "description": "A document's path, or a file's path relative to the worktree." },
            } })
        },
    },
];

fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

fn bad_args(detail: impl Into<String>) -> WireError {
    WireError {
        code: "invalid_arguments".into(),
        message: detail.into(),
        remedy: None,
        stale_transcript: None,
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, WireError> {
    arg(args, key).ok_or_else(|| bad_args(format!("`{key}` is required")))
}

fn delivered(delivery: MessageDelivery) -> &'static str {
    match delivery {
        MessageDelivery::Sent => "sent",
        MessageDelivery::Interrupted => {
            "interrupted: the agent's running turn was stopped, and this message is its next turn"
        }
        MessageDelivery::Queued => {
            "queued: the agent is mid-turn, so this slash command, or a message behind one, is typed at its next idle"
        }
    }
}

fn interrupts(args: &Value) -> Result<bool, WireError> {
    match arg(args, "delivery") {
        None | Some("queue") => Ok(false),
        Some("interrupt") => Ok(true),
        Some(_) => Err(bad_args("`delivery` is `queue` or `interrupt`")),
    }
}

fn message_text(args: &Value) -> Result<&str, WireError> {
    arg(args, "text")
        .or_else(|| arg(args, "message"))
        .ok_or_else(|| bad_args("`text` is required: the message to send"))
}

/// A closed-vocabulary argument, read through its own serde names.
fn parsed<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, WireError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| bad_args(format!("`{key}` is not one of its listed values"))),
    }
}

fn wait_args(args: &Value) -> Result<(WaitUntil, u32), WireError> {
    let until = parsed(args, "until")?.unwrap_or(WaitUntil::Idle);
    let secs = args
        .get("timeout_sec")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_SECS)
        .clamp(1, MAX_WAIT_SECS);
    Ok((until, (secs * 1000) as u32))
}

enum Target<'a> {
    Session(&'a str),
    Terminal(&'a str),
}

fn target(args: &Value) -> Result<Target<'_>, WireError> {
    match (arg(args, "session_id"), arg(args, "terminal_id")) {
        (Some(id), None) => Ok(Target::Session(id)),
        (None, Some(id)) => Ok(Target::Terminal(id)),
        _ => Err(bad_args(
            "give exactly one of `session_id` or `terminal_id`",
        )),
    }
}

fn positive(args: &Value, key: &str) -> Option<u64> {
    let value = args.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .filter(|n| *n > 0)
}

/// The user's Goals settings aren't readable from an agent connection; the app's defaults stand in.
fn default_limits() -> GoalLimits {
    let d = GoalSettings::default();
    GoalLimits {
        max_iterations: d.max_iterations,
        max_cost_usd: Some(d.max_cost_usd),
        max_tokens: Some(d.max_tokens),
        max_active_secs: d.max_active_secs,
    }
}

fn plan_session(
    me: &AgentWhoami,
    args: &Value,
    stage: Stage,
) -> Result<SpawnSessionRequest, WireError> {
    Ok(SpawnSessionRequest {
        workstream_id: arg(args, "workstream_id")
            .unwrap_or(&me.workstream_id)
            .to_string(),
        stage: Some(stage),
        prompt: String::new(),
        harness: parsed(args, "harness")?,
        model: arg(args, "model").map(str::to_string),
        effort: arg(args, "effort").map(str::to_string),
        permission_mode: parsed(args, "permission_mode")?,
        attached_docs: vec![required(args, "plan")?.to_string()],
        title: arg(args, "title").map(str::to_string),
    })
}

fn session_summary(session: &hyprlayer_core::Session) -> Value {
    json!({
        "sessionId": session.id,
        "workstreamId": session.workstream_id,
        "harness": session.harness,
        "stage": session.stage,
        "title": session.title,
    })
}

fn pty_summary(agent: &PtyAgent) -> Value {
    json!({
        "terminalId": agent.terminal_id,
        "workstreamId": agent.workstream_id,
        "engineId": agent.engine_id,
        "state": agent.state,
    })
}

fn spawned_summary(spawned: &SpawnedAgent) -> Value {
    match spawned {
        SpawnedAgent::Session { session } => session_summary(session),
        SpawnedAgent::Pty { agent } => pty_summary(agent),
    }
}

fn encode<T: serde::Serialize>(value: T) -> Result<Value, WireError> {
    serde_json::to_value(value).map_err(|e| WireError::internal(e.to_string()))
}

/// Runs one tool for the announced agent; the server enforces scope, this only shapes the call.
pub(crate) async fn call_tool(agent: &Agent, name: &str, args: &Value) -> Result<Value, WireError> {
    let client = &agent.client;
    let me = &agent.whoami;
    let workstream = || {
        arg(args, "workstream_id")
            .unwrap_or(&me.workstream_id)
            .to_string()
    };
    match name {
        // Asked afresh: the level follows the setting and the live mode, and `off` must refuse.
        "whoami" => encode(client.announce_agent(agent.token.clone()).await?),
        "list_workstreams" => {
            let views = client
                .list_workstreams(me.project_path.clone(), false)
                .await?;
            Ok(views
                .into_iter()
                .map(|v| {
                    json!({
                        "id": v.item.id,
                        "title": v.item.title,
                        "stage": v.item.stage,
                        "branch": v.item.branch,
                        "status": v.item.status,
                    })
                })
                .collect())
        }
        "list_agents" => {
            let sessions: Vec<Value> = client
                .list_active_sessions()
                .await?
                .into_iter()
                .map(|a| {
                    json!({
                        "sessionId": a.session.id,
                        "workstreamId": a.session.workstream_id,
                        "title": a.session.title,
                        "harness": a.session.harness,
                        "stage": a.session.stage,
                        "state": a.session.state,
                        "working": a.session.working,
                    })
                })
                .collect();
            let terminals = client.list_pty_agents(None).await?;
            let stopped = client.list_stopped_pty_agents().await?;
            Ok(json!({ "sessions": sessions, "ptyAgents": terminals, "stoppedPtyAgents": stopped }))
        }
        "read_agent" => {
            let session_id = match target(args)? {
                Target::Session(id) => id,
                Target::Terminal(id) => {
                    let lines = positive(args, "limit").map_or(DEFAULT_LINES, |n| n as u32);
                    return Ok(json!(client.read_terminal(id.into(), lines).await?));
                }
            };
            let limit = positive(args, "limit").map_or(DEFAULT_TURNS, |n| n as usize);
            let log = client.get_session_log(session_id.to_string()).await?;
            let turns: Vec<Value> = log
                .into_iter()
                .filter_map(|entry| match entry.event {
                    SessionEvent::UserMessage { text, .. } => {
                        Some(json!({ "role": "user", "text": text }))
                    }
                    SessionEvent::AssistantText {
                        text,
                        parent_id: None,
                        ..
                    } => Some(json!({ "role": "assistant", "text": text })),
                    _ => None,
                })
                .collect();
            let skip = turns.len().saturating_sub(limit);
            Ok(Value::Array(turns.into_iter().skip(skip).collect()))
        }
        "read_terminal" => {
            let lines = positive(args, "lines").map_or(DEFAULT_LINES, |n| n as u32);
            let terminal_id = required(args, "terminal_id")?.to_string();
            Ok(json!(client.read_terminal(terminal_id, lines).await?))
        }
        "list_docs" => encode(client.list_workstream_docs(workstream()).await?),
        "read_doc" => {
            let doc = client.get_doc(required(args, "id")?.to_string()).await?;
            Ok(json!({ "id": doc.meta.id, "relPath": doc.meta.rel_path, "content": doc.content }))
        }
        "pr_status" => encode(client.workstream_pr_status(workstream()).await?),
        "git_status" => encode(client.worktree_git_status(workstream()).await?),
        "script_output" => {
            let kind: ScriptKind = serde_json::from_value(json!(required(args, "kind")?))
                .map_err(|_| bad_args("`kind` is setup, run or archive"))?;
            encode(client.get_script_output(workstream(), kind).await?)
        }
        "notify_user" => {
            let (title, body) = (required(args, "title")?, required(args, "body")?);
            client.notify_user(title.into(), body.into()).await?;
            Ok(json!("sent"))
        }
        "wake_when" => {
            let condition: Option<WakeCondition> = parsed(args, "condition")?;
            let wake = match (condition, arg(args, "cancel")) {
                (Some(_), Some(_)) => {
                    return Err(bad_args("give `condition` or `cancel`, not both"))
                }
                (Some(condition), None) => {
                    let timeout_ms = args
                        .get("timeout_sec")
                        .and_then(Value::as_u64)
                        .map(|secs| (secs.min(MAX_WAKE_SECS) * 1000) as u32);
                    Some(client.wake_when(condition, timeout_ms).await?)
                }
                (None, Some(id)) => {
                    client.cancel_wake(id.into()).await?;
                    None
                }
                (None, None) => None,
            };
            let mut result = json!({ "pending": encode(client.list_wakes().await?)? });
            if let Some(wake) = wake {
                result["wake"] = encode(wake)?;
            }
            Ok(result)
        }
        "request_attention" => {
            client
                .request_attention(required(args, "reason")?.into())
                .await?;
            Ok(json!("sent"))
        }
        "spawn_agent" => {
            if let Some(engine_id) = arg(args, "terminal_engine") {
                let agent: PtyAgent = client
                    .spawn_pty_agent(SpawnPtyAgentRequest {
                        workstream_id: workstream(),
                        engine_id: engine_id.to_string(),
                        prompt: arg(args, "prompt").map(str::to_string),
                        bypass: args.get("bypass").and_then(Value::as_bool).unwrap_or(false),
                        permission_mode: parsed(args, "permission_mode")?,
                        model: arg(args, "model").map(str::to_string),
                        title: arg(args, "title").map(str::to_string),
                    })
                    .await?;
                return Ok(pty_summary(&agent));
            }
            let attached_docs = parsed(args, "attached_docs")?.unwrap_or_default();
            let spawned = client
                .spawn_agent(SpawnSessionRequest {
                    workstream_id: workstream(),
                    stage: parsed(args, "stage")?,
                    prompt: required(args, "prompt")?.to_string(),
                    harness: parsed(args, "harness")?,
                    model: arg(args, "model").map(str::to_string),
                    effort: arg(args, "effort").map(str::to_string),
                    permission_mode: parsed(args, "permission_mode")?,
                    attached_docs,
                    title: arg(args, "title").map(str::to_string),
                })
                .await?;
            Ok(spawned_summary(&spawned))
        }
        "wait_agent" => {
            let (until, timeout_ms) = wait_args(args)?;
            match target(args)? {
                Target::Session(id) => {
                    encode(client.wait_session(id.into(), until, timeout_ms).await?)
                }
                Target::Terminal(id) => {
                    encode(client.wait_pty_agent(id.into(), until, timeout_ms).await?)
                }
            }
        }
        "message_agent" => {
            let (text, interrupt) = (message_text(args)?, interrupts(args)?);
            let delivery = match target(args)? {
                Target::Session(id) => {
                    client
                        .send_agent_message(id.into(), text.into(), interrupt)
                        .await?
                }
                Target::Terminal(id) => {
                    client
                        .send_pty_message(id.into(), text.into(), interrupt)
                        .await?
                }
            };
            Ok(json!(delivered(delivery)))
        }
        "prompt_and_wait" => {
            let (text, interrupt) = (message_text(args)?, interrupts(args)?);
            let (until, timeout_ms) = wait_args(args)?;
            match target(args)? {
                Target::Session(id) => {
                    client
                        .send_agent_message(id.into(), text.into(), interrupt)
                        .await?;
                    encode(client.wait_session(id.into(), until, timeout_ms).await?)
                }
                Target::Terminal(id) => {
                    client
                        .send_pty_message(id.into(), text.into(), interrupt)
                        .await?;
                    encode(client.wait_pty_agent(id.into(), until, timeout_ms).await?)
                }
            }
        }
        "stop_agent" => {
            match target(args)? {
                Target::Session(id) => client.interrupt_session(id.into()).await?,
                Target::Terminal(id) => client.interrupt_pty_agent(id.into()).await?,
            }
            Ok(json!("stopped"))
        }
        "kill_agent" => {
            match target(args)? {
                Target::Session(id) => client.kill_session(id.into()).await?,
                Target::Terminal(id) => client.kill_pty_agent(id.into()).await?,
            }
            Ok(json!("killed"))
        }
        "archive_agent" => {
            client
                .set_session_archived(required(args, "session_id")?.into(), true)
                .await?;
            Ok(json!("archived"))
        }
        "archive_workstream" => {
            let include_children = args.get("include_children").and_then(Value::as_bool);
            encode(
                client
                    .archive_workstream_for_agent(
                        required(args, "workstream_id")?.into(),
                        include_children.unwrap_or(false),
                    )
                    .await?,
            )
        }
        "answer_question" => {
            let answers: QuestionAnswers =
                parsed(args, "answers")?.ok_or_else(|| bad_args("`answers` is required"))?;
            client
                .answer_question(
                    required(args, "session_id")?.into(),
                    required(args, "request_id")?.into(),
                    answers,
                )
                .await?;
            Ok(json!("answered"))
        }
        "set_agent_title" => {
            let session_id = match (arg(args, "session_id"), &me.caller) {
                (Some(id), _) => id.to_string(),
                (None, AgentCaller::Session { session_id }) => session_id.clone(),
                (None, AgentCaller::Terminal { .. }) => {
                    return Err(bad_args("`session_id` is required from a terminal agent"))
                }
            };
            client
                .set_session_title(session_id, Some(required(args, "title")?.into()))
                .await?;
            Ok(json!("renamed"))
        }
        "queue_add" => encode(
            client
                .add_lead_queue_item(
                    me.project_path.clone(),
                    required(args, "title")?.into(),
                    required(args, "summary")?.into(),
                )
                .await?,
        ),
        "queue_list" => encode(client.list_lead_queue(me.project_path.clone()).await?),
        "queue_start" => encode(
            client
                .start_lead_queue_item(
                    me.project_path.clone(),
                    required(args, "id")?.into(),
                    required(args, "workstream_id")?.into(),
                )
                .await?,
        ),
        "queue_done" => encode(
            client
                .finish_lead_queue_item(me.project_path.clone(), required(args, "id")?.into())
                .await?,
        ),
        "queue_remove" => {
            client
                .remove_lead_queue_item(me.project_path.clone(), required(args, "id")?.into())
                .await?;
            Ok(json!("removed"))
        }
        "create_workstream" => {
            let mode = parsed(args, "worktree_mode")?.unwrap_or(WorktreeMode::Isolated);
            if !matches!(mode, WorktreeMode::Isolated | WorktreeMode::Existing) {
                return Err(bad_args("`worktree_mode` is isolated or existing"));
            }
            let view = client
                .create_workstream(
                    me.project_path.clone(),
                    required(args, "title")?.to_string(),
                    arg(args, "base").map(str::to_string),
                    mode,
                    arg(args, "branch").map(str::to_string),
                    None,
                )
                .await?;
            Ok(json!({
                "id": view.item.id,
                "title": view.item.title,
                "branch": view.item.branch,
                "worktreePath": view.item.worktree_path,
            }))
        }
        // Implement activates the attached plan as it starts, as Approve & build does.
        "approve_plan" => {
            let request = plan_session(me, args, Stage::Implement)?;
            Ok(spawned_summary(&client.spawn_agent(request).await?))
        }
        "validate_plan" => {
            let mut request = plan_session(me, args, Stage::Validate)?;
            request.permission_mode = None;
            Ok(spawned_summary(&client.spawn_agent(request).await?))
        }
        "start_goal" => {
            let spec = GoalSpec {
                condition: required(args, "condition")?.to_string(),
                work_prompt: required(args, "work_prompt")?.to_string(),
                source: GoalSource::Composer,
                source_ref: None,
                source_phase: None,
                checks: parsed(args, "checks")?.ok_or_else(|| bad_args("`checks` is required"))?,
                limits: parsed(args, "limits")?.unwrap_or_else(default_limits),
            };
            let target = match (arg(args, "session_id"), args.get("new_session")) {
                (Some(_), Some(_)) => {
                    return Err(bad_args("give `session_id` or `new_session`, not both"))
                }
                (Some(id), None) => GoalTarget::Session {
                    session_id: id.to_string(),
                },
                (None, Some(fields)) => {
                    let mut fields = fields.clone();
                    if let Value::Object(map) = &mut fields {
                        map.entry("kind").or_insert(json!("new_session"));
                        map.entry("workstreamId").or_insert(json!(me.workstream_id));
                    }
                    serde_json::from_value(fields)
                        .map_err(|e| bad_args(format!("`new_session` is malformed: {e}")))?
                }
                (None, None) => match &me.caller {
                    AgentCaller::Session { session_id } => GoalTarget::Session {
                        session_id: session_id.clone(),
                    },
                    AgentCaller::Terminal { .. } => {
                        return Err(bad_args(
                            "a terminal agent names `session_id` or `new_session`",
                        ))
                    }
                },
            };
            let goal = client.start_goal(StartGoalRequest { target, spec }).await?;
            Ok(json!({ "goalId": goal.id, "sessionId": goal.session_id, "status": goal.status }))
        }
        "stop_goal" => {
            let goal = client
                .stop_goal(required(args, "goal_id")?.to_string())
                .await?;
            Ok(json!({ "goalId": goal.id, "status": goal.status }))
        }
        "run_script" => {
            let kind: ScriptKind = serde_json::from_value(json!(required(args, "kind")?))
                .map_err(|_| bad_args("`kind` is setup, run or archive"))?;
            client.script_start(workstream(), kind).await?;
            Ok(json!("started"))
        }
        "stop_script" => {
            client.script_stop(workstream()).await?;
            Ok(json!("stopped"))
        }
        "comment" => {
            let line = positive(args, "line").ok_or_else(|| bad_args("`line` is a line number"))?;
            let comment = client
                .add_review_comment(AddReviewCommentRequest {
                    workstream_id: workstream(),
                    path: required(args, "path")?.to_string(),
                    side: parsed(args, "side")?.unwrap_or(ReviewSide::Modified),
                    line: u32::try_from(line).map_err(|_| bad_args("`line` is too large"))?,
                    anchor_text: arg(args, "anchor_text").unwrap_or_default().to_string(),
                    body: required(args, "body")?.to_string(),
                    parent_id: arg(args, "parent_id").map(str::to_string),
                    reply_to_note_id: None,
                })
                .await?;
            Ok(json!({ "commentId": comment.id }))
        }
        "resolve_comment" => {
            client
                .resolve_review_comment(
                    workstream(),
                    required(args, "comment_id")?.to_string(),
                    true,
                )
                .await?;
            Ok(json!("resolved"))
        }
        "reveal" => {
            let path = required(args, "path")?.to_string();
            let target = match required(args, "kind")? {
                "doc" => RevealTarget::Doc { path },
                "file" => RevealTarget::File { path },
                "diff" => RevealTarget::Diff { path },
                _ => return Err(bad_args("`kind` is doc, file or diff")),
            };
            client.request_reveal(target).await?;
            Ok(json!("revealed"))
        }
        other => Err(bad_args(format!("no tool named `{other}`"))),
    }
}

/// A refusal reads as the code, the message and what to do about it.
pub(crate) fn describe(error: &WireError) -> String {
    match &error.remedy {
        Some(remedy) => format!("{}: {}\n{remedy}", error.code, error.message),
        None => format!("{}: {}", error.code, error.message),
    }
}

struct Session {
    env: Option<AgentEnv>,
    read_only: bool,
    agent: Option<Agent>,
}

impl Session {
    /// Read tools only unless the agent is at `full` and this face isn't read-only.
    async fn tools(&mut self) -> Value {
        if !self.read_only && self.agent.is_none() {
            if let Some(env) = self.env.as_ref() {
                self.agent = tokio::time::timeout(LIST_PROBE, env.connect())
                    .await
                    .ok()
                    .and_then(Result::ok);
            }
        }
        let full = !self.read_only
            && self
                .agent
                .as_ref()
                .is_some_and(|agent| agent.whoami.level == AgentControlLevel::Full);
        let tools: Vec<Value> = TOOLS
            .iter()
            .filter(|t| t.read_only || full)
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": (t.schema)(),
                    "annotations": { "readOnlyHint": t.read_only },
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    /// Connects lazily; a read tool reconnects once, so a restarted server costs one retry, not the tool.
    /// A mutating tool never retries: the dropped request may already have run.
    async fn call(&mut self, tool: &Tool, args: &Value) -> Result<Value, WireError> {
        let Some(env) = self.env.as_ref() else {
            return Err(not_an_agent());
        };
        let attempts = if tool.read_only { 2 } else { 1 };
        for attempt in 0..attempts {
            let agent = match self.agent.take() {
                Some(agent) => agent,
                None => env.connect().await?,
            };
            match call_tool(&agent, tool.name, args).await {
                // The dead connection is dropped either way, so the next call reconnects.
                Err(e) if e.code == WireError::disconnected().code => {
                    if attempt + 1 == attempts {
                        return Err(e);
                    }
                }
                result => {
                    self.agent = Some(agent);
                    return result;
                }
            }
        }
        Err(WireError::disconnected())
    }
}

pub(crate) fn not_an_agent() -> WireError {
    WireError {
        code: "not_an_agent".into(),
        message:
            "not inside a hyprlayer agent: HYPRLAYER_SOCKET and HYPRLAYER_AGENT_TOKEN are unset"
                .into(),
        remedy: Some("Run this from an agent Hyprlayer launched.".into()),
        stale_transcript: None,
    }
}

pub async fn run(read_only: bool) -> i32 {
    let mut session = Session {
        env: AgentEnv::from_env(),
        read_only,
        agent: None,
    };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&mut session, &message).await,
            Err(e) => Some(error_reply(
                Value::Null,
                -32700,
                &format!("parse error: {e}"),
            )),
        };
        if let Some(reply) = reply {
            let mut out = std::io::stdout().lock();
            if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
                return 0;
            }
        }
    }
    0
}

/// `None` for a notification: JSON-RPC never answers a message without an id.
async fn handle(session: &mut Session, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match message.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| PROTOCOL_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hyprlayer", "version": crate::VERSION },
            })
        }
        "ping" => json!({}),
        "tools/list" => session.tools().await,
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(tool) = tool(name) else {
                return Some(error_reply(id, -32602, &format!("unknown tool `{name}`")));
            };
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            // `tools/list` hid it; a client that calls it anyway is refused here too.
            let called = if session.read_only && !tool.read_only {
                Err(read_only_refusal(name))
            } else {
                session.call(tool, &args).await
            };
            let (text, is_error) = match called {
                Ok(value) => (
                    serde_json::to_string_pretty(&value).unwrap_or_default(),
                    false,
                ),
                Err(e) => (describe(&e), true),
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
        }
        method => {
            return Some(error_reply(
                id,
                -32601,
                &format!("method not found: {method}"),
            ))
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn read_only_refusal(name: &str) -> WireError {
    WireError {
        code: "agent_control_denied".into(),
        message: format!("{name} is not available: this agent's hyprlayer tools are read-only"),
        remedy: Some("Ask the user to do this in Hyprlayer.".into()),
        stale_transcript: None,
    }
}

fn error_reply(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_reads_text_or_its_message_alias() {
        assert_eq!(message_text(&json!({ "text": "hi" })).unwrap(), "hi");
        assert_eq!(message_text(&json!({ "message": "hi" })).unwrap(), "hi");
        assert_eq!(
            message_text(&json!({ "text": "a", "message": "b" })).unwrap(),
            "a"
        );
        assert!(message_text(&json!({ "session_id": "s" })).is_err());
    }
}
