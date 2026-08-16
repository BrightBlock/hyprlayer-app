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
use std::sync::atomic::Ordering;
use std::time::Duration;

use hyprlayer_core::docs::pr_record;
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
            // Read focus + the wake handle without holding the State guard across
            // an await.
            let (focused, wake) = {
                let state = app.state::<AppState>();
                (
                    state.focused.load(Ordering::Relaxed),
                    state.focus_changed.clone(),
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

            let delay = tick(&app, &mut last).await;
            // Wake early if focus flips (e.g. a blur mid-FAST-sleep) so we
            // re-evaluate instead of finishing the sleep in the background.
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wake.notified() => {}
            }
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
    // it (the workstream is usually archived right after anyway). Folder rows
    // have no repo to ask, so gh would fail (or answer for an enclosing repo).
    let to_poll: Vec<Workstream> = items
        .into_iter()
        .filter(|item| !item.is_folder())
        .filter(|item| !matches!(last.get(&item.id), Some(Some(s)) if s.state == PrState::Merged))
        .collect();

    // Kept alongside the poll because the result rows carry only the workstream
    // id, and promoting a merged PR's record needs the project it belongs to.
    let projects: HashMap<String, String> = to_poll
        .iter()
        .map(|item| (item.id.clone(), item.project_path.clone()))
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
            // Merged is terminal and we stop polling it above, so this is the
            // only tick that ever sees the transition — promote the PR's
            // thoughts record here or never. First sight counts: it's how a
            // merge that landed while the app was closed gets caught up.
            if let Some(s) = status.as_ref().filter(|s| s.state == PrState::Merged) {
                if let Some(project) = projects.get(&id) {
                    promote_record(app, project, s.number).await;
                }
            }
            last.insert(id.clone(), status);
            let _ = PrStatusChanged { workstream_id: id }.emit(app);
        }
    }

    cadence(last)
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
