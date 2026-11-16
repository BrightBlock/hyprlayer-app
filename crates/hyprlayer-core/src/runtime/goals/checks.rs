use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::{mpsc, Notify};

use hyprlayer_models::goal::{GoalCheck, GoalCheckKind, GoalCheckOutcome, GoalCheckResult};
use hyprlayer_models::ClientRef;

use crate::agent::{AgentDriver, Harness};
use crate::claude::SessionEvent;
use crate::docs::plan_outline::{plan_outline, BoxSection};
use crate::pipeline::{
    goal_check_label, validate_allow_rules, validation_goal_note, Stage, EVALUATOR_SYSTEM,
};
use crate::process::Exec;
use crate::review::validation_report::{find_validation_report, parse_validation_report};
use crate::runtime::providers::CheckOutcome;
use crate::runtime::scripts::script_env;
use crate::runtime::sessions::goal_loop::{idle_probe, IterationWatch, Progress, Watch, QUIET};
use crate::runtime::sessions::{SessionLineage, StartStageSessionRequest};
use crate::runtime::terminals::{default_shell, login_shell_args};
use crate::runtime::HyprlayerRuntime;
use crate::scripts::build_block;
use crate::store::Workstream;

const RING_BYTES: usize = 64 * 1024;
const KEPT_TAIL_BYTES: usize = 8 * 1024;
const EVALUATOR_TIMEOUT: Duration = Duration::from_secs(90);
const EVALUATOR_TEXT_CHARS: usize = 4_000;
const EVALUATOR_DIFF_LINES: usize = 50;
const REPLY_CHARS: usize = 200;
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a killed validate session gets to report its exit.
const VALIDATION_EXIT_WAIT: Duration = Duration::from_secs(10);
const HEAD_CHARS: usize = 12;

pub(crate) struct CheckContext {
    pub workstream: Workstream,
    pub checks: Vec<GoalCheck>,
    pub condition: String,
    /// The iteration's last top-level assistant text.
    pub last_text: Option<String>,
    pub spend_reached: bool,
    /// The goal session's harness; a validate session runs on it too.
    pub harness: Harness,
    pub author: Option<ClientRef>,
    /// A validate session's asks and answers, so the goal reads `waiting` while one is open.
    pub asks: mpsc::UnboundedSender<Progress>,
}

pub(crate) struct CheckRun {
    pub results: Vec<GoalCheckResult>,
    pub outcome: GoalCheckOutcome,
    pub reason: Option<String>,
    /// What the checks themselves spent: the evaluator's call, the validate session and its summary.
    pub cost_usd: f64,
}

#[derive(Deserialize)]
struct Verdict {
    met: bool,
    reason: String,
}

/// Runs every Command check in order, even past a failure, then the plan boxes; the evaluator
/// only once all of those passed, and validation last. `cancel` kills the check in flight and stops the run.
pub(crate) async fn run_checks(
    runtime: &HyprlayerRuntime,
    run: &CheckContext,
    cancel: &Notify,
) -> CheckRun {
    let workstream = &run.workstream;
    let port = runtime.scripts.take_port(&workstream.id).await;
    let augmented = runtime.binaries.augmented_path().await;
    let env = script_env(
        workstream,
        port,
        &augmented,
        runtime.environment.overlay_env(),
    );

    let mut results = Vec::new();
    for check in &run.checks {
        let GoalCheck::Command {
            command,
            timeout_secs,
            cwd,
            phases,
            ..
        } = check
        else {
            continue;
        };
        let label = goal_check_label(check);
        let (mut result, cancelled) = match command_dir(&workstream.worktree_path, cwd.as_deref()) {
            Ok(dir) => run_command(&env, &dir, label, command, *timeout_secs, cancel).await,
            Err((outcome, reason)) => (unrun_command(label, outcome, reason), false),
        };
        result.phase = phases.first().cloned();
        results.push(result);
        if cancelled {
            return summarize(results, 0.0);
        }
    }
    for check in &run.checks {
        if let GoalCheck::PlanBoxes { plan_path, phase } = check {
            let plan = Path::new(&workstream.worktree_path).join(plan_path);
            results.push(plan_boxes(check, &plan, phase.as_deref()).await);
        }
    }
    let mut cost_usd = 0.0;
    // A later layer runs only once every earlier one passed.
    if all_passed(&results) {
        for check in &run.checks {
            let GoalCheck::Evaluator { model } = check else {
                continue;
            };
            let (result, spent, cancelled) =
                evaluate(runtime, run, &results, model.as_deref(), cancel).await;
            cost_usd += spent;
            results.push(result);
            if cancelled {
                return summarize(results, cost_usd);
            }
        }
    }
    if all_passed(&results) {
        for check in &run.checks {
            let GoalCheck::Validation { plan_path } = check else {
                continue;
            };
            let (result, spent, cancelled) = validate(runtime, run, plan_path, cancel).await;
            cost_usd += spent;
            results.push(result);
            if cancelled {
                return summarize(results, cost_usd);
            }
        }
    }
    summarize(results, cost_usd)
}

