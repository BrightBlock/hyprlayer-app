//! In-memory index of every markdown document across the thoughts roots.
//! PUBLIC API IS FROZEN — the Tauri shell builds against it. Scan/query methods
//! are synchronous; async callers wrap them in `spawn_blocking`. Personal-corpus
//! scale (see `search.rs`).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use super::roots::ThoughtsRoot;
use super::search;
use crate::docs::frontmatter::{self, Frontmatter};

/// Search bodies are read fresh from disk at query time and capped so one
/// pathological file cannot stall a query.
const BODY_SEARCH_CAP: usize = 256 * 1024;
/// Frontmatter reads (scan-time prefix, query-time body-offset skip) are
/// capped independently of the body cap so one pathological frontmatter
/// block cannot force a large read. The largest frontmatter block in the
/// real corpus is a few hundred bytes.
const FRONTMATTER_READ_CAP: usize = 64 * 1024;
/// Defense against symlink cycles when walking with `follow_links(true)`.
const MAX_SCAN_DEPTH: usize = 12;

/// Where a document lives within its root's layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum Area {
    /// `<repos|global>/<…>/<user>/…` — a specific user's documents.
    User { name: String },
    /// `<repos|global>/<…>/shared/…`.
    Shared,
    /// Anything outside the repos/global layout.
    Other,
}

/// Everything the UI needs to list and filter a document, sans body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DocMeta {
    /// Absolute path string — the canonical unique id.
    pub id: String,
    /// Path relative to its root, `/`-separated on every platform.
    pub rel_path: String,
    /// Mapped repo name when the doc lives under `<repos_dir>/<name>/…`.
    pub project: Option<String>,
    pub area: Area,
    /// `"user" | "shared" | "global"` derived from the path; for
    /// [`Area::Other`] falls back to `fm.scope` or `"shared"`.
    pub scope: String,
    /// Profile that owns the root, `None` for the default backend.
    pub profile: Option<String>,
    /// `"git" | "obsidian"`.
    pub backend: String,
    pub fm: Frontmatter,
    pub fm_warnings: Vec<String>,
    pub mtime_ms: f64,
    pub size_bytes: f64,
}

/// How multiple `tags` filters combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum TagMode {
    And,
    #[default]
    Or,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    /// Text-match score; without `text` this falls through to `Date`.
    Relevance,
    #[default]
    Date,
    Mtime,
    Title,
}

/// A library query: filters AND across fields, OR within a field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct ThoughtQuery {
    pub text: Option<String>,
    pub types: Vec<String>,
    pub statuses: Vec<String>,
    pub projects: Vec<String>,
    pub scopes: Vec<String>,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
    pub tag_mode: TagMode,
    /// Inclusive lexicographic bounds against `fm.date` (ISO dates sort).
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub sort: SortKey,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// Matches before limit/offset.
    pub total: u32,
    pub items: Vec<DocMeta>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ScanStats {
    pub indexed: u32,
    pub skipped: u32,
    pub errors: u32,
}

/// One indexed document: metadata plus the change-detection hash. No body
/// text is resident — a text query reads and scores bodies fresh from disk
/// (see `read_body_for_search`), so memory stays flat with corpus size.
#[derive(Debug)]
struct DocRecord {
    meta: DocMeta,
    content_hash: u64,
}

/// Thread-safe in-memory document index keyed by absolute path.
#[derive(Debug, Default)]
pub struct ThoughtsIndex {
    records: RwLock<HashMap<String, Arc<DocRecord>>>,
}

