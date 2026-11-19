//! The pull-request surface against real GitHub. `pr.rs` shells out to `gh`
//! and parses its `--json` stdout, so the contract under test is gh's output
//! shape as much as our own code — which only a live run can check.
//!
//! Needs network, a `gh` authenticated with `repo` scope, and push access to
//! `E2E_REPO`. It is the one scenario that deliberately defeats the sandbox's
//! "keep the network out" pin, so it stays behind `--ignored`.
//!
//! The run leaves a merged PR behind. GitHub has no API for deleting a pull
//! request at any scope, so the fixture repo accumulates them by design; the
//! branch is cleaned up, `main` is never written to except through a merge.
#![cfg(unix)]

use std::path::Path;

use hyprlayer_core::pr::MergeMethod;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};

const E2E_REPO: &str = "jtsiros/hyprlayer-e2e-fixture";

/// `--ignored` alone is not enough to opt in. `scripts/release.sh` runs
/// `cargo nextest run --run-ignored all` as its gate, and this scenario would
/// then open a pull request on every release and demand network plus GitHub
/// auth on the release builder.
const OPT_IN: &str = "HYPRLAYER_E2E_GITHUB";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs GitHub: cargo test -p hyprlayer-e2e -- --ignored"]
async fn pr_lifecycle() {
    if std::env::var_os(OPT_IN).is_none() {
        eprintln!("skipping pr_lifecycle: set {OPT_IN}=1 to run it against {E2E_REPO}");
        return;
    }
    let mut sandbox = Sandbox::new();
    let gh = sandbox.unpin_tool("gh");
    let token = gh_token(&gh);
    sandbox.set_env("GH_TOKEN", token.clone());
    let repo = clone_fixture(&sandbox, &token);

    let mut j = Journey::new("pr_lifecycle", &sandbox);
    // Before anything else: the token must never reach the artifact, and the
    // clone URL carries it.
    j.alias(token.clone(), "$GH_TOKEN");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Pr lifecycle").await;
    let id = text(&view, "/item/id");
    let branch = text(&view, "/item/branch");
    let worktree = text(&view, "/item/worktreePath");
    j.alias(&branch, "$BRANCH");

    j.step("push the workstream branch");
    // Unique per run: the fixture repo is long-lived, so every previous run's
    // probe is already merged into main and an identical file stages nothing.
    std::fs::write(
        Path::new(&worktree).join("probe.txt"),
        format!("{branch}\n"),
    )
    .expect("write a probe file");
    // Not `add --all`: worktree provisioning links `thoughts/` into the tree and
    // the CLI's pre-commit hook refuses to commit it.
    sandbox.git(Path::new(&worktree), &["add", "probe.txt"]);
    sandbox.git(
        Path::new(&worktree),
        &["commit", "--quiet", "--message", "Probe commit"],
    );
    sandbox.git(
        Path::new(&worktree),
        &["push", "--quiet", "origin", &branch],
    );
    let pushed = gh_json(
        &gh,
        &token,
        &["api", &format!("repos/{E2E_REPO}/branches/{branch}")],
    );
    assert_eq!(
        text(&pushed, "/name"),
        branch,
        "the branch reaches the remote"
    );

    j.step("open a pull request");
    // The app has no create-PR command: the Ship agent runs `gh` itself, so the
    // scenario opens the PR the same way and exercises what the app does own.
    let opened = run(
        &gh,
        &token,
        &[
            "pr",
            "create",
            "--repo",
            E2E_REPO,
            "--base",
            "main",
            "--head",
            &branch,
            "--title",
            "E2E probe",
            "--body",
            "opened by pr_lifecycle",
        ],
    );
    assert!(opened, "gh pr create failed");
    let number = pr_number_for(&gh, &token, &branch) as u32;
    // Deliberately NOT `alias(number)`: a bare small integer rewrites every
    // matching digit in the artifact, and it turned "E2E probe" into
    // "E$PRE probe". The number is kept out of the evidence instead.
    j.alias(
        format!("https://github.com/{E2E_REPO}/pull/{number}"),
        "$PR_URL",
    );
    let attached = j
        .call(
            &client,
            Request::AttachWorkstreamPr {
                id: id.clone(),
                pr_number: number,
            },
        )
        .await
        .expect("attach_workstream_pr");
    j.guard(
        "attaching reads the PR back through gh",
        attached.pointer("/number").and_then(Value::as_u64) == Some(number as u64),
        json!({ "state": attached.pointer("/state"), "title": attached.pointer("/title") }),
    );

    j.step("read its status");
    let status = j
        .call(&client, Request::WorkstreamPrStatus { id: id.clone() })
        .await
        .expect("workstream_pr_status");
    j.guard(
        "an open PR reports itself open and unmerged",
        text(&status, "/state") == "open",
        json!({ "state": status.pointer("/state"), "mergeable": status.pointer("/mergeable") }),
    );
    let preview = j
        .call(
            &client,
            Request::PreviewWorkstreamPr {
                id: id.clone(),
                pr_number: number,
            },
        )
        .await
        .expect("preview_workstream_pr");
    j.guard(
        "preview resolves the same PR by number",
        text(&preview, "/headRefName") == branch,
        json!({ "head": preview.pointer("/headRefName"), "draft": preview.pointer("/isDraft") }),
    );
    let open = j
        .call(
            &client,
            Request::ListWorkstreamOpenPrs {
                id: id.clone(),
                query: String::new(),
            },
        )
        .await
        .expect("list_workstream_open_prs");
    j.guard(
        "the branch's PR is listed as open",
        numbers(&open).contains(&(number as u64)),
        json!({ "listed": numbers(&open).len() }),
    );

    j.step("merge it");
    j.call(
        &client,
        Request::MergeWorkstreamPr {
            id: id.clone(),
            method: MergeMethod::Squash,
        },
    )
    .await
    .expect("merge_workstream_pr");
    let merged = gh_json(
        &gh,
        &token,
        &[
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            E2E_REPO,
            "--json",
            "state,mergedAt",
        ],
    );
    j.guard(
        "GitHub reports the PR merged",
        text(&merged, "/state") == "MERGED" && !merged["mergedAt"].is_null(),
        json!({ "state": merged.pointer("/state") }),
    );
    let after = j
        .call(&client, Request::WorkstreamPrStatus { id: id.clone() })
        .await
        .expect("workstream_pr_status");
    j.guard(
        "the app sees the merge too",
        text(&after, "/state") == "merged",
        json!({ "state": after.pointer("/state") }),
    );

    j.step("clean up the branch");
    run(
        &gh,
        &token,
        &[
            "api",
            "--method",
            "DELETE",
            &format!("repos/{E2E_REPO}/git/refs/heads/{branch}"),
        ],
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

fn gh_token(gh: &Path) -> String {
    let out = std::process::Command::new(gh)
        .arg("auth")
        .arg("token")
        .output()
        .expect("gh auth token");
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        !token.is_empty(),
        "gh is not authenticated; run `gh auth login`"
    );
    token
}

/// The fixture repo, cloned with the token in the remote URL: the sandbox has
/// no credential helper and `git` cannot reach the host's keyring.
fn clone_fixture(sandbox: &Sandbox, token: &str) -> std::path::PathBuf {
    let dest = sandbox.home.join("code").join("fixture");
    std::fs::create_dir_all(dest.parent().expect("code dir")).expect("create the code dir");
    let url = format!("https://x-access-token:{token}@github.com/{E2E_REPO}.git");
    sandbox.git(
        &sandbox.home,
        &["clone", "--quiet", &url, &dest.display().to_string()],
    );
    dest
}

fn run(gh: &Path, token: &str, args: &[&str]) -> bool {
    std::process::Command::new(gh)
        .args(args)
        .env("GH_TOKEN", token)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn gh_json(gh: &Path, token: &str, args: &[&str]) -> Value {
    let out = std::process::Command::new(gh)
        .args(args)
        .env("GH_TOKEN", token)
        .output()
        .expect("run gh");
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

/// `gh pr create` prints a URL and has no `--json`, so the number is looked up
/// by head branch instead of parsed out of its output.
fn pr_number_for(gh: &Path, token: &str, branch: &str) -> u64 {
    let found = gh_json(
        gh,
        token,
        &[
            "pr", "list", "--repo", E2E_REPO, "--head", branch, "--json", "number",
        ],
    );
    found
        .as_array()
        .and_then(|a| a.first())
        .and_then(|p| p.pointer("/number"))
        .and_then(Value::as_u64)
        .expect("a PR for the pushed branch")
}

fn numbers(prs: &Value) -> Vec<u64> {
    prs.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|p| p.pointer("/number").and_then(Value::as_u64))
                .collect()
        })
        .unwrap_or_default()
}
