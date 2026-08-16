//! Backend poller that watches each active workstream's PR and pushes a
//! [`PrStatusChanged`] event when its status actually changes — so the merge
//! bar flips (e.g. Merge → Archive when the PR lands on GitHub, or the agent
//! actions appear when checks settle) without the frontend polling `gh` on a
//! timer. Attention follows the UI: the workstream on screen is polled on a live
//! cadence, everything else only rarely (see [`BG_OPEN`]). Each result is diffed
//! against the last seen status and emitted only on a real change, so the
//! frontend refetches just the touched workstream.
//!
//! This is the "push" half of the PR status flow: the React Query hook still
//! fetches once on mount (initial paint), then sits idle and lets these events
//! drive every subsequent refresh.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use hyprlayer_core::docs::pr_record;
use hyprlayer_core::error::Result;
use hyprlayer_core::pr::{self, ChecksState, PrState, PrStatus};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::Workstream;
use tauri::Manager;
use tauri_specta::Event;

use crate::events::PrStatusChanged;
use crate::state::AppState;

/// Cadence for the workstream the UI is showing: fast while its checks run,
/// medium while open-but-settled (catch a merge landing elsewhere), slow with
/// no PR. This is the only workstream whose PR any view actually renders.
const FAST: Duration = Duration::from_secs(8);
const MEDIUM: Duration = Duration::from_secs(20);
const SLOW: Duration = Duration::from_secs(60);

/// Cadence for a workstream nobody is looking at. No view reads its PR — the
/// frontend mounts the PR query for the active workstream alone, and the
/// board/dashboard render the stored `pr_number`, not this — so the only reason
/// to poll at all is [`promote_record`] noticing a merge that landed outside the
/// app. An idle branch is checked rarer still: it has no PR to merge, and one
/// appearing is the ship path, which the frontend polls for itself.
///
/// This is the whole quota story. Every `gh pr view` spends a GitHub GraphQL
/// call against 5000/hr, and polling every workstream at the active tier cost
/// 450/hr EACH — a dozen idle branches drained the quota on their own, after
/// which every probe fails and a failed probe reads as "no PR".
const BG_OPEN: Duration = Duration::from_secs(5 * 60);
const BG_IDLE: Duration = Duration::from_secs(20 * 60);

/// Backoff once GitHub reports the quota spent — retrying at the usual cadence
/// just keeps it pinned at zero.
const RATE_LIMITED: Duration = Duration::from_secs(15 * 60);

/// Floor on the loop's sleep, so staggered due-times can't spin the tick.
const MIN_TICK: Duration = Duration::from_secs(4);

/// Cap on concurrent `gh pr view` subprocesses per tick, so a user with many
/// active workstreams doesn't spawn a process storm.
const MAX_CONCURRENT: usize = 6;

/// What the watcher remembers per workstream between ticks.
struct Tracked {
    status: Option<PrStatus>,
    /// When this workstream is next due a poll (its own cadence).
    due: Instant,
}

/// What the watcher carries between ticks.
#[derive(Default)]
struct WatchState {
    tracked: HashMap<String, Tracked>,
    /// The workstream the UI showed last tick, so a switch is a detectable edge
    /// rather than something re-derived (and re-clamped) every tick.
    active: Option<String>,
    /// Nothing is polled before this instant. Deliberately NOT folded into the
    /// per-workstream `due` times: a cadence clamp would keep shortening it,
    /// which is exactly how a quota stays pinned at zero.
    backoff_until: Option<Instant>,
}

