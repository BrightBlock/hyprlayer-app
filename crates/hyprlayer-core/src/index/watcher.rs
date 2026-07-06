//! Filesystem watching that keeps a [`ThoughtsIndex`] fresh. One debounced
//! watcher (300 ms) recursively covers every existing root; batches mutate the
//! synchronous index on the watcher thread, reporting one [`IndexEvent`] per
//! kind per flush.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};

use super::indexer::ThoughtsIndex;
use super::roots::ThoughtsRoot;
use crate::error::{CoreError, Result};

/// What changed in the index after a debounce flush.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexEvent {
    Updated {
        ids: Vec<String>,
    },
    Removed {
        ids: Vec<String>,
    },
    /// The OS asked for a rescan; a full rescan of all watched roots ran.
    Rescanned,
}

/// Keeps the debounced watcher alive; dropping it stops watching.
pub struct WatchHandle {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchHandle").finish_non_exhaustive()
    }
}

/// A root plus its canonicalized path: macOS FSEvents (and symlinked roots
/// generally) report real paths, while index ids use the configured path.
/// Events under the canonical path are rebased onto the configured one so
/// ids stay consistent with `full_scan`.
struct WatchedRoot {
    root: ThoughtsRoot,
    canon: Option<PathBuf>,
}

/// Watch every existing root and apply debounced changes to `index`,
/// reporting them through `on_event` (invoked on the watcher thread).
/// Missing roots are skipped with a warning.
pub fn watch_roots(
    index: Arc<ThoughtsIndex>,
    roots: Vec<ThoughtsRoot>,
    on_event: Box<dyn Fn(IndexEvent) + Send + Sync>,
) -> Result<WatchHandle> {
    let watched: Vec<WatchedRoot> = roots
        .iter()
        .map(|root| WatchedRoot {
            canon: std::fs::canonicalize(&root.root)
                .ok()
                .filter(|c| c != &root.root),
            root: root.clone(),
        })
        .collect();

    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| match result {
            Ok(events) => handle_batch(&index, &watched, on_event.as_ref(), events),
            Err(errors) => {
                for err in errors {
                    tracing::warn!(error = %err, "thoughts watcher error");
                }
            }
        },
    )
    .map_err(notify_to_core)?;

    for root in &roots {
        if !root.root.is_dir() {
            tracing::warn!(root = %root.root.display(), "thoughts root missing, not watching");
            continue;
        }
        debouncer
            .watch(&root.root, RecursiveMode::Recursive)
            .map_err(notify_to_core)?;
    }

    Ok(WatchHandle {
        _debouncer: debouncer,
    })
}

/// Watch a worktree for git-status-affecting changes, firing `on_change` once
/// per debounced batch touching a relevant path. Also watches `git_dir`
/// (`git2::Repository::path`): a linked worktree's index/HEAD/reflog live
/// outside the tree, so a bare commit/stage leaves nothing under `worktree`.
/// Dropping the handle stops watching.
pub fn watch_worktree(
    worktree: &Path,
    git_dir: &Path,
    on_change: Box<dyn Fn() + Send + Sync>,
) -> Result<WatchHandle> {
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| {
            if let Ok(events) = result {
                let relevant = events
                    .iter()
                    .flat_map(|e| e.paths.iter())
                    .any(|p| is_relevant_change(p));
                if relevant {
                    on_change();
                }
            }
        },
    )
    .map_err(notify_to_core)?;
    debouncer
        .watch(worktree, RecursiveMode::Recursive)
        .map_err(notify_to_core)?;
    // Linked worktree: `.git` is a gitfile and metadata sits outside the tree.
    // A primary checkout's `.git` dir is already covered by the watch above.
    if worktree.join(".git").is_file() && git_dir.is_dir() {
        debouncer
            .watch(git_dir, RecursiveMode::Recursive)
            .map_err(notify_to_core)?;
    }
    Ok(WatchHandle {
        _debouncer: debouncer,
    })
}

/// Whether a changed path should trigger a git-status refresh.
fn is_relevant_change(path: &Path) -> bool {
    if is_git_meta(path) {
        is_git_status_signal(path)
    } else {
        !is_build_noise(path)
    }
}

/// A path inside git's metadata (carries a `.git` component) — a primary
/// checkout's `.git` or a linked worktree's `.git/worktrees/<name>/`.
fn is_git_meta(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::Normal(os) if os == ".git"))
}

