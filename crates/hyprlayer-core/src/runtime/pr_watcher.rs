use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};

use crate::config::ConfigService;
use crate::error::CoreError;
use crate::gitops;
use crate::index::watcher::WatchHandle;
use crate::pr::{self, PrLookup, PrLookupFailure, PrState, PrStatus, Quota};
use crate::process::binaries::{BinaryKind, BinaryResolver};
use crate::store::{now_ms, Db, ObservedPr, Workstream};
use hyprlayer_models::RuntimeEvent;

use super::doc_status;
use super::shutdown::TrackedTask;
use super::workstreams::{current_pr_ref, has_own_pr, pr_dir, workstream_pr};

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

const QUOTA_FLOOR: u32 = 50;

// Reported on the change, not every tick.
static GH_MISSING_REPORTED: AtomicBool = AtomicBool::new(false);
static SIGNED_OUT_REPORTED: AtomicBool = AtomicBool::new(false);

/// Every workstream a client watches; `None` where there was nothing to watch.
pub type WorktreeWatch = Arc<AsyncMutex<HashMap<String, Option<WatchHandle>>>>;

pub(crate) struct PrEntry {
    pub(crate) pr_ref: String,
    pub(crate) lookup: PrLookup,
    written: Instant,
}

pub(crate) type PrCache = Arc<RwLock<HashMap<String, PrEntry>>>;

pub(crate) fn cached_lookup(cache: &PrCache, id: &str, pr_ref: &str) -> Option<PrLookup> {
    let cache = cache.read().unwrap_or_else(PoisonError::into_inner);
    cache
        .get(id)
        .filter(|e| e.pr_ref == pr_ref)
        .map(|e| e.lookup.clone())
}

pub(crate) fn cached_lookup_any_ref(cache: &PrCache, id: &str) -> Option<(String, PrLookup)> {
    let cache = cache.read().unwrap_or_else(PoisonError::into_inner);
    cache.get(id).map(|e| (e.pr_ref.clone(), e.lookup.clone()))
}

pub(crate) fn cached_age(cache: &PrCache, id: &str) -> Option<Duration> {
    let cache = cache.read().unwrap_or_else(PoisonError::into_inner);
    cache.get(id).map(|e| e.written.elapsed())
}

pub(crate) fn edit_cached_pr(cache: &PrCache, id: &str, edit: impl FnOnce(&mut PrStatus)) {
    let mut cache = cache.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(PrLookup::Found { pr } | PrLookup::Failed { last: Some(pr), .. }) =
        cache.get_mut(id).map(|e| &mut e.lookup)
    {
        edit(pr);
    }
}

pub(crate) fn store_lookup(
    cache: &PrCache,
    id: &str,
    pr_ref: &str,
    next: PrLookup,
    keep_last: bool,
    started: Instant,
) -> Option<(PrLookup, bool)> {
    let mut cache = cache.write().unwrap_or_else(PoisonError::into_inner);
    if cache.get(id).is_some_and(|e| started < e.written) {
        return None;
    }
    let same_ref = cache.get(id).filter(|e| e.pr_ref == pr_ref);
    let next = match next {
        PrLookup::Failed { cause, last } if keep_last => PrLookup::Failed {
            cause,
            last: same_ref.and_then(|e| e.lookup.known().cloned()).or(last),
        },
        next => next,
    };
    let changed = same_ref.is_none_or(|e| e.lookup != next);
    cache.insert(
        id.to_string(),
        PrEntry {
            pr_ref: pr_ref.to_string(),
            lookup: next.clone(),
            written: Instant::now(),
        },
    );
    Some((next, changed))
}

#[derive(Default)]
pub(crate) struct PrBudget(std::sync::Mutex<Budget>);

#[derive(Default)]
struct Budget {
    quota: Option<Quota>,
    watcher_backoff_until: Option<f64>,
}

