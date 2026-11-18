//! With app actions in a CLI, an agent's spawn_agent and approve_plan start a PTY agent in the
//! default engine: the stage's prompt is composed and pasted with the caller's header after it,
//! and the stage's deny list still applies. A pinned harness, a stage on an engine without its
//! guardrails, an unhooked TUI for a non-bypass caller, or the setting off keeps a session.
//!
//! The lead is `goal_stub`'s claude stand-in and the engine `agent_stub`'s PTY claude.
#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use hyprlayer_core::models::{AgentControlLevel, PermissionMode};
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::agent_stub::{
    agent_env, env_var, install, launch_log, lines, pasted, set_engine, set_level, tool_failed,
    tool_text, update_settings, McpDriver, PTY_CLAUDE,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(30);
const PLAN: &str =
    "---\ntitle: Spawned plan\ntype: plan\nstatus: draft\nauthor: e2e\n---\n\n# Spawned plan\n\n## Phase 1\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_spawn_in_cli() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("agent_spawn_in_cli", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Spawns").await;
    let ws = text(&view, "/item/id");
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    update_settings(&mut j, &c, |s| {
        s.multiplexer.enabled = true;
        s.agent_control.max_live_children = 10;
    })
    .await;
    let lead = start_lead(&mut j, &c, &ws).await;
    let mut mcp = driver(&mut j, &sandbox, &lead).await;
    let header = format!("[from agent \"Lead\" · {lead}]");

    j.step("spawn_agent opens the default engine");
    let spawned = parsed(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_REVIEW", "title": "Reviewer" }),
        )
        .await,
    );
    let terminal = spawned["terminalId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    j.alias(&terminal, "$REVIEWER");
    let log = launch_log(&sandbox, &terminal);
    let expected = pasted(&format!("E2E_REVIEW\n\n{header}"));
    let stdin = log.join("stdin.log");
    let typed = eventually(IO, || lines(&stdin) == expected).await;
    j.guard(
        "a PTY agent, not a session, typed the prompt with the caller's header after it",
        !terminal.is_empty() && spawned.get("sessionId").is_none() && typed,
        json!({ "engineId": spawned["engineId"], "hasSessionId": spawned.get("sessionId").is_some(), "matchesThePaste": typed }),
    );

    j.step("an unhooked engine falls back to a session for a non-bypass caller");
    set_engine(
        &mut j,
        &c,
        "claude-code",
        &pty_claude,
        &["--settings", "{}"],
    )
    .await;
    let fallback = parsed(
        &mcp.call("spawn_agent", json!({ "prompt": "E2E_FALLBACK" }))
            .await,
    );
    let ptys = c
        .list_pty_agents(Some(ws.clone()))
        .await
        .expect("list_pty_agents");
    // Only the reviewer's: the unhooked one was closed before the spawn returned.
    let closed = ptys.len() == 1;
    j.guard(
        "a claude that came up unhooked is closed, and the spawn becomes a session instead of a refusal",
        fallback["sessionId"].is_string() && fallback.get("terminalId").is_none() && closed,
        json!({ "result": fallback.get("sessionId").map(|_| "session"), "error": fallback.get("error"), "livePtys": ptys.len() }),
    );
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    j.step("approve_plan builds the plan in the default engine");
    set_mode(&mut j, &c, &lead, PermissionMode::BypassPermissions).await;
    let plan = write_plan(&mut j, &c, &project).await;
    let approved = parsed(&mcp.call("approve_plan", json!({ "plan": plan })).await);
    let implement = approved["terminalId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    j.alias(&implement, "$IMPLEMENT");
    let log = launch_log(&sandbox, &implement);
    let opening = ["\x1b[200~/implement_plan".to_string(), String::new()];
    let stdin = log.join("stdin.log");
    let opened = eventually(IO, || lines(&stdin).starts_with(&opening)).await;
    let argv = lines(&log.join("argv"));
    let denies_push = argv
        .windows(2)
        .any(|w| w[0] == "--disallowedTools" && w[1] == "Bash(git push:*)");
    let bypasses = argv.iter().any(|a| a == "--dangerously-skip-permissions");
    let active = eventually(IO, || plan_status(&plan) == "active").await;
    j.guard(
        "the paste opens with /implement_plan under Implement's deny list, in the bypass caller's mode, and the plan turns active",
        opened && denies_push && bypasses && active,
        json!({ "opensWithTheSkill": opened, "deniesPush": denies_push, "bypasses": bypasses, "planActive": active }),
    );

    j.step("a stage on an engine that can't apply its guardrails keeps a session");
    update_settings(&mut j, &c, |s| {
        s.multiplexer.default_engine_id = "codex".into()
    })
    .await;
    let codex = parsed(&mcp.call("validate_plan", json!({ "plan": plan })).await);
    j.guard(
        "validate_plan with codex as the default engine starts a validate session, whose posture is read-only",
        codex["sessionId"].is_string() && codex["stage"] == json!("validate"),
        json!({ "hasSessionId": codex["sessionId"].is_string(), "stage": codex["stage"], "error": codex.get("error") }),
    );
    update_settings(&mut j, &c, |s| {
        s.multiplexer.default_engine_id = "claude-code".into()
    })
    .await;

    j.step("a pinned harness keeps a session");
    let pinned = parsed(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_PINNED", "harness": "claude" }),
        )
        .await,
    );
    j.guard(
        "the caller's harness choice outranks the engine",
        pinned["sessionId"].is_string() && pinned.get("terminalId").is_none(),
        json!({ "hasSessionId": pinned["sessionId"].is_string(), "hasTerminalId": pinned.get("terminalId").is_some() }),
    );

    j.step("with app actions in a chat, spawns stay sessions");
    update_settings(&mut j, &c, |s| s.multiplexer.actions_in_cli = false).await;
    let chat = parsed(
        &mcp.call("spawn_agent", json!({ "prompt": "E2E_CHAT" }))
            .await,
    );
    j.guard(
        "spawn_agent starts a composer session",
        chat["sessionId"].is_string() && chat.get("terminalId").is_none(),
        json!({ "hasSessionId": chat["sessionId"].is_string(), "hasTerminalId": chat.get("terminalId").is_some() }),
    );

    j.finish();
}

