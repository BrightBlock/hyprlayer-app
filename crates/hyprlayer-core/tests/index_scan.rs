//! Integration tests for the thoughts indexer: scan, classify, query, upsert.

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

/// repos/myproj/{jt,shared}/{plans,research}/… and global/{shared,jt}/…,
/// including a doc with no frontmatter and one with malformed YAML.
fn build_tree() -> (TempDir, ThoughtsRoot) {
    let tmp = TempDir::new().unwrap();
    // macOS temp dirs live behind a /var -> /private/var symlink; canonicalize
    // so ids match what the watcher sees.
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
    // Real-corpus case: no frontmatter at all (cf. global/shared/pr_description.md).
    write(
        &root,
        DOC_NO_FM,
        "## describe_pr template\n\nNo frontmatter here.\n",
    );
    // Malformed YAML frontmatter degrades to warnings + path fallbacks.
    write(
        &root,
        DOC_MALFORMED,
        "---\n: : : not yaml [\n---\n\nBody of the malformed doc.\n",
    );
    // Outside the repos/global layout -> Area::Other.
    write(&root, DOC_STRAY, "---\nscope: global\n---\nstray body\n");
    // Never indexed.
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
    // readme.txt is visited and skipped; .obsidian is pruned before visiting.
    assert_eq!(stats.skipped, 1);
    assert_eq!(stats.errors, 0);

    assert!(index
        .get(&id_of(&root, "repos/myproj/jt/.obsidian/cache.md"))
        .is_none());
    assert!(index
        .get(&id_of(&root, "repos/myproj/jt/plans/readme.txt"))
        .is_none());

    // Rescan replaces, not duplicates.
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
    // No frontmatter: warning recorded, title falls back to the filename.
    assert!(!doc.fm_warnings.is_empty());
    assert_eq!(doc.fm.title.as_deref(), Some("pr description"));
    assert_eq!(doc.fm.date, None);

    let doc = index.get(&id_of(&root, DOC_MALFORMED)).unwrap();
    assert_eq!(doc.area, Area::User { name: "jt".into() });
    assert_eq!(doc.scope, "global");
    assert!(doc.fm_warnings.iter().any(|w| w.contains("YAML")));
    // Path fallbacks still fill date and type.
    assert_eq!(doc.fm.date.as_deref(), Some("2026-04-01"));
    assert_eq!(doc.fm.doc_type.as_deref(), Some("note"));

    // Area::Other takes its scope from fm.scope when present.
    let doc = index.get(&id_of(&root, DOC_STRAY)).unwrap();
    assert_eq!(doc.area, Area::Other);
    assert_eq!(doc.scope, "global");

    // doc_path round-trips the id.
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
    // malformed (title "malformed"), pr_description ("pr description"),
    // stray ("stray") — title sort ascending.
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

    // "refactor" is in the auth plan's title and the cache research's body.
    let q = ThoughtQuery {
        text: Some("refactor".into()),
        sort: SortKey::Relevance,
        ..Default::default()
    };
    let result = index.query(&q);
    assert_eq!(result.total, 2);
    assert_eq!(result.items[0].rel_path, DOC_AUTH_PLAN);
    assert_eq!(result.items[1].rel_path, DOC_CACHE_RESEARCH);

    // Every needle must match: only the auth plan has both words.
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

/// Bodies are read fresh from disk in query pass 2. A doc whose title and
/// tags don't contain the needle at all must still be found via its body,
/// and still rank below a doc whose title matches the same needle.
#[test]
fn body_only_match_ranks_below_title_match_and_is_returned() {
    let (_tmp, root, index) = scanned();

    let rel = "repos/myproj/jt/notes/2026-06-05-body-only.md";
    let abs = write(
        &root.root,
        rel,
        "---\ntitle: Unrelated Title\ntype: note\ntags: [misc]\n---\n\n\
         This note only mentions the refactor deep in the body.\n",
    );
    assert_eq!(
        index.upsert_path(&root, &abs),
        Some(abs.to_string_lossy().into_owned())
    );

    // "refactor" is in DOC_AUTH_PLAN's title, DOC_CACHE_RESEARCH's body, and
    // now this new doc's body only (title/tags don't contain it).
    let q = ThoughtQuery {
        text: Some("refactor".into()),
        sort: SortKey::Relevance,
        ..Default::default()
    };
    let result = index.query(&q);
    assert_eq!(result.total, 3);
    assert_eq!(result.items[0].rel_path, DOC_AUTH_PLAN);
    let tail: Vec<&str> = result.items[1..]
        .iter()
        .map(|m| m.rel_path.as_str())
        .collect();
    assert!(tail.contains(&DOC_CACHE_RESEARCH), "{tail:?}");
    assert!(tail.contains(&rel), "{tail:?}");
}

/// A file removed from disk between the filter pass (pass 1, under the read
/// lock) and the body-read pass (pass 2, unlocked I/O) must drop out of a
/// text query cleanly rather than panicking. A non-text query never opens
/// the file, so the (now stale) record it still holds is unaffected.
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

/// No frontmatter block at all means `body_offset_bytes` returns 0 — the
/// whole file is searched as body, matching `extract_block`'s degradation.
#[test]
fn doc_with_no_frontmatter_is_body_searchable_over_whole_content() {
    let (_tmp, _root, index) = scanned();

    // "describe_pr" only occurs in DOC_NO_FM's body text, never in its
    // filename-derived title ("pr description").
    let q = ThoughtQuery {
        text: Some("describe_pr".into()),
        ..Default::default()
    };
    assert_eq!(rel_paths(&index, &q), vec![DOC_NO_FM]);
}

/// A frontmatter block too large for the capped reads never shows its
/// closing delimiter, and both halves of the path then degrade exactly as
/// they do for genuinely unterminated frontmatter: at query time
/// `body_offset_bytes` returns 0, so the raw header text is scored as body;
/// at scan time `frontmatter::parse` falls back to path-derived metadata, so
/// the title matched here is the filename-derived one (asserted directly in
/// `scan_time_oversized_frontmatter_degrades_to_path_fallbacks_with_warning`).
/// The doc stays findable and nothing panics anywhere in the path.
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

    // Default sort: date desc, docs without a date last.
    let all = rel_paths(&index, &ThoughtQuery::default());
    assert_eq!(
        &all[..4],
        [
            DOC_CACHE_RESEARCH, // 2026-06-03
            DOC_AUTH_PLAN,      // 2026-06-01
            DOC_SHARED_ROADMAP, // 2026-05-20
            DOC_MALFORMED,      // 2026-04-01 (filename fallback)
        ]
    );
    // The two undated docs come last; their mutual order is an mtime
    // tiebreak that depends on file-creation timing.
    let mut tail = vec![all[4].as_str(), all[5].as_str()];
    tail.sort_unstable();
    assert_eq!(tail, vec![DOC_NO_FM, DOC_STRAY]);

    // Date bounds exclude undated docs entirely.
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

    // Rewriting identical bytes (mtime touch) does not reindex.
    let original = fs::read(&abs).unwrap();
    fs::write(&abs, &original).unwrap();
    assert_eq!(index.upsert_path(&root, &abs), None);

    // A real content change does, and the meta reflects it.
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

    // Ignored and unreadable paths yield None.
    assert_eq!(
        index.upsert_path(&root, &root.root.join("repos/myproj/jt/plans/readme.txt")),
        None
    );
    assert_eq!(
        index.upsert_path(&root, &root.root.join("repos/myproj/jt/plans/missing.md")),
        None
    );
}

