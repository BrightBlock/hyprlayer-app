use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyprlayer_models::{GoalStatus, WorkstreamStatus};
use serde_json::Value;
use tokio::task::JoinSet;

use crate::claude::SessionEvent;
use crate::error::Result;
use crate::pipeline::RESTART_PROMPT_NAME;
use crate::runtime::goals::{resume_running, GoalRunners, MAX_RESTART_RESUMES, OVERLAY_WAIT};
use crate::runtime::shutdown::TrackedTask;
use crate::runtime::HyprlayerRuntime;
use crate::store::{utc_date, Session, SessionLogEntry};

use super::goal_loop::Watch;
use super::service::{blocking, AppTurn};

const CONCURRENT_WAKES: usize = 2;
const STARTED_WAIT: Duration = Duration::from_secs(60);

struct Wake {
    session: Session,
    project_path: String,
    tasks: Vec<String>,
}

pub(crate) fn spawn_after_restart(
    runners: Arc<GoalRunners>,
    runtime: Weak<HyprlayerRuntime>,
) -> TrackedTask {
    let restarted_at = utc_minute(SystemTime::now());
    TrackedTask::spawn(move |stop| async move {
        // Waking only after the resumer's pass: a goal it skipped as paused is then this pass's to wake.
        let pickup = async {
            resume_running(&runners, &runtime).await;
            wake_stopped(&runtime, &restarted_at).await;
            if let Some(rt) = runtime.upgrade() {
                super::held::sweep_after_restart(&rt).await;
            }
        };
        tokio::select! {
            () = pickup => {}
            () = stop.notified() => {}
        }
    })
}

