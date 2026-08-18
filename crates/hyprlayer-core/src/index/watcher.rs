//! Filesystem watching that keeps a [`ThoughtsIndex`] fresh. One debounced
//! watcher (300 ms) recursively covers every existing root; batches mutate the
//! synchronous index on the watcher thread, reporting one [`IndexEvent`] per
//! kind per flush.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, ModifyKind};
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
        // `classify` rejects every path under `.git`, so an object write or a
        // fetch can only ever cost us a wakeup and a discarded event.
        let plan = watch_plan(&root.root, &|dir| {
            dir.file_name().is_some_and(|n| n == ".git")
        });
        apply_plan(&mut debouncer, plan)?;
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
                    .filter(|e| signals_change(&e.kind))
                    .flat_map(|e| e.paths.iter())
                    .any(|p| is_relevant_change(p));
                if relevant {
                    on_change();
                }
            }
        },
    )
    .map_err(notify_to_core)?;
    // Keep `.git`, its index/HEAD writes are the signals this watcher exists
    // for, but never arm build output or anything the repo itself ignores.
    let repo = git2::Repository::open(worktree).ok();
    let tracked = repo.as_ref().map(tracked_dirs).unwrap_or_default();
    let prune = |dir: &Path| {
        if dir.file_name().is_some_and(|n| n == ".git") {
            return false;
        }
        // Match on the path *below* the worktree: a checkout that happens to
        // live under a directory called `build` is not build output.
        let rel = dir.strip_prefix(worktree).unwrap_or(dir);
        if tracked.contains(rel) {
            return false;
        }
        is_build_noise(rel)
            || repo
                .as_ref()
                .is_some_and(|r| r.is_path_ignored(dir).unwrap_or(false))
    };
    apply_plan(&mut debouncer, watch_plan(worktree, &prune))?;
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

/// Directories to watch, paired with the mode each is watched in.
type Watches = Vec<(PathBuf, RecursiveMode)>;

/// The watches covering `root` without arming anything `prune` claims: the
/// largest recursive watches that fit, plus a non-recursive "spine" joining
/// them.
///
/// A blanket `RecursiveMode::Recursive` arms one watch per directory, and a
/// working checkout is mostly build output (4046 directories here, 62 ours).
/// Dropping those events on arrival still pays for the watch and the wakeup,
/// and watches are a finite per-user resource.
///
/// Arming everything then `unwatch`ing the unwanted subtrees would be shorter,
/// but only inotify supports it. FSEvents keys its table by the paths it was
/// handed, so unwatching a child misses and restarts the whole stream on its
/// way to failing; Windows ignores it outright. Building the set up front is
/// portable and never transiently holds the watches we are avoiding, which
/// matters when the reason to prune is a limit we would otherwise hit first.
///
/// Directories already covered by a recursive watch are left out: notify arms
/// new subdirectories itself, so the plan survives the tree growing.
fn watch_plan(root: &Path, prune: &dyn Fn(&Path) -> bool) -> Watches {
    let mut plan = Vec::new();
    if plan_dir(root, prune, &mut plan) {
        plan.push((root.to_path_buf(), RecursiveMode::Recursive));
    }
    plan
}

/// Append the watches covering `dir`, returning whether its whole subtree came
/// out clean, in which case the caller covers it with one recursive watch and
/// discards what we appended.
fn plan_dir(dir: &Path, prune: &dyn Fn(&Path) -> bool, out: &mut Watches) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // Unreadable (permissions, or it vanished mid-walk): claim clean so an
        // ancestor still covers it rather than leaving a hole in the plan.
        return true;
    };

    let mut pruned_any = false;
    // `(path, descend)`, a symlinked directory is opaque: we never walk it
    // (that is how a watcher finds a cycle), but a recursive watch still covers
    // it exactly as a blanket watch would have.
    let mut children: Vec<(PathBuf, bool)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let symlink = kind.is_symlink();
        if !kind.is_dir() && !(symlink && path.is_dir()) {
            continue;
        }
        if prune(&path) {
            pruned_any = true;
        } else {
            children.push((path, !symlink));
        }
    }

    let mut sub: Vec<(PathBuf, bool, Watches)> = Vec::new();
    for (path, descend) in children {
        let mut local = Vec::new();
        let clean = !descend || plan_dir(&path, prune, &mut local);
        sub.push((path, clean, local));
    }

    if !pruned_any && sub.iter().all(|(_, clean, _)| *clean) {
        return true;
    }
    out.push((dir.to_path_buf(), RecursiveMode::NonRecursive));
    for (path, clean, local) in sub {
        if clean {
            out.push((path, RecursiveMode::Recursive));
        } else {
            out.extend(local);
        }
    }
    false
}

