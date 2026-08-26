use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hyprlayer_core::config::BackendKind;
use hyprlayer_core::index::watcher::{watch_roots, IndexEvent};
use hyprlayer_core::index::{ThoughtsIndex, ThoughtsRoot};
use tempfile::TempDir;

const EVENT_DEADLINE: Duration = Duration::from_secs(5);

fn wait_for<F: Fn(&IndexEvent) -> bool>(
    rx: &mpsc::Receiver<IndexEvent>,
    pred: F,
) -> Option<IndexEvent> {
    let deadline = Instant::now() + EVENT_DEADLINE;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(event) if pred(&event) => return Some(event),
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    None
}

#[test]
fn watcher_indexes_created_modified_and_removed_files() {
    let tmp = TempDir::new().unwrap();
    let root_path = fs::canonicalize(tmp.path()).unwrap();
    fs::create_dir_all(root_path.join("repos/myproj/jt/plans")).unwrap();

    let root = ThoughtsRoot {
        root: root_path.clone(),
        repos_dir: "repos".into(),
        global_dir: "global".into(),
        backend: BackendKind::Git,
        profile: None,
    };

    let index = Arc::new(ThoughtsIndex::new());
    index.full_scan(std::slice::from_ref(&root));

    let (tx, rx) = mpsc::channel::<IndexEvent>();
    let handle = watch_roots(
        Arc::clone(&index),
        vec![root.clone()],
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    )
    .expect("watcher should start");

    let doc = root_path.join("repos/myproj/jt/plans/2026-06-12-watched.md");
    let id = doc.to_string_lossy().into_owned();
    fs::write(
        &doc,
        "---\ntitle: Watched Doc\ntype: plan\ndate: 2026-06-12\n---\n\nhello\n",
    )
    .unwrap();

    let updated = wait_for(
        &rx,
        |e| matches!(e, IndexEvent::Updated { ids } if ids.contains(&id)),
    );
    assert!(updated.is_some(), "expected Updated event for {id}");
    let meta = index.get(&id).expect("doc should be indexed");
    assert_eq!(meta.fm.title.as_deref(), Some("Watched Doc"));

    fs::write(
        &doc,
        "---\ntitle: Watched Doc v2\ntype: plan\ndate: 2026-06-12\n---\n\nhello again\n",
    )
    .unwrap();
    let updated = wait_for(
        &rx,
        |e| matches!(e, IndexEvent::Updated { ids } if ids.contains(&id)),
    );
    assert!(updated.is_some(), "expected Updated event after modify");
    assert_eq!(
        index.get(&id).unwrap().fm.title.as_deref(),
        Some("Watched Doc v2")
    );

    fs::remove_file(&doc).unwrap();
    let removed = wait_for(
        &rx,
        |e| matches!(e, IndexEvent::Removed { ids } if ids.contains(&id)),
    );
    assert!(removed.is_some(), "expected Removed event for {id}");
    assert!(index.get(&id).is_none());

    let ignored = root_path.join("repos/myproj/jt/plans/.hidden.md");
    fs::write(&ignored, "ignored").unwrap();
    let ignored_id = ignored.to_string_lossy().into_owned();
    let event = wait_for_quiet(&rx, Duration::from_millis(800), &ignored_id);
    assert!(event.is_none(), "ignored file should not produce an event");
    assert!(index.get(&ignored_id).is_none());

    drop(handle);
}

fn wait_for_quiet(
    rx: &mpsc::Receiver<IndexEvent>,
    window: Duration,
    id: &str,
) -> Option<IndexEvent> {
    let deadline = Instant::now() + window;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok(event) => {
                let mentions = match &event {
                    IndexEvent::Updated { ids } | IndexEvent::Removed { ids } => {
                        ids.iter().any(|i| i == id)
                    }
                    IndexEvent::Rescanned => false,
                };
                if mentions {
                    return Some(event);
                }
            }
            Err(_) => break,
        }
    }
    None
}

#[test]
fn watcher_handles_renames_as_remove_plus_upsert() {
    let tmp = TempDir::new().unwrap();
    let root_path = fs::canonicalize(tmp.path()).unwrap();
    fs::create_dir_all(root_path.join("global/shared")).unwrap();

    let root = ThoughtsRoot {
        root: root_path.clone(),
        repos_dir: "repos".into(),
        global_dir: "global".into(),
        backend: BackendKind::Git,
        profile: None,
    };

    let old = root_path.join("global/shared/old-name.md");
    fs::write(&old, "---\ntitle: Renamed\n---\nbody\n").unwrap();

    let index = Arc::new(ThoughtsIndex::new());
    index.full_scan(std::slice::from_ref(&root));
    let old_id = old.to_string_lossy().into_owned();
    assert!(index.get(&old_id).is_some());

    let (tx, rx) = mpsc::channel::<IndexEvent>();
    let _handle = watch_roots(
        Arc::clone(&index),
        vec![root.clone()],
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    )
    .expect("watcher should start");

    let new = root_path.join("global/shared/new-name.md");
    let new_id = new.to_string_lossy().into_owned();
    fs::rename(&old, &new).unwrap();

    let deadline = Instant::now() + EVENT_DEADLINE;
    let (mut saw_update, mut saw_remove) = (false, false);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if saw_update && saw_remove {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(IndexEvent::Updated { ids }) if ids.contains(&new_id) => saw_update = true,
            Ok(IndexEvent::Removed { ids }) if ids.contains(&old_id) => saw_remove = true,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(saw_update, "expected Updated for the rename target");
    assert!(saw_remove, "expected Removed for the rename source");
    assert!(index.get(&old_id).is_none());
    assert!(index.get(&new_id).is_some());
}

#[test]
fn watch_roots_tolerates_missing_root() {
    let index = Arc::new(ThoughtsIndex::new());
    let root = ThoughtsRoot {
        root: Path::new("/definitely/does/not/exist/hyprlayer-watch").to_path_buf(),
        repos_dir: "repos".into(),
        global_dir: "global".into(),
        backend: BackendKind::Git,
        profile: None,
    };
    let handle = watch_roots(index, vec![root], Box::new(|_| {}));
    assert!(handle.is_ok());
}