impl ThoughtsIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Walk every root and rebuild the index entries under them (entries
    /// belonging to other roots are preserved). Synchronous by design —
    /// callers use `spawn_blocking`.
    pub fn full_scan(&self, roots: &[ThoughtsRoot]) -> ScanStats {
        let mut stats = ScanStats::default();
        let mut fresh: HashMap<String, Arc<DocRecord>> = HashMap::new();

        for root in roots {
            if !root.root.is_dir() {
                tracing::warn!(root = %root.root.display(), "thoughts root missing, skipping scan");
                continue;
            }
            let walker = WalkDir::new(&root.root)
                .follow_links(true)
                .max_depth(MAX_SCAN_DEPTH)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0 || !is_ignored_component(&e.file_name().to_string_lossy())
                });
            for entry in walker {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        tracing::warn!(error = %err, "scan error");
                        stats.errors += 1;
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let abs = entry.path();
                let Some(parts) = Self::classify(root, abs) else {
                    stats.skipped += 1;
                    continue;
                };
                // walkdir already stat'd this entry while walking (needed for
                // `file_type()` under `follow_links`), so this reuses that
                // rather than issuing a fresh `std::fs::metadata` call.
                let meta = match entry.metadata() {
                    Ok(meta) => meta,
                    Err(err) => {
                        tracing::warn!(error = %err, path = %abs.display(), "stat error");
                        stats.errors += 1;
                        continue;
                    }
                };
                let Some(prefix) = read_capped_prefix(abs, FRONTMATTER_READ_CAP) else {
                    stats.errors += 1;
                    continue;
                };
                let record = build_record(root, abs, parts, &meta, prefix);
                fresh.insert(record.meta.id.clone(), Arc::new(record));
                stats.indexed += 1;
            }
        }

        let mut map = self.records.write().expect("index lock poisoned");
        map.retain(|id, _| !roots.iter().any(|r| Path::new(id).starts_with(&r.root)));
        map.extend(fresh);
        stats
    }

    /// (Re)index a single file. Returns `Some(id)` when consumers should be
    /// notified of a real change; `None` when the path is ignored,
    /// unreadable, or the change basis is unchanged.
    ///
    /// Change basis: the frontmatter prefix hash plus file size. mtime is
    /// deliberately excluded — a bare touch that rewrites identical bytes
    /// (`git checkout`, `hyprlayer thoughts sync`) must not emit an update
    /// event, which is the property the old whole-file hash provided and
    /// PR #221 cared about. When only mtime moved, the stored record is
    /// still refreshed (so mtime-sorted lists and the Dashboard's
    /// most-recent window stay accurate) but silently — `None` is returned,
    /// no event fires. Accepted gap: a body edit past the frontmatter prefix
    /// cap that preserves file size refreshes the record silently too (see
    /// the plan's accepted divergences).
    pub fn upsert_path(&self, root: &ThoughtsRoot, abs: &Path) -> Option<String> {
        let parts = Self::classify(root, abs)?;
        let id = abs.to_string_lossy().into_owned();
        let meta = std::fs::metadata(abs).ok()?;
        let prefix = read_capped_prefix(abs, FRONTMATTER_READ_CAP)?;
        let size_bytes = meta.len() as f64;
        let mtime_ms = mtime_ms_from(&meta);
        let hash = xxh3_64(&prefix);

        // Both mtime values come from the same `as_millis() as f64`
        // computation and are integral, so exact float comparison is
        // correct here.
        let stored = {
            let map = self.records.read().expect("index lock poisoned");
            map.get(&id).map(|r| {
                let unchanged = r.content_hash == hash && r.meta.size_bytes == size_bytes;
                (unchanged, r.meta.mtime_ms)
            })
        };
        match stored {
            // Nothing moved at all — skip without rebuilding.
            Some((true, stored_mtime)) if stored_mtime == mtime_ms => return None,
            // Only the mtime moved. Refresh the record so mtime-sorted lists
            // and the Dashboard's most-recent window stay accurate, but
            // report no change so no event fires.
            Some((true, _)) => {
                let record = build_record(root, abs, parts, &meta, prefix);
                self.records
                    .write()
                    .expect("index lock poisoned")
                    .insert(id, Arc::new(record));
                return None;
            }
            // New, or the frontmatter/size genuinely changed.
            _ => {}
        }

        let record = build_record(root, abs, parts, &meta, prefix);
        self.records
            .write()
            .expect("index lock poisoned")
            .insert(id.clone(), Arc::new(record));
        Some(id)
    }

    /// Drop a document. Returns `Some(id)` if it was indexed.
    pub fn remove_path(&self, abs: &Path) -> Option<String> {
        let id = abs.to_string_lossy().into_owned();
        self.records
            .write()
            .expect("index lock poisoned")
            .remove(&id)
            .map(|_| id)
    }

    /// Drop every indexed document whose file no longer exists on disk.
    /// The watcher runs this on create/rename batches: macOS FSEvents can
    /// replay a stale `Create` flag on a rename source, which makes the
    /// debouncer swallow the rename-out event entirely — the swept file is
    /// the only trace left. Returns the removed ids.
    pub(crate) fn remove_missing(&self) -> Vec<String> {
        let ids: Vec<String> = {
            let map = self.records.read().expect("index lock poisoned");
            map.keys().cloned().collect()
        };
        // Stat outside the lock; a concurrent upsert after removal re-adds.
        let missing: Vec<String> = ids
            .into_iter()
            .filter(|id| !Path::new(id).exists())
            .collect();
        if missing.is_empty() {
            return missing;
        }
        let mut map = self.records.write().expect("index lock poisoned");
        missing
            .into_iter()
            .filter(|id| map.remove(id).is_some())
            .collect()
    }

    /// Drop every document under `prefix` (used by the watcher when a
    /// directory disappears). Returns the removed ids.
    pub(crate) fn remove_subtree(&self, prefix: &Path) -> Vec<String> {
        let mut map = self.records.write().expect("index lock poisoned");
        let ids: Vec<String> = map
            .keys()
            .filter(|id| Path::new(id.as_str()).starts_with(prefix))
            .cloned()
            .collect();
        for id in &ids {
            map.remove(id);
        }
        ids
    }

    /// Filter, search, sort, paginate. `total` counts matches before
    /// limit/offset.
    pub fn query(&self, q: &ThoughtQuery) -> QueryResult {
        let matcher = search::TextMatcher::new(search::parse_needles(q.text.as_deref()));
        let has_text = matcher.is_some();

        // Pass 1 — filter under the read lock. No I/O here, matching the
        // convention `resolve_id` and `remove_missing` already follow.
        let candidates: Vec<Arc<DocRecord>> = {
            let map = self.records.read().expect("index lock poisoned");
            map.values()
                .filter(|r| search::passes_filters(q, &r.meta))
                .map(Arc::clone)
                .collect()
        };

        // Pass 2 — body I/O and scoring, lock released. Bodies are read only
        // when the query carries text, so metadata-only queries (the
        // Dashboard graph, unfiltered library lists) still touch no files.
        let mut matched: Vec<(Arc<DocRecord>, u32)> = Vec::with_capacity(candidates.len());
        for record in candidates {
            let score = match &matcher {
                Some(m) => {
                    let Some(body) = read_body_for_search(Path::new(&record.meta.id)) else {
                        continue;
                    };
                    match search::text_score(m, &record.meta, &body) {
                        Some(score) => score,
                        None => continue,
                    }
                }
                None => 0,
            };
            matched.push((record, score));
        }

        matched.sort_by(|a, b| {
            search::compare_docs(q.sort, has_text, (&a.0.meta, a.1), (&b.0.meta, b.1))
        });

        let total = matched.len() as u32;
        let offset = q.offset.unwrap_or(0) as usize;
        let limit = q.limit.map_or(usize::MAX, |l| l as usize);
        let items = matched
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(record, _)| record.meta.clone())
            .collect();
        QueryResult { total, items }
    }

    pub fn get(&self, id: &str) -> Option<DocMeta> {
        self.records
            .read()
            .expect("index lock poisoned")
            .get(id)
            .map(|r| r.meta.clone())
    }

    pub fn doc_path(&self, id: &str) -> Option<PathBuf> {
        self.records
            .read()
            .expect("index lock poisoned")
            .get(id)
            .map(|r| PathBuf::from(&r.meta.id))
    }

    /// Resolve an externally-supplied path to the id it is indexed under.
    ///
    /// Agents write docs through a worktree's `thoughts/` symlinks, so a path
    /// captured from a `Write` tool call (e.g.
    /// `<worktree>/thoughts/shared/plans/x.md`) points *through* the link,
    /// whereas the index keys docs by their real location under the thoughts
    /// root (`<root>/repos/<mapped>/shared/plans/x.md`). Returns the literal id
    /// when it is already indexed; otherwise the canonicalized (symlink-
    /// resolved) id when *that* is indexed; otherwise `None`. The filesystem
    /// `canonicalize` only runs on a direct miss, so the common case (library
    /// ids, already canonical) stays a pure in-memory lookup.
    pub fn resolve_id(&self, id: &str) -> Option<String> {
        {
            let map = self.records.read().expect("index lock poisoned");
            if map.contains_key(id) {
                return Some(id.to_string());
            }
        }
        // Resolve symlinks outside the lock (a syscall), then re-check.
        let canonical = std::fs::canonicalize(id).ok()?;
        let canonical = canonical.to_string_lossy().into_owned();
        let map = self.records.read().expect("index lock poisoned");
        map.contains_key(&canonical).then_some(canonical)
    }

    /// Classify a path within its root: `(rel_path, project, area, scope)`.
    /// `None` means the path is not indexable (outside the root, not `.md`,
    /// or inside an ignored directory). For [`Area::Other`] the returned
    /// scope is the `"shared"` placeholder; `fm.scope` overrides it after
    /// parsing. Public for tests.
    pub fn classify(
        root: &ThoughtsRoot,
        abs: &Path,
    ) -> Option<(String, Option<String>, Area, String)> {
        let rel = abs.strip_prefix(&root.root).ok()?;
        let mut comps: Vec<&str> = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(os) => comps.push(os.to_str()?),
                // `..`/`/` inside the relative part — refuse to classify.
                _ => return None,
            }
        }
        if comps.is_empty() {
            return None;
        }
        if !abs
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            return None;
        }
        if comps.iter().any(|c| is_ignored_component(c)) {
            return None;
        }
        let rel_path = comps.join("/");

        // <repos_dir>/<mapped>/<seg>/…
        if comps.len() >= 4 && comps[0] == root.repos_dir {
            let project = Some(comps[1].to_string());
            let (area, scope) = area_for(comps[2]);
            return Some((rel_path, project, area, scope.to_string()));
        }
        // <global_dir>/<seg>/…
        if comps.len() >= 3 && comps[0] == root.global_dir {
            let (area, _) = area_for(comps[1]);
            return Some((rel_path, None, area, "global".to_string()));
        }
        Some((rel_path, None, Area::Other, "shared".to_string()))
    }
}

