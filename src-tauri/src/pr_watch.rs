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

const FAST: Duration = Duration::from_secs(8);
const MEDIUM: Duration = Duration::from_secs(20);
const SLOW: Duration = Duration::from_secs(60);

const BG_OPEN: Duration = Duration::from_secs(5 * 60);
const BG_IDLE: Duration = Duration::from_secs(20 * 60);

const RATE_LIMITED: Duration = Duration::from_secs(15 * 60);

const MIN_TICK: Duration = Duration::from_secs(4);

const MAX_CONCURRENT: usize = 6;

struct Tracked {
    status: Option<PrStatus>,
    due: Instant,
}

#[derive(Default)]
struct WatchState {
    tracked: HashMap<String, Tracked>,
    active: Option<String>,
    backoff_until: Option<Instant>,
}

pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut state = WatchState::default();
        loop {
            let (focused, wake) = {
                let app_state = app.state::<AppState>();
                (
                    app_state.focused.load(Ordering::Relaxed),
                    app_state.focus_changed.clone(),
                )
            };

            if !focused {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep(SLOW) => {}
                }
                continue;
            }

            let delay = tick(&app, &mut state).await;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wake.notified() => {}
            }
        }
    });
}

async fn tick(app: &tauri::AppHandle, st: &mut WatchState) -> Duration {
    let app_state = app.state::<AppState>();
    let now = Instant::now();

    if let Some(until) = st.backoff_until {
        if now < until {
            return until.saturating_duration_since(now).clamp(MIN_TICK, SLOW);
        }
        st.backoff_until = None;
    }

    let Ok(gh) = app_state.binaries.require(BinaryKind::Gh).await else {
        return SLOW;
    };

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

    let active = app_state
        .worktree_watch
        .lock()
        .await
        .as_ref()
        .map(|(id, _)| id.clone());

    let mut alive: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(items.len());
    for item in &items {
        alive.insert(item.id.as_str());
    }
    st.tracked.retain(|id, _| alive.contains(id.as_str()));

    if active != st.active {
        if let Some(t) = active.as_deref().and_then(|id| st.tracked.get_mut(id)) {
            t.due = now;
        }
        st.active.clone_from(&active);
    }

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
            Err(e) => {
                rate_limited |= is_rate_limited(&e.to_string());
                log::debug!("pr watch: gh status failed for {id}: {e}");
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
        let changed = st.tracked.get(&id).map(|t| &t.status) != Some(&status);
        if changed {
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

    st.backoff_until = rate_limited.then(|| {
        log::warn!("pr watch: GitHub API quota spent, holding off {RATE_LIMITED:?}");
        polled_at + RATE_LIMITED
    });

    next_tick(&st.tracked, polled_at, st.backoff_until)
}

fn is_rate_limited(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("rate limit") || s.contains("secondary rate limit")
}

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

    match tauri::async_runtime::spawn_blocking(move || pr_record::promote_merged_record(&path))
        .await
    {
        Ok(Ok(true)) => log::info!("pr watch: PR #{number} merged, record promoted"),
        Ok(Ok(false)) => {}
        Ok(Err(e)) => log::debug!("pr watch: promoting PR #{number} record failed: {e}"),
        Err(e) => log::debug!("pr watch: promote task join error: {e}"),
    }
}

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

fn cadence(status: &Option<PrStatus>, active: bool) -> Duration {
    match (active, status) {
        (false, Some(s)) if s.state == PrState::Open => BG_OPEN,
        (false, _) => BG_IDLE,
        (true, Some(s)) if s.state == PrState::Open && s.checks == ChecksState::Pending => FAST,
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
