//! Archived worktrees: what the list reports about each archive, Keep, the
//! retention sweep across a restart, restoring to a new path, and purging a
//! merged archive by id.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::claude::memory::project_key;
use hyprlayer_core::{Harness, SessionEvent, Stage, WorktreeMode};
use hyprlayer_e2e::fake_gh::{self, set_pr, FakePr};
use hyprlayer_e2e::flows::onboard_and_import;
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const DAY_MS: f64 = 86_400_000.0;
const IO: Duration = Duration::from_secs(20);

/// One reply per user turn, in claude's stream-json.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY"}]},"session_id":"%s"}\n' "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY","session_id":"%s","num_turns":1,"total_cost_usd":0,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$sid"
done
"#;

struct Ws {
    id: String,
    slug: String,
    branch: String,
    worktree: PathBuf,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_retention() {
    let sandbox = Sandbox::new();
    let gh_state = sandbox.root.join("gh-state");
    std::fs::create_dir_all(&gh_state).expect("create the gh state dir");
    sandbox.stage_tool("gh", &fake_gh::script(&gh_state));
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    sandbox.git(&repo, &["remote", "add", "github", fake_gh::GITHUB_REMOTE]);
    let mut j = Journey::new("archive_retention", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    j.step("set up four workstreams");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let merged = create(&mut j, &client, &project, "Merged work", "$WS_A").await;
    let dirty = create(&mut j, &client, &project, "Dirty work", "$WS_B").await;
    let kept = create(&mut j, &client, &project, "Kept work", "$WS_C").await;
    let legacy = create(&mut j, &client, &project, "Legacy work", "$WS_D").await;
    set_pr(&gh_state, &merged.branch, &FakePr::new(7, "MERGED"));
    std::fs::write(dirty.worktree.join("committed.txt"), "one\n").expect("write a file");
    sandbox.git(&dirty.worktree, &["add", "committed.txt"]);
    sandbox.git(
        &dirty.worktree,
        &["commit", "--quiet", "--message", "Unmerged work"],
    );
    std::fs::write(dirty.worktree.join("draft.txt"), "wip\n").expect("write a draft");
    let session = run_one_turn(&mut j, &client, &kept.id).await;
    j.guard(
        "the kept workstream runs a session",
        session.is_some(),
        json!({ "session": session.is_some() }),
    );
    let session = session.unwrap_or_default();

    j.step("archive");
    for ws in [&merged, &dirty, &kept, &legacy] {
        j.call(
            &client,
            Request::ArchiveWorkstream {
                id: ws.id.clone(),
                skip_scripts: false,
            },
        )
        .await
        .expect("archive_workstream");
    }
    let listed = list(&mut j, &client).await;
    let [a, b, c, d] = [&merged, &dirty, &kept, &legacy].map(|ws| find(&listed, &ws.id));
    let dirty_archive = PathBuf::from(text(&b, "/archive_path"));
    j.guard(
        "each archive is due to purge 30 days out",
        [&a, &b, &c, &d].iter().all(|e| purge_days(e) == Some(30)),
        json!([&a, &b, &c, &d].map(purge_days)),
    );
    j.guard(
        "each archive's size is measured",
        [&a, &b, &c, &d]
            .iter()
            .all(|e| e["info"]["sizeBytes"].as_f64().is_some_and(|n| n > 0.0)),
        json!([&a, &b, &c, &d].map(|e| e["info"]["sizeBytes"].as_f64().is_some_and(|n| n > 0.0))),
    );
    j.guard(
        "a clean archive has nothing to lose",
        a["info"]["uncommitted"] == json!(0) && a["info"]["unpushed"] == json!(0),
        json!({ "uncommitted": a["info"]["uncommitted"], "unpushed": a["info"]["unpushed"] }),
    );
    j.guard(
        "uncommitted files and unpushed commits are counted",
        b["info"]["uncommitted"] == json!(1) && b["info"]["unpushed"] == json!(1),
        json!({ "uncommitted": b["info"]["uncommitted"], "unpushed": b["info"]["unpushed"] }),
    );
    j.guard(
        "a PR found while archiving is recorded at once; the rest are unknown until refreshed",
        a["info"]["pr"] == json!({ "state": "merged", "number": 7 })
            && [&b, &c, &d]
                .iter()
                .all(|e| e["info"]["pr"] == json!({ "state": "unknown" })),
        json!([&a, &b, &c, &d].map(|e| e["info"]["pr"].clone())),
    );
    j.guard(
        "deleting an archive the app branched deletes its branch",
        a["info"]["deletesBranch"] == json!(true),
        json!({ "deletesBranch": a["info"]["deletesBranch"] }),
    );

    j.step("refresh PR state");
    let changed = j
        .call(&client, Request::RefreshArchivedWorkstreams {})
        .await
        .expect("refresh_archived_workstreams");
    let listed = list(&mut j, &client).await;
    let [a, b, c, d] = [&merged, &dirty, &kept, &legacy].map(|ws| find(&listed, &ws.id));
    j.guard(
        "refresh keeps the merged PR",
        a["info"]["pr"] == json!({ "state": "merged", "number": 7 }),
        json!({ "pr": a["info"]["pr"] }),
    );
    j.guard(
        "refresh records no PR for the rest",
        [&b, &c, &d]
            .iter()
            .all(|e| e["info"]["pr"] == json!({ "state": "none" })),
        json!([&b, &c, &d].map(|e| e["info"]["pr"].clone())),
    );
    j.guard(
        "refresh counts the rows it changed",
        changed == json!(3),
        json!({ "changed": changed }),
    );
    let again = j
        .call(&client, Request::RefreshArchivedWorkstreams {})
        .await
        .expect("refresh_archived_workstreams");
    j.guard(
        "a settled PR state is not asked for again",
        again == json!(0),
        json!({ "changed": again }),
    );

    j.step("keep");
    j.call(
        &client,
        Request::KeepArchivedWorkstream {
            id: kept.id.clone(),
        },
    )
    .await
    .expect("keep_archived_workstream");
    let listed = list(&mut j, &client).await;
    let flags =
        [&merged, &dirty, &kept, &legacy].map(|ws| find(&listed, &ws.id)["info"]["kept"].clone());
    j.guard(
        "Keep pins exactly that archive",
        flags == [json!(false), json!(false), json!(true), json!(false)],
        json!(flags),
    );

    j.step("thirty days pass");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the server stops cleanly",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );
    time_travel(&sandbox, &[dirty.id.as_str(), kept.id.as_str()], &legacy.id);
    let server = Server::start(&sandbox).await;
    let client = server.client();

    j.step("the sweep");
    let swept = poll_archived(&client, |items| find(items, &dirty.id).is_null()).await;
    j.guard(
        "the sweep purges an expired archive",
        find(&swept, &dirty.id).is_null() && !dirty_archive.exists(),
        json!({ "listed": !find(&swept, &dirty.id).is_null(), "archive_exists": dirty_archive.exists() }),
    );
    j.guard(
        "the sweep leaves an unmerged branch",
        sandbox.branch_exists(&repo, &dirty.branch),
        json!({ "branch": dirty.branch }),
    );
    let listed = list(&mut j, &client).await;
    j.guard(
        "a kept archive survives the sweep",
        !find(&listed, &kept.id).is_null(),
        json!({ "listed": !find(&listed, &kept.id).is_null() }),
    );
    let legacy_days = purge_days(&find(&listed, &legacy.id));
    j.guard(
        "a stale record restarts with a 7-day warning",
        legacy_days == Some(7),
        json!({ "purge_days": legacy_days }),
    );
    let merged_days = purge_days(&find(&listed, &merged.id));
    j.guard(
        "an untouched archive keeps its deadline",
        merged_days == Some(30),
        json!({ "purge_days": merged_days }),
    );

    j.step("restore to a new path");
    let original = kept.worktree.clone();
    std::fs::create_dir_all(&original).expect("squat the original path");
    std::fs::write(original.join("squatter.txt"), "mine\n").expect("write the squatter");
    let transcripts = sandbox.home.join(".claude").join("projects");
    let old_transcript = transcript(&transcripts, &original, &session);
    std::fs::create_dir_all(old_transcript.parent().expect("transcript dir")).expect("create it");
    std::fs::write(&old_transcript, "{\"type\":\"user\"}\n").expect("seed a transcript");
    let occupied = j
        .call(
            &client,
            Request::RestoreWorkstream {
                id: kept.id.clone(),
                target: None,
            },
        )
        .await;
    j.guard(
        "restoring onto an occupied path is refused",
        code(&occupied) == "restore_target_occupied",
        json!({ "code": code(&occupied) }),
    );
    let inside = j
        .call(
            &client,
            Request::RestoreWorkstream {
                id: kept.id.clone(),
                target: Some(repo.join("nested").display().to_string()),
            },
        )
        .await;
    j.guard(
        "a target inside the project is refused",
        code(&inside) == "restore_target_invalid",
        json!({ "code": code(&inside) }),
    );
    let elsewhere = sandbox.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("create the destination");
    let restored = j
        .call(
            &client,
            Request::RestoreWorkstream {
                id: kept.id.clone(),
                target: Some(sandbox.as_given(&elsewhere).display().to_string()),
            },
        )
        .await
        .expect("restore_workstream");
    let moved_to = PathBuf::from(text(&restored, "/item/worktreePath"));
    let head = moved_to
        .is_dir()
        .then(|| sandbox.git(&moved_to, &["rev-parse", "--abbrev-ref", "HEAD"]));
    j.guard(
        "an existing folder receives the worktree under its slug",
        moved_to == elsewhere.join(&kept.slug) && head.as_deref() == Some(kept.branch.as_str()),
        json!({ "worktree": moved_to, "head": head }),
    );
    j.guard(
        "the squatter is left alone",
        sandbox.read(&original.join("squatter.txt")).as_deref() == Some("mine\n"),
        json!({ "squatter": sandbox.listing(&original) }),
    );
    let sessions = j
        .call(
            &client,
            Request::ListSessions {
                workstream_id: kept.id.clone(),
            },
        )
        .await
        .expect("list_sessions");
    let cwds: Vec<String> = sessions
        .as_array()
        .map(|all| all.iter().map(|s| text(s, "/cwd")).collect())
        .unwrap_or_default();
    j.guard(
        "the session follows the worktree",
        !cwds.is_empty() && cwds.iter().all(|cwd| Path::new(cwd) == moved_to),
        json!({ "cwds": cwds }),
    );
    let new_transcript = transcript(&transcripts, &moved_to, &session);
    j.guard(
        "claude's transcript is copied to the new project key",
        new_transcript.is_file() && old_transcript.is_file(),
        json!({ "copied": new_transcript.is_file(), "original_kept": old_transcript.is_file() }),
    );

    j.step("purge merged");
    let report = j
        .call(
            &client,
            Request::PurgeArchivedWorkstreams {
                ids: vec![merged.id.clone()],
            },
        )
        .await
        .expect("purge_archived_workstreams");
    j.guard(
        "purging by id deletes exactly the requested archive",
        report == json!({ "deleted": 1, "failed": 0 }),
        report.clone(),
    );
    let listed = list(&mut j, &client).await;
    j.guard(
        "only the requested archive is gone",
        find(&listed, &merged.id).is_null() && !find(&listed, &legacy.id).is_null(),
        json!({ "merged": !find(&listed, &merged.id).is_null(), "legacy": !find(&listed, &legacy.id).is_null() }),
    );
    j.guard(
        "a manual purge deletes the branch the app created",
        !sandbox.branch_exists(&repo, &merged.branch),
        json!({ "branch": merged.branch }),
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

async fn create(
    j: &mut Journey,
    client: &RpcClient,
    project: &str,
    title: &str,
    alias: &str,
) -> Ws {
    let view = j
        .call(
            client,
            Request::CreateWorkstream {
                project_path: project.to_string(),
                title: title.to_string(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let slug = text(&view, "/item/slug");
    j.alias(text(&view, "/item/id"), alias);
    if let Some((stem, _)) = slug.rsplit_once('-') {
        j.alias(&slug, format!("{stem}-$SUFFIX"));
    }
    Ws {
        id: text(&view, "/item/id"),
        branch: text(&view, "/item/branch"),
        worktree: PathBuf::from(text(&view, "/item/worktreePath")),
        slug,
    }
}

/// Launches a claude session in the workstream and waits out its first turn.
async fn run_one_turn(j: &mut Journey, client: &RpcClient, ws: &str) -> Option<String> {
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_PROMPT".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .ok()?;
    j.alias(&session.id, "$SESSION");
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .ok()?;
    let done = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(event, SessionEvent::TurnDone { .. }) {
                return true;
            }
        }
        false
    })
    .await;
    matches!(done, Ok(true)).then_some(session.id)
}

async fn list(j: &mut Journey, client: &RpcClient) -> Value {
    j.call(client, Request::ListArchivedWorkstreams {})
        .await
        .expect("list_archived_workstreams")
}

/// Polls off the record: the sweep runs in the background after start.
async fn poll_archived(client: &RpcClient, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + IO;
    loop {
        let response = client
            .call(Request::ListArchivedWorkstreams {})
            .await
            .expect("list_archived_workstreams");
        let items = serde_json::to_value(&response)
            .ok()
            .and_then(|mut v| v.get_mut("list_archived_workstreams").map(Value::take))
            .unwrap_or(Value::Null);
        if done(&items) || tokio::time::Instant::now() >= deadline {
            return items;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Expires `expire`, and backdates `stale` by 40 days so its record no longer
/// matches, the way an older server re-archiving it would leave it.
fn time_travel(sandbox: &Sandbox, expire: &[&str], stale: &str) {
    let db = rusqlite::Connection::open(sandbox.app_data.join("hyprlayer.db"))
        .expect("open the profile database");
    for id in expire {
        db.execute(
            "UPDATE workstream_archives SET purge_after = 0 WHERE workstream_id = ?1",
            [*id],
        )
        .expect("expire an archive");
    }
    db.execute(
        "UPDATE workstreams SET archived_at = archived_at - ?2 WHERE id = ?1",
        rusqlite::params![stale, 40.0 * DAY_MS],
    )
    .expect("backdate an archive");
}

fn find(items: &Value, id: &str) -> Value {
    items
        .as_array()
        .and_then(|items| items.iter().find(|item| text(item, "/item/id") == id))
        .cloned()
        .unwrap_or(Value::Null)
}

fn purge_days(entry: &Value) -> Option<i64> {
    entry["info"]["purgeInMs"]
        .as_f64()
        .map(|ms| (ms / DAY_MS).ceil() as i64)
}

fn transcript(projects: &Path, cwd: &Path, session: &str) -> PathBuf {
    projects
        .join(project_key(&cwd.to_string_lossy()))
        .join(format!("{session}.jsonl"))
}

fn code(result: &Result<Value, WireError>) -> String {
    result
        .as_ref()
        .err()
        .map(|e| e.code.clone())
        .unwrap_or_default()
}
