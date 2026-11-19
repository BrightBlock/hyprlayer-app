use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hyprlayer_core::config::BackendKind;
use hyprlayer_core::index::{Area, SortKey, TagMode, ThoughtQuery, ThoughtsIndex, ThoughtsRoot};
use tempfile::TempDir;

const DOC_AUTH_PLAN: &str = "repos/myproj/jt/plans/2026-06-01-auth-refactor.md";
const DOC_CACHE_RESEARCH: &str = "repos/myproj/jt/research/2026-06-03-cache-research.md";
const DOC_SHARED_ROADMAP: &str = "repos/myproj/shared/plans/2026-05-20-shared-roadmap.md";
const DOC_NO_FM: &str = "global/shared/pr_description.md";
const DOC_MALFORMED: &str = "global/jt/notes/2026-04-01-malformed.md";
const DOC_STRAY: &str = "stray.md";

fn write(root: &Path, rel: &str, content: &str) -> PathBuf {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

fn build_tree() -> (TempDir, ThoughtsRoot) {
    let tmp = TempDir::new().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();

    write(
        &root,
        DOC_AUTH_PLAN,
        "---\ntitle: Auth Refactor Plan\ntype: plan\ndate: 2026-06-01\nstatus: active\n\
         tags: [security, backend]\nauthor: jt\n---\n\nRotate the signing tokens carefully.\n",
    );
    write(
        &root,
        DOC_CACHE_RESEARCH,
        "---\ntitle: Cache Research\ntype: research\ndate: 2026-06-03\nstatus: complete\n\
         tags: [perf]\nauthor: jt\n---\n\nThe refactor made the cache faster.\n",
    );
    write(
        &root,
        DOC_SHARED_ROADMAP,
        "---\ntitle: Shared Roadmap\ntype: plan\ndate: 2026-05-20\nstatus: draft\n\
         tags: [roadmap]\nauthor: alice\n---\n\nQuarterly goals.\n",
    );
    write(
        &root,
        DOC_NO_FM,
        "## describe_pr template\n\nNo frontmatter here.\n",
    );
    write(
        &root,
        DOC_MALFORMED,
        "---\n: : : not yaml [\n---\n\nBody of the malformed doc.\n",
    );
    write(&root, DOC_STRAY, "---\nscope: global\n---\nstray body\n");
    write(&root, "repos/myproj/jt/.obsidian/cache.md", "ignored");
    write(&root, "repos/myproj/jt/plans/readme.txt", "not markdown");

    let thoughts_root = ThoughtsRoot {
        root,
        repos_dir: "repos".into(),
        global_dir: "global".into(),
        backend: BackendKind::Git,
        profile: None,
    };
    (tmp, thoughts_root)
}

fn scanned() -> (TempDir, ThoughtsRoot, ThoughtsIndex) {
    let (tmp, root) = build_tree();
    let index = ThoughtsIndex::new();
    index.full_scan(std::slice::from_ref(&root));
    (tmp, root, index)
}

fn id_of(root: &ThoughtsRoot, rel: &str) -> String {
    root.root.join(rel).to_string_lossy().into_owned()
}

fn rel_paths(index: &ThoughtsIndex, q: &ThoughtQuery) -> Vec<String> {
    index
        .query(q)
        .items
        .into_iter()
        .map(|m| m.rel_path)
        .collect()
}

#[test]
fn full_scan_counts_and_skips() {
    let (_tmp, root) = build_tree();
    let index = ThoughtsIndex::new();
    let stats = index.full_scan(std::slice::from_ref(&root));

    assert_eq!(stats.indexed, 6, "six markdown docs are indexable");
    assert_eq!(stats.skipped, 1);
    assert_eq!(stats.errors, 0);

    assert!(index
        .get(&id_of(&root, "repos/myproj/jt/.obsidian/cache.md"))
        .is_none());
    assert!(index
        .get(&id_of(&root, "repos/myproj/jt/plans/readme.txt"))
        .is_none());

    let stats = index.full_scan(std::slice::from_ref(&root));
    assert_eq!(stats.indexed, 6);
    assert_eq!(index.query(&ThoughtQuery::default()).total, 6);
}

#[test]
fn classification_project_area_scope() {
    let (_tmp, root, index) = scanned();

    let doc = index.get(&id_of(&root, DOC_AUTH_PLAN)).unwrap();
    assert_eq!(doc.project.as_deref(), Some("myproj"));
    assert_eq!(doc.area, Area::User { name: "jt".into() });
    assert_eq!(doc.scope, "user");
    assert_eq!(doc.backend, "git");
    assert_eq!(doc.profile, None);
    assert_eq!(doc.rel_path, DOC_AUTH_PLAN);
    assert!(doc.size_bytes > 0.0);
    assert!(doc.mtime_ms > 0.0);

    let doc = index.get(&id_of(&root, DOC_SHARED_ROADMAP)).unwrap();
    assert_eq!(doc.project.as_deref(), Some("myproj"));
    assert_eq!(doc.area, Area::Shared);
    assert_eq!(doc.scope, "shared");

    let doc = index.get(&id_of(&root, DOC_NO_FM)).unwrap();
    assert_eq!(doc.project, None);
    assert_eq!(doc.area, Area::Shared);
    assert_eq!(doc.scope, "global");
    assert!(!doc.fm_warnings.is_empty());
    assert_eq!(doc.fm.title.as_deref(), Some("pr description"));
    assert_eq!(doc.fm.date, None);

    let doc = index.get(&id_of(&root, DOC_MALFORMED)).unwrap();
    assert_eq!(doc.area, Area::User { name: "jt".into() });
    assert_eq!(doc.scope, "global");
    assert!(doc.fm_warnings.iter().any(|w| w.contains("YAML")));
    assert_eq!(doc.fm.date.as_deref(), Some("2026-04-01"));
    assert_eq!(doc.fm.doc_type.as_deref(), Some("note"));

    let doc = index.get(&id_of(&root, DOC_STRAY)).unwrap();
    assert_eq!(doc.area, Area::Other);
    assert_eq!(doc.scope, "global");

    let id = id_of(&root, DOC_AUTH_PLAN);
    assert_eq!(index.doc_path(&id), Some(PathBuf::from(&id)));
    assert!(index.doc_path("/nope.md").is_none());
}

#[test]
fn query_filters_combine_with_and() {
    let (_tmp, _root, index) = scanned();

    let q = ThoughtQuery {
        types: vec!["plan".into()],
        ..Default::default()
    };
    assert_eq!(
        rel_paths(&index, &q),
        vec![DOC_AUTH_PLAN, DOC_SHARED_ROADMAP]
    );

    let q = ThoughtQuery {
        types: vec!["PLAN".into()],
        statuses: vec!["Active".into()],
        projects: vec!["myproj".into()],
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![DOC_AUTH_PLAN]);

    let q = ThoughtQuery {
        authors: vec!["alice".into()],
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![DOC_SHARED_ROADMAP]);

    let q = ThoughtQuery {
        scopes: vec!["global".into()],
        sort: SortKey::Title,
        ..Default::default()
    };
    assert_eq!(
        rel_paths(&index, &q),
        vec![DOC_MALFORMED, DOC_NO_FM, DOC_STRAY]
    );
}

#[test]
fn tag_filters_respect_or_and_modes() {
    let (_tmp, _root, index) = scanned();

    let or_q = ThoughtQuery {
        tags: vec!["security".into(), "perf".into()],
        ..Default::default()
    };
    assert_eq!(
        rel_paths(&index, &or_q),
        vec![DOC_CACHE_RESEARCH, DOC_AUTH_PLAN]
    );

    let and_q = ThoughtQuery {
        tags: vec!["security".into(), "backend".into()],
        tag_mode: TagMode::And,
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &and_q), vec![DOC_AUTH_PLAN]);

    let and_miss = ThoughtQuery {
        tags: vec!["security".into(), "perf".into()],
        tag_mode: TagMode::And,
        ..Default::default()
    };
    assert_eq!(index.query(&and_miss).total, 0);
}

