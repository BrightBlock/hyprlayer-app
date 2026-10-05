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
use super::roots::{canon_roots, rebase_onto_roots, RootCanon, ThoughtsRoot};
use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum IndexEvent {
    Updated { ids: Vec<String> },
    Removed { ids: Vec<String> },
    Rescanned,
}

pub struct WatchHandle {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
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
        _debouncer: debouncer,
    })
}

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
    let repo = git2::Repository::open(worktree).ok();
    let tracked = repo.as_ref().map(tracked_dirs).unwrap_or_default();
    let prune = |dir: &Path| {
        if dir.file_name().is_some_and(|n| n == ".git") {
            return false;
        }
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
    if worktree.join(".git").is_file() && git_dir.is_dir() {
        debouncer
            .watch(git_dir, RecursiveMode::Recursive)
            .map_err(notify_to_core)?;
    }
    Ok(WatchHandle {
        _debouncer: debouncer,
    })
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

fn apply_plan(
    debouncer: &mut Debouncer<RecommendedWatcher, RecommendedCache>,
    plan: Watches,
) -> Result<()> {
    for (path, mode) in plan {
        debouncer.watch(&path, mode).map_err(notify_to_core)?;
    }
    Ok(())
}

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
        while let Some(dir) = cur.filter(|d| !d.as_os_str().is_empty()) {
            if !dirs.insert(dir.to_path_buf()) {
                break;
            }
            cur = dir.parent();
        }
    }
    dirs
}

pub(crate) fn signals_change(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(
            AccessKind::Open(_) | AccessKind::Read | AccessKind::Close(AccessMode::Read)
        )
    )
}

fn is_relevant_change(path: &Path) -> bool {
    if is_git_meta(path) {
        is_git_status_signal(path)
    } else {
        !is_build_noise(path)
    }
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
    fn watch_skips_missing_roots() {
        let index = Arc::new(ThoughtsIndex::new());
        let handle = watch_roots(
            index,
            vec![root_at("/definitely/does/not/exist/hyprlayer-test")],
            Box::new(|_| {}),
        );
        assert!(handle.is_ok());
    }

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
        assert!(is_relevant_change(Path::new("/wt/.git/index")));
        assert!(is_relevant_change(Path::new("/wt/.git/HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/logs/HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/ORIG_HEAD")));
        assert!(is_relevant_change(Path::new("/wt/.git/MERGE_HEAD")));
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
        assert!(!is_relevant_change(Path::new(
            "/wt/.git/objects/ab/cdef123"
        )));
        assert!(!is_relevant_change(Path::new("/wt/.git/packed-refs")));
        assert!(!is_relevant_change(Path::new("/wt/.git/COMMIT_EDITMSG")));
        assert!(!is_relevant_change(Path::new("/wt/.git/FETCH_HEAD")));
        assert!(!is_relevant_change(Path::new(
            "/wt/.git/refs/remotes/origin/main"
        )));
        assert!(!is_relevant_change(Path::new(
            "/wt/.git/logs/refs/heads/feat"
        )));
        assert!(!is_relevant_change(Path::new("/wt/.git/index.lock")));
        assert!(!is_relevant_change(Path::new("/wt/.git/HEAD.lock")));
    }
}
