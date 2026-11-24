//! Where thoughts docs' `status` settles across the pipeline: a plan activates
//! when implementing starts; a merge settles the PR record, built plans,
//! research and handoffs; a close archives the record; archiving settles what
//! the PR watcher never saw; and a sweep settles records no workstream tracks.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::docs::frontmatter;
use hyprlayer_core::index::indexer::ThoughtQuery;
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::fake_gh::{self, set_pr, FakePr};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// `WRITE_DOC <manifest>` copies each `<src> <dest>` line and reports one successful `Write` per copy.
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
  case "$line" in
    *WRITE_DOC*)
      manifest=$(printf '%s\n' "$line" | sed -n 's/.*WRITE_DOC \([^" ]*\).*/\1/p')
      n=0
      while read -r src dest; do
        n=$((n + 1))
        mkdir -p "$(dirname "$dest")"
        cp "$src" "$dest"
        printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_%s_%s","name":"Write","input":{"file_path":"%s","content":"Written by the agent."}}]},"session_id":"%s"}\n' "$turn" "$n" "$dest" "$sid"
        printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_%s_%s","content":"File written.","is_error":false}]},"session_id":"%s"}\n' "$turn" "$n" "$sid"
      done < "$manifest"
      ;;
  esac
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7}}\n' "$turn" "$sid" "$turn"
done
"##;

const BUILT_PLAN: &str = "---\ntitle: Widget\ntype: plan\nstatus: draft\nauthor: e2e\n---\n\n\
# Widget\n\n## Phase 1: Model\n\n### Changes Required\n\nAdd the model.\n\n\
```md\n- [ ] a box inside a fence is not a task\n```\n\n\
### Success Criteria\n\n#### Automated Verification:\n- [x] `cargo test` passes\n\n\
#### Manual Verification:\n- [ ] The widget renders\n\n\
## Phase 2: View\n\n### Success Criteria\n\n#### Automated Verification:\n- [x] `pnpm test` passes\n\n\
#### Manual Verification:\n- [ ] Hover reads well\n";

const PARTIAL_PLAN: &str = "---\ntitle: Partial\ntype: plan\nstatus: active\n---\n\n# Partial\n\n\
## Phase 1: First PR\n\n### Success Criteria\n\n#### Automated Verification:\n- [x] Builds\n\n\
## Phase 2: Second PR\n\n### Success Criteria\n\n#### Automated Verification:\n- [ ] Builds\n";

const BARE_PLAN: &str = "---\ntitle: Bare\ntype: plan\nstatus: active\n---\n\n# Bare\n\n\
## Phase 1: Spike\n\nTry it and see; nothing to tick.\n";

const REPLACED_PLAN: &str =
    "---\ntitle: Replaced\ntype: plan\nstatus: superseded\n---\n\n# Replaced\n\n\
## Phase 1: Old\n\n- [x] Done\n";

/// Phase 2 is the next workstream's PR.
const HANDED_ON_PLAN: &str =
    "---\ntitle: Handed on\ntype: plan\nstatus: active\n---\n\n# Handed on\n\n\
## Phase 1: First PR\n\n- [x] Model\n\n## Phase 2: Second PR\n\n- [ ] View\n";

const ABANDONED_PLAN: &str =
    "---\ntitle: Abandoned\ntype: plan\nstatus: active\n---\n\n# Abandoned\n\n\
## Phase 1: Try\n\n- [x] Done\n";

const RESEARCH: &str =
    "---\ntitle: Widget research\ntype: research\nstatus: complete\n---\n\nThe widget lives in one module.\n";

const LATE_RESEARCH: &str =
    "---\ntitle: Late research\ntype: research\nstatus: draft\n---\n\nFound after the merge.\n";

const HANDOFF: &str =
    "---\ntitle: Widget handoff\ntype: handoff\nstatus: active\n---\n\nPhase 2 next.\n";

