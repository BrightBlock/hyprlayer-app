use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{Local, TimeZone};
use hyprlayer_models::RuntimeEvent;
use tokio::sync::{broadcast, Notify};
use tokio::time::Instant;

use crate::claude::SessionEvent;
use crate::runtime::shutdown::TrackedTask;
use crate::runtime::HyprlayerRuntime;
use crate::store::SessionLogEntry;

const DEBOUNCE: Duration = Duration::from_secs(20);
const TIMEOUT: Duration = Duration::from_secs(60);
const EXCERPT_CHARS: usize = 12_000;
const LINE_CHARS: usize = 600;
const MAX_CHARS: usize = 100;

const SYSTEM: &str = "You write one-line status summaries of a lead engineer agent's day.";
const INSTRUCTIONS: &str = "Below are today's messages to and replies from the Lead Engineer, \
an agent that coordinates coding work on a project. Summarize what the Lead has done today in \
one line of at most 80 characters, e.g. \"Merged 14 PRs: goal-runner and skill-install fixes; \
dev build installed\". Lead with outcomes and counts, drop pleasantries, never write \"I\" or \
\"the Lead\". Plain text only: no quotes, markdown or preamble.";

struct Summary {
    text: String,
    day_start: i64,
    through: f64,
}

#[derive(Default)]
struct State {
    summaries: HashMap<String, Summary>,
    due: HashMap<String, Instant>,
    backfilled: HashSet<String>,
}

/// One-line digests of each Lead session's day, kept in memory: a restarted
/// server backfills a Lead's summary the first time its view is read.
#[derive(Default)]
pub(crate) struct LeadSummaries {
    state: Mutex<State>,
    wake: Notify,
}

impl LeadSummaries {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn schedule(&self, session_id: &str) {
        self.lock()
            .due
            .insert(session_id.to_string(), Instant::now() + DEBOUNCE);
        self.wake.notify_one();
    }

    /// Today's summary, scheduling the first one for a session this server hasn't summarized.
    pub(crate) fn get(&self, session_id: &str) -> Option<String> {
        let mut state = self.lock();
        if let Some(summary) = state.summaries.get(session_id) {
            return (summary.day_start == day_start()).then(|| summary.text.clone());
        }
        if state.backfilled.insert(session_id.to_string()) {
            drop(state);
            self.schedule(session_id);
        }
        None
    }

    fn take_due(&self) -> (Vec<String>, Option<Instant>) {
        let now = Instant::now();
        let mut state = self.lock();
        let ready: Vec<String> = state
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ready {
            state.due.remove(id);
        }
        (ready, state.due.values().min().copied())
    }
}

fn day_start() -> i64 {
    let midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map_or(0, |t| t.timestamp_millis())
}

pub(crate) fn spawn_worker(
    summaries: Arc<LeadSummaries>,
    runtime: &Arc<HyprlayerRuntime>,
) -> TrackedTask {
    let mut events = runtime.events.subscribe();
    let runtime = Arc::downgrade(runtime);
    TrackedTask::spawn(move |stop| async move {
        loop {
            let (ready, next) = summaries.take_due();
            for session_id in ready {
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                tokio::select! {
                    _ = summarize(&runtime, &summaries, &session_id) => {}
                    _ = stop.notified() => return,
                }
            }
            let sleep = tokio::time::sleep_until(next.unwrap_or_else(|| Instant::now() + DEBOUNCE));
            tokio::select! {
                event = events.recv() => match event {
                    Ok(RuntimeEvent::SessionRevised { session_id, working: false, .. }) => {
                        let Some(runtime) = runtime.upgrade() else {
                            return;
                        };
                        if is_lead(&runtime, &session_id).await {
                            summaries.schedule(&session_id);
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = sleep, if next.is_some() => {}
                _ = summaries.wake.notified() => {}
                _ = stop.notified() => return,
            }
        }
    })
}

async fn is_lead(runtime: &HyprlayerRuntime, session_id: &str) -> bool {
    let db = runtime.db.clone();
    let id = session_id.to_string();
    matches!(
        tokio::task::spawn_blocking(move || db.get_session(&id)).await,
        Ok(Ok(Some(session))) if session.lead
    )
}

async fn summarize(runtime: &HyprlayerRuntime, summaries: &LeadSummaries, session_id: &str) {
    let today = day_start();
    let db = runtime.db.clone();
    let id = session_id.to_string();
    let log = match tokio::task::spawn_blocking(move || db.session_log(&id)).await {
        Ok(Ok(log)) => log,
        Ok(Err(e)) => return tracing::warn!("lead {session_id}: reading its log: {e}"),
        Err(e) => return tracing::warn!("lead {session_id}: reading its log: {e}"),
    };
    let Some((excerpt, through)) = excerpt(&log, today as f64) else {
        return;
    };
    let unchanged = summaries
        .lock()
        .summaries
        .get(session_id)
        .is_some_and(|s| s.day_start == today && s.through == through);
    if unchanged {
        return;
    }

    let settings = runtime.settings.get().await;
    let model = settings.tab_titles.effective_model(&settings.providers);
    let prompt = format!("{INSTRUCTIONS}\n\n<today>\n{excerpt}\n</today>");
    let text = match runtime
        .providers()
        .one_shot(model, SYSTEM, &prompt, TIMEOUT)
        .await
    {
        Ok(reply) => clean(&reply.text),
        Err(e) => return tracing::info!("lead {session_id}: no summary generated: {e}"),
    };
    let Some(text) = text else {
        return;
    };
    summaries.lock().summaries.insert(
        session_id.to_string(),
        Summary {
            text,
            day_start: today,
            through,
        },
    );

    let db = runtime.db.clone();
    let id = session_id.to_string();
    // Clients refetch leads on any lifecycle event; a new event kind would
    // break older clients' frame decoding.
    if let Ok(Ok(Some(session))) = tokio::task::spawn_blocking(move || db.get_session(&id)).await {
        let _ = runtime.events.send(RuntimeEvent::SessionLifecycle {
            session_id: session.id,
            workstream_id: session.workstream_id,
            state: session.state,
        });
    }
}

fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

/// Today's asks and replies, newest kept when over budget, with the time of the newest event.
fn excerpt(log: &[SessionLogEntry], since: f64) -> Option<(String, f64)> {
    let today: Vec<&SessionLogEntry> = log.iter().filter(|e| e.created_at >= since).collect();
    let through = today.last()?.created_at;
    let mut lines = Vec::new();
    let mut budget = EXCERPT_CHARS;
    for entry in today.iter().rev() {
        let line = match &entry.event {
            SessionEvent::UserMessage { text, .. } => format!("Asked: {}", clip(text, LINE_CHARS)),
            SessionEvent::AssistantText {
                text,
                parent_id: None,
                injected: false,
                auth_required: false,
            } if !text.trim().is_empty() => format!("Lead: {}", clip(text, LINE_CHARS)),
            _ => continue,
        };
        if line.len() > budget {
            break;
        }
        budget -= line.len() + 1;
        lines.push(line);
    }
    if !lines.iter().any(|l| l.starts_with("Lead: ")) {
        return None;
    }
    lines.reverse();
    Some((lines.join("\n"), through))
}

fn clean(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '*'));
    (!line.is_empty() && !line.starts_with('⚠')).then(|| clip(line, MAX_CHARS))
}