async fn wake_stopped(runtime: &Weak<HyprlayerRuntime>, restarted_at: &str) {
    let Some(rt) = runtime.upgrade() else {
        return;
    };
    let db = rt.db.clone();
    let pending = match blocking(move || db.pending_wakes()).await {
        Ok(pending) if pending.is_empty() => return,
        Ok(pending) => pending,
        Err(e) => {
            tracing::warn!("could not read the sessions to wake after the restart: {e}");
            return;
        }
    };
    let mut arrived = rt.overlay_arrived.subscribe();
    drop(rt);
    if tokio::time::timeout(OVERLAY_WAIT, arrived.wait_for(|arrived| *arrived))
        .await
        .is_err()
    {
        tracing::info!("no client environment within {OVERLAY_WAIT:?}; waking sessions without it");
    }
    let Some(rt) = runtime.upgrade() else {
        return;
    };
    let mut due = Vec::new();
    for (id, mid_turn) in &pending {
        match check(&rt, id, *mid_turn).await {
            Ok(Some(wake)) => {
                due.push(wake);
                continue;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("session {id}: not checked for a wake after the restart: {e}"),
        }
        forget(&rt, id).await;
    }
    let texts: Vec<String> = due
        .iter()
        .map(|wake| wake_text(wake, &children(wake, &due), restarted_at))
        .collect();
    let mut running = JoinSet::new();
    let mut woken = 0;
    for (wake, text) in due.into_iter().zip(texts) {
        if running.len() >= CONCURRENT_WAKES {
            woken += usize::from(matches!(running.join_next().await, Some(Ok(true))));
        }
        running.spawn(wake_and_forget(rt.clone(), wake, text));
    }
    while let Some(result) = running.join_next().await {
        woken += usize::from(matches!(result, Ok(true)));
    }
    let stopped = pending.len();
    tracing::info!("woke {woken} of {stopped} session(s) the last server stopped");
}

fn children(wake: &Wake, due: &[Wake]) -> Vec<String> {
    due.iter()
        .filter(|child| child.session.spawn_parent() == Some(wake.session.id.as_str()))
        .map(|child| {
            child
                .session
                .title
                .clone()
                .unwrap_or_else(|| child.session.id.clone())
        })
        .collect()
}

async fn wake_and_forget(rt: Arc<HyprlayerRuntime>, wake: Wake, text: String) -> bool {
    let id = wake.session.id.clone();
    let mut watch = Watch::attach(rt.session_actors.clone(), &id).await.ok();
    let woken = match wake_one(&rt, &wake, &text).await {
        Ok(woken) => woken,
        Err(e) => {
            tracing::warn!("session {id}: not woken after the server restart: {e}");
            false
        }
    };
    if woken {
        tracing::info!("session {id}: woken after the server restart");
        if let Some(watch) = watch.as_mut() {
            let _ = tokio::time::timeout(STARTED_WAIT, started(watch)).await;
        }
    }
    forget(&rt, &id).await;
    woken
}

async fn started(watch: &mut Watch) {
    while let Some(event) = watch.next().await {
        if matches!(
            event,
            SessionEvent::Init { .. } | SessionEvent::Exited { .. }
        ) {
            return;
        }
    }
}

async fn forget(rt: &HyprlayerRuntime, id: &str) {
    let (db, sid) = (rt.db.clone(), id.to_string());
    if let Err(e) = blocking(move || db.delete_pending_wake(&sid)).await {
        tracing::warn!("session {id}: its wake stays queued: {e}");
    }
}

async fn check(rt: &HyprlayerRuntime, id: &str, mid_turn: bool) -> Result<Option<Wake>> {
    let sessions = rt.sessions();
    let Some(session) = sessions.get(id).await? else {
        return Ok(None);
    };
    // A running goal is the goal resumer's; a paused one's session is woken like any other.
    let goal_running = matches!(&session.goal, Some(goal) if goal.status == GoalStatus::Running);
    if session.ended_at.is_some() || session.archived_at.is_some() || goal_running {
        return Ok(None);
    }
    let workstream = rt.workstreams().find(&session.workstream_id).await?;
    if workstream.status == WorkstreamStatus::Archived {
        return Ok(None);
    }
    let log = sessions.log(id).await?;
    let tasks = open_background_tasks(&log);
    if !mid_turn && tasks.is_empty() {
        return Ok(None);
    }
    if session.provider_session_id.is_none() {
        tracing::warn!("session {id}: not woken: it never started a conversation to resume");
        return Ok(None);
    }
    let (db, sid) = (rt.db.clone(), id.to_string());
    let unanswered = blocking(move || db.unanswered_restart_wakes(&sid)).await?;
    if unanswered >= MAX_RESTART_RESUMES {
        tracing::warn!(
            "session {id}: not woken: the last {unanswered} restart wakes never finished a turn"
        );
        return Ok(None);
    }
    Ok(Some(Wake {
        session,
        project_path: workstream.project_path,
        tasks,
    }))
}

async fn wake_one(rt: &HyprlayerRuntime, wake: &Wake, text: &str) -> Result<bool> {
    let sessions = rt.sessions();
    let id = &wake.session.id;
    let op = sessions.op_lock(id).await;
    let guard = op.lock().await;
    let Some(session) = sessions.get(id).await? else {
        return Ok(false);
    };
    if sessions
        .resume_if_exited_held(session, wake.project_path.clone(), &guard)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    // Before the send: a server that stops before the actor stores the wake's echo must still count it.
    let (db, sid) = (rt.db.clone(), id.clone());
    blocking(move || db.record_restart_wake(&sid)).await?;
    let turn = AppTurn {
        text,
        prompt_name: Some(RESTART_PROMPT_NAME),
        author: None,
        nonce: uuid::Uuid::new_v4().to_string(),
        collab_mode: None,
    };
    sessions.send_app_turn_held(id, turn, &guard).await?;
    Ok(true)
}

fn wake_text(wake: &Wake, children: &[String], restarted_at: &str) -> String {
    let harness = wake.session.harness.as_str();
    let stopped = if wake.tasks.is_empty() {
        format!("Your {harness} process was stopped mid-turn.")
    } else {
        format!(
            "Your {harness} process and its background tasks ({}) were stopped.",
            wake.tasks.join(", ")
        )
    };
    let mut text = format!(
        "[from hyprlayer]\n\nhyprlayer restarted at {restarted_at}. {stopped} \
         Re-check what you were waiting on and carry on."
    );
    if !children.is_empty() {
        text.push_str(&format!(
            "\n\nThe agents you spawned were stopped too and are being woken the same way: {}.",
            children.join(", ")
        ));
    }
    text
}

fn open_background_tasks(log: &[SessionLogEntry]) -> Vec<String> {
    let since = log
        .iter()
        .rposition(|entry| {
            matches!(
                entry.event,
                SessionEvent::Relaunched { .. }
                    | SessionEvent::Exited { .. }
                    | SessionEvent::Rewound {}
            )
        })
        .map_or(0, |boundary| boundary + 1);
    let mut open: Vec<(&str, String)> = Vec::new();
    for entry in &log[since..] {
        match &entry.event {
            SessionEvent::ToolUse {
                id,
                name,
                input,
                parent_id: None,
            } => {
                if let Some(label) = background_label(name, input) {
                    open.push((id, label));
                }
            }
            SessionEvent::ToolResult {
                tool_use_id,
                is_error: true,
                ..
            } => open.retain(|(id, _)| id != tool_use_id),
            SessionEvent::BackgroundTaskUpdate {
                tool_use_id: Some(tool_use_id),
                status,
                ..
            } if status != "running" => open.retain(|(id, _)| id != tool_use_id),
            _ => {}
        }
    }
    open.into_iter().map(|(_, label)| label).collect()
}

fn background_label(name: &str, input: &str) -> Option<String> {
    let input: Value = serde_json::from_str(input).unwrap_or_default();
    let background = input.get("run_in_background").and_then(Value::as_bool);
    // As the client's task list reads it: an agent backgrounds unless told not to, a shell command only when told to.
    let spawned = match name {
        "Workflow" => true,
        "Agent" | "Task" => background != Some(false),
        "Bash" => background == Some(true),
        _ => false,
    };
    let description = input
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty());
    spawned.then(|| description.unwrap_or(name).to_string())
}

fn utc_minute(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!(
        "{} {:02}:{:02} UTC",
        utc_date(secs),
        secs % 86_400 / 3_600,
        secs % 3_600 / 60
    )
}
