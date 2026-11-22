//! The orchestration plugin: written per server version at start, stale versions pruned, and
//! passed per launch to composer and PTY claude (`--plugin-dir`) and composer codex (an extra
//! skill root); nothing at `off`, and nothing when the plugin can't be written.
//!
//! The harnesses are `goal_stub`'s claude and `agent_stub`'s PTY claude and codex.
#![cfg(unix)]

use hyprlayer_core::models::AgentControlLevel;
use hyprlayer_core::settings::LeadRuleScope;
use hyprlayer_core::{Harness, Stage};
use hyprlayer_e2e::agent_stub::{
    appears, install, launch_log, launch_pty, lines, run_face, set_engine, set_level, type_line,
    update_settings, PTY_CLAUDE, PTY_CODEX,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{
    eventually, json, text, wait_for_exit, Journey, Request, Sandbox, Server, Value,
};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: std::time::Duration = std::time::Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_skill_plugin() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    sandbox.stage_tool("codex", PTY_CODEX);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let plugin_root = sandbox.app_data.join("agent-control").join("plugin");
    let stale = plugin_root.join("0.0.1");
    std::fs::create_dir_all(stale.join("skills")).expect("seed a stale plugin version");
    std::fs::write(stale.join("skills").join("old.md"), "old\n").expect("seed a stale skill");

    let mut j = Journey::new("agent_skill_plugin", &sandbox);
    let version = run_face(&["--version"], &[], b"")
        .await
        .stdout
        .trim()
        .to_string();
    let plugin = plugin_root.join(&version);
    j.alias(plugin.display().to_string(), "$PLUGIN");
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Plugin").await;
    let ws = text(&view, "/item/id");
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    j.step("the plugin is written per version and stale versions are pruned");
    let versions: Vec<String> = std::fs::read_dir(&plugin_root)
        .map(|dir| {
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    j.guard(
        "only this server's version dir is left",
        versions == [version.clone()],
        json!({ "onlyCurrent": versions == [version.clone()], "staleGone": !stale.exists() }),
    );
    let manifest = sandbox.read_json(&plugin.join(".claude-plugin").join("plugin.json"));
    let skill = sandbox
        .read(&plugin.join("skills").join("orchestrate").join("SKILL.md"))
        .unwrap_or_default();
    let named = skill.starts_with("---\n") && skill.contains("\nname: orchestrate\n");
    j.guard(
        "the manifest names the hyprlayer plugin at this version, beside the orchestrate skill",
        manifest["name"] == json!("hyprlayer") && manifest["version"] == json!(version) && named,
        json!({
            "name": manifest["name"],
            "versionMatches": manifest["version"] == json!(version),
            "skillNamed": named,
        }),
    );

    j.step("composer and PTY claude get --plugin-dir");
    let composer = composer_claude(&mut j, &c, &sandbox, &ws, "$CLAUDE_SESSION").await;
    let mcp_then_plugin = follows(&composer, "--mcp-config", "--plugin-dir");
    j.guard(
        "composer claude passes the plugin dir right after its MCP config",
        mcp_then_plugin && flag(&composer, "--plugin-dir") == Some(plugin.display().to_string()),
        json!({
            "afterMcpConfig": mcp_then_plugin,
            "pluginDir": flag(&composer, "--plugin-dir"),
        }),
    );
    let (pty, settings_then_plugin) = pty_claude_argv(&c, &sandbox, &ws).await;
    j.alias(&pty.0, "$PTY");
    j.guard(
        "PTY claude passes the plugin dir after its hook settings",
        pty.1
            && settings_then_plugin
            && flag(&pty.2, "--plugin-dir") == Some(plugin.display().to_string()),
        json!({
            "hooked": pty.1,
            "afterSettings": settings_then_plugin,
            "pluginDir": flag(&pty.2, "--plugin-dir"),
        }),
    );
    let (handshaken, roots) =
        composer_codex_roots(&mut j, &c, &sandbox, &ws, "$CODEX_SESSION", false).await;
    let skills_root = plugin.join("skills").display().to_string();
    j.guard(
        "composer codex gets the plugin's skills as an extra root",
        handshaken && roots.contains(&skills_root),
        json!({ "handshaken": handshaken, "pluginSkills": roots.contains(&skills_root) }),
    );

    j.step("a Lead launches as its own agent with the enabled guardrails");
    let rules = std::path::Path::new(&project)
        .join(".claude")
        .join("agents");
    std::fs::create_dir_all(&rules).expect("create the project's agents dir");
    std::fs::write(
        rules.join("lead-engineer.md"),
        "---\nname: lead-engineer\n---\n\nE2E_PROJECT_RULE\n",
    )
    .expect("write the project's Lead rules");
    update_settings(&mut j, &c, |s| {
        let lead = s.agent_control.lead.get_or_insert_with(Default::default);
        lead.rules[0].enabled = false;
        lead.projects
            .insert(project.clone(), LeadRuleScope::InheritAndFile);
    })
    .await;
    let lead = message_lead(&mut j, &c, &project).await;
    j.alias(&lead, "$LEAD_SESSION");
    let first = launches(&sandbox, &lead, 1)
        .await
        .into_iter()
        .next()
        .unwrap_or_default();
    j.guard(
        "the launch names the Lead agent, its enabled rules and the project's own file",
        first.contains("--agent hyprlayer-lead-engineer")
            && first.contains("Git through git and gh")
            && !first.contains("Merge only after validation")
            && first.contains("E2E_PROJECT_RULE"),
        json!({
            "agent": first.contains("--agent hyprlayer-lead-engineer"),
            "enabledRule": first.contains("Git through git and gh"),
            "disabledRule": first.contains("Merge only after validation"),
            "projectFile": first.contains("E2E_PROJECT_RULE"),
        }),
    );
    j.guard(
        "the Lead launches in auto mode, without file tools, behind the checkout guard",
        first.contains("--permission-mode auto")
            && first.contains(r#""disallowedTools":["Edit","Write","NotebookEdit"]"#)
            && first.contains(" lead-guard "),
        json!({
            "auto": first.contains("--permission-mode auto"),
            "noFileTools": first.contains(r#""disallowedTools":["Edit","Write","NotebookEdit"]"#),
            "guardHook": first.contains(" lead-guard "),
        }),
    );
    update_settings(&mut j, &c, |s| {
        s.agent_control
            .lead
            .get_or_insert_with(Default::default)
            .projects
            .insert(project.clone(), LeadRuleScope::InheritOnly);
    })
    .await;
    kill_and_wait(&mut j, &c, &lead).await;
    let resumed = message_lead(&mut j, &c, &project).await;
    let second = launches(&sandbox, &lead, 2).await.pop().unwrap_or_default();
    j.guard(
        "under Inherit only the resumed Lead keeps its rules and drops the project's file",
        resumed == lead
            && second.contains("--resume")
            && second.contains("Git through git and gh")
            && !second.contains("E2E_PROJECT_RULE"),
        json!({
            "sameSession": resumed == lead,
            "resumed": second.contains("--resume"),
            "enabledRule": second.contains("Git through git and gh"),
            "projectFile": second.contains("E2E_PROJECT_RULE"),
        }),
    );

    j.step("off omits the plugin");
    set_level(&mut j, &c, AgentControlLevel::Off).await;
    let composer = composer_claude(&mut j, &c, &sandbox, &ws, "$OFF_CLAUDE_SESSION").await;
    j.guard(
        "composer claude gets neither MCP nor the plugin",
        !composer
            .iter()
            .any(|a| a == "--plugin-dir" || a == "--mcp-config"),
        json!({
            "pluginDir": composer.iter().any(|a| a == "--plugin-dir"),
            "mcpConfig": composer.iter().any(|a| a == "--mcp-config"),
        }),
    );
    let (pty, _) = pty_claude_argv(&c, &sandbox, &ws).await;
    j.guard(
        "PTY claude launches unhooked and without the plugin",
        !pty.1 && !pty.2.iter().any(|a| a == "--plugin-dir"),
        json!({ "hooked": pty.1, "pluginDir": pty.2.iter().any(|a| a == "--plugin-dir") }),
    );
    let (handshaken, roots) =
        composer_codex_roots(&mut j, &c, &sandbox, &ws, "$OFF_CODEX_SESSION", true).await;
    j.guard(
        "composer codex's skill roots leave the plugin out",
        handshaken && !roots.contains(&skills_root),
        json!({ "handshaken": handshaken, "pluginSkills": roots.contains(&skills_root) }),
    );

    j.step("an unwritable plugin dir is left out of every launch");
    let stopped = server.shutdown(&c).await;
    std::fs::remove_dir_all(&plugin_root).expect("remove the plugin root");
    std::fs::write(&plugin_root, "not a directory\n").expect("block the plugin root");
    let server = Server::start(&sandbox).await;
    let c = server.client();
    set_level(&mut j, &c, AgentControlLevel::Read).await;
    let composer = composer_claude(&mut j, &c, &sandbox, &ws, "$BLOCKED_CLAUDE_SESSION").await;
    j.guard(
        "composer claude keeps its MCP config but loses the plugin",
        stopped.status.success()
            && composer.iter().any(|a| a == "--mcp-config")
            && !composer.iter().any(|a| a == "--plugin-dir"),
        json!({
            "restarted": stopped.status.success(),
            "mcpConfig": composer.iter().any(|a| a == "--mcp-config"),
            "pluginDir": composer.iter().any(|a| a == "--plugin-dir"),
        }),
    );
    let (pty, _) = pty_claude_argv(&c, &sandbox, &ws).await;
    let settings = pty.2.iter().any(|a| a == "--settings");
    j.guard(
        "PTY claude stays hooked without the plugin",
        pty.1 && settings && !pty.2.iter().any(|a| a == "--plugin-dir"),
        json!({
            "hooked": pty.1,
            "settings": settings,
            "pluginDir": pty.2.iter().any(|a| a == "--plugin-dir"),
        }),
    );
    let warned = sandbox
        .server_log()
        .contains("agents get no orchestration skill");
    j.guard("the server logs why", warned, json!({ "warned": warned }));

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

/// A composer claude session's launch argv, one token per element (`goal_stub` space-joins it).
async fn composer_claude(
    j: &mut Journey,
    c: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
    alias: &str,
) -> Vec<String> {
    let id = start_composer(j, c, ws, Harness::Claude, alias).await;
    let log = sandbox.root.join("goal-control").join(&id).join("argv.log");
    appears(&log).await;
    lines(&log)
        .first()
        .map(|line| line.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

async fn message_lead(j: &mut Journey, c: &RpcClient, project: &str) -> String {
    let id = j
        .call(
            c,
            Request::MessageLead {
                project_path: project.to_string(),
                text: "E2E_LEAD".into(),
            },
        )
        .await
        .expect("message_lead");
    id.as_str().unwrap_or_default().to_string()
}

async fn launches(sandbox: &Sandbox, session_id: &str, count: usize) -> Vec<String> {
    let log = sandbox
        .root
        .join("goal-control")
        .join(session_id)
        .join("argv.log");
    eventually(IO, || lines(&log).len() >= count).await;
    lines(&log)
}

async fn kill_and_wait(j: &mut Journey, c: &RpcClient, session_id: &str) {
    j.call(
        c,
        Request::KillSession {
            session_id: session_id.to_string(),
        },
    )
    .await
    .expect("kill_session");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let live = c
            .list_active_sessions()
            .await
            .expect("list_active_sessions");
        if !live.iter().any(|a| a.session.id == session_id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Whether a composer codex session finished its handshake, and the extra skill roots it set.
async fn composer_codex_roots(
    j: &mut Journey,
    c: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
    alias: &str,
    off: bool,
) -> (bool, Vec<String>) {
    let id = start_composer(j, c, ws, Harness::Codex, alias).await;
    // At `off` there is no session id in the env to key the stub's log by.
    let log = launch_log(sandbox, if off { "app-server" } else { &id }).join("requests.log");
    let requests: Vec<Value> = lines(&log)
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let handshaken = requests
        .iter()
        .any(|r| r["method"] == json!("thread/start"));
    let roots = requests
        .iter()
        .filter(|r| r["method"] == json!("skills/extraRoots/set"))
        .flat_map(|r| {
            r["params"]["extraRoots"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|root| root.as_str().map(str::to_string))
        .collect();
    (handshaken, roots)
}

async fn start_composer(
    j: &mut Journey,
    c: &RpcClient,
    ws: &str,
    harness: Harness,
    alias: &str,
) -> String {
    let (session, _prompt, _stream_id, _events) = c
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_PLUGIN".into(),
            vec![],
            StreamStartOpts {
                harness: Some(harness),
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
    session.id
}

/// Launches PTY claude, reads its argv and lets it exit: `((terminal id, hooked, argv), --settings then --plugin-dir)`.
async fn pty_claude_argv(
    c: &RpcClient,
    sandbox: &Sandbox,
    ws: &str,
) -> ((String, bool, Vec<String>), bool) {
    let (launched, mut events) = launch_pty(c, ws, "claude-code").await;
    let key = if launched.hooked {
        launched.terminal_id.clone()
    } else {
        format!(
            "session-{}",
            launched.provider_session_id.clone().unwrap_or_default()
        )
    };
    let argv_path = launch_log(sandbox, &key).join("argv");
    appears(&argv_path).await;
    let argv = lines(&argv_path);
    type_line(c, &launched.terminal_id, "exit").await;
    wait_for_exit(&mut events, IO).await;
    let ordered = follows(&argv, "--settings", "--plugin-dir");
    ((launched.terminal_id, launched.hooked, argv), ordered)
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == name)
        .and_then(|i| argv.get(i + 1))
        .cloned()
}

/// `second` is the flag right after `first`'s single value.
fn follows(argv: &[String], first: &str, second: &str) -> bool {
    argv.iter()
        .position(|a| a == first)
        .is_some_and(|i| argv.get(i + 2).is_some_and(|a| a == second))
}
