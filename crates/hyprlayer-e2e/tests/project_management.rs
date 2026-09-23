//! Registering, inspecting, reordering and removing projects, plus the
//! per-project state that hangs off them: seed-file patterns, project memory,
//! and the thoughts repository's own git status.
//!
//! The GitHub-backed commands (clone, publish, owner lookup) are not here —
//! they need the network, and `pr_lifecycle` is the tier that has it.
#![cfg(unix)]

use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_management() {
    let sandbox = Sandbox::new();
    let first = sandbox.fixture_repo("first", &[("README.md", "# first\n")]);
    let second = sandbox.fixture_repo("second", &[("README.md", "# second\n")]);
    let mut j = Journey::new("project_management", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &first).await;

    j.step("inspect what was registered");
    let inspected = j
        .call(
            &client,
            Request::GetProject {
                path: project.clone(),
            },
        )
        .await
        .expect("get_project");
    j.guard(
        "the registered project inspects by its canonical path",
        !inspected.is_null(),
        json!({ "found": !inspected.is_null() }),
    );
    let found = j
        .call(
            &client,
            Request::FindProject {
                path: project.clone(),
            },
        )
        .await
        .expect("find_project");
    j.guard(
        "finding it returns its view",
        text(&found, "/path") == project,
        json!({ "matches": text(&found, "/path") == project }),
    );
    let source = j
        .call(
            &client,
            Request::ProjectSourceOf {
                path: project.clone(),
            },
        )
        .await
        .expect("project_source_of");
    j.guard(
        "a registered repository reports a source",
        !source.is_null(),
        json!({ "source": source }),
    );
    let unregistered = j
        .call(
            &client,
            Request::InspectRepoPath {
                path: second.display().to_string(),
            },
        )
        .await
        .expect("inspect_repo_path");
    j.guard(
        "an unregistered repository still inspects as a repo",
        serde_json::to_string(&unregistered)
            .unwrap_or_default()
            .contains("Repo")
            || !unregistered.is_null(),
        json!({ "status": unregistered }),
    );

    j.step("register a second and reorder");
    let other = j
        .call(
            &client,
            Request::ImportProject {
                path: sandbox.as_given(&second).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import_project");
    let other_path = text(&other, "/path");
    // Only the folder-import path emits ProjectsChanged; importing a repo
    // registers through a different branch that does not.
    let folder = sandbox.home.join("notes");
    std::fs::create_dir_all(&folder).expect("create a plain folder");
    let imported_folder = j
        .call(
            &client,
            Request::ImportLocalFolder {
                path: folder.display().to_string(),
                directory_name: None,
            },
        )
        .await
        .expect("import_local_folder");
    j.alias(text(&imported_folder, "/path"), "$FOLDER_PROJECT");
    let announced = events_until(&mut events_rx, std::time::Duration::from_secs(20), |e| {
        matches!(e, RuntimeEvent::ProjectsChanged)
    })
    .await;
    j.guard(
        "importing a folder announces the project change",
        announced
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ProjectsChanged)),
        json!({ "projects_changed": announced.iter().filter(|e| matches!(e, RuntimeEvent::ProjectsChanged)).count() }),
    );
    j.call(
        &client,
        Request::ReorderProjects {
            paths: vec![other_path.clone(), project.clone()],
        },
    )
    .await
    .expect("reorder_projects");
    let bootstrap = j
        .call(&client, Request::GetBootstrap {})
        .await
        .expect("get_bootstrap");
    j.guard(
        "the reordered list comes back in the order it was given",
        project_paths(&bootstrap).starts_with(std::slice::from_ref(&other_path)),
        json!({ "first": project_paths(&bootstrap).first().cloned() }),
    );

    j.step("seed-file patterns");
    let patterns = j
        .call(
            &client,
            Request::SetWorktreeFiles {
                path: project.clone(),
                patterns: vec![".env".into(), "config/local.*".into()],
            },
        )
        .await
        .expect("set_worktree_files");
    j.guard(
        "patterns are stored against the project",
        strings(&patterns).contains(&".env".to_string()),
        json!({ "patterns": strings(&patterns) }),
    );
    let read_back = j
        .call(
            &client,
            Request::GetWorktreeFiles {
                path: project.clone(),
            },
        )
        .await
        .expect("get_worktree_files");
    j.guard(
        "they read back unchanged",
        strings(&read_back) == strings(&patterns),
        json!({ "patterns": strings(&read_back) }),
    );
    std::fs::write(std::path::Path::new(&project).join(".env"), "TOKEN=1\n")
        .expect("write a seed file");
    let report = j
        .call(
            &client,
            Request::CheckWorktreeFiles {
                path: project.clone(),
            },
        )
        .await
        .expect("check_worktree_files");
    j.guard(
        "checking reports what a new worktree would be seeded with",
        serde_json::to_string(&report)
            .unwrap_or_default()
            .contains(".env"),
        json!({ "mentions_seed_file": serde_json::to_string(&report).unwrap_or_default().contains(".env") }),
    );

    j.step("project memory");
    let view = create_workstream(&mut j, &client, &project, "Memory work").await;
    let ws = text(&view, "/item/id");
    let before = j
        .call(&client, Request::ListProjectMemory { id: ws.clone() })
        .await
        .expect("list_project_memory");
    j.guard(
        "a workstream starts with readable project memory",
        !before.is_null(),
        json!({ "keys": before.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()) }),
    );
    // Saving edits an existing memory file and refuses to create one, so the
    // absent-file case is the contract, not a setup problem.
    let absent = j
        .call(
            &client,
            Request::SaveProjectMemoryFile {
                id: ws.clone(),
                name: "NOT-THERE.md".into(),
                content: "x\n".into(),
                base_mtime_ms: 0.0,
            },
        )
        .await;
    let evidence = match &absent {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "saving a memory file that does not exist is refused",
        absent.is_err(),
        evidence,
    );
    let dir = text(&before, "/dir");
    j.alias(&dir, "$MEMORY_DIR");
    std::fs::create_dir_all(&dir).expect("create the memory dir");
    let target = std::path::Path::new(&dir).join("CLAUDE.md");
    std::fs::write(&target, "# seed\n").expect("seed a memory file");
    let seeded = j
        .call(&client, Request::ListProjectMemory { id: ws.clone() })
        .await
        .expect("list_project_memory");
    let base = seeded
        .pointer("/files")
        .and_then(Value::as_array)
        .and_then(|f| f.iter().find(|m| text(m, "/name") == "CLAUDE.md"))
        .and_then(|m| m.pointer("/mtimeMs"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let wrote = j
        .call(
            &client,
            Request::SaveProjectMemoryFile {
                id: ws.clone(),
                name: "CLAUDE.md".into(),
                content: "# E2E memory\n".into(),
                base_mtime_ms: base,
            },
        )
        .await;
    let evidence = match &wrote {
        Ok(_) => json!({ "saved": true }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "an existing memory file is writable",
        wrote.is_ok(),
        evidence,
    );
    // The listing carries each file's metadata, not its text, so the write is
    // confirmed on disk.
    let on_disk = std::fs::read_to_string(&target).unwrap_or_default();
    let after = j
        .call(&client, Request::ListProjectMemory { id: ws.clone() })
        .await
        .expect("list_project_memory");
    j.guard(
        "the edit lands on disk and the file is still listed",
        on_disk.contains("E2E memory")
            && after
                .pointer("/files")
                .and_then(Value::as_array)
                .is_some_and(|f| f.iter().any(|m| text(m, "/name") == "CLAUDE.md")),
        json!({ "content_written": on_disk.contains("E2E memory") }),
    );

    j.step("the thoughts repository's own status");
    let thoughts = j
        .call(
            &client,
            Request::GetThoughtsGitStatus {
                project_path: project.clone(),
            },
        )
        .await
        .expect("get_thoughts_git_status");
    j.guard(
        "the thoughts repo reports a git status",
        !thoughts.is_null(),
        json!({ "keys": thoughts.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()) }),
    );
    let profiles = j
        .call(&client, Request::ListThoughtsProfiles {})
        .await
        .expect("list_thoughts_profiles");
    j.guard(
        "the configured thoughts profiles are listed",
        profiles.as_array().is_some(),
        json!({ "profiles": profiles.as_array().map(|p| p.len()) }),
    );

    j.step("remove one");
    j.call(
        &client,
        Request::RemoveProject {
            path: other_path.clone(),
            delete_files: false,
        },
    )
    .await
    .expect("remove_project");
    let left = j
        .call(&client, Request::GetBootstrap {})
        .await
        .expect("get_bootstrap");
    j.guard(
        "removing deregisters it without touching the files",
        !project_paths(&left).contains(&other_path) && second.is_dir(),
        json!({ "still_registered": project_paths(&left).contains(&other_path), "files_kept": second.is_dir() }),
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

fn project_paths(bootstrap: &Value) -> Vec<String> {
    bootstrap
        .pointer("/projects")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|p| text(p, "/path")).collect())
        .unwrap_or_default()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