impl PrBudget {
    fn lock(&self) -> std::sync::MutexGuard<'_, Budget> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn record(&self, quota: Quota) {
        self.lock().quota = Some(quota);
    }

    fn quota_spent(&self, now: f64) {
        let mut budget = self.lock();
        let reset_at = budget
            .quota
            .and_then(|q| q.reset_at)
            .filter(|&reset| reset > now)
            .unwrap_or(now + RATE_LIMITED.as_millis() as f64);
        if budget.quota.is_none_or(|q| q.remaining > 0) {
            let wait = Duration::from_millis((reset_at - now) as u64);
            tracing::warn!("pr watch: GitHub API quota spent, holding off {wait:?}");
        }
        budget.quota = Some(Quota {
            limit: budget.quota.map_or(0, |q| q.limit),
            remaining: 0,
            reset_at: Some(reset_at),
        });
    }

    fn secondary_limited(&self, now: f64) {
        let mut budget = self.lock();
        if budget
            .watcher_backoff_until
            .is_none_or(|until| until <= now)
        {
            tracing::warn!(
                "pr watch: GitHub secondary rate limit, polling resumes in {RATE_LIMITED:?}"
            );
        }
        budget.watcher_backoff_until = Some(now + RATE_LIMITED.as_millis() as f64);
    }

    pub(crate) fn held(&self, now: f64, background: bool) -> Option<f64> {
        let quota = self.lock().quota?;
        let reset = quota.reset_at.filter(|&reset| reset > now)?;
        let low = quota.remaining < QUOTA_FLOOR
            || (background && u64::from(quota.remaining) * 10 < u64::from(quota.limit));
        low.then_some(reset)
    }

    fn watcher_backoff(&self, now: f64) -> Option<f64> {
        self.lock()
            .watcher_backoff_until
            .filter(|&until| until > now)
    }
}

fn note_rate_limit(budget: &PrBudget, text: &str) -> Option<PrLookupFailure> {
    let now = now_ms();
    if is_secondary_limit(text) {
        budget.secondary_limited(now);
        Some(PrLookupFailure::Other)
    } else if is_rate_limited(text) {
        budget.quota_spent(now);
        Some(PrLookupFailure::RateLimited {
            resumes_at: budget.held(now, false),
        })
    } else {
        None
    }
}

pub(crate) fn failure_cause(e: &CoreError, budget: &PrBudget) -> PrLookupFailure {
    let text = e.to_string();
    note_rate_limit(budget, &text).unwrap_or_else(|| {
        if crate::repos::is_github_auth_failure(&text) {
            PrLookupFailure::SignedOut
        } else {
            PrLookupFailure::Other
        }
    })
}

#[derive(Default)]
pub(crate) struct PrKick {
    ids: std::sync::Mutex<HashSet<String>>,
    notify: Notify,
}

impl PrKick {
    pub(crate) fn kick(&self, id: &str) {
        self.ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_string());
        self.notify.notify_one();
    }

    fn take(&self) -> HashSet<String> {
        std::mem::take(&mut *self.ids.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

pub struct PrWatcherContext {
    pub binaries: Arc<BinaryResolver>,
    pub db: Arc<Db>,
    pub config: Arc<ConfigService>,
    pub worktree_watch: WorktreeWatch,
    pub focused: Arc<AtomicBool>,
    pub focus_changed: Arc<Notify>,
    pub events: broadcast::Sender<RuntimeEvent>,
    pub env_overlay: Arc<RwLock<BTreeMap<String, String>>>,
    pub(crate) pr_cache: PrCache,
    pub(crate) kick: Arc<PrKick>,
    pub(crate) budget: Arc<PrBudget>,
}

struct Tracked {
    status: Option<PrStatus>,
    due: Instant,
    pr_ref: String,
    /// A moved-to ref whose lookup failed: it waits out `due` instead of retrying every tick.
    failed_ref: Option<String>,
    settled: Option<Settled>,
}

struct Settled {
    number: u32,
    state: PrState,
    head: String,
    tip_then: Option<String>,
}

impl Settled {
    fn of(status: &PrStatus, item: &Workstream, pr_ref: &str) -> Option<Self> {
        (status.state != PrState::Open).then(|| Settled {
            number: status.number,
            state: status.state,
            head: status.head_ref_oid.clone(),
            tip_then: gitops::local_tip(pr_dir(item), pr_ref),
        })
    }
}

#[derive(Default)]
struct WatchState {
    tracked: HashMap<String, Tracked>,
    active: HashSet<String>,
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
                    _ = ctx.kick.notify.notified() => {}
                    _ = tokio::time::sleep(SLOW) => {}
                    _ = stop.notified() => return,
                }
                continue;
            }

            let delay = tick(&ctx, &mut state).await;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = ctx.focus_changed.notified() => {}
                _ = ctx.kick.notify.notified() => {}
                _ = stop.notified() => return,
            }
        }
    })
}

