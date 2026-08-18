//! Matching, scoring, and ordering helpers behind [`ThoughtsIndex::query`].
//! Trait-free and linear by design: for a small personal corpus (<= 5k docs)
//! every query full-scans (Unicode-exact lowercase title/tags per candidate,
//! an `aho-corasick` multi-needle search over a size-capped body read fresh
//! from disk at query time, no body text is cached), with no inverted index
//! or caching (< 1 ms per 1000 docs plus one bounded read per candidate with
//! text). Body matching is ASCII-case-insensitive only: a needle that is the
//! lowercase form of a *cased non-ASCII* letter (e.g. `"δ"` vs body `"Δ"`)
//! will not match in the body, though title/tag matching stays Unicode-exact.
//! Measured against the real corpus this affects a single symbolic
//! occurrence; see the plan's "Accepted divergences" for the full accounting.

use std::cmp::Ordering;

use aho_corasick::{AhoCorasick, MatchKind};

use super::indexer::{DocMeta, SortKey, TagMode, ThoughtQuery};

/// Case-insensitive multi-needle matcher, built once per query.
pub(crate) struct TextMatcher {
    needles: Vec<String>,
    ac: AhoCorasick,
}

impl TextMatcher {
    /// `None` when there is no text to search for.
    pub(crate) fn new(needles: Vec<String>) -> Option<Self> {
        if needles.is_empty() {
            return None;
        }
        let ac = AhoCorasick::builder()
            // Standard is required by `find_overlapping_iter`.
            .match_kind(MatchKind::Standard)
            .ascii_case_insensitive(true)
            .build(&needles)
            .ok()?;
        Some(Self { needles, ac })
    }

    pub(crate) fn needles(&self) -> &[String] {
        &self.needles
    }

    /// Per-needle occurrence counts, reproducing `memchr::memmem::find_iter`
    /// (leftmost, non-overlapping, *per needle*) exactly.
    ///
    /// A plain `find_iter` would be non-overlapping across *all* patterns, so
    /// needles `["ab","bc"]` on `"abc"` would lose the `bc` hit and wrongly
    /// exclude the document. `find_overlapping_iter` reports every match;
    /// `last_end` then discards only the self-overlaps memmem would have
    /// skipped (`"aa"` in `"aaa"` counts 1, not 2). Matches for a fixed-length
    /// pattern arrive in increasing end order, hence increasing start order,
    /// so the greedy test is correct.
    pub(crate) fn count_each(&self, haystack: &[u8]) -> Vec<u32> {
        let mut counts = vec![0u32; self.needles.len()];
        let mut last_end = vec![0usize; self.needles.len()];
        for m in self.ac.find_overlapping_iter(haystack) {
            let i = m.pattern().as_usize();
            if m.start() >= last_end[i] {
                counts[i] = counts[i].saturating_add(1);
                last_end[i] = m.end();
            }
        }
        counts
    }
}

/// Lowercased whitespace-split search terms; empty when there is no text.
pub(crate) fn parse_needles(text: Option<&str>) -> Vec<String> {
    text.unwrap_or("")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect()
}

/// Non-text filters: AND across fields, OR within a field. types/statuses/
/// authors/tags compare case-insensitively against frontmatter; `projects`
/// accepts either the path-derived project or `fm.project`; `scopes` matches
/// the derived scope; date bounds compare lexicographically against `fm.date`
/// (docs without a date are excluded only when a date bound is set).
pub(crate) fn passes_filters(q: &ThoughtQuery, meta: &DocMeta) -> bool {
    if !matches_any_ci(&q.types, meta.fm.doc_type.as_deref()) {
        return false;
    }
    if !matches_any_ci(&q.statuses, meta.fm.status.as_deref()) {
        return false;
    }
    if !matches_any_ci(&q.authors, meta.fm.author.as_deref()) {
        return false;
    }
    if !q.projects.is_empty() {
        let hit = q.projects.iter().any(|p| {
            eq_ci_opt(p, meta.project.as_deref()) || eq_ci_opt(p, meta.fm.project.as_deref())
        });
        if !hit {
            return false;
        }
    }
    if !q.scopes.is_empty() && !q.scopes.iter().any(|s| s.eq_ignore_ascii_case(&meta.scope)) {
        return false;
    }
    if !q.tags.is_empty() {
        let has_tag = |t: &String| meta.fm.tags.iter().any(|doc| doc.eq_ignore_ascii_case(t));
        let ok = match q.tag_mode {
            TagMode::And => q.tags.iter().all(has_tag),
            TagMode::Or => q.tags.iter().any(has_tag),
        };
        if !ok {
            return false;
        }
    }
    if q.date_from.is_some() || q.date_to.is_some() {
        let Some(date) = meta.fm.date.as_deref() else {
            return false;
        };
        if q.date_from.as_deref().is_some_and(|from| date < from) {
            return false;
        }
        if q.date_to.as_deref().is_some_and(|to| date > to) {
            return false;
        }
    }
    true
}

