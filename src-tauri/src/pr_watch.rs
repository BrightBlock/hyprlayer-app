//! Backend poller that watches each active workstream's PR and pushes a
//! [`PrStatusChanged`] event when its status actually changes — so the merge
//! bar flips (e.g. Merge → Archive when the PR lands on GitHub, or the agent
//! actions appear when checks settle) without the frontend polling `gh` on a
//! timer. One poll covers every workstream; we diff each result against the
//! last seen status and emit only on a real change, so the frontend refetches
//! just the touched workstream.
//!
//! This is the "push" half of the PR status flow: the React Query hook still
//! fetches once on mount (initial paint), then sits idle and lets these events
//! drive every subsequent refresh.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use hyprlayer_core::error::Result;
use hyprlayer_core::pr::{self, ChecksState, PrState, PrStatus};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::Workstream;
use tauri::Manager;
use tauri_specta::Event;

use crate::events::PrStatusChanged;
use crate::state::AppState;

/// Poll cadences, picked from the set of tracked PRs each tick:
/// - fast while any open PR still has checks running (the bar advances
///   running → pass/fail on its own),
/// - medium while a PR is open but settled (catch a merge that lands
///   elsewhere — GitHub UI, `gh pr merge`, auto-merge),
/// - slow when nothing is open (just watch for a new PR after a ship).
const FAST: Duration = Duration::from_secs(8);
const MEDIUM: Duration = Duration::from_secs(20);
const SLOW: Duration = Duration::from_secs(60);

/// Cap on concurrent `gh pr view` subprocesses per tick, so a user with many
/// active workstreams doesn't spawn a process storm.
const MAX_CONCURRENT: usize = 6;

/// Start the watcher loop. Runs for the life of the app; `last` persists across
/// ticks so we only emit on change.
pub fn spawn(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: HashMap<String, Option<PrStatus>> = HashMap::new();
        loop {
            let delay = tick(&app, &mut last).await;
            tokio::time::sleep(delay).await;
        }
    });
}

/// Poll every active workstream's PR once, emit on changes, and return how long
/// to wait before the next poll.
async fn tick(app: &tauri::AppHandle, last: &mut HashMap<String, Option<PrStatus>>) -> Duration {
    let state = app.state::<AppState>();

    // No gh (missing/unauthenticated) → nothing to poll; back off hard.
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return SLOW;
    };

    // Active workstreams across all projects — the same set the sidebar shows.
    let db = state.db.clone();
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

    // Forget workstreams that were archived/removed since the last tick.
    let mut alive: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(items.len());
    for item in &items {
        alive.insert(item.id.as_str());
    }
    last.retain(|id, _| alive.contains(id.as_str()));

    // A merged PR is terminal — its status can't change again, so stop polling
    // it (the workstream is usually archived right after anyway).
    let to_poll: Vec<Workstream> = items
        .into_iter()
        .filter(|item| !matches!(last.get(&item.id), Some(Some(s)) if s.state == PrState::Merged))
        .collect();

    let results = poll_all(&gh, to_poll).await;

    for (id, res) in results {
        let status = match res {
            Ok(status) => status,
            // Transient gh failure (network, rate limit, worktree mid-teardown):
            // keep the last known value and don't emit a spurious change.
            Err(e) => {
                log::debug!("pr watch: gh status failed for {id}: {e}");
                continue;
            }
        };
        // Emit on a real change and on first sight (so a PR that appears after a
        // ship reaches the UI). First sight only fires a no-op invalidation for
        // unmounted workstreams, so the startup pass is cheap.
        if last.get(&id) != Some(&status) {
            last.insert(id.clone(), status);
            let _ = PrStatusChanged { workstream_id: id }.emit(app);
        }
    }

    cadence(last)
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
            let res = pr::pr_status(&gh, Path::new(&item.worktree_path), &item.branch).await;
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

/// Pick the next poll interval from the currently tracked statuses.
fn cadence(last: &HashMap<String, Option<PrStatus>>) -> Duration {
    let open: Vec<&PrStatus> = last
        .values()
        .flatten()
        .filter(|s| s.state == PrState::Open)
        .collect();
    if open.iter().any(|s| s.checks == ChecksState::Pending) {
        FAST
    } else if !open.is_empty() {
        MEDIUM
    } else {
        SLOW
    }
}