/// Git-metadata files whose change moves the working-tree status or HEAD
/// (index/HEAD/merge pseudo-refs/HEAD reflog). Excludes objects, packed-refs,
/// fetch churn (`FETCH_HEAD`, `refs/remotes`) and `*.lock`.
fn is_git_status_signal(path: &Path) -> bool {
    if path.extension().is_some_and(|ext| ext == "lock") {
        return false;
    }
    // The HEAD reflog, not `logs/refs/**` (remote-tracking churn).
    if path.ends_with("logs/HEAD") {
        return true;
    }
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("index" | "HEAD" | "ORIG_HEAD" | "MERGE_HEAD")
    )
}

/// Build/dependency output dirs whose churn shouldn't trigger a refresh.
fn is_build_noise(path: &Path) -> bool {
    const NOISE: &[&str] = &[
        "node_modules",
        "target",
        "dist",
        "build",
        ".next",
        ".turbo",
        ".cache",
    ];
    path.components().any(|c| {
        matches!(c, std::path::Component::Normal(os) if NOISE.iter().any(|n| os == *n))
    })
}

fn handle_batch(
    index: &ThoughtsIndex,
    watched: &[WatchedRoot],
    on_event: &(dyn Fn(IndexEvent) + Send + Sync),
    events: Vec<DebouncedEvent>,
) {
    // Dedup affected paths across the batch, preserving order. Rename pairs
    // arrive as one event with both paths; the stat below sorts them out
    // (old path gone -> remove, new path present -> upsert).
    let mut need_rescan = false;
    let mut sweep_missing = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for event in &events {
        need_rescan |= event.need_rescan();
        // macOS FSEvents can replay a stale Create flag on a rename source,
        // making the debouncer drop the rename-out event without a trace.
        // Create/rename batches therefore trigger a missing-file sweep.
        sweep_missing |= matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
        );
        for path in &event.paths {
            if seen.insert(path.clone()) {
                paths.push(path.clone());
            }
        }
    }

    if need_rescan {
        let roots: Vec<ThoughtsRoot> = watched.iter().map(|w| w.root.clone()).collect();
        index.full_scan(&roots);
        on_event(IndexEvent::Rescanned);
        return;
    }

    let mut updated: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for path in paths {
        let Some((root, local)) = resolve_owner(watched, &path) else {
            continue;
        };
        match std::fs::metadata(&local) {
            Ok(meta) if meta.is_file() => {
                // upsert applies the skip rules (classify) and the content
                // hash short-circuit, so untracked/unchanged files yield no id.
                if let Some(id) = index.upsert_path(root, &local) {
                    updated.push(id);
                }
            }
            Ok(_) => {} // directory event; contained files get their own events
            Err(_) => {
                // Gone: a file (exact id) or a directory (whole subtree).
                if let Some(id) = index.remove_path(&local) {
                    removed.push(id);
                } else {
                    removed.extend(index.remove_subtree(&local));
                }
            }
        }
    }

    if sweep_missing {
        for id in index.remove_missing() {
            if !removed.contains(&id) {
                removed.push(id);
            }
        }
    }

    if !updated.is_empty() {
        on_event(IndexEvent::Updated { ids: updated });
    }
    if !removed.is_empty() {
        on_event(IndexEvent::Removed { ids: removed });
    }
}

/// Find the root owning `path` (deepest prefix wins) and the path expressed
/// under the configured root (rebasing canonical paths when needed).
fn resolve_owner<'a>(
    watched: &'a [WatchedRoot],
    path: &Path,
) -> Option<(&'a ThoughtsRoot, PathBuf)> {
    let mut best: Option<(&ThoughtsRoot, PathBuf, usize)> = None;
    let mut consider = |root: &'a ThoughtsRoot, local: PathBuf, depth: usize| {
        if best.as_ref().is_none_or(|(_, _, d)| depth > *d) {
            best = Some((root, local, depth));
        }
    };
    for w in watched {
        if path.starts_with(&w.root.root) {
            consider(
                &w.root,
                path.to_path_buf(),
                w.root.root.components().count(),
            );
        } else if let Some(canon) = &w.canon {
            if let Ok(rel) = path.strip_prefix(canon) {
                consider(&w.root, w.root.root.join(rel), canon.components().count());
            }
        }
    }
    best.map(|(root, local, _)| (root, local))
}

