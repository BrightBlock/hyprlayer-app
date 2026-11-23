//! App actions run in a PTY agent: a desktop launch carrying a stage's first prompt gets it
//! composed by the server and delivered once the TUI is ready, Claude with its slash command and
//! other engines with the skill named in prose.
//!
//! The engines are `agent_stub`'s PTY claude; nothing here covers the real binaries.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::{PtyFirstPrompt, PtyLaunchSession};
use hyprlayer_core::pipeline;
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::Stage;
use hyprlayer_e2e::agent_stub::{delivered, install, launch_log, lines, set_engine, PTY_CLAUDE};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server};
use hyprlayer_server::client::RpcClient;

const IO: Duration = Duration::from_secs(30);
const PLAN: &str = "thoughts/shared/plans/demo-plan.md";
const DRAFT_PLAN: &str =
    "---\ntitle: Demo\ntype: plan\nstatus: draft\nauthor: e2e\n---\n\n# Demo\n\n## Phase 1\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_action_prompt() {
    let sandbox = Sandbox::new();
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("pty_action_prompt", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Actions").await;
    let ws = text(&view, "/item/id");
    let base = text(&view, "/item/baseBranch");
    let base = base.strip_prefix("origin/").unwrap_or(&base).to_string();
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    set_engine(&mut j, &c, "gemini", &pty_claude, &[]).await;

    j.step("commit in claude opens with its slash command");
    let commit_log = launch(&sandbox, &c, &ws, "claude-code", Stage::Commit, "", &[]).await;
    let expected = delivered(pipeline::COMMIT_PROMPT_DEFAULT);
    let typed = typed_once(&commit_log, &expected).await;
    j.guard(
        "the TUI reads /commit typed and the rest of the prompt as one bracketed paste after it, then Enter",
        typed,
        json!({ "matchesThePaste": typed, "firstLine": first_line(pipeline::COMMIT_PROMPT_DEFAULT) }),
    );

    j.step("ship asks for the description in the same turn");
    let log = launch(&sandbox, &c, &ws, "claude-code", Stage::Ship, "", &[]).await;
    let ship = format!(
        "{}\n\n{}",
        pipeline::PTY_SHIP_PROMPT_TEMPLATE.replace("{{base_branch}}", &base),
        pipeline::PTY_SHIP_DESCRIBE
    );
    let typed = typed_once(&log, &delivered(&ship)).await;
    j.guard(
        "ship's prompt ends by asking for /describe_pr, since no follow-up turn comes",
        typed,
        json!({ "matchesThePaste": typed, "lastLine": pipeline::PTY_SHIP_DESCRIBE }),
    );

    j.step("validate carries the plan as an attached document");
    let log = launch(
        &sandbox,
        &c,
        &ws,
        "claude-code",
        Stage::Validate,
        "",
        &[PLAN],
    )
    .await;
    let validate = format!("/validate_plan\n\nRelevant documents:\n- {PLAN}");
    let typed = typed_once(&log, &delivered(&validate)).await;
    j.guard(
        "the plan rides after the skill, as a native start lists it",
        typed,
        json!({ "matchesThePaste": typed }),
    );

    j.step("implement opens with its skill, accepting edits, and starts the plan");
    let plan = shared_thoughts(&mut j, &c, &project)
        .await
        .join("plans/demo.md");
    std::fs::create_dir_all(plan.parent().expect("plans dir")).expect("create plans dir");
    std::fs::write(&plan, DRAFT_PLAN).expect("write the draft plan");
    let plan_doc = plan.display().to_string();
    let log = launch(
        &sandbox,
        &c,
        &ws,
        "claude-code",
        Stage::Implement,
        "",
        &[plan_doc.as_str()],
    )
    .await;
    // The review-notes line outruns the stub's cooked-mode line limit; claude reads raw.
    let stdin = log.join("stdin.log");
    let opened = eventually(IO, || {
        lines(&stdin)
            .first()
            .is_some_and(|line| line.starts_with("/implement_plan \x1b[200~"))
    })
    .await;
    let argv = lines(&log.join("argv"));
    let accepts_edits = argv
        .windows(2)
        .any(|w| w[0] == "--permission-mode" && w[1] == "acceptEdits");
    let denies_push = argv
        .windows(2)
        .any(|w| w[0] == "--disallowedTools" && w[1] == "Bash(git push:*)");
    j.guard(
        "/implement_plan is typed with the rest pasted as its arguments, in the mode and deny list a native Implement starts with",
        opened && accepts_edits && denies_push,
        json!({ "opensWithTheSkill": opened, "acceptsEdits": accepts_edits, "deniesPush": denies_push }),
    );
    let worktree = text(&view, "/item/worktreePath");
    let activated = std::fs::read_to_string(&plan).is_ok_and(|t| t.contains("status: active"));
    let excluded = review_dir_excluded(Path::new(&worktree));
    j.guard(
        "as a native start does, the draft plan turns active and the review dir is git-excluded",
        activated && excluded,
        json!({ "planActive": activated, "reviewDirExcluded": excluded }),
    );

    j.step("commit's argv keeps the CLI's own mode");
    let commit_argv = lines(&commit_log.join("argv"));
    let moded = commit_argv.iter().any(|a| a == "--permission-mode");
    j.guard(
        "a stage whose native mode is the default passes no --permission-mode",
        !moded,
        json!({ "passesAMode": moded }),
    );

    j.step("revise carries the PR instruction");
    let instruction = "CI checks are failing on PR #7.";
    let log = launch(&sandbox, &c, &ws, "gemini", Stage::Revise, instruction, &[]).await;
    let revise = format!("{}\n\n{instruction}", pipeline::REVISE_PROMPT);
    let typed = typed_once(&log, &delivered(&revise)).await;
    j.guard(
        "the instruction follows the revise prompt, unchanged for an engine without slash skills",
        typed,
        json!({ "matchesThePaste": typed }),
    );

    j.step("another engine gets the skill in prose");
    let log = launch(&sandbox, &c, &ws, "gemini", Stage::Commit, "", &[]).await;
    let prose =
        pipeline::COMMIT_PROMPT_NO_ASK_TOOL.replacen("/commit", "Use the `commit` skill.", 1);
    let typed = typed_once(&log, &delivered(&prose)).await;
    j.guard(
        "no leading /commit for a TUI that would refuse it, and no AskUserQuestion it may lack",
        typed,
        json!({ "matchesThePaste": typed, "firstLine": first_line(&prose) }),
    );

    j.finish();
}