/// Path components that are never indexed (and never descended into).
fn is_ignored_component(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "searchable" | "_comments" | "node_modules")
}

fn area_for(seg: &str) -> (Area, &'static str) {
    if seg == "shared" {
        (Area::Shared, "shared")
    } else {
        (
            Area::User {
                name: seg.to_string(),
            },
            "user",
        )
    }
}

/// Read up to `cap` bytes from the start of a file. `None` when the file is
/// gone, unreadable, or (on Unix) a directory. Shared by the scan-time
/// frontmatter-prefix read and the query-time body read.
fn read_capped_prefix(abs: &Path, cap: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(abs)
        .ok()?
        .take(cap as u64)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// Frontmatter-stripped body bytes for search, read fresh from disk and
/// capped so one pathological file cannot stall a query (the disk-side
/// counterpart of `BODY_SEARCH_CAP`). `None` when the file is gone or
/// unreadable — a doc removed between the two query passes simply drops
/// out, consistent with `remove_missing`'s tolerance for vanished files.
fn read_body_for_search(abs: &Path) -> Option<Vec<u8>> {
    let mut buf = read_capped_prefix(abs, FRONTMATTER_READ_CAP + BODY_SEARCH_CAP)?;
    let offset = frontmatter::body_offset_bytes(&buf);
    if offset >= buf.len() {
        return Some(Vec::new());
    }
    // Shift the body to the front in place rather than reallocating.
    buf.drain(..offset);
    buf.truncate(truncate_bytes_at_char_boundary(&buf, BODY_SEARCH_CAP));
    Some(buf)
}

/// `mtime_ms` from an already-obtained `Metadata`, tolerant of platforms
/// without a modified-time (degrades to `0.0`, never fails the caller).
fn mtime_ms_from(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// Build a [`DocRecord`] from the caller's already-obtained `Metadata` and a
/// (possibly frontmatter-only) prefix of the file's bytes. `size_bytes` and
/// `mtime_ms` come from `meta` — no separate stat — and `content_hash` is
/// over `prefix`, not the whole file. `frontmatter::parse` is tolerant by
/// contract, so a prefix that cuts off before the closing `---` degrades to
/// `Frontmatter::default()` + path fallbacks + a warning, the same shape as
/// genuinely unterminated frontmatter.
fn build_record(
    root: &ThoughtsRoot,
    abs: &Path,
    (rel_path, project, area, path_scope): (String, Option<String>, Area, String),
    meta: &std::fs::Metadata,
    prefix: Vec<u8>,
) -> DocRecord {
    let size_bytes = meta.len() as f64;
    let content_hash = xxh3_64(&prefix);
    let content = String::from_utf8_lossy(&prefix);
    let parsed = frontmatter::parse(&content);
    let mut fm = parsed.fm;
    frontmatter::apply_path_fallbacks(&mut fm, abs);

    let scope = if area == Area::Other {
        fm.scope
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or(path_scope)
    } else {
        path_scope
    };

    DocRecord {
        meta: DocMeta {
            id: abs.to_string_lossy().into_owned(),
            rel_path,
            project,
            area,
            scope,
            profile: root.profile.clone(),
            backend: root.backend.as_str().to_string(),
            fm,
            fm_warnings: parsed.warnings,
            mtime_ms: mtime_ms_from(meta),
            size_bytes,
        },
        content_hash,
    }
}

/// Build a best-effort [`DocMeta`] for a markdown document that is **not** part
/// of any thoughts root — e.g. a plan Claude wrote under `~/.claude/plans/`.
///
/// Such docs are surfaced in a workstream's Changes panel (any `.md` an agent
/// `Write`s under a `plans`/`research`/`handoffs` dir) but live outside the
/// indexed library, so read-only preview consumers fall back to this to render
/// them anyway. With no root there is no repo/profile/area context: `project`
/// and `profile` are `None`, `area` is [`Area::Other`], `backend` is empty, and
/// `scope` comes from frontmatter (else `"shared"`, matching how the indexer
/// scopes [`Area::Other`] docs). Title/type/date are filled from the filename +
/// parent-dir fallbacks, exactly as for an indexed doc. `content` is the doc's
/// already-read text, so this does no body I/O (only a cheap mtime stat).
pub fn unindexed_meta(abs: &Path, content: &str) -> DocMeta {
    let parsed = frontmatter::parse(content);
    let mut fm = parsed.fm;
    frontmatter::apply_path_fallbacks(&mut fm, abs);

    let scope = fm
        .scope
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "shared".to_string());

    let rel_path = abs
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();

    let mtime_ms = std::fs::metadata(abs)
        .map(|m| mtime_ms_from(&m))
        .unwrap_or(0.0);

    DocMeta {
        id: abs.to_string_lossy().into_owned(),
        rel_path,
        project: None,
        area: Area::Other,
        scope,
        profile: None,
        backend: String::new(),
        fm,
        fm_warnings: parsed.warnings,
        mtime_ms,
        size_bytes: content.len() as f64,
    }
}

/// Caps a raw disk read to `max` bytes without splitting a UTF-8 sequence,
/// backing up while the byte at the cut is a continuation byte
/// (`b & 0xC0 == 0x80`). Returns the length to truncate to.
fn truncate_bytes_at_char_boundary(bytes: &[u8], max: usize) -> usize {
    if bytes.len() <= max {
        return bytes.len();
    }
    let mut end = max;
    while end > 0 && (bytes[end] & 0xC0) == 0x80 {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendKind;

    fn test_root() -> ThoughtsRoot {
        ThoughtsRoot {
            root: PathBuf::from("/t"),
            repos_dir: "repos".into(),
            global_dir: "global".into(),
            backend: BackendKind::Git,
            profile: None,
        }
    }

    fn classify(path: &str) -> Option<(String, Option<String>, Area, String)> {
        ThoughtsIndex::classify(&test_root(), Path::new(path))
    }

    #[test]
    fn classifies_repos_user_and_shared() {
        let (rel, project, area, scope) = classify("/t/repos/myproj/jt/plans/x.md").unwrap();
        assert_eq!(rel, "repos/myproj/jt/plans/x.md");
        assert_eq!(project.as_deref(), Some("myproj"));
        assert_eq!(area, Area::User { name: "jt".into() });
        assert_eq!(scope, "user");

        let (_, project, area, scope) = classify("/t/repos/myproj/shared/research/x.md").unwrap();
        assert_eq!(project.as_deref(), Some("myproj"));
        assert_eq!(area, Area::Shared);
        assert_eq!(scope, "shared");
    }

    #[test]
    fn classifies_global() {
        let (rel, project, area, scope) = classify("/t/global/shared/pr_description.md").unwrap();
        assert_eq!(rel, "global/shared/pr_description.md");
        assert_eq!(project, None);
        assert_eq!(area, Area::Shared);
        assert_eq!(scope, "global");

        let (_, _, area, scope) = classify("/t/global/jt/notes/x.md").unwrap();
        assert_eq!(area, Area::User { name: "jt".into() });
        assert_eq!(scope, "global");
    }

    #[test]
    fn classifies_other_for_shallow_or_unknown_layouts() {
        // Too shallow for repos layout.
        let (_, project, area, scope) = classify("/t/repos/orphan.md").unwrap();
        assert_eq!(project, None);
        assert_eq!(area, Area::Other);
        assert_eq!(scope, "shared");

        // Too shallow for global layout.
        let (_, _, area, _) = classify("/t/global/loose.md").unwrap();
        assert_eq!(area, Area::Other);

        // Stray file at the root.
        let (rel, _, area, _) = classify("/t/stray.md").unwrap();
        assert_eq!(rel, "stray.md");
        assert_eq!(area, Area::Other);
    }

    #[test]
    fn rejects_non_md_ignored_and_foreign_paths() {
        assert!(classify("/t/repos/myproj/jt/plans/notes.txt").is_none());
        assert!(classify("/t/repos/myproj/jt/.obsidian/cache.md").is_none());
        assert!(classify("/t/.git/objects/x.md").is_none());
        assert!(classify("/t/searchable/x.md").is_none());
        assert!(classify("/t/repos/myproj/_comments/x.md").is_none());
        assert!(classify("/t/node_modules/pkg/readme.md").is_none());
        assert!(classify("/t/repos/myproj/jt/plans/.draft.md").is_none());
        assert!(classify("/elsewhere/x.md").is_none());
        // The root itself is not a document.
        assert!(classify("/t").is_none());
        // Uppercase extension is still markdown.
        assert!(classify("/t/global/shared/UPPER.MD").is_some());
    }

    #[test]
    fn area_serializes_internally_tagged_camel_case() {
        let user = serde_json::to_value(Area::User { name: "jt".into() }).unwrap();
        assert_eq!(user, serde_json::json!({"kind": "user", "name": "jt"}));
        let shared = serde_json::to_value(Area::Shared).unwrap();
        assert_eq!(shared, serde_json::json!({"kind": "shared"}));
        let back: Area = serde_json::from_value(user).unwrap();
        assert_eq!(back, Area::User { name: "jt".into() });
    }

    #[test]
    fn query_dto_defaults_and_camel_case() {
        let q: ThoughtQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(q.tag_mode, TagMode::Or);
        assert_eq!(q.sort, SortKey::Date);
        assert!(q.text.is_none() && q.types.is_empty() && q.limit.is_none());

        let q: ThoughtQuery = serde_json::from_str(
            r#"{"text":"x","tagMode":"and","sort":"mtime","dateFrom":"2026-01-01","limit":5}"#,
        )
        .unwrap();
        assert_eq!(q.tag_mode, TagMode::And);
        assert_eq!(q.sort, SortKey::Mtime);
        assert_eq!(q.date_from.as_deref(), Some("2026-01-01"));
        assert_eq!(q.limit, Some(5));
    }

    #[test]
    fn doc_meta_serializes_camel_case() {
        let meta = DocMeta {
            id: "/t/a.md".into(),
            rel_path: "a.md".into(),
            project: None,
            area: Area::Other,
            scope: "shared".into(),
            profile: None,
            backend: "git".into(),
            fm: Frontmatter::default(),
            fm_warnings: vec![],
            mtime_ms: 1.0,
            size_bytes: 2.0,
        };
        let v = serde_json::to_value(&meta).unwrap();
        for key in ["relPath", "fmWarnings", "mtimeMs", "sizeBytes"] {
            assert!(v.get(key).is_some(), "missing key {key}");
        }
    }

    #[test]
    fn unindexed_meta_for_claude_plan() {
        // A plan Claude wrote under ~/.claude/plans/: no frontmatter at all, so
        // title/type fall back to the filename + parent dir, and with no root it
        // classifies as a rootless Area::Other doc. Rooted at a guaranteed-absent
        // dir so the mtime stat below deterministically degrades to 0.
        let path =
            Path::new("/hyprlayer-nonexistent-xyzzy/.claude/plans/ethereal-swinging-kurzweil.md");
        let content = "# Ethereal Swinging Kurzweil\n\nStep 1. Do the thing.\n";
        let meta = unindexed_meta(path, content);

        assert_eq!(
            meta.id,
            "/hyprlayer-nonexistent-xyzzy/.claude/plans/ethereal-swinging-kurzweil.md"
        );
        assert_eq!(meta.rel_path, "ethereal-swinging-kurzweil.md");
        assert_eq!(meta.area, Area::Other);
        assert_eq!(meta.scope, "shared");
        assert_eq!(meta.project, None);
        assert_eq!(meta.profile, None);
        assert!(meta.backend.is_empty());
        // Filename/parent-dir fallbacks fill the title and type.
        assert_eq!(meta.fm.title.as_deref(), Some("ethereal swinging kurzweil"));
        assert_eq!(meta.fm.doc_type.as_deref(), Some("plan"));
        assert_eq!(meta.size_bytes, content.len() as f64);
        // No real file at this path, so the mtime stat degrades to 0 not an error.
        assert_eq!(meta.mtime_ms, 0.0);
    }

    #[test]
    fn unindexed_meta_honors_frontmatter() {
        // Explicit frontmatter wins over the path fallbacks, and a declared
        // scope is preserved instead of the "shared" default.
        let path = Path::new("/tmp/whatever/handoffs/x.md");
        let content =
            "---\ntitle: Real Title\ntype: research\nscope: global\ntags: [a, b]\n---\n\nBody.\n";
        let meta = unindexed_meta(path, content);

        assert_eq!(meta.fm.title.as_deref(), Some("Real Title"));
        assert_eq!(meta.fm.doc_type.as_deref(), Some("research"));
        assert_eq!(meta.scope, "global");
        assert_eq!(meta.fm.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(meta.area, Area::Other);
    }

    #[test]
    fn byte_truncation_respects_char_boundaries() {
        let s = "aé"; // 'é' is 2 bytes starting at index 1
        assert_eq!(truncate_bytes_at_char_boundary(s.as_bytes(), 2), 1);
        assert_eq!(truncate_bytes_at_char_boundary(s.as_bytes(), 3), 3);
        assert_eq!(truncate_bytes_at_char_boundary(b"", 0), 0);
        assert_eq!(truncate_bytes_at_char_boundary(b"short", 100), 5);
    }
}