fn notify_to_core(err: notify::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendKind;

    fn root_at(path: &str) -> ThoughtsRoot {
        ThoughtsRoot {
            root: PathBuf::from(path),
            repos_dir: "repos".into(),
            global_dir: "global".into(),
            backend: BackendKind::Git,
            profile: None,
        }
    }

    #[test]
    fn resolve_owner_prefers_deepest_root_and_rebases_canonical_paths() {
        let watched = vec![
            WatchedRoot {
                root: root_at("/a"),
                canon: None,
            },
            WatchedRoot {
                root: root_at("/a/nested"),
                canon: None,
            },
            WatchedRoot {
                root: root_at("/sym/root"),
                canon: Some(PathBuf::from("/real/root")),
            },
        ];

        let (root, local) = resolve_owner(&watched, Path::new("/a/nested/x.md")).unwrap();
        assert_eq!(root.root, PathBuf::from("/a/nested"));
        assert_eq!(local, PathBuf::from("/a/nested/x.md"));

        let (root, local) = resolve_owner(&watched, Path::new("/real/root/global/x.md")).unwrap();
        assert_eq!(root.root, PathBuf::from("/sym/root"));
        assert_eq!(local, PathBuf::from("/sym/root/global/x.md"));

        assert!(resolve_owner(&watched, Path::new("/elsewhere/x.md")).is_none());
    }

    #[test]
    fn watch_skips_missing_roots() {
        let index = Arc::new(ThoughtsIndex::new());
        let handle = watch_roots(
            index,
            vec![root_at("/definitely/does/not/exist/hyprlayer-test")],
            Box::new(|_| {}),
        );
        assert!(handle.is_ok());
    }

    // Real notify watcher: a linked-worktree commit writes only the external
    // git dir — assert we still fire. Ignored: fs timing shouldn't gate releases.
    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn watches_external_git_dir_and_fires_on_index_write() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("wt");
        let git_dir = tmp.path().join("gitdir");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&git_dir).unwrap();
        // Linked worktree: `.git` is a gitfile — makes watch_worktree watch git_dir.
        std::fs::write(wt.join(".git"), "gitdir: ../gitdir\n").unwrap();
        std::fs::write(git_dir.join("index"), b"v0").unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let handle = watch_worktree(
            &wt,
            &git_dir,
            Box::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        // Let the OS arm the watch before mutating.
        std::thread::sleep(Duration::from_millis(400));

        // The bug's exact shape: only the (external) index moved, no file under wt.
        std::fs::write(git_dir.join("index"), b"v1-after-commit").unwrap();

        let fired = (0..50).any(|_| {
            if hits.load(Ordering::SeqCst) > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
            false
        });
        drop(handle);
        assert!(fired, "an index write in the external git dir must fire on_change");
    }

    #[test]
    fn relevant_change_fires_on_working_tree_edits() {
        assert!(is_relevant_change(Path::new("/wt/src/main.rs")));
        assert!(is_relevant_change(Path::new("/wt/README.md")));
        // A source file that merely happens to be named like a git file.
        assert!(is_relevant_change(Path::new("/wt/src/index")));
    }

    #[test]
    fn relevant_change_ignores_build_and_dependency_churn() {
        assert!(!is_relevant_change(Path::new("/wt/target/debug/app")));
        assert!(!is_relevant_change(Path::new("/wt/node_modules/x/index.js")));
        assert!(!is_relevant_change(Path::new("/wt/dist/bundle.js")));
    }

    #[test]
    fn relevant_change_fires_on_stage_commit_signals() {
        // Primary checkout (`.git` is a real dir) — stage/commit/checkout.
        assert!(is_relevant_change(Path::new("/wt/.git/index")));
        assert!(is_relevant_change(Path::new("/wt/.git/HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/logs/HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/ORIG_HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/MERGE_HEAD")));
        // Linked worktree — metadata under `.git/worktrees/<name>/`.
        assert!(is_relevant_change(Path::new(
            "/repo/.git/worktrees/feat/index"
        )));
        assert!(is_relevant_change(Path::new(
            "/repo/.git/worktrees/feat/logs/HEAD"
        )));
    }

    #[test]
    fn relevant_change_ignores_git_noise() {
        // Object writes, packed-refs, and the commit message buffer.
        assert!(!is_relevant_change(Path::new("/wt/.git/objects/ab/cdef123")));
        assert!(!is_relevant_change(Path::new("/wt/.git/packed-refs")));
        assert!(!is_relevant_change(Path::new("/wt/.git/COMMIT_EDITMSG")));
        // A background `git fetch`: FETCH_HEAD and remote-tracking refs/reflogs.
        assert!(!is_relevant_change(Path::new("/wt/.git/FETCH_HEAD")));
        assert!(!is_relevant_change(Path::new(
            "/wt/.git/refs/remotes/origin/main"
        )));
        assert!(!is_relevant_change(Path::new(
            "/wt/.git/logs/refs/heads/feat"
        )));
        // Transient lock files bracketing an index/ref write.
        assert!(!is_relevant_change(Path::new("/wt/.git/index.lock")));
        assert!(!is_relevant_change(Path::new("/wt/.git/HEAD.lock")));
    }
}