/// Start the watcher loop. Runs for the life of the app; the state persists
/// across ticks so we only emit on change.
pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut state = WatchState::default();
        loop {
            // Read focus + the wake handle without holding the State guard across
            // an await.
            let (focused, wake) = {
                let app_state = app.state::<AppState>();
                (
                    app_state.focused.load(Ordering::Relaxed),
                    app_state.focus_changed.clone(),
                )
            };

            // Unfocused: skip the poll entirely (no background `gh` storm). Wake
            // immediately on refocus for a catch-up, or fall back to a SLOW tick.
            // The frontend's focus-refetch already refreshes the visible PR, so a
            // small resume delay on the push side is invisible.
            if !focused {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep(SLOW) => {}
                }
                continue;
            }

            let delay = tick(&app, &mut state).await;
            // Wake early if focus flips (e.g. a blur mid-FAST-sleep) so we
            // re-evaluate instead of finishing the sleep in the background.
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wake.notified() => {}
            }
        }
    });
}

/// Poll the active workstreams that are due, emit on changes, and return how
/// long to wait before the next tick.
async fn tick(app: &tauri::AppHandle, st: &mut WatchState) -> Duration {
    let app_state = app.state::<AppState>();
    let now = Instant::now();

    // Quota spent: poll nothing at all until it lifts. Checked before anything
    // that could shorten it, so the backoff really is a ceiling.
    if let Some(until) = st.backoff_until {
        if now < until {
            return until.saturating_duration_since(now).clamp(MIN_TICK, SLOW);
        }
        st.backoff_until = None;
    }

    // No gh (missing/unauthenticated) → nothing to poll; back off hard.
    let Ok(gh) = app_state.binaries.require(BinaryKind::Gh).await else {
        return SLOW;
    };

    // Active workstreams across all projects — the same set the sidebar shows.
    let db = app_state.db.clone();
    let items =
        match tauri::async_runtime::spawn_blocking(move || db.list_active_workstreams()).await {
            Ok(Ok(items)) => items,
            Ok(Err(e)) => {
                log::debug!("pr watch: listing workstreams failed: {e}");
                return SLOW;
            }
            Err(e) => {
                log::debug!("pr watch: list task join error: {e}");
                return SLOW;
            }
        };

    // The workstream the UI is showing — the route points the single-slot
    // worktree watcher at it, and clears it when no workstream is on screen
    // (`commands::watch_worktree`). `None` means everything is background.
    let active = app_state
        .worktree_watch
        .lock()
        .await
        .as_ref()
        .map(|(id, _)| id.clone());

    // Forget workstreams that were archived/removed since the last tick.
    let mut alive: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(items.len());
    for item in &items {
        alive.insert(item.id.as_str());
    }
    st.tracked.retain(|id, _| alive.contains(id.as_str()));

    // A workstream that just came on screen polls at once — it may have been
    // sitting on a background due time 20 minutes out. On the EDGE only: doing
    // this every tick would also drag the rate-limit backoff forward.
    if active != st.active {
        if let Some(t) = active.as_deref().and_then(|id| st.tracked.get_mut(id)) {
            t.due = now;
        }
        st.active.clone_from(&active);
    }

    // A merged PR is terminal — its status can't change again, so stop polling
    // it (the workstream is usually archived right after anyway). Folder rows
    // have no repo to ask, so gh would fail (or answer for an enclosing repo).
    // Everything else waits for its own `due` time; a workstream first seen this
    // tick has none, so it polls immediately.
    let to_poll: Vec<Workstream> = items
        .into_iter()
        .filter(|item| !item.is_folder())
        .filter(|item| match st.tracked.get(&item.id) {
            Some(t) => !matches!(&t.status, Some(s) if s.state == PrState::Merged) && t.due <= now,
            None => true,
        })
        .collect();
    if to_poll.is_empty() {
        return next_tick(&st.tracked, now, st.backoff_until);
    }

    // Kept alongside the poll because the result rows carry only the workstream
    // id, and promoting a merged PR's record needs the project it belongs to.
    let projects: HashMap<String, String> = to_poll
        .iter()
        .map(|item| (item.id.clone(), item.project_path.clone()))
        .collect();

    let results = poll_all(&gh, to_poll).await;
    let polled_at = Instant::now();
    let mut rate_limited = false;

    for (id, res) in results {
        let status = match res {
            Ok(status) => status,
            // Transient gh failure (network, rate limit, worktree mid-teardown):
            // keep the last known value and don't emit a spurious change.
            Err(e) => {
                rate_limited |= is_rate_limited(&e.to_string());
                log::debug!("pr watch: gh status failed for {id}: {e}");
                // Reschedule anyway — an entry with no due time is polled every
                // tick — and never at FAST: a probe that just failed is the last
                // one worth retrying hardest.
                let is_active = active.as_deref() == Some(id.as_str());
                let due = polled_at
                    + st.tracked
                        .get(&id)
                        .map_or(SLOW, |t| cadence(&t.status, is_active))
                        .max(MEDIUM);
                st.tracked
                    .entry(id)
                    .and_modify(|t| t.due = due)
                    .or_insert(Tracked { status: None, due });
                continue;
            }
        };
        // Emit on a real change and on first sight (so a PR that appears after a
        // ship reaches the UI). First sight only fires a no-op invalidation for
        // unmounted workstreams, so the startup pass is cheap.
        let changed = st.tracked.get(&id).map(|t| &t.status) != Some(&status);
        if changed {
            // Merged is terminal and we stop polling it above, so this is the
            // only tick that ever sees the transition — promote the PR's
            // thoughts record here or never. First sight counts: it's how a
            // merge that landed while the app was closed gets caught up.
            if let Some(s) = status.as_ref().filter(|s| s.state == PrState::Merged) {
                if let Some(project) = projects.get(&id) {
                    promote_record(app, project, s.number).await;
                }
            }
        }
        let due = polled_at + cadence(&status, active.as_deref() == Some(id.as_str()));
        st.tracked.insert(id.clone(), Tracked { status, due });
        if changed {
            let _ = PrStatusChanged { workstream_id: id }.emit(app);
        }
    }

    // One flag for the whole watcher: the quota is per GitHub account, so a
    // refusal on any workstream means every other probe would be refused too.
    st.backoff_until = rate_limited.then(|| {
        log::warn!("pr watch: GitHub API quota spent — holding off {RATE_LIMITED:?}");
        polled_at + RATE_LIMITED
    });

    next_tick(&st.tracked, polled_at, st.backoff_until)
}

