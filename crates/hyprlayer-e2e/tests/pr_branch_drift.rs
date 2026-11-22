//! An isolated worktree whose branch was renamed or switched still finds its PR. The board's stage
//! and observed PR follow the workstream from its first session through merge and onto a new branch.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hyprlayer_core::pr::MergeMethod;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::fake_gh::{self, set_pr, FakePr};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
/// Longer than the watcher's 20s poll of a focused open PR.
const WATCHER: Duration = Duration::from_secs(30);
const IN_FLIGHT_POLL: Duration = Duration::from_secs(1);
const NEVER_FETCHED: &str = "1111111111111111111111111111111111111111";

const FAKE_CLAUDE: &str = r##"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    --resume) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  printf '{"type":"result","subtype":"success","is_error":false,"result":"ok","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}\n' "$sid" "$turn"
done
"##;

fn git(dir: &str, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

async fn run_session(j: &mut Journey, client: &RpcClient, workstream: &str, stage: Stage) {
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            workstream.to_string(),
            stage,
            String::new(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(event, SessionEvent::TurnDone { .. }) {
                return;
            }
        }
    })
    .await;
}

async fn stage_of(client: &RpcClient, id: &str) -> Value {
    let listed = client
        .call(Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let listed = serde_json::to_value(&listed).expect("serialize the list");
    find_in(&listed, id)
        .and_then(|v| v.pointer("/item/stage").cloned())
        .unwrap_or(Value::Null)
}

fn find_in<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    match value {
        Value::Array(items) => items
            .iter()
            .find(|v| v.pointer("/item/id").and_then(Value::as_str) == Some(id)),
        Value::Object(map) => map.values().find_map(|v| find_in(v, id)),
        _ => None,
    }
}

