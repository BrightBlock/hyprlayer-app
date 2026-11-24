//! The agent-library surface: the files under the profile's agent bundle, and
//! what the app reads back out of them. `agent_bundle.rs` is pure and
//! filesystem-only, so all of this runs inside the sandbox.
//!
//! The install/update/repair commands are deliberately absent: they fetch from
//! GitHub and rewrite the bundle. The sandbox seeds that bundle, so nothing here
//! reaches GitHub; `agent_bundle_status` stays because on a 1.6.0+ CLI it is a
//! local version comparison (ADR-0002).
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
    // The sandbox seeds the CLI's bundle and the CLI links it into the profile's
    // home at startup, so the home is not empty; that bundle being readable is the property.
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

    j.step("skill files");
    let probe_dir = repo.join(".claude/skills/e2e-probe");
    std::fs::create_dir_all(probe_dir.join("refs")).expect("create the probe skill dirs");
    std::fs::write(
        probe_dir.join("SKILL.md"),
        "---\nname: e2e-probe\ndescription: Probe skill.\nallowed-tools: Read\n---\n\nProbe the skill reader.\n",
    )
    .expect("write the probe SKILL.md");
    std::fs::write(probe_dir.join("refs/notes.md"), "notes\n").expect("write the probe asset");
    let files = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list_skill_files");
    let skills = j
        .call(
            &client,
            Request::AgentSkills {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("agent_skills");
    let user_skills = entries_of(&skills, "user").len();
    let mut user_slash_names: Vec<String> = entries_of(&files, "user")
        .iter()
        .map(|f| text(f, "/slashName"))
        .collect();
    user_slash_names.sort();
    user_slash_names.dedup();
    let project_files = entries_of(&files, "project");
    let all_files = files.as_array().map(Vec::as_slice).unwrap_or_default();
    let managed = all_files
        .iter()
        .filter(|f| f.pointer("/managed") == Some(&Value::Bool(true)))
        .count();
    j.guard(
        "skill files list every installed skill plus the project probe",
        user_slash_names.len() == user_skills && project_files.len() == 1,
        json!({
            "entries": all_files.len(),
            "sources": sources(&files),
            "managed": managed,
        }),
    );
    let well_formed = all_files.iter().all(|f| {
        text(f, "/path").ends_with("/SKILL.md")
            && text(f, "/path").starts_with(&text(f, "/dir"))
            && !text(f, "/slashName").is_empty()
    });
    j.guard(
        "every skill file points at its SKILL.md and has a slash name",
        !all_files.is_empty() && well_formed,
        json!({ "entries": all_files.len(), "well_formed": well_formed }),
    );
    let probe = project_files.first();
    let keys_of = |pointer: &str, key: &str| -> Vec<String> {
        probe
            .and_then(|p| p.pointer(pointer))
            .and_then(Value::as_array)
            .map(|items| items.iter().map(|i| text(i, key)).collect())
            .unwrap_or_default()
    };
    let field_keys = keys_of("/frontmatter", "/key");
    let asset_paths = keys_of("/assets", "/relPath");
    let agent_ref = probe.map(|p| text(p, "/agentRef")).unwrap_or_default();
    j.guard(
        "the probe keeps frontmatter order and lists its assets",
        field_keys == ["name", "description", "allowed-tools"]
            && asset_paths == ["refs/notes.md"]
            && agent_ref == "e2e-probe",
        json!({ "frontmatter": field_keys, "assets": asset_paths, "agent_ref": agent_ref }),
    );

    let read = |path: String| Request::ReadSkillFile {
        project_path: Some(project.clone()),
        path,
    };
    let probe_path = probe.map(|p| text(p, "/path")).unwrap_or_default();
    let probe_text = j.call(&client, read(probe_path)).await;
    let probe_text = probe_text.as_ref().map(|v| text(v, "")).unwrap_or_default();
    j.guard(
        "reading the probe returns its whole SKILL.md",
        probe_text.starts_with("---\nname: e2e-probe\n")
            && probe_text.ends_with("Probe the skill reader.\n"),
        json!({ "chars": probe_text.chars().count() }),
    );
    let managed_path = all_files
        .iter()
        .find(|f| f.pointer("/managed") == Some(&Value::Bool(true)))
        .map(|f| text(f, "/path"))
        .unwrap_or_default();
    let managed_text = j.call(&client, read(managed_path)).await;
    j.guard(
        "a bundle skill reads through its symlinked directory",
        managed_text
            .as_ref()
            .is_ok_and(|v| text(v, "").starts_with("---")),
        json!({ "ok": managed_text.is_ok() }),
    );
    let asset = j
        .call(
            &client,
            read(
                probe_dir
                    .join("refs/notes.md")
                    .to_string_lossy()
                    .into_owned(),
            ),
        )
        .await
        .err()
        .map(|e| e.code);
    let stray = repo.join("stray/SKILL.md");
    std::fs::create_dir_all(repo.join("stray")).expect("create the stray dir");
    std::fs::write(&stray, "---\nname: stray\n---\n").expect("write the stray SKILL.md");
    let outside = j
        .call(&client, read(stray.to_string_lossy().into_owned()))
        .await
        .err()
        .map(|e| e.code);
    j.guard(
        "only a SKILL.md inside a skill root can be read",
        asset.is_some() && outside.is_some(),
        json!({ "asset": asset, "outside_roots": outside }),
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
                project_path: Some(project),
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

fn entries_of<'a>(items: &'a Value, source: &str) -> Vec<&'a Value> {
    items
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|i| text(i, "/source") == source)
                .collect()
        })
        .unwrap_or_default()
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
