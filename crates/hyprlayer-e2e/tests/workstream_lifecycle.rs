//! Import a repository, then take one workstream through create, a terminal,
//! a staged edit, archive, restore and purge against the real server.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::runtime::projects::ThoughtsSource;
use hyprlayer_core::WorktreeMode;
use hyprlayer_e2e::{
    json, keystrokes, read_until, text, wait_for_exit, Journey, Request, Sandbox, Server, Value,
};

const SETTINGS: &str = r#"[scripts]
setup = 'pwd > "$HOME/e2e-setup-ran"'
archive = 'pwd > "$HOME/e2e-archive-ran"'
"#;

const IO: Duration = Duration::from_secs(15);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workstream_lifecycle() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# demo\n"),
            (".hyprlayer/settings.toml", SETTINGS),
        ],
    );
    let mut j = Journey::new("workstream_lifecycle", &sandbox);

    j.step("start");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    j.guard(
        "a fresh profile starts and listens",
        true,
        json!({ "socket": server.socket() }),
    );
    j.guard(
        "client and server agree on the protocol",
        client.server_version() == hyprlayer_proto::PROTOCOL_VERSION,
        json!({ "protocol": client.server_version() }),
    );
    j.note(
        "server",
        json!({ "version": client.server_semver(), "build": client.server_build() }),
    );

    j.step("onboard");
    let bootstrap = j
        .call(&client, Request::GetBootstrap {})
        .await
        .expect("get_bootstrap");
    j.guard(
        "a fresh profile asks for onboarding",
        bootstrap.pointer("/config/Ready/configured") == Some(&json!(false)),
        json!({ "config": bootstrap["config"] }),
    );
    let probe = j
        .call(&client, Request::ProbeDefaultThoughts {})
        .await
        .expect("probe_default_thoughts");
    let thoughts_path = text(&probe, "/default_path");
    j.call(
        &client,
        Request::InitThoughtsRepo {
            user: "e2e".into(),
            source: ThoughtsSource::Create {
                path: thoughts_path.clone(),
            },
        },
    )
    .await
    .expect("init_thoughts_repo");
    let thoughts_repo = sandbox.home.join("thoughts");
    j.guard(
        "onboarding creates the thoughts repository",
        thoughts_repo.join(".git").exists(),
        json!({ "probe": probe, "thoughts": sandbox.listing(&thoughts_repo) }),
    );

    j.step("import the repository");
    let project = j
        .call(
            &client,
            Request::ImportProject {
                path: sandbox.as_given(&repo).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import_project");
    let registered = text(&project, "/path");
    j.guard(
        "the project registers under its canonical path",
        Path::new(&registered) == repo,
        json!({ "path": registered, "mapped_name": project["mapped_name"], "source": project["source"] }),
    );
    let cli_config = sandbox.read_json(&sandbox.cli_config());
    let mappings = cli_config["thoughts"]["repoMappings"].clone();
    j.guard(
        "import bootstraps the CLI config and maps the repository",
        mappings.get(&registered).is_some(),
        json!({ "user": cli_config["thoughts"]["user"], "repoMappings": mappings }),
    );
    j.guard(
        "thoughts init links the repository",
        repo.join("thoughts").join("shared").exists(),
        json!({ "thoughts": sandbox.listing(&repo.join("thoughts")) }),
    );

    j.step("create a workstream");
    let view = j
        .call(
            &client,
            Request::CreateWorkstream {
                project_path: registered.clone(),
                title: "Add greeting".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let id = text(&view, "/item/id");
    let slug = text(&view, "/item/slug");
    let branch = text(&view, "/item/branch");
    let worktree = PathBuf::from(text(&view, "/item/worktreePath"));
    j.alias(&id, "$WS");
    if let Some((stem, _)) = slug.rsplit_once('-') {
        j.alias(&slug, format!("{stem}-$SUFFIX"));
    }
    let head = worktree
        .is_dir()
        .then(|| sandbox.git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]));
    j.guard(
        "the worktree exists on its new branch",
        head.as_deref() == Some(branch.as_str()),
        json!({ "worktree": worktree, "branch": branch, "head": head }),
    );
    let tip = sandbox.git(&repo, &["rev-parse", &branch]);
    let main = sandbox.git(&repo, &["rev-parse", "main"]);
    j.guard(
        "the branch starts from main",
        tip == main && text(&view, "/item/baseBranch") == "main",
        json!({ "baseBranch": view["item"]["baseBranch"], "tip": tip, "main": main }),
    );
    let known = sandbox.worktrees(&repo);
    j.guard(
        "git knows the worktree",
        known.iter().any(|w| Path::new(w) == worktree),
        json!({ "worktrees": known }),
    );
    let setup = sandbox.read(&sandbox.home.join("e2e-setup-ran"));
    j.guard(
        "the Setup script ran inside the worktree",
        setup.as_deref().map(str::trim) == Some(&*worktree.to_string_lossy()),
        json!({ "pwd": setup.map(|s| s.trim().to_string()) }),
    );
    let cli_config = sandbox.read_json(&sandbox.cli_config());
    let worktree_key = worktree.to_string_lossy().to_string();
    j.guard(
        "the worktree gets its own thoughts mapping and link",
        cli_config["thoughts"]["repoMappings"]
            .get(&worktree_key)
            .is_some()
            && worktree.join("thoughts").join("shared").exists(),
        json!({
            "mapping": cli_config["thoughts"]["repoMappings"].get(&worktree_key),
            "thoughts": sandbox.listing(&worktree.join("thoughts")),
        }),
    );

    j.step("open a terminal");
    let (terminal, _, mut events) = client
        .terminal_open_stream(id.clone(), 80, 24)
        .await
        .expect("terminal_open");
    j.alias(&terminal, "$TERMINAL");
    j.call(
        &client,
        Request::TerminalWrite {
            terminal_id: terminal.clone(),
            data: keystrokes("printf 'CWD=%s\\n' \"$PWD\"; printf 'E2E_%s\\n' OK\n"),
        },
    )
    .await
    .expect("terminal_write");
    let output = read_until(&mut events, "E2E_OK", IO).await;
    let cwd = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("CWD=/").map(|p| format!("/{p}")));
    j.event("terminal", json!({ "cwd": cwd }));
    j.guard(
        "the terminal opens in the worktree",
        cwd.as_deref().map(canonical) == Some(canonical(&worktree.to_string_lossy())),
        json!({ "cwd": cwd }),
    );
    j.guard(
        "terminal input and output round-trip",
        output.contains("E2E_OK"),
        json!({ "marker": output.contains("E2E_OK") }),
    );

    j.step("edit and stage a file");
    j.call(
        &client,
        Request::WorktreeWriteFile {
            id: id.clone(),
            path: "greeting.txt".into(),
            content: "hello\n".into(),
            base_mtime_ms: 0.0,
        },
    )
    .await
    .expect("worktree_write_file");
    let status = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "a file written through the server shows in git status",
        lists(&status["unstaged"], "greeting.txt"),
        status.clone(),
    );
    j.call(
        &client,
        Request::WorktreeStage {
            id: id.clone(),
            paths: vec!["greeting.txt".into()],
        },
    )
    .await
    .expect("worktree_stage");
    let status = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "staging it sticks",
        lists(&status["staged"], "greeting.txt") && !lists(&status["unstaged"], "greeting.txt"),
        status,
    );

    j.step("archive");
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: id.clone(),
            skip_scripts: false,
        },
    )
    .await
    .expect("archive_workstream");
    let exited = wait_for_exit(&mut events, IO).await;
    j.guard(
        "archive stops the workstream's terminal",
        exited.is_some(),
        json!({ "exited": exited.is_some() }),
    );
    j.guard(
        "archive clears the worktree root",
        !worktree.exists(),
        json!({ "worktree_exists": worktree.exists() }),
    );
    let archived = j
        .call(&client, Request::ListArchivedWorkstreams {})
        .await
        .expect("list_archived_workstreams");
    let entry = find(&archived, "/item/id", &id);
    let archive_path = PathBuf::from(text(&entry, "/archive_path"));
    j.guard(
        "the workstream is listed as archived",
        entry["archive_exists"] == json!(true),
        json!({ "archive_path": archive_path, "archive_exists": entry["archive_exists"] }),
    );
    let kept = sandbox.read(&archive_path.join("greeting.txt"));
    j.guard(
        "archive keeps the uncommitted work",
        kept.as_deref() == Some("hello\n"),
        json!({ "greeting": kept }),
    );
    let archive_ran = sandbox.read(&sandbox.home.join("e2e-archive-ran"));
    j.guard(
        "the Archive script ran inside the worktree",
        archive_ran.as_deref().map(str::trim) == Some(&*worktree.to_string_lossy()),
        json!({ "pwd": archive_ran.map(|s| s.trim().to_string()) }),
    );
    j.guard(
        "the branch survives archive",
        sandbox.branch_exists(&repo, &branch),
        json!({ "branch": branch }),
    );

    j.step("restore");
    let restored = j
        .call(&client, Request::RestoreWorkstream { id: id.clone() })
        .await
        .expect("restore_workstream");
    let restored_path = PathBuf::from(text(&restored, "/item/worktreePath"));
    let head = restored_path
        .is_dir()
        .then(|| sandbox.git(&restored_path, &["rev-parse", "--abbrev-ref", "HEAD"]));
    j.guard(
        "restore brings the worktree back on its branch",
        restored_path == worktree && head.as_deref() == Some(branch.as_str()),
        json!({ "worktree": restored_path, "head": head }),
    );
    let greeting = sandbox.read(&restored_path.join("greeting.txt"));
    j.guard(
        "restore keeps the uncommitted work",
        greeting.as_deref() == Some("hello\n"),
        json!({ "greeting": greeting }),
    );
    let status = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "restore keeps the staged index",
        lists(&status["staged"], "greeting.txt"),
        status,
    );

    j.step("purge");
    let settings = j
        .call(&client, Request::GetAppSettings {})
        .await
        .expect("get_app_settings");
    let delete_branch = settings["git"]["deleteBranchOnArchive"] == json!(true);
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    j.call(
        &client,
        Request::DeleteArchivedWorkstream { id: id.clone() },
    )
    .await
    .expect("delete_archived_workstream");
    j.guard(
        "purge removes the archive and the worktree",
        !archive_path.exists() && !worktree.exists(),
        json!({ "archive_exists": archive_path.exists(), "worktree_exists": worktree.exists() }),
    );
    let all = j
        .call(
            &client,
            Request::ListWorkstreams {
                project_path: registered.clone(),
                include_archived: true,
            },
        )
        .await
        .expect("list_workstreams");
    let archived = j
        .call(&client, Request::ListArchivedWorkstreams {})
        .await
        .expect("list_archived_workstreams");
    j.guard(
        "purge forgets the workstream",
        find(&all, "/item/id", &id).is_null() && find(&archived, "/item/id", &id).is_null(),
        json!({ "listed": all.as_array().map(Vec::len), "archived": archived.as_array().map(Vec::len) }),
    );
    j.guard(
        "the worktree registration is gone from git",
        !sandbox
            .worktrees(&repo)
            .iter()
            .any(|w| Path::new(w) == worktree),
        json!({ "worktrees": sandbox.worktrees(&repo) }),
    );
    let branch_left = sandbox.branch_exists(&repo, &branch);
    j.guard(
        "purge deletes the branch exactly when deleteBranchOnArchive is set",
        branch_left != delete_branch,
        json!({ "deleteBranchOnArchive": delete_branch, "branch_exists": branch_left }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );
    j.guard(
        "the socket stops accepting",
        !stopped.still_listening,
        json!({ "listening": stopped.still_listening }),
    );

    j.finish();
}

fn canonical(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn lists(entries: &Value, path: &str) -> bool {
    entries
        .as_array()
        .is_some_and(|entries| entries.iter().any(|e| e["path"] == json!(path)))
}

fn find(items: &Value, pointer: &str, id: &str) -> Value {
    items
        .as_array()
        .and_then(|items| items.iter().find(|item| text(item, pointer) == id))
        .cloned()
        .unwrap_or(Value::Null)
}