async fn observed_pr(client: &RpcClient, id: &str) -> Value {
    let listed = client
        .call(Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let listed = serde_json::to_value(&listed).expect("serialize the list");
    find_in(&listed, id)
        .and_then(|v| v.pointer("/item/observedPr").cloned())
        .unwrap_or(Value::Null)
}

/// Changing the focused workstream makes it due on the watcher's next tick, and the foreground
/// call wakes that tick; a plain wait would sit out a 60s cadence.
async fn poke_watcher(client: &RpcClient, id: &str, round: u32) {
    let _ = client
        .call(Request::WatchWorktree {
            workstream_id: round.is_multiple_of(2).then(|| id.to_string()),
        })
        .await;
    let _ = client
        .call(Request::SetForeground { foreground: true })
        .await;
}

async fn await_observed(client: &RpcClient, id: &str, want: Value) -> Value {
    let seen = observed_within(client, id, &want).await;
    assert_eq!(seen, want, "the watcher never recorded this PR state");
    seen
}

async fn observed_within(client: &RpcClient, id: &str, want: &Value) -> Value {
    let deadline = tokio::time::Instant::now() + WATCHER;
    let mut round = 0;
    loop {
        let seen = observed_pr(client, id).await;
        if seen == *want || tokio::time::Instant::now() >= deadline {
            return seen;
        }
        poke_watcher(client, id, round).await;
        round += 1;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn await_stage(client: &RpcClient, id: &str, want: Value) -> Value {
    let deadline = tokio::time::Instant::now() + WATCHER;
    let mut round = 0;
    loop {
        let seen = stage_of(client, id).await;
        if seen == want || tokio::time::Instant::now() >= deadline {
            assert_eq!(seen, want, "the watcher never moved this stage");
            return seen;
        }
        poke_watcher(client, id, round).await;
        round += 1;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Waits out several polled watcher ticks, for checks that nothing changes.
async fn let_watcher_tick(client: &RpcClient, id: &str) {
    for round in 0..12 {
        poke_watcher(client, id, round).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Whether the next status event for `id` says the row's observed PR changed.
async fn next_status_event(rx: &mut tokio::sync::mpsc::Receiver<RuntimeEvent>, id: &str) -> Value {
    let is_status = |e: &RuntimeEvent| matches!(e, RuntimeEvent::PrStatusChanged { workstream_id, .. } if workstream_id == id);
    events_until(rx, WATCHER, is_status)
        .await
        .iter()
        .find_map(|e| match e {
            RuntimeEvent::PrStatusChanged {
                workstream_id,
                observed_pr_changed,
            } if workstream_id == id => Some(json!(observed_pr_changed)),
            _ => None,
        })
        .unwrap_or(Value::Null)
}

async fn set_stage(j: &mut Journey, client: &RpcClient, id: &str, stage: Stage) -> Value {
    j.call(
        client,
        Request::SetWorkstreamStage {
            id: id.to_string(),
            stage,
        },
    )
    .await
    .expect("set_workstream_stage");
    stage_of(client, id).await
}

async fn lookup(client: &RpcClient, id: &str) -> Value {
    let status = client
        .call(Request::WorkstreamPrStatus { id: id.to_string() })
        .await
        .expect("workstream_pr_status");
    serde_json::to_value(&status).expect("serialize the PR status")["workstream_pr_status"].take()
}

async fn pr_number(client: &RpcClient, id: &str) -> Value {
    first_number(&lookup(client, id).await)
}

async fn await_unwatched(client: &RpcClient, id: &str, want: Value) -> Value {
    let deadline = tokio::time::Instant::now() + WATCHER;
    loop {
        let seen = observed_pr(client, id).await;
        if seen == want || tokio::time::Instant::now() >= deadline {
            assert_eq!(seen, want, "the watcher never recorded this PR state");
            return seen;
        }
        let _ = client
            .call(Request::SetForeground { foreground: true })
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn observed_unpoked(client: &RpcClient, id: &str, want: &Value, within: Duration) -> Value {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let seen = observed_pr(client, id).await;
        if seen == *want || tokio::time::Instant::now() >= deadline {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn archived_pr(client: &RpcClient, id: &str) -> Value {
    let listed = client
        .call(Request::ListArchivedWorkstreams {})
        .await
        .expect("list_archived_workstreams");
    let listed = serde_json::to_value(&listed).expect("serialize the archive list");
    find_in(&listed, id)
        .and_then(|v| v.pointer("/info/pr").cloned())
        .unwrap_or(Value::Null)
}

fn gh_lines(dir: &Path, prefix: &str) -> usize {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

fn gh_node(dir: &Path, reference: &str, field: &str) -> Value {
    std::fs::read_to_string(dir.join(format!("{}.json", reference.replace('/', "_"))))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map_or(Value::Null, |node| node[field].clone())
}

fn remote_has(sandbox: &Sandbox, repo: &Path, branch: &str) -> bool {
    !sandbox
        .git(
            repo,
            &["ls-remote", "origin", &format!("refs/heads/{branch}")],
        )
        .trim()
        .is_empty()
}

fn first_number(value: &Value) -> Value {
    match value {
        Value::Object(map) => map
            .get("number")
            .cloned()
            .or_else(|| map.values().map(first_number).find(|v| !v.is_null()))
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pr_branch_drift() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let gh_state = sandbox.root.join("gh-state");
    std::fs::create_dir_all(&gh_state).expect("create the gh state dir");
    sandbox.stage_tool("gh", &fake_gh::script(&gh_state));
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    sandbox.git(&repo, &["remote", "add", "github", fake_gh::GITHUB_REMOTE]);
    let mut j = Journey::new("pr_branch_drift", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("stage sessions move the workstream forward, never back");
    let ws = create_workstream(&mut j, &client, &project, "Helvetica now").await;
    let id = text(&ws, "/item/id");
    let worktree = text(&ws, "/item/worktreePath");
    let branch = text(&ws, "/item/branch");
    run_session(&mut j, &client, &id, Stage::Plan).await;
    let planned = stage_of(&client, &id).await;
    run_session(&mut j, &client, &id, Stage::Ship).await;
    let shipped = stage_of(&client, &id).await;
    run_session(&mut j, &client, &id, Stage::Plan).await;
    let replanned = stage_of(&client, &id).await;
    run_session(&mut j, &client, &id, Stage::General).await;
    let general = stage_of(&client, &id).await;
    let stages =
        json!({ "plan": planned, "ship": shipped, "planAgain": replanned, "general": general });
    j.guard(
        "plan → ship advances; a later plan or general session leaves it at ship",
        stages == json!({ "plan": "plan", "ship": "ship", "planAgain": "ship", "general": "ship" }),
        stages,
    );

    j.step("a renamed branch still finds its PR");
    set_pr(&gh_state, "feature/csp-1", &FakePr::new(339, "OPEN"));
    let before = pr_number(&client, &id).await;
    let renamed = git(&worktree, &["branch", "-m", "feature/csp-1"]);
    let after = pr_number(&client, &id).await;
    let recorded = serde_json::to_value(
        client
            .call(Request::ListAllWorkstreams {})
            .await
            .expect("list_all_workstreams"),
    )
    .ok()
    .and_then(|v| find_in(&v, &id).and_then(|w| w.pointer("/item/branch").cloned()))
    .unwrap_or(Value::Null);
    let evidence = json!({
        "beforeRename": before,
        "renamed": renamed,
        "afterRename": after,
        "keepsAppBranch": recorded == json!(branch),
    });
    j.guard(
        "no PR on the app's branch; after `git branch -m` the PR on the new name is found, \
         and the row keeps the app's branch",
        evidence
            == json!({ "beforeRename": null, "renamed": true, "afterRename": 339, "keepsAppBranch": true }),
        evidence,
    );

    j.step("a switched branch follows HEAD");
    set_pr(&gh_state, "feature/csp-2", &FakePr::new(340, "OPEN"));
    let cached = pr_number(&client, &id).await;
    let switched = git(&worktree, &["switch", "-q", "-c", "feature/csp-2"]);
    let followed = pr_number(&client, &id).await;
    j.guard(
        "after `git switch -c` the new branch's PR is found, not the old branch's cached one",
        cached == json!(339) && switched && followed == json!(340),
        json!({ "cached": cached, "switched": switched, "pr": followed }),
    );
    let listed = serde_json::to_value(
        client
            .call(Request::ListAllWorkstreams {})
            .await
            .expect("list_all_workstreams"),
    )
    .expect("serialize the list");
    let row = find_in(&listed, &id);
    let evidence = json!({
        "rowKeepsAppBranch": row.and_then(|v| v.pointer("/item/branch")) == Some(&json!(branch)),
        "head": row.and_then(|v| v.pointer("/git/branch").cloned()),
    });
    j.guard(
        "the row keeps the app's branch and the git status names the branch HEAD is on",
        evidence == json!({ "rowKeepsAppBranch": true, "head": "feature/csp-2" }),
        evidence,
    );

    j.step("an attached PR wins over HEAD");
    set_pr(&gh_state, "339", &FakePr::new(339, "OPEN"));
    j.call(
        &client,
        Request::AttachWorkstreamPr {
            id: id.clone(),
            pr_number: 339,
        },
    )
    .await
    .expect("attach_workstream_pr");
    let attached = pr_number(&client, &id).await;
    j.guard(
        "with #339 attached the lookup ignores HEAD's #340",
        attached == json!(339),
        json!({ "pr": attached }),
    );
    j.call(&client, Request::DetachWorkstreamPr { id: id.clone() })
        .await
        .expect("detach_workstream_pr");

    j.step("the watcher records the current branch's PR on the row");
    let open = await_observed(&client, &id, json!({ "number": 340, "state": "open" })).await;
    while events_rx.try_recv().is_ok() {}
    set_pr(
        &gh_state,
        "feature/csp-2",
        &FakePr {
            merge_state: "CLEAN",
            ..FakePr::new(340, "OPEN")
        },
    );
    j.call(
        &client,
        Request::WatchWorktree {
            workstream_id: Some(id.clone()),
        },
    )
    .await
    .expect("watch_worktree");
    let _ = client
        .call(Request::SetForeground { foreground: true })
        .await;
    let churn_changes_row = next_status_event(&mut events_rx, &id).await;
    set_pr(&gh_state, "feature/csp-2", &FakePr::new(340, "MERGED"));
    let merged = await_observed(&client, &id, json!({ "number": 340, "state": "merged" })).await;
    let merge_changes_row = next_status_event(&mut events_rx, &id).await;
    let attached_row = client
        .call(Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let attached_row = serde_json::to_value(&attached_row).expect("serialize the list");
    let pr_column = find_in(&attached_row, &id)
        .and_then(|v| v.pointer("/item/prNumber").cloned())
        .unwrap_or(Value::Null);
    let evidence = json!({
        "open": open,
        "merged": merged,
        "attached": pr_column,
        "churnChangesRow": churn_changes_row,
        "mergeChangesRow": merge_changes_row,
    });
    j.guard(
        "a PR nobody attached is observed open, then merged, and pr_number stays empty; \
         only the merge tells clients their workstream lists are stale",
        evidence
            == json!({
                "open": { "number": 340, "state": "open" },
                "merged": { "number": 340, "state": "merged" },
                "attached": null,
                "churnChangesRow": false,
                "mergeChangesRow": true,
            }),
        evidence,
    );

    j.step("a merged worktree keeps following its branch");
    set_stage(&mut j, &client, &id, Stage::Ship).await;
    set_pr(&gh_state, "hotfix/csp-3", &FakePr::new(341, "MERGED"));
    git(&worktree, &["switch", "-q", "-c", "hotfix/csp-3"]);
    let hotfix = await_observed(&client, &id, json!({ "number": 341, "state": "merged" })).await;
    let hotfix_stage = stage_of(&client, &id).await;
    git(&worktree, &["switch", "-q", "-c", "feature/csp-4"]);
    let fresh = await_observed(&client, &id, Value::Null).await;
    let fresh_stage = await_stage(&client, &id, json!("general")).await;
    let evidence = json!({
        "hotfix": hotfix,
        "hotfixStage": hotfix_stage,
        "fresh": fresh,
        "freshStage": fresh_stage,
    });
    j.guard(
        "another merged PR keeps the stage; a branch with no PR clears it and restarts at general",
        evidence
            == json!({
                "hotfix": { "number": 341, "state": "merged" },
                "hotfixStage": "ship",
                "fresh": null,
                "freshStage": "general",
            }),
        evidence,
    );

    j.step("a tab's mode only moves the stage forward");
    let forward = set_stage(&mut j, &client, &id, Stage::Implement).await;
    let back = set_stage(&mut j, &client, &id, Stage::Plan).await;
    let general = set_stage(&mut j, &client, &id, Stage::General).await;
    let evidence = json!({ "implement": forward, "plan": back, "general": general });
    j.guard(
        "implement advances; plan and general leave it at implement",
        evidence
            == json!({ "implement": "implement", "plan": "implement", "general": "implement" }),
        evidence,
    );

    j.step("moving to another branch starts the card over, whatever came before");
    set_stage(&mut j, &client, &id, Stage::Ship).await;
    git(&worktree, &["switch", "-q", "-c", "feature/csp-5"]);
    let no_pr_before = await_stage(&client, &id, json!("general")).await;
    set_pr(&gh_state, "feature/csp-5", &FakePr::new(342, "OPEN"));
    await_observed(&client, &id, json!({ "number": 342, "state": "open" })).await;
    set_stage(&mut j, &client, &id, Stage::Ship).await;
    git(&worktree, &["switch", "-q", "-c", "feature/csp-6"]);
    let open_before = await_stage(&client, &id, json!("general")).await;
    let open_cleared = observed_pr(&client, &id).await;
    let evidence = json!({
        "noPrBefore": no_pr_before,
        "openBefore": open_before,
        "openCleared": open_cleared,
    });
    j.guard(
        "ship with no PR, and ship with an open PR, both restart at general on a new branch",
        evidence
            == json!({ "noPrBefore": "general", "openBefore": "general", "openCleared": null }),
        evidence,
    );

    j.step("leaving the branch for a detached HEAD holds the card");
    set_pr(&gh_state, "feature/csp-6", &FakePr::new(343, "MERGED"));
    await_observed(&client, &id, json!({ "number": 343, "state": "merged" })).await;
    set_stage(&mut j, &client, &id, Stage::Ship).await;
    git(&worktree, &["switch", "-q", "--detach"]);
    let_watcher_tick(&client, &id).await;
    let during =
        json!({ "pr": observed_pr(&client, &id).await, "stage": stage_of(&client, &id).await });
    git(&worktree, &["switch", "-q", "feature/csp-6"]);
    let_watcher_tick(&client, &id).await;
    let after =
        json!({ "pr": observed_pr(&client, &id).await, "stage": stage_of(&client, &id).await });
    let held = json!({ "pr": { "number": 343, "state": "merged" }, "stage": "ship" });
    j.guard(
        "a rebase-style detach shows the same PR and stage during it and after it",
        during == held && after == held,
        json!({ "during": during, "after": after }),
    );

    j.step("a stage picked on a new branch survives the watcher noticing the move");
    git(&worktree, &["switch", "-q", "-c", "feature/csp-7"]);
    let picked = set_stage(&mut j, &client, &id, Stage::Plan).await;
    let_watcher_tick(&client, &id).await;
    let evidence = json!({ "picked": picked, "afterTicks": stage_of(&client, &id).await });
    j.guard(
        "plan is lower than ship but sticks, because it is the new branch's first stage",
        evidence == json!({ "picked": "plan", "afterTicks": "plan" }),
        evidence,
    );

    j.step("a failed lookup after a move shows no PR, and recovers");
    set_pr(&gh_state, "feature/csp-7", &FakePr::new(344, "OPEN"));
    await_observed(&client, &id, json!({ "number": 344, "state": "open" })).await;
    std::fs::write(gh_state.join("FAIL"), "").expect("make gh fail");
    while events_rx.try_recv().is_ok() {}
    git(&worktree, &["switch", "-q", "-c", "feature/csp-8"]);
    let hidden = observed_pr(&client, &id).await;
    let_watcher_tick(&client, &id).await;
    let told_clients = next_status_event(&mut events_rx, &id).await;
    let failing =
        json!({ "pr": observed_pr(&client, &id).await, "stage": stage_of(&client, &id).await });
    std::fs::remove_file(gh_state.join("FAIL")).expect("let gh succeed");
    let recovered = await_stage(&client, &id, json!("general")).await;
    let evidence = json!({
        "hidden": hidden,
        "toldClients": told_clients,
        "failing": failing,
        "recovered": recovered,
    });
    j.guard(
        "the old branch's PR disappears at once, the stage waits for a real answer, then restarts",
        evidence
            == json!({
                "hidden": null,
                "toldClients": true,
                "failing": { "pr": null, "stage": "plan" },
                "recovered": "general",
            }),
        evidence,
    );

    j.step("an in-app branch rename is the same work");
    let renamed = create_workstream(&mut j, &client, &project, "Rename me").await;
    let renamed_id = text(&renamed, "/item/id");
    let_watcher_tick(&client, &renamed_id).await;
    set_stage(&mut j, &client, &renamed_id, Stage::Implement).await;
    let_watcher_tick(&client, &renamed_id).await;
    j.call(
        &client,
        Request::RenameWorkstreamBranch {
            id: renamed_id.clone(),
            title: "Renamed for real".into(),
        },
    )
    .await
    .expect("rename_workstream_branch");
    let_watcher_tick(&client, &renamed_id).await;
    let kept = stage_of(&client, &renamed_id).await;
    j.guard(
        "the stage stays through the rename and the watcher's next ticks",
        kept == json!("implement"),
        json!({ "stage": kept }),
    );

    j.step("a merged PR reads through a missing worktree");
    let husk = create_workstream(&mut j, &client, &project, "Husk").await;
    let husk_id = text(&husk, "/item/id");
    set_pr(&gh_state, "360", &FakePr::new(360, "OPEN"));
    j.call(
        &client,
        Request::AttachWorkstreamPr {
            id: husk_id.clone(),
            pr_number: 360,
        },
    )
    .await
    .expect("attach_workstream_pr");
    let on_attach = observed_pr(&client, &husk_id).await;
    std::fs::remove_dir_all(text(&husk, "/item/worktreePath")).expect("remove the worktree");
    set_pr(&gh_state, "360", &FakePr::new(360, "MERGED"));
    j.call(
        &client,
        Request::WatchWorktree {
            workstream_id: Some(husk_id.clone()),
        },
    )
    .await
    .expect("watch_worktree");
    let gone = await_observed(
        &client,
        &husk_id,
        json!({ "number": 360, "state": "merged" }),
    )
    .await;
    let evidence = json!({ "onAttach": on_attach, "worktreeGone": gone });
    j.guard(
        "attaching records the PR at once, and the watcher still sees it merge with no worktree",
        evidence
            == json!({
                "onAttach": { "number": 360, "state": "open" },
                "worktreeGone": { "number": 360, "state": "merged" },
            }),
        evidence,
    );

    j.step("archiving records the PR it just looked up");
    let quick = create_workstream(&mut j, &client, &project, "Quick merge").await;
    let quick_id = text(&quick, "/item/id");
    set_pr(
        &gh_state,
        &text(&quick, "/item/branch"),
        &FakePr::new(350, "MERGED"),
    );
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: quick_id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let listed = client
        .call(Request::ListArchivedWorkstreams {})
        .await
        .expect("list_archived_workstreams");
    let listed = serde_json::to_value(&listed).expect("serialize the archive list");
    let pr = find_in(&listed, &quick_id)
        .and_then(|v| v.pointer("/info/pr").cloned())
        .unwrap_or(Value::Null);
    j.guard(
        "the archive list has #350 merged without a sweep",
        pr.to_string().contains("350") && pr.to_string().to_lowercase().contains("merged"),
        pr,
    );

    j.step("restoring keeps what the archive learned");
    j.call(
        &client,
        Request::RestoreWorkstream {
            id: quick_id.clone(),
            target: None,
        },
    )
    .await
    .expect("restore_workstream");
    let restored = observed_pr(&client, &quick_id).await;
    j.guard(
        "the restored row has #350 merged without waiting for a poll",
        restored == json!({ "number": 350, "state": "merged" }),
        json!({ "pr": restored }),
    );

    j.step("an archive looks up the branch it was archived on");
    git(&worktree, &["switch", "-q", "feature/csp-2"]);
    std::fs::remove_file(gh_state.join("feature_csp-2.json"))
        .expect("hide the PR from the archive");
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    set_pr(&gh_state, "feature/csp-2", &FakePr::new(340, "MERGED"));
    j.call(&client, Request::RefreshArchivedWorkstreams {})
        .await
        .expect("refresh_archived_workstreams");
    let archived = client
        .call(Request::ListArchivedWorkstreams {})
        .await
        .expect("list_archived_workstreams");
    let archived = serde_json::to_value(&archived).expect("serialize the archive list");
    let pr = find_in(&archived, &id)
        .and_then(|v| v.pointer("/info/pr").cloned())
        .unwrap_or(Value::Null);
    j.guard(
        "the archive sweep reads HEAD from the archived worktree and records #340 merged",
        pr.to_string().contains("340") && pr.to_string().to_lowercase().contains("merged"),
        pr,
    );

    j.step("a new PR on a merged branch reaches the row and an event");
    let reuse = create_workstream(&mut j, &client, &project, "Reuse").await;
    let reuse_id = text(&reuse, "/item/id");
    let reuse_tree = PathBuf::from(text(&reuse, "/item/worktreePath"));
    let reuse_branch = text(&reuse, "/item/branch");
    set_pr(&gh_state, &reuse_branch, &FakePr::new(500, "OPEN"));
    let open = await_observed(
        &client,
        &reuse_id,
        json!({ "number": 500, "state": "open" }),
    )
    .await;
    let tip = sandbox.git(&reuse_tree, &["rev-parse", "HEAD"]);
    set_pr(
        &gh_state,
        &reuse_branch,
        &FakePr {
            head_oid: Some(tip),
            ..FakePr::new(500, "MERGED")
        },
    );
    let merged = await_observed(
        &client,
        &reuse_id,
        json!({ "number": 500, "state": "merged" }),
    )
    .await;
    let before = fake_gh::calls(&gh_state, &reuse_branch);
    let_watcher_tick(&client, &reuse_id).await;
    let settled_polls = fake_gh::calls(&gh_state, &reuse_branch) - before;
    sandbox.git(
        &reuse_tree,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "more work",
        ],
    );
    let after_commit = lookup(&client, &reuse_id).await["kind"].clone();
    let cleared = await_observed(&client, &reuse_id, Value::Null).await;
    while events_rx.try_recv().is_ok() {}
    set_pr(&gh_state, &reuse_branch, &FakePr::new(501, "OPEN"));
    let next = await_observed(
        &client,
        &reuse_id,
        json!({ "number": 501, "state": "open" }),
    )
    .await;
    let mut told_clients = false;
    while let Ok(event) = events_rx.try_recv() {
        told_clients |= matches!(
            event,
            RuntimeEvent::PrStatusChanged { workstream_id, observed_pr_changed: true }
                if workstream_id == reuse_id
        );
    }
    let evidence = json!({
        "open": open,
        "merged": merged,
        "settledPolls": settled_polls,
        "afterCommit": after_commit,
        "cleared": cleared,
        "next": next,
        "toldClients": told_clients,
    });
    j.guard(
        "a merged PR is not polled; a commit past its head offers Ship, and the next PR on the \
         branch reaches the row and the clients",
        evidence
            == json!({
                "open": { "number": 500, "state": "open" },
                "merged": { "number": 500, "state": "merged" },
                "settledPolls": 0,
                "afterCommit": "noPr",
                "cleared": null,
                "next": { "number": 501, "state": "open" },
                "toldClients": true,
            }),
        evidence,
    );

    j.step("a merged PR whose head is not in the branch is not this branch's");
    let stray = create_workstream(&mut j, &client, &project, "Stray").await;
    let stray_id = text(&stray, "/item/id");
    let stray_tree = PathBuf::from(text(&stray, "/item/worktreePath"));
    let tree = sandbox.git(&stray_tree, &["rev-parse", "HEAD^{tree}"]);
    let elsewhere = sandbox.git(&stray_tree, &["commit-tree", &tree, "-m", "elsewhere"]);
    sandbox.git(&stray_tree, &["branch", "throwaway", &elsewhere]);
    set_pr(
        &gh_state,
        &text(&stray, "/item/branch"),
        &FakePr {
            head_oid: Some(elsewhere),
            ..FakePr::new(510, "MERGED")
        },
    );
    let stray_lookup = lookup(&client, &stray_id).await["kind"].clone();
    let_watcher_tick(&client, &stray_id).await;
    let evidence = json!({ "lookup": stray_lookup, "row": observed_pr(&client, &stray_id).await });
    j.guard(
        "an old merged PR reusing the branch name, whose head this branch never held, reads as no PR",
        evidence == json!({ "lookup": "noPr", "row": null }),
        evidence,
    );

    j.step("a draft reads as a draft with its base");
    let draft = create_workstream(&mut j, &client, &project, "Draft").await;
    set_pr(
        &gh_state,
        &text(&draft, "/item/branch"),
        &FakePr {
            is_draft: true,
            base: "release",
            ..FakePr::new(520, "OPEN")
        },
    );
    let found = lookup(&client, &text(&draft, "/item/id")).await;
    let evidence = json!({
        "kind": found["kind"],
        "isDraft": found.pointer("/pr/isDraft"),
        "baseRefName": found.pointer("/pr/baseRefName"),
    });
    j.guard(
        "the lookup carries the draft flag and the PR's own base",
        evidence == json!({ "kind": "found", "isDraft": true, "baseRefName": "release" }),
        evidence,
    );

    j.step("Fetch refreshes PR status");
    let _ = client
        .call(Request::WatchWorktree {
            workstream_id: None,
        })
        .await;
    let fetched = create_workstream(&mut j, &client, &project, "Fetch me").await;
    let fetched_id = text(&fetched, "/item/id");
    let fetched_branch = text(&fetched, "/item/branch");
    set_pr(&gh_state, &fetched_branch, &FakePr::new(530, "OPEN"));
    let seen = await_unwatched(
        &client,
        &fetched_id,
        json!({ "number": 530, "state": "open" }),
    )
    .await;
    while events_rx.try_recv().is_ok() {}
    set_pr(
        &gh_state,
        &fetched_branch,
        &FakePr {
            merge_state: "CLEAN",
            ..FakePr::new(530, "OPEN")
        },
    );
    let _ = client
        .call(Request::FetchWorkstreamRemote {
            id: fetched_id.clone(),
        })
        .await;
    let is_status = |e: &RuntimeEvent| matches!(e, RuntimeEvent::PrStatusChanged { workstream_id, .. } if *workstream_id == fetched_id);
    let told = !events_until(&mut events_rx, Duration::from_secs(5), is_status)
        .await
        .iter()
        .filter(|e| is_status(e))
        .collect::<Vec<_>>()
        .is_empty();
    let merge_state = lookup(&client, &fetched_id)
        .await
        .pointer("/pr/mergeState")
        .cloned();
    let evidence = json!({ "seen": seen, "toldWithin5s": told, "mergeState": merge_state });
    j.guard(
        "an unwatched open PR, polled every 5 min, picks up its new merge state from one Fetch",
        evidence
            == json!({
                "seen": { "number": 530, "state": "open" },
                "toldWithin5s": true,
                "mergeState": "clean",
            }),
        evidence,
    );

    j.step("a merge with a watcher lookup in flight stays merged");
    let race = create_workstream(&mut j, &client, &project, "Race").await;
    let race_id = text(&race, "/item/id");
    let race_branch = text(&race, "/item/branch");
    set_pr(&gh_state, &race_branch, &FakePr::new(701, "OPEN"));
    let open = await_observed(&client, &race_id, json!({ "number": 701, "state": "open" })).await;
    let slow_once = gh_state.join("SLOW_ONCE");
    std::fs::write(&slow_once, &race_branch).expect("slow the next lookup");
    let deadline = tokio::time::Instant::now() + WATCHER;
    let mut round = 0;
    while slow_once.exists() && tokio::time::Instant::now() < deadline {
        poke_watcher(&client, &race_id, round).await;
        round += 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let in_flight = !slow_once.exists();
    j.call(
        &client,
        Request::MergeWorkstreamPr {
            id: race_id.clone(),
            method: MergeMethod::Squash,
        },
    )
    .await
    .expect("merge_workstream_pr");
    let mut states = std::collections::BTreeSet::new();
    for _ in 0..9 {
        let state = lookup(&client, &race_id).await;
        states.insert(
            state
                .pointer("/pr/state")
                .and_then(Value::as_str)
                .unwrap_or("none")
                .to_string(),
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let row = observed_within(
        &client,
        &race_id,
        &json!({ "number": 701, "state": "merged" }),
    )
    .await;
    let evidence =
        json!({ "open": open, "inFlight": in_flight, "statesAfterMerge": states, "row": row });
    j.guard(
        "the watcher's slower answer from before the merge never replaces the merged PR",
        evidence
            == json!({
                "open": { "number": 701, "state": "open" },
                "inFlight": true,
                "statesAfterMerge": ["merged"],
                "row": { "number": 701, "state": "merged" },
            }),
        evidence,
    );

    j.step("a closed PR stays settled until Fetch sees it reopened");
    let reopened = create_workstream(&mut j, &client, &project, "Reopened").await;
    let reopened_id = text(&reopened, "/item/id");
    let reopened_branch = text(&reopened, "/item/branch");
    set_pr(&gh_state, &reopened_branch, &FakePr::new(800, "OPEN"));
    await_observed(
        &client,
        &reopened_id,
        json!({ "number": 800, "state": "open" }),
    )
    .await;
    set_pr(&gh_state, &reopened_branch, &FakePr::new(800, "CLOSED"));
    let closed = await_observed(
        &client,
        &reopened_id,
        json!({ "number": 800, "state": "closed" }),
    )
    .await;
    set_pr(&gh_state, &reopened_branch, &FakePr::new(800, "OPEN"));
    let before = fake_gh::calls(&gh_state, &reopened_branch);
    let_watcher_tick(&client, &reopened_id).await;
    let idle_polls = fake_gh::calls(&gh_state, &reopened_branch) - before;
    let still = observed_pr(&client, &reopened_id).await;
    let _ = client
        .call(Request::FetchWorkstreamRemote {
            id: reopened_id.clone(),
        })
        .await;
    let after_fetch = await_unwatched(
        &client,
        &reopened_id,
        json!({ "number": 800, "state": "open" }),
    )
    .await;
    let evidence = json!({
        "closed": closed,
        "idlePolls": idle_polls,
        "beforeFetch": still,
        "afterFetch": after_fetch,
    });
    j.guard(
        "a closed PR is not polled while idle; one Fetch sees it reopened",
        evidence
            == json!({
                "closed": { "number": 800, "state": "closed" },
                "idlePolls": 0,
                "beforeFetch": { "number": 800, "state": "closed" },
                "afterFetch": { "number": 800, "state": "open" },
            }),
        evidence,
    );

    j.step("a repository without a GitHub remote has no PR and is not polled");
    let local = sandbox.fixture_repo("local-only", &[("README.md", "# local\n")]);
    let local_project = j
        .call(
            &client,
            Request::ImportProject {
                path: sandbox.as_given(&local).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import_project");
    let local_ws = create_workstream(
        &mut j,
        &client,
        &text(&local_project, "/path"),
        "Local only",
    )
    .await;
    let local_id = text(&local_ws, "/item/id");
    let local_branch = text(&local_ws, "/item/branch");
    let first = lookup(&client, &local_id).await["kind"].clone();
    let_watcher_tick(&client, &local_id).await;
    let evidence = json!({
        "first": first,
        "ghCalls": fake_gh::calls(&gh_state, &local_branch),
        "afterTicks": lookup(&client, &local_id).await["kind"],
    });
    j.guard(
        "the lookup says the repository is not on GitHub, without asking gh",
        evidence == json!({ "first": "notGitHub", "ghCalls": 0, "afterTicks": "notGitHub" }),
        evidence,
    );

    j.step("a rebase keeps its PR, renamed or not");
    let plain = create_workstream(&mut j, &client, &project, "Rebase in place").await;
    let plain_id = text(&plain, "/item/id");
    let plain_tree = text(&plain, "/item/worktreePath");
    let plain_branch = text(&plain, "/item/branch");
    set_pr(&gh_state, &plain_branch, &FakePr::new(710, "OPEN"));
    await_observed(
        &client,
        &plain_id,
        json!({ "number": 710, "state": "open" }),
    )
    .await;
    let before = fake_gh::calls(&gh_state, &plain_branch);
    git(&plain_tree, &["checkout", "-q", "--detach"]);
    let_watcher_tick(&client, &plain_id).await;
    let plain_during = pr_number(&client, &plain_id).await;
    j.note(
        "rebaseInPlaceWatcherCalls",
        json!(fake_gh::calls(&gh_state, &plain_branch) - before),
    );
    git(&plain_tree, &["switch", "-q", &plain_branch]);

    let moved = create_workstream(&mut j, &client, &project, "Rebase renamed").await;
    let moved_id = text(&moved, "/item/id");
    let moved_tree = text(&moved, "/item/worktreePath");
    let moved_branch = text(&moved, "/item/branch");
    set_pr(&gh_state, "feature/rebase-1", &FakePr::new(711, "OPEN"));
    git(&moved_tree, &["branch", "-m", "feature/rebase-1"]);
    await_observed(
        &client,
        &moved_id,
        json!({ "number": 711, "state": "open" }),
    )
    .await;
    set_pr(&gh_state, &moved_branch, &FakePr::new(712, "OPEN"));
    let moved_calls = || {
        ["711", "712", "feature/rebase-1", moved_branch.as_str()]
            .iter()
            .map(|r| fake_gh::calls(&gh_state, r))
            .sum::<usize>()
    };
    git(&moved_tree, &["checkout", "-q", "--detach"]);
    tokio::time::sleep(IN_FLIGHT_POLL).await;
    let before = moved_calls();
    let first_read = pr_number(&client, &moved_id).await;
    let_watcher_tick(&client, &moved_id).await;
    let after_ticks = pr_number(&client, &moved_id).await;
    let detached_calls = moved_calls() - before;
    let stopped = server.shutdown(&client).await;
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let after_restart = lookup(&client, &moved_id).await["kind"].clone();
    let restart_calls = moved_calls() - before;
    let merges_before = gh_lines(&gh_state, "pr merge");
    let merge = client
        .call(Request::MergeWorkstreamPr {
            id: moved_id.clone(),
            method: MergeMethod::Squash,
        })
        .await;
    let merge_evidence = json!({
        "refused": merge.err().map(|e| e.code),
        "ghMergeCalls": gh_lines(&gh_state, "pr merge") - merges_before,
        "oldNamePr": gh_node(&gh_state, &moved_branch, "state"),
    });
    git(&moved_tree, &["switch", "-q", "feature/rebase-1"]);
    let back = pr_number(&client, &moved_id).await;
    let evidence = json!({
        "inPlace": plain_during,
        "renamed": {
            "firstRead": first_read,
            "afterTicks": after_ticks,
            "ghCallsWhileDetached": detached_calls,
            "restarted": stopped.status.success(),
            "afterRestart": after_restart,
            "ghCallsAfterRestart": restart_calls,
            "backOnBranch": back,
        },
    });
    j.guard(
        "a detached HEAD keeps the PR: an unrenamed branch is still looked up, a renamed one is \
         served from the cache without asking gh, and with nothing cached reads unknown, not no PR",
        evidence
            == json!({
                "inPlace": 710,
                "renamed": {
                    "firstRead": 711,
                    "afterTicks": 711,
                    "ghCallsWhileDetached": 0,
                    "restarted": true,
                    "afterRestart": "unknown",
                    "ghCallsAfterRestart": 0,
                    "backOnBranch": 711,
                },
            }),
        evidence,
    );
    j.guard(
        "with nothing cached mid-rebase, Merge is refused instead of merging the old name's PR",
        merge_evidence
            == json!({
                "refused": "pr_unknown_while_detached",
                "ghMergeCalls": 0,
                "oldNamePr": "OPEN",
            }),
        merge_evidence,
    );

    j.step("a base change mid-rebase reaches GitHub and the sidebar");
    let based = create_workstream(&mut j, &client, &project, "Base mid rebase").await;
    let based_id = text(&based, "/item/id");
    let based_tree = text(&based, "/item/worktreePath");
    set_pr(&gh_state, "feature/base-1", &FakePr::new(714, "OPEN"));
    git(&based_tree, &["branch", "-m", "feature/base-1"]);
    await_observed(
        &client,
        &based_id,
        json!({ "number": 714, "state": "open" }),
    )
    .await;
    git(&based_tree, &["checkout", "-q", "--detach"]);
    let mut retargets = Vec::new();
    for base in ["origin/release", "origin/main"] {
        let set = client
            .call(Request::SetWorkstreamBaseBranch {
                id: based_id.clone(),
                base: base.into(),
            })
            .await;
        let shown = lookup(&client, &based_id).await;
        retargets.push(json!({
            "ok": set.is_ok(),
            "github": gh_node(&gh_state, "feature/base-1", "baseRefName"),
            "sidebar": shown.pointer("/pr/baseRefName"),
        }));
    }
    git(&based_tree, &["switch", "-q", "feature/base-1"]);
    let evidence = json!({ "retargets": retargets, "ghEdits": gh_lines(&gh_state, "pr edit") });
    j.guard(
        "each retarget while detached edits the PR on GitHub and the sidebar shows the new base",
        evidence
            == json!({
                "retargets": [
                    { "ok": true, "github": "release", "sidebar": "release" },
                    { "ok": true, "github": "main", "sidebar": "main" },
                ],
                "ghEdits": 2,
            }),
        evidence,
    );

    j.step("a rebase right after a rename keeps the PR");
    let quick = create_workstream(&mut j, &client, &project, "Quick rename").await;
    let quick_id = text(&quick, "/item/id");
    let quick_tree = text(&quick, "/item/worktreePath");
    let quick_branch = text(&quick, "/item/branch");
    let _ = client
        .call(Request::WatchWorktree {
            workstream_id: Some(quick_id.clone()),
        })
        .await;
    let _ = client
        .call(Request::SetForeground { foreground: true })
        .await;
    let deadline = tokio::time::Instant::now() + WATCHER;
    while fake_gh::calls(&gh_state, &quick_branch) == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    set_pr(&gh_state, "feature/quick-1", &FakePr::new(715, "OPEN"));
    git(&quick_tree, &["branch", "-m", "feature/quick-1"]);
    let recorded = observed_unpoked(
        &client,
        &quick_id,
        &json!({ "number": 715, "state": "open" }),
        Duration::from_secs(10),
    )
    .await;
    let polled = fake_gh::calls(&gh_state, "feature/quick-1");
    std::fs::write(PathBuf::from(&quick_tree).join("edit.txt"), "edit\n").expect("write a file");
    git(&quick_tree, &["add", "edit.txt"]);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let polled_after_edit = fake_gh::calls(&gh_state, "feature/quick-1") - polled;

    let rushed = create_workstream(&mut j, &client, &project, "Rushed rebase").await;
    let rushed_id = text(&rushed, "/item/id");
    let rushed_tree = text(&rushed, "/item/worktreePath");
    let rushed_branch = text(&rushed, "/item/branch");
    set_pr(&gh_state, "feature/quick-2", &FakePr::new(716, "OPEN"));
    git(&rushed_tree, &["branch", "-m", "feature/quick-2"]);
    let shown_after_rename = pr_number(&client, &rushed_id).await;
    let old_name_before = fake_gh::calls(&gh_state, &rushed_branch);
    git(&rushed_tree, &["checkout", "-q", "--detach"]);
    let shown_detached = pr_number(&client, &rushed_id).await;
    let old_name_calls = fake_gh::calls(&gh_state, &rushed_branch) - old_name_before;
    git(&rushed_tree, &["switch", "-q", "feature/quick-2"]);
    let evidence = json!({
        "renameRecordedWithoutARead": recorded,
        "pollsAfterAnEdit": polled_after_edit,
        "readThenDetachAtOnce": {
            "afterRename": shown_after_rename,
            "detached": shown_detached,
            "ghCallsForTheOldName": old_name_calls,
        },
    });
    j.guard(
        "a terminal rename is recorded within seconds, an edit asks gh nothing, and a rebase \
         started right after the sidebar showed the PR keeps it without asking for the old name",
        evidence
            == json!({
                "renameRecordedWithoutARead": { "number": 715, "state": "open" },
                "pollsAfterAnEdit": 0,
                "readThenDetachAtOnce": {
                    "afterRename": 716,
                    "detached": 716,
                    "ghCallsForTheOldName": 0,
                },
            }),
        evidence,
    );

    j.step("a merged PR survives a pruned remote branch and a restart");
    let origin = sandbox.root.join("origin.git");
    sandbox.git(
        &sandbox.root,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch",
            "main",
            "origin.git",
        ],
    );
    sandbox.git(
        &repo,
        &["remote", "add", "origin", &origin.display().to_string()],
    );
    sandbox.git(&repo, &["push", "--quiet", "origin", "main"]);
    let kept = create_workstream(&mut j, &client, &project, "Pruned").await;
    let kept_id = text(&kept, "/item/id");
    let kept_tree = PathBuf::from(text(&kept, "/item/worktreePath"));
    let kept_branch = text(&kept, "/item/branch");
    sandbox.git(
        &kept_tree,
        &["commit", "--quiet", "--allow-empty", "--message", "ours"],
    );
    sandbox.git(
        &kept_tree,
        &["push", "--quiet", "-u", "origin", &kept_branch],
    );
    set_pr(&gh_state, &kept_branch, &FakePr::new(370, "OPEN"));
    let opened = await_observed(&client, &kept_id, json!({ "number": 370, "state": "open" })).await;
    sandbox.git(
        &sandbox.root,
        &[
            "clone",
            "--quiet",
            "--branch",
            &kept_branch,
            "origin.git",
            "mate",
        ],
    );
    let mate = sandbox.root.join("mate");
    sandbox.git(
        &mate,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "teammate",
        ],
    );
    let teammate_head = sandbox.git(&mate, &["rev-parse", "HEAD"]);
    sandbox.git(&mate, &["push", "--quiet", "origin", &kept_branch]);
    set_pr(
        &gh_state,
        &kept_branch,
        &FakePr {
            head_oid: Some(teammate_head),
            ..FakePr::new(370, "MERGED")
        },
    );
    let merged = await_observed(
        &client,
        &kept_id,
        json!({ "number": 370, "state": "merged" }),
    )
    .await;
    sandbox.git(
        &mate,
        &["push", "--quiet", "origin", "--delete", &kept_branch],
    );
    sandbox.git(&kept_tree, &["fetch", "--quiet", "--prune", "origin"]);
    let pruned = !sandbox
        .git(
            &kept_tree,
            &[
                "branch",
                "--remotes",
                "--list",
                &format!("origin/{kept_branch}"),
            ],
        )
        .contains(&kept_branch);

    let stuck = create_workstream(&mut j, &client, &project, "Stuck").await;
    let stuck_id = text(&stuck, "/item/id");
    let stuck_branch = text(&stuck, "/item/branch");
    set_pr(&gh_state, &stuck_branch, &FakePr::new(380, "MERGED"));
    await_observed(
        &client,
        &stuck_id,
        json!({ "number": 380, "state": "merged" }),
    )
    .await;

    let down = create_workstream(&mut j, &client, &project, "Down").await;
    let down_id = text(&down, "/item/id");
    let down_tree = PathBuf::from(text(&down, "/item/worktreePath"));
    let down_branch = text(&down, "/item/branch");
    set_pr(&gh_state, &down_branch, &FakePr::new(390, "OPEN"));
    await_observed(&client, &down_id, json!({ "number": 390, "state": "open" })).await;
    set_pr(
        &gh_state,
        &down_branch,
        &FakePr {
            head_oid: Some(NEVER_FETCHED.into()),
            ..FakePr::new(390, "MERGED")
        },
    );
    await_observed(
        &client,
        &down_id,
        json!({ "number": 390, "state": "merged" }),
    )
    .await;

    let stopped = server.shutdown(&client).await;
    sandbox.git(
        &down_tree,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "new work while the server was down",
        ],
    );
    set_pr(&gh_state, &down_branch, &FakePr::new(391, "OPEN"));
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let before = fake_gh::calls(&gh_state, &kept_branch);
    let_watcher_tick(&client, &kept_id).await;
    let watcher_polled = fake_gh::calls(&gh_state, &kept_branch) > before;
    let row = observed_pr(&client, &kept_id).await;
    let found = lookup(&client, &kept_id).await;
    let evidence = json!({
        "opened": opened,
        "merged": merged,
        "pruned": pruned,
        "restarted": stopped.status.success(),
        "watcherPolled": watcher_polled,
        "row": row,
        "lookup": { "kind": found["kind"], "state": found.pointer("/pr/state") },
    });
    j.guard(
        "a PR merged after a teammate's unfetched push stays merged across a prune and a restart, \
         and the restarted watcher does not poll it",
        evidence
            == json!({
                "opened": { "number": 370, "state": "open" },
                "merged": { "number": 370, "state": "merged" },
                "pruned": true,
                "restarted": true,
                "watcherPolled": false,
                "row": { "number": 370, "state": "merged" },
                "lookup": { "kind": "found", "state": "merged" },
            }),
        evidence,
    );

    j.step("a failed lookup after a restart keeps the merged PR and is retried");
    std::fs::write(gh_state.join("FAIL"), "").expect("make gh fail");
    let failed = lookup(&client, &stuck_id).await;
    std::fs::remove_file(gh_state.join("FAIL")).expect("let gh succeed");
    let before = fake_gh::calls(&gh_state, &stuck_branch);
    let_watcher_tick(&client, &stuck_id).await;
    let watcher_retried = fake_gh::calls(&gh_state, &stuck_branch) > before;
    let recovered = lookup(&client, &stuck_id).await;
    let evidence = json!({
        "failed": {
            "kind": failed["kind"],
            "last": { "number": failed.pointer("/last/number"), "state": failed.pointer("/last/state") },
        },
        "watcherRetried": watcher_retried,
        "recovered": { "kind": recovered["kind"], "state": recovered.pointer("/pr/state") },
    });
    j.guard(
        "the failure shows the merged PR the row recorded, and the watcher retries until gh answers",
        evidence
            == json!({
                "failed": { "kind": "failed", "last": { "number": 380, "state": "merged" } },
                "watcherRetried": true,
                "recovered": { "kind": "found", "state": "merged" },
            }),
        evidence,
    );

    j.step("new work and a new PR while the server was down reach the row");
    let reached = observed_within(
        &client,
        &down_id,
        &json!({ "number": 391, "state": "open" }),
    )
    .await;
    set_pr(&gh_state, &down_branch, &FakePr::new(391, "MERGED"));
    let merged = observed_within(
        &client,
        &down_id,
        &json!({ "number": 391, "state": "merged" }),
    )
    .await;
    let evidence = json!({ "afterRestart": reached, "thenMerged": merged });
    j.guard(
        "a commit made while the server was stopped wakes the settled PR, and the new PR is \
         recorded and followed to its merge",
        evidence
            == json!({
                "afterRestart": { "number": 391, "state": "open" },
                "thenMerged": { "number": 391, "state": "merged" },
            }),
        evidence,
    );

    j.step("pushing a branch does not claim an old merged PR with its name");
    let reused = create_workstream(&mut j, &client, &project, "Reused name").await;
    let reused_id = text(&reused, "/item/id");
    let reused_tree = PathBuf::from(text(&reused, "/item/worktreePath"));
    let reused_branch = text(&reused, "/item/branch");
    sandbox.git(
        &reused_tree,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "new work",
        ],
    );
    set_pr(
        &gh_state,
        &reused_branch,
        &FakePr {
            head_oid: Some(NEVER_FETCHED.into()),
            ..FakePr::new(600, "MERGED")
        },
    );
    let unpushed = lookup(&client, &reused_id).await["kind"].clone();
    sandbox.git(
        &reused_tree,
        &["push", "--quiet", "-u", "origin", &reused_branch],
    );
    let pushed = lookup(&client, &reused_id).await["kind"].clone();
    let_watcher_tick(&client, &reused_id).await;
    let pushed_row = observed_pr(&client, &reused_id).await;
    set_pr(&gh_state, &reused_branch, &FakePr::new(601, "OPEN"));
    let next = observed_within(
        &client,
        &reused_id,
        &json!({ "number": 601, "state": "open" }),
    )
    .await;
    let evidence = json!({
        "unpushed": unpushed,
        "pushed": pushed,
        "pushedRow": pushed_row,
        "next": next,
    });
    j.guard(
        "an old merged PR whose head the branch never held stays someone else's after a push, \
         and the branch's own new PR is found",
        evidence
            == json!({
                "unpushed": "noPr",
                "pushed": "noPr",
                "pushedRow": null,
                "next": { "number": 601, "state": "open" },
            }),
        evidence,
    );

    j.step("archiving mid-rebase deletes the app's branch only for its own PR");
    let mut kept_on_origin = serde_json::Map::new();
    for (title, renamed, number, detach) in [
        ("renamedDetached", Some("feature/arch-1"), 801, true),
        ("renamed", Some("feature/arch-2"), 802, false),
        ("ownDetached", None, 803, true),
    ] {
        let ws = create_workstream(&mut j, &client, &project, &format!("Archive {title}")).await;
        let ws_id = text(&ws, "/item/id");
        let tree = PathBuf::from(text(&ws, "/item/worktreePath"));
        let app_branch = text(&ws, "/item/branch");
        sandbox.git(
            &tree,
            &["commit", "--quiet", "--allow-empty", "--message", "ours"],
        );
        sandbox.git(&tree, &["push", "--quiet", "-u", "origin", &app_branch]);
        let pr_ref = match renamed {
            Some(name) => {
                sandbox.git(&tree, &["branch", "-m", name]);
                name.to_string()
            }
            None => app_branch.clone(),
        };
        set_pr(&gh_state, &pr_ref, &FakePr::new(number, "OPEN"));
        await_observed(
            &client,
            &ws_id,
            json!({ "number": number, "state": "open" }),
        )
        .await;
        set_pr(&gh_state, &pr_ref, &FakePr::new(number, "MERGED"));
        await_observed(
            &client,
            &ws_id,
            json!({ "number": number, "state": "merged" }),
        )
        .await;
        if detach {
            sandbox.git(&tree, &["checkout", "--quiet", "--detach"]);
        }
        j.call(
            &client,
            Request::ArchiveWorkstream {
                id: ws_id,
                skip_scripts: true,
            },
        )
        .await
        .expect("archive_workstream");
        kept_on_origin.insert(
            title.into(),
            json!(remote_has(&sandbox, &repo, &app_branch)),
        );
    }
    let evidence = Value::Object(kept_on_origin);
    j.guard(
        "a merged PR on a renamed branch never deletes the app's branch on origin, detached or \
         not; the app branch's own merged PR still does",
        evidence == json!({ "renamedDetached": true, "renamed": true, "ownDetached": false }),
        evidence,
    );

    j.step("an archive sweep mid-rebase keeps the PR the archive recorded");
    let swept = create_workstream(&mut j, &client, &project, "Sweep mid rebase").await;
    let swept_id = text(&swept, "/item/id");
    let swept_tree = PathBuf::from(text(&swept, "/item/worktreePath"));
    let swept_branch = text(&swept, "/item/branch");
    set_pr(&gh_state, "feature/sweep-1", &FakePr::new(713, "OPEN"));
    sandbox.git(&swept_tree, &["branch", "-m", "feature/sweep-1"]);
    await_observed(
        &client,
        &swept_id,
        json!({ "number": 713, "state": "open" }),
    )
    .await;
    sandbox.git(&swept_tree, &["checkout", "--quiet", "--detach"]);
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: swept_id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let at_archive = archived_pr(&client, &swept_id).await;
    let backdated = rusqlite::Connection::open(sandbox.app_data.join("hyprlayer.db"))
        .expect("open the profile database")
        .execute(
            "UPDATE workstream_archives SET pr_checked_at = pr_checked_at - ?2
             WHERE workstream_id = ?1",
            rusqlite::params![swept_id, 16.0 * 60_000.0],
        )
        .expect("backdate the archive's last PR check");
    let before = fake_gh::calls(&gh_state, &swept_branch);
    let _ = client.call(Request::RefreshArchivedWorkstreams {}).await;
    let evidence = json!({
        "atArchive": at_archive,
        "backdated": backdated,
        "afterSweep": archived_pr(&client, &swept_id).await,
        "ghCallsForTheOldName": fake_gh::calls(&gh_state, &swept_branch) - before,
    });
    j.guard(
        "the sweep keeps the open PR a mid-rebase archive recorded and asks gh nothing for the \
         app's old branch name",
        evidence
            == json!({
                "atArchive": { "state": "open", "number": 713 },
                "backdated": 1,
                "afterSweep": { "state": "open", "number": 713 },
                "ghCallsForTheOldName": 0,
            }),
        evidence,
    );

    j.step("a secondary rate limit holds only the watcher");
    let secondary = create_workstream(&mut j, &client, &project, "Secondary").await;
    let secondary_id = text(&secondary, "/item/id");
    std::fs::write(gh_state.join("SECONDARY"), "").expect("trip the secondary limit");
    let limited = lookup(&client, &secondary_id).await;
    std::fs::remove_file(gh_state.join("SECONDARY")).expect("clear the secondary limit");
    let after = lookup(&client, &secondary_id).await;
    let evidence = json!({
        "limited": { "kind": limited["kind"], "cause": limited["cause"] },
        "next": after["kind"],
    });
    j.guard(
        "a secondary limit is no quota hold: the next lookup goes through",
        evidence
            == json!({
                "limited": { "kind": "failed", "cause": { "kind": "other" } },
                "next": "noPr",
            }),
        evidence,
    );

    j.step("a failed lookup with nothing cached is not 'no PR'");
    let quota = create_workstream(&mut j, &client, &project, "Quota").await;
    std::fs::write(gh_state.join("RATE_LIMITED"), "").expect("rate-limit gh");
    let failed = lookup(&client, &text(&quota, "/item/id")).await;
    j.guard(
        "a rate-limited lookup reads as failed, with the reset GitHub reported and no last PR",
        failed
            == json!({
                "kind": "failed",
                "cause": { "kind": "rateLimited", "resumesAt": 4_070_908_800_000.0 },
                "last": null,
            }),
        failed,
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
