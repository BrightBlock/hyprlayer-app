//! Resolves `related:` values (stems, relative/root-relative paths, titles,
//! quoted `"[[x]]"` and URLs) against the index. Lives in `index/` because it
//! only needs `DocMeta` and `ThoughtsRoot` (ADR-0008).

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{DocMeta, ThoughtsRoot};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RelatedLink {
    pub raw: String,
    pub target: RelatedTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum RelatedTarget {
    Doc {
        id: String,
        title: String,
        doc_type: Option<String>,
        rel_path: String,
    },
    External {
        url: String,
    },
    Unresolved {
        ambiguous: bool,
    },
}

/// Returns exactly one `RelatedLink` per input value, in input order. Empty
/// or whitespace-only values are `Unresolved { ambiguous: false }`.
pub fn resolve(
    source: &DocMeta,
    root: &ThoughtsRoot,
    values: &[String],
    docs: &[DocMeta],
    resolve_abs: impl Fn(&Path) -> Option<String>,
) -> Vec<RelatedLink> {
    values
        .iter()
        .map(|value| RelatedLink {
            raw: value.clone(),
            target: resolve_one(source, root, value, docs, &resolve_abs),
        })
        .collect()
}

fn resolve_one(
    source: &DocMeta,
    root: &ThoughtsRoot,
    value: &str,
    docs: &[DocMeta],
    resolve_abs: &impl Fn(&Path) -> Option<String>,
) -> RelatedTarget {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return RelatedTarget::Unresolved { ambiguous: false };
    }

    let normalised = normalise(trimmed);
    if is_url(&normalised) {
        return RelatedTarget::External {
            url: trimmed.to_string(),
        };
    }

    if looks_like_path(&normalised) {
        if let Some(target) = resolve_path_forms(source, root, &normalised, docs, resolve_abs) {
            return target;
        }
    }

    let last_segment = normalised.rsplit('/').next().unwrap_or(&normalised);
    let stem_key = strip_md_suffix(last_segment);
    if let Some(target) = resolve_by_key(stem_key, docs, source, |d| {
        Path::new(&d.id)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }) {
        return target;
    }

    if let Some(target) = resolve_by_key(&normalised, docs, source, |d| d.fm.title.clone()) {
        return target;
    }

    RelatedTarget::Unresolved { ambiguous: false }
}

/// Trim, strip `[[`/`]]`, replace `\` with `/`, and strip a leading `./`.
fn normalise(trimmed: &str) -> String {
    let stripped = trimmed
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(trimmed);
    let replaced = stripped.replace('\\', "/");
    replaced
        .strip_prefix("./")
        .map(str::to_string)
        .unwrap_or(replaced)
}

fn is_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.to_ascii_lowercase().ends_with(".md")
        || Path::new(value).is_absolute()
}

fn strip_md_suffix(s: &str) -> &str {
    if s.len() >= 3 && s[s.len() - 3..].eq_ignore_ascii_case(".md") {
        &s[..s.len() - 3]
    } else {
        s
    }
}

/// Tries, in order: the absolute path; `thoughts/global/<rest>`;
/// `thoughts/<seg>/<rest>` (only when `source.project` is `Some`); the value
/// relative to the source doc's directory; the value relative to `root.root`.
fn resolve_path_forms(
    source: &DocMeta,
    root: &ThoughtsRoot,
    value: &str,
    docs: &[DocMeta],
    resolve_abs: &impl Fn(&Path) -> Option<String>,
) -> Option<RelatedTarget> {
    let with_ext = if Path::new(value).extension().is_some() {
        value.to_string()
    } else {
        format!("{value}.md")
    };

    let mut candidates = Vec::new();
    let as_path = Path::new(&with_ext);
    if as_path.is_absolute() {
        candidates.push(as_path.to_path_buf());
    }
    if let Some(rest) = with_ext.strip_prefix("thoughts/global/") {
        candidates.push(root.root.join(&root.global_dir).join(rest));
    } else if let Some(rest) = with_ext.strip_prefix("thoughts/") {
        if let Some((seg, rest)) = rest.split_once('/') {
            if let Some(project) = &source.project {
                candidates.push(
                    root.root
                        .join(&root.repos_dir)
                        .join(project)
                        .join(seg)
                        .join(rest),
                );
            }
        }
    }
    if let Some(dir) = Path::new(&source.id).parent() {
        candidates.push(dir.join(&with_ext));
    }
    candidates.push(root.root.join(&with_ext));

    for candidate in candidates {
        if let Some(id) = resolve_abs(&candidate) {
            if let Some(d) = docs.iter().find(|d| d.id == id) {
                return Some(doc_target(d));
            }
        }
    }
    None
}

/// `None` means no hit (try the next resolution step). A single hit resolves;
/// several are narrowed to the source's own project and profile, and an
/// ambiguous narrowing stops resolution rather than falling through.
fn resolve_by_key(
    key: &str,
    docs: &[DocMeta],
    source: &DocMeta,
    key_of: impl Fn(&DocMeta) -> Option<String>,
) -> Option<RelatedTarget> {
    if key.is_empty() {
        return None;
    }
    let key_lower = key.to_lowercase();
    let hits: Vec<&DocMeta> = docs
        .iter()
        .filter(|d| key_of(d).is_some_and(|k| k.to_lowercase() == key_lower))
        .collect();
    match hits.len() {
        0 => None,
        1 => Some(doc_target(hits[0])),
        _ => {
            let narrowed: Vec<&DocMeta> = hits
                .into_iter()
                .filter(|d| d.project == source.project && d.profile == source.profile)
                .collect();
            if narrowed.len() == 1 {
                Some(doc_target(narrowed[0]))
            } else {
                Some(RelatedTarget::Unresolved { ambiguous: true })
            }
        }
    }
}

/// `d.fm.title` is already path-derived by the indexer when missing; the
/// file name is the last fallback.
fn doc_target(d: &DocMeta) -> RelatedTarget {
    let title =
        d.fm.title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                Path::new(&d.id)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
    RelatedTarget::Doc {
        id: d.id.clone(),
        title,
        doc_type: d.fm.doc_type.clone(),
        rel_path: d.rel_path.clone(),
    }
}