/// A titled composer lead, its first turn done, so the header naming it is fixed.
async fn start_lead(j: &mut Journey, c: &RpcClient, ws: &str) -> String {
    let (session, _prompt, _stream_id, _events) = c
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_START".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode: Some(PermissionMode::Default),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$LEAD");
    j.call(
        c,
        Request::SetSessionTitle {
            session_id: session.id.clone(),
            title: Some("Lead".into()),
        },
    )
    .await
    .expect("set_session_title");
    j.call(
        c,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let log = c
            .get_session_log(session.id.clone())
            .await
            .expect("get_session_log");
        if log
            .iter()
            .any(|e| matches!(e.event, SessionEvent::TurnDone { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    session.id
}

/// The MCP face under the lead's launch env, initialized.
async fn driver(j: &mut Journey, sandbox: &Sandbox, session_id: &str) -> McpDriver {
    eventually(IO, || {
        !env_var(&agent_env(sandbox, session_id), "HYPRLAYER_AGENT_TOKEN").is_empty()
    })
    .await;
    let env = agent_env(sandbox, session_id);
    j.alias(env_var(&env, "HYPRLAYER_AGENT_TOKEN"), "$LEAD_TOKEN");
    let mut mcp = McpDriver::start(&env, false);
    mcp.initialize("2025-06-18").await;
    mcp
}

async fn set_mode(j: &mut Journey, c: &RpcClient, session_id: &str, mode: PermissionMode) {
    j.call(
        c,
        Request::SetSessionPermissionMode {
            session_id: session_id.to_string(),
            mode: Some(mode),
        },
    )
    .await
    .expect("set_session_permission_mode");
}

/// A draft plan indexed in the project's thoughts; returns its path.
async fn write_plan(j: &mut Journey, c: &RpcClient, project: &str) -> String {
    let meta = j
        .call(
            c,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.to_string()),
                    scope: "shared".into(),
                    doc_type: "plan".into(),
                    title: "Spawned plan".into(),
                    date: "2026-01-01".into(),
                    ticket: None,
                    tags: vec![],
                },
            },
        )
        .await
        .expect("create_doc");
    let path = j
        .call(
            c,
            Request::ResolveDocPath {
                id: text(&meta, "/id"),
            },
        )
        .await
        .expect("resolve_doc_path");
    let path = text(&path, "");
    if let Some(dir) = Path::new(&path).parent().and_then(Path::parent) {
        j.alias(dir.display().to_string(), "$SHARED");
    }
    std::fs::write(&path, PLAN).expect("write the plan");
    path
}

fn plan_status(plan: &str) -> String {
    std::fs::read_to_string(plan)
        .ok()
        .and_then(|doc| {
            doc.lines()
                .find_map(|l| l.strip_prefix("status: ").map(str::to_string))
        })
        .unwrap_or_default()
}

fn parsed(result: &Value) -> Value {
    if tool_failed(result) {
        return json!({ "error": tool_text(result) });
    }
    serde_json::from_str(&tool_text(result)).unwrap_or(Value::Null)
}
