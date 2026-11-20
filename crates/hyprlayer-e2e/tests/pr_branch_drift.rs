//! An isolated worktree whose branch was renamed or switched still finds its PR. The board's stage
//! and observed PR follow the workstream from its first session through merge and onto a new branch.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);
/// Longer than the watcher's 20s poll of a focused open PR.
const WATCHER: Duration = Duration::from_secs(30);

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

/// `gh pr view <ref>` answers from `<dir>/<ref with / as _>` ("<number> <STATE> [MERGEABLE]"), else as gh does for no PR.
fn fake_gh(dir: &Path) -> String {
    format!(
        r##"#!/bin/sh
dir="{dir}"
[ "$1" = "--version" ] && {{ echo "gh version 2.81.0 (2026-01-01)"; exit 0; }}
[ -f "$dir/FAIL" ] && {{ echo "HTTP 502: Bad Gateway" >&2; exit 1; }}
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  key=$(printf '%s' "$3" | tr '/' '_')
  if [ -f "$dir/$key" ]; then
    read -r number state mergeable < "$dir/$key"
    printf '{{"number":%s,"url":"https://github.com/e2e/demo/pull/%s","title":"E2E #%s","state":"%s","mergeable":"%s","reviewDecision":"","statusCheckRollup":[]}}\n' "$number" "$number" "$number" "$state" "${{mergeable:-UNKNOWN}}"
    exit 0
  fi
  echo "no pull requests found for branch \"$3\"" >&2
  exit 1
fi
echo "e2e gh stub: unsupported: $*" >&2
exit 1
"##,
        dir = dir.display()
    )
}

fn set_pr(dir: &Path, reference: &str, number: u32, state: &str) {
    std::fs::write(
        dir.join(reference.replace('/', "_")),
        format!("{number} {state}\n"),
    )
    .expect("write the gh state");
}

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
    let deadline = tokio::time::Instant::now() + WATCHER;
    let mut round = 0;
    loop {
        let seen = observed_pr(client, id).await;
        if seen == want || tokio::time::Instant::now() >= deadline {
            assert_eq!(seen, want, "the watcher never recorded this PR state");
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

async fn pr_number(client: &RpcClient, id: &str) -> Value {
    let status = client
        .call(Request::WorkstreamPrStatus { id: id.to_string() })
        .await
        .expect("workstream_pr_status");
    let status = serde_json::to_value(&status).expect("serialize the PR status");
    first_number(&status)
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
    sandbox.stage_tool("gh", &fake_gh(&gh_state));
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
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
    set_pr(&gh_state, "feature/csp-1", 339, "OPEN");
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
    set_pr(&gh_state, "feature/csp-2", 340, "OPEN");
    let switched = git(&worktree, &["switch", "-q", "-c", "feature/csp-2"]);
    let followed = pr_number(&client, &id).await;
    j.guard(
        "after `git switch -c` the new branch's PR is found",
        switched && followed == json!(340),
        json!({ "switched": switched, "pr": followed }),
    );

    j.step("an attached PR wins over HEAD");
    set_pr(&gh_state, "339", 339, "OPEN");
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
    std::fs::write(gh_state.join("feature_csp-2"), "340 OPEN MERGEABLE\n")
        .expect("write the gh state");
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
    set_pr(&gh_state, "feature/csp-2", 340, "MERGED");
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
    set_pr(&gh_state, "hotfix/csp-3", 341, "MERGED");
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
    set_pr(&gh_state, "feature/csp-5", 342, "OPEN");
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
    set_pr(&gh_state, "feature/csp-6", 343, "MERGED");
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
    set_pr(&gh_state, "feature/csp-7", 344, "OPEN");
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
    set_pr(&gh_state, "360", 360, "OPEN");
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
    set_pr(&gh_state, "360", 360, "MERGED");
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
    set_pr(&gh_state, &text(&quick, "/item/branch"), 350, "MERGED");
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
    std::fs::remove_file(gh_state.join("feature_csp-2")).expect("hide the PR from the archive");
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    set_pr(&gh_state, "feature/csp-2", 340, "MERGED");
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

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}