/// Apply a plan, failing on the first watch the backend rejects.
fn apply_plan(
    debouncer: &mut Debouncer<RecommendedWatcher, RecommendedCache>,
    plan: Watches,
) -> Result<()> {
    for (path, mode) in plan {
        debouncer.watch(&path, mode).map_err(notify_to_core)?;
    }
    Ok(())
}

/// Directories holding a tracked file, at every level, relative to the worktree.
///
/// Ignore rules are evaluated independently of tracking, so `git status` still
/// reports a tracked file that a later `.gitignore` rule covers (or one added
/// with `git add -f`). Pruning by ignore rules alone would go blind to exactly
/// those edits, so a directory on this list is never pruned however well it
/// matches.
fn tracked_dirs(repo: &git2::Repository) -> HashSet<PathBuf> {
    let mut dirs = HashSet::new();
    let Ok(index) = repo.index() else {
        return dirs;
    };
    for entry in index.iter() {
        let Ok(path) = std::str::from_utf8(&entry.path) else {
            continue;
        };
        let mut cur = Path::new(path).parent();
        // Ancestors are shared, so the first one already recorded means every
        // ancestor above it is too.
        while let Some(dir) = cur.filter(|d| !d.as_os_str().is_empty()) {
            if !dirs.insert(dir.to_path_buf()) {
                break;
            }
            cur = dir.parent();
        }
    }
    dirs
}

/// Whether an event kind can signal a mutation at all.
///
/// notify's inotify mask includes `IN_OPEN`, so on Linux *reading* a watched
/// file reports `Access(Open)`. That turns a read into a change signal, and for
/// the worktree watcher it closes a loop: a git-status refresh opens
/// `.git/index`, the open fires `WorktreeChanged`, the UI refetches status, and
/// the next status opens the index again, a self-sustaining spin that costs
/// real CPU with the app idle.
///
/// Only *unambiguously* read-only kinds are rejected. `Access` also carries
/// `Close(Write)` (a completed write) and the `Any` catch-alls, which notify
/// documents as "kind unknown", a backend that reports one of those may well
/// be reporting a mutation, and dropping a real change is far worse than the
/// occasional redundant refresh.
fn signals_change(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(
            AccessKind::Open(_) | AccessKind::Read | AccessKind::Close(AccessMode::Read)
        )
    )
}

/// Whether a changed path should trigger a git-status refresh.
fn is_relevant_change(path: &Path) -> bool {
    if is_git_meta(path) {
        is_git_status_signal(path)
    } else {
        !is_build_noise(path)
    }
}