fn settled_from_row(item: &Workstream, now: Instant) -> Option<Tracked> {
    let observed = item.observed_pr.filter(|o| o.state != PrState::Open)?;
    let head = item.observed_pr_head.clone()?;
    if item.is_folder() {
        return None;
    }
    let pr_ref =
        current_pr_ref(item).filter(|r| item.observed_ref.as_deref() == Some(r.as_str()))?;
    let tip_then = item.observed_pr_tip.clone();
    Some(Tracked {
        status: None,
        due: now + SLOW,
        pr_ref,
        failed_ref: None,
        settled: Some(Settled {
            number: observed.number,
            state: observed.state,
            head,
            tip_then,
        }),
    })
}

fn unsettle_if_the_cache_disagrees(t: &mut Tracked, cached: Option<PrLookup>, now: Instant) {
    let Some(s) = &t.settled else { return };
    let disagrees = match cached {
        Some(PrLookup::Failed { .. }) => true,
        Some(PrLookup::Found { pr }) => pr.number != s.number,
        _ => false,
    };
    if disagrees {
        t.settled = None;
        t.due = now;
    }
}

fn mark_open_prs_stale(cache: &PrCache, resumes: f64) -> Vec<String> {
    let mut cache = cache.write().unwrap_or_else(PoisonError::into_inner);
    let mut marked = Vec::new();
    for (id, entry) in cache.iter_mut() {
        let stale = match &entry.lookup {
            PrLookup::Found { pr } if pr.state == PrState::Open => PrLookup::Failed {
                cause: PrLookupFailure::RateLimited {
                    resumes_at: Some(resumes),
                },
                last: Some(pr.clone()),
            },
            _ => continue,
        };
        entry.lookup = stale;
        marked.push(id.clone());
    }
    marked
}

fn until(epoch_ms: f64) -> Duration {
    Duration::from_millis((epoch_ms - now_ms()).max(0.0) as u64)
}