#[test]
fn text_search_ranks_title_hits_above_body_hits() {
    let (_tmp, _root, index) = scanned();

    let q = ThoughtQuery {
        text: Some("refactor".into()),
        sort: SortKey::Relevance,
        ..Default::default()
    };
    let result = index.query(&q);
    assert_eq!(result.total, 2);
    assert_eq!(result.items[0].rel_path, DOC_AUTH_PLAN);
    assert_eq!(result.items[1].rel_path, DOC_CACHE_RESEARCH);

    let q = ThoughtQuery {
        text: Some("refactor tokens".into()),
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![DOC_AUTH_PLAN]);

    let q = ThoughtQuery {
        text: Some("zzz-nothing-matches".into()),
        ..Default::default()
    };
    assert_eq!(index.query(&q).total, 0);
}

#[test]
fn file_removed_between_query_passes_drops_out_of_text_search_only() {
    let (_tmp, root, index) = scanned();
    let abs = root.root.join(DOC_CACHE_RESEARCH);
    fs::remove_file(&abs).unwrap();

    let q = ThoughtQuery {
        text: Some("refactor".into()),
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![DOC_AUTH_PLAN]);

    let all = rel_paths(&index, &ThoughtQuery::default());
    assert!(all.contains(&DOC_CACHE_RESEARCH.to_string()), "{all:?}");
}

