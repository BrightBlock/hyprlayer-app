//! Renaming a workstream can rename its app-made branch while that branch is local and PR-less;
//! once pushed, taken, or switched away from, the rename is refused and the branch is untouched.
#![cfg(unix)]

use std::path::Path;

use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;

async fn preview(client: &RpcClient, id: &str, title: &str) -> Value {
    let found = client
        .call(Request::PreviewBranchRename {
            id: id.to_string(),
            title: title.to_string(),
        })
        .await
        .expect("preview_branch_rename");
    let found = serde_json::to_value(&found).expect("serialize the preview");
    first_with(&found, "branch").unwrap_or(Value::Null)
}

fn first_with(value: &Value, key: &str) -> Option<Value> {
    match value {
        Value::Object(map) if map.contains_key(key) => Some(value.clone()),
        Value::Object(map) => map.values().find_map(|v| first_with(v, key)),
        _ => None,
    }
}

/// `<prefix><stem>-<suffix>` with the workstream's own prefix and suffix.
fn renamed(view: &Value, stem: &str) -> String {
    let (branch, slug) = (text(view, "/item/branch"), text(view, "/item/slug"));
    let prefix = branch
        .strip_suffix(slug.as_str())
        .expect("branch ends in its slug");
    let suffix = slug.rsplit_once('-').expect("slug has a suffix").1;
    format!("{prefix}{stem}-{suffix}")
}

async fn row(client: &RpcClient, id: &str) -> Value {
    let listed = client
        .call(Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let listed = serde_json::to_value(&listed).expect("serialize the list");
    find_item(&listed, id).unwrap_or(Value::Null)
}

fn find_item(value: &Value, id: &str) -> Option<Value> {
    match value {
        Value::Object(map) if map.get("id").and_then(Value::as_str) == Some(id) => {
            Some(value.clone())
        }
        Value::Object(map) => map.values().find_map(|v| find_item(v, id)),
        Value::Array(items) => items.iter().find_map(|v| find_item(v, id)),
        _ => None,
    }
}

fn head(sandbox: &Sandbox, worktree: &str) -> String {
    sandbox
        .git(Path::new(worktree), &["branch", "--show-current"])
        .trim()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workstream_branch_rename() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("workstream_branch_rename", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("a local branch follows the new title");
    let ws = create_workstream(&mut j, &client, &project, "Helvetica now").await;
    let id = text(&ws, "/item/id");
    let worktree = text(&ws, "/item/worktreePath");
    let (old, want) = (text(&ws, "/item/branch"), renamed(&ws, "helvetica-now-v2"));
    j.alias(&want, "$RENAMED");
    let same = preview(&client, &id, "Helvetica  now!").await;
    let offered = preview(&client, &id, "Helvetica Now v2").await;
    j.guard(
        "a title with the same slug offers nothing; a new title offers the renamed branch, unblocked",
        same.is_null() && offered == json!({ "branch": want, "blocked": null }),
        json!({ "same": same, "offered": offered }),
    );
    j.call(
        &client,
        Request::RenameWorkstreamBranch {
            id: id.clone(),
            title: "Helvetica Now v2".into(),
        },
    )
    .await
    .expect("rename_workstream_branch");
    j.call(
        &client,
        Request::RenameWorkstream {
            id: id.clone(),
            title: "Helvetica Now v2".into(),
        },
    )
    .await
    .expect("rename_workstream");
    let after = row(&client, &id).await;
    let evidence = json!({
        "recorded": after.get("branch"),
        "head": head(&sandbox, &worktree),
        "oldGone": !sandbox.branch_exists(&repo, &old),
        "slugKept": after.get("slug") == ws.pointer("/item/slug"),
        "worktreeKept": Path::new(&worktree).is_dir(),
    });
    j.guard(
        "the row and the worktree's HEAD move to the new branch; the old one is gone; slug and folder stay",
        evidence
            == json!({ "recorded": want, "head": want, "oldGone": true, "slugKept": true, "worktreeKept": true }),
        evidence,
    );

    j.step("a renamed branch can be renamed again, unless the name is taken");
    let again = preview(&client, &id, "Taken name").await;
    let taken = renamed(&ws, "taken-name");
    j.alias(&taken, "$TAKEN");
    sandbox.git(&repo, &["branch", &taken]);
    let refused = preview(&client, &id, "Taken name").await;
    j.guard(
        "a second rename is offered; an existing branch with that name blocks it",
        again == json!({ "branch": taken, "blocked": null })
            && refused == json!({ "branch": taken, "blocked": "taken" }),
        json!({ "again": again, "refused": refused }),
    );

    j.step("a pushed branch is refused and left alone");
    sandbox.git(
        &repo,
        &["update-ref", &format!("refs/remotes/origin/{want}"), &want],
    );
    let pushed = preview(&client, &id, "Pushed name").await;
    let err = j
        .call(
            &client,
            Request::RenameWorkstreamBranch {
                id: id.clone(),
                title: "Pushed name".into(),
            },
        )
        .await
        .err()
        .map(|e| e.code);
    let evidence = json!({
        "blocked": pushed.get("blocked"),
        "error": err,
        "head": head(&sandbox, &worktree),
    });
    j.guard(
        "the preview says pushed, the rename errors, and HEAD keeps its branch",
        evidence == json!({ "blocked": "pushed", "error": "branch_rename_blocked", "head": want }),
        evidence,
    );

    j.step("a worktree switched off its branch is refused");
    let other = create_workstream(&mut j, &client, &project, "Switched").await;
    let other_id = text(&other, "/item/id");
    sandbox.git(
        Path::new(&text(&other, "/item/worktreePath")),
        &["switch", "-q", "-c", "elsewhere"],
    );
    let moved = preview(&client, &other_id, "Switched again").await;
    j.guard(
        "HEAD on another branch blocks the rename",
        moved.get("blocked") == Some(&json!("moved")),
        json!({ "blocked": moved.get("blocked") }),
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