async fn tick(ctx: &PrWatcherContext, st: &mut WatchState) -> Duration {
    let now = Instant::now();

    if let Some(resumes) = hold(ctx) {
        return until(resumes).clamp(MIN_TICK, SLOW);
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

    let alive: HashSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
    st.tracked.retain(|id, _| alive.contains(id.as_str()));
    ctx.pr_cache
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|id, _| alive.contains(id.as_str()));

    for id in active.difference(&st.active) {
        if let Some(t) = st.tracked.get_mut(id) {
            t.due = now;
        }
    }
    st.active.clone_from(&active);
    let kicked = ctx.kick.take();

    let projects: HashMap<String, String> = items
        .iter()
        .filter(|item| has_own_pr(item))
        .map(|item| (item.id.clone(), item.project_path.clone()))
        .collect();

    for item in &items {
        if !st.tracked.contains_key(&item.id) {
            if let Some(t) = settled_from_row(item, now) {
                st.tracked.insert(item.id.clone(), t);
            }
        }
    }
    for (id, t) in st.tracked.iter_mut() {
        unsettle_if_the_cache_disagrees(t, cached_lookup(&ctx.pr_cache, id, &t.pr_ref), now);
    }

    // A merged PR is never polled again, but its record can land after the merge.
    let resettle: Vec<(String, u32)> = st
        .tracked
        .iter()
        .filter_map(|(id, t)| match &t.settled {
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
            let kicked = kicked.contains(&item.id);
            let not_github = matches!(
                cached_lookup(&ctx.pr_cache, &item.id, &pr_ref),
                Some(PrLookup::NotGitHub)
            );
            if not_github && !kicked {
                return None;
            }
            let work_past = |s: &Settled| {
                item.pr_number.is_none()
                    && gitops::has_work_past(pr_dir(&item), &pr_ref, &s.head, s.tip_then.as_deref())
            };
            let due = is_due(st.tracked.get(&item.id), &pr_ref, now, kicked, work_past);
            let held = ctx
                .budget
                .held(now_ms(), !active.contains(&item.id))
                .is_some();
            (due && !held).then_some((item, pr_ref))
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
    let results = poll_all(&gh, to_poll, &overlay, &ctx.budget, &active).await;
    let polled_at = Instant::now();

    for (item, pr_ref, lookup) in results {
        let id = item.id.clone();
        let moved = item
            .observed_ref
            .as_deref()
            .is_some_and(|seen| seen != pr_ref);
        let Some((_, entry_changed)) =
            store_lookup(&ctx.pr_cache, &id, &pr_ref, lookup.clone(), !moved, now)
        else {
            continue;
        };
        let status = match lookup {
            PrLookup::Found { pr } => Some(pr),
            PrLookup::NoPr | PrLookup::NotGitHub | PrLookup::Unknown => None,
            PrLookup::Failed { cause, .. } => {
                let rate_limited = matches!(cause, PrLookupFailure::RateLimited { .. });
                if cause == PrLookupFailure::SignedOut
                    && !SIGNED_OUT_REPORTED.swap(true, Ordering::Relaxed)
                {
                    tracing::warn!("pr watch: gh is not signed in; run `gh auth login`");
                }
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
                    settled: None,
                });
                if !rate_limited {
                    t.due = polled_at + wait;
                }
                t.settled = None;
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
                } else if entry_changed {
                    let _ = ctx.events.send(RuntimeEvent::PrStatusChanged {
                        workstream_id: id,
                        observed_pr_changed: false,
                    });
                }
                continue;
            }
        };
        if SIGNED_OUT_REPORTED.swap(false, Ordering::Relaxed) {
            tracing::info!("pr watch: gh is signed in again");
        }
        let changed = st.tracked.get(&id).map(|t| &t.status) != Some(&status);
        if let Some(s) = status.as_ref().filter(|s| s.state != PrState::Open) {
            if let Some(project) = projects
                .get(&id)
                .filter(|_| settles_again(&item, &pr_ref, s))
            {
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
        let settled = status.as_ref().and_then(|s| Settled::of(s, &item, &pr_ref));
        let own = status.as_ref().filter(|_| projects.contains_key(&id));
        let observed = own.map(|s| ObservedPr {
            number: s.number,
            state: s.state,
        });
        let head = own.map(|s| s.head_ref_oid.clone());
        let tip = own.and(settled.as_ref()).and_then(|s| s.tip_then.clone());
        let recorded = record_observed(ctx.db.clone(), &id, observed, &pr_ref, head, tip).await;
        let due = polled_at + cadence(&status, active.contains(&id));
        st.tracked.insert(
            id.clone(),
            Tracked {
                status,
                due,
                pr_ref,
                failed_ref: None,
                settled,
            },
        );
        if changed || recorded || entry_changed {
            let _ = ctx.events.send(RuntimeEvent::PrStatusChanged {
                workstream_id: id,
                observed_pr_changed: recorded,
            });
        }
    }

    let sweep_due = st
        .swept_at
        .is_none_or(|at| polled_at.saturating_duration_since(at) >= RECORD_SWEEP);
    if sweep_due && ctx.budget.held(now_ms(), true).is_none() {
        st.swept_at = Some(polled_at);
        let tracked: HashSet<(String, u32)> = st
            .tracked
            .iter()
            .filter_map(|(id, t)| {
                let number = t
                    .status
                    .as_ref()
                    .map(|s| s.number)
                    .or(t.settled.as_ref().map(|s| s.number))?;
                Some((projects.get(id)?.clone(), number))
            })
            .collect();
        sweep_records(ctx, &gh, &overlay, &tracked, &mut st.unresolved).await;
    }

    let held_until = hold(ctx).map(|resumes| polled_at + until(resumes));
    next_tick(&st.tracked, polled_at, held_until)
}

fn hold(ctx: &PrWatcherContext) -> Option<f64> {
    let Some(resumes) = ctx.budget.held(now_ms(), false) else {
        return ctx.budget.watcher_backoff(now_ms());
    };
    for id in mark_open_prs_stale(&ctx.pr_cache, resumes) {
        let _ = ctx.events.send(RuntimeEvent::PrStatusChanged {
            workstream_id: id,
            observed_pr_changed: false,
        });
    }
    Some(resumes)
}

fn is_due(
    t: Option<&Tracked>,
    pr_ref: &str,
    now: Instant,
    kicked: bool,
    work_past: impl FnOnce(&Settled) -> bool,
) -> bool {
    let Some(t) = t else { return true };
    if t.pr_ref != pr_ref && t.failed_ref.as_deref() != Some(pr_ref) {
        return true;
    }
    let settled = t.pr_ref == pr_ref && t.settled.as_ref().is_some_and(|s| !work_past(s));
    kicked || (!settled && t.due <= now)
}

fn settles_again(row_at_tick_start: &Workstream, pr_ref: &str, s: &PrStatus) -> bool {
    row_at_tick_start.observed_ref.as_deref() != Some(pr_ref)
        || row_at_tick_start.observed_pr
            != Some(ObservedPr {
                number: s.number,
                state: s.state,
            })
}

async fn record_observed(
    db: Arc<Db>,
    id: &str,
    observed: Option<ObservedPr>,
    pr_ref: &str,
    head: Option<String>,
    tip: Option<String>,
) -> bool {
    let (wid, reference) = (id.to_string(), pr_ref.to_string());
    let written = tokio::task::spawn_blocking(move || {
        db.record_pr_observation(&wid, observed, &reference, head.as_deref(), tip.as_deref())
    })
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

pub(crate) fn is_rate_limited(stderr: &str) -> bool {
    stderr.to_lowercase().contains("rate limit")
}

fn is_secondary_limit(stderr: &str) -> bool {
    stderr.to_lowercase().contains("secondary rate limit")
}

/// Settles PR records no live workstream tracks (opened elsewhere, or archived before the merge
/// was seen).
async fn sweep_records(
    ctx: &PrWatcherContext,
    gh: &Path,
    overlay: &[(String, String)],
    tracked: &HashSet<(String, u32)>,
    unresolved: &mut HashSet<PathBuf>,
) {
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
            Err(e) if note_rate_limit(&ctx.budget, &e.to_string()).is_some() => return,
            Err(e) => {
                tracing::debug!("pr watch: looking up PR #{number} for its record failed: {e}")
            }
        }
    }
}

