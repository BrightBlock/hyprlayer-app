//! The agent-library surface: the files under the profile's agent bundle, and
//! what the app reads back out of them. `agent_bundle.rs` is pure and
//! filesystem-only, so all of this runs inside the sandbox.
//!
//! The install/update/repair commands are deliberately absent. They fetch from
//! GitHub and rewrite the bundle, which is the one thing the sandbox's pins
//! exist to prevent; `agent_bundle_status` is included because on a 1.6.0+ CLI
//! it is a local version comparison that fetches nothing (ADR-0002).
#![cfg(unix)]

use hyprlayer_core::agents::{AgentFile, AgentOverlay};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_files() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("agent_files", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("onboarding installs the CLI's agent bundle");
    // `thoughts init` installs the CLI's own agents into the profile's home, so
    // a freshly onboarded sandbox is not empty — that install is the property.
    let bundled = j
        .call(
            &client,
            Request::ListAgentFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list_agent_files");
    j.guard(
        "the bundle's agents are readable after onboarding",
        bundled.as_array().is_some_and(|a| !a.is_empty())
            && sources(&bundled).contains(&"user".to_string()),
        json!({ "count": bundled.as_array().map(|a| a.len()), "sources": sources(&bundled) }),
    );

    j.step("write one");
    let path = j
        .call(
            &client,
            Request::NewAgentFilePath {
                name: "e2e-probe".into(),
                source: "project".into(),
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("new_agent_file_path");
    let path = text(&path, "");
    j.guard(
        "a new agent path is offered under the project",
        path.ends_with("e2e-probe.md") && !path.is_empty(),
        json!({ "named_for_the_agent": path.ends_with("e2e-probe.md") }),
    );
    j.alias(&path, "$AGENT_PATH");
    let agent = AgentFile {
        path: path.clone(),
        source: "project".into(),
        plugin: None,
        managed: false,
        name: "e2e-probe".into(),
        description: "probe agent".into(),
        tools: vec!["Read".into()],
        skills: vec![],
        model: None,
        color: None,
        memory: None,
        category: None,
        prompt: "You are a probe.".into(),
        updated_at: None,
        shipped_path: None,
        modified: false,
    };
    let saved = j
        .call(
            &client,
            Request::SaveAgentFile {
                agent: agent.clone(),
            },
        )
        .await
        .expect("save_agent_file");
    j.guard(
        "saving returns the agent it wrote",
        text(&saved, "/name") == "e2e-probe" && text(&saved, "/description") == "probe agent",
        json!({ "name": saved.pointer("/name"), "tools": saved.pointer("/tools") }),
    );
    j.guard(
        "the file exists on disk where the path said",
        std::path::Path::new(&path).is_file(),
        json!({ "on_disk": std::path::Path::new(&path).is_file() }),
    );

    j.step("read it back");
    let listed = j
        .call(
            &client,
            Request::ListAgentFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list_agent_files");
    j.guard(
        "the saved agent is listed",
        names(&listed).contains(&"e2e-probe".to_string()),
        json!({ "agents": names(&listed) }),
    );
    let exported = j
        .call(
            &client,
            Request::ExportAgentFile {
                agent,
                overlay: AgentOverlay::default(),
            },
        )
        .await
        .expect("export_agent_file");
    // `export_agent` writes a file and returns its path, not the rendered text.
    let exported_path = text(&exported, "");
    j.alias(&exported_path, "$EXPORT_PATH");
    let rendered = std::fs::read_to_string(&exported_path).unwrap_or_default();
    j.guard(
        "exporting writes a file carrying the agent's prompt",
        std::path::Path::new(&exported_path).is_file() && rendered.contains("You are a probe."),
        json!({
            "wrote_file": std::path::Path::new(&exported_path).is_file(),
            "has_prompt": rendered.contains("You are a probe."),
        }),
    );

    j.step("the bundle's other readers");
    for (label, request) in [
        (
            "context files",
            Request::AgentContextFiles {
                project_path: Some(project.clone()),
            },
        ),
        (
            "mcp servers",
            Request::AgentMcpServers {
                project_path: Some(project.clone()),
            },
        ),
        (
            "skills",
            Request::AgentSkills {
                project_path: Some(project.clone()),
            },
        ),
    ] {
        let out = j.call(&client, request).await;
        let evidence = match &out {
            Ok(v) => json!({ "entries": v.as_array().map(|a| a.len()) }),
            Err(e) => json!({ "code": e.code }),
        };
        j.guard(
            &format!("{label} read from the installed bundle"),
            out.is_ok(),
            evidence,
        );
    }
    let counts = j
        .call(&client, Request::AgentHarnessCounts {})
        .await
        .expect("agent_harness_counts");
    let claude = counts
        .as_array()
        .and_then(|a| a.iter().find(|c| text(c, "/platformId") == "claude"));
    let claude_root = sandbox.home.join(".claude");
    let (disk_agents, disk_skills) = (
        count_ext(&claude_root.join("agents"), "md"),
        count_skill_dirs(&claude_root.join("skills")),
    );
    let claude_matches_disk = claude.is_some_and(|c| {
        c.pointer("/agents").and_then(Value::as_u64) == Some(disk_agents)
            && c.pointer("/skills").and_then(Value::as_u64) == Some(disk_skills)
    });
    j.guard(
        "harness counts match the files on disk",
        claude_matches_disk && disk_agents > 0,
        json!({ "platforms": counts, "claude_matches_disk": claude_matches_disk }),
    );
    let view = create_workstream(&mut j, &client, &project, "Agent work").await;
    let ws = text(&view, "/item/id");
    let commands = j
        .call(&client, Request::ListSlashCommands { id: ws })
        .await
        .expect("list_slash_commands");
    j.guard(
        "slash commands read from the worktree",
        commands.is_array(),
        json!({ "commands": commands.as_array().map(|c| c.len()) }),
    );

    j.step("bundle status");
    // Offline on a 1.6.0+ CLI: a comparison of the CLI's recorded assets
    // version against the pin, with nothing fetched (ADR-0002).
    let status = j
        .call(&client, Request::AgentBundleStatus {})
        .await
        .expect("agent_bundle_status");
    j.guard(
        "status answers offline",
        status.get("manageable").is_some(),
        json!({
            "manageable": status.pointer("/manageable"),
            "update_available": status.pointer("/updateAvailable"),
        }),
    );

    j.step("delete it");
    j.call(&client, Request::DeleteAgentFile { path: path.clone() })
        .await
        .expect("delete_agent_file");
    let after = j
        .call(
            &client,
            Request::ListAgentFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list_agent_files");
    j.guard(
        "deleting removes it from the listing and the disk",
        !names(&after).contains(&"e2e-probe".to_string()) && !std::path::Path::new(&path).exists(),
        json!({ "agents": names(&after), "on_disk": std::path::Path::new(&path).exists() }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn sources(agents: &Value) -> Vec<String> {
    let mut all: Vec<String> = agents
        .as_array()
        .map(|items| items.iter().map(|a| text(a, "/source")).collect())
        .unwrap_or_default();
    all.sort();
    all.dedup();
    all
}

fn names(agents: &Value) -> Vec<String> {
    agents
        .as_array()
        .map(|items| items.iter().map(|a| text(a, "/name")).collect())
        .unwrap_or_default()
}

fn count_ext(dir: &std::path::Path, ext: &str) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|x| x == ext))
                .count() as u64
        })
        .unwrap_or(0)
}

fn count_skill_dirs(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("SKILL.md").is_file())
                .count() as u64
        })
        .unwrap_or(0)
}
