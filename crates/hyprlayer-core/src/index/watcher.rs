//! Filesystem watching that keeps a [`ThoughtsIndex`] fresh.
//!
//! One debounced watcher (300 ms) covers every existing root recursively.
//! Debounced batches are translated into index mutations on the watcher
//! thread (the index is synchronous), and the resulting ids are reported as
//! one [`IndexEvent`] per kind per flush.

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
}
