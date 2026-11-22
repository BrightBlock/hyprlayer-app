//! PTY claude and codex launched by the server: the hooks, MCP and plugin each launch injects,
//! the state their hook signals fold into, the post-deny settle, token revocation, the codex
//! version gate and its server-side mint, `off`, a user's own `--settings`, and the `hook`
//! face's never-block contract.
//!
//! The harnesses are `agent_stub`'s PTY claude and codex, which run exactly the hook commands
//! their argv and settings name; nothing here covers the real binaries.
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use hyprlayer_core::models::{
    AgentControlLevel, PermissionMode, PtyAgentState, PtyLaunchSession, UiRequest, WorktreeMode,
};
use hyprlayer_core::runtime::agent_control::inject::{CLAUDE_HOOK_EVENTS, CODEX_HOOK_EVENTS};
use hyprlayer_core::runtime::agent_control::AGENT_ENV;
use hyprlayer_core::{Harness, Stage};
use hyprlayer_e2e::agent_stub::{
    agent_env, appears, env_var, install, launch_log, launch_pty, lines, run_face, set_engine,
    set_level, stub_dir, tool_failed, tool_text, type_line, McpDriver, PTY_CLAUDE, PTY_CODEX,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, eventually, json, keystrokes, text, wait_for_exit, Journey, Request,
    RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;
use tokio::sync::mpsc::Receiver;

const IO: Duration = Duration::from_secs(30);
/// The settle needs 2 s of quiet; the plan allows it 4.
const SETTLE_BOUND: Duration = Duration::from_secs(4);
const READ_TOOL_RULES: usize = 12;

#[derive(Debug, Clone)]
struct Change {
    state: PtyAgentState,
    provider: Option<String>,
    activity: Option<String>,
    last_reply: Option<String>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_hook_signals() {
    let sandbox = Sandbox::new();
    let codex = sandbox.stage_tool("codex", PTY_CODEX);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let codex_old = install(
        &sandbox,
        "codex-old",
        &format!(
            "#!/bin/sh\nPTY_CODEX_VERSION=0.163.0 exec '{}' \"$@\"\n",
            codex.display()
        ),
    );
    let user_settings = sandbox.root.join("user-settings.json");
    std::fs::write(&user_settings, "{}\n").expect("write the user's settings");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("pty_hook_signals", &sandbox);
    let ctl = std::fs::canonicalize(hyprlayer_e2e::server_binary()).expect("the server binary");
    j.alias(ctl.display().to_string(), "$CTL");
    j.alias(hyprlayer_e2e::server_binary().display().to_string(), "$CTL");
    let launch_root = sandbox.app_data.join("agent-control").join("launch");

    let server = Server::start(&sandbox).await;
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
    let view = create_workstream(&mut j, &c, &project, "Hooks").await;
    let ws = text(&view, "/item/id");
    let worktree = text(&view, "/item/worktreePath");
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    set_engine(&mut j, &c, "codex", &codex, &[]).await;

    j.step("the launch injects hooks, MCP and the plugin");
    let (launched, mut claude_events) = launch_pty(&c, &ws, "claude-code").await;
    let term = launched.terminal_id.clone();
    let sid = launched.provider_session_id.clone().unwrap_or_default();
    j.alias(&term, "$CLAUDE_TERMINAL");
    j.alias(&sid, "$CLAUDE_SESSION");
    let log = launch_log(&sandbox, &term);
    appears(&log.join("hooks.log")).await;
    let argv = lines(&log.join("argv"));
    let env = agent_env(&sandbox, &term);
    let token = env_var(&env, "HYPRLAYER_AGENT_TOKEN").to_string();
    j.alias(&token, "$CLAUDE_TOKEN");
    let dir = launch_root.join(&term);
    let launch_dir_live = dir.is_dir();
    let plugin_dir = argv.get(5).map(|p| Path::new(p).is_dir()).unwrap_or(false);
    // Versioned, so it would pin the golden to one server release.
    if let Some(plugin) = argv.get(5) {
        j.alias(plugin, "$PLUGIN");
    }
    let shape = argv.len() == 8
        && argv[0] == "--mcp-config"
        && argv[1] == dir.join("mcp.json").display().to_string()
        && argv[2] == "--settings"
        && argv[3] == dir.join("settings.json").display().to_string()
        && argv[4] == "--plugin-dir"
        && plugin_dir
        && argv[6] == "--session-id"
        && argv[7] == sid;
    j.guard(
        "argv is --mcp-config, --settings, --plugin-dir from the launch dir, then the session id the launch returned",
        launched.hooked && shape && launch_dir_live && !sid.is_empty(),
        json!({ "hooked": launched.hooked, "argv": argv, "launchDirLive": launch_dir_live }),
    );
    let settings = sandbox.read_json(&log.join("settings.json"));
    let quoted = format!("'{}'", ctl.display());
    let hooks: Vec<(String, bool)> = settings["hooks"]
        .as_object()
        .map(|hooks| {
            hooks
                .iter()
                .map(|(event, hook)| {
                    let command = text(hook, "/0/hooks/0/command");
                    (
                        event.clone(),
                        command == format!("{quoted} hook claude {event}"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut events: Vec<&str> = hooks.iter().map(|(e, _)| e.as_str()).collect();
    events.sort();
    let mut expected = CLAUDE_HOOK_EVENTS.to_vec();
    expected.sort();
    let status_line = text(&settings, "/statusLine/command");
    j.guard(
        "the settings file runs `<ctl> hook claude <Event>` for all 12 events, and `<ctl> statusline claude`",
        events == expected
            && hooks.iter().all(|(_, ok)| *ok)
            && status_line == format!("{quoted} statusline claude"),
        json!({
            "events": events,
            "commandsRunCtl": hooks.iter().all(|(_, ok)| *ok),
            "statusLine": status_line,
        }),
    );
    let allow: Vec<String> = settings["permissions"]["allow"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    j.guard(
        "the same file allows the 12 hyprlayer read tools, read_terminal included",
        allow.len() == READ_TOOL_RULES
            && allow.iter().all(|r| r.starts_with("mcp__hyprlayer__"))
            && allow.iter().any(|r| r == "mcp__hyprlayer__read_terminal"),
        json!({ "allow": allow }),
    );
    let mcp = sandbox.read_json(&log.join("mcp.json"));
    let server_entry = &mcp["mcpServers"]["hyprlayer"];
    let mut mcp_env: Vec<String> = server_entry["env"]
        .as_object()
        .map(|e| e.keys().cloned().collect())
        .unwrap_or_default();
    mcp_env.sort();
    let mcp_ok = server_entry["command"] == json!(ctl.display().to_string())
        && server_entry["args"] == json!(["mcp"])
        && server_entry["env"]["HYPRLAYER_AGENT_TOKEN"] == json!(token)
        && server_entry["env"]["HYPRLAYER_TERMINAL_ID"] == json!(term);
    j.guard(
        "the MCP config runs `<ctl> mcp` with this terminal's identity",
        mcp_ok,
        json!({ "command": server_entry["command"], "args": server_entry["args"], "env": mcp_env }),
    );
    let ctl_env = std::fs::canonicalize(env_var(&env, "HYPRLAYER_CTL")).ok();
    let env_ok = env_var(&env, "HYPRLAYER_TERMINAL_ID") == term
        && env_var(&env, "HYPRLAYER_WORKSTREAM_ID") == ws
        && env_var(&env, "HYPRLAYER_SOCKET") == sandbox.socket().display().to_string()
        && ctl_env.as_deref() == Some(ctl.as_path())
        && !token.is_empty();
    j.guard(
        "env names this terminal, its workstream, the socket and the server binary",
        env_ok,
        json!({ "identityKeys": sorted(
            &env.iter()
                .map(|(k, _)| k.as_str())
                .filter(|k| AGENT_ENV.contains(k))
                .collect::<Vec<_>>(),
        ) }),
    );

    j.step("SessionStart records the provider session");
    let seen = changes_until(&mut rx, &term, |c| c.state == PtyAgentState::Idle).await;
    let last = seen.last().cloned();
    j.guard(
        "the launch reads starting, then SessionStart idles it with the provider session",
        states(&seen) == ["starting", "idle"]
            && last.as_ref().and_then(|c| c.provider.clone()) == Some(sid.clone()),
        json!({ "states": states(&seen), "provider": last.and_then(|c| c.provider) }),
    );
    let listed = pty_agent(&mut j, &c, &ws, &term).await;
    j.guard(
        "list_pty_agents shows it hooked and idle on that session",
        listed["hooked"] == json!(true)
            && listed["state"] == json!("idle")
            && listed["providerSessionId"] == json!(sid),
        json!({ "listed": listed }),
    );

    j.step("a prompt works, then Stop settles with the reply");
    type_line(&c, &term, "work").await;
    let seen = changes_until(&mut rx, &term, |c| c.state == PtyAgentState::Idle).await;
    let last = seen.last().cloned();
    j.guard(
        "working, then idle with Stop's reply and the last tool as activity",
        states(&seen) == ["working", "idle"]
            && last.as_ref().and_then(|c| c.last_reply.clone()).as_deref() == Some("done: work")
            && last.as_ref().and_then(|c| c.activity.clone()).as_deref() == Some("Bash ls -la"),
        json!({
            "states": states(&seen),
            "lastReply": last.as_ref().and_then(|c| c.last_reply.clone()),
            "activity": last.and_then(|c| c.activity),
        }),
    );

    j.step("a permission ask blocks");
    type_line(&c, &term, "ask").await;
    let seen = changes_until(&mut rx, &term, |c| {
        c.state == PtyAgentState::AwaitingPermission
    })
    .await;
    j.guard(
        "PermissionRequest reads awaiting_permission",
        states(&seen) == ["awaiting_permission"],
        json!({ "states": states(&seen) }),
    );

    j.step("another tool's hooks leave the ask open");
    // A parallel or subagent tool: its PreToolUse and PostToolUse don't answer the prompt on screen.
    type_line(&c, &term, "tools").await;
    let parallel = changes_within(&mut rx, &term, SETTLE_BOUND).await;
    let listed = pty_agent(&mut j, &c, &ws, &term).await;
    j.guard(
        "a Read's PreToolUse and PostToolUse during the Bash ask change nothing, and no settle idles it",
        parallel.is_empty() && listed["state"] == json!("awaiting_permission"),
        json!({ "states": states(&parallel), "listed": listed["state"] }),
    );

    j.step("a deny settles after quiet output");
    let stops_before = count(&log.join("hooks.log"), "Stop ");
    // xterm's own focus report: not a person at the keyboard, so it must not arm the settle.
    c.terminal_write(term.clone(), keystrokes("\x1b[I"))
        .await
        .expect("terminal_write");
    let unarmed = changes_within(&mut rx, &term, Duration::from_secs(3)).await;
    j.guard(
        "a terminal reply alone leaves it asking",
        unarmed.is_empty(),
        json!({ "states": states(&unarmed) }),
    );
    // A person moving the prompt's highlight: the quiet after it must not read as a deny.
    c.terminal_write(term.clone(), keystrokes("\x1b[B"))
        .await
        .expect("terminal_write");
    let moved = changes_within(&mut rx, &term, SETTLE_BOUND).await;
    j.guard(
        "an arrow key with the prompt still on screen leaves it asking past the quiet window",
        moved.is_empty(),
        json!({ "states": states(&moved) }),
    );
    let denied_at = Instant::now();
    type_line(&c, &term, "n").await;
    let seen = changes_until(&mut rx, &term, |c| c.state == PtyAgentState::Idle).await;
    let elapsed = denied_at.elapsed();
    let stops_after = count(&log.join("hooks.log"), "Stop ");
    j.guard(
        "the typed deny settles to idle after the quiet window, within 4 s, with no Stop",
        states(&seen) == ["idle"]
            && elapsed >= Duration::from_millis(1900)
            && elapsed < SETTLE_BOUND
            && stops_after == stops_before,
        json!({
            "states": states(&seen),
            "afterQuiet": elapsed >= Duration::from_millis(1900),
            "within4s": elapsed < SETTLE_BOUND,
            "newStops": stops_after - stops_before,
        }),
    );

    j.step("a question blocks");
    type_line(&c, &term, "question").await;
    let seen = changes_until(&mut rx, &term, |c| {
        c.state == PtyAgentState::AwaitingQuestion
    })
    .await;
    j.guard(
        "AskUserQuestion reads awaiting_question",
        states(&seen) == ["awaiting_question"],
        json!({ "states": states(&seen) }),
    );

    j.step("/clear tracks the new session id");
    let cleared = format!("{sid}-cleared");
    type_line(&c, &term, "clear").await;
    let seen = changes_until(&mut rx, &term, |c| {
        c.state == PtyAgentState::Idle && c.provider.as_deref() == Some(cleared.as_str())
    })
    .await;
    let listed = pty_agent(&mut j, &c, &ws, &term).await;
    j.guard(
        "SessionEnd exits it, and SessionStart on the same token revives it on the new session",
        states(&seen) == ["exited", "idle"]
            && listed["providerSessionId"] == json!(cleared)
            && listed["state"] == json!("idle"),
        json!({ "states": states(&seen), "listed": listed }),
    );

    j.step("malformed payloads are ignored");
    type_line(&c, &term, "bad").await;
    type_line(&c, &term, "ask").await;
    let seen = changes_until(&mut rx, &term, |c| {
        c.state == PtyAgentState::AwaitingPermission
    })
    .await;
    // The stub logs a hook after it exits, which can trail the event it caused.
    eventually(IO, || {
        lines(&log.join("hooks.log"))
            .last()
            .is_some_and(|l| l.starts_with("PermissionRequest"))
    })
    .await;
    let hook_lines = lines(&log.join("hooks.log"));
    let bad: Vec<&String> = hook_lines.iter().rev().skip(1).take(3).collect();
    j.guard(
        "three bad Stop payloads exit 0 and change nothing before the next ask",
        states(&seen) == ["awaiting_permission"]
            && bad.len() == 3
            && bad.iter().all(|l| *l == "Stop exit=0"),
        json!({ "states": states(&seen), "badHooks": bad }),
    );

    j.step("the asked tool's own result ends the ask");
    type_line(&c, &term, "approve").await;
    let seen = changes_until(&mut rx, &term, |c| c.state == PtyAgentState::Idle).await;
    j.guard(
        "PostToolUse for the asked Bash call reads working, then Stop idles it",
        states(&seen) == ["working", "idle"],
        json!({ "states": states(&seen) }),
    );

    j.step("exit revokes the token");
    type_line(&c, &term, "exit").await;
    let seen = changes_until(&mut rx, &term, |c| c.state == PtyAgentState::Exited).await;
    let exited = wait_for_exit(&mut claude_events, IO).await.is_some();
    let gone = gone_from_list(&c, &ws, &term).await;
    let dir_gone = eventually(IO, || !dir.exists()).await;
    j.guard(
        "SessionEnd reads exited, the PTY exits, and the server forgets it and its launch dir",
        states(&seen) == ["exited"] && exited && gone && dir_gone,
        json!({ "states": states(&seen), "ptyExited": exited, "unlisted": gone, "launchDirGone": dir_gone }),
    );
    let late = run_face(
        &["hook", "claude", "Stop"],
        &env,
        br#"{"session_id":"late","last_assistant_message":"after exit"}"#,
    )
    .await;
    let after = changes_within(&mut rx, &term, Duration::from_secs(1)).await;
    j.guard(
        "the old token's hook exits 0 and changes nothing",
        late.code == Some(0) && late.stdout.is_empty() && after.is_empty(),
        json!({ "exit": late.code, "stdout": late.stdout, "changes": after.len() }),
    );
    let refused = announce_refused(&server, &token).await;
    j.guard(
        "announce_agent with the old token is agent_control_denied",
        refused.0 == "agent_control_denied" && refused.1.contains("exited"),
        json!({ "code": refused.0, "message": refused.1 }),
    );

    j.step("codex launches hooked without a mint");
    let (launched, mut codex_events) = launch_pty(&c, &ws, "codex").await;
    let cterm = launched.terminal_id.clone();
    j.alias(&cterm, "$CODEX_TERMINAL");
    let clog = launch_log(&sandbox, &cterm);
    appears(&clog.join("hooks.log")).await;
    let cenv = agent_env(&sandbox, &cterm);
    j.alias(env_var(&cenv, "HYPRLAYER_AGENT_TOKEN"), "$CODEX_TOKEN");
    let cargv = lines(&clog.join("argv"));
    let shape = codex_argv(&cargv, &ctl);
    j.guard(
        "argv is the MCP pairs, a -c hook per event running `<ctl> hook codex <Event>`, then the trust bypass, and nothing else",
        launched.hooked
            && launched.provider_session_id.is_none()
            && shape["mcp"] == json!(true)
            && shape["events"] == json!(sorted(&CODEX_HOOK_EVENTS))
            && shape["commandsRunCtl"] == json!(true)
            && shape["endsWithTrustBypass"] == json!(true)
            && cargv.len() == 27
            && env_var(&cenv, "HYPRLAYER_TERMINAL_ID") == cterm,
        json!({
            "hooked": launched.hooked,
            "launchSessionId": launched.provider_session_id,
            "argc": cargv.len(),
            "shape": shape,
        }),
    );
    let seen = changes_until(&mut rx, &cterm, |c| c.state == PtyAgentState::Idle).await;
    let minted = stub_dir(&sandbox).join("exec.log").exists();
    j.guard(
        "no exec mint runs; the session id arrives with SessionStart",
        !minted
            && seen.last().and_then(|c| c.provider.clone()).as_deref()
                == Some("codex-hooked-thread"),
        json!({ "minted": minted, "states": states(&seen), "provider": seen.last().and_then(|c| c.provider.clone()) }),
    );
    type_line(&c, &cterm, "work").await;
    let work = changes_until(&mut rx, &cterm, |c| c.state == PtyAgentState::Idle).await;
    type_line(&c, &cterm, "ask").await;
    let ask = changes_until(&mut rx, &cterm, |c| {
        c.state == PtyAgentState::AwaitingPermission
    })
    .await;
    type_line(&c, &cterm, "esc").await;
    let esc = changes_until(&mut rx, &cterm, |c| c.state == PtyAgentState::Idle).await;
    type_line(&c, &cterm, "exit").await;
    let exit = changes_until(&mut rx, &cterm, |c| c.state == PtyAgentState::Exited).await;
    wait_for_exit(&mut codex_events, IO).await;
    j.guard(
        "codex's turn, ask, Interrupt and SessionEnd fold like claude's",
        states(&work) == ["working", "idle"]
            && work.last().and_then(|c| c.last_reply.clone()).as_deref() == Some("done: work")
            && states(&ask) == ["awaiting_permission"]
            && states(&esc) == ["idle"]
            && states(&exit) == ["exited"],
        json!({
            "work": states(&work),
            "lastReply": work.last().and_then(|c| c.last_reply.clone()),
            "ask": states(&ask),
            "esc": states(&esc),
            "exit": states(&exit),
        }),
    );
    let acks = lines(&clog.join("acks.log"));
    let hook_calls = lines(&clog.join("hooks.log"));
    j.guard(
        "every hook call printed {} and exited 0",
        acks.len() == 8
            && acks.iter().all(|a| a == "ok")
            && hook_calls.len() == 8
            && hook_calls.iter().all(|l| l.ends_with(" exit=0")),
        json!({ "calls": hook_calls.len(), "acks": acks }),
    );

    j.step("old codex launches unhooked and minted");
    set_engine(&mut j, &c, "codex", &codex_old, &[]).await;
    let (launched, mut old_events) = launch_pty(&c, &ws, "codex").await;
    let oterm = launched.terminal_id.clone();
    j.alias(&oterm, "$OLD_CODEX_TERMINAL");
    let olog = launch_log(&sandbox, &oterm);
    appears(&olog.join("argv")).await;
    let oargv = lines(&olog.join("argv"));
    let oenv = agent_env(&sandbox, &oterm);
    j.alias(env_var(&oenv, "HYPRLAYER_AGENT_TOKEN"), "$OLD_CODEX_TOKEN");
    let exec_log = lines(&stub_dir(&sandbox).join("exec.log"));
    let unlisted = !listed_ids(&c, &ws).await.contains(&oterm);
    j.guard(
        "below 0.164 nothing is injected, one exec mint names the session, and it isn't listed",
        !launched.hooked
            && launched.provider_session_id.as_deref() == Some("minted-thread-e2e")
            && oargv == ["resume", "minted-thread-e2e"]
            && exec_log.len() == 1
            && exec_log[0].starts_with("exec --json")
            && unlisted,
        json!({
            "hooked": launched.hooked,
            "provider": launched.provider_session_id,
            "argv": oargv,
            "mints": exec_log.len(),
            "unlisted": unlisted,
        }),
    );
    type_line(&c, &oterm, "exit").await;
    wait_for_exit(&mut old_events, IO).await;
    j.guard(
        "it fires no hooks",
        !olog.join("hooks.log").exists(),
        json!({ "hooks": lines(&olog.join("hooks.log")).len() }),
    );
    set_engine(&mut j, &c, "codex", &codex, &[]).await;

    j.step("a codex session out of bypass has no mutating access");
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    let (session_id, _stream) =
        start_codex(&mut j, &c, &ws, PermissionMode::BypassPermissions).await;
    let slog = launch_log(&sandbox, &session_id);
    appears(&slog.join("env")).await;
    let benv = agent_env(&sandbox, &session_id);
    let bypass_token = env_var(&benv, "HYPRLAYER_AGENT_TOKEN").to_string();
    j.alias(&bypass_token, "$BYPASS_CODEX_TOKEN");
    let bargs = mcp_args(&lines(&slog.join("argv")));
    let mut mcp = McpDriver::start(&benv, false);
    mcp.initialize("2025-06-18").await;
    let who = whoami(&mut mcp).await;
    let reveal = mcp
        .call(
            "reveal",
            json!({ "kind": "file", "path": format!("{worktree}/README.md") }),
        )
        .await;
    let revealed = events_until(&mut rx, IO, |e| {
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
                request: UiRequest::Reveal { .. }
            }
        )
    });
    j.guard(
        "launched in bypass: a writable MCP face, whoami full, and a mutating reveal reaches the desktop",
        bargs == "['mcp']"
            && who["level"] == json!("full")
            && who["permissionMode"] == json!(PermissionMode::BypassPermissions)
            && !tool_failed(&reveal)
            && revealed,
        json!({
            "mcpArgs": bargs,
            "level": who["level"],
            "mode": who["permissionMode"],
            "revealOk": !tool_failed(&reveal),
            "revealed": revealed,
        }),
    );
    drop(mcp);
    let in_place = j
        .call(
            &c,
            Request::SetSessionPermissionMode {
                session_id: session_id.clone(),
                mode: Some(PermissionMode::Default),
            },
        )
        .await
        .err();
    j.guard(
        "codex can't switch in place, so a mode change is a restart",
        in_place.is_some(),
        json!({ "refused": in_place.is_some() }),
    );
    let (_session, _mode, _sid, _restarted) = c
        .restart_session_stream(
            session_id.clone(),
            None,
            None,
            Some(PermissionMode::Default),
        )
        .await
        .expect("restart_session");
    let relaunched = eventually(IO, || {
        let token = env_var(&agent_env(&sandbox, &session_id), "HYPRLAYER_AGENT_TOKEN").to_string();
        !token.is_empty() && token != bypass_token
    })
    .await;
    let denv = agent_env(&sandbox, &session_id);
    j.alias(
        env_var(&denv, "HYPRLAYER_AGENT_TOKEN"),
        "$DEFAULT_CODEX_TOKEN",
    );
    let dargs = mcp_args(&lines(&slog.join("argv")));
    let old = announce_refused(&server, &bypass_token).await;
    // A client that ignores `--read-only` gets the plain face, so the refusal below is the server's.
    let mut mcp = McpDriver::start(&denv, false);
    mcp.initialize("2025-06-18").await;
    let who = whoami(&mut mcp).await;
    let reveal = mcp
        .call(
            "reveal",
            json!({ "kind": "file", "path": format!("{worktree}/README.md") }),
        )
        .await;
    let spawn = mcp
        .call(
            "spawn_agent",
            json!({ "prompt": "review this", "harness": "codex" }),
        )
        .await;
    let refused = |r: &Value| {
        tool_failed(r)
            && tool_text(r).starts_with("agent_control_denied")
            && tool_text(r).contains("only in bypass mode")
    };
    j.guard(
        "restarted in Default: a read-only MCP face and the bypass token revoked",
        relaunched && dargs == "['mcp','--read-only']" && old.0 == "agent_control_denied",
        json!({ "relaunched": relaunched, "mcpArgs": dargs, "oldToken": old.0 }),
    );
    j.guard(
        "whoami still says full, yet reveal and spawn_agent are refused for codex outside bypass",
        who["level"] == json!("full")
            && who["permissionMode"] == json!(PermissionMode::Default)
            && refused(&reveal)
            && refused(&spawn),
        json!({
            "level": who["level"],
            "mode": who["permissionMode"],
            "reveal": tool_text(&reveal),
            "spawnAgent": tool_text(&spawn),
        }),
    );
    drop(mcp);
    set_level(&mut j, &c, AgentControlLevel::Read).await;

    j.step("an unannounced client hears nothing");
    let heard: Vec<RuntimeEvent> = std::iter::from_fn(|| quiet_rx.try_recv().ok()).collect();
    let targeted = heard
        .iter()
        .filter(|e| {
            matches!(
                e,
                RuntimeEvent::PtyAgentChanged { .. } | RuntimeEvent::UiRequested { .. }
            )
        })
        .count();
    j.guard(
        "no PtyAgentChanged or UiRequested, though it heard broadcasts",
        targeted == 0 && !heard.is_empty(),
        json!({ "targeted": targeted, "heardBroadcasts": !heard.is_empty() }),
    );

    j.step("engine args and bypass follow the injected flags");
    set_engine(&mut j, &c, "claude-code", &pty_claude, &["--verbose"]).await;
    let (launched, _stream_id, mut bypass_events) = c
        .pty_launch_stream(
            ws.clone(),
            "claude-code".into(),
            PtyLaunchSession {
                session_id: None,
                resume: false,
                bypass: true,
                model: None,
                permission_mode: None,
                effort: None,
                first_prompt: None,
            },
            80,
            24,
        )
        .await
        .expect("pty_launch");
    let bterm = launched.terminal_id.clone();
    let bsid = launched.provider_session_id.clone().unwrap_or_default();
    j.alias(&bterm, "$BYPASS_TERMINAL");
    j.alias(&bsid, "$BYPASS_SESSION");
    let blog = launch_log(&sandbox, &bterm);
    appears(&blog.join("hooks.log")).await;
    j.alias(
        env_var(&agent_env(&sandbox, &bterm), "HYPRLAYER_AGENT_TOKEN"),
        "$BYPASS_TOKEN",
    );
    let bargv = lines(&blog.join("argv"));
    let flags: Vec<&str> = bargv
        .iter()
        .map(String::as_str)
        .filter(|a| a.starts_with("--"))
        .collect();
    j.guard(
        "injected flags, then the engine's args, the session id and the bypass flag",
        launched.hooked
            && flags
                == [
                    "--mcp-config",
                    "--settings",
                    "--plugin-dir",
                    "--verbose",
                    "--session-id",
                    "--dangerously-skip-permissions",
                ]
            && bargv.get(8) == Some(&bsid),
        json!({ "hooked": launched.hooked, "flags": flags, "sessionIdAfterFlag": bargv.get(8) == Some(&bsid) }),
    );
    type_line(&c, &bterm, "exit").await;
    wait_for_exit(&mut bypass_events, IO).await;
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    j.step("off disables injection");
    set_level(&mut j, &c, AgentControlLevel::Off).await;
    let (launched, mut off_events) = launch_pty(&c, &ws, "claude-code").await;
    let off_sid = launched.provider_session_id.clone().unwrap_or_default();
    j.alias(&launched.terminal_id, "$OFF_TERMINAL");
    j.alias(&off_sid, "$OFF_SESSION");
    let off_log = launch_log(&sandbox, &format!("session-{off_sid}"));
    appears(&off_log.join("env")).await;
    let off_argv = lines(&off_log.join("argv"));
    let off_env: Vec<String> = agent_env(&sandbox, &format!("session-{off_sid}"))
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| AGENT_ENV.contains(&k.as_str()))
        .collect();
    let off_unlisted = !listed_ids(&c, &ws).await.contains(&launched.terminal_id);
    j.guard(
        "only the session id; no agent identity env, not hooked, not listed",
        !launched.hooked
            && off_argv == ["--session-id", off_sid.as_str()]
            && off_env.is_empty()
            && off_unlisted,
        json!({ "hooked": launched.hooked, "argv": off_argv, "identityEnv": off_env, "unlisted": off_unlisted }),
    );
    type_line(&c, &launched.terminal_id, "exit").await;
    wait_for_exit(&mut off_events, IO).await;
    set_level(&mut j, &c, AgentControlLevel::Read).await;

    j.step("a user --settings disables hooks");
    let user = user_settings.display().to_string();
    set_engine(
        &mut j,
        &c,
        "claude-code",
        &pty_claude,
        &["--settings", &user],
    )
    .await;
    let (launched, mut user_events) = launch_pty(&c, &ws, "claude-code").await;
    let uterm = launched.terminal_id.clone();
    let usid = launched.provider_session_id.clone().unwrap_or_default();
    j.alias(&uterm, "$USER_SETTINGS_TERMINAL");
    j.alias(&usid, "$USER_SETTINGS_SESSION");
    let ulog = launch_log(&sandbox, &uterm);
    appears(&ulog.join("env")).await;
    let uargv = lines(&ulog.join("argv"));
    let uenv = agent_env(&sandbox, &uterm);
    j.alias(
        env_var(&uenv, "HYPRLAYER_AGENT_TOKEN"),
        "$USER_SETTINGS_TOKEN",
    );
    let unlisted = !listed_ids(&c, &ws).await.contains(&uterm);
    j.guard(
        "hooks, MCP and the plugin are all skipped; the user's settings pass through; identity env stays",
        !launched.hooked
            && uargv == ["--settings", user.as_str(), "--session-id", usid.as_str()]
            && env_var(&uenv, "HYPRLAYER_TERMINAL_ID") == uterm
            && unlisted,
        json!({ "hooked": launched.hooked, "argv": uargv, "identityEnv": env_var(&uenv, "HYPRLAYER_TERMINAL_ID") == uterm, "unlisted": unlisted }),
    );
    type_line(&c, &uterm, "exit").await;
    wait_for_exit(&mut user_events, IO).await;
    j.guard(
        "it fires no hooks",
        !ulog.join("hooks.log").exists(),
        json!({ "hooks": lines(&ulog.join("hooks.log")).len() }),
    );
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.step("a hook against a dead server returns at once");
    let dead_env = vec![
        ("HYPRLAYER_AGENT_TOKEN".to_string(), token.clone()),
        (
            "HYPRLAYER_SOCKET".to_string(),
            sandbox.socket().display().to_string(),
        ),
    ];
    let payload = br#"{"session_id":"dead","last_assistant_message":"bye"}"#;
    let codex_dead = run_face(&["hook", "codex", "Stop"], &dead_env, payload).await;
    let claude_dead = run_face(&["hook", "claude", "Stop"], &dead_env, payload).await;
    j.guard(
        "codex and claude hooks exit 0 in under a second; codex still prints {}",
        codex_dead.code == Some(0)
            && codex_dead.stdout == "{}\n"
            && codex_dead.elapsed < Duration::from_secs(1)
            && claude_dead.code == Some(0)
            && claude_dead.stdout.is_empty()
            && claude_dead.elapsed < Duration::from_secs(1),
        json!({
            "codex": { "exit": codex_dead.code, "stdout": codex_dead.stdout, "underASecond": codex_dead.elapsed < Duration::from_secs(1) },
            "claude": { "exit": claude_dead.code, "stdout": claude_dead.stdout, "underASecond": claude_dead.elapsed < Duration::from_secs(1) },
        }),
    );
    // Accepts connections at the kernel and never answers: a hung server.
    let hung_path = sandbox.root.join("tmp").join("hung.sock");
    let _hung = std::os::unix::net::UnixListener::bind(&hung_path).expect("bind a hung socket");
    let hung_env = vec![
        ("HYPRLAYER_AGENT_TOKEN".to_string(), token.clone()),
        (
            "HYPRLAYER_SOCKET".to_string(),
            hung_path.display().to_string(),
        ),
    ];
    let hung = run_face(&["hook", "codex", "Stop"], &hung_env, payload).await;
    j.guard(
        "against a hung server the 500 ms budget ends it, still exit 0 with {}",
        hung.code == Some(0)
            && hung.stdout == "{}\n"
            && hung.elapsed >= Duration::from_millis(450)
            && hung.elapsed < Duration::from_secs(2),
        json!({
            "exit": hung.code,
            "stdout": hung.stdout,
            "waitedForBudget": hung.elapsed >= Duration::from_millis(450),
            "underTwoSeconds": hung.elapsed < Duration::from_secs(2),
        }),
    );

    j.finish();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_worktree_create() {
    let sandbox = Sandbox::new();
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let other = sandbox.fixture_repo("other", &[("README.md", "# other\n")]);
    let mut j = Journey::new("pty_claude_worktrees", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Lead").await;
    let lead = text(&view, "/item/id");
    let lead_tree = text(&view, "/item/worktreePath");
    j.alias(&lead, "$LEAD");
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    let (launched, mut stream) = launch_pty(&c, &lead, "claude-code").await;
    let term = launched.terminal_id.clone();
    j.alias(&term, "$TERMINAL");
    j.alias(
        launched.provider_session_id.clone().unwrap_or_default(),
        "$SESSION",
    );
    let hooks = launch_log(&sandbox, &term).join("hooks.log");
    appears(&hooks).await;
    let worktree = |line: &str| {
        let (c, term, hooks) = (&c, &term, &hooks);
        let line = line.to_string();
        async move {
            let names = line.split_whitespace().skip(1).count();
            let before = count(hooks, "WorktreeCreate ");
            type_line(c, term, &line).await;
            eventually(IO, || count(hooks, "WorktreeCreate ") >= before + names).await;
        }
    };
    let active = || async {
        c.list_workstreams(project.clone(), false)
            .await
            .expect("list_workstreams")
            .into_iter()
            .map(|view| view.item)
            .collect::<Vec<_>>()
    };

    j.step("at Full, a claude worktree is a workstream at its name's path");
    worktree("worktree foo").await;
    let (exit, foo) = created(&hooks, "foo");
    let rows = active().await;
    let row = rows.iter().find(|w| w.worktree_path == foo);
    let moved_in = lines(&hooks).contains(&format!("cwd {foo}"));
    j.guard(
        "the hook prints the workstream's worktree, on its claude branch, nested under the lead, and claude moves in",
        exit == "0"
            && foo.ends_with("/worktrees/demo/foo")
            && row.is_some_and(|w| {
                w.title == "foo"
                    && w.branch == "hyprlayer/claude/foo"
                    && w.worktree_mode == WorktreeMode::Isolated
                    && w.parent_workstream_id.as_deref() == Some(lead.as_str())
            })
            && moved_in,
        json!({
            "exit": exit,
            "path": foo,
            "branch": row.map(|w| w.branch.clone()),
            "nestedUnderLead": row.map(|w| w.parent_workstream_id.as_deref() == Some(lead.as_str())),
            "movedIn": moved_in,
        }),
    );

    j.step("the same name again finds the same worktree");
    worktree("worktree foo").await;
    let (exit, again) = created(&hooks, "foo");
    let named_foo = active().await.iter().filter(|w| w.title == "foo").count();
    j.guard(
        "the second create returns the first's path and adds no workstream",
        exit == "0" && again == foo && named_foo == 1,
        json!({ "exit": exit, "samePath": again == foo, "workstreamsNamedFoo": named_foo }),
    );

    j.step("three subagents at once get three workstreams");
    worktree("worktree agent-a1 agent-b2 agent-c3").await;
    let agents = ["agent-a1", "agent-b2", "agent-c3"].map(|name| created(&hooks, name));
    let rows = active().await;
    let mut paths: Vec<&str> = agents.iter().map(|(_, p)| p.as_str()).collect();
    paths.sort();
    paths.dedup();
    let all_rows = agents
        .iter()
        .all(|(_, p)| rows.iter().any(|w| &w.worktree_path == p));
    j.guard(
        "each create exits 0 with its own path, and each path is a workstream",
        agents.iter().all(|(exit, _)| exit == "0") && paths.len() == 3 && all_rows,
        json!({
            "exits": agents.iter().map(|(exit, _)| exit.clone()).collect::<Vec<_>>(),
            "distinctPaths": paths.len(),
            "allWorkstreams": all_rows,
        }),
    );

    j.step("a cwd in another repo gets claude's own layout there");
    let before = active().await.len();
    type_line(&c, &term, &format!("cd {}", other.display())).await;
    worktree("worktree bar").await;
    let (exit, bar) = created(&hooks, "bar");
    let after = active().await.len();
    let branch = sandbox.branch_exists(&other, "worktree-bar");
    j.guard(
        "the fallback is `<other>/.claude/worktrees/bar` on `worktree-bar`, with no workstream",
        exit == "0" && bar.ends_with("/other/.claude/worktrees/bar") && branch && after == before,
        json!({ "exit": exit, "path": bar, "branch": branch, "workstreamsAdded": after - before }),
    );

    j.step("a name recreated after its archive takes the next free slug");
    type_line(&c, &term, &format!("cd {lead_tree}")).await;
    let old = active()
        .await
        .into_iter()
        .find(|w| w.title == "foo")
        .expect("foo's workstream");
    c.archive_workstream(old.id.clone(), true)
        .await
        .expect("archive foo");
    worktree("worktree foo").await;
    let (exit, recreated) = created(&hooks, "foo");
    let row = active()
        .await
        .into_iter()
        .find(|w| w.worktree_path == recreated);
    j.guard(
        "the create lands at `foo-2` on `claude/foo-2`: the archived row keeps slug `foo` and its branch",
        exit == "0"
            && recreated.ends_with("/worktrees/demo/foo-2")
            && row
                .as_ref()
                .is_some_and(|w| w.id != old.id && w.branch == "hyprlayer/claude/foo-2"),
        json!({ "exit": exit, "path": recreated, "branch": row.map(|w| w.branch) }),
    );

    j.step("a restored claude workstream answers its name again");
    worktree("worktree baz").await;
    let (_, baz) = created(&hooks, "baz");
    let baz_id = active()
        .await
        .into_iter()
        .find(|w| w.worktree_path == baz)
        .expect("baz's workstream")
        .id;
    c.archive_workstream(baz_id.clone(), true)
        .await
        .expect("archive baz");
    c.restore_workstream(baz_id.clone(), None)
        .await
        .expect("restore baz");
    let before = c
        .list_workstreams(project.clone(), true)
        .await
        .expect("list_workstreams")
        .len();
    worktree("worktree baz").await;
    let (exit, again) = created(&hooks, "baz");
    let after = c
        .list_workstreams(project.clone(), true)
        .await
        .expect("list_workstreams")
        .len();
    let answered = active()
        .await
        .into_iter()
        .find(|w| w.worktree_path == again)
        .map(|w| w.id);
    j.guard(
        "the create after the restore returns baz's original path and adds no workstream",
        exit == "0"
            && again == baz
            && answered.as_deref() == Some(baz_id.as_str())
            && after == before,
        json!({
            "exit": exit,
            "path": again,
            "samePath": again == baz,
            "sameWorkstream": answered.as_deref() == Some(baz_id.as_str()),
            "workstreamsAdded": after - before,
        }),
    );

    j.step("at Read, claude makes its own worktree");
    set_level(&mut j, &c, AgentControlLevel::Read).await;
    type_line(&c, &term, &format!("cd {lead_tree}")).await;
    let before = active().await.len();
    worktree("worktree foo").await;
    let (exit, native) = created(&hooks, "foo");
    let after = active().await.len();
    j.guard(
        "the hook prints `<cwd>/.claude/worktrees/foo` and no workstream is added",
        exit == "0" && native == format!("{lead_tree}/.claude/worktrees/foo") && after == before,
        json!({ "exit": exit, "path": native, "workstreamsAdded": after - before }),
    );
    type_line(&c, &term, "exit").await;
    wait_for_exit(&mut stream, IO).await;

    j.step("with the server gone, the hook still makes a worktree at once");
    let token = env_var(&agent_env(&sandbox, &term), "HYPRLAYER_AGENT_TOKEN").to_string();
    server.shutdown(&c).await;
    let dead_env = vec![
        ("HYPRLAYER_AGENT_TOKEN".to_string(), token),
        (
            "HYPRLAYER_SOCKET".to_string(),
            sandbox.socket().display().to_string(),
        ),
    ];
    let payload = json!({ "session_id": "dead", "cwd": repo, "name": "dead" }).to_string();
    let dead = run_face(
        &["hook", "claude", "WorktreeCreate"],
        &dead_env,
        payload.as_bytes(),
    )
    .await;
    let path = dead.stdout.trim();
    j.guard(
        "it prints `<repo>/.claude/worktrees/dead` and exits 0 in under a second",
        dead.code == Some(0)
            && path.ends_with("/demo/.claude/worktrees/dead")
            && Path::new(path).is_dir()
            && dead.elapsed < Duration::from_secs(1),
        json!({
            "exit": dead.code,
            "path": path,
            "underASecond": dead.elapsed < Duration::from_secs(1),
        }),
    );

    j.finish();
}

fn created(hooks: &Path, name: &str) -> (String, String) {
    let prefix = format!("WorktreeCreate {name} exit=");
    lines(hooks)
        .iter()
        .rev()
        .find_map(|line| {
            let (exit, path) = line.strip_prefix(&prefix)?.split_once(" path=")?;
            Some((exit.to_string(), path.to_string()))
        })
        .unwrap_or_default()
}

/// `PtyAgentChanged`s for `terminal` until `stop` matches one, or `IO` passes.
async fn changes_until(
    rx: &mut Receiver<RuntimeEvent>,
    terminal: &str,
    stop: impl Fn(&Change) -> bool,
) -> Vec<Change> {
    events_until(rx, IO, |e| change_of(e, terminal).is_some_and(|c| stop(&c)))
        .await
        .iter()
        .filter_map(|e| change_of(e, terminal))
        .collect()
}

/// Every `PtyAgentChanged` for `terminal` over `within`.
async fn changes_within(
    rx: &mut Receiver<RuntimeEvent>,
    terminal: &str,
    within: Duration,
) -> Vec<Change> {
    events_until(rx, within, |_| false)
        .await
        .iter()
        .filter_map(|e| change_of(e, terminal))
        .collect()
}

fn change_of(event: &RuntimeEvent, terminal: &str) -> Option<Change> {
    match event {
        RuntimeEvent::PtyAgentChanged {
            terminal_id,
            state,
            provider_session_id,
            activity,
            last_reply,
            ..
        } if terminal_id == terminal => Some(Change {
            state: *state,
            provider: provider_session_id.clone(),
            activity: activity.clone(),
            last_reply: last_reply.clone(),
        }),
        _ => None,
    }
}

/// The state sequence with repeats collapsed: activity-only changes are throttled, so their count varies.
fn states(changes: &[Change]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for change in changes {
        let state = json!(change.state).as_str().unwrap_or_default().to_string();
        if out.last() != Some(&state) {
            out.push(state);
        }
    }
    out
}

fn count(path: &Path, prefix: &str) -> usize {
    lines(path).iter().filter(|l| l.starts_with(prefix)).count()
}

fn sorted(items: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    items.sort();
    items
}

async fn pty_agent(j: &mut Journey, c: &RpcClient, ws: &str, terminal: &str) -> Value {
    let agents = j
        .call(
            c,
            Request::ListPtyAgents {
                workstream_id: Some(ws.to_string()),
            },
        )
        .await
        .expect("list_pty_agents");
    agents
        .as_array()
        .and_then(|a| a.iter().find(|a| a["terminalId"] == json!(terminal)))
        .cloned()
        .unwrap_or(Value::Null)
}

async fn listed_ids(c: &RpcClient, ws: &str) -> Vec<String> {
    c.list_pty_agents(Some(ws.to_string()))
        .await
        .expect("list_pty_agents")
        .into_iter()
        .map(|a| a.terminal_id)
        .collect()
}

async fn gone_from_list(c: &RpcClient, ws: &str, terminal: &str) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        if !listed_ids(c, ws).await.iter().any(|t| t == terminal) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// `announce_agent(token)`'s error, each try on a throwaway connection; revocation trails the exit.
async fn announce_refused(server: &Server, token: &str) -> (String, String) {
    let deadline = tokio::time::Instant::now() + IO;
    loop {
        let probe = server.connect().await;
        drop(probe.take_events());
        match probe.announce_agent(token.to_string()).await {
            Err(e) => return (e.code, e.message),
            Ok(_) if tokio::time::Instant::now() >= deadline => {
                return ("accepted".into(), String::new())
            }
            Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// What a hooked codex argv holds, per pair, without its token-bearing values.
fn codex_argv(argv: &[String], ctl: &Path) -> Value {
    let pairs: Vec<&str> = argv
        .chunks(2)
        .filter(|pair| pair[0] == "-c" && pair.len() == 2)
        .map(|pair| pair[1].as_str())
        .collect();
    let mcp = pairs
        .iter()
        .filter(|p| p.starts_with("mcp_servers.hyprlayer."))
        .count()
        == 5
        && argv.len() >= 10
        && argv[..10]
            .chunks(2)
            .all(|p| p[1].starts_with("mcp_servers.hyprlayer."));
    let hooks: Vec<(String, bool)> = pairs
        .iter()
        .filter_map(|p| p.strip_prefix("hooks."))
        .filter_map(|p| p.split_once('='))
        .map(|(event, value)| {
            let command = format!("command='{} hook codex {event}'", ctl.display());
            (event.to_string(), value.contains(&command))
        })
        .collect();
    let mut events: Vec<String> = hooks.iter().map(|(e, _)| e.clone()).collect();
    events.sort();
    json!({
        "mcp": mcp,
        "events": events,
        "commandsRunCtl": !hooks.is_empty() && hooks.iter().all(|(_, ok)| *ok),
        "endsWithTrustBypass": argv.last().map(String::as_str) == Some("--dangerously-bypass-hook-trust"),
    })
}

/// The `mcp_servers.hyprlayer.args` value in a codex argv.
fn mcp_args(argv: &[String]) -> String {
    argv.iter()
        .find_map(|a| a.strip_prefix("mcp_servers.hyprlayer.args="))
        .unwrap_or_default()
        .to_string()
}

async fn whoami(mcp: &mut McpDriver) -> Value {
    serde_json::from_str(&tool_text(&mcp.call("whoami", json!({})).await)).unwrap_or(Value::Null)
}

async fn start_codex(
    j: &mut Journey,
    c: &RpcClient,
    ws: &str,
    mode: PermissionMode,
) -> (
    String,
    hyprlayer_server::client::EventStream<hyprlayer_core::SessionEvent>,
) {
    let (session, _prompt, _stream_id, events) = c
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_CODEX".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Codex),
                permission_mode: Some(mode),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$CODEX_SESSION");
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