#[test]
fn doc_with_no_frontmatter_is_body_searchable_over_whole_content() {
    let (_tmp, _root, index) = scanned();

    let q = ThoughtQuery {
        text: Some("describe_pr".into()),
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![DOC_NO_FM]);
}

#[test]
fn oversized_frontmatter_degrades_at_query_time_without_panic() {
    let (_tmp, root, index) = scanned();

    let padding = "x".repeat(400 * 1024);
    let content = format!(
        "---\ntitle: Huge Frontmatter Doc\ntype: note\npadding: \"{padding}\"\n---\n\nBody text.\n"
    );
    let rel = "repos/myproj/jt/notes/2026-06-06-huge-frontmatter.md";
    let abs = write(&root.root, rel, &content);
    assert_eq!(
        index.upsert_path(&root, &abs),
        Some(abs.to_string_lossy().into_owned())
    );

    let q = ThoughtQuery {
        text: Some("huge frontmatter".into()),
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![rel]);
}

#[test]
fn date_sort_and_bounds_handle_missing_dates() {
    let (_tmp, _root, index) = scanned();

    let all = rel_paths(&index, &ThoughtQuery::default());
    assert_eq!(
        &all[..4],
        [
            DOC_CACHE_RESEARCH,
            DOC_AUTH_PLAN,
            DOC_SHARED_ROADMAP,
            DOC_MALFORMED,
        ]
    );
    let mut tail = vec![all[4].as_str(), all[5].as_str()];
    tail.sort_unstable();
    assert_eq!(tail, vec![DOC_NO_FM, DOC_STRAY]);

    let q = ThoughtQuery {
        date_from: Some("2026-05-01".into()),
        ..Default::default()
    };
    assert_eq!(
        rel_paths(&index, &q),
        vec![DOC_CACHE_RESEARCH, DOC_AUTH_PLAN, DOC_SHARED_ROADMAP]
    );

    let q = ThoughtQuery {
        date_from: Some("2026-05-01".into()),
        date_to: Some("2026-06-01".into()),
        ..Default::default()
    };
    assert_eq!(
        rel_paths(&index, &q),
        vec![DOC_AUTH_PLAN, DOC_SHARED_ROADMAP]
    );
}

