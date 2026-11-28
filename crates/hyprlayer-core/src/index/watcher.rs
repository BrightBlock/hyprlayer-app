use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, ModifyKind, RemoveKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};

use super::indexer::ThoughtsIndex;
use super::roots::{canon_roots, rebase_onto_roots, RootCanon, ThoughtsRoot};
use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum IndexEvent {
    Updated { ids: Vec<String> },
    Removed { ids: Vec<String> },
    Rescanned,
}

type FsDebouncer = Debouncer<RecommendedWatcher, RecommendedCache>;

pub struct WatchHandle {
    debouncer: Arc<Mutex<FsDebouncer>>,
    watched: Arc<Mutex<Watches>>,
}

impl WatchHandle {
    /// Returns once the OS watches are closed; a drop only asks the watcher thread to close them,
    /// and Windows refuses to rename a directory while one is open.
    pub fn release(self) {
        let mut watched = self.watched.lock().unwrap_or_else(PoisonError::into_inner);
        let mut debouncer = self
            .debouncer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (path, _) in watched.drain(..) {
            let _ = debouncer.unwatch(&path);
        }
        // The watcher thread answers this after the unwatches queued ahead of it.
        let _ = debouncer.configure(notify::Config::default());
    }
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchHandle").finish_non_exhaustive()
    }
}

pub fn watch_roots(
    index: Arc<ThoughtsIndex>,
    roots: Vec<ThoughtsRoot>,
    on_event: Box<dyn Fn(IndexEvent) + Send + Sync>,
) -> Result<WatchHandle> {
    let watched = canon_roots(&roots);

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
        let plan = watch_plan(&root.root, &|dir| {
            dir.file_name().is_some_and(|n| n == ".git")
        });
        apply_plan(&mut debouncer, plan)?;
    }

    Ok(WatchHandle {
        debouncer: Arc::new(Mutex::new(debouncer)),
        watched: Arc::default(),
    })
}

pub fn watch_worktree(
    worktree: &Path,
    git_dir: &Path,
    on_change: Box<dyn Fn() + Send + Sync>,
) -> Result<WatchHandle> {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let worktree = canonical(worktree);
    let repo = git2::Repository::open(&worktree).ok();
    let mut tracked = repo.as_ref().map(tracked_paths).unwrap_or_default();
    let roots = Roots {
        git_dir: canonical(git_dir),
        common: canonical(
            &repo
                .as_ref()
                .map_or_else(|| git_dir.to_path_buf(), crate::gitops::common_dir),
        ),
        worktree,
    };

    let prune = |dir: &Path| prunes_worktree_dir(&roots.worktree, repo.as_ref(), &tracked, dir);
    let mut plan = watch_plan(&roots.worktree, &prune);
    let mut extras = Vec::new();
    if roots.worktree.join(".git").is_file() {
        extras.push((roots.git_dir.clone(), RecursiveMode::Recursive));
    }
    extras.push((roots.common.join("refs"), RecursiveMode::Recursive));
    extras.push((roots.common.clone(), RecursiveMode::NonRecursive));
    for (path, mode) in extras {
        if path.is_dir() && !covers(&plan, &path, mode) {
            plan.push((path, mode));
        }
    }

    // The handler adds watches through the debouncer that owns it; a strong ref would be a cycle.
    let slot: Arc<OnceLock<Weak<Mutex<FsDebouncer>>>> = Arc::default();
    let own = slot.clone();
    let watched = Arc::new(Mutex::new(plan.clone()));
    let handler_watched = watched.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| {
            let Ok(events) = result else {
                return;
            };
            if let (Some(repo), Some(debouncer)) =
                (repo.as_ref(), own.get().and_then(Weak::upgrade))
            {
                let mut watched = handler_watched
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                follow_tracked(
                    &events,
                    repo,
                    &roots,
                    &mut tracked,
                    &mut watched,
                    &debouncer,
                );
            }
            let relevant = events
                .iter()
                .filter(|e| signals_change(&e.kind))
                .flat_map(|e| e.paths.iter())
                .any(|p| is_relevant_change(&roots, &tracked, p));
            if relevant {
                on_change();
            }
        },
    )
    .map_err(notify_to_core)?;
    apply_plan(&mut debouncer, plan)?;
    let debouncer = Arc::new(Mutex::new(debouncer));
    let _ = slot.set(Arc::downgrade(&debouncer));
    Ok(WatchHandle { debouncer, watched })
}