/// Whether a `gh` failure is GitHub refusing on quota rather than a real error.
fn is_rate_limited(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("rate limit") || s.contains("secondary rate limit")
}

/// Promote PR `number`'s thoughts record now that it has merged.
///
/// The record is where a PR's description survives after the branch is gone,
/// but nothing outside a `describe_pr` run ever wrote its `status` — so a PR
/// merged from the GitHub UI, `gh pr merge`, or a queued auto-merge left the
/// record claiming the PR was still open. This is the callback that closes it.
///
/// Best-effort and silent on the normal misses: most PRs were never described
/// (no record on disk), `notion`/`anytype` have no local tree to patch, and an
/// unmapped project has no record location at all.
async fn promote_record(app: &tauri::AppHandle, project_path: &str, number: u32) {
    let config = app.state::<AppState>().config.clone();
    let Ok(loaded) = config.load().await else {
        return;
    };
    let Some(thoughts) = loaded.config.thoughts.as_ref() else {
        return;
    };

    let effective = thoughts.effective_config_for(project_path);
    let (Some(root), Some((repos_dir, _)), Some(mapped)) = (
        effective.backend.thoughts_root(),
        effective.backend.tree_dirs(),
        effective.mapped_name.as_deref(),
    ) else {
        return;
    };
    let path = pr_record::record_path(&root, repos_dir, mapped, number);

    // Sync IO (read + patch + write of one small file) off the async runtime.
    match tauri::async_runtime::spawn_blocking(move || pr_record::promote_merged_record(&path))
        .await
    {
        Ok(Ok(true)) => log::info!("pr watch: PR #{number} merged — record promoted"),
        // No record, or its status was already terminal.
        Ok(Ok(false)) => {}
        Ok(Err(e)) => log::debug!("pr watch: promoting PR #{number} record failed: {e}"),
        Err(e) => log::debug!("pr watch: promote task join error: {e}"),
    }
}

