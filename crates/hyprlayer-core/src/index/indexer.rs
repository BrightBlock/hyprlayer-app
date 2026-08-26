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

const BODY_SEARCH_CAP: usize = 256 * 1024;
const FRONTMATTER_READ_CAP: usize = 64 * 1024;
const MAX_SCAN_DEPTH: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum Area {
    User { name: String },
    Shared,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DocMeta {
    pub id: String,
    pub rel_path: String,
    pub project: Option<String>,
    pub area: Area,
    pub scope: String,
    pub profile: Option<String>,
    pub backend: String,
    pub fm: Frontmatter,
    pub fm_warnings: Vec<String>,
    pub mtime_ms: f64,
    pub size_bytes: f64,
}

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
    Relevance,
    #[default]
    Date,
    Mtime,
    Title,
}

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

#[derive(Debug)]
struct DocRecord {
    meta: DocMeta,
    content_hash: u64,
}

#[derive(Debug, Default)]
pub struct ThoughtsIndex {
    records: RwLock<HashMap<String, Arc<DocRecord>>>,
}

impl ThoughtsIndex {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn upsert_path(&self, root: &ThoughtsRoot, abs: &Path) -> Option<String> {
        let parts = Self::classify(root, abs)?;
        let id = abs.to_string_lossy().into_owned();
        let meta = std::fs::metadata(abs).ok()?;
        let prefix = read_capped_prefix(abs, FRONTMATTER_READ_CAP)?;
        let size_bytes = meta.len() as f64;
        let mtime_ms = mtime_ms_from(&meta);
        let hash = xxh3_64(&prefix);

        let stored = {
            let map = self.records.read().expect("index lock poisoned");
            map.get(&id).map(|r| {
                let unchanged = r.content_hash == hash && r.meta.size_bytes == size_bytes;
                (unchanged, r.meta.mtime_ms)
            })
        };
        match stored {
            Some((true, stored_mtime)) if stored_mtime == mtime_ms => return None,
            Some((true, _)) => {
                let record = build_record(root, abs, parts, &meta, prefix);
                self.records
                    .write()
                    .expect("index lock poisoned")
                    .insert(id, Arc::new(record));
                return None;
            }
            _ => {}
        }

        let record = build_record(root, abs, parts, &meta, prefix);
        self.records
            .write()
            .expect("index lock poisoned")
            .insert(id.clone(), Arc::new(record));
        Some(id)
    }

    pub fn remove_path(&self, abs: &Path) -> Option<String> {
        let id = abs.to_string_lossy().into_owned();
        self.records
            .write()
            .expect("index lock poisoned")
            .remove(&id)
            .map(|_| id)
    }

    pub(crate) fn remove_missing(&self) -> Vec<String> {
        let ids: Vec<String> = {
            let map = self.records.read().expect("index lock poisoned");
            map.keys().cloned().collect()
        };
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

    pub fn query(&self, q: &ThoughtQuery) -> QueryResult {
        let matcher = search::TextMatcher::new(search::parse_needles(q.text.as_deref()));
        let has_text = matcher.is_some();

        let candidates: Vec<Arc<DocRecord>> = {
            let map = self.records.read().expect("index lock poisoned");
            map.values()
                .filter(|r| search::passes_filters(q, &r.meta))
                .map(Arc::clone)
                .collect()
        };

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

    pub fn resolve_id(&self, id: &str) -> Option<String> {
        {
            let map = self.records.read().expect("index lock poisoned");
            if map.contains_key(id) {
                return Some(id.to_string());
            }
        }
        let canonical = std::fs::canonicalize(id).ok()?;
        let canonical = canonical.to_string_lossy().into_owned();
        let map = self.records.read().expect("index lock poisoned");
        map.contains_key(&canonical).then_some(canonical)
    }

    pub fn classify(
        root: &ThoughtsRoot,
        abs: &Path,
    ) -> Option<(String, Option<String>, Area, String)> {
        let rel = abs.strip_prefix(&root.root).ok()?;
        let mut comps: Vec<&str> = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(os) => comps.push(os.to_str()?),
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

        if comps.len() >= 4 && comps[0] == root.repos_dir {
            let project = Some(comps[1].to_string());
            let (area, scope) = area_for(comps[2]);
            return Some((rel_path, project, area, scope.to_string()));
        }
        if comps.len() >= 3 && comps[0] == root.global_dir {
            let (area, _) = area_for(comps[1]);
            return Some((rel_path, None, area, "global".to_string()));
        }
        Some((rel_path, None, Area::Other, "shared".to_string()))
    }
}

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

fn read_body_for_search(abs: &Path) -> Option<Vec<u8>> {
    let mut buf = read_capped_prefix(abs, FRONTMATTER_READ_CAP + BODY_SEARCH_CAP)?;
    let offset = frontmatter::body_offset_bytes(&buf);
    if offset >= buf.len() {
        return Some(Vec::new());
    }
    buf.drain(..offset);
    buf.truncate(truncate_bytes_at_char_boundary(&buf, BODY_SEARCH_CAP));
    Some(buf)
}

fn mtime_ms_from(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_millis() as f64)
}

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
        let (_, project, area, scope) = classify("/t/repos/orphan.md").unwrap();
        assert_eq!(project, None);
        assert_eq!(area, Area::Other);
        assert_eq!(scope, "shared");

        let (_, _, area, _) = classify("/t/global/loose.md").unwrap();
        assert_eq!(area, Area::Other);

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
        assert!(classify("/t").is_none());
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
        assert_eq!(meta.fm.title.as_deref(), Some("ethereal swinging kurzweil"));
        assert_eq!(meta.fm.doc_type.as_deref(), Some("plan"));
        assert_eq!(meta.size_bytes, content.len() as f64);
        assert_eq!(meta.mtime_ms, 0.0);
    }

    #[test]
    fn unindexed_meta_honors_frontmatter() {
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
        let s = "aé";
        assert_eq!(truncate_bytes_at_char_boundary(s.as_bytes(), 2), 1);
        assert_eq!(truncate_bytes_at_char_boundary(s.as_bytes(), 3), 3);
        assert_eq!(truncate_bytes_at_char_boundary(b"", 0), 0);
        assert_eq!(truncate_bytes_at_char_boundary(b"short", 100), 5);
    }
}