/// AND over needles: every needle must hit title, tags, or body. Returns the
/// summed rank (`title_hits*8 + tag_hits*4 + body_count` per needle), or
/// `None` when any needle misses everywhere. Title/tag matching stays exact
/// Unicode lowercasing; body matching goes through `matcher`, which is
/// ASCII-case-insensitive only (see the module-level divergence note).
pub(crate) fn text_score(matcher: &TextMatcher, meta: &DocMeta, body: &[u8]) -> Option<u32> {
    let title_lc = meta
        .fm
        .title
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_default();
    let tags_lc: Vec<String> = meta.fm.tags.iter().map(|t| t.to_lowercase()).collect();
    let body_hits = matcher.count_each(body);

    let mut score: u32 = 0;
    for (i, needle) in matcher.needles().iter().enumerate() {
        let title_hits = count_occurrences(&title_lc, needle);
        let tag_hits = u32::try_from(
            tags_lc
                .iter()
                .filter(|t| t.contains(needle.as_str()))
                .count(),
        )
        .unwrap_or(u32::MAX);
        let body_hits = body_hits[i];
        if title_hits == 0 && tag_hits == 0 && body_hits == 0 {
            return None;
        }
        score = score
            .saturating_add(title_hits.saturating_mul(8))
            .saturating_add(tag_hits.saturating_mul(4))
            .saturating_add(body_hits);
    }
    Some(score)
}

/// Total order for query results; `(meta, score)` pairs. Relevance without
/// text falls through to date. Final tiebreak on id keeps results stable.
pub(crate) fn compare_docs(
    sort: SortKey,
    has_text: bool,
    a: (&DocMeta, u32),
    b: (&DocMeta, u32),
) -> Ordering {
    let effective = match sort {
        SortKey::Relevance if !has_text => SortKey::Date,
        other => other,
    };
    let ordering = match effective {
        SortKey::Relevance => {
            b.1.cmp(&a.1)
                .then_with(|| date_desc_none_last(a.0, b.0))
                .then_with(|| mtime_desc(a.0, b.0))
        }
        SortKey::Date => date_desc_none_last(a.0, b.0).then_with(|| mtime_desc(a.0, b.0)),
        SortKey::Mtime => mtime_desc(a.0, b.0),
        SortKey::Title => title_asc(a.0, b.0),
    };
    ordering.then_with(|| a.0.id.cmp(&b.0.id))
}

fn count_occurrences(haystack: &str, needle: &str) -> u32 {
    if needle.is_empty() {
        return 0;
    }
    u32::try_from(memchr::memmem::find_iter(haystack.as_bytes(), needle.as_bytes()).count())
        .unwrap_or(u32::MAX)
}

/// Empty filter list always passes; otherwise the value must be present and
/// equal (ASCII case-insensitive) to one of the filters.
fn matches_any_ci(filters: &[String], value: Option<&str>) -> bool {
    if filters.is_empty() {
        return true;
    }
    match value {
        Some(v) => filters.iter().any(|f| f.eq_ignore_ascii_case(v)),
        None => false,
    }
}

fn eq_ci_opt(filter: &str, value: Option<&str>) -> bool {
    value.is_some_and(|v| filter.eq_ignore_ascii_case(v))
}

