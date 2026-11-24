//! The file and git surface a workstream's Changes pane drives: read, write,
//! stage, unstage, discard, diff both sides, commit, and read a commit back.
//! `workstream_lifecycle` proves a worktree exists; this drives what happens
//! inside one, including the destructive paths nothing else covers.
#![cfg(unix)]

use std::path::Path;

use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use std::time::Duration;

use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worktree_files() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# demo\n"),
            ("src/keep.rs", "fn keep() {}\n"),
            ("src/edit.rs", "fn edit() {}\n"),
        ],
    );
    let mut j = Journey::new("worktree_files", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "File work").await;
    let id = text(&view, "/item/id");
    let worktree = text(&view, "/item/worktreePath");

    j.step("read the tree");
    let listed = j
        .call(
            &client,
            Request::WorktreeListDir {
                id: id.clone(),
                path: "src".into(),
            },
        )
        .await
        .expect("worktree_list_dir");
    j.guard(
        "a directory lists its files",
        names(&listed) == vec!["edit.rs".to_string(), "keep.rs".to_string()],
        json!({ "entries": names(&listed) }),
    );
    let read = j
        .call(
            &client,
            Request::WorktreeReadFile {
                id: id.clone(),
                path: "src/edit.rs".into(),
            },
        )
        .await
        .expect("worktree_read_file");
    j.guard(
        "a file reads back its committed content",
        text(&read, "/content") == "fn edit() {}\n",
        json!({ "content": read.pointer("/content") }),
    );
    let found = j
        .call(
            &client,
            Request::WorktreeSearchFiles {
                id: id.clone(),
                query: "edit".into(),
                limit: 10,
            },
        )
        .await
        .expect("worktree_search_files");
    j.guard(
        "search finds the file by name",
        strings(&found).iter().any(|p| p.ends_with("edit.rs")),
        json!({ "hits": strings(&found) }),
    );

    j.step("edit, stage, and see both sides");
    j.call(
        &client,
        Request::WatchWorktree {
            workstream_id: Some(id.clone()),
        },
    )
    .await
    .expect("watch_worktree");
    let mtime = j
        .call(
            &client,
            Request::WorktreeWriteFile {
                id: id.clone(),
                path: "src/edit.rs".into(),
                content: "fn edit() { changed() }\n".into(),
                base_mtime_ms: 0.0,
            },
        )
        .await
        .expect("worktree_write_file");
    j.guard(
        "writing reports a new mtime",
        mtime.as_f64().unwrap_or(0.0) > 0.0,
        json!({ "wrote": mtime.as_f64().unwrap_or(0.0) > 0.0 }),
    );
    j.call(
        &client,
        Request::WorktreeStage {
            id: id.clone(),
            paths: vec!["src/edit.rs".into()],
        },
    )
    .await
    .expect("worktree_stage");
    let sides = j
        .call(
            &client,
            Request::WorktreeFileSides {
                id: id.clone(),
                path: "src/edit.rs".into(),
                staged: true,
            },
        )
        .await
        .expect("worktree_file_sides");
    j.guard(
        "the staged diff carries both sides",
        text(&sides, "/original") == "fn edit() {}\n"
            && text(&sides, "/modified") == "fn edit() { changed() }\n",
        json!({
            "original_exists": sides.pointer("/originalExists"),
            "modified_exists": sides.pointer("/modifiedExists"),
        }),
    );

    // The watcher is a filesystem watch armed above; the write that just went
    // through the server is what it should see.
    let changed = events_until(
        &mut events_rx,
        Duration::from_secs(20),
        |e| matches!(e, RuntimeEvent::WorktreeChanged { workstream_id } if *workstream_id == id),
    )
    .await;
    j.guard(
        "an armed watcher announces the edit",
        changed
            .iter()
            .any(|e| matches!(e, RuntimeEvent::WorktreeChanged { workstream_id } if *workstream_id == id)),
        json!({ "worktree_changed": changed.iter().filter(|e| matches!(e, RuntimeEvent::WorktreeChanged { .. })).count() }),
    );

    j.step("unstage and discard");
    j.call(
        &client,
        Request::WorktreeUnstage {
            id: id.clone(),
            paths: vec!["src/edit.rs".into()],
        },
    )
    .await
    .expect("worktree_unstage");
    let status = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "unstaging moves the file out of the index",
        staged_paths(&status).is_empty(),
        json!({ "staged": staged_paths(&status) }),
    );
    j.call(
        &client,
        Request::WorktreeDiscard {
            id: id.clone(),
            paths: vec!["src/edit.rs".into()],
        },
    )
    .await
    .expect("worktree_discard");
    let on_disk =
        std::fs::read_to_string(Path::new(&worktree).join("src/edit.rs")).unwrap_or_default();
    j.guard(
        "discard restores the committed content on disk",
        on_disk == "fn edit() {}\n",
        json!({ "restored": on_disk == "fn edit() {}\n" }),
    );
    let after_discard = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "the tree is clean again",
        changed_paths(&after_discard).is_empty(),
        json!({ "changed": changed_paths(&after_discard) }),
    );

    j.step("a staged rename is one row with its old path, and Unstage takes both");
    sandbox.git(Path::new(&worktree), &["mv", "src/keep.rs", "src/kept.rs"]);
    let renamed = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "the rename is one staged row naming its old path",
        rows(&renamed, "/staged") == vec!["renamed:src/kept.rs<-src/keep.rs".to_string()]
            && rows(&renamed, "/unstaged").is_empty(),
        json!({ "staged": rows(&renamed, "/staged"), "unstaged": rows(&renamed, "/unstaged") }),
    );
    j.call(
        &client,
        Request::WorktreeUnstage {
            id: id.clone(),
            paths: vec!["src/kept.rs".into()],
        },
    )
    .await
    .expect("worktree_unstage");
    let unstaged = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    let index_vs_head = sandbox.git(Path::new(&worktree), &["diff", "--cached", "--name-only"]);
    j.guard(
        "unstaging the new path puts the old one back in the index",
        index_vs_head.is_empty()
            && staged_paths(&unstaged).is_empty()
            && rows(&unstaged, "/unstaged")
                == vec![
                    "deleted:src/keep.rs".to_string(),
                    "untracked:src/kept.rs".to_string(),
                ],
        json!({ "unstaged": rows(&unstaged, "/unstaged"), "git_staged": index_vs_head }),
    );
    j.call(
        &client,
        Request::WorktreeDiscard {
            id: id.clone(),
            paths: vec!["src/keep.rs".into(), "src/kept.rs".into()],
        },
    )
    .await
    .expect("worktree_discard");
    let restored = j
        .call(&client, Request::WorktreeGitStatus { id: id.clone() })
        .await
        .expect("worktree_git_status");
    j.guard(
        "discarding both paths restores the tree",
        changed_paths(&restored).is_empty(),
        json!({ "changed": changed_paths(&restored) }),
    );

    j.step("commit and read the commit back");
    j.call(
        &client,
        Request::WorktreeWriteFile {
            id: id.clone(),
            path: "src/keep.rs".into(),
            content: "fn keep() { kept() }\n".into(),
            base_mtime_ms: 0.0,
        },
    )
    .await
    .expect("worktree_write_file");
    j.call(
        &client,
        Request::WorktreeStage {
            id: id.clone(),
            paths: vec!["src/keep.rs".into()],
        },
    )
    .await
    .expect("worktree_stage");
    // No commit RPC: the Commit stage's agent runs git itself (ADR-0007), so the
    // scenario commits the same way and exercises what the app reads back.
    sandbox.git(
        Path::new(&worktree),
        &["commit", "--quiet", "--message", "Keep it"],
    );
    let commits = j
        .call(&client, Request::ListWorkstreamCommits { id: id.clone() })
        .await
        .expect("list_workstream_commits");
    let sha = commits
        .as_array()
        .and_then(|c| c.first())
        .map(|c| text(c, "/sha"))
        .unwrap_or_default();
    j.alias(&sha, "$SHA");
    if sha.len() >= 7 {
        j.alias(sha[..7].to_string(), "$SHORT_SHA");
    }
    j.guard(
        "the new commit tops the workstream's list",
        commits
            .as_array()
            .and_then(|c| c.first())
            .map(|c| text(c, "/subject"))
            .unwrap_or_default()
            == "Keep it",
        json!({ "subject": commits.as_array().and_then(|c| c.first()).map(|c| text(c, "/subject")) }),
    );
    let files = j
        .call(
            &client,
            Request::WorktreeCommitFiles {
                id: id.clone(),
                sha: sha.clone(),
            },
        )
        .await
        .expect("worktree_commit_files");
    j.guard(
        "the commit names the file it changed",
        strings_at(&files, "/path")
            .iter()
            .any(|p| p.ends_with("keep.rs")),
        json!({ "files": strings_at(&files, "/path") }),
    );
    let commit_sides = j
        .call(
            &client,
            Request::WorktreeCommitFileSides {
                id,
                sha,
                path: "src/keep.rs".into(),
            },
        )
        .await
        .expect("worktree_commit_file_sides");
    j.guard(
        "a commit's file diffs against its parent",
        text(&commit_sides, "/original") == "fn keep() {}\n"
            && text(&commit_sides, "/modified") == "fn keep() { kept() }\n",
        json!({ "both_sides": text(&commit_sides, "/original") != text(&commit_sides, "/modified") }),
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

fn names(entries: &Value) -> Vec<String> {
    let mut out: Vec<String> = entries
        .as_array()
        .map(|items| items.iter().map(|e| text(e, "/name")).collect())
        .unwrap_or_default();
    out.sort();
    out
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

fn strings_at(value: &Value, pointer: &str) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(|i| text(i, pointer)).collect())
        .unwrap_or_default()
}

fn rows(status: &Value, side: &str) -> Vec<String> {
    status
        .pointer(side)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|e| match e.pointer("/oldPath").and_then(Value::as_str) {
                    Some(old) => format!("{}:{}<-{old}", text(e, "/change"), text(e, "/path")),
                    None => format!("{}:{}", text(e, "/change"), text(e, "/path")),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn staged_paths(status: &Value) -> Vec<String> {
    strings_at(status.pointer("/staged").unwrap_or(&Value::Null), "/path")
}

fn changed_paths(status: &Value) -> Vec<String> {
    let mut all = staged_paths(status);
    all.extend(strings_at(
        status.pointer("/unstaged").unwrap_or(&Value::Null),
        "/path",
    ));
    all
}