/// Run `gh pr view` for each workstream with bounded concurrency, returning
/// `(workstream id, status result)` for each.
async fn poll_all(gh: &Path, items: Vec<Workstream>) -> Vec<(String, Result<Option<PrStatus>>)> {
    use tokio::task::JoinSet;

    let mut set: JoinSet<(String, Result<Option<PrStatus>>)> = JoinSet::new();
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        if set.len() >= MAX_CONCURRENT {
            if let Some(Ok(r)) = set.join_next().await {
                results.push(r);
            }
        }
        let gh = gh.to_path_buf();
        set.spawn(async move {
            let res = pr::pr_status(&gh, Path::new(&item.worktree_path), &item.pr_ref()).await;
            (item.id, res)
        });
    }
    while let Some(joined) = set.join_next().await {
        if let Ok(r) = joined {
            results.push(r);
        }
    }
    results
}

/// How long one workstream waits before its next poll, from its own PR and
/// whether the UI is showing it.
fn cadence(status: &Option<PrStatus>, active: bool) -> Duration {
    match (active, status) {
        (false, Some(s)) if s.state == PrState::Open => BG_OPEN,
        (false, _) => BG_IDLE,
        (true, Some(s)) if s.state == PrState::Open && s.checks == ChecksState::Pending => FAST,
        (true, Some(s)) if s.state == PrState::Open => MEDIUM,
        (true, _) => SLOW,
    }
}

/// Sleep until the earliest due workstream — never before `backoff_until`,
/// floored so a spread of due times can't spin the loop, and capped at `SLOW`
/// so a workstream switch is noticed promptly. Nothing is due on such a tick,
/// so the ceiling costs no `gh` calls.
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
    use hyprlayer_core::pr::MergeableState;

    fn pr(state: PrState, checks: ChecksState) -> Option<PrStatus> {
        Some(PrStatus {
            number: 1,
            url: String::new(),
            title: String::new(),
            state,
            mergeable: true,
            mergeable_state: MergeableState::Mergeable,
            checks,
            checks_total: 0,
            checks_passed: 0,
            checks_failed: 0,
            checks_list: Vec::new(),
            review_decision: None,
        })
    }

    fn tracked(status: Option<PrStatus>, due: Instant) -> Tracked {
        Tracked { status, due }
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
    fn background_workstreams_are_polled_rarely_whatever_their_checks() {
        // Running checks on a branch nobody is looking at buy it nothing: no
        // view renders its PR, so it's watched only for a merge landing.
        assert_eq!(
            cadence(&pr(PrState::Open, ChecksState::Pending), false),
            BG_OPEN
        );
        assert_eq!(
            cadence(&pr(PrState::Open, ChecksState::Passing), false),
            BG_OPEN
        );
        // No PR at all → nothing to merge, and a new one is the ship path.
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

        // Merged is terminal — its due time must not drag the loop awake.
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
        // All background: the soonest due is 20 minutes out, but the loop still
        // has to wake to see the user switch workstreams. The tick it wakes for
        // polls nothing, so the ceiling is free.
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert("idle".into(), tracked(None, now + BG_IDLE));
        assert_eq!(next_tick(&t, now, None), SLOW);
    }

    #[test]
    fn a_backoff_outranks_every_due_time() {
        // The regression that made RATE_LIMITED cosmetic: a due time inside the
        // backoff must not pull the loop forward into polling a spent quota.
        // The 60s ceiling still applies — that wake polls nothing (`tick`
        // returns early while the backoff holds), it just re-checks the clock.
        let now = Instant::now();
        let mut t = HashMap::new();
        t.insert(
            "overdue".into(),
            tracked(pr(PrState::Open, ChecksState::Pending), now),
        );
        assert_eq!(next_tick(&t, now, Some(now + RATE_LIMITED)), SLOW);
        // An expired backoff is inert.
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