fn follow_tracked(
    events: &[DebouncedEvent],
    repo: &git2::Repository,
    roots: &Roots,
    tracked: &mut HashSet<PathBuf>,
    watched: &mut Watches,
    debouncer: &Mutex<FsDebouncer>,
) {
    let paths = |kind: fn(&EventKind) -> bool| {
        events
            .iter()
            .filter(move |e| kind(&e.kind))
            .flat_map(|e| e.paths.iter())
    };
    let created = |k: &EventKind| {
        matches!(
            k,
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
        )
    };
    let mut dirs: Vec<PathBuf> = paths(created)
        .filter(|p| {
            p.strip_prefix(&roots.worktree)
                .is_ok_and(|rel| tracked.contains(rel))
        })
        .cloned()
        .collect();
    let removed_dir = |k: &EventKind| matches!(k, EventKind::Remove(r) if *r != RemoveKind::File);
    for gone in paths(removed_dir).chain(&dirs) {
        watched.retain(|(w, _)| !w.starts_with(gone));
    }
    let index = roots.git_dir.join("index");
    if paths(signals_change).any(|p| *p == index) {
        let now = tracked_paths(repo);
        dirs.extend(now.difference(tracked).map(|rel| roots.worktree.join(rel)));
        *tracked = now;
    }
    if dirs.is_empty() {
        return;
    }
    let prune = |dir: &Path| prunes_worktree_dir(&roots.worktree, Some(repo), tracked, dir);
    let mut debouncer = debouncer.lock().unwrap_or_else(PoisonError::into_inner);
    dirs.sort_by_key(|d| d.components().count());
    for dir in dirs {
        if !dir.is_dir() || covers(watched, &dir, RecursiveMode::NonRecursive) {
            continue;
        }
        for (path, mode) in watch_plan(&dir, &prune) {
            if !covers(watched, &path, mode) && debouncer.watch(&path, mode).is_ok() {
                watched.push((path, mode));
            }
        }
    }
}

fn covers(watches: &Watches, path: &Path, mode: RecursiveMode) -> bool {
    watches.iter().any(|(p, m)| match m {
        RecursiveMode::Recursive => path.starts_with(p),
        RecursiveMode::NonRecursive => p == path && mode == RecursiveMode::NonRecursive,
    })
}

struct Roots {
    worktree: PathBuf,
    git_dir: PathBuf,
    common: PathBuf,
}

type Watches = Vec<(PathBuf, RecursiveMode)>;

fn watch_plan(root: &Path, prune: &dyn Fn(&Path) -> bool) -> Watches {
    let mut plan = Vec::new();
    if plan_dir(root, prune, &mut plan) {
        plan.push((root.to_path_buf(), RecursiveMode::Recursive));
    }
    plan
}

fn plan_dir(dir: &Path, prune: &dyn Fn(&Path) -> bool, out: &mut Watches) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };

    let mut pruned_any = false;
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

fn apply_plan(debouncer: &mut FsDebouncer, plan: Watches) -> Result<()> {
    for (path, mode) in plan {
        debouncer.watch(&path, mode).map_err(notify_to_core)?;
    }
    Ok(())
}

fn prunes_worktree_dir(
    worktree: &Path,
    repo: Option<&git2::Repository>,
    tracked: &HashSet<PathBuf>,
    dir: &Path,
) -> bool {
    if dir.file_name().is_some_and(|n| n == ".git") {
        return false;
    }
    let rel = dir.strip_prefix(worktree).unwrap_or(dir);
    if tracked.contains(rel) {
        return false;
    }
    is_build_noise(rel) || repo.is_some_and(|r| r.is_path_ignored(dir).unwrap_or(false))
}

fn tracked_paths(repo: &git2::Repository) -> HashSet<PathBuf> {
    let mut paths = HashSet::new();
    let Ok(mut index) = repo.index() else {
        return paths;
    };
    let _ = index.read(false);
    for entry in index.iter() {
        let Ok(path) = std::str::from_utf8(&entry.path) else {
            continue;
        };
        let path = Path::new(path);
        if is_build_noise(path) {
            paths.insert(path.to_path_buf());
        }
        let mut cur = path.parent();
        while let Some(dir) = cur.filter(|d| !d.as_os_str().is_empty()) {
            if !paths.insert(dir.to_path_buf()) {
                break;
            }
            cur = dir.parent();
        }
    }
    paths
}

pub(crate) fn signals_change(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(
            AccessKind::Open(_) | AccessKind::Read | AccessKind::Close(AccessMode::Read)
        )
    )
}