/// A bare mtime touch (`git checkout`, `hyprlayer thoughts sync` rewriting
/// identical bytes) must not be reported as a change — but the stored
/// `mtime_ms` is still refreshed silently, so mtime-sorted lists and the
/// Dashboard's most-recent window stay accurate.
#[test]
fn bare_mtime_touch_returns_none_but_refreshes_stored_mtime() {
    let (_tmp, root, index) = scanned();
    let abs = root.root.join(DOC_AUTH_PLAN);
    let id = id_of(&root, DOC_AUTH_PLAN);

    let before_mtime = index.get(&id).unwrap().mtime_ms;

    // Bump mtime without changing a single byte.
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

/// A body-only edit that changes the file's total size is a real change
/// under the new (frontmatter-prefix-hash + size) basis, even though the
/// frontmatter block itself is untouched.
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

/// Accepted divergence 2: a body edit that lands *past* the scan-time
/// `FRONTMATTER_READ_CAP` prefix and preserves total file size leaves the
/// (frontmatter-prefix-hash + size) basis unchanged, so it is treated the
/// same as a bare mtime touch — refreshed silently, no `Some(id)`.
#[test]
fn body_edit_past_frontmatter_cap_that_preserves_size_refreshes_silently() {
    let (_tmp, root, index) = scanned();

    // Frontmatter is tiny; a long body pushes the file well past the 64 KiB
    // scan-time prefix cap, so a marker swap deep inside it falls outside
    // the window the prefix hash covers.
    let filler = "x".repeat(80 * 1024);
    let content = format!("---\ntitle: Big Body Doc\ntype: note\n---\n\n{filler}MARKER_A end.\n");
    let rel = "repos/myproj/jt/notes/2026-06-07-big-body.md";
    let abs = write(&root.root, rel, &content);
    let id = abs.to_string_lossy().into_owned();
    assert_eq!(index.upsert_path(&root, &abs), Some(id.clone()));

    let before_mtime = index.get(&id).unwrap().mtime_ms;

    // Same-length swap, past the prefix cap.
    let updated = content.replace("MARKER_A", "MARKER_B");
    assert_eq!(updated.len(), content.len());
    fs::write(&abs, &updated).unwrap();
    let bumped = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::open(&abs)
        .unwrap()
        .set_modified(bumped)
        .unwrap();

    // No event-worthy change is reported...
    assert_eq!(index.upsert_path(&root, &abs), None);
    // ...but the stored record is still refreshed (mtime moves).
    let after_mtime = index.get(&id).unwrap().mtime_ms;
    assert!(
        after_mtime > before_mtime,
        "{before_mtime} -> {after_mtime}"
    );
}

/// Accepted divergence 3, scan-time half: a frontmatter block itself larger
/// than `FRONTMATTER_READ_CAP` never shows its closing delimiter within the
/// scan-time prefix read, so `frontmatter::parse` degrades to
/// `Frontmatter::default()` + path fallbacks + a warning — the doc still
/// indexes (with `size_bytes` from the real, uncapped stat) rather than the
/// scan erroring.
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
    // Path fallback fills the title from the filename, not the (unreachable)
    // frontmatter title.
    assert_eq!(doc.fm.title.as_deref(), Some("huge frontmatter scan"));
    assert!(
        doc.fm_warnings
            .iter()
            .any(|w| w.contains("no frontmatter block")),
        "{:?}",
        doc.fm_warnings
    );
    // size_bytes still reflects the real (uncapped) file size from stat.
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

    // Removing again is a no-op.
    assert_eq!(index.remove_path(&abs), None);
}