fn date_desc_none_last(a: &DocMeta, b: &DocMeta) -> Ordering {
    match (a.fm.date.as_deref(), b.fm.date.as_deref()) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn mtime_desc(a: &DocMeta, b: &DocMeta) -> Ordering {
    b.mtime_ms.total_cmp(&a.mtime_ms)
}

fn title_asc(a: &DocMeta, b: &DocMeta) -> Ordering {
    let ta = a.fm.title.as_deref().unwrap_or("").to_lowercase();
    let tb = b.fm.title.as_deref().unwrap_or("").to_lowercase();
    ta.cmp(&tb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::frontmatter::Frontmatter;
    use crate::index::indexer::Area;

    fn meta(title: &str, tags: &[&str], date: Option<&str>, mtime: f64) -> DocMeta {
        DocMeta {
            id: format!("/t/{title}.md"),
            rel_path: format!("{title}.md"),
            project: Some("myproj".into()),
            area: Area::Shared,
            scope: "shared".into(),
            profile: None,
            backend: "git".into(),
            fm: Frontmatter {
                title: Some(title.to_string()),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                date: date.map(|d| d.to_string()),
                doc_type: Some("plan".into()),
                status: Some("active".into()),
                author: Some("jt".into()),
                ..Default::default()
            },
            fm_warnings: vec![],
            mtime_ms: mtime,
            size_bytes: 0.0,
        }
    }

    fn matcher(text: &str) -> TextMatcher {
        TextMatcher::new(parse_needles(Some(text))).unwrap()
    }

    #[test]
    fn title_hit_outranks_tag_hit_outranks_body_hit() {
        let m = matcher("Auth");
        let in_title = meta("auth plan", &[], None, 0.0);
        let in_tags = meta("other", &["auth"], None, 0.0);
        let in_body = meta("other", &[], None, 0.0);

        let t = text_score(&m, &in_title, b"").unwrap();
        let g = text_score(&m, &in_tags, b"").unwrap();
        let b = text_score(&m, &in_body, b"auth is discussed here").unwrap();
        assert!(t > g && g > b, "expected {t} > {g} > {b}");
        assert_eq!(t, 8);
        assert_eq!(g, 4);
        assert_eq!(b, 1);
    }

    #[test]
    fn every_needle_must_match_somewhere() {
        let m = matcher("auth refactor");
        let doc = meta("auth plan", &[], None, 0.0);
        assert!(text_score(&m, &doc, b"no relevant words").is_none());
        assert!(text_score(&m, &doc, b"the big refactor").is_some());
    }

    #[test]
    fn overlapping_needles_both_count_and_document_is_not_excluded() {
        // A plain non-overlapping-across-all-patterns `find_iter` would lose
        // the "bc" hit here and wrongly exclude the doc.
        let m = matcher("ab bc");
        let doc = meta("other", &[], None, 0.0);
        let score = text_score(&m, &doc, b"abc").unwrap();
        // Both needles hit the body once: (0*8+0*4+1) + (0*8+0*4+1) = 2.
        assert_eq!(score, 2);
    }

    #[test]
    fn self_overlapping_needle_counts_once_like_memmem() {
        let m = matcher("aa");
        let doc = meta("other", &[], None, 0.0);
        // memmem::find_iter("aaa", "aa") yields exactly one match.
        let score = text_score(&m, &doc, b"aaa").unwrap();
        assert_eq!(score, 1);
    }

    #[test]
    fn repeated_body_occurrence_scores_the_count() {
        let m = matcher("refactor");
        let doc = meta("other", &[], None, 0.0);
        let score = text_score(&m, &doc, b"refactor refactor refactor").unwrap();
        assert_eq!(score, 3);
    }

    /// Accepted divergence 1: body matching is ASCII-case-insensitive only.
    /// A needle that is the lowercase form of a *cased non-ASCII* letter does
    /// not match its uppercase body form, while ASCII needles still match
    /// across case as before.
    #[test]
    fn body_case_folding_is_ascii_only() {
        let doc = meta("other", &[], None, 0.0);

        let m = matcher("δ");
        assert!(text_score(&m, &doc, "Δ is used as a symbol".as_bytes()).is_none());

        let m = matcher("AUTH");
        let score = text_score(&m, &doc, b"discussing auth here").unwrap();
        assert_eq!(score, 1);
    }

    #[test]
    fn date_sort_puts_missing_dates_last_and_breaks_ties_by_mtime() {
        let newer = meta("a", &[], Some("2026-06-02"), 1.0);
        let older = meta("b", &[], Some("2026-06-01"), 9.0);
        let none = meta("c", &[], None, 99.0);
        let tie = meta("d", &[], Some("2026-06-02"), 5.0);

        let cmp = |a: &DocMeta, b: &DocMeta| compare_docs(SortKey::Date, false, (a, 0), (b, 0));
        assert_eq!(cmp(&newer, &older), Ordering::Less);
        assert_eq!(cmp(&older, &none), Ordering::Less);
        assert_eq!(cmp(&none, &newer), Ordering::Greater);
        // Same date: higher mtime first.
        assert_eq!(cmp(&tie, &newer), Ordering::Less);
    }

    #[test]
    fn relevance_without_text_falls_through_to_date() {
        let dated = meta("a", &[], Some("2026-06-01"), 0.0);
        let undated = meta("b", &[], None, 100.0);
        assert_eq!(
            compare_docs(SortKey::Relevance, false, (&dated, 0), (&undated, 0)),
            Ordering::Less
        );
    }

    #[test]
    fn filters_are_case_insensitive_and_tag_modes_work() {
        let doc = meta("a", &["Security", "backend"], Some("2026-06-01"), 0.0);
        let q = |f: fn(&mut ThoughtQuery)| {
            let mut q = ThoughtQuery::default();
            f(&mut q);
            q
        };

        assert!(passes_filters(&q(|q| q.types = vec!["PLAN".into()]), &doc));
        assert!(!passes_filters(&q(|q| q.types = vec!["note".into()]), &doc));
        assert!(passes_filters(
            &q(|q| q.statuses = vec!["Active".into()]),
            &doc
        ));
        assert!(passes_filters(&q(|q| q.authors = vec!["JT".into()]), &doc));
        assert!(passes_filters(
            &q(|q| q.projects = vec!["MyProj".into()]),
            &doc
        ));
        assert!(passes_filters(
            &q(|q| q.scopes = vec!["shared".into()]),
            &doc
        ));
        assert!(!passes_filters(
            &q(|q| q.scopes = vec!["user".into()]),
            &doc
        ));

        // OR: one of two tags present suffices; AND: all must be present.
        let or_q = ThoughtQuery {
            tags: vec!["security".into(), "missing".into()],
            ..Default::default()
        };
        assert!(passes_filters(&or_q, &doc));
        let mut and_q = or_q.clone();
        and_q.tag_mode = TagMode::And;
        assert!(!passes_filters(&and_q, &doc));
        and_q.tags = vec!["security".into(), "BACKEND".into()];
        assert!(passes_filters(&and_q, &doc));
    }

    #[test]
    fn date_bounds_are_inclusive_and_exclude_undated_docs() {
        let dated = meta("a", &[], Some("2026-06-01"), 0.0);
        let undated = meta("b", &[], None, 0.0);

        let mut q = ThoughtQuery {
            date_from: Some("2026-06-01".into()),
            ..Default::default()
        };
        assert!(passes_filters(&q, &dated));
        assert!(!passes_filters(&q, &undated));

        q.date_from = Some("2026-06-02".into());
        assert!(!passes_filters(&q, &dated));

        q.date_from = None;
        q.date_to = Some("2026-06-01".into());
        assert!(passes_filters(&q, &dated));
        q.date_to = Some("2026-05-31".into());
        assert!(!passes_filters(&q, &dated));
    }

    #[test]
    fn project_filter_accepts_fm_project_too() {
        let mut doc = meta("a", &[], None, 0.0);
        doc.project = None;
        doc.fm.project = Some("from-fm".into());
        let mut q = ThoughtQuery {
            projects: vec!["from-fm".into()],
            ..Default::default()
        };
        assert!(passes_filters(&q, &doc));
        q.projects = vec!["nope".into()];
        assert!(!passes_filters(&q, &doc));
    }
}