#[test]
fn limit_offset_report_full_total() {
    let (_tmp, _root, index) = scanned();
    let q = ThoughtQuery {
        limit: Some(2),
        offset: Some(1),
        ..Default::default()
    };
    let result = index.query(&q);
    assert_eq!(result.total, 6);
    assert_eq!(result.items.len(), 2);
    assert_eq!(result.items[0].rel_path, DOC_AUTH_PLAN);
    assert_eq!(result.items[1].rel_path, DOC_SHARED_ROADMAP);

    let q = ThoughtQuery {
        offset: Some(100),
        ..Default::default()
    };
    let result = index.query(&q);
    assert_eq!(result.total, 6);
    assert!(result.items.is_empty());
}

#[test]
fn upsert_hash_short_circuit_and_content_change() {
    let (_tmp, root, index) = scanned();
    let abs = root.root.join(DOC_AUTH_PLAN);
    let id = id_of(&root, DOC_AUTH_PLAN);

    let original = fs::read(&abs).unwrap();
    fs::write(&abs, &original).unwrap();
    assert_eq!(index.upsert_path(&root, &abs), None);

    let updated = String::from_utf8(original)
        .unwrap()
        .replace("title: Auth Refactor Plan", "title: Auth Refactor Plan v2");
    fs::write(&abs, updated).unwrap();
    assert_eq!(index.upsert_path(&root, &abs), Some(id.clone()));
    assert_eq!(
        index.get(&id).unwrap().fm.title.as_deref(),
        Some("Auth Refactor Plan v2")
    );
    assert_eq!(index.query(&ThoughtQuery::default()).total, 6);

    assert_eq!(
        index.upsert_path(&root, &root.root.join("repos/myproj/jt/plans/readme.txt")),
        None
    );
    assert_eq!(
        index.upsert_path(&root, &root.root.join("repos/myproj/jt/plans/missing.md")),
        None
    );
}

#[test]
fn bare_mtime_touch_returns_none_but_refreshes_stored_mtime() {
    let (_tmp, root, index) = scanned();
    let abs = root.root.join(DOC_AUTH_PLAN);
    let id = id_of(&root, DOC_AUTH_PLAN);

    let before_mtime = index.get(&id).unwrap().mtime_ms;

    let bumped = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::open(&abs)
        .unwrap()
        .set_modified(bumped)
        .unwrap();

    assert_eq!(index.upsert_path(&root, &abs), None);
    let after_mtime = index.get(&id).unwrap().mtime_ms;
    assert!(
        after_mtime > before_mtime,
        "{before_mtime} -> {after_mtime}"
    );
}

#[test]
fn body_edit_that_changes_size_reindexes() {
    let (_tmp, root, index) = scanned();
    let abs = root.root.join(DOC_AUTH_PLAN);
    let id = id_of(&root, DOC_AUTH_PLAN);

    let original = fs::read_to_string(&abs).unwrap();
    let updated = original.replace(
        "Rotate the signing tokens carefully.\n",
        "Rotate the signing tokens carefully, and audit the logs too.\n",
    );
    assert_ne!(original.len(), updated.len());
    fs::write(&abs, &updated).unwrap();

    assert_eq!(index.upsert_path(&root, &abs), Some(id.clone()));
    let doc = index.get(&id).unwrap();
    assert_eq!(doc.size_bytes, updated.len() as f64);
}

#[test]
fn body_edit_past_frontmatter_cap_that_preserves_size_refreshes_silently() {
    let (_tmp, root, index) = scanned();

    let filler = "x".repeat(80 * 1024);
    let content = format!("---\ntitle: Big Body Doc\ntype: note\n---\n\n{filler}MARKER_A end.\n");
    let rel = "repos/myproj/jt/notes/2026-06-07-big-body.md";
    let abs = write(&root.root, rel, &content);
    let id = abs.to_string_lossy().into_owned();
    assert_eq!(index.upsert_path(&root, &abs), Some(id.clone()));

    let before_mtime = index.get(&id).unwrap().mtime_ms;

    let updated = content.replace("MARKER_A", "MARKER_B");
    assert_eq!(updated.len(), content.len());
    fs::write(&abs, &updated).unwrap();
    let bumped = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::open(&abs)
        .unwrap()
        .set_modified(bumped)
        .unwrap();

    assert_eq!(index.upsert_path(&root, &abs), None);
    let after_mtime = index.get(&id).unwrap().mtime_ms;
    assert!(
        after_mtime > before_mtime,
        "{before_mtime} -> {after_mtime}"
    );
}