#[test]
fn full_scan_preserves_other_roots() {
    let (_tmp_a, root_a, index) = scanned();
    let (_tmp_b, root_b) = build_tree();

    index.full_scan(std::slice::from_ref(&root_b));
    // Both roots' docs coexist…
    assert_eq!(index.query(&ThoughtQuery::default()).total, 12);
    // …and rescanning one root does not disturb the other.
    index.full_scan(std::slice::from_ref(&root_a));
    assert_eq!(index.query(&ThoughtQuery::default()).total, 12);
    assert!(index.get(&id_of(&root_b, DOC_AUTH_PLAN)).is_some());
}

/// A doc the agent wrote through a worktree's `thoughts/` symlink is opened by
/// that symlinked path, but the index keys it by its real location under the
/// thoughts root. `resolve_id` must bridge the two so the doc still loads.
#[cfg(unix)]
#[test]
fn resolve_id_follows_worktree_symlinks() {
    use std::os::unix::fs::symlink;

    let (_tmp, root, index) = scanned();

    // An already-canonical id resolves to itself (the library's fast path).
    let canonical = id_of(&root, DOC_SHARED_ROADMAP);
    assert_eq!(index.resolve_id(&canonical), Some(canonical.clone()));

    // Mirror the per-worktree layout: <wt>/thoughts/shared -> the repo's shared
    // tree, the symlink an agent's `Write` tool path points through.
    let wt = TempDir::new().unwrap();
    let thoughts = wt.path().join("thoughts");
    fs::create_dir_all(&thoughts).unwrap();
    symlink(
        root.root.join("repos/myproj/shared"),
        thoughts.join("shared"),
    )
    .unwrap();

    // The symlinked path the UI captured resolves to the canonical index id.
    let via_link = thoughts
        .join("shared/plans/2026-05-20-shared-roadmap.md")
        .to_string_lossy()
        .into_owned();
    assert_ne!(via_link, canonical);
    assert_eq!(index.resolve_id(&via_link), Some(canonical));

    // A symlinked path to a file that isn't indexed (and a plain missing path)
    // stay unresolved rather than resolving to some unindexed id.
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