const NOTE: &str = "---\ntitle: Widget note\ntype: note\nstatus: draft\n---\n\nA scratch note.\n";

fn record(number: u32, status: &str) -> String {
    format!(
        "---\ntitle: \"PR #{number}: E2E\"\ntype: pr\nstatus: {status}\nproject: demo\n---\n\n\
         ## What problem(s) was I solving?\n\nBody stays put.\n"
    )
}

fn status_of(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| frontmatter::parse(&c).fm.status)
        .map_or(Value::Null, Value::String)
}

/// Polls `path` off the transcript until its status is `want`, waking the PR watcher between looks.
async fn await_status(client: &RpcClient, path: &Path, want: &str) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    loop {
        if status_of(path) == json!(want) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        let _ = client
            .call(Request::SetForeground { foreground: true })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Writes each doc through one agent turn in `workstream`, then waits until every one is attributed.
async fn agent_writes(
    j: &mut Journey,
    client: &RpcClient,
    sandbox: &Sandbox,
    workstream: &str,
    docs: &[(&Path, &str)],
) {
    let staging = sandbox.root.join("staging").join(workstream);
    std::fs::create_dir_all(&staging).expect("create the staging dir");
    let mut manifest = String::new();
    for (i, (dest, content)) in docs.iter().enumerate() {
        let src = staging.join(format!("{i}.md"));
        std::fs::write(&src, content).expect("stage a doc");
        manifest.push_str(&format!("{} {}\n", src.display(), dest.display()));
    }
    let manifest_path = staging.join("manifest");
    std::fs::write(&manifest_path, manifest).expect("write the manifest");

    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            workstream.to_string(),
            Stage::General,
            format!("WRITE_DOC {}", manifest_path.display()),
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
    drain_turn(&mut events).await;
    let ids: Vec<String> = docs.iter().map(|(p, _)| p.display().to_string()).collect();
    assert!(
        await_attributed(client, &ids).await,
        "the agent's writes were not attributed in time"
    );
}

async fn await_attributed(client: &RpcClient, ids: &[String]) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    loop {
        if let Ok(result) = client.query_docs(ThoughtQuery::default()).await {
            let attributed = ids.iter().all(|id| {
                result
                    .items
                    .iter()
                    .any(|item| item.id == *id && item.agent.is_some())
            });
            if attributed {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn drain_turn(events: &mut EventStream<SessionEvent>) {
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            if matches!(event, SessionEvent::TurnDone { .. }) {
                return;
            }
        }
    })
    .await;
}

async fn tick_watching(client: &RpcClient, id: &str, rounds: u32) {
    for round in 0..rounds {
        let _ = client
            .call(Request::WatchWorktree {
                workstream_id: (!round.is_multiple_of(2)).then(|| id.to_string()),
            })
            .await;
        let _ = client
            .call(Request::SetForeground { foreground: true })
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn lookup_kind(client: &RpcClient, id: &str) -> Value {
    let status = client
        .call(Request::WorkstreamPrStatus { id: id.to_string() })
        .await
        .expect("workstream_pr_status");
    serde_json::to_value(&status).expect("serialize the PR status")["workstream_pr_status"]["kind"]
        .take()
}

fn statuses(named: &[(&str, &Path)]) -> Value {
    Value::Object(
        named
            .iter()
            .map(|(name, path)| (name.to_string(), status_of(path)))
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doc_status_lifecycle() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let gh_state = sandbox.root.join("gh-state");
    std::fs::create_dir_all(&gh_state).expect("create the gh state dir");
    sandbox.stage_tool("gh", &fake_gh::script(&gh_state));
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    sandbox.git(&repo, &["remote", "add", "github", fake_gh::GITHUB_REMOTE]);
    let mut j = Journey::new("doc_status_lifecycle", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("find the project's shared thoughts");
    let seed = j
        .call(
            &client,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.clone()),
                    scope: "shared".into(),
                    doc_type: "note".into(),
                    title: "Seed".into(),
                    date: "2026-01-01".into(),
                    ticket: None,
                    tags: vec![],
                },
            },
        )
        .await
        .expect("create_doc");
    let resolved = j
        .call(
            &client,
            Request::ResolveDocPath {
                id: text(&seed, "/id"),
            },
        )
        .await
        .expect("resolve_doc_path");
    let shared: PathBuf = Path::new(&text(&resolved, ""))
        .ancestors()
        .nth(2)
        .expect("<shared>/notes/<file>")
        .to_path_buf();
    j.alias(shared.display().to_string(), "$SHARED");
    j.guard(
        "the seed note lands under <shared>/notes",
        shared.join("notes").is_dir(),
        json!({ "shared": shared.display().to_string() }),
    );
    let doc = |rel: &str| shared.join(rel);
    let (widget, partial, bare, replaced) = (
        doc("plans/widget.md"),
        doc("plans/partial.md"),
        doc("plans/bare.md"),
        doc("plans/replaced.md"),
    );
    let (research, handoff, note, other_record) = (
        doc("research/widget.md"),
        doc("handoffs/widget.md"),
        doc("notes/widget.md"),
        doc("prs/7_description.md"),
    );
    let (handed_on, abandoned) = (doc("plans/handed-on.md"), doc("plans/abandoned.md"));

    j.step("an implement session activates the plan it starts from");
    let w1 = create_workstream(&mut j, &client, &project, "Ship the widget").await;
    let w1_id = text(&w1, "/item/id");
    let w1_branch = text(&w1, "/item/branch");
    agent_writes(
        &mut j,
        &client,
        &sandbox,
        &w1_id,
        &[
            (widget.as_path(), BUILT_PLAN),
            (partial.as_path(), PARTIAL_PLAN),
            (bare.as_path(), BARE_PLAN),
            (replaced.as_path(), REPLACED_PLAN),
            (research.as_path(), RESEARCH),
            (handoff.as_path(), HANDOFF),
            (note.as_path(), NOTE),
            (other_record.as_path(), record(7, "active").as_str()),
            (handed_on.as_path(), HANDED_ON_PLAN),
        ],
    )
    .await;
    let (implement, _prompt, _stream_id, mut implement_events) = client
        .start_stage_session_stream(
            w1_id.clone(),
            Stage::Implement,
            String::new(),
            vec![widget.display().to_string()],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start the implement session");
    let activated = status_of(&widget);
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: implement.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    drain_turn(&mut implement_events).await;
    j.guard(
        "the attached draft plan is active before the agent's first turn, without the Approve button",
        activated == json!("active"),
        json!({ "widget": activated }),
    );

    j.step("another workstream builds the rest of a plan");
    let w2 = create_workstream(&mut j, &client, &project, "Second half").await;
    let w2_id = text(&w2, "/item/id");
    let w2_branch = text(&w2, "/item/branch");
    let finished = HANDED_ON_PLAN.replace("- [ ] View", "- [x] View");
    agent_writes(
        &mut j,
        &client,
        &sandbox,
        &w2_id,
        &[
            (handed_on.as_path(), finished.as_str()),
            (abandoned.as_path(), ABANDONED_PLAN),
        ],
    )
    .await;
    let listed = client
        .query_docs(ThoughtQuery::default())
        .await
        .expect("query_docs");
    let newest = listed
        .items
        .iter()
        .find(|d| d.id == handed_on.display().to_string())
        .and_then(|d| d.agent.as_ref())
        .map(|a| a.workstream_id.clone());
    j.guard(
        "the second workstream is the plan's newest writer, and every box is now ticked",
        newest.as_deref() == Some(w2_id.as_str())
            && std::fs::read_to_string(&handed_on).ok().as_deref() == Some(finished.as_str()),
        json!({ "newestWriterIsSecond": newest.as_deref() == Some(w2_id.as_str()) }),
    );

    j.step("a merge settles what the workstream wrote");
    let record_41 = doc("prs/41_description.md");
    set_pr(&gh_state, &w1_branch, &FakePr::new(41, "MERGED"));
    j.call(
        &client,
        Request::WatchWorktree {
            workstream_id: Some(w1_id.clone()),
        },
    )
    .await
    .expect("watch_worktree");
    let seen = await_status(&client, &widget, "implemented").await;
    let after_merge = statuses(&[
        ("widget", widget.as_path()),
        ("partial", partial.as_path()),
        ("bare", bare.as_path()),
        ("replaced", replaced.as_path()),
        ("research", research.as_path()),
        ("handoff", handoff.as_path()),
        ("note", note.as_path()),
        ("pr7", other_record.as_path()),
        ("pr41", record_41.as_path()),
        ("handedOn", handed_on.as_path()),
    ]);
    j.guard(
        "built plan → implemented, research → implemented, handoff → archived; an unfinished or \
         checkbox-less plan, a superseded plan, a note, another PR's record, and a plan the next \
         workstream has since built stay; no record yet",
        seen && after_merge
            == json!({
                "widget": "implemented",
                "partial": "active",
                "bare": "active",
                "replaced": "superseded",
                "research": "implemented",
                "handoff": "archived",
                "note": "draft",
                "pr7": "active",
                "pr41": null,
                "handedOn": "active",
            }),
        after_merge,
    );
    let widget_text = std::fs::read_to_string(&widget).unwrap_or_default();
    j.guard(
        "only the status line changed: manual boxes and the fenced box stay as written",
        widget_text == BUILT_PLAN.replace("status: draft", "status: implemented"),
        json!({ "unchanged": widget_text == BUILT_PLAN.replace("status: draft", "status: implemented") }),
    );

    j.step("a record written after the merge was seen still settles");
    std::fs::write(record_41.as_path(), record(41, "active")).expect("write the late record");
    let settled = await_status(&client, &record_41, "implemented").await;
    let record_text = std::fs::read_to_string(&record_41).unwrap_or_default();
    j.guard(
        "describe_pr's late `active` record becomes implemented, body untouched",
        settled && record_text == record(41, "implemented"),
        json!({ "pr41": status_of(&record_41), "bodyIntact": record_text == record(41, "implemented") }),
    );

    j.step("a doc written after the merge stays draft");
    let after_merge = doc("research/after-merge.md");
    agent_writes(
        &mut j,
        &client,
        &sandbox,
        &w1_id,
        &[(after_merge.as_path(), LATE_RESEARCH)],
    )
    .await;
    let before = fake_gh::calls(&gh_state, &w1_branch);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut round = 0u32;
    while tokio::time::Instant::now() < deadline {
        let _ = client
            .call(Request::WatchWorktree {
                workstream_id: (round % 2 == 1).then(|| w1_id.clone()),
            })
            .await;
        let _ = client
            .call(Request::SetForeground { foreground: true })
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        round += 1;
    }
    let idle_polls = fake_gh::calls(&gh_state, &w1_branch) - before;
    let idle_status = status_of(&after_merge);
    let _ = client
        .call(Request::FetchWorkstreamRemote { id: w1_id })
        .await;
    let kicked = eventually(Duration::from_secs(5), || {
        fake_gh::calls(&gh_state, &w1_branch) > before + idle_polls
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let kick_polls = fake_gh::calls(&gh_state, &w1_branch) - before - idle_polls;
    let evidence = json!({
        "idleStatus": idle_status,
        "idlePolls": idle_polls,
        "kicked": kicked,
        "kickPolls": kick_polls,
        "afterKick": status_of(&after_merge),
    });
    j.guard(
        "a settled merge is not polled and leaves a later doc alone; one Fetch polls it once and \
         still leaves the doc alone",
        evidence
            == json!({
                "idleStatus": "draft",
                "idlePolls": 0,
                "kicked": true,
                "kickPolls": 1,
                "afterKick": "draft",
            }),
        evidence,
    );

    j.step("a PR closed unmerged archives its record and nothing else");
    let record_42 = doc("prs/42_description.md");
    std::fs::write(record_42.as_path(), record(42, "active")).expect("write record 42");
    set_pr(&gh_state, &w2_branch, &FakePr::new(42, "CLOSED"));
    j.call(
        &client,
        Request::WatchWorktree {
            workstream_id: Some(w2_id),
        },
    )
    .await
    .expect("watch_worktree");
    let archived = await_status(&client, &record_42, "archived").await;
    let after_close = statuses(&[
        ("pr42", record_42.as_path()),
        ("abandoned", abandoned.as_path()),
        ("handedOn", handed_on.as_path()),
    ]);
    j.guard(
        "the record is archived; the workstream's built plans are not promoted",
        archived
            && after_close
                == json!({ "pr42": "archived", "abandoned": "active", "handedOn": "active" }),
        after_close,
    );

    j.step("pushing a reused branch name does not settle its docs");
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
    let origin = sandbox.root.join("origin.git");
    sandbox.git(
        &repo,
        &["remote", "add", "origin", &origin.display().to_string()],
    );
    sandbox.git(&repo, &["push", "--quiet", "origin", "main"]);
    let w4 = create_workstream(&mut j, &client, &project, "Reused name").await;
    let w4_id = text(&w4, "/item/id");
    let w4_branch = text(&w4, "/item/branch");
    let w4_tree = PathBuf::from(text(&w4, "/item/worktreePath"));
    let (reused_research, reused_handoff) = (doc("research/reused.md"), doc("handoffs/reused.md"));
    agent_writes(
        &mut j,
        &client,
        &sandbox,
        &w4_id,
        &[
            (reused_research.as_path(), RESEARCH),
            (reused_handoff.as_path(), HANDOFF),
        ],
    )
    .await;
    sandbox.git(
        &w4_tree,
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
        &w4_branch,
        &FakePr {
            head_oid: Some("1111111111111111111111111111111111111111".into()),
            ..FakePr::new(600, "MERGED")
        },
    );
    sandbox.git(&w4_tree, &["push", "--quiet", "-u", "origin", &w4_branch]);
    let before = fake_gh::calls(&gh_state, &w4_branch);
    tick_watching(&client, &w4_id, 12).await;
    let reused = json!({
        "polled": fake_gh::calls(&gh_state, &w4_branch) > before,
        "lookup": lookup_kind(&client, &w4_id).await,
        "research": status_of(&reused_research),
        "handoff": status_of(&reused_handoff),
    });
    j.guard(
        "an old merged PR with the branch's name, whose head it never held, settles nothing",
        reused
            == json!({ "polled": true, "lookup": "noPr", "research": "complete", "handoff": "active" }),
        reused,
    );

    j.step("a merge seen with an unpushed local commit settles the docs");
    let w5 = create_workstream(&mut j, &client, &project, "Unpushed").await;
    let w5_id = text(&w5, "/item/id");
    let w5_branch = text(&w5, "/item/branch");
    let w5_tree = PathBuf::from(text(&w5, "/item/worktreePath"));
    let (late_research, late_handoff) = (doc("research/unpushed.md"), doc("handoffs/unpushed.md"));
    agent_writes(
        &mut j,
        &client,
        &sandbox,
        &w5_id,
        &[
            (late_research.as_path(), RESEARCH),
            (late_handoff.as_path(), HANDOFF),
        ],
    )
    .await;
    sandbox.git(
        &w5_tree,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "the PR's head",
        ],
    );
    let head = sandbox.git(&w5_tree, &["rev-parse", "HEAD"]);
    let pr = |state| FakePr {
        head_oid: Some(head.clone()),
        ..FakePr::new(610, state)
    };
    set_pr(&gh_state, &w5_branch, &pr("OPEN"));
    let before = fake_gh::calls(&gh_state, &w5_branch);
    tick_watching(&client, &w5_id, 12).await;
    let saw_open = fake_gh::calls(&gh_state, &w5_branch) > before;
    sandbox.git(
        &w5_tree,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "a follow-up nobody pushed",
        ],
    );
    set_pr(&gh_state, &w5_branch, &pr("MERGED"));
    tick_watching(&client, &w5_id, 2).await;
    let settled = await_status(&client, &late_research, "implemented").await
        && await_status(&client, &late_handoff, "archived").await;
    let unpushed = json!({
        "sawOpen": saw_open,
        "settled": settled,
        "research": status_of(&late_research),
        "handoff": status_of(&late_handoff),
        "lookup": lookup_kind(&client, &w5_id).await,
    });
    j.guard(
        "the first sight of the merge settles its docs; the unpushed commit then offers Ship",
        unpushed
            == json!({
                "sawOpen": true,
                "settled": true,
                "research": "implemented",
                "handoff": "archived",
                "lookup": "noPr",
            }),
        unpushed,
    );

    j.step("archiving settles a merge the watcher never saw");
    j.call(&client, Request::SetForeground { foreground: false })
        .await
        .expect("set_foreground");
    let w3 = create_workstream(&mut j, &client, &project, "Late research").await;
    let w3_id = text(&w3, "/item/id");
    let w3_branch = text(&w3, "/item/branch");
    let late = doc("research/late.md");
    agent_writes(
        &mut j,
        &client,
        &sandbox,
        &w3_id,
        &[(late.as_path(), LATE_RESEARCH)],
    )
    .await;
    let record_43 = doc("prs/43_description.md");
    std::fs::write(record_43.as_path(), record(43, "draft")).expect("write record 43");
    set_pr(&gh_state, &w3_branch, &FakePr::new(43, "MERGED"));
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: w3_id,
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let at_archive = statuses(&[("late", late.as_path()), ("pr43", record_43.as_path())]);
    j.guard(
        "with the app in the background, archive alone settles the research and the record",
        at_archive == json!({ "late": "implemented", "pr43": "implemented" }),
        at_archive,
    );
    j.call(&client, Request::SetForeground { foreground: true })
        .await
        .expect("set_foreground");

    j.step("a restart sweeps records no workstream tracks");
    let sweep: Vec<(u32, &str, Option<&str>)> = vec![
        (44, "active", Some("MERGED")),
        (45, "merged", Some("MERGED")),
        (46, "active", Some("OPEN")),
        (47, "draft", None),
        (48, "closed", Some("CLOSED")),
    ];
    for (number, status, state) in &sweep {
        std::fs::write(
            doc(&format!("prs/{number}_description.md")),
            record(*number, status),
        )
        .expect("write a sweep record");
        if let Some(state) = state {
            set_pr(&gh_state, &number.to_string(), &FakePr::new(*number, state));
        }
    }
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the first server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let swept = await_status(&client, &doc("prs/44_description.md"), "implemented").await
        && await_status(&client, &doc("prs/45_description.md"), "implemented").await
        && await_status(&client, &doc("prs/48_description.md"), "archived").await;
    let after_sweep = statuses(&[
        ("pr44", doc("prs/44_description.md").as_path()),
        ("pr45", doc("prs/45_description.md").as_path()),
        ("pr46", doc("prs/46_description.md").as_path()),
        ("pr47", doc("prs/47_description.md").as_path()),
        ("pr48", doc("prs/48_description.md").as_path()),
        ("pr7", other_record.as_path()),
    ]);
    j.guard(
        "merged → implemented (incl. the skill's `merged`), closed → archived (incl. `closed`); \
         an open PR, a number with no PR, and #7 stay",
        swept
            && after_sweep
                == json!({
                    "pr44": "implemented",
                    "pr45": "implemented",
                    "pr46": "active",
                    "pr47": "draft",
                    "pr48": "archived",
                    "pr7": "active",
                }),
        after_sweep,
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
