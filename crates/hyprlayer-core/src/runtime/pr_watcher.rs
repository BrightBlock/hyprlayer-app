use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};

use crate::config::ConfigService;
use crate::error::Result;
use crate::index::watcher::WatchHandle;
use crate::pr::{self, PrState, PrStatus};
use crate::process::binaries::{BinaryKind, BinaryResolver};
use crate::store::{Db, ObservedPr, Workstream};
use hyprlayer_models::RuntimeEvent;

use super::doc_status;
use super::shutdown::TrackedTask;
use super::workstreams::{current_pr_ref, has_own_pr, pr_dir};

const FAST: Duration = Duration::from_secs(8);
const MEDIUM: Duration = Duration::from_secs(20);
const SLOW: Duration = Duration::from_secs(60);

const BG_OPEN: Duration = Duration::from_secs(5 * 60);
const BG_IDLE: Duration = Duration::from_secs(20 * 60);

const RATE_LIMITED: Duration = Duration::from_secs(15 * 60);

const MIN_TICK: Duration = Duration::from_secs(4);

const MAX_CONCURRENT: usize = 6;

const RECORD_SWEEP: Duration = BG_IDLE;
const RECORDS_PER_SWEEP: usize = 12;

// Reported on the change, not every tick.
static GH_MISSING_REPORTED: AtomicBool = AtomicBool::new(false);
static SIGNED_OUT_REPORTED: AtomicBool = AtomicBool::new(false);

/// Every workstream a client watches; `None` where there was nothing to watch.
pub type WorktreeWatch = Arc<AsyncMutex<HashMap<String, Option<WatchHandle>>>>;

pub struct PrWatcherContext {
    pub binaries: Arc<BinaryResolver>,
    pub db: Arc<Db>,
    pub config: Arc<ConfigService>,
    pub worktree_watch: WorktreeWatch,
    pub focused: Arc<AtomicBool>,
    pub focus_changed: Arc<Notify>,
    pub events: broadcast::Sender<RuntimeEvent>,
    pub env_overlay: Arc<RwLock<BTreeMap<String, String>>>,
}

struct Tracked {
    status: Option<PrStatus>,
    due: Instant,
    pr_ref: String,
    /// A moved-to ref whose lookup failed: it waits out `due` instead of retrying every tick.
    failed_ref: Option<String>,
}

#[derive(Default)]
struct WatchState {
    tracked: HashMap<String, Tracked>,
    active: HashSet<String>,
    backoff_until: Option<Instant>,
    swept_at: Option<Instant>,
    /// Records whose number names no PR in their project's repository.
    unresolved: HashSet<PathBuf>,
}

pub fn spawn(ctx: PrWatcherContext) -> TrackedTask {
    TrackedTask::spawn(move |stop| async move {
        let mut state = WatchState::default();
        loop {
            if !ctx.focused.load(Ordering::Relaxed) {
                tokio::select! {
                    _ = ctx.focus_changed.notified() => {}
                    _ = tokio::time::sleep(SLOW) => {}
                    _ = stop.notified() => return,
                }
                continue;
            }

            let delay = tick(&ctx, &mut state).await;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = ctx.focus_changed.notified() => {}
                _ = stop.notified() => return,
            }
        }
    })
}

