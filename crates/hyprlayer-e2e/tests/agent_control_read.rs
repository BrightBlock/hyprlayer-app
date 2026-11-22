//! Read-only agent control: the identity and MCP config a composer launch carries, the MCP
//! protocol, the read surface and its project scope, refusals at every level, the level resolved
//! from the live permission mode, and what an agent connection never gets (events, streams, a
//! server shutdown).
//!
//! The harnesses are `goal_stub`'s claude and `agent_stub`'s PTY claude; the `mcp` and `ctl` faces
//! are the real server binary, run under each agent's env.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_core::index::indexer::ThoughtQuery;
use hyprlayer_core::models::{
    AgentControlLevel, PermissionMode, PtyAgentState, PtyLaunchSession, RevealTarget, UiRequest,
};
use hyprlayer_core::pr::MergeMethod;
use hyprlayer_core::runtime::agent_control::inject::READ_TOOLS;
use hyprlayer_core::runtime::agent_control::AGENT_ENV;
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::settings::AppSettings;
use hyprlayer_core::{Harness, LogCursor, SessionEvent, Stage, WorktreeMode};
use hyprlayer_e2e::agent_stub::{
    agent_env, appears, env_var, install, launch_log, launch_pty, run_face, set_engine, set_level,
    tool_failed, tool_text, type_line, McpDriver, PTY_CLAUDE,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{
    events_until, eventually, json, server_binary, text, Journey, Request, RuntimeEvent, Sandbox,
    Server, Value,
};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::{EventStream, RpcClient, TerminalStream};
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;
use tokio::sync::mpsc::Receiver;

const IO: Duration = Duration::from_secs(30);

/// `Request` isn't `Clone`; each list is asked twice, of the agent and of the desktop.
type MakeRequest = Box<dyn Fn() -> Request>;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_control_read() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let other_repo = sandbox.fixture_repo("other", &[("README.md", "# other\n")]);
    let mut j = Journey::new("agent_control_read", &sandbox);
    let ctl = std::fs::canonicalize(server_binary()).expect("the server binary");
    j.alias(ctl.display().to_string(), "$CTL");
    j.alias(server_binary().display().to_string(), "$CTL");

    let mut server = Server::start(&sandbox).await;
    let c = server.client();
    let mut rx = c.take_events().expect("the desktop's runtime events");
    let quiet = server.connect().await;
    let mut quiet_rx = quiet
        .take_events()
        .expect("the unannounced client's events");
    j.call(
        &c,
        Request::AnnounceClient {
            client_id: "desktop-a".into(),
            device: "device-a".into(),
            capabilities: TARGETED_EVENTS.iter().map(|k| k.to_string()).collect(),
        },
    )
    .await
    .expect("announce_client");
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Agents").await;
    let ws = text(&view, "/item/id");
    let other = text(
        &j.call(
            &c,
            Request::ImportProject {
                path: sandbox.as_given(&other_repo).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import the other project"),
        "/path",
    );
    let other_view = j
        .call(
            &c,
            Request::CreateWorkstream {
                project_path: other.clone(),
                title: "Elsewhere".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create the other workstream");
    let other_ws = text(&other_view, "/item/id");
    j.alias(&other_ws, "$OTHER_WS");
    let other_slug = text(&other_view, "/item/slug");
    if let Some((stem, _)) = other_slug.rsplit_once('-') {
        j.alias(&other_slug, format!("{stem}-$SUFFIX"));
    }
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    // Bound for the whole run: a dropped stream would end the agent and revoke its token.
    let (session, _session_events) = start_claude(&mut j, &c, &ws, "$SESSION").await;
    let s_env = identity(&mut j, &sandbox, &session, "$SESSION_TOKEN").await;
    let (pty, _pty_events) = launch_hooked(&mut j, &c, &sandbox, &ws, "$PTY").await;
    let t_env = identity(&mut j, &sandbox, &pty, "$PTY_TOKEN").await;
    let (other_session, _other_session_events) =
        start_claude(&mut j, &c, &other_ws, "$OTHER_SESSION").await;
    identity(&mut j, &sandbox, &other_session, "$OTHER_SESSION_TOKEN").await;
    let (other_pty, _other_pty_events) =
        launch_hooked(&mut j, &c, &sandbox, &other_ws, "$OTHER_PTY").await;
    identity(&mut j, &sandbox, &other_pty, "$OTHER_PTY_TOKEN").await;
    let token = env_var(&s_env, "HYPRLAYER_AGENT_TOKEN").to_string();

    j.step("composer launches carry identity and MCP");
    let ctl_env = std::fs::canonicalize(env_var(&s_env, "HYPRLAYER_CTL")).ok();
    let env_ok = env_var(&s_env, "HYPRLAYER_SESSION_ID") == session
        && env_var(&s_env, "HYPRLAYER_WORKSTREAM_ID") == ws
        && env_var(&s_env, "HYPRLAYER_SOCKET") == sandbox.socket().display().to_string()
        && ctl_env.as_deref() == Some(ctl.as_path())
        && env_var(&s_env, "HYPRLAYER_TERMINAL_ID").is_empty();
    j.guard(
        "the agent-env names this session, its workstream, the socket and the server binary",
        env_ok,
        json!({ "identityKeys": sorted(
            s_env
                .iter()
                .map(|(k, _)| k.clone())
                .filter(|k| AGENT_ENV.contains(&k.as_str()))
                .collect(),
        ) }),
    );
    let argv: Vec<String> = sandbox
        .read(
            &sandbox
                .root
                .join("goal-control")
                .join(&session)
                .join("argv.log"),
        )
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let hyprlayer_rules: Vec<String> = flag_values(&argv, "--allowedTools")
        .iter()
        .filter_map(|t| t.strip_prefix("mcp__hyprlayer__").map(str::to_string))
        .collect();
    j.guard(
        "--allowedTools holds mcp__hyprlayer__whoami and every read tool, read_terminal included",
        sorted(hyprlayer_rules.clone()) == sorted(READ_TOOLS.map(str::to_string).to_vec()),
        json!({ "hyprlayerRules": sorted(hyprlayer_rules) }),
    );
    let mcp_config: Value = flag_values(&argv, "--mcp-config")
        .first()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(Value::Null);
    let entry = &mcp_config["mcpServers"]["hyprlayer"];
    let mcp_env_keys: Vec<String> = entry["env"]
        .as_object()
        .map(|e| e.keys().cloned().collect())
        .unwrap_or_default();
    j.guard(
        "--mcp-config runs `<ctl> mcp` with this session's identity",
        entry["command"] == json!(ctl.display().to_string())
            && entry["args"] == json!(["mcp"])
            && entry["env"]["HYPRLAYER_AGENT_TOKEN"] == json!(token)
            && entry["env"]["HYPRLAYER_SESSION_ID"] == json!(session)
            && entry["env"]["HYPRLAYER_WORKSTREAM_ID"] == json!(ws),
        json!({
            "command": entry["command"],
            "args": entry["args"],
            "env": sorted(mcp_env_keys),
        }),
    );

    j.step("initialize and tools/list");
    let mut mcp = McpDriver::start(&s_env, false);
    let mut versions = Vec::new();
    let mut server_name = Value::Null;
    for asked in ["2025-06-18", "2025-03-26", "2024-11-05", "2099-01-01"] {
        let reply = mcp.initialize(asked).await;
        server_name = reply["result"]["serverInfo"]["name"].clone();
        versions.push(json!({ "asked": asked, "answered": reply["result"]["protocolVersion"] }));
    }
    j.guard(
        "known protocol versions are echoed, an unknown one gets 2025-06-18",
        versions.iter().take(3).all(|v| v["asked"] == v["answered"])
            && versions[3]["answered"] == json!("2025-06-18")
            && server_name == json!("hyprlayer"),
        json!({ "versions": versions, "serverName": server_name }),
    );
    let ping = mcp.request("ping", json!({})).await;
    let unknown = mcp.request("resources/list", json!({})).await;
    j.guard(
        "ping answers and an unknown method is -32601",
        ping["result"] == json!({}) && unknown["error"]["code"] == json!(-32601),
        json!({ "ping": ping["result"], "unknown": unknown["error"]["code"] }),
    );
    let listed = tool_list(&mut mcp).await;
    j.guard(
        "at read only the 13 read tools are listed, each marked read-only",
        listed.names == sorted(READ_TOOLS.map(str::to_string).to_vec()) && listed.all_read_only,
        json!({ "tools": listed.names, "allReadOnly": listed.all_read_only }),
    );
    let mut lost = McpDriver::start(&[], false);
    lost.initialize("2025-06-18").await;
    let lost_list = tool_list(&mut lost).await;
    let lost_call = lost.call("whoami", json!({})).await;
    j.guard(
        "with no agent env it still lists the read tools and refuses a call as not_an_agent",
        lost_list.names.len() == READ_TOOLS.len()
            && tool_failed(&lost_call)
            && tool_text(&lost_call).starts_with("not_an_agent"),
        json!({ "tools": lost_list.names.len(), "call": first_line(&tool_text(&lost_call)) }),
    );
    drop(lost);
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    let mut full = McpDriver::start(&s_env, false);
    full.initialize("2025-06-18").await;
    let full_list = tool_list(&mut full).await;
    drop(full);
    let mut read_only = McpDriver::start(&s_env, true);
    read_only.initialize("2025-06-18").await;
    let read_only_list = tool_list(&mut read_only).await;
    let read_only_call = read_only
        .call("spawn_agent", json!({ "prompt": "review this" }))
        .await;
    drop(read_only);
    set_level(&mut j, &c, AgentControlLevel::Read).await;
    j.guard(
        "at full every tool is listed; --read-only still lists only the read tools",
        full_list.names.len() == 32
            && full_list.names.iter().any(|n| n == "spawn_agent")
            && read_only_list.names == listed.names,
        json!({ "full": full_list.names.len(), "readOnly": read_only_list.names.len() }),
    );
    j.guard(
        "--read-only refuses a mutating tool a client calls anyway",
        tool_failed(&read_only_call)
            && tool_text(&read_only_call).starts_with("agent_control_denied")
            && tool_text(&read_only_call).contains("read-only"),
        json!({ "call": first_line(&tool_text(&read_only_call)) }),
    );

    j.step("whoami names the caller");
    let me = whoami(&mut mcp).await;
    j.guard(
        "the composer session, its workstream and project, at read, depth 0 with no parent",
        me["caller"] == json!({ "kind": "session", "sessionId": session })
            && me["workstreamId"] == json!(ws)
            && me["projectPath"] == json!(project)
            && me["level"] == json!("read")
            && me["depth"] == json!(0)
            && me["parent"].is_null()
            && me["reportOnly"] == json!(false),
        json!({ "whoami": me }),
    );
    let face = run_face(&["ctl", "whoami", "--json"], &t_env, b"").await;
    let term_me: Value = serde_json::from_str(&face.stdout).unwrap_or(Value::Null);
    j.guard(
        "`ctl whoami` from the PTY agent names the terminal",
        face.code == Some(0)
            && term_me["caller"] == json!({ "kind": "terminal", "terminalId": pty })
            && term_me["workstreamId"] == json!(ws)
            && term_me["level"] == json!("read"),
        json!({ "exit": face.code, "whoami": term_me }),
    );

    j.step("read tools see the project");
    let workstreams = mcp.call("list_workstreams", json!({})).await;
    let ws_ids = ids(&parse(&workstreams), "/id");
    let agents = parse(&mcp.call("list_agents", json!({})).await);
    let agent_sessions = ids(&agents["sessions"], "/sessionId");
    let agent_ptys = ids(&agents["ptyAgents"], "/terminalId");
    j.guard(
        "list_workstreams and list_agents hold this project's rows only",
        ws_ids == [ws.clone()]
            && agent_sessions == [session.clone()]
            && agent_ptys == [pty.clone()],
        json!({ "workstreams": ws_ids, "sessions": agent_sessions, "ptyAgents": agent_ptys }),
    );
    let mut turns = Value::Null;
    for _ in 0..300 {
        turns = parse(
            &mcp.call("read_agent", json!({ "session_id": session }))
                .await,
        );
        if turns.as_array().is_some_and(|t| t.len() >= 2) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let roles = ids(&turns, "/role");
    j.guard(
        "read_agent returns the session's prompt and reply",
        roles == ["user", "assistant"] && text(&turns, "/1/text") == "counter is 1",
        json!({ "roles": roles, "reply": text(&turns, "/1/text") }),
    );
    type_line(&c, &pty, "hello-from-the-desktop").await;
    let mut screen = String::new();
    for _ in 0..300 {
        let read = mcp
            .call("read_terminal", json!({ "terminal_id": pty, "lines": 20 }))
            .await;
        screen = serde_json::from_str::<String>(&tool_text(&read)).unwrap_or_default();
        if screen.contains("hello-from-the-desktop") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    j.guard(
        "read_terminal returns the PTY agent's screen, without escape codes",
        screen.contains("hello-from-the-desktop") && !screen.contains('\x1b'),
        json!({ "showsTypedLine": screen.contains("hello-from-the-desktop"), "escapes": screen.contains('\x1b') }),
    );
    let git = mcp.call("git_status", json!({})).await;
    let docs = mcp.call("list_docs", json!({})).await;
    j.guard(
        "git_status and list_docs answer for the caller's workstream",
        !tool_failed(&git) && !tool_failed(&docs),
        json!({ "gitStatus": !tool_failed(&git), "listDocs": !tool_failed(&docs) }),
    );

    j.step("a mutating tool is refused at read");
    let sessions_before = c
        .list_sessions(ws.clone())
        .await
        .expect("list_sessions")
        .len();
    let spawn = mcp
        .call("spawn_agent", json!({ "prompt": "review this" }))
        .await;
    let reveal = mcp
        .call("reveal", json!({ "kind": "file", "path": "README.md" }))
        .await;
    let sessions_after = c
        .list_sessions(ws.clone())
        .await
        .expect("list_sessions")
        .len();
    let needs_full = |r: &Value| {
        tool_failed(r)
            && tool_text(r).starts_with("agent_control_denied")
            && tool_text(r).contains("Full")
    };
    j.guard(
        "spawn_agent and reveal are agent_control_denied, and nothing was spawned",
        needs_full(&spawn) && needs_full(&reveal) && sessions_after == sessions_before,
        json!({
            "spawnAgent": first_line(&tool_text(&spawn)),
            "reveal": first_line(&tool_text(&reveal)),
            "spawned": sessions_after - sessions_before,
        }),
    );

    j.step("notify reaches announced desktops");
    let sent = mcp
        .call(
            "notify_user",
            json!({ "title": "Build finished", "body": "tests pass" }),
        )
        .await;
    let note = notify(&mut rx, "Build finished").await;
    j.guard(
        "notify_user reaches the desktop as UiRequested::Notify for the caller's workstream",
        !tool_failed(&sent) && note == notice(&ws, "tests pass"),
        json!({ "sent": tool_text(&sent), "notify": note }),
    );
    let asked = mcp
        .call("request_attention", json!({ "reason": "pick a database" }))
        .await;
    let attention = notify(&mut rx, "An agent needs you").await;
    j.guard(
        "request_attention is a Notify that the agent needs the user",
        !tool_failed(&asked) && attention == notice(&ws, "pick a database"),
        json!({ "notify": attention }),
    );
    let face = run_face(
        &["ctl", "notify", "From the terminal", "hello"],
        &t_env,
        b"",
    )
    .await;
    let from_pty = notify(&mut rx, "From the terminal").await;
    j.guard(
        "`ctl notify` from the PTY agent reaches it too",
        face.code == Some(0) && from_pty == notice(&ws, "hello"),
        json!({ "exit": face.code, "notify": from_pty }),
    );

    j.step("excluded commands are refused at full");
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    let agent = connect_agent(&mut j, &server, &token).await;
    let merge = j
        .call(
            &agent,
            Request::MergeWorkstreamPr {
                id: ws.clone(),
                method: MergeMethod::Squash,
            },
        )
        .await
        .err();
    let mut settings: AppSettings = serde_json::from_value(
        j.call(&c, Request::GetAppSettings {})
            .await
            .expect("get_app_settings"),
    )
    .expect("decode the app settings");
    settings.agent_control.level = AgentControlLevel::Off;
    let set = j
        .call(&agent, Request::SetAppSettings { settings })
        .await
        .err();
    let kept = j
        .call(&c, Request::GetAppSettings {})
        .await
        .expect("get_app_settings");
    let never = |e: &Option<WireError>| {
        e.as_ref().is_some_and(|e| {
            e.code == "agent_control_denied" && e.message.contains("never open to agents")
        })
    };
    j.guard(
        "merge_workstream_pr and set_app_settings are never open to agents, even at full",
        never(&merge) && never(&set),
        json!({ "merge": wire(&merge), "setAppSettings": wire(&set) }),
    );
    j.guard(
        "the level the agent tried to change is still full",
        kept["agentControl"]["level"] == json!("full"),
        json!({ "level": kept["agentControl"]["level"] }),
    );
    let archive = j
        .call(
            &agent,
            Request::ArchiveWorkstream {
                id: ws.clone(),
                skip_scripts: true,
            },
        )
        .await
        .err();
    let still_open = c
        .list_workstreams(project.clone(), false)
        .await
        .expect("list_workstreams")
        .iter()
        .any(|w| w.item.id == ws);
    j.guard(
        "archive_workstream is never open to agents either, and the workstream stays",
        never(&archive)
            && archive
                .as_ref()
                .is_some_and(|e| e.message.contains("it ends a workstream"))
            && still_open,
        json!({ "archive": wire(&archive), "workstreamOpen": still_open }),
    );

    j.step("agent connections cannot open streams");
    let ptys_before = c
        .list_pty_agents(None)
        .await
        .expect("list_pty_agents")
        .len();
    let launch = agent
        .pty_launch_stream(
            ws.clone(),
            "claude-code".into(),
            PtyLaunchSession {
                session_id: None,
                resume: false,
                bypass: false,
                model: None,
                permission_mode: None,
                effort: None,
                first_prompt: None,
            },
            80,
            24,
        )
        .await
        .err();
    let start = agent
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            "from an agent".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .err();
    let ptys_after = c
        .list_pty_agents(None)
        .await
        .expect("list_pty_agents")
        .len();
    let sessions_now = c
        .list_sessions(ws.clone())
        .await
        .expect("list_sessions")
        .len();
    let no_stream = |e: &Option<WireError>| {
        e.as_ref().is_some_and(|e| {
            e.code == "agent_control_denied" && e.message.contains("streams are not open to agents")
        })
    };
    j.guard(
        "pty_launch and start_stage_session are refused as streams at full, and nothing started",
        no_stream(&launch)
            && no_stream(&start)
            && ptys_after == ptys_before
            && sessions_now == sessions_after,
        json!({
            "ptyLaunch": wire(&launch),
            "startStageSession": wire(&start),
            "newPtys": ptys_after - ptys_before,
            "newSessions": sessions_now - sessions_after,
        }),
    );

    j.step("off refuses every call");
    set_level(&mut j, &c, AgentControlLevel::Off).await;
    let off_whoami = mcp.call("whoami", json!({})).await;
    let off_list = mcp.call("list_workstreams", json!({})).await;
    let off_raw = j
        .call(
            &agent,
            Request::ListPtyAgents {
                workstream_id: None,
            },
        )
        .await
        .err();
    let fresh = server.connect().await;
    drop(fresh.take_events());
    let off_announce = j
        .call(
            &fresh,
            Request::AnnounceAgent {
                token: token.clone(),
            },
        )
        .await
        .err();
    let off_ctl = run_face(&["ctl", "whoami"], &t_env, b"").await;
    let off = |r: &Value| tool_failed(r) && tool_text(r).starts_with("agent_control_off");
    let off_wire = |e: &Option<WireError>| {
        e.as_ref().is_some_and(|e| {
            e.code == "agent_control_off"
                && e.remedy
                    .as_deref()
                    .is_some_and(|r| r.contains("Agent control"))
        })
    };
    j.guard(
        "an announced MCP face, a raw call, a fresh announce and ctl all get agent_control_off",
        off(&off_whoami)
            && off(&off_list)
            && off_wire(&off_raw)
            && off_wire(&off_announce)
            && off_ctl.code == Some(1)
            && off_ctl.stderr.starts_with("agent_control_off"),
        json!({
            "mcpWhoami": first_line(&tool_text(&off_whoami)),
            "mcpListWorkstreams": first_line(&tool_text(&off_list)),
            "raw": wire(&off_raw),
            "announce": wire(&off_announce),
            "ctl": { "exit": off_ctl.code, "stderr": first_line(&off_ctl.stderr) },
        }),
    );
    type_line(&c, &pty, "work").await;
    let states = pty_states(&mut rx, &pty).await;
    // The prompt also earns the tab a title; later lists record it, so it must have landed.
    let titled = titled(&c, &pty).await;
    j.guard(
        "the running PTY agent's hooks still fold at off, and its first prompt still earns a title",
        states == ["working", "idle"] && titled,
        json!({ "states": states, "titled": titled }),
    );
    set_level(&mut j, &c, AgentControlLevel::Read).await;

    j.step("a bypass caller resolves to full");
    let readme = RevealTarget::File {
        path: "README.md".into(),
    };
    j.call(
        &c,
        Request::SetSessionPermissionMode {
            session_id: session.clone(),
            mode: Some(PermissionMode::BypassPermissions),
        },
    )
    .await
    .expect("switch the session to bypass");
    let bypass_me = announce(&mut j, &server, &token).await;
    let mcp_me = whoami(&mut mcp).await;
    let revealed = j
        .call(
            &agent,
            Request::RequestReveal {
                target: readme.clone(),
            },
        )
        .await
        .is_ok();
    let reveal_event = events_until(&mut rx, IO, |e| {
        matches!(
            e,
            RuntimeEvent::UiRequested {
                request: UiRequest::Reveal { .. }
            }
        )
    })
    .await
    .iter()
    .any(|e| {
        matches!(
            e,
            RuntimeEvent::UiRequested {
                request: UiRequest::Reveal { workstream_id, .. }
            } if *workstream_id == ws
        )
    });
    j.guard(
        "switched to bypass at read: whoami says full, and a connection from before the switch may reveal",
        bypass_me["level"] == json!("full")
            && bypass_me["permissionMode"] == json!("bypassPermissions")
            && mcp_me["level"] == json!("full")
            && revealed
            && reveal_event,
        json!({
            "announced": { "level": bypass_me["level"], "mode": bypass_me["permissionMode"] },
            "mcpWhoami": mcp_me["level"],
            "revealOk": revealed,
            "revealReachedDesktop": reveal_event,
        }),
    );
    j.call(
        &c,
        Request::SetSessionPermissionMode {
            session_id: session.clone(),
            mode: Some(PermissionMode::Default),
        },
    )
    .await
    .expect("switch the session back to default");
    let default_me = announce(&mut j, &server, &token).await;
    let refused = j
        .call(&agent, Request::RequestReveal { target: readme })
        .await
        .err();
    j.guard(
        "switched back, the same connection is read again and the reveal is refused",
        default_me["level"] == json!("read")
            && refused
                .as_ref()
                .is_some_and(|e| e.code == "agent_control_denied" && e.message.contains("Full")),
        json!({ "level": default_me["level"], "reveal": wire(&refused) }),
    );

    j.step("agent connections receive no events");
    let listener = server.connect().await;
    let mut listener_rx = listener.take_events().expect("the agent's events");
    j.call(
        &listener,
        Request::AnnounceAgent {
            token: token.clone(),
        },
    )
    .await
    .expect("announce_agent");
    drain(&mut listener_rx);
    mcp.call(
        "notify_user",
        json!({ "title": "Barrier", "body": "after the agent announced" }),
    )
    .await;
    j.call(&c, Request::RedetectBinaries {})
        .await
        .expect("redetect_binaries");
    let desktop_saw = events_until(&mut rx, IO, |e| matches!(e, RuntimeEvent::BinariesChanged))
        .await
        .iter()
        .map(kind)
        .collect::<Vec<_>>();
    // Events are queued per connection in broadcast order, so a reply after them proves none is in flight.
    j.call(
        &listener,
        Request::ListPtyAgents {
            workstream_id: None,
        },
    )
    .await
    .expect("a round trip on the agent connection");
    let agent_heard = drain(&mut listener_rx);
    j.guard(
        "the desktop heard the notify and a broadcast; the agent connection heard nothing",
        desktop_saw.contains(&"UiRequested")
            && desktop_saw.contains(&"BinariesChanged")
            && agent_heard.is_empty(),
        json!({
            "desktopHeardNotify": desktop_saw.contains(&"UiRequested"),
            "desktopHeardBroadcast": desktop_saw.contains(&"BinariesChanged"),
            "agentHeard": agent_heard.iter().map(kind).collect::<Vec<_>>(),
        }),
    );

    j.step("an unannounced client never receives UiRequested");
    j.call(
        &quiet,
        Request::ListPtyAgents {
            workstream_id: None,
        },
    )
    .await
    .expect("a round trip on the unannounced connection");
    let heard = drain(&mut quiet_rx);
    let targeted = heard
        .iter()
        .filter(|e| {
            matches!(
                e,
                RuntimeEvent::UiRequested { .. } | RuntimeEvent::PtyAgentChanged { .. }
            )
        })
        .count();
    let broadcast = heard
        .iter()
        .any(|e| matches!(e, RuntimeEvent::BinariesChanged));
    j.guard(
        "after every notify so far it heard the broadcast, and no UiRequested or PtyAgentChanged",
        targeted == 0 && broadcast,
        json!({ "targeted": targeted, "heardBroadcast": broadcast }),
    );

    j.step("other projects are out of scope for reads");
    let turns_of = |id: &str| Request::ListSessionTurns {
        session_id: id.to_string(),
        cursor: LogCursor::Empty,
        limit: 20,
    };
    let own_turns = j.call(&agent, turns_of(&session)).await.is_ok();
    let other_turns = j.call(&agent, turns_of(&other_session)).await.err();
    let other_log = j
        .call(
            &agent,
            Request::GetSessionLog {
                session_id: other_session.clone(),
            },
        )
        .await
        .err();
    let read_of = |id: &str| Request::ReadTerminal {
        terminal_id: id.to_string(),
        lines: 20,
    };
    let own_screen = j.call(&agent, read_of(&pty)).await.is_ok();
    let other_screen = j.call(&agent, read_of(&other_pty)).await.err();
    let no_screen = j.call(&agent, read_of("not-a-terminal")).await.err();
    let other_git = j
        .call(
            &agent,
            Request::WorktreeGitStatus {
                id: other_ws.clone(),
            },
        )
        .await
        .err();
    let mcp_other = mcp
        .call("read_agent", json!({ "session_id": other_session }))
        .await;
    let elsewhere = |e: &Option<WireError>| {
        e.as_ref().is_some_and(|e| {
            e.code == "agent_control_denied" && e.message.contains("another project")
        })
    };
    j.guard(
        "turns, the log, the screen and git status of another project's ids are refused; the caller's own pass",
        own_turns
            && own_screen
            && elsewhere(&other_turns)
            && elsewhere(&other_log)
            && elsewhere(&other_screen)
            && elsewhere(&other_git)
            && tool_failed(&mcp_other)
            && tool_text(&mcp_other).contains("another project"),
        json!({
            "ownTurns": own_turns,
            "ownScreen": own_screen,
            "listSessionTurns": wire(&other_turns),
            "getSessionLog": wire(&other_log),
            "readTerminal": wire(&other_screen),
            "worktreeGitStatus": wire(&other_git),
            "mcpReadAgent": first_line(&tool_text(&mcp_other)),
        }),
    );
    j.guard(
        "read_terminal refuses an id no PTY agent holds",
        no_screen.as_ref().is_some_and(|e| {
            e.code == "agent_control_denied" && e.message.contains("not a running PTY agent")
        }),
        json!({ "readTerminal": wire(&no_screen) }),
    );
    let other_project = other.clone();
    let own_project = project.clone();
    let other_ws_id = other_ws.clone();
    let requests: [(&str, MakeRequest); 5] = [
        (
            "listWorkstreams(other)",
            Box::new(move || Request::ListWorkstreams {
                project_path: other_project.clone(),
                include_archived: false,
            }),
        ),
        (
            "listWorkstreams(own)",
            Box::new(move || Request::ListWorkstreams {
                project_path: own_project.clone(),
                include_archived: false,
            }),
        ),
        (
            "listSessions(other)",
            Box::new(move || Request::ListSessions {
                workstream_id: other_ws_id.clone(),
            }),
        ),
        (
            "listActiveSessions",
            Box::new(|| Request::ListActiveSessions {}),
        ),
        (
            "listPtyAgents",
            Box::new(|| Request::ListPtyAgents {
                workstream_id: None,
            }),
        ),
    ];
    // Sorted by role, not by id: raw ids sort differently on every run.
    let roles = [
        (ws.as_str(), "ownWorkstream"),
        (other_ws.as_str(), "otherWorkstream"),
        (session.as_str(), "ownSession"),
        (other_session.as_str(), "otherSession"),
        (pty.as_str(), "ownPty"),
        (other_pty.as_str(), "otherPty"),
    ];
    let named = |rows: &Value| -> Vec<String> {
        sorted(
            row_ids(rows)
                .iter()
                .map(|id| {
                    roles
                        .iter()
                        .find(|(known, _)| known == id)
                        .map_or("unknown", |(_, role)| role)
                        .to_string()
                })
                .collect(),
        )
    };
    let mut lists = Vec::new();
    for (name, request) in &requests {
        let agent_rows = j.call(&agent, request()).await.unwrap_or(Value::Null);
        let desktop_rows = j.call(&c, request()).await.unwrap_or(Value::Null);
        lists.push(json!({
            "list": name,
            "agent": named(&agent_rows),
            "desktop": named(&desktop_rows),
        }));
    }
    let rows = |name: &str, side: &str| {
        lists
            .iter()
            .find(|l| l["list"] == json!(name))
            .map(|l| l[side].clone())
            .unwrap_or(Value::Null)
    };
    j.guard(
        "list verbs keep only the caller's project, though the desktop sees both",
        rows("listWorkstreams(other)", "agent") == json!([])
            && rows("listWorkstreams(other)", "desktop") == json!(["otherWorkstream"])
            && rows("listWorkstreams(own)", "agent") == json!(["ownWorkstream"])
            && rows("listSessions(other)", "agent") == json!([])
            && rows("listSessions(other)", "desktop") == json!(["otherSession"])
            && rows("listActiveSessions", "agent") == json!(["ownSession"])
            && rows("listActiveSessions", "desktop") == json!(["otherSession", "ownSession"])
            && rows("listPtyAgents", "agent") == json!(["ownPty"])
            && rows("listPtyAgents", "desktop") == json!(["otherPty", "ownPty"]),
        json!({ "lists": lists }),
    );
    let own_doc = shared_note(&mut j, &c, &project, "Own note").await;
    let other_doc = shared_note(&mut j, &c, &other, "Other note").await;
    let everything = || Request::QueryDocs {
        query: ThoughtQuery::default(),
    };
    let first = || Request::QueryDocs {
        query: ThoughtQuery {
            limit: Some(1),
            ..ThoughtQuery::default()
        },
    };
    let agent_all = j.call(&agent, everything()).await.unwrap_or(Value::Null);
    let agent_page = j.call(&agent, first()).await.unwrap_or(Value::Null);
    let desktop_all = j.call(&c, everything()).await.unwrap_or(Value::Null);
    let doc_ids = |result: &Value| ids(&result["items"], "/id");
    let agent_ids = doc_ids(&agent_all);
    let agent_total = agent_all["total"].as_u64();
    let unread = agent_all["items"].as_array().map_or(0, |items| {
        items.iter().filter(|d| d["unread"] == json!(true)).count()
    });
    j.guard(
        "query_docs counts only what the agent may read: its total, project count and unread match its items, and a page keeps that total",
        agent_ids.contains(&own_doc)
            && !agent_ids.contains(&other_doc)
            && doc_ids(&desktop_all).contains(&other_doc)
            && agent_total == Some(agent_ids.len() as u64)
            && agent_all["projects"] == json!(1)
            && agent_all["unread"].as_u64() == Some(unread as u64)
            && desktop_all["total"].as_u64() > agent_total
            && agent_page["items"].as_array().map(Vec::len) == Some(1)
            && agent_page["total"].as_u64() == agent_total,
        json!({
            "seesOwnDoc": agent_ids.contains(&own_doc),
            "seesOtherDoc": agent_ids.contains(&other_doc),
            "totalMatchesItems": agent_total == Some(agent_ids.len() as u64),
            "projects": agent_all["projects"],
            "unreadMatchesItems": agent_all["unread"].as_u64() == Some(unread as u64),
            "desktopCountsMore": desktop_all["total"].as_u64() > agent_total,
            "pageItems": agent_page["items"].as_array().map(Vec::len),
            "pageKeepsTotal": agent_page["total"].as_u64() == agent_total,
        }),
    );

    j.step("an agent connection cannot shut the server down");
    agent
        .request_shutdown()
        .await
        .expect("send a shutdown from the agent");
    let logged = eventually(IO, || {
        sandbox
            .server_log()
            .contains("refused an agent connection's shutdown")
    })
    .await;
    let exited = server.exited(Duration::from_secs(1)).await;
    let still_answers = j
        .call(
            &agent,
            Request::ListPtyAgents {
                workstream_id: None,
            },
        )
        .await
        .is_ok();
    let reconnects = RpcClient::connect_socket(server.socket()).await.is_ok();
    j.guard(
        "Control::Shutdown from the agent is logged and refused; the server and the connection stay up",
        logged && exited.is_none() && still_answers && reconnects,
        json!({
            "logged": logged,
            "exited": exited.map(|s| s.code()),
            "connectionAnswers": still_answers,
            "reconnects": reconnects,
        }),
    );
    // The sandbox profile too, so a regressed guard could only ever stop this test's server.
    let mut face_env: Vec<(String, String)> = sandbox
        .server_env()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned()))
        .collect();
    face_env.extend(s_env.iter().cloned());
    let stop = run_face(&["stop"], &face_env, b"").await;
    let relay = run_face(&["relay"], &face_env, b"").await;
    let exited = server.exited(Duration::from_secs(1)).await;
    let refusal = "not available inside a hyprlayer agent";
    j.guard(
        "`stop` and `relay` under an agent token exit 2 and the server keeps running",
        stop.code == Some(2)
            && stop.stderr.contains(refusal)
            && relay.code == Some(2)
            && relay.stderr.contains(refusal)
            && exited.is_none(),
        json!({
            "stop": { "exit": stop.code, "stderr": stop.stderr.trim() },
            "relay": { "exit": relay.code, "stderr": relay.stderr.trim() },
            "serverExited": exited.is_some(),
        }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "the desktop's shutdown still stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn start_claude(
    j: &mut Journey,
    c: &RpcClient,
    ws: &str,
    alias: &str,
) -> (String, EventStream<SessionEvent>) {
    let (session, _prompt, _stream_id, events) = c
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_AGENT".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, alias);
    j.call(
        c,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    (session.id, events)
}

/// A hooked PTY claude that has fired `SessionStart`.
async fn launch_hooked(
    j: &mut Journey,
    c: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
    alias: &str,
) -> (String, TerminalStream) {
    let (launched, events) = launch_pty(c, ws, "claude-code").await;
    assert!(launched.hooked, "PTY claude launches hooked at read");
    j.alias(&launched.terminal_id, alias);
    if let Some(sid) = &launched.provider_session_id {
        j.alias(sid, format!("{alias}_SESSION"));
    }
    appears(&launch_log(sandbox, &launched.terminal_id).join("hooks.log")).await;
    (launched.terminal_id, events)
}

/// An agent's `HYPRLAYER_*` env once its token is written; the token is aliased before anything records it.
async fn identity(
    j: &mut Journey,
    sandbox: &Sandbox,
    id: &str,
    alias: &str,
) -> Vec<(String, String)> {
    let written = eventually(IO, || {
        !env_var(&agent_env(sandbox, id), "HYPRLAYER_AGENT_TOKEN").is_empty()
    })
    .await;
    assert!(written, "{alias}: the agent never wrote its env");
    let env = agent_env(sandbox, id);
    j.alias(env_var(&env, "HYPRLAYER_AGENT_TOKEN"), alias);
    env
}

async fn connect_agent(j: &mut Journey, server: &Server, token: &str) -> std::sync::Arc<RpcClient> {
    let agent = server.connect().await;
    drop(agent.take_events());
    j.call(
        &agent,
        Request::AnnounceAgent {
            token: token.to_string(),
        },
    )
    .await
    .expect("announce_agent");
    agent
}

/// `announce_agent` on a fresh connection: the level as of now.
async fn announce(j: &mut Journey, server: &Server, token: &str) -> Value {
    let probe = server.connect().await;
    drop(probe.take_events());
    j.call(
        &probe,
        Request::AnnounceAgent {
            token: token.to_string(),
        },
    )
    .await
    .unwrap_or(Value::Null)
}

struct Listed {
    names: Vec<String>,
    all_read_only: bool,
}

async fn tool_list(mcp: &mut McpDriver) -> Listed {
    let reply = mcp.request("tools/list", json!({})).await;
    let tools = reply["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Listed {
        names: sorted(tools.iter().map(|t| text(t, "/name")).collect()),
        all_read_only: !tools.is_empty()
            && tools
                .iter()
                .all(|t| t["annotations"]["readOnlyHint"] == json!(true)),
    }
}

async fn whoami(mcp: &mut McpDriver) -> Value {
    parse(&mcp.call("whoami", json!({})).await)
}

/// A successful tool result's JSON.
fn parse(result: &Value) -> Value {
    serde_json::from_str(&tool_text(result)).unwrap_or(Value::Null)
}

fn ids(items: &Value, pointer: &str) -> Vec<String> {
    items
        .as_array()
        .map(|a| a.iter().map(|v| text(v, pointer)).collect())
        .unwrap_or_default()
}

/// The ids of a list response's rows, whatever the row shape.
fn row_ids(rows: &Value) -> Vec<String> {
    let id = |row: &Value| {
        ["/item/id", "/session/id", "/terminalId", "/id"]
            .iter()
            .map(|p| text(row, p))
            .find(|s| !s.is_empty())
            .unwrap_or_default()
    };
    rows.as_array()
        .map(|a| a.iter().map(id).collect())
        .unwrap_or_default()
}

fn sorted(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items
}

/// The space-split tokens a flag's value spans, up to the next flag.
fn flag_values(argv: &[String], flag: &str) -> Vec<String> {
    argv.iter()
        .skip_while(|a| *a != flag)
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect()
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}

fn wire(error: &Option<WireError>) -> Value {
    match error {
        Some(e) => json!({ "code": e.code, "message": e.message }),
        None => json!("accepted"),
    }
}

/// A shared note in `project`'s thoughts; returns its id.
async fn shared_note(j: &mut Journey, c: &RpcClient, project: &str, title: &str) -> String {
    let meta = j
        .call(
            c,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.to_string()),
                    scope: "shared".into(),
                    doc_type: "note".into(),
                    title: title.into(),
                    date: "2026-01-01".into(),
                    ticket: None,
                    tags: vec![],
                },
            },
        )
        .await
        .expect("create_doc");
    text(&meta, "/id")
}

/// The first `Notify` titled `title`, shaped as `notice` shapes one; `Null` if none comes.
async fn notify(rx: &mut Receiver<RuntimeEvent>, title: &str) -> Value {
    let matching = |e: &RuntimeEvent| match e {
        RuntimeEvent::UiRequested {
            request:
                UiRequest::Notify {
                    workstream_id,
                    title: t,
                    body,
                    session_id,
                    terminal_id,
                },
        } if t == title => Some(json!({
            "workstreamId": workstream_id,
            "body": body,
            "namesSessionOrTerminal": session_id.is_some() || terminal_id.is_some(),
        })),
        _ => None,
    };
    events_until(rx, IO, |e| matching(e).is_some())
        .await
        .iter()
        .find_map(matching)
        .unwrap_or(Value::Null)
}

/// An agent's notify: the caller's workstream, naming no session or terminal to focus.
fn notice(ws: &str, body: &str) -> Value {
    json!({ "workstreamId": ws, "body": body, "namesSessionOrTerminal": false })
}

/// The terminal's state sequence, repeats collapsed, until it reads idle.
async fn pty_states(rx: &mut Receiver<RuntimeEvent>, terminal: &str) -> Vec<String> {
    let state_of = |e: &RuntimeEvent| match e {
        RuntimeEvent::PtyAgentChanged {
            terminal_id, state, ..
        } if terminal_id == terminal => Some(*state),
        _ => None,
    };
    let mut out: Vec<String> = Vec::new();
    for state in events_until(rx, IO, |e| state_of(e) == Some(PtyAgentState::Idle))
        .await
        .iter()
        .filter_map(state_of)
    {
        let state = json!(state).as_str().unwrap_or_default().to_string();
        if out.last() != Some(&state) {
            out.push(state);
        }
    }
    out
}

fn drain(rx: &mut Receiver<RuntimeEvent>) -> Vec<RuntimeEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn kind(event: &RuntimeEvent) -> &'static str {
    match event {
        RuntimeEvent::UiRequested { .. } => "UiRequested",
        RuntimeEvent::BinariesChanged => "BinariesChanged",
        RuntimeEvent::PtyAgentChanged { .. } => "PtyAgentChanged",
        _ => "other",
    }
}

/// Waits for the PTY agent's generated title, which lands a moment after its first prompt.
async fn titled(c: &RpcClient, terminal: &str) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let listed = c.list_pty_agents(None).await.expect("list_pty_agents");
        if listed
            .iter()
            .any(|a| a.terminal_id == terminal && a.title.is_some())
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}