async fn poll_all(
    gh: &Path,
    items: Vec<(Workstream, String)>,
    overlay: &[(String, String)],
    budget: &Arc<PrBudget>,
    active: &HashSet<String>,
) -> Vec<(Workstream, String, PrLookup)> {
    use tokio::task::JoinSet;

    let mut set: JoinSet<(Workstream, String, PrLookup)> = JoinSet::new();
    let mut results = Vec::with_capacity(items.len());
    for (item, pr_ref) in items {
        if set.len() >= MAX_CONCURRENT {
            if let Some(Ok(r)) = set.join_next().await {
                results.push(r);
            }
        }
        let gh = gh.to_path_buf();
        let overlay = overlay.to_vec();
        let budget = budget.clone();
        let background = !active.contains(&item.id);
        set.spawn(async move {
            let lookup = workstream_pr(&gh, &item, &pr_ref, &overlay, &budget, background).await;
            (item, pr_ref, lookup)
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

fn next_tick(tracked: &HashMap<String, Tracked>, now: Instant, hold: Option<Instant>) -> Duration {
    let soonest = tracked
        .values()
        .filter(|t| t.settled.is_none())
        .map(|t| t.due)
        .min()
        .unwrap_or(now + SLOW);
    hold.map_or(soonest, |h| soonest.max(h))
        .saturating_duration_since(now)
        .clamp(MIN_TICK, SLOW)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::{ChecksState, MergeState, PrChecks};

    fn pr(state: PrState, checks: ChecksState) -> Option<PrStatus> {
        Some(PrStatus {
            number: 1,
            url: String::new(),
            title: String::new(),
            state,
            is_draft: false,
            base_ref_name: String::new(),
            head_ref_oid: String::new(),
            merge_state: MergeState::Unknown,
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
            reviews: Vec::new(),
        })
    }

    fn found(number: u32, state: PrState) -> PrLookup {
        let mut status = pr(state, ChecksState::None).unwrap();
        status.number = number;
        PrLookup::Found { pr: status }
    }

    fn tracked(status: Option<PrStatus>, due: Instant) -> Tracked {
        Tracked {
            status,
            due,
            pr_ref: String::new(),
            failed_ref: None,
            settled: None,
        }
    }

    fn settled(due: Instant) -> Tracked {
        Tracked {
            settled: Some(Settled {
                number: 1,
                state: PrState::Merged,
                head: "h".into(),
                tip_then: None,
            }),
            ..tracked(pr(PrState::Merged, ChecksState::Passing), due)
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
    fn next_tick_wakes_for_the_soonest_and_skips_settled() {
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert("idle".into(), tracked(None, now + SLOW));
        t.insert(
            "busy".into(),
            tracked(pr(PrState::Open, ChecksState::Pending), now + FAST),
        );
        assert_eq!(next_tick(&t, now, None), FAST);

        let mut only_settled = HashMap::new();
        only_settled.insert("done".into(), settled(now));
        assert_eq!(next_tick(&only_settled, now, None), SLOW);
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
    fn due_rules_for_moved_settled_and_kicked_entries() {
        let now = Instant::now();
        let past = now - Duration::from_secs(1);
        let later = now + SLOW;
        let no_work = |_: &Settled| false;
        let new_work = |_: &Settled| true;

        assert!(is_due(None, "a", now, false, no_work));

        let mut moved = tracked(None, later);
        moved.pr_ref = "a".into();
        assert!(is_due(Some(&moved), "b", now, false, no_work));
        moved.failed_ref = Some("b".into());
        assert!(!is_due(Some(&moved), "b", now, false, no_work));
        moved.due = past;
        assert!(is_due(Some(&moved), "b", now, false, no_work));

        let mut done = settled(past);
        done.pr_ref = "a".into();
        assert!(!is_due(Some(&done), "a", now, false, no_work));
        assert!(is_due(Some(&done), "a", now, false, new_work));
        assert!(is_due(Some(&done), "a", now, true, no_work));
        done.due = later;
        assert!(!is_due(Some(&done), "a", now, false, new_work));
    }

    #[test]
    fn docs_settle_once_per_number_state_and_ref() {
        let mut item = crate::store::Workstream {
            observed_ref: Some("feat".into()),
            observed_pr: Some(ObservedPr {
                number: 1,
                state: PrState::Merged,
            }),
            ..test_workstream()
        };
        let merged = pr(PrState::Merged, ChecksState::None).unwrap();
        assert!(!settles_again(&item, "feat", &merged));
        assert!(settles_again(&item, "other", &merged));
        let closed = pr(PrState::Closed, ChecksState::None).unwrap();
        assert!(settles_again(&item, "feat", &closed));
        let mut renumbered = merged.clone();
        renumbered.number = 2;
        assert!(settles_again(&item, "feat", &renumbered));
        item.observed_pr = None;
        assert!(settles_again(&item, "feat", &merged));
    }

    fn test_workstream() -> Workstream {
        let db = Db::open_in_memory().unwrap();
        db.insert_workstream(crate::store::NewWorkstream {
            project_path: "/repo".into(),
            title: "t".into(),
            slug: "t-1".into(),
            branch: "feat".into(),
            worktree_path: "/wt".into(),
            base_branch: "origin/main".into(),
            worktree_mode: crate::store::WorktreeMode::Isolated,
        })
        .unwrap()
    }

    #[test]
    fn a_stored_lookup_keeps_the_last_pr_only_for_its_own_ref() {
        let cache = PrCache::default();
        let rate_limited = |last| PrLookup::Failed {
            cause: PrLookupFailure::RateLimited { resumes_at: None },
            last,
        };
        store_lookup(
            &cache,
            "w",
            "a",
            found(7, PrState::Open),
            true,
            Instant::now(),
        );

        let (stale, changed) =
            store_lookup(&cache, "w", "a", rate_limited(None), true, Instant::now()).unwrap();
        assert!(changed);
        assert_eq!(stale.known().map(|p| p.number), Some(7));

        let row = found(3, PrState::Merged).known().cloned();
        let (other, _) =
            store_lookup(&cache, "w", "b", rate_limited(row), true, Instant::now()).unwrap();
        assert_eq!(other.known().map(|p| p.number), Some(3));
    }

    #[test]
    fn an_answer_never_replaces_an_entry_written_after_its_lookup_started() {
        let failed_with_last = PrLookup::Failed {
            cause: PrLookupFailure::Other,
            last: found(42, PrState::Open).known().cloned(),
        };
        for (newer_ref, newer, late) in [
            ("a", found(7, PrState::Merged), found(7, PrState::Open)),
            ("b", found(42, PrState::Open), PrLookup::NoPr),
            ("a", failed_with_last, PrLookup::NoPr),
        ] {
            let cache = PrCache::default();
            let started = Instant::now();
            store_lookup(&cache, "w", newer_ref, newer.clone(), true, Instant::now());
            assert_eq!(store_lookup(&cache, "w", "a", late, true, started), None);
            assert_eq!(cached_lookup(&cache, "w", newer_ref), Some(newer));
        }
    }

    #[test]
    fn new_work_past_a_merge_replaces_the_settled_pr() {
        let cache = PrCache::default();
        store_lookup(
            &cache,
            "w",
            "a",
            found(7, PrState::Merged),
            true,
            Instant::now(),
        );
        let (next, changed) =
            store_lookup(&cache, "w", "a", PrLookup::NoPr, true, Instant::now()).unwrap();
        assert_eq!(next, PrLookup::NoPr);
        assert!(changed);
        assert_eq!(cached_lookup(&cache, "w", "a"), Some(PrLookup::NoPr));
        assert_eq!(cached_lookup(&cache, "w", "b"), None);
    }

    #[test]
    fn a_failed_or_different_answer_unsettles_a_settled_entry() {
        let now = Instant::now();
        let later = now + SLOW;
        let failed = PrLookup::Failed {
            cause: PrLookupFailure::Other,
            last: None,
        };
        for (cached, unsettled, due) in [
            (Some(failed), true, now),
            (Some(found(2, PrState::Open)), true, now),
            (Some(found(1, PrState::Merged)), false, later),
            (Some(PrLookup::NoPr), false, later),
            (None, false, later),
        ] {
            let mut t = settled(later);
            unsettle_if_the_cache_disagrees(&mut t, cached, now);
            assert_eq!((t.settled.is_none(), t.due), (unsettled, due));
        }
    }

    #[test]
    fn a_quota_hold_marks_open_prs_stale_once_and_leaves_settled_ones() {
        let cache = PrCache::default();
        let at = Instant::now();
        store_lookup(&cache, "open", "a", found(7, PrState::Open), true, at);
        store_lookup(&cache, "merged", "b", found(8, PrState::Merged), true, at);
        store_lookup(&cache, "none", "c", PrLookup::NoPr, true, at);

        assert_eq!(mark_open_prs_stale(&cache, 5.0), vec!["open".to_string()]);
        let stale = cached_lookup(&cache, "open", "a").unwrap();
        assert_eq!(
            stale,
            PrLookup::Failed {
                cause: PrLookupFailure::RateLimited {
                    resumes_at: Some(5.0)
                },
                last: found(7, PrState::Open).known().cloned(),
            }
        );
        assert_eq!(
            cached_lookup(&cache, "merged", "b"),
            Some(found(8, PrState::Merged))
        );
        assert!(mark_open_prs_stale(&cache, 5.0).is_empty());
    }

    #[test]
    fn the_budget_holds_background_lookups_first_then_all() {
        let now = 1_000_000.0;
        let reset = now + 60_000.0;
        let budget = PrBudget::default();
        assert_eq!(budget.held(now, true), None);

        budget.record(Quota {
            limit: 5000,
            remaining: 400,
            reset_at: Some(reset),
        });
        assert_eq!(budget.held(now, true), Some(reset));
        assert_eq!(budget.held(now, false), None);

        budget.record(Quota {
            limit: 5000,
            remaining: 49,
            reset_at: Some(reset),
        });
        assert_eq!(budget.held(now, false), Some(reset));
        assert_eq!(budget.held(reset + 1.0, true), None);

        budget.quota_spent(now);
        assert_eq!(budget.held(now, false), Some(reset));

        let fresh = PrBudget::default();
        fresh.quota_spent(now);
        assert_eq!(
            fresh.held(now, false),
            Some(now + RATE_LIMITED.as_millis() as f64)
        );
    }

    #[test]
    fn a_secondary_limit_holds_only_the_watcher_and_not_as_low_quota() {
        let budget = PrBudget::default();
        let now = now_ms();
        budget.record(Quota {
            limit: 5000,
            remaining: 4900,
            reset_at: Some(now + 55.0 * 60_000.0),
        });
        let err = CoreError::Exec {
            program: "gh".into(),
            exit: Some(1),
            stderr_tail: "gh: You have exceeded a secondary rate limit. Please wait a few minutes before you try again.".into(),
        };
        assert_eq!(failure_cause(&err, &budget), PrLookupFailure::Other);
        assert_eq!(budget.held(now_ms(), false), None);
        assert_eq!(budget.held(now_ms(), true), None);
        let watcher_minutes = budget
            .watcher_backoff(now_ms())
            .map(|until| ((until - now_ms()) / 60_000.0).round());
        assert_eq!(watcher_minutes, Some(15.0));

        let spent = CoreError::Exec {
            program: "gh".into(),
            exit: Some(1),
            stderr_tail: "gh: API rate limit already exceeded for user ID 1.".into(),
        };
        assert!(matches!(
            failure_cause(&spent, &budget),
            PrLookupFailure::RateLimited {
                resumes_at: Some(_)
            }
        ));
        assert!(budget.held(now_ms(), false).is_some());
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