/// A path inside git's metadata (carries a `.git` component), a primary
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
    path.components()
        .any(|c| matches!(c, std::path::Component::Normal(os) if NOISE.iter().any(|n| os == *n)))
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
        // A read (Linux `IN_OPEN`) carries no new content: re-reading and
        // re-hashing the file would burn CPU to reach the same index state.
        if !signals_change(&event.kind) {
            continue;
        }
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
    // git dir, assert we still fire. Ignored: fs timing shouldn't gate releases.
    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn watches_external_git_dir_and_fires_on_index_write() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("wt");
        let git_dir = tmp.path().join("gitdir");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&git_dir).unwrap();
        // Linked worktree: `.git` is a gitfile, makes watch_worktree watch git_dir.
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
        assert!(
            fired,
            "an index write in the external git dir must fire on_change"
        );
    }

    // The CPU-spin regression, at the real seam: a *read* of the watched git
    // index must not fire. Linux inotify reports `IN_OPEN`, so before the kind
    // filter a git-status refresh re-armed itself, status opens `.git/index`,
    // the open emits `WorktreeChanged`, the UI refetches status, forever.
    // Ignored: fs timing shouldn't gate releases (`signals_change` covers the
    // logic deterministically).
    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn reading_the_git_index_does_not_fire() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("wt");
        let git_dir = tmp.path().join("gitdir");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../gitdir\n").unwrap();
        let index = git_dir.join("index");
        std::fs::write(&index, b"v0").unwrap();

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
        std::thread::sleep(Duration::from_millis(400));

        // Exactly what a git-status refresh does to the index: open and read it.
        for _ in 0..5 {
            let _ = std::fs::read(&index).unwrap();
        }
        // Well past the 300 ms debounce, so a fire would have landed.
        std::thread::sleep(Duration::from_millis(1200));
        let fired = hits.load(Ordering::SeqCst);
        drop(handle);
        assert_eq!(fired, 0, "reading the git index must not signal a change");
    }

    /// A plan as `(path-relative-to-root, recursive?)`, sorted for comparison.
    fn plan_of(root: &Path, prune: &dyn Fn(&Path) -> bool) -> Vec<(String, bool)> {
        let mut plan: Vec<(String, bool)> = watch_plan(root, prune)
            .into_iter()
            .map(|(p, mode)| {
                let rel = p.strip_prefix(root).unwrap_or(&p);
                let rel = rel.to_string_lossy().replace('\\', "/");
                (
                    if rel.is_empty() { ".".into() } else { rel },
                    mode == RecursiveMode::Recursive,
                )
            })
            .collect();
        plan.sort();
        plan
    }

    #[test]
    fn a_clean_tree_is_one_recursive_watch() {
        let tmp = tempfile::tempdir().unwrap();
        for d in ["src/lib", "crates/core"] {
            std::fs::create_dir_all(tmp.path().join(d)).unwrap();
        }
        // Nothing to prune, so the whole tree costs exactly one watch.
        assert_eq!(
            plan_of(tmp.path(), &|_| false),
            vec![(".".to_string(), true)]
        );
    }

    #[test]
    fn pruned_subtrees_are_never_in_the_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for d in [
            "src/lib",
            "crates/core",
            "node_modules/a/dist",
            "packages/web/node_modules/b",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let prune = |p: &Path| p.file_name().is_some_and(|n| n == "node_modules");

        // The spine is non-recursive; each clean subtree is one recursive watch.
        // Crucially `node_modules` appears nowhere, not even to be removed.
        assert_eq!(
            plan_of(root, &prune),
            vec![
                (".".to_string(), false),
                ("crates".to_string(), true),
                ("packages".to_string(), false),
                // Non-recursive precisely because it holds a nested
                // `node_modules`: recursive here would re-arm what we pruned.
                ("packages/web".to_string(), false),
                ("src".to_string(), true),
            ]
        );
    }

    #[test]
    fn a_symlinked_directory_is_covered_but_never_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("real/deep")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        #[cfg(not(unix))]
        return;

        let prune = |p: &Path| p.file_name().is_some_and(|n| n == "node_modules");
        let plan = plan_of(root, &prune);
        // Covered by a recursive watch (as a blanket watch would have), but the
        // walk never descended through it, no `link/deep` entry.
        assert!(plan.contains(&("link".to_string(), true)), "{plan:?}");
        assert!(
            !plan.iter().any(|(p, _)| p.starts_with("link/")),
            "{plan:?}"
        );
    }

    // Ignore rules are evaluated independently of tracking, so `git status`
    // still reports a force-added file under an ignored directory. Pruning by
    // ignore rules alone would go blind to exactly those edits.
    #[test]
    fn an_ignored_directory_holding_a_tracked_file_is_still_watched() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("repo");
        std::fs::create_dir_all(wt.join("vendor")).unwrap();
        std::fs::create_dir_all(wt.join("node_modules")).unwrap();
        let repo = git2::Repository::init(&wt).unwrap();
        std::fs::write(wt.join(".gitignore"), "vendor\nnode_modules\n").unwrap();
        std::fs::write(wt.join("vendor/pinned.rs"), b"// force-added\n").unwrap();

        // `git add -f vendor/pinned.rs`
        let mut index = repo.index().unwrap();
        index
            .add_path(Path::new("vendor/pinned.rs"))
            .expect("force-add");
        index.write().unwrap();

        let tracked = tracked_dirs(&repo);
        assert!(tracked.contains(Path::new("vendor")));
        assert!(!tracked.contains(Path::new("node_modules")));

        let prune = |dir: &Path| {
            let rel = dir.strip_prefix(&wt).unwrap_or(dir);
            !tracked.contains(rel) && repo.is_path_ignored(dir).unwrap_or(false)
        };
        let plan = plan_of(&wt, &prune);
        assert!(plan.contains(&("vendor".to_string(), true)), "{plan:?}");
        assert!(
            !plan.iter().any(|(p, _)| p.starts_with("node_modules")),
            "{plan:?}"
        );
    }

    // The gitignore arm, exercised with names `is_build_noise` knows nothing
    // about, otherwise the hard-coded list would carry the test on its own.
    #[test]
    fn gitignore_prunes_names_the_hardcoded_list_does_not_know() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("repo");
        std::fs::create_dir_all(wt.join("vendor/pkg")).unwrap();
        std::fs::create_dir_all(wt.join(".venv/lib")).unwrap();
        std::fs::create_dir_all(wt.join("src")).unwrap();
        let repo = git2::Repository::init(&wt).unwrap();
        std::fs::write(wt.join(".gitignore"), "vendor\n.venv\n").unwrap();

        assert!(!is_build_noise(Path::new("vendor")));
        assert!(!is_build_noise(Path::new(".venv")));

        let prune = |dir: &Path| repo.is_path_ignored(dir).unwrap_or(false);
        let plan = plan_of(&wt, &prune);
        assert!(plan.contains(&("src".to_string(), true)), "{plan:?}");
        assert!(
            !plan
                .iter()
                .any(|(p, _)| p.starts_with("vendor") || p.starts_with(".venv")),
            "{plan:?}"
        );
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
        assert!(!is_relevant_change(Path::new(
            "/wt/node_modules/x/index.js"
        )));
        assert!(!is_relevant_change(Path::new("/wt/dist/bundle.js")));
    }

    #[test]
    fn relevant_change_fires_on_stage_commit_signals() {
        // Primary checkout (`.git` is a real dir), stage/commit/checkout.
        assert!(is_relevant_change(Path::new("/wt/.git/index")));
        assert!(is_relevant_change(Path::new("/wt/.git/HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/logs/HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/ORIG_HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/MERGE_HEAD")));
        // Linked worktree, metadata under `.git/worktrees/<name>/`.
        assert!(is_relevant_change(Path::new(
            "/repo/.git/worktrees/feat/index"
        )));
        assert!(is_relevant_change(Path::new(
            "/repo/.git/worktrees/feat/logs/HEAD"
        )));
    }

    #[test]
    fn reads_never_signal_change() {
        use notify::event::{DataChange, RenameMode};

        // The loop this guards: a git-status refresh opens `.git/index`, and on
        // Linux that open would otherwise fire another refresh, forever.
        assert!(!signals_change(&EventKind::Access(AccessKind::Open(
            AccessMode::Read
        ))));
        assert!(!signals_change(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!signals_change(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!signals_change(&EventKind::Access(AccessKind::Read)));

        // Real mutations still get through, including the `Access` variants
        // that report a completed write. `Any` means "kind unknown", so it is
        // deliberately allowed: a redundant refresh beats a dropped change.
        assert!(signals_change(&EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(signals_change(&EventKind::Access(AccessKind::Close(
            AccessMode::Any
        ))));
        assert!(signals_change(&EventKind::Access(AccessKind::Any)));
        assert!(signals_change(&EventKind::Modify(ModifyKind::Data(
            DataChange::Content
        ))));
        assert!(signals_change(&EventKind::Modify(ModifyKind::Name(
            RenameMode::Both
        ))));
        assert!(signals_change(&EventKind::Create(
            notify::event::CreateKind::File
        )));
        assert!(signals_change(&EventKind::Remove(
            notify::event::RemoveKind::File
        )));
    }

    #[test]
    fn relevant_change_ignores_git_noise() {
        // Object writes, packed-refs, and the commit message buffer.
        assert!(!is_relevant_change(Path::new(
            "/wt/.git/objects/ab/cdef123"
        )));
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
