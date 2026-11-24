//! The breadth verbs an agent at `full` drives: PTY agents it spawns, messages, reads and waits
//! on, and the workflow around them (workstreams, plans, goals, scripts, review comments,
//! reveals). Merge and archive refusals live in `agent_control_read`.
//!
//! The caller is `goal_stub`'s claude stand-in driven through `hyprlayer-server mcp`; its PTY
//! children are `agent_stub`'s PTY claude. Nothing here covers the real binaries.
#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use hyprlayer_core::models::goal::GoalCheck;
use hyprlayer_core::models::{AgentControlLevel, PermissionMode, UiRequest};
use hyprlayer_core::runtime::goals::GoalPatch;
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::{Harness, ScriptKind, SessionEvent, Stage};
use hyprlayer_e2e::agent_stub::{
    agent_env, env_var, install, launch_log, lines, set_engine, set_level, tool_failed, tool_text,
    update_settings, McpDriver, PTY_CLAUDE,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{
    events_until, eventually, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;
use tokio::sync::mpsc::Receiver;

const IO: Duration = Duration::from_secs(30);
const GOAL_WAIT: Duration = Duration::from_secs(90);
/// Under `McpDriver`'s 30 s reply wait.
const WAIT_SECS: u64 = 20;

const SETTINGS: &str = r#"[scripts]
setup = 'pwd >> "$HOME/e2e-setup-ran"'
run = 'printf "E2E_RUN_OK\n"'
archive = 'touch "$HOME/e2e-archive-ran"'
"#;

/// Colors, a title, words placed by column moves (as claude places them) and a move far past the
/// edge before the stub runs, so `read_terminal` has escapes to strip.
const ANSI_CLAUDE: &str = "#!/bin/sh\nprintf '\\033]0;E2E title\\007\\033[1;32mE2E_READY\\033[0m\\nE2E\\033[6Gspaced\\033[14Gwords\\nE2E_FAR\\033[999999999CEDGE\\n'\nexec \"$(dirname \"$0\")/pty-claude\" \"$@\"\n";

const PLAN: &str = "---\ntitle: Breadth plan\ntype: plan\nstatus: draft\nauthor: e2e\n---\n\n\
# Breadth plan\n\n## Phase 1\n\n- [ ] Raise the counter\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_breadth() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    install(&sandbox, "pty-claude", PTY_CLAUDE);
    let pty_claude = install(&sandbox, "ansi-claude", ANSI_CLAUDE);
    let user_settings = sandbox.root.join("user-settings.json");
    std::fs::write(&user_settings, "{}\n").expect("write the user's settings");
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# breadth\n"),
            (".hyprlayer/settings.toml", SETTINGS),
        ],
    );
    let mut j = Journey::new("agent_breadth", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let mut rx = c.take_events().expect("the desktop's runtime events");
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
    let view = create_workstream(&mut j, &c, &project, "Breadth").await;
    let ws = text(&view, "/item/id");
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    let lead = start_lead(&mut j, &c, &ws).await;
    let lead_env = env_of(&sandbox, &lead).await;
    j.alias(env_var(&lead_env, "HYPRLAYER_AGENT_TOKEN"), "$LEAD_TOKEN");
    let mut mcp = McpDriver::start(&lead_env, false);
    mcp.initialize("2025-06-18").await;

    j.step("a spawned PTY agent opens an adopted tab");
    let spawned = parsed(
        &mcp.call("spawn_agent", json!({ "terminal_engine": "claude-code" }))
            .await,
    );
    let term = text(&spawned, "/terminalId");
    j.alias(&term, "$PTY");
    let tab = events_until(&mut rx, IO, |e| is_terminal_tab(e, &term))
        .await
        .into_iter()
        .find(|e| is_terminal_tab(e, &term))
        .and_then(|e| match e {
            RuntimeEvent::UiRequested { request } => serde_json::to_value(request).ok(),
            _ => None,
        });
    if let Some(sid) = tab.as_ref().map(|t| text(t, "/providerSessionId")) {
        j.alias(sid, "$PTY_SESSION");
    }
    let log = launch_log(&sandbox, &term);
    eventually(IO, || {
        lines(&log.join("hooks.log"))
            .iter()
            .any(|l| l.starts_with("SessionStart"))
    })
    .await;
    j.alias(
        env_var(&agent_env(&sandbox, &term), "HYPRLAYER_AGENT_TOKEN"),
        "$PTY_TOKEN",
    );
    let listed = pty_agent(&mut j, &c, &ws, &term).await;
    j.guard(
        "every announced desktop gets an OpenTerminalTab for it, not in bypass",
        tab.as_ref().is_some_and(|t| {
            t["engineId"] == json!("claude-code")
                && t["workstreamId"] == json!(ws)
                && t["bypass"] == json!(false)
                && t["providerSessionId"].is_string()
        }),
        json!({ "tab": tab }),
    );
    j.guard(
        "it is listed hooked, spawned by the caller",
        listed["hooked"] == json!(true)
            && listed["spawnedBy"] == json!({ "kind": "session", "sessionId": lead }),
        json!({ "hooked": listed["hooked"], "spawnedBy": listed["spawnedBy"], "state": listed["state"] }),
    );

    j.step("an engine nobody configured is refused");
    let raw = refusal(
        &mcp.call("spawn_agent", json!({ "terminal_engine": "bash" }))
            .await,
    );
    let strays = c
        .list_pty_agents(Some(ws.clone()))
        .await
        .expect("list_pty_agents")
        .iter()
        .filter(|a| a.terminal_id != term)
        .count();
    j.guard(
        "terminal_engine bash is agent_control_denied and no PTY starts",
        raw.starts_with("agent_control_denied: ")
            && raw.contains("not a configured engine")
            && strays == 0,
        json!({ "refusal": raw, "otherPtys": strays }),
    );

    j.step("a message is bracketed paste then Enter");
    let reply_before = listed["lastReply"].clone();
    // The stub acts on whole lines, so the paste's middle line is its `work` command.
    let answered = mcp
        .call(
            "prompt_and_wait",
            json!({ "terminal_id": term, "text": "work\nE2E_END", "timeout_sec": WAIT_SECS }),
        )
        .await;
    let header = format!("[from agent \"Lead\" · {lead}]");
    let expected = vec![
        format!("\x1b[200~{header}"),
        String::new(),
        "work".into(),
        "E2E_END\x1b[201~".into(),
    ];
    eventually(IO, || lines(&log.join("stdin.log")) == expected).await;
    let logged = lines(&log.join("stdin.log"));
    j.guard(
        "the TUI reads ESC[200~ and the header, a blank line, the text, then ESC[201~ and the line's end",
        !tool_failed(&answered) && logged == expected,
        json!({
            "sent": !tool_failed(&answered),
            "matchesThePaste": logged == expected,
            "opensWithHeader": logged.first().is_some_and(|l| l.starts_with("\x1b[200~[from agent")),
            "closesOnText": logged.last().is_some_and(|l| l == "E2E_END\x1b[201~"),
        }),
    );

    j.step("wait_pty_agent returns the last reply");
    let waited = outcome(&answered);
    j.guard(
        "prompt_and_wait blocks through the turn it started: no reply before, Stop's reply after",
        reply_before.is_null()
            && waited["state"] == json!("idle")
            && waited["lastReply"] == json!("done: work")
            && waited["stalled"] == json!(false)
            && waited["timedOut"] == json!(false),
        json!({ "replyBefore": reply_before, "outcome": waited }),
    );
    // The `work` line also earns the tab a title; later lists record it, so it must have landed.
    titled(&c, &ws, &term).await;

    j.step("a message is refused while the target asks");
    // From the keyboard: the stub runs a pasted line before its Enter, and an ask opening under a
    // message makes it hold that Enter back.
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("ask\r"))
        .await
        .expect("terminal_write");
    let asking = outcome(
        &mcp.call(
            "wait_agent",
            json!({ "terminal_id": term, "until": "needs_you", "timeout_sec": WAIT_SECS }),
        )
        .await,
    );
    let before = lines(&log.join("stdin.log")).len();
    let refused = refusal(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": term, "text": "yes" }),
        )
        .await,
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = lines(&log.join("stdin.log")).len();
    j.guard(
        "while it asks for permission a message is agent_control_denied and nothing reaches the TUI",
        asking["state"] == json!("needs_you")
            && refused.starts_with("agent_control_denied: ")
            && refused.contains("asking its user something")
            && after == before,
        json!({ "waited": asking["state"], "refusal": refused, "typedSince": after - before }),
    );
    // The person's own answer, which also settles the ask.
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("n\r"))
        .await
        .expect("terminal_write");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline
        && pty_state(&c, &ws, &term).await.as_deref() != Some("idle")
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let settled = outcome(
        &mcp.call(
            "wait_agent",
            json!({ "terminal_id": term, "timeout_sec": WAIT_SECS }),
        )
        .await,
    );
    j.guard(
        "the person's answer settles it, and the old reply stands",
        settled["state"] == json!("idle") && settled["lastReply"] == json!("done: work"),
        json!({ "outcome": settled }),
    );

    j.step("a slash command to a busy agent waits for its next idle");
    let stdin = log.join("stdin.log");
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("hold\r"))
        .await
        .expect("terminal_write");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline
        && pty_state(&c, &ws, &term).await.as_deref() != Some("working")
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let before = lines(&stdin).len();
    // The stub acts on whole lines, so the paste's `work` line ends the turn the command starts.
    let held = tool_text(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": term, "text": "/echo-args\nwork\nE2E_HELD" }),
        )
        .await,
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let typed_while_busy = lines(&stdin).len() - before;
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("work\r"))
        .await
        .expect("terminal_write");
    let expected = vec![
        format!("/echo-args \x1b[200~{header}"),
        String::new(),
        "work".into(),
        "E2E_HELD\x1b[201~".into(),
    ];
    let typed_at_idle = eventually(IO, || lines(&stdin).ends_with(&expected)).await;
    j.guard(
        "mid-turn, the command is queued and nothing reaches the TUI; at the next idle it is typed with the rest pasted after it",
        held.starts_with("\"queued: ") && typed_while_busy == 0 && typed_at_idle,
        json!({ "reply": held, "typedWhileBusy": typed_while_busy, "typedAtIdle": typed_at_idle }),
    );

    j.step("an interrupting message stops the turn and is the next one");
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("hold\r"))
        .await
        .expect("terminal_write");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline
        && pty_state(&c, &ws, &term).await.as_deref() != Some("working")
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let reply = tool_text(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": term, "text": "work\nE2E_INTERRUPT", "delivery": "interrupt" }),
        )
        .await,
    );
    // The stub reads lines, so the Esc leads the paste that follows the settle.
    let expected = vec![
        format!("\x1b\x1b[200~{header}"),
        String::new(),
        "work".into(),
        "E2E_INTERRUPT\x1b[201~".into(),
    ];
    let typed = eventually(IO, || lines(&stdin).ends_with(&expected)).await;
    j.guard(
        "mid-turn, delivery interrupt sends Esc, and once the turn settles the message is typed as the next turn",
        reply.starts_with("\"interrupted: ") && typed,
        json!({ "reply": reply, "typedAfterTheEsc": typed }),
    );

    j.step("a message is refused while the target shows a prompt");
    let stdin = log.join("stdin.log");
    let typed = lines(&stdin).len();
    // A person's own /model picker: no hook reports it, so the agent still reads idle.
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("picker\r"))
        .await
        .expect("terminal_write");
    eventually(IO, || lines(&stdin).len() == typed + 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let state = pty_state(&c, &ws, &term).await;
    let before = lines(&stdin).len();
    let refused = refusal(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": term, "text": "E2E_PICKED" }),
        )
        .await,
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = lines(&stdin).len();
    // The person closes it; the stub repaints its input box.
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("\r"))
        .await
        .expect("terminal_write");
    eventually(IO, || lines(&stdin).len() == after + 1).await;
    j.guard(
        "idle with a picker over its input box, a message is agent_control_denied and nothing reaches the TUI",
        state.as_deref() == Some("idle")
            && refused == "agent_control_denied: agent control refused this: the agent is showing a prompt"
            && after == before,
        json!({ "state": state, "refusal": refused, "typedSince": after - before }),
    );

    j.step("read_terminal returns the stripped tail");
    // The stub repaints after it logs the line, and a repaint can reach the ring a few ms later.
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let tail = tool_text(
            &mcp.call("read_terminal", json!({ "terminal_id": term, "lines": 2 }))
                .await,
        );
        if tail.contains("Try") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let screen = tool_text(
        &mcp.call(
            "read_terminal",
            json!({ "terminal_id": term, "lines": 500 }),
        )
        .await,
    );
    let screen: String = serde_json::from_str(&screen).unwrap_or_default();
    let two = tool_text(
        &mcp.call("read_terminal", json!({ "terminal_id": term, "lines": 2 }))
            .await,
    );
    let two: String = serde_json::from_str(&two).unwrap_or_default();
    j.guard(
        "the agent's colored banner reads as plain text, its column moves as spaces up to the terminal's edge, with no escape sequence or title left",
        screen.lines().any(|l| l == "E2E_READY")
            && screen.lines().any(|l| l == "E2E  spaced  words")
            && screen.lines().all(|l| l.chars().count() <= 200)
            && !screen.contains('\x1b')
            && !screen.contains("E2E title")
            && screen.contains("E2E_END"),
        json!({
            "banner": screen.lines().any(|l| l == "E2E_READY"),
            "spaced": screen.lines().any(|l| l == "E2E  spaced  words"),
            "widest": screen.lines().map(|l| l.chars().count()).max(),
            "escapes": screen.contains('\x1b'),
            "title": screen.contains("E2E title"),
            "echoedMessage": screen.contains("E2E_END"),
        }),
    );
    j.guard(
        "lines bounds the tail",
        two.lines().count() <= 2 && screen.ends_with(&two),
        json!({ "lines": two.lines().count(), "isTheTail": screen.ends_with(&two) }),
    );

    j.step("unhooked agents cannot be waited on");
    let user = user_settings.display().to_string();
    set_engine(
        &mut j,
        &c,
        "claude-code",
        &pty_claude,
        &["--settings", &user],
    )
    .await;
    let plain = parsed(
        &mcp.call("spawn_agent", json!({ "terminal_engine": "claude-code" }))
            .await,
    );
    let plain_term = text(&plain, "/terminalId");
    j.alias(&plain_term, "$UNHOOKED_PTY");
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    eventually(IO, || {
        launch_log(&sandbox, &plain_term).join("env").exists()
    })
    .await;
    j.alias(
        env_var(&agent_env(&sandbox, &plain_term), "HYPRLAYER_AGENT_TOKEN"),
        "$UNHOOKED_TOKEN",
    );
    let listed = pty_agent(&mut j, &c, &ws, &plain_term).await;
    let no_wait = refusal(
        &mcp.call(
            "wait_agent",
            json!({ "terminal_id": plain_term, "timeout_sec": 1 }),
        )
        .await,
    );
    let readable = !tool_failed(
        &mcp.call("read_terminal", json!({ "terminal_id": plain_term }))
            .await,
    );
    let typed_before = lines(&launch_log(&sandbox, &plain_term).join("stdin.log")).len();
    let no_message = refusal(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": plain_term, "text": "E2E_UNHOOKED" }),
        )
        .await,
    );
    let typed_after = lines(&launch_log(&sandbox, &plain_term).join("stdin.log")).len();
    let killed = mcp
        .call("kill_agent", json!({ "terminal_id": plain_term }))
        .await;
    let gone = gone_from_list(&c, &ws, &plain_term).await;
    j.guard(
        "an agent-spawned unhooked PTY is listed unhooked, readable, and its wait is refused",
        listed["hooked"] == json!(false)
            && no_wait.starts_with("agent_control_denied: ")
            && no_wait.contains("reports no state to wait on")
            && readable,
        json!({ "hooked": listed["hooked"], "refusal": no_wait, "readable": readable }),
    );
    j.guard(
        "a Default caller's message to an unhooked PTY is refused, since nothing reports a prompt it may show",
        no_message.starts_with("agent_control_denied: ")
            && no_message.contains("reports no state")
            && typed_after == typed_before,
        json!({ "refusal": no_message, "typedSince": typed_after - typed_before }),
    );
    j.guard(
        "the caller can kill the PTY it spawned",
        !tool_failed(&killed) && gone,
        json!({ "kill": tool_text(&killed), "unlisted": gone }),
    );

    j.step("a non-reply terminal_write resets a PTY's hops");
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 2).await;
    let mut hops = Vec::new();
    for _ in 0..3 {
        hops.push(refusal(
            &mcp.call(
                "message_agent",
                json!({ "terminal_id": term, "text": "E2E_HOP" }),
            )
            .await,
        ));
    }
    // A bare Enter: a person at the keyboard, which the stub reads as an empty line.
    c.terminal_write(term.clone(), hyprlayer_e2e::keystrokes("\r"))
        .await
        .expect("terminal_write");
    let after_human = refusal(
        &mcp.call(
            "message_agent",
            json!({ "terminal_id": term, "text": "E2E_HOP" }),
        )
        .await,
    );
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 6).await;
    j.guard(
        "two messages pass, the third is agent_limit_reached, and the desktop's keystroke lets the next through",
        hops[..2].iter().all(|h| h == "ok")
            && hops[2].starts_with("agent_limit_reached: ")
            && hops[2].contains("got 2 agent messages with no human turn")
            && after_human == "ok",
        json!({ "messages": hops, "afterKeystroke": after_human }),
    );

    j.step("child PTYs count toward max_live_children");
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 1).await;
    let crowded = refusal(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_CROWDED", "title": "Crowded" }),
        )
        .await,
    );
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 4).await;
    j.guard(
        "with its PTY child live, Max live children 1 refuses a session child",
        crowded.starts_with("agent_limit_reached: ")
            && crowded.contains("already has 1 live children"),
        json!({ "refusal": crowded }),
    );

    j.step("depth carries through a terminal parent");
    let mut pty_mcp = McpDriver::start(&agent_env(&sandbox, &term), false);
    pty_mcp.initialize("2025-06-18").await;
    let pty_who = parsed(&pty_mcp.call("whoami", json!({})).await);
    let spawned = parsed(
        &pty_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_UNDER_PTY", "title": "Under PTY" }),
            )
            .await,
    );
    let under = text(&spawned, "/sessionId");
    j.alias(&under, "$UNDER_PTY");
    let under_env = env_of(&sandbox, &under).await;
    j.alias(
        env_var(&under_env, "HYPRLAYER_AGENT_TOKEN"),
        "$UNDER_PTY_TOKEN",
    );
    let mut under_mcp = McpDriver::start(&under_env, false);
    under_mcp.initialize("2025-06-18").await;
    let under_who = parsed(&under_mcp.call("whoami", json!({})).await);
    let too_deep = refusal(
        &under_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_TOO_DEEP", "title": "Too deep" }),
            )
            .await,
    );
    let cleaned = pty_mcp
        .call("kill_agent", json!({ "session_id": under }))
        .await;
    drop((pty_mcp, under_mcp));
    j.guard(
        "the PTY child is depth 1 under the lead; the session it spawns is depth 2 under the terminal",
        pty_who["depth"] == json!(1)
            && pty_who["parent"] == json!({ "kind": "session", "sessionId": lead })
            && under_who["depth"] == json!(2)
            && under_who["parent"] == json!({ "kind": "terminal", "terminalId": term }),
        json!({
            "pty": { "depth": pty_who["depth"], "parent": pty_who["parent"] },
            "session": { "depth": under_who["depth"], "parent": under_who["parent"] },
        }),
    );
    j.guard(
        "so a third level is refused at Max depth 2, and the terminal can stop its own child",
        too_deep.starts_with("agent_limit_reached: ")
            && too_deep.contains("a child would be 3 agents deep")
            && !tool_failed(&cleaned),
        json!({ "refusal": too_deep, "kill": tool_text(&cleaned) }),
    );
    // The hop Notify and the child's tab are not what later steps wait on.
    drain(&mut rx);

    j.step("create_workstream runs the setup script");
    let bad_mode = refusal(
        &mcp.call(
            "create_workstream",
            json!({ "title": "Attached", "worktree_mode": "attached" }),
        )
        .await,
    );
    let made = parsed(
        &mcp.call("create_workstream", json!({ "title": "Spawned work" }))
            .await,
    );
    let changed = |e: &RuntimeEvent| matches!(e, RuntimeEvent::WorkstreamsChanged { project_path } if *project_path == project);
    let announced = events_until(&mut rx, IO, changed).await.iter().any(changed);
    let new_ws = text(&made, "/id");
    let new_tree = text(&made, "/worktreePath");
    j.alias(&new_ws, "$AGENT_WS");
    if let Some(slug) = Path::new(&new_tree).file_name() {
        let slug = slug.to_string_lossy().into_owned();
        if let Some((stem, _)) = slug.rsplit_once('-') {
            j.alias(&slug, format!("{stem}-$AGENT_SUFFIX"));
        }
    }
    let ran = sandbox.home.join("e2e-setup-ran");
    let setup_ran = eventually(IO, || lines(&ran).contains(&new_tree)).await;
    let listed_ws = j
        .call(
            &c,
            Request::ListWorkstreams {
                project_path: project.clone(),
                include_archived: false,
            },
        )
        .await
        .expect("list_workstreams");
    let in_project = listed_ws
        .as_array()
        .is_some_and(|all| all.iter().any(|w| text(w, "/item/id") == new_ws));
    j.guard(
        "the MCP face offers only the isolated and existing modes",
        bad_mode.starts_with("invalid_arguments: "),
        json!({ "attached": bad_mode }),
    );
    j.guard(
        "the agent's workstream lands in its project and the setup script runs in its worktree",
        in_project && setup_ran && made["title"] == json!("Spawned work"),
        json!({ "title": made["title"], "inProject": in_project, "setupRanInWorktree": setup_ran }),
    );
    j.guard(
        "the desktop hears WorkstreamsChanged for the project, so its sidebar lists the agent's workstream",
        announced,
        json!({ "workstreamsChanged": announced }),
    );

    j.step("documents outside the project stay read-only");
    let token = env_var(&lead_env, "HYPRLAYER_AGENT_TOKEN").to_string();
    let agent = server.connect().await;
    drop(agent.take_events());
    agent.announce_agent(token).await.expect("announce_agent");
    let global = j
        .call(
            &c,
            Request::CreateDoc {
                input: note(None, "global", "Breadth global"),
            },
        )
        .await
        .expect("create the global doc");
    let global_id = text(&global, "/id");
    j.call(
        &c,
        Request::AttachWorkstreamDoc {
            id: ws.clone(),
            doc_path: global_id.clone(),
            role: "context".into(),
        },
    )
    .await
    .expect("the desktop attaches the global doc");
    let read_global = agent.get_doc(global_id.clone()).await.is_ok();
    let save_global = wire(
        agent
            .save_doc(
                global_id.clone(),
                "E2E_AGENT_EDIT\n".into(),
                global["mtimeMs"].as_f64().unwrap_or_default(),
            )
            .await
            .err(),
    );
    let attach_global = wire(
        agent
            .attach_workstream_doc(ws.clone(), global_id.clone(), "context".into())
            .await
            .err(),
    );
    let untouched =
        std::fs::read_to_string(&global_id).is_ok_and(|d| !d.contains("E2E_AGENT_EDIT"));
    let create_global = wire(
        agent
            .create_doc(note(Some(&project), "global", "Breadth agent global"))
            .await
            .err(),
    );
    let own = agent
        .create_doc(note(Some(&project), "shared", "Breadth agent note"))
        .await;
    let save_own = match &own {
        Ok(meta) => wire(
            agent
                .save_doc(meta.id.clone(), "E2E_OWN_EDIT\n".into(), meta.mtime_ms)
                .await
                .err(),
        ),
        Err(e) => format!("{}: {}", e.code, e.message),
    };
    let write_refused = |r: &str| {
        r.starts_with("agent_control_denied: ") && r.contains("mapped to its own project")
    };
    j.guard(
        "a global doc the desktop attached reads, yet the agent's save of it is refused and the file is unchanged",
        read_global && write_refused(&save_global) && untouched,
        json!({ "read": read_global, "save": save_global, "fileUnchanged": untouched }),
    );
    j.guard(
        "the agent's own attach of a global doc is refused",
        write_refused(&attach_global),
        json!({ "attach": attach_global }),
    );
    j.guard(
        "create_doc with global scope is refused though it names the caller's project; a shared doc there is created and saved",
        create_global.starts_with("agent_control_denied: ")
            && create_global.contains("global document is the user's")
            && own.is_ok()
            && save_own == "ok",
        json!({ "createGlobal": create_global, "createShared": own.is_ok(), "saveShared": save_own }),
    );

    j.step("a Default caller cannot widen what it may run");
    let plan = write_plan(&mut j, &c, &project).await;
    let approve_up = refusal(&mcp.call("approve_plan", json!({ "plan": plan })).await);
    let validate_up = refusal(&mcp.call("validate_plan", json!({ "plan": plan })).await);
    let goal_up = refusal(
        &mcp.call(
            "start_goal",
            json!({
                "condition": "E2E_SHELL_GOAL",
                "work_prompt": "Raise the counter.",
                "checks": [{ "kind": "command", "command": "touch e2e-shell-ran", "timeoutSecs": 30 }],
            }),
        )
        .await,
    );
    let goals = c
        .list_goals(Some(ws.clone()), false)
        .await
        .expect("list_goals")
        .len();
    let still_draft = plan_status(&plan) == "draft";
    j.guard(
        "approve_plan and validate_plan are refused: implement and validate pre-approve tools a General caller lacks, and the plan stays a draft",
        approve_up.starts_with("agent_control_denied: ")
            && approve_up.contains("cannot run the implement stage")
            && validate_up.starts_with("agent_control_denied: ")
            && validate_up.contains("cannot run the validate stage")
            && still_draft,
        json!({ "approve": approve_up, "validate": validate_up, "planStillDraft": still_draft }),
    );
    j.guard(
        "a goal with a command check is refused outside bypass, and no goal is written",
        goal_up.starts_with("agent_control_denied: ")
            && goal_up.contains("command check")
            && goals == 0,
        json!({ "startGoal": goal_up, "goals": goals }),
    );

    j.step("approve_plan activates and builds the plan");
    set_mode(&mut j, &c, &lead, PermissionMode::BypassPermissions).await;
    let approved = parsed(&mcp.call("approve_plan", json!({ "plan": plan })).await);
    let implement = text(&approved, "/sessionId");
    j.alias(&implement, "$IMPLEMENT");
    let implement_tab = session_tab(&mut rx, &implement).await;
    let status = plan_status(&plan);
    let row = session_row(&mut j, &c, &ws, &implement).await;
    let attached = first_turn(&c, &implement).await;
    j.guard(
        "the plan is active, and an implement session under the caller has it attached and runs in the caller's mode",
        status == "active"
            && approved["stage"] == json!("implement")
            && row["origin"] == json!("agent")
            && row["parentSessionId"] == json!(lead)
            && row["permissionMode"] == json!("bypassPermissions")
            && attached.contains(&plan),
        json!({
            "planStatus": status,
            "stage": approved["stage"],
            "origin": row["origin"],
            "parentSessionId": row["parentSessionId"],
            "permissionMode": row["permissionMode"],
            "planAttached": attached.contains(&plan),
        }),
    );
    j.guard(
        "desktops open it in the background in the caller's mode, now bypass",
        implement_tab.as_ref().is_some_and(|t| {
            t["focus"] == json!(false) && t["permissionMode"] == json!("bypassPermissions")
        }),
        json!({ "tab": implement_tab }),
    );

    j.step("validate_plan opens a validate session");
    let validated = parsed(&mcp.call("validate_plan", json!({ "plan": plan })).await);
    let validate = text(&validated, "/sessionId");
    j.alias(&validate, "$VALIDATE");
    let validate_tab = session_tab(&mut rx, &validate).await;
    let opened = first_turn(&c, &validate).await;
    j.guard(
        "a validate session on the plan, in the caller's own mode",
        validated["stage"] == json!("validate")
            && opened.contains(&plan)
            && validate_tab
                .as_ref()
                .is_some_and(|t| t["permissionMode"] == json!("bypassPermissions")),
        json!({
            "stage": validated["stage"],
            "planAttached": opened.contains(&plan),
            "tabMode": validate_tab.as_ref().map(|t| t["permissionMode"].clone()),
        }),
    );
    for id in [&implement, &validate] {
        wait_session(&mut mcp, id).await;
        mcp.call("kill_agent", json!({ "session_id": id })).await;
    }

    j.step("start_goal on itself");
    let goal = parsed(
        &mcp.call(
            "start_goal",
            json!({
                "condition": "E2E_SELF_GOAL",
                "work_prompt": "Raise the counter.",
                "checks": [{ "kind": "command", "command": "test -f counter.txt", "timeoutSecs": 30 }],
            }),
        )
        .await,
    );
    let goal_id = text(&goal, "/goalId");
    j.alias(&goal_id, "$GOAL");
    let ended = wait_goal(&c, &goal_id).await;
    j.guard(
        "in bypass, a command-checked goal with no target runs on the caller's own session and meets",
        goal["sessionId"] == json!(lead) && ended == "met",
        json!({ "sessionId": goal["sessionId"], "status": ended }),
    );
    set_mode(&mut j, &c, &lead, PermissionMode::Default).await;
    let swap = wire(
        agent
            .update_goal(
                goal_id.clone(),
                GoalPatch {
                    condition: None,
                    work_prompt: None,
                    checks: Some(vec![GoalCheck::Command {
                        command: "touch e2e-shell-ran".into(),
                        timeout_secs: 30,
                        label: None,
                        cwd: None,
                        phases: vec![],
                    }]),
                    limits: None,
                },
            )
            .await
            .err(),
    );
    let kept = c
        .get_goal(goal_id)
        .await
        .expect("get_goal")
        .goal
        .checks
        .iter()
        .all(|check| !matches!(check, GoalCheck::Command { command, .. } if command.contains("e2e-shell-ran")));
    j.guard(
        "back in Default, update_goal swapping in a command check is refused and the goal keeps its checks",
        swap.starts_with("agent_control_denied: ") && swap.contains("command check") && kept,
        json!({ "updateGoal": swap, "checksKept": kept }),
    );

    j.step("script_start runs without a stream");
    let started = mcp.call("run_script", json!({ "kind": "run" })).await;
    let mut output = String::new();
    let deadline = tokio::time::Instant::now() + IO;
    while !output.contains("E2E_RUN_OK") && tokio::time::Instant::now() < deadline {
        output = c
            .get_script_output(ws.clone(), ScriptKind::Run)
            .await
            .expect("get_script_output")
            .unwrap_or_default();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let read_back = tool_text(&mcp.call("script_output", json!({ "kind": "run" })).await);
    j.guard(
        "run_script starts the run script with nobody attached, and its output is readable after",
        !tool_failed(&started) && output.contains("E2E_RUN_OK") && read_back.contains("E2E_RUN_OK"),
        json!({
            "started": tool_text(&started),
            "desktopReads": output.contains("E2E_RUN_OK"),
            "agentReads": read_back.contains("E2E_RUN_OK"),
        }),
    );
    let archive_run = refusal(&mcp.call("run_script", json!({ "kind": "archive" })).await);
    let raw_archive = wire(
        agent
            .script_start(ws.clone(), ScriptKind::Archive)
            .await
            .err(),
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let archive_ran = sandbox.home.join("e2e-archive-ran").exists();
    j.guard(
        "the archive script is refused through run_script and raw script_start, and never runs",
        archive_run.starts_with("agent_control_denied: ")
            && archive_run.contains("the archive script is the user's")
            && raw_archive == archive_run
            && !archive_ran,
        json!({ "runScript": archive_run, "scriptStart": raw_archive, "archiveRan": archive_ran }),
    );

    j.step("review comments");
    let commented = parsed(
        &mcp.call(
            "comment",
            json!({ "path": "README.md", "line": 1, "body": "E2E agent note", "anchor_text": "# breadth" }),
        )
        .await,
    );
    let comment_id = text(&commented, "/commentId");
    j.alias(&comment_id, "$COMMENT");
    let open = comment(&mut j, &c, &ws, &comment_id).await;
    let resolved = mcp
        .call("resolve_comment", json!({ "comment_id": comment_id }))
        .await;
    let closed = comment(&mut j, &c, &ws, &comment_id).await;
    j.guard(
        "the agent's comment reads back on its line, and resolving it sticks",
        open["body"] == json!("E2E agent note")
            && open["path"] == json!("README.md")
            && open["line"] == json!(1)
            && open["resolved"] == json!(false)
            && !tool_failed(&resolved)
            && closed["resolved"] == json!(true),
        json!({
            "body": open["body"],
            "path": open["path"],
            "line": open["line"],
            "side": open["side"],
            "resolvedBefore": open["resolved"],
            "resolvedAfter": closed["resolved"],
        }),
    );

    j.step("reveal reaches desktops");
    drain(&mut rx);
    let asks = [
        ("doc", plan),
        ("file", "README.md".to_string()),
        ("diff", "README.md".to_string()),
    ];
    let mut answers = Vec::new();
    for (kind, path) in &asks {
        answers.push(tool_text(
            &mcp.call("reveal", json!({ "kind": kind, "path": path }))
                .await,
        ));
    }
    let reveals: Vec<Value> = events_until(&mut rx, Duration::from_secs(3), |_| false)
        .await
        .into_iter()
        .filter_map(|e| match e {
            RuntimeEvent::UiRequested {
                request: request @ UiRequest::Reveal { .. },
            } => serde_json::to_value(request).ok(),
            _ => None,
        })
        .collect();
    j.guard(
        "a doc, a file and a diff each reach the desktop as a Reveal in the caller's workstream",
        answers.iter().all(|a| a == "\"revealed\"")
            && reveals.len() == 3
            && reveals.iter().all(|r| r["workstreamId"] == json!(ws))
            && reveals
                .iter()
                .map(|r| text(r, "/target/kind"))
                .collect::<Vec<_>>()
                == ["doc", "file", "diff"],
        json!({ "reveals": reveals }),
    );

    j.step("a Lead's own live-children limit sits above every agent's");
    let project_lead = j
        .call(
            &c,
            Request::MessageLead {
                project_path: project.clone(),
                text: "E2E_PROJECT_LEAD".into(),
            },
        )
        .await
        .expect("message_lead");
    let project_lead = project_lead.as_str().unwrap_or_default().to_string();
    j.alias(&project_lead, "$PROJECT_LEAD");
    first_turn_done(&c, &project_lead).await;
    let mut lead_mcp = McpDriver::start(&env_of(&sandbox, &project_lead).await, false);
    lead_mcp.initialize("2025-06-18").await;
    let mut lead_children = Vec::new();
    for n in 1..=5 {
        let spawned = lead_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": format!("E2E_LEAD_CHILD_{n}"), "title": format!("Lead child {n}"), "harness": "claude" }),
            )
            .await;
        let id = text(&parsed(&spawned), "/sessionId");
        j.alias(&id, format!("$LEAD_CHILD_{n}"));
        lead_children.push((refusal(&spawned), id));
    }
    let leads = j.call(&c, Request::ListLeads {}).await.expect("list_leads");
    let listed = leads
        .as_array()
        .and_then(|all| all.iter().find(|l| text(l, "/projectPath") == project))
        .cloned()
        .unwrap_or(Value::Null);
    j.alias(text(&listed, "/workstreamId"), "$MAIN");
    let mut child_mcp = McpDriver::start(&env_of(&sandbox, &lead_children[0].1).await, false);
    child_mcp.initialize("2025-06-18").await;
    let mut grandchildren = Vec::new();
    let mut grandchild_ids = Vec::new();
    for n in 1..=5 {
        let spawned = child_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": format!("E2E_GRANDCHILD_{n}"), "title": format!("Grandchild {n}"), "harness": "claude" }),
            )
            .await;
        let id = text(&parsed(&spawned), "/sessionId");
        j.alias(&id, format!("$GRANDCHILD_{n}"));
        grandchild_ids.push(id);
        grandchildren.push(refusal(&spawned));
    }
    j.guard(
        "with Max live children at 4, the Lead holds 5 live children and lists them",
        lead_children.iter().all(|(r, _)| r == "ok")
            && text(&listed, "/sessionId") == project_lead
            && listed["liveChildren"] == json!(5),
        json!({
            "spawns": lead_children.iter().map(|(r, _)| r).collect::<Vec<_>>(),
            "listed": listed,
        }),
    );
    j.guard(
        "a caller that is not a Lead is still refused its fifth child",
        grandchildren[..4].iter().all(|r| r == "ok")
            && grandchildren[4].starts_with("agent_limit_reached: ")
            && grandchildren[4].contains("already has 4 live children, the Max live children"),
        json!({ "spawns": grandchildren }),
    );

    j.step("a Lead's messages to its own child are not agent hops; a non-Lead's still are");
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 2).await;
    let mut from_lead = Vec::new();
    for n in 1..=3 {
        let sent = lead_mcp
            .call(
                "message_agent",
                json!({ "session_id": lead_children[0].1, "text": format!("E2E_LEAD_HOP_{n}") }),
            )
            .await;
        from_lead.push(refusal(&sent));
    }
    let mut from_child = Vec::new();
    for n in 1..=3 {
        let sent = child_mcp
            .call(
                "message_agent",
                json!({ "session_id": grandchild_ids[0], "text": format!("E2E_CHILD_HOP_{n}") }),
            )
            .await;
        from_child.push(refusal(&sent));
    }
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 6).await;
    j.guard(
        "with Max agent hops at 2, all three of the Lead's messages reach its child, and the child's third to its own child is agent_limit_reached",
        from_lead.iter().all(|r| r == "ok")
            && from_child[..2].iter().all(|r| r == "ok")
            && from_child[2].starts_with("agent_limit_reached: "),
        json!({ "fromLead": from_lead, "fromChild": from_child }),
    );
    drop(child_mcp);

    j.step("a Lead's terminal child runs in the Lead's mode and counts as driving");
    let lead_who = parsed(&lead_mcp.call("whoami", json!({})).await);
    let spawned = parsed(
        &lead_mcp
            .call(
                "spawn_agent",
                json!({ "terminal_engine": "claude-code", "workstream_id": ws, "title": "Lead terminal" }),
            )
            .await,
    );
    let lead_pty = text(&spawned, "/terminalId");
    j.alias(&lead_pty, "$LEAD_PTY");
    let pty_env = env_of(&sandbox, &lead_pty).await;
    j.alias(
        env_var(&pty_env, "HYPRLAYER_AGENT_TOKEN"),
        "$LEAD_PTY_TOKEN",
    );
    let argv = lines(&launch_log(&sandbox, &lead_pty).join("argv"));
    let mode_flag: Vec<&String> = argv
        .windows(2)
        .find(|w| w[0] == "--permission-mode")
        .map(|w| w.iter().collect())
        .unwrap_or_default();
    let mut pty_mcp = McpDriver::start(&pty_env, false);
    pty_mcp.initialize("2025-06-18").await;
    let pty_who = parsed(&pty_mcp.call("whoami", json!({})).await);
    let to_lead = refusal(
        &pty_mcp
            .call(
                "message_agent",
                json!({ "session_id": project_lead, "text": "E2E_FROM_LEAD_PTY" }),
            )
            .await,
    );
    drop(pty_mcp);
    let deadline = tokio::time::Instant::now() + IO;
    let mut delivered = false;
    while !delivered && tokio::time::Instant::now() < deadline {
        delivered = c
            .get_session_log(project_lead.clone())
            .await
            .expect("get_session_log")
            .iter()
            .any(|e| matches!(&e.event, SessionEvent::UserMessage { text, .. } if text.contains("E2E_FROM_LEAD_PTY")));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let leads = j.call(&c, Request::ListLeads {}).await.expect("list_leads");
    let listed = leads
        .as_array()
        .and_then(|all| all.iter().find(|l| text(l, "/projectPath") == project))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the Lead runs in auto, and a terminal child spawned with no mode launches with --permission-mode auto",
        lead_who["permissionMode"] == json!("auto")
            && mode_flag == ["--permission-mode", "auto"]
            && pty_who["permissionMode"] == json!("auto"),
        json!({ "lead": lead_who["permissionMode"], "argv": mode_flag, "child": pty_who["permissionMode"] }),
    );
    j.guard(
        "so its message to the Lead is delivered, not refused for rank",
        to_lead == "ok" && delivered,
        json!({ "message": to_lead, "delivered": delivered }),
    );
    j.guard(
        "list_leads counts the terminal child's workstream as driven",
        listed["driving"] == json!([ws]) && listed["liveChildren"] == json!(6),
        json!({ "driving": listed["driving"], "liveChildren": listed["liveChildren"] }),
    );
    drop(lead_mcp);

    j.step("shutdown");
    drop(mcp);
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