fn is_relevant_change(roots: &Roots, tracked: &HashSet<PathBuf>, path: &Path) -> bool {
    if let Ok(rel) = path.strip_prefix(&roots.git_dir) {
        return is_git_status_signal(rel) || is_shared_ref(rel);
    }
    if let Ok(rel) = path.strip_prefix(&roots.common) {
        return is_shared_ref(rel);
    }
    let rel = path.strip_prefix(&roots.worktree).unwrap_or(path);
    if is_git_meta(rel) {
        return is_git_status_signal(rel);
    }
    !is_build_noise(rel)
        || tracked.contains(rel)
        || rel.parent().is_some_and(|dir| tracked.contains(dir))
}

fn is_shared_ref(rel: &Path) -> bool {
    rel.extension().is_none_or(|ext| ext != "lock")
        && (rel == Path::new("packed-refs")
            || rel.starts_with("refs/heads")
            || rel.starts_with("refs/remotes"))
}

fn is_git_meta(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::Normal(os) if os == ".git"))
}

fn is_git_status_signal(path: &Path) -> bool {
    if path.extension().is_some_and(|ext| ext == "lock") {
        return false;
    }
    if path.ends_with("logs/HEAD") {
        return true;
    }
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("index" | "HEAD" | "ORIG_HEAD" | "MERGE_HEAD")
    )
}

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
    watched: &[RootCanon],
    on_event: &(dyn Fn(IndexEvent) + Send + Sync),
    events: Vec<DebouncedEvent>,
) {
    let mut need_rescan = false;
    let mut sweep_missing = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for event in &events {
        need_rescan |= event.need_rescan();
        if !signals_change(&event.kind) {
            continue;
        }
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
        let Some((root, local)) = rebase_onto_roots(watched, &path) else {
            continue;
        };
        match std::fs::metadata(&local) {
            Ok(meta) if meta.is_file() => {
                if let Some(id) = index.upsert_path(root, &local) {
                    updated.push(id);
                }
            }
            Ok(_) => {}
            Err(_) => {
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

pub(crate) fn notify_to_core(err: notify::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn watches_external_git_dir_and_fires_on_index_write() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("wt");
        let git_dir = tmp.path().join("gitdir");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&git_dir).unwrap();
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
        std::thread::sleep(Duration::from_millis(400));

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

        for _ in 0..5 {
            let _ = std::fs::read(&index).unwrap();
        }
        std::thread::sleep(Duration::from_millis(1200));
        let fired = hits.load(Ordering::SeqCst);
        drop(handle);
        assert_eq!(fired, 0, "reading the git index must not signal a change");
    }

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

        assert_eq!(
            plan_of(root, &prune),
            vec![
                (".".to_string(), false),
                ("crates".to_string(), true),
                ("packages".to_string(), false),
                ("packages/web".to_string(), false),
                ("src".to_string(), true),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_covered_but_never_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("real/deep")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

        let prune = |p: &Path| p.file_name().is_some_and(|n| n == "node_modules");
        let plan = plan_of(root, &prune);
        assert!(plan.contains(&("link".to_string(), true)), "{plan:?}");
        assert!(
            !plan.iter().any(|(p, _)| p.starts_with("link/")),
            "{plan:?}"
        );
    }

    #[test]
    fn an_ignored_directory_holding_a_tracked_file_is_still_watched() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("repo");
        std::fs::create_dir_all(wt.join("vendor")).unwrap();
        std::fs::create_dir_all(wt.join("node_modules")).unwrap();
        let repo = git2::Repository::init(&wt).unwrap();
        std::fs::write(wt.join(".gitignore"), "vendor\nnode_modules\n").unwrap();
        std::fs::write(wt.join("vendor/pinned.rs"), b"// force-added\n").unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_path(Path::new("vendor/pinned.rs"))
            .expect("force-add");
        index.write().unwrap();

        let tracked = tracked_paths(&repo);
        assert!(tracked.contains(Path::new("vendor")));
        assert!(!tracked.contains(Path::new("node_modules")));

        let prune = |dir: &Path| prunes_worktree_dir(&wt, Some(&repo), &tracked, dir);
        let plan = plan_of(&wt, &prune);
        assert!(plan.contains(&("vendor".to_string(), true)), "{plan:?}");
        assert!(
            !plan.iter().any(|(p, _)| p.starts_with("node_modules")),
            "{plan:?}"
        );
    }

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

        let tracked = tracked_paths(&repo);
        let prune = |dir: &Path| prunes_worktree_dir(&wt, Some(&repo), &tracked, dir);
        let plan = plan_of(&wt, &prune);
        assert!(plan.contains(&("src".to_string(), true)), "{plan:?}");
        assert!(
            !plan
                .iter()
                .any(|(p, _)| p.starts_with("vendor") || p.starts_with(".venv")),
            "{plan:?}"
        );
    }

    fn main_checkout() -> Roots {
        Roots {
            worktree: "/wt".into(),
            git_dir: "/wt/.git".into(),
            common: "/wt/.git".into(),
        }
    }

    fn linked() -> Roots {
        Roots {
            worktree: "/wt".into(),
            git_dir: "/repo/.git/worktrees/feat".into(),
            common: "/repo/.git".into(),
        }
    }

    fn relevant(roots: &Roots, path: &str) -> bool {
        is_relevant_change(roots, &HashSet::new(), Path::new(path))
    }

    #[test]
    fn relevant_change_fires_on_working_tree_edits() {
        for roots in [main_checkout(), linked()] {
            assert!(relevant(&roots, "/wt/src/main.rs"));
            assert!(relevant(&roots, "/wt/README.md"));
            assert!(relevant(&roots, "/wt/src/index"));
        }
    }

    #[test]
    fn relevant_change_ignores_build_and_dependency_churn() {
        let roots = main_checkout();
        assert!(!relevant(&roots, "/wt/target/debug/app"));
        assert!(!relevant(&roots, "/wt/node_modules/x/index.js"));
        assert!(!relevant(&roots, "/wt/dist/bundle.js"));
    }

    #[test]
    fn relevant_change_fires_on_stage_commit_signals() {
        let main = main_checkout();
        for path in ["index", "HEAD", "logs/HEAD", "ORIG_HEAD", "MERGE_HEAD"] {
            assert!(relevant(&main, &format!("/wt/.git/{path}")), "{path}");
        }
        let linked = linked();
        assert!(relevant(&linked, "/repo/.git/worktrees/feat/index"));
        assert!(relevant(&linked, "/repo/.git/worktrees/feat/logs/HEAD"));
    }

    #[test]
    fn relevant_change_fires_on_shared_refs() {
        for (roots, git) in [(main_checkout(), "/wt/.git"), (linked(), "/repo/.git")] {
            for path in ["packed-refs", "refs/remotes/origin/main", "refs/heads/feat"] {
                assert!(relevant(&roots, &format!("{git}/{path}")), "{git}/{path}");
            }
        }
    }

    #[test]
    fn reads_never_signal_change() {
        use notify::event::{DataChange, RenameMode};

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
        for (roots, git) in [(main_checkout(), "/wt/.git"), (linked(), "/repo/.git")] {
            for path in [
                "objects/ab/cdef123",
                "COMMIT_EDITMSG",
                "FETCH_HEAD",
                "logs/refs/heads/feat",
                "refs/heads/feat.lock",
                "packed-refs.lock",
                "index.lock",
                "HEAD.lock",
            ] {
                assert!(!relevant(&roots, &format!("{git}/{path}")), "{git}/{path}");
            }
        }
        let linked = linked();
        assert!(!relevant(&linked, "/repo/.git/HEAD"));
        assert!(!relevant(&linked, "/repo/.git/index"));
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-C"])
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn committed_repo(dir: &Path, files: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-b", "main"]);
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "v1\n").unwrap();
        }
        git(dir, &["add", "."]);
        git(dir, &["commit", "-m", "init"]);
    }

    fn pushed_to_a_bare_remote(root: &Path) -> (PathBuf, PathBuf) {
        let (repo, remote) = (root.join("repo"), root.join("remote.git"));
        committed_repo(&repo, &["README.md"]);
        git(
            root,
            &["init", "--bare", "-b", "main", remote.to_str().unwrap()],
        );
        git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&repo, &["push", "-u", "origin", "main"]);
        (repo, remote)
    }

    fn teammate_pushes_main(root: &Path, remote: &Path) {
        let clone = root.join("teammate");
        git(
            root,
            &["clone", remote.to_str().unwrap(), clone.to_str().unwrap()],
        );
        std::fs::write(clone.join("theirs.txt"), "theirs\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-m", "theirs"]);
        git(&clone, &["push", "origin", "main"]);
    }

    fn counting_watch(worktree: &Path) -> (WatchHandle, Arc<std::sync::atomic::AtomicUsize>) {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = hits.clone();
        let git_dir = crate::gitops::git_dir(worktree).unwrap();
        let handle = watch_worktree(
            worktree,
            &git_dir,
            Box::new(move || {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .unwrap();
        (handle, hits)
    }

    fn fires(hits: &std::sync::atomic::AtomicUsize, act: impl FnOnce()) -> bool {
        use std::sync::atomic::Ordering;
        std::thread::sleep(Duration::from_millis(800));
        hits.store(0, Ordering::SeqCst);
        act();
        (0..100).any(|_| {
            if hits.load(Ordering::SeqCst) > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
            false
        })
    }

    #[test]
    fn watcher_fires_on_push_and_fetch_in_a_linked_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, remote) = pushed_to_a_bare_remote(tmp.path());
        let wt = tmp.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat",
                wt.to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(wt.join("feat.txt"), "feat\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-m", "feat"]);
        let (_handle, hits) = counting_watch(&wt);

        assert!(
            fires(&hits, || git(&wt, &["push", "origin", "feat"])),
            "push"
        );
        teammate_pushes_main(tmp.path(), &remote);
        assert!(fires(&hits, || git(&wt, &["fetch", "origin"])), "fetch");
    }

    #[test]
    fn watcher_fires_on_push_and_fetch_in_the_main_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, remote) = pushed_to_a_bare_remote(tmp.path());
        git(&repo, &["checkout", "-b", "feat"]);
        let (_handle, hits) = counting_watch(&repo);

        assert!(
            fires(&hits, || git(&repo, &["push", "origin", "feat"])),
            "push"
        );
        teammate_pushes_main(tmp.path(), &remote);
        assert!(fires(&hits, || git(&repo, &["fetch", "origin"])), "fetch");
    }

    #[test]
    fn editing_a_tracked_file_under_build_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        committed_repo(&repo, &["build/release.sh"]);
        let (_handle, hits) = counting_watch(&repo);

        assert!(fires(&hits, || std::fs::write(
            repo.join("build/release.sh"),
            "v2\n"
        )
        .unwrap()));
    }

    #[test]
    fn a_build_file_tracked_after_the_watch_opened_fires_on_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        committed_repo(&repo, &["README.md"]);
        let (_handle, hits) = counting_watch(&repo);
        std::fs::create_dir(repo.join("build")).unwrap();
        std::fs::write(repo.join("build/release.sh"), "v1\n").unwrap();
        git(&repo, &["add", "build/release.sh"]);
        git(&repo, &["commit", "-m", "track build"]);

        assert!(fires(&hits, || std::fs::write(
            repo.join("build/release.sh"),
            "v2\n"
        )
        .unwrap()));
    }

    #[test]
    fn a_tracked_directory_deleted_and_recreated_fires_on_edits() {
        let settle = || std::thread::sleep(Duration::from_millis(800));
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        committed_repo(&repo, &["gen/x.txt"]);
        std::fs::write(repo.join(".gitignore"), "out/\n").unwrap();
        let (_handle, hits) = counting_watch(&repo);

        std::fs::remove_dir_all(repo.join("gen")).unwrap();
        settle();
        std::fs::create_dir(repo.join("gen")).unwrap();
        std::fs::write(repo.join("gen/x.txt"), "v2\n").unwrap();
        assert!(
            fires(&hits, || std::fs::write(repo.join("gen/x.txt"), "v3\n")
                .unwrap()),
            "tracked when the watch opened"
        );

        std::fs::create_dir(repo.join("out")).unwrap();
        std::fs::write(repo.join("out/a.txt"), "v1\n").unwrap();
        git(&repo, &["add", "-f", "out/a.txt"]);
        settle();
        git(&repo, &["rm", "-r", "-q", "--cached", "out"]);
        std::fs::remove_dir_all(repo.join("out")).unwrap();
        settle();
        std::fs::create_dir(repo.join("out")).unwrap();
        std::fs::write(repo.join("out/a.txt"), "v2\n").unwrap();
        git(&repo, &["add", "-f", "out/a.txt"]);
        assert!(
            fires(&hits, || std::fs::write(repo.join("out/a.txt"), "v3\n")
                .unwrap()),
            "tracked after the watch opened"
        );
    }

    #[test]
    fn a_tracked_root_level_file_named_build_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        committed_repo(&repo, &["build", "src/main.rs"]);
        let (_handle, hits) = counting_watch(&repo);

        assert!(fires(&hits, || std::fs::write(repo.join("build"), "v2\n")
            .unwrap()));
    }

    #[test]
    fn a_worktree_under_a_dir_named_dist_fires_on_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("dist").join("repo");
        committed_repo(&repo, &["src/main.rs"]);
        let (_handle, hits) = counting_watch(&repo);

        assert!(fires(&hits, || std::fs::write(
            repo.join("src/main.rs"),
            "v2\n"
        )
        .unwrap()));
    }
}
