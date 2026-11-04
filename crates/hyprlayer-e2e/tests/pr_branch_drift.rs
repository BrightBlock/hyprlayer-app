//! An isolated worktree whose branch was renamed or switched still finds its PR, and starting a
//! later stage's session moves the workstream forward on the board but never back.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

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

/// `gh pr view <ref>` answers from `<dir>/<ref with / as _>` ("<number> <STATE>"), else as gh does for no PR.
fn fake_gh(dir: &Path) -> String {
    format!(
        r##"#!/bin/sh
dir="{dir}"
[ "$1" = "--version" ] && {{ echo "gh version 2.81.0 (2026-01-01)"; exit 0; }}
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  key=$(printf '%s' "$3" | tr '/' '_')
  if [ -f "$dir/$key" ]; then
    read -r number state < "$dir/$key"
    printf '{{"number":%s,"url":"https://github.com/e2e/demo/pull/%s","title":"E2E #%s","state":"%s","mergeable":"UNKNOWN","reviewDecision":"","statusCheckRollup":[]}}\n' "$number" "$number" "$number" "$state"
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

    j.step("an archive looks up the branch it was archived on");
    set_pr(&gh_state, "feature/csp-2", 340, "MERGED");
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
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