fn all_passed(results: &[GoalCheckResult]) -> bool {
    results.iter().all(|r| r.outcome == GoalCheckOutcome::Pass)
}

/// Every non-manual box in scope ticked, and at least one of them; the file is read fresh each time.
async fn plan_boxes(check: &GoalCheck, plan: &Path, phase: Option<&str>) -> GoalCheckResult {
    let started = Instant::now();
    let (outcome, reason) = match tokio::fs::read_to_string(plan).await {
        Err(e) => (
            GoalCheckOutcome::Error,
            format!("the plan {} could not be read: {e}", plan.display()),
        ),
        Ok(content) => {
            let outline = plan_outline(&content);
            let boxes: Vec<_> = outline
                .phases
                .iter()
                .filter(|p| phase.is_none_or(|id| p.id.eq_ignore_ascii_case(id)))
                .flat_map(|p| &p.boxes)
                .filter(|b| b.section != BoxSection::Manual)
                .collect();
            let open = boxes.iter().filter(|b| !b.ticked).count();
            match (boxes.len(), phase) {
                (0, Some(id)) => (
                    GoalCheckOutcome::Fail,
                    format!("phase {id} has no automated boxes"),
                ),
                (0, None) => (
                    GoalCheckOutcome::Fail,
                    "the plan has no automated boxes".to_string(),
                ),
                (_, _) if open == 0 => (GoalCheckOutcome::Pass, String::new()),
                (total, _) => (
                    GoalCheckOutcome::Fail,
                    format!("{open} of {total} automated boxes open"),
                ),
            }
        }
    };
    GoalCheckResult {
        kind: GoalCheckKind::PlanBoxes,
        label: goal_check_label(check),
        outcome,
        exit_code: None,
        duration_ms: elapsed_ms(started),
        output_tail: String::new(),
        reason: (outcome != GoalCheckOutcome::Pass).then_some(reason),
        session_id: None,
        phase: phase.map(str::to_string),
    }
}

/// The evaluator's verdict, what its call cost, and whether `cancel` cut it short.
async fn evaluate(
    runtime: &HyprlayerRuntime,
    run: &CheckContext,
    earlier: &[GoalCheckResult],
    model: Option<&str>,
    cancel: &Notify,
) -> (GoalCheckResult, f64, bool) {
    let started = Instant::now();
    let result = |outcome, reason: String, tail: &str| GoalCheckResult {
        kind: GoalCheckKind::Evaluator,
        label: "evaluator".to_string(),
        outcome,
        exit_code: None,
        duration_ms: elapsed_ms(started),
        output_tail: crate::redact::tail(tail, KEPT_TAIL_BYTES).to_string(),
        reason: Some(reason),
        session_id: None,
        phase: None,
    };
    if run.spend_reached {
        let skipped = result(
            GoalCheckOutcome::Fail,
            "not run (budget reached)".to_string(),
            "",
        );
        return (skipped, 0.0, false);
    }
    let model = evaluator_model(runtime, model).await;
    let prompt = evaluator_prompt(run, earlier).await;
    let providers = runtime.providers();
    let replied = tokio::select! {
        replied = providers.one_shot(&model, EVALUATOR_SYSTEM, &prompt, EVALUATOR_TIMEOUT) => replied,
        () = cancel.notified() => {
            return (result(GoalCheckOutcome::Error, "cancelled".to_string(), ""), 0.0, true);
        }
    };
    let reply = match replied {
        Ok(reply) => reply,
        Err(e) => {
            let failed = result(
                GoalCheckOutcome::Error,
                format!("the evaluator did not answer: {e}"),
                "",
            );
            return (failed, 0.0, false);
        }
    };
    let spent = reply.cost_usd.unwrap_or(0.0);
    let text = reply.text.trim();
    let judged = match parse_verdict(text) {
        _ if text.is_empty() => result(
            GoalCheckOutcome::Error,
            "the evaluator gave an empty reply".to_string(),
            "",
        ),
        Some(verdict) if verdict.met => result(GoalCheckOutcome::Pass, verdict.reason, ""),
        Some(verdict) => result(GoalCheckOutcome::Fail, verdict.reason, text),
        None => {
            let shown: String = text.chars().take(REPLY_CHARS).collect();
            result(
                GoalCheckOutcome::Error,
                format!("the evaluator's reply is not {{met, reason}}: {shown}"),
                text,
            )
        }
    };
    (judged, spent, false)
}

