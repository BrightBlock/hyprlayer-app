//! A worktree made outside the app (another instance, or plain `git`) opened
//! as a workstream: offered by the branch list, attached without running
//! Setup, and left on disk through archive, restore and purge.
#![cfg(unix)]

use std::path::Path;

use hyprlayer_core::WorktreeMode;
use hyprlayer_e2e::flows::onboard_and_import;
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};

const SETTINGS: &str = r#"[scripts]
setup = 'pwd > "$HOME/e2e-setup-ran"'
"#;

const BRANCH: &str = "feat/elsewhere";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_worktree() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# demo\n"),
            (".hyprlayer/settings.toml", SETTINGS),
        ],
    );
    let mut j = Journey::new("attached_worktree", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("a worktree made outside the app");
    let outside = sandbox.home.join("elsewhere").join("demo-elsewhere");
    sandbox.git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            BRANCH,
            &outside.to_string_lossy(),
        ],
    );
    std::fs::write(outside.join("wip.txt"), "unsaved\n").expect("write uncommitted work");
    let outside_path = outside.display().to_string();

    let bases = j
        .call(
            &client,
            Request::ListWorkstreamBases {
                project_path: project.clone(),
            },
        )
        .await
        .expect("list_workstream_bases");
    let offered = bases["worktrees"]
        .as_array()
        .and_then(|ws| ws.iter().find(|w| text(w, "/branch") == BRANCH))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the branch list offers the worktree with its path",
        text(&offered, "/path") == outside_path,
        json!({ "worktrees": bases["worktrees"] }),
    );
    j.guard(
        "the project's own checkout is not offered as a worktree",
        !bases["worktrees"]
            .as_array()
            .is_some_and(|ws| ws.iter().any(|w| text(w, "/branch") == "main")),
        json!({ "worktrees": bases["worktrees"] }),
    );

    j.step("a branch in no worktree is refused");
    let refused = j
        .call(&client, attach(&project, "Not checked out", "main"))
        .await;
    j.guard(
        "attaching the main checkout's branch fails as worktree_not_found",
        refused.as_ref().err().map(|e| e.code.as_str()) == Some("worktree_not_found"),
        json!({ "code": refused.as_ref().err().map(|e| e.code.clone()) }),
    );

    j.step("attach");
    let view = j
        .call(&client, attach(&project, "Finish elsewhere", BRANCH))
        .await
        .expect("create_workstream");
    let id = text(&view, "/item/id");
    j.alias(&id, "$WS");
    let slug = text(&view, "/item/slug");
    if let Some((stem, _)) = slug.rsplit_once('-') {
        j.alias(&slug, format!("{stem}-$SUFFIX"));
    }
    j.guard(
        "the workstream runs in the existing folder on its branch",
        text(&view, "/item/worktreePath") == outside_path
            && text(&view, "/item/branch") == BRANCH
            && text(&view, "/item/worktreeMode") == "attached",
        json!({
            "worktreePath": view["item"]["worktreePath"],
            "branch": view["item"]["branch"],
            "worktreeMode": view["item"]["worktreeMode"],
        }),
    );
    j.guard(
        "attaching keeps the uncommitted work",
        sandbox.read(&outside.join("wip.txt")).as_deref() == Some("unsaved\n"),
        json!({ "wip": sandbox.read(&outside.join("wip.txt")) }),
    );
    let setup = sandbox.read(&sandbox.home.join("e2e-setup-ran"));
    j.guard(
        "the Setup script does not run in a folder the app did not make",
        setup.is_none(),
        json!({ "pwd": setup.map(|s| s.trim().to_string()) }),
    );

    j.step("attaching it twice is refused");
    let twice = j
        .call(&client, attach(&project, "Second copy", BRANCH))
        .await;
    j.guard(
        "a second workstream on the same worktree fails as worktree_attached",
        twice.as_ref().err().map(|e| e.code.as_str()) == Some("worktree_attached"),
        json!({ "code": twice.as_ref().err().map(|e| e.code.clone()) }),
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
    untouched(&mut j, &sandbox, &repo, &outside, "archive");

    j.step("restore is refused while another workstream holds the folder");
    let second = j
        .call(&client, attach(&project, "Second copy", BRANCH))
        .await
        .expect("create_workstream");
    let second_id = text(&second, "/item/id");
    j.alias(&second_id, "$WS2");
    if let Some((stem, _)) = text(&second, "/item/slug").rsplit_once('-') {
        j.alias(text(&second, "/item/slug"), format!("{stem}-$SUFFIX2"));
    }
    let clash = j.call(&client, restore(&id)).await;
    j.guard(
        "restoring the first fails as worktree_attached",
        clash.as_ref().err().map(|e| e.code.as_str()) == Some("worktree_attached"),
        json!({ "code": clash.as_ref().err().map(|e| e.code.clone()) }),
    );
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: second_id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");

    j.step("restore");
    let restored = j
        .call(&client, restore(&id))
        .await
        .expect("restore_workstream");
    j.guard(
        "restore reopens the same folder",
        text(&restored, "/item/worktreePath") == outside_path
            && text(&restored, "/item/status") == "active",
        json!({
            "worktreePath": restored["item"]["worktreePath"],
            "status": restored["item"]["status"],
        }),
    );

    j.step("purge");
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
    untouched(&mut j, &sandbox, &repo, &outside, "purge");
    j.guard(
        "purge keeps the branch",
        sandbox.branch_exists(&repo, BRANCH),
        json!({ "branch_exists": sandbox.branch_exists(&repo, BRANCH) }),
    );

    j.step("the app's own archived worktrees are not offered");
    let own = j
        .call(
            &client,
            Request::CreateWorkstream {
                project_path: project.clone(),
                title: "Owned by the app".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let own_branch = text(&own, "/item/branch");
    j.alias(text(&own, "/item/id"), "$WS3");
    if let Some((stem, _)) = text(&own, "/item/slug").rsplit_once('-') {
        j.alias(text(&own, "/item/slug"), format!("{stem}-$SUFFIX3"));
    }
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: text(&own, "/item/id"),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let archived_in_git = sandbox
        .git(&repo, &["worktree", "list", "--porcelain"])
        .contains(&format!("branch refs/heads/{own_branch}"));
    let bases = j
        .call(
            &client,
            Request::ListWorkstreamBases {
                project_path: project.clone(),
            },
        )
        .await
        .expect("list_workstream_bases");
    let offered = bases["worktrees"]
        .as_array()
        .is_some_and(|ws| ws.iter().any(|w| text(w, "/branch") == own_branch));
    j.guard(
        "an archive is still a git worktree but is not offered",
        archived_in_git && !offered,
        json!({ "registered_in_git": archived_in_git, "offered": offered }),
    );
    let refused = j
        .call(&client, attach(&project, "Steal the archive", &own_branch))
        .await;
    j.guard(
        "attaching an archive fails as worktree_not_found",
        refused.as_ref().err().map(|e| e.code.as_str()) == Some("worktree_not_found"),
        json!({ "code": refused.as_ref().err().map(|e| e.code.clone()) }),
    );

    j.step("restore after the folder is gone");
    sandbox.git(
        &repo,
        &["worktree", "remove", "--force", &outside.to_string_lossy()],
    );
    let gone = j.call(&client, restore(&second_id)).await;
    j.guard(
        "restoring fails as worktree_missing",
        gone.as_ref().err().map(|e| e.code.as_str()) == Some("worktree_missing"),
        json!({ "code": gone.as_ref().err().map(|e| e.code.clone()) }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );

    j.finish();
}

fn attach(project: &str, title: &str, branch: &str) -> Request {
    Request::CreateWorkstream {
        project_path: project.to_string(),
        title: title.to_string(),
        base: None,
        mode: WorktreeMode::Attached,
        branch: Some(branch.to_string()),
        branch_prefix: None,
    }
}

fn restore(id: &str) -> Request {
    Request::RestoreWorkstream {
        id: id.to_string(),
        target: None,
    }
}

fn untouched(j: &mut Journey, sandbox: &Sandbox, repo: &Path, outside: &Path, after: &str) {
    let wip = sandbox.read(&outside.join("wip.txt"));
    let registered = sandbox
        .worktrees(repo)
        .iter()
        .any(|w| Path::new(w) == outside);
    j.guard(
        &format!("{after} leaves the folder, its work and its git registration"),
        wip.as_deref() == Some("unsaved\n") && registered,
        json!({ "wip": wip, "registered": registered }),
    );
}