async fn launch(
    sandbox: &Sandbox,
    c: &RpcClient,
    ws: &str,
    engine: &str,
    stage: Stage,
    input: &str,
    docs: &[&str],
) -> PathBuf {
    let (launched, _stream_id, _events) = c
        .pty_launch_stream(
            ws.to_string(),
            engine.to_string(),
            PtyLaunchSession {
                first_prompt: Some(PtyFirstPrompt {
                    stage,
                    input: input.to_string(),
                    attached_docs: docs.iter().map(|d| d.to_string()).collect(),
                }),
                ..Default::default()
            },
            80,
            24,
        )
        .await
        .expect("pty_launch");
    launch_log(sandbox, &launched.terminal_id)
}

/// The project's shared thoughts dir, found through a seed note the server places in it.
async fn shared_thoughts(j: &mut Journey, c: &RpcClient, project: &str) -> PathBuf {
    let seed = j
        .call(
            c,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.to_string()),
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
            c,
            Request::ResolveDocPath {
                id: text(&seed, "/id"),
            },
        )
        .await
        .expect("resolve_doc_path");
    Path::new(&text(&resolved, ""))
        .ancestors()
        .nth(2)
        .expect("<shared>/notes/<file>")
        .to_path_buf()
}

/// Whether the worktree's repo excludes the agent review-notes dir.
fn review_dir_excluded(worktree: &Path) -> bool {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(worktree)
        .output()
        .expect("git rev-parse");
    let common = worktree.join(String::from_utf8_lossy(&out.stdout).trim());
    std::fs::read_to_string(common.join("info/exclude"))
        .is_ok_and(|t| t.contains(".hyprlayer/review/"))
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// True once `stdin.log` holds exactly the paste: nothing before it, and nothing sent twice.
async fn typed_once(log: &std::path::Path, expected: &[String]) -> bool {
    let stdin = log.join("stdin.log");
    eventually(IO, || lines(&stdin) == expected).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    lines(&stdin) == expected
}
