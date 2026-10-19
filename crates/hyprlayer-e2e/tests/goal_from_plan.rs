//! Goals derived from a plan, a phase, a range, the composer and a research question, and the
//! two checks that read them back: PlanBoxes against the live plan, and the evaluator one-shot.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckKind, GoalCheckOutcome, GoalDetail, GoalLimits, GoalSource, GoalSpec,
    GoalStatus,
};
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::runtime::providers::ValidationSummary;
use hyprlayer_core::{Harness, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{clear, control, GOAL_CLAUDE};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;

const GOAL_WAIT: Duration = Duration::from_secs(90);
const COUNTER_COMMAND: &str = r#"test "$(cat counter.txt)" -ge 3"#;
const REPO_COMMAND: &str = "test -f counter.txt";
const QUESTION: &str = "Where does the counter live?";

const PLAN: &str = "---\ntitle: Counter plan\ntype: plan\nstatus: active\nauthor: e2e\n---\n\n\
# Counter plan\n\n## Phase 1: Count\n\n### Changes Required\n\nRaise the counter.\n\n\
```md\n- [ ] a box inside a fence is not a criterion\n```\n\n\
### Success Criteria\n\n#### Automated Verification:\n\
- [ ] Counter reaches 3: `test \"$(cat counter.txt)\" -ge 3`\n\n\
#### Manual Verification:\n- [ ] The counter reads well\n\n\
## Phase 2: Keep\n\n### Success Criteria\n\n#### Automated Verification:\n\
- [ ] Counter file exists\n\n#### Manual Verification:\n- [ ] Nobody minds the file\n";

const RESEARCH: &str = "---\ntitle: Counter research\ntype: research\nstatus: draft\n---\n\n\
# Counter research\n\n## Research Question\n\nWhere does the counter live?\n\n## Summary\n\nTBD.\n";

const SUMMARY: &str = r#"{"gate":"PASS","checks":[{"text":"cargo test","outcome":"pass"}],"issues":["Fix the counter"],"notes":["Pre-existing lint"],"manual":["Look at it"]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_from_plan() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# demo\n"),
            (
                ".hyprlayer/settings.toml",
                "[goals]\nchecks = [\"test -f counter.txt\"]\n",
            ),
        ],
    );
    let mut j = Journey::new("goal_from_plan", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Goal from plan").await;
    let ws = text(&view, "/item/id");

    let shared = shared_thoughts(&mut j, &c, &project).await;
    let plan = shared.join("plans/counter.md");
    let research = shared.join("research/counter.md");
    for (path, content) in [(&plan, PLAN), (&research, RESEARCH)] {
        std::fs::create_dir_all(path.parent().expect("a doc dir")).expect("create a doc dir");
        std::fs::write(path, content).expect("write a doc");
    }
    let plan_path = plan.display().to_string();
    let research_path = research.display().to_string();

    j.step("derive a goal from the whole plan");
    let whole = derive(
        &mut j,
        &c,
        json!({ "workstreamId": ws, "source": "plan", "planPath": plan_path, "phases": [] }),
    )
    .await;
    let counter_check = whole.checks.first().cloned();
    j.guard(
        "the work prompt implements every remaining phase",
        whole.work_prompt
            == format!("/implement_plan {plan_path} all remaining phases consecutively"),
        json!({ "workPrompt": whole.work_prompt }),
    );
    j.guard(
        "the source is the whole plan",
        whole.source == GoalSource::Plan
            && whole.source_ref.as_deref() == Some(plan_path.as_str())
            && whole.source_phase.is_none(),
        json!({ "source": whole.source.as_str(), "sourcePhase": whole.source_phase }),
    );
    j.guard(
        "one command from the criterion's last span, then the plan's boxes, then the evaluator",
        kinds(&whole) == ["command", "plan_boxes", "evaluator"]
            && matches!(&counter_check, Some(GoalCheck::Command { command, timeout_secs: 600, label: Some(label) })
                if command == COUNTER_COMMAND && label.starts_with("Counter reaches 3") && !label.contains('`'))
            && whole.checks.get(1)
                == Some(&GoalCheck::PlanBoxes {
                    plan_path: plan_path.clone(),
                    phase: None,
                }),
        json!({ "checks": whole.checks }),
    );
    j.guard(
        "the condition names the plan and carries the prose criterion",
        whole.condition.contains("Counter plan") && whole.condition.contains("Counter file exists"),
        json!({ "condition": whole.condition }),
    );
    let spec_text = serde_json::to_string(&whole).unwrap_or_default();
    j.guard(
        "manual criteria, the fenced box and the repo default stay out of a plan goal",
        !spec_text.contains("reads well")
            && !spec_text.contains("inside a fence")
            && !spec_text.contains(REPO_COMMAND),
        json!({
            "manual": spec_text.contains("reads well"),
            "fenced": spec_text.contains("inside a fence"),
            "repoDefault": spec_text.contains(REPO_COMMAND),
        }),
    );
    j.guard(
        "the limits are Settings' defaults for Claude",
        whole.limits
            == GoalLimits {
                max_iterations: 10,
                max_cost_usd: Some(10.0),
                max_tokens: None,
                max_active_secs: 3600,
            },
        json!({ "limits": whole.limits }),
    );

    j.step("derive from one phase");
    let phase = derive(
        &mut j,
        &c,
        json!({ "workstreamId": ws, "source": "plan_phase", "planPath": plan_path, "phases": ["2"] }),
    )
    .await;
    j.guard(
        "the work prompt names the phase",
        phase.work_prompt == format!("/implement_plan {plan_path} phase 2")
            && phase.source == GoalSource::PlanPhase
            && phase.source_phase.as_deref() == Some("2"),
        json!({
            "workPrompt": phase.work_prompt,
            "source": phase.source.as_str(),
            "sourcePhase": phase.source_phase,
        }),
    );
    j.guard(
        "only phase 2's criteria: its boxes and the evaluator, no phase 1 command",
        phase.checks
            == [
                GoalCheck::PlanBoxes {
                    plan_path: plan_path.clone(),
                    phase: Some("2".into()),
                },
                GoalCheck::Evaluator { model: None },
            ],
        json!({ "checks": phase.checks }),
    );

    j.step("derive a phase range");
    let range = derive(
        &mut j,
        &c,
        json!({ "workstreamId": ws, "source": "plan_phase", "planPath": plan_path, "phases": ["1", "2"] }),
    )
    .await;
    let boxes: Vec<Option<&str>> = range
        .checks
        .iter()
        .filter_map(|check| match check {
            GoalCheck::PlanBoxes { phase, .. } => Some(phase.as_deref()),
            _ => None,
        })
        .collect();
    j.guard(
        "the work prompt spans the range, with one boxes check per phase",
        range.work_prompt == format!("/implement_plan {plan_path} phases 1-2 consecutively")
            && range.source_phase.as_deref() == Some("1-2")
            && boxes == [Some("1"), Some("2")]
            && kinds(&range) == ["command", "plan_boxes", "plan_boxes", "evaluator"],
        json!({
            "workPrompt": range.work_prompt,
            "sourcePhase": range.source_phase,
            "checks": range.checks,
        }),
    );

    j.step("repo default checks prefill");
    let composer = derive(
        &mut j,
        &c,
        json!({ "workstreamId": ws, "source": "composer", "text": "Make the counter file.", "phases": [] }),
    )
    .await;
    j.guard(
        "the composer text is both condition and work prompt",
        composer.condition == "Make the counter file."
            && composer.work_prompt == "Make the counter file."
            && composer.source == GoalSource::Composer,
        json!({ "condition": composer.condition, "workPrompt": composer.work_prompt }),
    );
    j.guard(
        "the repo's [goals] command is proposed, plus the evaluator",
        kinds(&composer) == ["command", "evaluator"]
            && matches!(composer.checks.first(), Some(GoalCheck::Command { command, .. }) if command == REPO_COMMAND),
        json!({ "checks": composer.checks }),
    );

    j.step("research question source");
    let asked = derive(
        &mut j,
        &c,
        json!({ "workstreamId": ws, "source": "research", "researchPath": research_path, "phases": [] }),
    )
    .await;
    j.guard(
        "the question comes from its Research Question section",
        asked.work_prompt == format!("/research_codebase {QUESTION}")
            && asked.condition == format!("A research document answers: {QUESTION}")
            && asked.source == GoalSource::Research,
        json!({ "workPrompt": asked.work_prompt, "condition": asked.condition }),
    );
    j.guard(
        "the evaluator is its only check",
        asked.checks == [GoalCheck::Evaluator { model: None }],
        json!({ "checks": asked.checks }),
    );

    j.step("boxes and the evaluator gate met");
    control(&sandbox, "tick-plan", &format!("{plan_path}\n4\n"));
    let goal = start_new(
        &mut j,
        &c,
        &ws,
        Stage::Implement,
        &plan_path,
        whole.clone(),
        "PLAN",
    )
    .await;
    let rewritten = PLAN.replace(COUNTER_COMMAND, "false");
    std::fs::write(&plan, &rewritten).expect("rewrite the plan's command");
    let detail = wait_goal(&c, &goal.id, |d| d.goal.status != GoalStatus::Running).await;
    let outcomes: Vec<Option<&str>> = detail
        .iterations
        .iter()
        .map(|i| i.outcome.map(GoalCheckOutcome::as_str))
        .collect();
    j.guard(
        "the counter fails twice, the open boxes once, then everything passes and the goal is met",
        detail.goal.status == GoalStatus::Met
            && outcomes == [Some("fail"), Some("fail"), Some("fail"), Some("pass")],
        json!({ "status": detail.goal.status.as_str(), "outcomes": outcomes }),
    );
    let third = result_of(&detail, 3, GoalCheckKind::PlanBoxes);
    let third_command = result_of(&detail, 3, GoalCheckKind::Command);
    j.guard(
        "the third iteration's command passes and its boxes are still open",
        third.as_ref().and_then(|r| r.reason.as_deref()) == Some("2 of 2 automated boxes open")
            && third_command.as_ref().map(|r| r.outcome) == Some(GoalCheckOutcome::Pass),
        json!({
            "boxes": third.as_ref().map(|r| (r.outcome.as_str(), r.reason.clone())),
            "command": third_command.as_ref().map(|r| r.outcome.as_str()),
        }),
    );
    j.guard(
        "the goal ran its snapshot, not the plan's rewritten `false`",
        matches!(detail.goal.checks.first(), Some(GoalCheck::Command { command, .. }) if command == COUNTER_COMMAND),
        json!({ "checks": detail.goal.checks }),
    );
    let evaluated = result_of(&detail, 4, GoalCheckKind::Evaluator);
    let prompts = evaluator_prompts(&sandbox);
    j.guard(
        "the evaluator runs once, after the boxes pass, and sees the condition",
        evaluated.as_ref().map(|r| r.outcome) == Some(GoalCheckOutcome::Pass)
            && prompts.len() == 1
            && prompts
                .first()
                .is_some_and(|p| p.contains(&whole.condition)),
        json!({
            "evaluator": evaluated.as_ref().map(|r| r.outcome.as_str()),
            "prompts": prompts.len(),
            "seesCondition": prompts.first().is_some_and(|p| p.contains(&whole.condition)),
        }),
    );
    let spent_cents = (detail.goal.spent_cost_usd * 100.0).round() as i64;
    j.guard(
        "spend counts four turns and the evaluator's cent",
        spent_cents == 161,
        json!({ "spentCents": spent_cents }),
    );
    clear(&sandbox, "tick-plan");

    j.step("an unparseable evaluator reply is an error");
    control(&sandbox, "evaluator", "not json");
    let judged = start_new(
        &mut j,
        &c,
        &ws,
        Stage::General,
        &plan_path,
        evaluator_spec(),
        "JUDGED",
    )
    .await;
    let detail = wait_goal(&c, &judged.id, |d| d.goal.status != GoalStatus::Running).await;
    let reply = result_of(&detail, 1, GoalCheckKind::Evaluator);
    j.guard(
        "the evaluator's result is an error naming the reply",
        detail.iterations.first().and_then(|i| i.outcome) == Some(GoalCheckOutcome::Error)
            && reply.as_ref().map(|r| r.outcome) == Some(GoalCheckOutcome::Error)
            && reply
                .as_ref()
                .and_then(|r| r.reason.as_deref())
                .is_some_and(|reason| reason.contains("not json")),
        json!({ "evaluator": reply.as_ref().map(|r| (r.outcome.as_str(), r.reason.clone())) }),
    );
    clear(&sandbox, "evaluator");

    j.step("the validation summary rides the one-shot");
    control(&sandbox, "validation-summary", SUMMARY);
    let summarized = j
        .call(
            &c,
            Request::SummarizeValidationReport {
                report: "## Validation Report\n\nGate: PASS\n".into(),
            },
        )
        .await
        .expect("summarize_validation_report");
    let summary: Option<ValidationSummary> =
        serde_json::from_value(summarized.clone()).expect("a summary or null");
    let expected: Value = serde_json::from_str(SUMMARY).expect("the fixture summary");
    j.guard(
        "the summary is the stub's JSON, parsed",
        summary.is_some() && summarized == expected,
        json!({ "summary": summarized }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

/// `derive_goal` sent as its wire JSON, every field present; the server's typed request is the contract under test.
async fn derive(j: &mut Journey, client: &RpcClient, fields: Value) -> GoalSpec {
    let mut request = json!({
        "harness": null, "text": null, "planPath": null, "phases": [], "researchPath": null, "findings": [],
    });
    if let (Some(all), Value::Object(given)) = (request.as_object_mut(), fields) {
        all.extend(given);
    }
    let request: Request = serde_json::from_value(json!({ "derive_goal": { "request": request } }))
        .expect("derive_goal is a command");
    let spec = j.call(client, request).await.expect("derive_goal");
    serde_json::from_value(spec).expect("derive_goal answers a spec")
}

fn kinds(spec: &GoalSpec) -> Vec<&'static str> {
    spec.checks
        .iter()
        .map(|check| match check {
            GoalCheck::Command { .. } => "command",
            GoalCheck::PlanBoxes { .. } => "plan_boxes",
            GoalCheck::Evaluator { .. } => "evaluator",
            GoalCheck::Validation { .. } => "validation",
        })
        .collect()
}

fn evaluator_spec() -> GoalSpec {
    GoalSpec {
        condition: "E2E_EVALUATOR_REPLY".into(),
        work_prompt: "Raise the counter.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![
            GoalCheck::Command {
                command: "true".into(),
                timeout_secs: 60,
                label: None,
            },
            GoalCheck::Evaluator { model: None },
        ],
        limits: GoalLimits {
            max_iterations: 1,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 3600,
        },
    }
}

async fn start_new(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    stage: Stage,
    plan_path: &str,
    spec: GoalSpec,
    alias: &str,
) -> Goal {
    let attached_docs = match stage {
        Stage::Implement => vec![plan_path.to_string()],
        _ => vec![],
    };
    let started = j
        .call(
            client,
            Request::StartGoal {
                request: StartGoalRequest {
                    target: GoalTarget::NewSession {
                        workstream_id: ws.into(),
                        stage,
                        harness: Some(Harness::Claude),
                        model: None,
                        effort: None,
                        permission_mode: None,
                        attached_docs,
                    },
                    spec,
                },
            },
        )
        .await
        .expect("start_goal");
    let goal: Goal = serde_json::from_value(started).expect("start_goal answers a goal");
    j.alias(&goal.id, format!("${alias}_GOAL"));
    j.alias(&goal.session_id, format!("${alias}_SESSION"));
    goal
}

fn result_of(
    detail: &GoalDetail,
    n: u32,
    kind: GoalCheckKind,
) -> Option<hyprlayer_core::models::goal::GoalCheckResult> {
    detail
        .iterations
        .iter()
        .find(|i| i.n == n)
        .and_then(|i| i.results.iter().find(|r| r.kind == kind).cloned())
}

fn evaluator_prompts(sandbox: &Sandbox) -> Vec<String> {
    std::fs::read_to_string(sandbox.root.join("goal-control/evaluator.log"))
        .unwrap_or_default()
        .split("\n---\n")
        .filter(|p| !p.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The project's shared thoughts dir, found through a seed note.
async fn shared_thoughts(j: &mut Journey, client: &RpcClient, project: &str) -> PathBuf {
    let seed = j
        .call(
            client,
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
            client,
            Request::ResolveDocPath {
                id: text(&seed, "/id"),
            },
        )
        .await
        .expect("resolve_doc_path");
    let shared = Path::new(&text(&resolved, ""))
        .ancestors()
        .nth(2)
        .expect("<shared>/notes/<file>")
        .to_path_buf();
    j.alias(shared.display().to_string(), "$SHARED");
    shared
}

/// Polls until `done` holds or the wait runs out; the last reading is returned either way.
async fn wait_goal(
    client: &RpcClient,
    goal_id: &str,
    done: impl Fn(&GoalDetail) -> bool,
) -> GoalDetail {
    let deadline = tokio::time::Instant::now() + GOAL_WAIT;
    loop {
        let detail = client
            .get_goal(goal_id.to_string())
            .await
            .expect("get_goal");
        if done(&detail) || tokio::time::Instant::now() >= deadline {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