/// A desktop-started Default claude session titled "Lead", through its first turn.
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
    // Set before the titler can, so the header naming this caller is fixed.
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
    first_turn_done(c, &session.id).await;
    session.id
}

async fn first_turn_done(c: &RpcClient, session_id: &str) {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let log = c
            .get_session_log(session_id.to_string())
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
}

async fn env_of(sandbox: &Sandbox, session_id: &str) -> Vec<(String, String)> {
    eventually(IO, || {
        !env_var(&agent_env(sandbox, session_id), "HYPRLAYER_AGENT_TOKEN").is_empty()
    })
    .await;
    agent_env(sandbox, session_id)
}

fn parsed(result: &Value) -> Value {
    serde_json::from_str(&tool_text(result)).unwrap_or(Value::Null)
}

fn outcome(result: &Value) -> Value {
    if tool_failed(result) {
        json!({ "error": tool_text(result) })
    } else {
        parsed(result)
    }
}

/// A refused tool's first line, `<code>: <message>`; `ok` when it was not refused.
fn refusal(result: &Value) -> String {
    if !tool_failed(result) {
        return "ok".into();
    }
    tool_text(result)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

fn is_terminal_tab(event: &RuntimeEvent, terminal: &str) -> bool {
    matches!(
        event,
        RuntimeEvent::UiRequested {
            request: UiRequest::OpenTerminalTab { terminal_id, .. }
        } if terminal_id == terminal
    )
}

async fn session_tab(rx: &mut Receiver<RuntimeEvent>, session_id: &str) -> Option<Value> {
    let is_tab = |e: &RuntimeEvent| {
        matches!(
            e,
            RuntimeEvent::UiRequested {
                request: UiRequest::OpenSessionTab { session_id: s, .. }
            } if s == session_id
        )
    };
    events_until(rx, IO, is_tab)
        .await
        .into_iter()
        .find(|e| is_tab(e))
        .and_then(|e| match e {
            RuntimeEvent::UiRequested { request } => serde_json::to_value(request).ok(),
            _ => None,
        })
}

fn drain(rx: &mut Receiver<RuntimeEvent>) {
    while rx.try_recv().is_ok() {}
}

async fn pty_agent(j: &mut Journey, c: &RpcClient, ws: &str, terminal: &str) -> Value {
    j.call(
        c,
        Request::ListPtyAgents {
            workstream_id: Some(ws.to_string()),
        },
    )
    .await
    .expect("list_pty_agents")
    .as_array()
    .and_then(|a| a.iter().find(|a| a["terminalId"] == json!(terminal)))
    .cloned()
    .unwrap_or(Value::Null)
}

async fn pty_state(c: &RpcClient, ws: &str, terminal: &str) -> Option<String> {
    c.list_pty_agents(Some(ws.to_string()))
        .await
        .expect("list_pty_agents")
        .into_iter()
        .find(|a| a.terminal_id == terminal)
        .map(|a| a.state.as_str().to_string())
}

async fn gone_from_list(c: &RpcClient, ws: &str, terminal: &str) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let listed = c
            .list_pty_agents(Some(ws.to_string()))
            .await
            .expect("list_pty_agents");
        if !listed.iter().any(|a| a.terminal_id == terminal) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// A note's create request: `project` none or the caller's, `scope` global, shared or a user's.
fn note(project: Option<&str>, scope: &str, title: &str) -> CreateDocumentRequest {
    CreateDocumentRequest {
        project_repo_path: project.map(str::to_string),
        scope: scope.into(),
        doc_type: "note".into(),
        title: title.into(),
        date: "2026-01-01".into(),
        ticket: None,
        tags: vec![],
    }
}

/// A refused RPC as `<code>: <message>`, the way `refusal` reads a tool; `ok` when it passed.
fn wire(error: Option<WireError>) -> String {
    error.map_or_else(|| "ok".into(), |e| format!("{}: {}", e.code, e.message))
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

/// The desktop's mode toggle on a live session; agents read the new mode on their next call.
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

/// A plan doc indexed in the project's thoughts, with a draft status; returns its path.
async fn write_plan(j: &mut Journey, c: &RpcClient, project: &str) -> String {
    let meta = j
        .call(
            c,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.to_string()),
                    scope: "shared".into(),
                    doc_type: "plan".into(),
                    title: "Breadth plan".into(),
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

async fn session_row(j: &mut Journey, c: &RpcClient, ws: &str, id: &str) -> Value {
    j.call(
        c,
        Request::ListSessions {
            workstream_id: ws.to_string(),
        },
    )
    .await
    .expect("list_sessions")
    .as_array()
    .and_then(|rows| rows.iter().find(|r| r["id"] == json!(id)).cloned())
    .unwrap_or(Value::Null)
}

/// The session's first user turn as logged.
async fn first_turn(c: &RpcClient, session_id: &str) -> String {
    c.get_session_log(session_id.to_string())
        .await
        .expect("get_session_log")
        .into_iter()
        .find_map(|e| match e.event {
            SessionEvent::UserMessage { text, .. } => Some(text),
            _ => None,
        })
        .unwrap_or_default()
}

async fn wait_session(mcp: &mut McpDriver, session_id: &str) {
    mcp.call(
        "wait_agent",
        json!({ "session_id": session_id, "timeout_sec": WAIT_SECS }),
    )
    .await;
}

async fn wait_goal(c: &RpcClient, goal_id: &str) -> String {
    let deadline = tokio::time::Instant::now() + GOAL_WAIT;
    loop {
        let detail = c.get_goal(goal_id.to_string()).await.expect("get_goal");
        let status = detail.goal.status.as_str().to_string();
        if status != "running" || tokio::time::Instant::now() >= deadline {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn comment(j: &mut Journey, c: &RpcClient, ws: &str, id: &str) -> Value {
    j.call(
        c,
        Request::GetReview {
            workstream_id: ws.to_string(),
        },
    )
    .await
    .expect("get_review")["comments"]
        .as_array()
        .and_then(|all| all.iter().find(|x| text(x, "/id") == id).cloned())
        .unwrap_or(Value::Null)
}

/// Waits for the PTY agent's generated title, which lands a moment after its first prompt.
async fn titled(c: &RpcClient, ws: &str, terminal: &str) {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let listed = c
            .list_pty_agents(Some(ws.to_string()))
            .await
            .expect("list_pty_agents");
        if listed
            .iter()
            .any(|a| a.terminal_id == terminal && a.title.is_some())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