#[test]
fn scan_time_oversized_frontmatter_degrades_to_path_fallbacks_with_warning() {
    let (_tmp, root, index) = scanned();

    let padding = "x".repeat(70 * 1024);
    let content = format!("---\ntitle: Should Not Parse\npadding: \"{padding}\"\n---\n\nBody.\n");
    let rel = "repos/myproj/jt/notes/2026-06-08-huge-frontmatter-scan.md";
    let abs = write(&root.root, rel, &content);
    let id = abs.to_string_lossy().into_owned();

    assert_eq!(index.upsert_path(&root, &abs), Some(id.clone()));
    let doc = index.get(&id).unwrap();
    assert_eq!(doc.fm.title.as_deref(), Some("huge frontmatter scan"));
    assert!(
        doc.fm_warnings
            .iter()
            .any(|w| w.contains("no frontmatter block")),
        "{:?}",
        doc.fm_warnings
    );
    assert_eq!(doc.size_bytes, content.len() as f64);
}

#[test]
fn remove_drops_the_doc() {
    let (_tmp, root, index) = scanned();
    let abs = root.root.join(DOC_STRAY);
    let id = id_of(&root, DOC_STRAY);

    fs::remove_file(&abs).unwrap();
    assert_eq!(index.remove_path(&abs), Some(id.clone()));
    assert!(index.get(&id).is_none());
    assert!(index.doc_path(&id).is_none());
    assert_eq!(index.query(&ThoughtQuery::default()).total, 5);

    assert_eq!(index.remove_path(&abs), None);
}

#[test]
fn full_scan_preserves_other_roots() {
    let (_tmp_a, root_a, index) = scanned();
    let (_tmp_b, root_b) = build_tree();

    index.full_scan(std::slice::from_ref(&root_b));
    assert_eq!(index.query(&ThoughtQuery::default()).total, 12);
    index.full_scan(std::slice::from_ref(&root_a));
    assert_eq!(index.query(&ThoughtQuery::default()).total, 12);
    assert!(index.get(&id_of(&root_b, DOC_AUTH_PLAN)).is_some());
}

#[cfg(unix)]
#[test]
fn resolve_id_follows_worktree_symlinks() {
    use std::os::unix::fs::symlink;

    let (_tmp, root, index) = scanned();

    let canonical = id_of(&root, DOC_SHARED_ROADMAP);
    assert_eq!(index.resolve_id(&canonical), Some(canonical.clone()));

    let wt = TempDir::new().unwrap();
    let thoughts = wt.path().join("thoughts");
    fs::create_dir_all(&thoughts).unwrap();
    symlink(
        root.root.join("repos/myproj/shared"),
        thoughts.join("shared"),
    )
    .unwrap();

    let via_link = thoughts
        .join("shared/plans/2026-05-20-shared-roadmap.md")
        .to_string_lossy()
        .into_owned();
    assert_ne!(via_link, canonical);
    assert_eq!(index.resolve_id(&via_link), Some(canonical));

    let missing = thoughts
        .join("shared/plans/2026-01-01-nope.md")
        .to_string_lossy()
        .into_owned();
    assert_eq!(index.resolve_id(&missing), None);
    assert_eq!(index.resolve_id("/definitely/missing.md"), None);
}

#[test]
fn missing_root_scans_to_empty_stats() {
    let index = ThoughtsIndex::new();
    let root = ThoughtsRoot {
        root: PathBuf::from("/definitely/does/not/exist/hyprlayer-test"),
        repos_dir: "repos".into(),
        global_dir: "global".into(),
        backend: BackendKind::Git,
        profile: None,
    };
    let stats = index.full_scan(&[root]);
    assert_eq!(stats.indexed, 0);
    assert_eq!(stats.errors, 0);
}