async fn tick(ctx: &PrWatcherContext, st: &mut WatchState) -> Duration {
    let now = Instant::now();

    if let Some(until) = st.backoff_until {
        if now < until {
            return until.saturating_duration_since(now).clamp(MIN_TICK, SLOW);
        }
        st.backoff_until = None;
    }

    let Ok(gh) = ctx.binaries.require(BinaryKind::Gh).await else {
        if !GH_MISSING_REPORTED.swap(true, Ordering::Relaxed) {
            tracing::info!(
                "pr watch: gh was not found; PR status stays unknown until it is installed"
            );
        }
        return SLOW;
    };
    GH_MISSING_REPORTED.store(false, Ordering::Relaxed);

    let db = ctx.db.clone();
    let items = match tokio::task::spawn_blocking(move || db.list_active_workstreams()).await {
        Ok(Ok(items)) => items,
        Ok(Err(e)) => {
            tracing::warn!("pr watch: listing workstreams failed: {e}");
            return SLOW;
        }
        Err(e) => {
            tracing::warn!("pr watch: list task join error: {e}");
            return SLOW;
        }
    };

    let active: HashSet<String> = ctx.worktree_watch.lock().await.keys().cloned().collect();

    let mut alive: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(items.len());
    for item in &items {
        alive.insert(item.id.as_str());
    }
    st.tracked.retain(|id, _| alive.contains(id.as_str()));

    for id in active.difference(&st.active) {
        if let Some(t) = st.tracked.get_mut(id) {
            t.due = now;
        }
    }
    st.active.clone_from(&active);

    let projects: HashMap<String, String> = items
        .iter()
        .filter(|item| has_own_pr(item))
        .map(|item| (item.id.clone(), item.project_path.clone()))
        .collect();

    // A merged PR is never polled again, but its record can land after the merge.
    let resettle: Vec<(String, u32)> = st
        .tracked
        .iter()
        .filter_map(|(id, t)| match &t.status {
            Some(s) if s.state == PrState::Merged => Some((id.clone(), s.number)),
            _ => None,
        })
        .collect();
    for (id, number) in resettle {
        if let Some(project) = projects.get(&id) {
            doc_status::settle_merged_record(ctx.db.clone(), &ctx.config, &id, project, number)
                .await;
        }
    }

    let to_poll: Vec<(Workstream, String)> = items
        .into_iter()
        .filter(|item| !item.is_folder())
        .filter_map(|item| {
            let pr_ref = current_pr_ref(&item)?;
            let due = match st.tracked.get(&item.id) {
                Some(t) => {
                    let moved = t.pr_ref != pr_ref;
                    // A merge settles one ref; the worktree can move on to another.
                    let settled =
                        !moved && matches!(&t.status, Some(s) if s.state == PrState::Merged);
                    (moved && t.failed_ref.as_deref() != Some(pr_ref.as_str()))
                        || (!settled && t.due <= now)
                }
                None => true,
            };
            due.then_some((item, pr_ref))
        })
        .collect();

    let overlay: Vec<(String, String)> = ctx
        .env_overlay
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let overlay = ctx.binaries.child_env(overlay).await;
    let results = if to_poll.is_empty() {
        Vec::new()
    } else {
        poll_all(&gh, to_poll, &overlay).await
    };
    let polled_at = Instant::now();
    let mut rate_limited = false;

    for (item, pr_ref, res) in results {
        let id = item.id.clone();
        let status = match res {
            Ok(status) => {
                if SIGNED_OUT_REPORTED.swap(false, Ordering::Relaxed) {
                    tracing::info!("pr watch: gh is signed in again");
                }
                status
            }
            Err(e) => {
                rate_limited |= is_rate_limited(&e.to_string());
                if crate::repos::is_github_auth_failure(&e.to_string())
                    && !SIGNED_OUT_REPORTED.swap(true, Ordering::Relaxed)
                {
                    tracing::warn!("pr watch: gh is not signed in; run `gh auth login`: {e}");
                }
                tracing::debug!("pr watch: gh status failed for {id}: {e}");
                let moved = item
                    .observed_ref
                    .as_deref()
                    .is_some_and(|seen| seen != pr_ref);
                let is_active = active.contains(&id);
                let wait = if moved {
                    MEDIUM
                } else {
                    st.tracked
                        .get(&id)
                        .map_or(SLOW, |t| cadence(&t.status, is_active))
                        .max(MEDIUM)
                };
                let t = st.tracked.entry(id.clone()).or_insert(Tracked {
                    status: None,
                    due: polled_at,
                    pr_ref: item.observed_ref.clone().unwrap_or_else(|| pr_ref.clone()),
                    failed_ref: None,
                });
                t.due = polled_at + wait;
                if !moved && t.pr_ref != pr_ref {
                    t.status = None;
                    t.pr_ref.clone_from(&pr_ref);
                }
                // Lists stop showing the old ref's PR now, not once a lookup succeeds.
                if moved && t.failed_ref.as_deref() != Some(pr_ref.as_str()) {
                    t.failed_ref = Some(pr_ref);
                    let _ = ctx.events.send(RuntimeEvent::PrStatusChanged {
                        workstream_id: id,
                        observed_pr_changed: true,
                    });
                }
                continue;
            }
        };
        let changed = st.tracked.get(&id).map(|t| &t.status) != Some(&status);
        if let Some(s) = status.as_ref().filter(|s| s.state != PrState::Open) {
            if let Some(project) = projects.get(&id) {
                doc_status::settle_workstream(
                    ctx.db.clone(),
                    &ctx.config,
                    &id,
                    project,
                    s.number,
                    s.state,
                )
                .await;
            }
        }
        let observed = status
            .as_ref()
            .filter(|_| projects.contains_key(&id))
            .map(|s| ObservedPr {
                number: s.number,
                state: s.state,
            });
        let recorded = record_observed(ctx.db.clone(), &id, observed, &pr_ref).await;
        let due = polled_at + cadence(&status, active.contains(&id));
        st.tracked.insert(
            id.clone(),
            Tracked {
                status,
                due,
                pr_ref,
                failed_ref: None,
            },
        );
        if changed || recorded {
            let _ = ctx.events.send(RuntimeEvent::PrStatusChanged {
                workstream_id: id,
                observed_pr_changed: recorded,
            });
        }
    }

    let sweep_due = st
        .swept_at
        .is_none_or(|at| polled_at.saturating_duration_since(at) >= RECORD_SWEEP);
    if !rate_limited && sweep_due {
        st.swept_at = Some(polled_at);
        let tracked: HashSet<(String, u32)> = st
            .tracked
            .iter()
            .filter_map(|(id, t)| Some((projects.get(id)?.clone(), t.status.as_ref()?.number)))
            .collect();
        rate_limited = sweep_records(ctx, &gh, &overlay, &tracked, &mut st.unresolved).await;
    }

    st.backoff_until = rate_limited.then(|| {
        tracing::warn!("pr watch: GitHub API quota spent, holding off {RATE_LIMITED:?}");
        polled_at + RATE_LIMITED
    });

    next_tick(&st.tracked, polled_at, st.backoff_until)
}

