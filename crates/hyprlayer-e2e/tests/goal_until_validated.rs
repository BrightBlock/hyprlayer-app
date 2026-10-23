//! Implement until validated: a goal whose last check opens a fresh validate session each
//! iteration, reads its report, summarizes it, and feeds the findings back.
//!
//! The harness is `goal_stub`'s claude stand-in; nothing here covers the real binary.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::goal::{
    Goal, GoalCheck, GoalCheckKind, GoalCheckOutcome, GoalCheckResult, GoalDetail, GoalEndReason,
    GoalSpec, GoalStatus, GoalStep,
};
use hyprlayer_core::models::session::SessionState;
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{clear, control, control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;

const GOAL_WAIT: Duration = Duration::from_secs(120);
const IO: Duration = Duration::from_secs(30);
const PLAIN_COMMAND: &str = "test -f counter.txt";
const SHELL_COMMAND: &str = r#"test "$(cat counter.txt)" -ge 1"#;
const APP_COMMAND: &str = "cd app && test -f marker.txt";
const ISSUE: &str = "render() ignores the theme";
const FIX_ALL: &str = "Resolve these findings from the plan validation.";

const PLAN: &str = "---\ntitle: Theme plan\ntype: plan\nstatus: active\nauthor: e2e\n---\n\n\
# Theme plan\n\n## Phase 1: Theme\n\n### Changes Required\n\nRender with the theme.\n\n\
### Success Criteria\n\n#### Automated Verification:\n\
- [ ] Counter file exists: `test -f counter.txt`\n\
- [ ] Counter is positive: `test \"$(cat counter.txt)\" -ge 1`\n\n\
#### Automated Verification (from `app/`):\n\
- [ ] App marker: `test -f marker.txt`\n\n\
#### Manual Verification:\n- [ ] The theme looks right\n";

const REPORT_ISSUE: &str = "## Validation Report: Theme plan\n\n\
### Automated Verification Results\n- Passed: test -f counter.txt\n\n\
### Potential Issues\n- render() ignores the theme\n";
const REPORT_CLEAN: &str = "## Validation Report: Theme plan\n\n\
### Automated Verification Results\n- Passed: test -f counter.txt\n\n\
### Potential Issues\n- None\n";
const REPORT_FAILED_CHECK: &str = "## Validation Report: Theme plan\n\n\
### Automated Verification Results\n- Failed: widget tests\n";

const SUMMARY_ISSUE: &str = r#"{"gate":null,"checks":[{"text":"test -f counter.txt","outcome":"pass"}],"issues":["render() ignores the theme"],"notes":[],"manual":["The theme looks right"]}"#;
const SUMMARY_CLEAN: &str = r#"{"gate":"PASS","checks":[{"text":"test -f counter.txt","outcome":"pass"}],"issues":[],"notes":[],"manual":[]}"#;
const SUMMARY_NOTES: &str = r#"{"gate":null,"checks":[{"text":"widget tests","outcome":"fail"}],"issues":[],"notes":["widget tests fail on main too"],"manual":[]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn goal_until_validated() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo(
        "demo",
        &[("README.md", "# demo\n"), ("app/marker.txt", "here\n")],
    );
    let mut j = Journey::new("goal_until_validated", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Until validated").await;
    let ws = text(&view, "/item/id");

    let plan = shared_thoughts(&mut j, &c, &project)
        .await
        .join("plans/theme.md");
    std::fs::create_dir_all(plan.parent().expect("a plans dir")).expect("create the plans dir");
    std::fs::write(&plan, PLAN).expect("write the plan");
    let plan_path = plan.display().to_string();
    control(&sandbox, "tick-plan", &format!("{plan_path}\n1\n"));
    let derived = derive(&mut j, &c, &ws, &plan_path).await;
    let mut seen = BTreeSet::new();

    j.step("validation findings feed the next iteration");
    control(&sandbox, "validation-report", REPORT_ISSUE);
    control(&sandbox, "validation-summary", SUMMARY_ISSUE);
    let first = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 3),
        "FIRST",
    )
    .await;
    // Iteration 2 waits for `release`, so the summary changes before its validation runs.
    control_for(&sandbox, &first.session_id, "hold", "2");
    let checked = wait_goal(&c, &first.id, |d| {
        d.iterations.first().is_some_and(|i| i.outcome.is_some())
    })
    .await;
    let found = result_of(&checked, 1, GoalCheckKind::Validation);
    j.guard(
        "iteration 1's validation fails on the summarized issue",
        checked.iterations.first().and_then(|i| i.outcome) == Some(GoalCheckOutcome::Fail)
            && found.as_ref().map(|r| r.outcome) == Some(GoalCheckOutcome::Fail)
            && found.as_ref().is_some_and(|r| r.output_tail.contains(ISSUE)),
        json!({
            "outcome1": checked.iterations.first().and_then(|i| i.outcome).map(GoalCheckOutcome::as_str),
            "validation": found.as_ref().map(|r| (r.outcome.as_str(), r.reason.clone())),
            "carriesIssue": found.as_ref().is_some_and(|r| r.output_tail.contains(ISSUE)),
        }),
    );
    control(&sandbox, "validation-report", REPORT_CLEAN);
    control(&sandbox, "validation-summary", SUMMARY_CLEAN);
    control(&sandbox, "release", "");
    let met = wait_goal(&c, &first.id, |d| d.goal.status != GoalStatus::Running).await;
    let second_prompt = user_lines(&sandbox, &first.session_id).get(1).cloned();
    j.guard(
        "iteration 2's prompt carries the Fix-all text and the issue",
        second_prompt
            .as_deref()
            .is_some_and(|p| p.contains(FIX_ALL) && p.contains(&format!("- {ISSUE}"))),
        json!({
            "fixAll": second_prompt.as_deref().is_some_and(|p| p.contains(FIX_ALL)),
            "issue": second_prompt.as_deref().is_some_and(|p| p.contains(&format!("- {ISSUE}"))),
        }),
    );

    j.step("met after a clean validation");
    j.guard(
        "the goal is met at iteration 2",
        met.goal.status == GoalStatus::Met
            && met.goal.end_reason == Some(GoalEndReason::ChecksPassed)
            && met.goal.iteration == 2,
        json!({
            "status": met.goal.status.as_str(),
            "endReason": met.goal.end_reason.map(GoalEndReason::as_str),
            "iteration": met.goal.iteration,
        }),
    );
    let first_validations = validate_sessions(&c, &ws, &seen).await;
    seen.extend(first_validations.iter().map(|(id, _)| id.clone()));
    let named: BTreeSet<String> = met
        .iterations
        .iter()
        .filter_map(|i| validation_of(i.results.as_slice()))
        .filter_map(|r| r.session_id.clone())
        .collect();
    let opened: BTreeSet<String> = first_validations.iter().map(|(id, _)| id.clone()).collect();
    j.guard(
        "two validate sessions, each killed after its report and named on its result",
        first_validations.len() == 2
            && first_validations
                .iter()
                .all(|(_, state)| *state == SessionState::Killed)
            && named == opened,
        json!({
            "sessions": first_validations.len(),
            "states": first_validations.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            "namedOnResults": named == opened,
        }),
    );
    let described = any_stdin_contains(&sandbox, "/describe_pr");
    let status_kept =
        std::fs::read_to_string(&plan).is_ok_and(|content| content.contains("\nstatus: active\n"));
    j.guard(
        "no /describe_pr was sent and the plan's status is unchanged",
        !described && status_kept,
        json!({ "describePr": described, "statusActive": status_kept }),
    );
    let spent_cents = (met.goal.spent_cost_usd * 100.0).round() as i64;
    j.guard(
        "spend counts both turns, both validate sessions and both summaries",
        spent_cents == 162,
        json!({ "spentCents": spent_cents }),
    );

    j.step("approved commands are allowed in the validate session");
    let shapes: Vec<Value> = first_validations
        .iter()
        .map(|(id, _)| {
            let argv = std::fs::read_to_string(control_path(&sandbox, id).join("argv.log"))
                .unwrap_or_default();
            let prompt = user_lines(&sandbox, id)
                .into_iter()
                .next()
                .unwrap_or_default();
            json!({
                "plainRule": argv.contains(&format!("Bash({PLAIN_COMMAND})")),
                "shellRule": argv.contains("-ge 1"),
                "appRule": argv.contains("Bash(cd app)") && argv.contains("Bash(test -f marker.txt)"),
                "opensWithSkill": prompt.starts_with("/validate_plan"),
                "notesPlain": prompt.contains(PLAIN_COMMAND),
                "notesShell": prompt.contains(SHELL_COMMAND),
                "notesApp": prompt.contains(APP_COMMAND),
                "doNotRerun": prompt.contains("do not run them again"),
            })
        })
        .collect();
    j.guard(
        "each validate session allows the plain commands only, and its prompt names all three, the app one with its cd",
        shapes.len() == 2
            && shapes.iter().all(|s| {
                s["plainRule"] == true
                    && s["shellRule"] == false
                    && s["appRule"] == true
                    && s["opensWithSkill"] == true
                    && s["notesPlain"] == true
                    && s["notesShell"] == true
                    && s["notesApp"] == true
                    && s["doNotRerun"] == true
            }),
        json!({ "sessions": shapes }),
    );
    let report_only: Vec<Value> = first_validations
        .iter()
        .map(|(id, _)| {
            let argv = std::fs::read_to_string(control_path(&sandbox, id).join("argv.log"))
                .unwrap_or_default();
            let denied = flag_value(&argv, "--disallowedTools");
            let allowed = flag_value(&argv, "--allowedTools");
            let prompt = user_lines(&sandbox, id)
                .into_iter()
                .next()
                .unwrap_or_default();
            json!({
                "deniesEdit": denied.split_whitespace().any(|rule| rule == "Edit"),
                "deniesSync": denied.contains("Bash(hyprlayer thoughts sync:*)"),
                "allowsThoughtsEdit": allowed.contains("Edit(//"),
                "toldReportOnly": prompt.contains("report-only"),
            })
        })
        .collect();
    j.guard(
        "each validate session is report-only: no edits, no thoughts sync, and told so",
        report_only.len() == 2
            && report_only.iter().all(|s| {
                s["deniesEdit"] == true
                    && s["deniesSync"] == true
                    && s["allowsThoughtsEdit"] == false
                    && s["toldReportOnly"] == true
            }),
        json!({ "sessions": report_only }),
    );

    j.step("a validate ask waits");
    control_for(&sandbox, "validate", "ask", "");
    let asking = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 1),
        "ASK",
    )
    .await;
    let waiting = wait_goal(&c, &asking.id, |d| d.goal.step == Some(GoalStep::Waiting)).await;
    let asked = validate_sessions(&c, &ws, &seen).await;
    let request_id = match asked.first() {
        Some((id, _)) => pending_permission(&c, id).await,
        None => None,
    };
    j.guard(
        "the validate session's ask moves the goal to waiting during its checks",
        waiting.goal.step == Some(GoalStep::Waiting)
            && waiting.goal.iteration == 1
            && asked.len() == 1
            && request_id.is_some(),
        json!({
            "step": waiting.goal.step.map(GoalStep::as_str),
            "iteration": waiting.goal.iteration,
            "validateSessions": asked.len(),
            "asked": request_id.is_some(),
        }),
    );
    clear(&sandbox, "validate/ask");
    if let (Some((vid, _)), Some(request_id)) = (asked.first(), request_id) {
        j.call(
            &c,
            Request::RespondPermission {
                session_id: vid.clone(),
                request_id,
                allow: true,
                message: None,
            },
        )
        .await
        .expect("respond_permission");
    }
    let answered = wait_goal(&c, &asking.id, |d| d.goal.status != GoalStatus::Running).await;
    seen.extend(asked.into_iter().map(|(id, _)| id));
    j.guard(
        "answering it lets the validation finish and the goal meet",
        answered.goal.status == GoalStatus::Met,
        json!({ "status": answered.goal.status.as_str() }),
    );

    j.step("a cancelled validation kills its session");
    control_for(&sandbox, "validate", "hold", "");
    let cancel = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 1),
        "CANCEL",
    )
    .await;
    let mut held = None;
    let deadline = tokio::time::Instant::now() + IO;
    while held.is_none() && tokio::time::Instant::now() < deadline {
        held = validate_sessions(&c, &ws, &seen)
            .await
            .into_iter()
            .map(|(id, _)| id)
            .find(|id| control_path(&sandbox, id).join("turns").exists());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let held = held.expect("a validate session holds its turn");
    j.call(
        &c,
        Request::SendUserMessage {
            session_id: cancel.session_id.clone(),
            text: "E2E_USER_DURING_VALIDATION".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    let mut killed = false;
    let deadline = tokio::time::Instant::now() + IO;
    while !killed && tokio::time::Instant::now() < deadline {
        killed = session_state(&c, &ws, &held).await == Some(SessionState::Killed);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    clear(&sandbox, "validate/hold");
    control(&sandbox, "release", "");
    let rerun = wait_goal(&c, &cancel.id, |d| d.goal.status != GoalStatus::Running).await;
    clear(&sandbox, "release");
    let reran = validate_sessions(&c, &ws, &seen).await;
    seen.extend(reran.iter().map(|(id, _)| id.clone()));
    let judged_by = result_of(&rerun, 1, GoalCheckKind::Validation).and_then(|r| r.session_id);
    j.guard(
        "the user's message kills the held validate session",
        killed,
        json!({ "killed": killed }),
    );
    j.guard(
        "the re-run opens a new validate session, whose report meets the goal",
        rerun.goal.status == GoalStatus::Met
            && reran.len() == 2
            && judged_by.as_ref().is_some_and(|id| *id != held),
        json!({
            "status": rerun.goal.status.as_str(),
            "validateSessions": reran.len(),
            "judgedByNewSession": judged_by.as_ref().is_some_and(|id| *id != held),
        }),
    );

    j.step("a failed summary falls back to the bare parse");
    control(&sandbox, "validation-report", REPORT_FAILED_CHECK);
    control(&sandbox, "validation-summary", "not json");
    let bare = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 2),
        "BARE",
    )
    .await;
    let ended = wait_goal(&c, &bare.id, |d| d.goal.status != GoalStatus::Running).await;
    let failed = result_of(&ended, 1, GoalCheckKind::Validation);
    let next = user_lines(&sandbox, &bare.session_id).get(1).cloned();
    j.guard(
        "the bare parse's failing check fails the validation",
        failed.as_ref().map(|r| r.outcome) == Some(GoalCheckOutcome::Fail)
            && ended.goal.end_reason == Some(GoalEndReason::IterationLimit),
        json!({
            "validation": failed.as_ref().map(|r| (r.outcome.as_str(), r.reason.clone())),
            "endReason": ended.goal.end_reason.map(GoalEndReason::as_str),
        }),
    );
    j.guard(
        "the next prompt carries the failed check",
        next.as_deref()
            .is_some_and(|p| p.contains(FIX_ALL) && p.contains("- Failed check: widget tests")),
        json!({ "failedCheck": next.as_deref().is_some_and(|p| p.contains("- Failed check: widget tests")) }),
    );
    seen.extend(
        validate_sessions(&c, &ws, &seen)
            .await
            .into_iter()
            .map(|(id, _)| id),
    );

    j.step("a pre-existing failure in notes does not block met");
    control(&sandbox, "validation-summary", SUMMARY_NOTES);
    let noted = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 1),
        "NOTES",
    )
    .await;
    let ended = wait_goal(&c, &noted.id, |d| d.goal.status != GoalStatus::Running).await;
    j.guard(
        "the summarized failure filed under notes leaves the goal met",
        ended.goal.status == GoalStatus::Met,
        json!({ "status": ended.goal.status.as_str() }),
    );
    seen.extend(
        validate_sessions(&c, &ws, &seen)
            .await
            .into_iter()
            .map(|(id, _)| id),
    );

    j.step("a missing report is an error");
    control(&sandbox, "validation-report", "");
    let silent = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 1),
        "SILENT",
    )
    .await;
    let ended = wait_goal(&c, &silent.id, |d| d.goal.status != GoalStatus::Running).await;
    let missing = result_of(&ended, 1, GoalCheckKind::Validation);
    j.guard(
        "the validation is an error saying it ended without a report",
        missing.as_ref().map(|r| r.outcome) == Some(GoalCheckOutcome::Error)
            && missing
                .as_ref()
                .and_then(|r| r.reason.as_deref())
                .is_some_and(|reason| reason.contains("ended without a report")),
        json!({ "validation": missing.as_ref().map(|r| (r.outcome.as_str(), r.reason.clone())) }),
    );

    j.step("a validation cancelled during its summary still counts its session's spend");
    control(&sandbox, "validation-report", REPORT_CLEAN);
    control(&sandbox, "validation-summary", SUMMARY_CLEAN);
    control(&sandbox, "hold-summary", "");
    let summarizing = start(
        &mut j,
        &c,
        &ws,
        &plan_path,
        validated(&derived, &plan_path, 1),
        "SUMMARY_CANCEL",
    )
    .await;
    let held = eventually(IO, || control_path(&sandbox, "summary-held").exists()).await;
    j.call(
        &c,
        Request::SendUserMessage {
            session_id: summarizing.session_id.clone(),
            text: "E2E_USER_DURING_SUMMARY".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    clear(&sandbox, "hold-summary");
    control(&sandbox, "release", "");
    let ended = wait_goal(&c, &summarizing.id, |d| {
        d.goal.status != GoalStatus::Running
    })
    .await;
    clear(&sandbox, "release");
    clear(&sandbox, "summary-held");
    let spent_cents = (ended.goal.spent_cost_usd * 100.0).round() as i64;
    j.guard(
        "spend counts the turn, both validate sessions and the one finished summary",
        held && ended.goal.status == GoalStatus::Met && spent_cents == 121,
        json!({
            "summaryHeld": held,
            "status": ended.goal.status.as_str(),
            "spentCents": spent_cents,
        }),
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

/// `derive_goal` sent as its wire JSON, the whole plan.
async fn derive(j: &mut Journey, client: &RpcClient, ws: &str, plan_path: &str) -> GoalSpec {
    let request = json!({
        "workstreamId": ws, "source": "plan", "harness": null, "text": null, "planPath": plan_path,
        "phases": [], "researchPath": null, "findings": [],
    });
    let request: Request = serde_json::from_value(json!({ "derive_goal": { "request": request } }))
        .expect("derive_goal is a command");
    let spec = j.call(client, request).await.expect("derive_goal");
    serde_json::from_value(spec).expect("derive_goal answers a spec")
}

/// The derived spec plus a Validation check, as the validate card's entry point sends it.
fn validated(derived: &GoalSpec, plan_path: &str, max_iterations: u32) -> GoalSpec {
    let mut spec = derived.clone();
    spec.checks.push(GoalCheck::Validation {
        plan_path: plan_path.to_string(),
    });
    spec.limits.max_iterations = max_iterations;
    spec
}

async fn start(
    j: &mut Journey,
    client: &RpcClient,
    ws: &str,
    plan_path: &str,
    spec: GoalSpec,
    alias: &str,
) -> Goal {
    let started = j
        .call(
            client,
            Request::StartGoal {
                request: StartGoalRequest {
                    target: GoalTarget::NewSession {
                        workstream_id: ws.into(),
                        stage: Stage::Implement,
                        harness: Some(Harness::Claude),
                        model: None,
                        effort: None,
                        permission_mode: None,
                        attached_docs: vec![plan_path.to_string()],
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

fn validation_of(results: &[GoalCheckResult]) -> Option<&GoalCheckResult> {
    results.iter().find(|r| r.kind == GoalCheckKind::Validation)
}

fn result_of(detail: &GoalDetail, n: u32, kind: GoalCheckKind) -> Option<GoalCheckResult> {
    detail
        .iterations
        .iter()
        .find(|i| i.n == n)
        .and_then(|i| i.results.iter().find(|r| r.kind == kind).cloned())
}

/// The workstream's validate sessions not in `seen`, oldest first.
async fn validate_sessions(
    client: &RpcClient,
    ws: &str,
    seen: &BTreeSet<String>,
) -> Vec<(String, SessionState)> {
    let mut sessions: Vec<_> = client
        .list_sessions(ws.to_string())
        .await
        .expect("list_sessions")
        .into_iter()
        .filter(|s| s.stage == Stage::Validate && !seen.contains(&s.id))
        .collect();
    sessions.sort_by(|a, b| a.started_at.total_cmp(&b.started_at));
    sessions.into_iter().map(|s| (s.id, s.state)).collect()
}

async fn session_state(client: &RpcClient, ws: &str, session_id: &str) -> Option<SessionState> {
    client
        .list_sessions(ws.to_string())
        .await
        .expect("list_sessions")
        .into_iter()
        .find(|s| s.id == session_id)
        .map(|s| s.state)
}

async fn pending_permission(client: &RpcClient, session_id: &str) -> Option<String> {
    let mut found = None;
    let deadline = tokio::time::Instant::now() + IO;
    while found.is_none() && tokio::time::Instant::now() < deadline {
        found = client
            .get_session_log(session_id.to_string())
            .await
            .expect("get_session_log")
            .iter()
            .rev()
            .find_map(|e| match &e.event {
                SessionEvent::PermissionAsk { request_id, .. } => Some(request_id.clone()),
                _ => None,
            });
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    found
}

fn control_path(sandbox: &Sandbox, name: &str) -> PathBuf {
    sandbox.root.join("goal-control").join(name)
}

/// One flag's value in a space-joined launch line; rule values never hold ` --`.
fn flag_value<'a>(argv: &'a str, flag: &str) -> &'a str {
    argv.split_once(&format!("{flag} "))
        .map(|(_, rest)| rest.split(" --").next().unwrap_or_default())
        .unwrap_or_default()
}

/// The text of each user message the stub read from stdin, in order.
fn user_lines(sandbox: &Sandbox, session_id: &str) -> Vec<String> {
    std::fs::read_to_string(control_path(sandbox, session_id).join("stdin.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["type"] == "user")
        .filter_map(|line| line["message"]["content"].as_str().map(str::to_string))
        .collect()
}

fn any_stdin_contains(sandbox: &Sandbox, needle: &str) -> bool {
    let Ok(dirs) = std::fs::read_dir(sandbox.root.join("goal-control")) else {
        return false;
    };
    dirs.flatten().any(|dir| {
        std::fs::read_to_string(dir.path().join("stdin.log")).is_ok_and(|log| log.contains(needle))
    })
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