async fn evaluator_model(runtime: &HyprlayerRuntime, model: Option<&str>) -> String {
    match model.filter(|m| !m.trim().is_empty()) {
        Some(model) => model.to_string(),
        None => {
            let settings = runtime.settings.get().await;
            settings
                .goals
                .effective_evaluator_model(&settings.providers)
                .to_string()
        }
    }
}

/// Kills a validate session whose check is cancelled or dropped before it closes the session itself.
struct KillOnDrop(Option<Arc<dyn AgentDriver>>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(driver) = self.0.take() {
            driver.kill();
        }
    }
}

/// A fresh validate session on the plan, told the approved commands already passed; its report
/// decides. Returns the result, what the session and its summary cost, and whether `cancel` cut it short.
async fn validate(
    runtime: &HyprlayerRuntime,
    run: &CheckContext,
    plan_path: &str,
    cancel: &Notify,
) -> (GoalCheckResult, f64, bool) {
    let started = Instant::now();
    let result =
        |outcome, reason: Option<String>, tail: &str, session_id: Option<&str>| GoalCheckResult {
            kind: GoalCheckKind::Validation,
            label: "validation".to_string(),
            outcome,
            exit_code: None,
            duration_ms: elapsed_ms(started),
            output_tail: crate::redact::tail(tail, KEPT_TAIL_BYTES).to_string(),
            reason,
            session_id: session_id.map(str::to_string),
            phase: None,
        };
    let error = |reason: String, session_id: Option<&str>| {
        result(GoalCheckOutcome::Error, Some(reason), "", session_id)
    };
    if run.spend_reached {
        let skipped = result(
            GoalCheckOutcome::Fail,
            Some("not run (budget reached)".to_string()),
            "",
            None,
        );
        return (skipped, 0.0, false);
    }

    let commands: Vec<String> = run
        .checks
        .iter()
        .filter_map(|check| match check {
            GoalCheck::Command { command, cwd, .. } => Some(match run_dir(cwd.as_deref()) {
                Some(dir) => format!("cd {} && {command}", shell_word(dir)),
                None => command.clone(),
            }),
            _ => None,
        })
        .collect();
    let head = head_of(&run.workstream.worktree_path).await;
    let nonce = uuid::Uuid::new_v4().to_string();
    let sessions = runtime.sessions();
    let prepared = sessions
        .prepare_start_with(
            StartStageSessionRequest {
                workstream: run.workstream.clone(),
                stage: Stage::Validate,
                input: validation_goal_note(&head, &commands),
                attached_docs: vec![plan_path.to_string()],
                harness: Some(run.harness),
                model: None,
                effort: None,
                permission_mode: None,
                prompt_name: None,
                client_message_id: Some(nonce.clone()),
                parent_session_id: None,
            },
            SessionLineage::goal(),
        )
        .await;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(e) => {
            let failed = error(format!("the validate session did not start: {e}"), None);
            return (failed, 0.0, false);
        }
    };
    let session_id = prepared.session.id.clone();
    let sid = Some(session_id.as_str());
    let mut launch = prepared.launch;
    launch.allow_also(validate_allow_rules(&commands));
    launch.report_only();
    // Attached before the launch, so the echo of the first message is seen.
    let mut watch = match Watch::attach(runtime.session_actors.clone(), &session_id).await {
        Ok(watch) => watch,
        Err(e) => {
            let failed = error(
                format!("the validate session could not be watched: {e}"),
                sid,
            );
            return (failed, 0.0, false);
        }
    };
    if let Err(e) = sessions
        .launch_and_converse(launch, run.author.clone())
        .await
    {
        drop(KillOnDrop(sessions.live_driver(&session_id).await));
        let failed = error(format!("the validate session did not start: {e}"), sid);
        return (failed, 0.0, false);
    }
    let mut guard = KillOnDrop(sessions.live_driver(&session_id).await);

    let idle = idle_probe(runtime.session_actors.clone(), session_id.clone());
    let mut state = IterationWatch::new(&nonce, QUIET);
    let deadline = tokio::time::Instant::now() + VALIDATION_TIMEOUT;
    let cancelled = loop {
        tokio::select! {
            biased;
            () = cancel.notified() => break Some(true),
            () = tokio::time::sleep_until(deadline) => break Some(false),
            progress = state.finish(&mut watch, &idle) => match progress {
                Progress::Ended => break None,
                change => {
                    let _ = run.asks.send(change);
                }
            },
        }
    };
    let live_cost = match &guard.0 {
        Some(driver) => driver.usage_so_far().cost_usd,
        None => None,
    };
    // Claude and OpenCode price a turn only at its end, so a cut-off turn may report nothing.
    let mut spent = state.partial().cost_usd.or(live_cost).unwrap_or(0.0);
    if let Some(cancelled) = cancelled {
        let reason = if cancelled {
            "cancelled".to_string()
        } else {
            format!(
                "the validation did not finish within {} minutes",
                VALIDATION_TIMEOUT.as_secs() / 60
            )
        };
        return (error(reason, sid), spent, cancelled);
    }
    let end = state.into_end();

    let report = match sessions.log(&session_id).await {
        Ok(log) => {
            let replies: Vec<&str> = log
                .iter()
                .filter_map(|entry| match &entry.event {
                    SessionEvent::AssistantText {
                        text,
                        parent_id: None,
                        injected: false,
                        auth_required: false,
                    } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            find_validation_report(replies.into_iter()).map(str::to_string)
        }
        Err(e) => {
            tracing::warn!("validate session {session_id}: its log was not read: {e}");
            None
        }
    };
    // Its row stays in history; only the harness goes, so iterations never pile up live sessions.
    if guard.0.take().is_some() && end.exited.is_none() {
        if let Err(e) = sessions.kill(&session_id).await {
            tracing::warn!("validate session {session_id}: not stopped: {e}");
        }
        let _ = tokio::time::timeout(VALIDATION_EXIT_WAIT, async {
            while let Some(event) = watch.next().await {
                if matches!(event, SessionEvent::Exited { .. }) {
                    return;
                }
            }
        })
        .await;
    }
    let Some(report) = report else {
        let missing = error("the validation ended without a report".to_string(), sid);
        return (missing, spent, false);
    };

    let model = evaluator_model(runtime, None).await;
    let providers = runtime.providers();
    let summarized = tokio::select! {
        summarized = providers.summarize_validation_on(&model, &report, EVALUATOR_TIMEOUT) => summarized,
        () = cancel.notified() => return (error("cancelled".to_string(), sid), spent, true),
    };
    // A failed or unusable summary falls back to the bare parse, as the validate card does.
    let (summary, judged) = match summarized {
        Ok((summary, cost)) => {
            spent += cost.unwrap_or(0.0);
            match summary {
                Some(summary) => (summary, true),
                None => (parse_validation_report(&report), false),
            }
        }
        Err(e) => {
            tracing::warn!("validate session {session_id}: its report was not summarized: {e}");
            (parse_validation_report(&report), false)
        }
    };
    // The summarizer files failures that predate the change under notes; the bare parse cannot tell.
    let failing: Vec<&str> = if judged {
        Vec::new()
    } else {
        summary
            .checks
            .iter()
            .filter(|check| check.outcome == Some(CheckOutcome::Fail))
            .map(|check| check.text.as_str())
            .collect()
    };
    let mut findings = summary.issues.clone();
    findings.extend(failing.iter().map(|text| format!("Failed check: {text}")));

    let plan = Path::new(&run.workstream.worktree_path).join(plan_path);
    let (open, total) = match tokio::fs::read_to_string(&plan).await {
        Ok(content) => open_boxes(&content),
        Err(e) => {
            let unread = error(
                format!("the plan {} could not be read: {e}", plan.display()),
                sid,
            );
            return (unread, spent, false);
        }
    };
    let mut counts = Vec::new();
    if !summary.issues.is_empty() {
        counts.push(plural(summary.issues.len(), "issue"));
    }
    if !failing.is_empty() {
        counts.push(plural(failing.len(), "failing check"));
    }
    if open > 0 {
        counts.push(format!("{open} of {total} automated boxes open"));
    }
    let judged = if counts.is_empty() {
        result(GoalCheckOutcome::Pass, None, "", sid)
    } else {
        let reason = format!("validation found {}", counts.join(", "));
        result(
            GoalCheckOutcome::Fail,
            Some(reason),
            &findings.join("\n"),
            sid,
        )
    };
    (judged, spent, false)
}

/// Open and total non-manual boxes across the whole plan; none at all leaves the report to decide.
fn open_boxes(content: &str) -> (usize, usize) {
    let outline = plan_outline(content);
    let boxes: Vec<_> = outline
        .phases
        .iter()
        .flat_map(|p| &p.boxes)
        .filter(|b| b.section != BoxSection::Manual)
        .collect();
    (boxes.iter().filter(|b| !b.ticked).count(), boxes.len())
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

async fn head_of(worktree: &str) -> String {
    let worktree = worktree.to_string();
    tokio::task::spawn_blocking(move || {
        let repo = crate::gitops::open_repo(Path::new(&worktree)).ok()?;
        let oid = repo.head().ok()?.target()?;
        Some(oid.to_string().chars().take(HEAD_CHARS).collect::<String>())
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "HEAD".to_string())
}

// Small models still wrap JSON in a fence or a sentence now and then.
fn parse_verdict(raw: &str) -> Option<Verdict> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(raw.get(start..=end)?).ok()
}

async fn evaluator_prompt(run: &CheckContext, earlier: &[GoalCheckResult]) -> String {
    let worktree = run.workstream.worktree_path.clone();
    let files =
        tokio::task::spawn_blocking(move || crate::gitops::worktree_diff(Path::new(&worktree)))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
    let mut diff: Vec<String> = files
        .iter()
        .take(EVALUATOR_DIFF_LINES)
        .map(|file| format!("{} {}", file.change, file.path))
        .collect();
    if files.len() > EVALUATOR_DIFF_LINES {
        diff.push(format!(
            "… {} more files",
            files.len() - EVALUATOR_DIFF_LINES
        ));
    }
    let commands: Vec<String> = earlier
        .iter()
        .filter(|r| r.kind == GoalCheckKind::Command)
        .map(|r| format!("{} — {}", r.label, r.outcome.as_str()))
        .collect();
    let text = run.last_text.as_deref().map(|text| {
        let skip = text.chars().count().saturating_sub(EVALUATOR_TEXT_CHARS);
        text.chars().skip(skip).collect::<String>()
    });
    let or_none = |s: String| {
        if s.is_empty() {
            "(none)".to_string()
        } else {
            s
        }
    };
    format!(
        "The goal:\n{}\n\nThe agent's last message:\n{}\n\nChanged files:\n{}\n\nCommand checks:\n{}",
        run.condition,
        or_none(text.unwrap_or_default()),
        or_none(diff.join("\n")),
        or_none(commands.join("\n")),
    )
}

fn elapsed_ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// A check's directory, none for the worktree root.
pub(crate) fn run_dir(cwd: Option<&str>) -> Option<&str> {
    cwd.map(str::trim).filter(|dir| !dir.is_empty())
}

/// Relative and free of `..`, so a check never runs outside its worktree.
pub(crate) fn stays_inside(dir: &str) -> bool {
    Path::new(dir)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// A directory a later phase creates fails rather than errors, so it never reads as broken checks.
fn command_dir(worktree: &str, cwd: Option<&str>) -> Result<PathBuf, (GoalCheckOutcome, String)> {
    let Some(dir) = run_dir(cwd) else {
        return Ok(PathBuf::from(worktree));
    };
    if !stays_inside(dir) {
        return Err((
            GoalCheckOutcome::Error,
            format!("the directory {dir} is not inside the worktree"),
        ));
    }
    let path = Path::new(worktree).join(dir);
    if !path.is_dir() {
        return Err((
            GoalCheckOutcome::Fail,
            format!("the directory {dir} does not exist"),
        ));
    }
    Ok(path)
}

fn unrun_command(label: String, outcome: GoalCheckOutcome, reason: String) -> GoalCheckResult {
    GoalCheckResult {
        kind: GoalCheckKind::Command,
        label,
        outcome,
        exit_code: None,
        duration_ms: 0,
        output_tail: String::new(),
        reason: Some(reason),
        session_id: None,
        phase: None,
    }
}

/// Quoted only when it has to be, so the plain `cd dir` part can become an allow rule.
fn shell_word(word: &str) -> String {
    if word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

async fn run_command(
    env: &[(String, String)],
    cwd: &Path,
    label: String,
    command: &str,
    timeout_secs: u32,
    cancel: &Notify,
) -> (GoalCheckResult, bool) {
    let exec = Exec::new(default_shell())
        .args(login_shell_args(&build_block(command)))
        .cwd(cwd)
        .envs(env.iter().cloned())
        .timeout(Duration::from_secs(u64::from(timeout_secs)));
    let started = Instant::now();
    let ran = exec.run_bounded(RING_BYTES, cancel).await;
    let duration_ms = elapsed_ms(started);

    let mut cancelled = false;
    let (outcome, exit_code, reason, tail) = match ran {
        Err(e) => (
            GoalCheckOutcome::Error,
            None,
            Some(e.to_string()),
            String::new(),
        ),
        Ok(out) if out.cancelled => {
            cancelled = true;
            (
                GoalCheckOutcome::Error,
                None,
                Some("cancelled".to_string()),
                out.tail,
            )
        }
        Ok(out) if out.timed_out => (
            GoalCheckOutcome::Error,
            None,
            Some(format!("timed out after {timeout_secs} s")),
            out.tail,
        ),
        Ok(out) if out.exit_code == Some(0) => {
            (GoalCheckOutcome::Pass, Some(0), None, String::new())
        }
        Ok(out) => (
            GoalCheckOutcome::Fail,
            out.exit_code,
            out.exit_code
                .is_none()
                .then(|| "terminated by a signal".to_string()),
            out.tail,
        ),
    };
    let result = GoalCheckResult {
        kind: GoalCheckKind::Command,
        label,
        outcome,
        exit_code,
        duration_ms,
        output_tail: crate::redact::tail(&tail, KEPT_TAIL_BYTES).to_string(),
        reason,
        session_id: None,
        phase: None,
    };
    (result, cancelled)
}

fn summarize(results: Vec<GoalCheckResult>, cost_usd: f64) -> CheckRun {
    let any = |outcome| results.iter().any(|r| r.outcome == outcome);
    let outcome = if results.is_empty() || any(GoalCheckOutcome::Error) {
        GoalCheckOutcome::Error
    } else if any(GoalCheckOutcome::Fail) {
        GoalCheckOutcome::Fail
    } else {
        GoalCheckOutcome::Pass
    };
    let reason = match results.iter().find(|r| r.outcome != GoalCheckOutcome::Pass) {
        Some(result) => Some(failure_reason(result)),
        None if results.is_empty() => Some("no checks ran".to_string()),
        None => None,
    };
    CheckRun {
        results,
        outcome,
        reason,
        cost_usd,
    }
}

fn failure_reason(result: &GoalCheckResult) -> String {
    let why = match (&result.reason, result.exit_code) {
        (Some(reason), _) => reason.clone(),
        (None, Some(code)) => format!("exit code {code}"),
        (None, None) => "failed".to_string(),
    };
    format!("{}: {why}", result.label)
}