async fn record_observed(
    db: Arc<Db>,
    id: &str,
    observed: Option<ObservedPr>,
    pr_ref: &str,
) -> bool {
    let (wid, reference) = (id.to_string(), pr_ref.to_string());
    let written =
        tokio::task::spawn_blocking(move || db.record_pr_observation(&wid, observed, &reference))
            .await;
    match written {
        Ok(Ok(recorded)) => recorded,
        Ok(Err(e)) => {
            tracing::warn!("pr watch: recording {id}'s PR failed: {e}");
            false
        }
        Err(e) => {
            tracing::warn!("pr watch: record task join error: {e}");
            false
        }
    }
}

fn is_rate_limited(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("rate limit") || s.contains("secondary rate limit")
}

/// Settles PR records no live workstream tracks (opened elsewhere, or archived before the merge
/// was seen). Returns whether GitHub's quota ran out.
async fn sweep_records(
    ctx: &PrWatcherContext,
    gh: &Path,
    overlay: &[(String, String)],
    tracked: &HashSet<(String, u32)>,
    unresolved: &mut HashSet<PathBuf>,
) -> bool {
    let mut records = doc_status::unsettled_records(ctx.db.clone(), &ctx.config).await;
    records.retain(|r| {
        !unresolved.contains(&r.path) && !tracked.contains(&(r.project_path.clone(), r.number))
    });
    records.sort_by_key(|r| std::cmp::Reverse(r.number));
    for record in records.into_iter().take(RECORDS_PER_SWEEP) {
        let number = record.number;
        match pr::pr_state(
            gh,
            Path::new(&record.project_path),
            &number.to_string(),
            overlay,
        )
        .await
        {
            Ok(Some(state)) => {
                doc_status::settle_record(record.path, number, state).await;
            }
            Ok(None) => {
                unresolved.insert(record.path);
            }
            Err(e) if is_rate_limited(&e.to_string()) => return true,
            Err(e) => {
                tracing::debug!("pr watch: looking up PR #{number} for its record failed: {e}")
            }
        }
    }
    false
}

async fn poll_all(
    gh: &Path,
    items: Vec<(Workstream, String)>,
    overlay: &[(String, String)],
) -> Vec<(Workstream, String, Result<Option<PrStatus>>)> {
    use tokio::task::JoinSet;

    let mut set: JoinSet<(Workstream, String, Result<Option<PrStatus>>)> = JoinSet::new();
    let mut results = Vec::with_capacity(items.len());
    for (item, pr_ref) in items {
        if set.len() >= MAX_CONCURRENT {
            if let Some(Ok(r)) = set.join_next().await {
                results.push(r);
            }
        }
        let gh = gh.to_path_buf();
        let overlay = overlay.to_vec();
        set.spawn(async move {
            let res = pr::pr_status(&gh, pr_dir(&item), &pr_ref, &overlay).await;
            (item, pr_ref, res)
        });
    }
    while let Some(joined) = set.join_next().await {
        if let Ok(r) = joined {
            results.push(r);
        }
    }
    results
}

fn cadence(status: &Option<PrStatus>, active: bool) -> Duration {
    match (active, status) {
        (false, Some(s)) if s.state == PrState::Open => BG_OPEN,
        (false, _) => BG_IDLE,
        (true, Some(s)) if s.state == PrState::Open && s.checks.pending > 0 => FAST,
        (true, Some(s)) if s.state == PrState::Open => MEDIUM,
        (true, _) => SLOW,
    }
}

fn next_tick(
    tracked: &HashMap<String, Tracked>,
    now: Instant,
    backoff_until: Option<Instant>,
) -> Duration {
    let soonest = tracked
        .values()
        .filter(|t| !matches!(&t.status, Some(s) if s.state == PrState::Merged))
        .map(|t| t.due)
        .min()
        .unwrap_or(now + SLOW);
    backoff_until
        .map_or(soonest, |b| soonest.max(b))
        .saturating_duration_since(now)
        .clamp(MIN_TICK, SLOW)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::{ChecksState, MergeableState, PrChecks};

    fn pr(state: PrState, checks: ChecksState) -> Option<PrStatus> {
        Some(PrStatus {
            number: 1,
            url: String::new(),
            title: String::new(),
            state,
            mergeable: true,
            mergeable_state: MergeableState::Mergeable,
            checks: PrChecks {
                state: checks,
                total: 0,
                passed: 0,
                failed: 0,
                pending: u32::from(checks == ChecksState::Pending),
                skipped: 0,
                cancelled: 0,
                list: Vec::new(),
            },
            review_decision: None,
        })
    }

    fn tracked(status: Option<PrStatus>, due: Instant) -> Tracked {
        Tracked {
            status,
            due,
            pr_ref: String::new(),
            failed_ref: None,
        }
    }

    #[test]
    fn active_workstream_gets_the_live_cadence() {
        assert_eq!(
            cadence(&pr(PrState::Open, ChecksState::Pending), true),
            FAST
        );
        assert_eq!(
            cadence(&pr(PrState::Open, ChecksState::Passing), true),
            MEDIUM
        );
        assert_eq!(cadence(&None, true), SLOW);
    }

    #[test]
    fn a_failing_pr_with_a_running_check_stays_fast() {
        let mut failing = pr(PrState::Open, ChecksState::Failing);
        let checks = &mut failing.as_mut().unwrap().checks;
        checks.failed = 1;
        checks.pending = 1;
        assert_eq!(cadence(&failing, true), FAST);
    }

    #[test]
    fn background_workstreams_are_polled_rarely_whatever_their_checks() {
        assert_eq!(
            cadence(&pr(PrState::Open, ChecksState::Pending), false),
            BG_OPEN
        );
        assert_eq!(
            cadence(&pr(PrState::Open, ChecksState::Passing), false),
            BG_OPEN
        );
        assert_eq!(cadence(&None, false), BG_IDLE);
    }

    #[test]
    fn next_tick_wakes_for_the_soonest_and_skips_merged() {
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert("idle".into(), tracked(None, now + SLOW));
        t.insert(
            "busy".into(),
            tracked(pr(PrState::Open, ChecksState::Pending), now + FAST),
        );
        assert_eq!(next_tick(&t, now, None), FAST);

        let mut only_merged = HashMap::new();
        only_merged.insert(
            "done".into(),
            tracked(pr(PrState::Merged, ChecksState::Passing), now),
        );
        assert_eq!(next_tick(&only_merged, now, None), SLOW);
    }

    #[test]
    fn next_tick_floors_a_past_due_time() {
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert(
            "overdue".into(),
            tracked(None, now - Duration::from_secs(90)),
        );
        assert_eq!(next_tick(&t, now, None), MIN_TICK);
    }

    #[test]
    fn next_tick_caps_at_slow_so_a_workstream_switch_is_noticed() {
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert("idle".into(), tracked(None, now + BG_IDLE));
        assert_eq!(next_tick(&t, now, None), SLOW);
    }

    #[test]
    fn a_backoff_outranks_every_due_time() {
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert(
            "overdue".into(),
            tracked(pr(PrState::Open, ChecksState::Pending), now),
        );
        assert_eq!(next_tick(&t, now, Some(now + RATE_LIMITED)), SLOW);
        assert_eq!(
            next_tick(&t, now, Some(now - Duration::from_secs(1))),
            MIN_TICK
        );
    }

    #[test]
    fn rate_limit_is_recognised_from_gh_stderr() {
        assert!(is_rate_limited(
            "GraphQL: API rate limit already exceeded for user ID 1"
        ));
        assert!(is_rate_limited("You have exceeded a secondary rate limit"));
        assert!(!is_rate_limited("no pull requests found for branch x"));
    }
}
