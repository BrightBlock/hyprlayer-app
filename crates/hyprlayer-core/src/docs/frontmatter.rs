//! Tolerant frontmatter parsing and line-surgical patching. Parsing never fails —
//! anomalies (no/malformed YAML, scalar-vs-list confusion) degrade to warnings with
//! filename/directory fallbacks. Public API is frozen: indexer + Tauri shell build on it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The 10-field thought schema (hyprlayer-cli/src/backends/schema.rs), all
/// optional because the parser must tolerate anything.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct Frontmatter {
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub doc_type: Option<String>,
    pub date: Option<String>,
    pub status: Option<String>,
    pub ticket: Option<String>,
    pub project: Option<String>,
    pub scope: Option<String>,
    pub tags: Vec<String>,
    pub author: Option<String>,
    pub related: Vec<String>,
    /// Unknown keys, preserved for display and round-tripping.
    /// Unknown keys in display form: strings as-is, everything else compact
    /// JSON, null as "". Read-only context for the UI — the line-surgical
    /// patcher preserves the underlying bytes regardless. (Stringified
    /// because specta forbids serde_json::Value's i64 arm in IPC types.)
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedDoc {
    pub fm: Frontmatter,
    pub warnings: Vec<String>,
    /// Byte offset where the body starts (0 when there is no frontmatter).
    pub body_offset: usize,
}

/// Parse a document's frontmatter without ever failing.
pub fn parse(content: &str) -> ParsedDoc {
    let Some((block, body_offset)) = extract_block(content) else {
        return ParsedDoc {
            fm: Frontmatter::default(),
            warnings: vec!["no frontmatter block".into()],
            body_offset: 0,
        };
    };

    match serde_yaml::from_str::<serde_yaml::Value>(block) {
        Ok(value) => {
            let mut warnings = Vec::new();
            let fm = from_yaml_value(&value, &mut warnings);
            ParsedDoc {
                fm,
                warnings,
                body_offset,
            }
        }
        Err(e) => {
            // serde_yaml rejects whole blocks for recoverable anomalies
            // (duplicate keys, one bad line). Fall back to a loose line scan
            // so the doc still indexes with its scalar fields intact.
            let mut warnings = vec![format!("frontmatter YAML error: {e}")];
            let fm = loose_recover(block, &mut warnings);
            ParsedDoc {
                fm,
                warnings,
                body_offset,
            }
        }
    }
}

/// Loose line-based recovery for blocks serde_yaml refuses to parse.
///
/// Recognizes top-level `key: scalar` pairs and simple `key:` + indented
/// `- item` block lists. Duplicate keys are last-wins; every anomaly
/// (duplicate, unrecoverable line) emits a warning.
fn loose_recover(block: &str, warnings: &mut Vec<String>) -> Frontmatter {
    let mut entries: Vec<(String, serde_yaml::Value)> = Vec::new();
    // Index into `entries` of the key currently accepting list items.
    let mut current: Option<usize> = None;

    for raw in block.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indented = line.starts_with(' ') || line.starts_with('\t');
        let list_item = trimmed
            .strip_prefix("- ")
            .map(str::trim)
            .or_else(|| (trimmed == "-").then_some(""));

        if indented || list_item.is_some() {
            if let (Some(item), Some(idx)) = (list_item, current) {
                let slot = &mut entries[idx].1;
                if slot.is_null() {
                    *slot = serde_yaml::Value::Sequence(Vec::new());
                }
                if let serde_yaml::Value::Sequence(seq) = slot {
                    seq.push(loose_scalar(item));
                    continue;
                }
            }
            warnings.push(format!("frontmatter recovery skipped line: {trimmed}"));
            continue;
        }

        let key_colon = find_key_colon(line);
        let key = key_colon.map(|i| unquote_key(line[..i].trim()));
        match (key_colon, key) {
            (Some(colon), Some(key)) if !key.is_empty() => {
                let rest = line[colon + 1..].trim();
                let value = if rest.is_empty() {
                    serde_yaml::Value::Null
                } else {
                    loose_scalar(rest)
                };
                if let Some(idx) = entries.iter().position(|(k, _)| *k == key) {
                    warnings.push(format!(
                        "duplicate frontmatter key `{key}`: last value wins"
                    ));
                    entries[idx].1 = value;
                    current = Some(idx);
                } else {
                    entries.push((key, value));
                    current = Some(entries.len() - 1);
                }
            }
            _ => {
                current = None;
                warnings.push(format!("frontmatter recovery skipped line: {trimmed}"));
            }
        }
    }

    let mut map = serde_yaml::Mapping::new();
    for (k, v) in entries {
        map.insert(serde_yaml::Value::String(k), v);
    }
    from_yaml_value(&serde_yaml::Value::Mapping(map), warnings)
}

/// Parse a single scalar (or flow list) with serde_yaml so quoting/comments
/// behave correctly; anything weirder degrades to the raw string.
fn loose_scalar(s: &str) -> serde_yaml::Value {
    match serde_yaml::from_str::<serde_yaml::Value>(s) {
        Ok(v)
            if !matches!(
                v,
                serde_yaml::Value::Mapping(_) | serde_yaml::Value::Tagged(_)
            ) =>
        {
            v
        }
        _ => serde_yaml::Value::String(s.to_string()),
    }
}

/// Byte index of the `:` separating a top-level key from its value — the
/// first colon followed by whitespace or end-of-line (YAML mapping syntax).
fn find_key_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    (0..bytes.len()).find(|&i| {
        bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
    })
}

fn unquote_key(key: &str) -> String {
    let stripped = key
        .strip_prefix('"')
        .and_then(|k| k.strip_suffix('"'))
        .or_else(|| key.strip_prefix('\'').and_then(|k| k.strip_suffix('\'')));
    stripped.unwrap_or(key).to_string()
}

/// Locate the frontmatter block: `---` on the first line, closed by a line
/// that is exactly `---` or `...`. Returns (block_str, body_offset).
pub fn extract_block(content: &str) -> Option<(&str, usize)> {
    let after_open = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))?;
    let open_len = content.len() - after_open.len();

    let mut offset = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            let block = &after_open[..offset];
            let body_offset = open_len + offset + line.len();
            return Some((block, body_offset));
        }
        offset += line.len();
    }
    None
}

fn yaml_scalar_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Accept both `tags: a` and `tags: [a, b]`.
fn yaml_string_or_seq(v: &serde_yaml::Value, key: &str, warnings: &mut Vec<String>) -> Vec<String> {
    match v {
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|item| {
                let s = yaml_scalar_to_string(item);
                if s.is_none() {
                    warnings.push(format!("{key}: ignored non-scalar list item"));
                }
                s
            })
            .collect(),
        other => match yaml_scalar_to_string(other) {
            Some(s) if !s.is_empty() => vec![s],
            _ => {
                warnings.push(format!("{key}: expected string or list"));
                Vec::new()
            }
        },
    }
}

fn from_yaml_value(value: &serde_yaml::Value, warnings: &mut Vec<String>) -> Frontmatter {
    let serde_yaml::Value::Mapping(map) = value else {
        if !value.is_null() {
            warnings.push("frontmatter is not a mapping".into());
        }
        return Frontmatter::default();
    };

    let mut fm = Frontmatter::default();
    for (k, v) in map {
        let Some(key) = k.as_str() else {
            warnings.push("ignored non-string frontmatter key".into());
            continue;
        };
        match key {
            "title" => fm.title = yaml_scalar_to_string(v),
            "type" => fm.doc_type = yaml_scalar_to_string(v),
            "date" => fm.date = yaml_scalar_to_string(v),
            "status" => fm.status = yaml_scalar_to_string(v),
            "ticket" => fm.ticket = yaml_scalar_to_string(v),
            "project" => fm.project = yaml_scalar_to_string(v),
            "scope" => fm.scope = yaml_scalar_to_string(v),
            "author" => fm.author = yaml_scalar_to_string(v),
            "tags" => fm.tags = yaml_string_or_seq(v, "tags", warnings),
            "related" => fm.related = yaml_string_or_seq(v, "related", warnings),
            other => {
                fm.extra.insert(other.to_string(), yaml_value_display(v));
            }
        }
    }
    fm
}

/// Display form for unknown-key values (see [`Frontmatter::extra`]).
fn yaml_value_display(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Null => String::new(),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|j| serde_json::to_string(&j).ok())
            .unwrap_or_default(),
    }
}

/// Fill title/date/type gaps from the file's name and parent directory,
/// matching the CLI's layout conventions
/// (`<plans|research|handoffs|prs|notes>/YYYY-MM-DD[-TICKET]-slug.md`).
pub fn apply_path_fallbacks(fm: &mut Frontmatter, abs_path: &Path) {
    let stem = abs_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();

    if fm.date.is_none() {
        if let Some(date) = leading_date(stem) {
            fm.date = Some(date.to_string());
        }
    }

    if fm.title.is_none() && !stem.is_empty() {
        let mut rest = stem;
        if let Some(date) = leading_date(stem) {
            rest = stem[date.len()..].trim_start_matches('-');
        }
        let title = rest.replace(['-', '_'], " ").trim().to_string();
        fm.title = Some(if title.is_empty() {
            stem.to_string()
        } else {
            title
        });
    }

    if fm.doc_type.is_none() {
        let parent = abs_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        fm.doc_type = match parent {
            "plans" => Some("plan".into()),
            "research" => Some("research".into()),
            "handoffs" => Some("handoff".into()),
            "prs" => Some("pr".into()),
            "notes" => Some("note".into()),
            _ => None,
        };
    }
}

/// `YYYY-MM-DD` prefix of a file stem, if present.
fn leading_date(stem: &str) -> Option<&str> {
    let bytes = stem.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let ok = bytes[..10].iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        _ => b.is_ascii_digit(),
    });
    if ok {
        Some(&stem[..10])
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Line-surgical frontmatter patching
//
// Agents and humans hand-edit thought files. A patch must therefore touch
// ONLY the lines of the keys it sets/removes: every other byte of the file —
// comments, quoting style, key order, unknown keys, EOL convention, the
// entire body — survives verbatim.
// ---------------------------------------------------------------------------

/// Schema key order (hyprlayer-cli/src/backends/schema.rs THOUGHT_SCHEMA).
const SCHEMA_KEYS: [&str; 10] = [
    "title", "type", "date", "status", "ticket", "project", "scope", "tags", "author", "related",
];

fn schema_pos(key: &str) -> Option<usize> {
    SCHEMA_KEYS.iter().position(|k| *k == key)
}

/// A value to write into frontmatter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum FmValue {
    Str(String),
    List(Vec<String>),
    Null,
}

/// A set of frontmatter edits applied in one surgical pass.
/// `remove` is applied first, then `set` (in order; later sets of the same
/// key win).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct FmPatch {
    pub set: Vec<(String, FmValue)>,
    pub remove: Vec<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum FmError {
    #[error("invalid frontmatter key `{0}` (keys must be non-empty, without whitespace, `:`, `#`, or quotes)")]
    InvalidKey(String),
    #[error("failed to render YAML value for key `{key}`: {source}")]
    Render {
        key: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("patched frontmatter failed validation: {0}")]
    Validation(String),
}

/// Apply `patch` to `original`, returning the new file contents.
///
/// Guarantees:
/// - body bytes after the closing `---` are never modified;
/// - untouched frontmatter spans keep their exact bytes;
/// - if there is no frontmatter block and the patch sets keys, a new block is
///   synthesized at byte 0 (keys in schema order) with the body preserved;
/// - the original EOL convention and trailing-newline presence are preserved;
/// - the patched result is re-parsed and every set key must read back, or an
///   error is returned (garbage is never produced).
pub fn patch_frontmatter(original: &str, patch: &FmPatch) -> Result<String, FmError> {
    for (key, _) in &patch.set {
        validate_key(key)?;
    }
    let eol = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let result = match extract_block(original) {
        None => {
            if patch.set.is_empty() {
                original.to_string()
            } else {
                synthesize_block(original, &patch.set, eol)?
            }
        }
        Some((block, body_offset)) => patch_block(original, block, body_offset, patch, eol)?,
    };

    validate_patch(&result, patch)?;
    Ok(result)
}

fn validate_key(key: &str) -> Result<(), FmError> {
    let ok = !key.is_empty()
        && !key.contains(|c: char| c.is_whitespace() || matches!(c, ':' | '#' | '\'' | '"'))
        && !key.starts_with('-');
    if ok {
        Ok(())
    } else {
        Err(FmError::InvalidKey(key.to_string()))
    }
}

/// A top-level key span inside the block: optional lead (comments/blank lines
/// attached to this span) + the `key:` line and its continuation lines.
struct Span {
    key: String,
    lead_start: usize,
    key_start: usize,
    end: usize,
}

/// Tokenize the block into spans. Returns (spans, tail_start) where
/// `block[tail_start..]` is trailing comments/blank lines owned by no span.
fn tokenize_block(block: &str) -> (Vec<Span>, usize) {
    let mut spans: Vec<Span> = Vec::new();
    let mut pending_start: Option<usize> = None;
    let mut offset = 0usize;

    for line in block.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let no_eol = line.trim_end_matches(['\r', '\n']);
        let trimmed = no_eol.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            pending_start.get_or_insert(start);
            continue;
        }

        let indented = no_eol.starts_with(' ') || no_eol.starts_with('\t');
        let list_item = trimmed.starts_with("- ") || trimmed == "-";

        if !indented && !list_item {
            if let Some(colon) = find_key_colon(no_eol) {
                let key = unquote_key(no_eol[..colon].trim());
                if !key.is_empty() {
                    spans.push(Span {
                        key,
                        lead_start: pending_start.take().unwrap_or(start),
                        key_start: start,
                        end: offset,
                    });
                    continue;
                }
            }
        }

        // Continuation (or stray) line: absorb it — and any blank/comment
        // lines between it and the current span — into the current span.
        if let Some(last) = spans.last_mut() {
            pending_start = None;
            last.end = offset;
        } else {
            pending_start.get_or_insert(start);
        }
    }

    let tail_start = pending_start.unwrap_or(block.len());
    (spans, tail_start)
}

/// Working item while rebuilding the block.
struct WorkItem<'a> {
    key: String,
    lead: &'a str,
    body: WorkBody<'a>,
}

enum WorkBody<'a> {
    Original(&'a str),
    Rendered(String),
}

fn patch_block(
    original: &str,
    block: &str,
    body_offset: usize,
    patch: &FmPatch,
    eol: &str,
) -> Result<String, FmError> {
    // extract_block guarantees the opening delimiter is one of these.
    let open_len = if original.starts_with("---\r\n") {
        5
    } else {
        4
    };
    let opening = &original[..open_len];
    let closing = &original[open_len + block.len()..body_offset];
    let body = &original[body_offset..];

    let (spans, tail_start) = tokenize_block(block);
    let tail = &block[tail_start..];

    let mut items: Vec<WorkItem<'_>> = spans
        .into_iter()
        .map(|s| WorkItem {
            lead: &block[s.lead_start..s.key_start],
            body: WorkBody::Original(&block[s.key_start..s.end]),
            key: s.key,
        })
        .collect();

    // Removes first: delete every span with the key, including its lead.
    items.retain(|item| !patch.remove.contains(&item.key));

    // Sets: replace the LAST span with the key (last-wins on read), or insert
    // a new span ordered by schema position.
    for (key, value) in &patch.set {
        let rendered = render_span(key, value, eol)?;
        if let Some(item) = items.iter_mut().rev().find(|item| item.key == *key) {
            item.body = WorkBody::Rendered(rendered);
            continue;
        }
        let idx = insertion_index(&items, key);
        items.insert(
            idx,
            WorkItem {
                key: key.clone(),
                lead: "",
                body: WorkBody::Rendered(rendered),
            },
        );
    }

    let mut out = String::with_capacity(original.len() + 64);
    out.push_str(opening);
    for item in &items {
        out.push_str(item.lead);
        match &item.body {
            WorkBody::Original(s) => out.push_str(s),
            WorkBody::Rendered(s) => out.push_str(s),
        }
    }
    out.push_str(tail);
    out.push_str(closing);
    out.push_str(body);
    Ok(out)
}

/// Where to insert a new key: schema keys slot in relative to whichever
/// schema keys already exist; unknown keys go last (before the closing ---).
fn insertion_index(items: &[WorkItem<'_>], key: &str) -> usize {
    let Some(pos) = schema_pos(key) else {
        return items.len();
    };
    // Before the first schema key that should come after it...
    if let Some(idx) = items
        .iter()
        .position(|item| schema_pos(&item.key).is_some_and(|q| q > pos))
    {
        return idx;
    }
    // ...else after the last schema key that comes before it...
    if let Some(idx) = items
        .iter()
        .rposition(|item| schema_pos(&item.key).is_some_and(|q| q <= pos))
    {
        return idx + 1;
    }
    // ...else (no schema keys at all) first, ahead of unknown keys.
    0
}

/// Render one `key: value` span, terminated with `eol`.
fn render_span(key: &str, value: &FmValue, eol: &str) -> Result<String, FmError> {
    Ok(match value {
        FmValue::Null => format!("{key}:{eol}"),
        FmValue::Str(s) => format!("{key}: {}{eol}", render_scalar(key, s, eol)?),
        FmValue::List(items) if items.is_empty() => format!("{key}: []{eol}"),
        FmValue::List(items) => {
            let mut out = format!("{key}:{eol}");
            for item in items {
                out.push_str(&format!("  - {}{eol}", render_scalar(key, item, eol)?));
            }
            out
        }
    })
}

/// Render a scalar through serde_yaml so edge cases (`:`, `#`, quotes,
/// leading/trailing spaces, `yes`/`no`, numbers) get correct quoting.
fn render_scalar(key: &str, s: &str, eol: &str) -> Result<String, FmError> {
    let rendered = serde_yaml::to_string(s).map_err(|source| FmError::Render {
        key: key.to_string(),
        source,
    })?;
    let rendered = rendered.trim_end_matches('\n');
    // Multi-line scalars render as block scalars; keep the file's EOLs.
    Ok(if eol == "\n" {
        rendered.to_string()
    } else {
        rendered.replace('\n', eol)
    })
}

/// Build a brand-new block (no existing frontmatter), keys in schema order,
/// body preserved verbatim.
fn synthesize_block(
    original: &str,
    set: &[(String, FmValue)],
    eol: &str,
) -> Result<String, FmError> {
    // Last-wins de-dup, preserving first-seen position for unknown keys.
    let mut entries: Vec<(&str, &FmValue)> = Vec::new();
    for (key, value) in set {
        if let Some(entry) = entries.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value;
        } else {
            entries.push((key, value));
        }
    }
    // Stable sort: schema keys in schema order, unknown keys after.
    entries.sort_by_key(|(k, _)| schema_pos(k).unwrap_or(usize::MAX));

    let mut out = format!("---{eol}");
    for (key, value) in entries {
        out.push_str(&render_span(key, value, eol)?);
    }
    out.push_str("---");
    out.push_str(eol);
    out.push_str(original);
    Ok(out)
}

/// Re-parse the patched result and confirm every set key reads back exactly.
fn validate_patch(result: &str, patch: &FmPatch) -> Result<(), FmError> {
    if patch.set.is_empty() {
        return Ok(());
    }
    let parsed = parse(result);
    let mut last: BTreeMap<&str, &FmValue> = BTreeMap::new();
    for (key, value) in &patch.set {
        last.insert(key.as_str(), value);
    }
    for (key, expected) in last {
        let ok = match key {
            "title" => scalar_matches(&parsed.fm.title, expected),
            "type" => scalar_matches(&parsed.fm.doc_type, expected),
            "date" => scalar_matches(&parsed.fm.date, expected),
            "status" => scalar_matches(&parsed.fm.status, expected),
            "ticket" => scalar_matches(&parsed.fm.ticket, expected),
            "project" => scalar_matches(&parsed.fm.project, expected),
            "scope" => scalar_matches(&parsed.fm.scope, expected),
            "author" => scalar_matches(&parsed.fm.author, expected),
            "tags" => list_matches(&parsed.fm.tags, expected),
            "related" => list_matches(&parsed.fm.related, expected),
            other => extra_matches(parsed.fm.extra.get(other), expected),
        };
        if !ok {
            return Err(FmError::Validation(format!(
                "key `{key}` did not read back after patching"
            )));
        }
    }
    Ok(())
}

fn scalar_matches(actual: &Option<String>, expected: &FmValue) -> bool {
    match expected {
        FmValue::Str(s) => actual.as_deref() == Some(s.as_str()),
        FmValue::Null => actual.is_none(),
        FmValue::List(_) => false, // scalar schema fields cannot hold lists
    }
}

fn list_matches(actual: &[String], expected: &FmValue) -> bool {
    match expected {
        FmValue::List(items) => actual == items.as_slice(),
        FmValue::Str(s) => actual.len() == 1 && actual[0] == *s,
        FmValue::Null => actual.is_empty(),
    }
}

fn extra_matches(actual: Option<&String>, expected: &FmValue) -> bool {
    match expected {
        FmValue::Str(s) => actual.is_some_and(|v| v == s),
        // Lists display as compact JSON arrays (yaml_value_display).
        FmValue::List(items) => {
            let expected_json = serde_json::to_string(items).unwrap_or_default();
            actual.is_some_and(|v| *v == expected_json)
        }
        FmValue::Null => actual.is_none_or(|v| v.is_empty()),
    }
}

#[cfg(test)]
mod stub_tests {
    use super::*;

    #[test]
    fn parses_typical_frontmatter() {
        let doc = "---\ntitle: \"Auth Refactor\"\ntype: plan\ndate: 2026-06-01\nstatus: active\ntags: [security, backend]\nauthor: jt\ncustomKey: 42\n---\n\n## Overview\n";
        let parsed = parse(doc);
        assert_eq!(parsed.fm.title.as_deref(), Some("Auth Refactor"));
        assert_eq!(parsed.fm.doc_type.as_deref(), Some("plan"));
        assert_eq!(parsed.fm.date.as_deref(), Some("2026-06-01"));
        assert_eq!(parsed.fm.tags, vec!["security", "backend"]);
        assert!(parsed.fm.extra.contains_key("customKey"));
        assert!(parsed.warnings.is_empty());
        assert_eq!(&doc[parsed.body_offset..], "\n## Overview\n");
    }

    #[test]
    fn tolerates_missing_and_malformed_frontmatter() {
        let no_fm = parse("just a body\n");
        assert_eq!(no_fm.body_offset, 0);
        assert!(no_fm.fm.title.is_none());
        assert_eq!(no_fm.warnings.len(), 1);

        let bad = parse("---\n: : : not yaml [\n---\nbody\n");
        assert!(!bad.warnings.is_empty());
        assert_eq!(
            &"---\n: : : not yaml [\n---\nbody\n"[bad.body_offset..],
            "body\n"
        );
    }

    #[test]
    fn scalar_tags_and_path_fallbacks() {
        let parsed = parse("---\ntags: solo\n---\nx");
        assert_eq!(parsed.fm.tags, vec!["solo"]);

        let mut fm = Frontmatter::default();
        apply_path_fallbacks(
            &mut fm,
            Path::new("/t/repos/myproj/shared/plans/2026-06-01-AUTH-12-refactor.md"),
        );
        assert_eq!(fm.date.as_deref(), Some("2026-06-01"));
        assert_eq!(fm.doc_type.as_deref(), Some("plan"));
        assert_eq!(fm.title.as_deref(), Some("AUTH 12 refactor"));
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn duplicate_keys_recover_with_last_value_winning() {
        let doc = "---\ntitle: First\nstatus: draft\ntitle: Second\ndate: 2026-06-12\n---\nbody\n";
        let parsed = parse(doc);
        assert_eq!(parsed.fm.title.as_deref(), Some("Second"));
        assert_eq!(parsed.fm.status.as_deref(), Some("draft"));
        assert_eq!(parsed.fm.date.as_deref(), Some("2026-06-12"));
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| w.contains("duplicate") && w.contains("title")),
            "{:?}",
            parsed.warnings
        );
        assert_eq!(&doc[parsed.body_offset..], "body\n");
    }

    #[test]
    fn recovery_handles_block_lists_comments_and_quoting() {
        let doc = "---\ntitle: \"Quoted: title\"\ntitle: dup forces recovery\n# a comment\ntags:\n  - alpha\n  - 'beta'\nstatus: active # trailing comment\n---\n";
        let parsed = parse(doc);
        assert_eq!(parsed.fm.title.as_deref(), Some("dup forces recovery"));
        assert_eq!(parsed.fm.tags, vec!["alpha", "beta"]);
        assert_eq!(parsed.fm.status.as_deref(), Some("active"));
    }

    #[test]
    fn recovery_handles_flow_lists_and_skips_junk_with_warnings() {
        let doc =
            "---\ndate: 2026-01-01\ndate: 2026-02-02\ntags: [a, b]\nthis line is junk\n---\nx";
        let parsed = parse(doc);
        assert_eq!(parsed.fm.date.as_deref(), Some("2026-02-02"));
        assert_eq!(parsed.fm.tags, vec!["a", "b"]);
        assert!(
            parsed.warnings.iter().any(|w| w.contains("skipped line")),
            "{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn recovery_keeps_unknown_keys_in_extra() {
        let doc = "---\ntitle: a\ntitle: b\ncustomKey: 42\n---\n";
        let parsed = parse(doc);
        assert_eq!(parsed.fm.extra.get("customKey"), Some(&"42".to_string()));
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    const COMMENTED_DOC: &str = "---\n# owner: platform team\ntitle: 'Auth: refactor'\ntype: plan\n# status is reviewed weekly\nstatus: draft\ncustomKey: \"keep me\"\ntags:\n  - security\n  - backend\n---\n\nBody stays --- untouched.\n";

    #[test]
    fn noop_patch_is_byte_identical() {
        let patch = FmPatch::default();
        assert_eq!(
            patch_frontmatter(COMMENTED_DOC, &patch).unwrap(),
            COMMENTED_DOC
        );

        let no_fm = "just a body\nwith lines\n";
        assert_eq!(patch_frontmatter(no_fm, &patch).unwrap(), no_fm);
    }

    #[test]
    fn replacing_one_key_keeps_every_other_byte() {
        let patch = FmPatch {
            set: vec![("status".into(), FmValue::Str("active".into()))],
            remove: vec![],
        };
        let out = patch_frontmatter(COMMENTED_DOC, &patch).unwrap();
        assert_eq!(
            out,
            "---\n# owner: platform team\ntitle: 'Auth: refactor'\ntype: plan\n# status is reviewed weekly\nstatus: active\ncustomKey: \"keep me\"\ntags:\n  - security\n  - backend\n---\n\nBody stays --- untouched.\n"
        );
    }

    #[test]
    fn replace_keeps_lead_comment_and_replaces_whole_span() {
        let patch = FmPatch {
            set: vec![("tags".into(), FmValue::List(vec!["frontend".into()]))],
            remove: vec![],
        };
        let out = patch_frontmatter(COMMENTED_DOC, &patch).unwrap();
        assert!(
            out.contains("customKey: \"keep me\"\ntags:\n  - frontend\n---\n"),
            "{out}"
        );
        assert!(!out.contains("security"));
    }

    #[test]
    fn insertion_follows_schema_order() {
        let doc = "---\ntitle: x\nstatus: draft\ncustomKey: 1\n---\nbody";
        let patch = FmPatch {
            set: vec![
                ("date".into(), FmValue::Str("2026-06-12".into())),
                ("related".into(), FmValue::List(vec!["other.md".into()])),
                ("brandNew".into(), FmValue::Str("v".into())),
            ],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(
            out,
            "---\ntitle: x\ndate: 2026-06-12\nstatus: draft\nrelated:\n  - other.md\ncustomKey: 1\nbrandNew: v\n---\nbody"
        );
    }

    #[test]
    fn insertion_with_no_schema_keys_goes_first() {
        let doc = "---\ncustomKey: 1\n---\n";
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::Str("t".into()))],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(out, "---\ntitle: t\ncustomKey: 1\n---\n");
    }

    #[test]
    fn removal_deletes_span_and_attached_comments() {
        let patch = FmPatch {
            set: vec![],
            remove: vec!["status".into(), "tags".into()],
        };
        let out = patch_frontmatter(COMMENTED_DOC, &patch).unwrap();
        assert_eq!(
            out,
            "---\n# owner: platform team\ntitle: 'Auth: refactor'\ntype: plan\ncustomKey: \"keep me\"\n---\n\nBody stays --- untouched.\n"
        );
    }

    #[test]
    fn synthesizes_block_when_missing() {
        let body = "# Heading\n\nNo frontmatter here.";
        let patch = FmPatch {
            set: vec![
                ("zCustom".into(), FmValue::Str("v".into())),
                ("date".into(), FmValue::Str("2026-06-12".into())),
                ("title".into(), FmValue::Str("New".into())),
            ],
            remove: vec!["whatever".into()],
        };
        let out = patch_frontmatter(body, &patch).unwrap();
        // Schema order, unknown keys last, body byte-identical (no trailing
        // newline added).
        assert_eq!(
            out,
            "---\ntitle: New\ndate: 2026-06-12\nzCustom: v\n---\n# Heading\n\nNo frontmatter here."
        );
    }

    #[test]
    fn special_char_scalars_round_trip() {
        let cases = [
            "has: colon",
            "#leading hash",
            " leading space",
            "trailing space ",
            "'single' and \"double\"",
            "yes",
            "42",
            "multi\nline value",
            "",
        ];
        for value in cases {
            let patch = FmPatch {
                set: vec![("title".into(), FmValue::Str(value.into()))],
                remove: vec![],
            };
            let out = patch_frontmatter("---\ntype: note\n---\nbody", &patch).unwrap();
            let parsed = parse(&out);
            assert_eq!(
                parsed.fm.title.as_deref(),
                Some(value),
                "case {value:?} -> {out:?}"
            );
            assert_eq!(parsed.fm.doc_type.as_deref(), Some("note"));
            assert!(out.ends_with("---\nbody"), "body modified: {out:?}");
        }
    }

    #[test]
    fn list_and_null_rendering() {
        let patch = FmPatch {
            set: vec![
                (
                    "tags".into(),
                    FmValue::List(vec!["a: b".into(), "c".into()]),
                ),
                ("related".into(), FmValue::List(vec![])),
                ("ticket".into(), FmValue::Null),
            ],
            remove: vec![],
        };
        let out = patch_frontmatter("---\ntitle: x\n---\n", &patch).unwrap();
        assert_eq!(
            out,
            "---\ntitle: x\nticket:\ntags:\n  - 'a: b'\n  - c\nrelated: []\n---\n"
        );
        let parsed = parse(&out);
        assert_eq!(parsed.fm.tags, vec!["a: b", "c"]);
        assert!(parsed.fm.related.is_empty());
        assert!(parsed.fm.ticket.is_none());
    }

    #[test]
    fn crlf_files_keep_crlf_and_untouched_bytes() {
        let doc = "---\r\ntitle: old\r\ncustomKey: stays\r\n---\r\nbody line\r\n";
        let patch = FmPatch {
            set: vec![
                ("title".into(), FmValue::Str("new".into())),
                ("date".into(), FmValue::Str("2026-06-12".into())),
            ],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(
            out,
            "---\r\ntitle: new\r\ndate: 2026-06-12\r\ncustomKey: stays\r\n---\r\nbody line\r\n"
        );
    }

    #[test]
    fn crlf_synthesis_uses_crlf() {
        let body = "body\r\nmore\r\n";
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::Str("t".into()))],
            remove: vec![],
        };
        let out = patch_frontmatter(body, &patch).unwrap();
        assert_eq!(out, "---\r\ntitle: t\r\n---\r\nbody\r\nmore\r\n");
    }

    #[test]
    fn missing_trailing_newline_is_preserved() {
        let doc = "---\ntitle: x\n---";
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::Str("y".into()))],
            remove: vec![],
        };
        assert_eq!(
            patch_frontmatter(doc, &patch).unwrap(),
            "---\ntitle: y\n---"
        );
    }

    #[test]
    fn duplicate_key_docs_patch_the_winning_span() {
        let doc = "---\ntitle: first\ntitle: second\n---\nbody";
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::Str("patched".into()))],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(out, "---\ntitle: first\ntitle: patched\n---\nbody");
        assert_eq!(parse(&out).fm.title.as_deref(), Some("patched"));

        // remove deletes ALL spans for the key
        let out = patch_frontmatter(
            doc,
            &FmPatch {
                set: vec![],
                remove: vec!["title".into()],
            },
        )
        .unwrap();
        assert_eq!(out, "---\n---\nbody");
    }

    #[test]
    fn later_set_of_same_key_wins() {
        let patch = FmPatch {
            set: vec![
                ("title".into(), FmValue::Str("first".into())),
                ("title".into(), FmValue::Str("second".into())),
            ],
            remove: vec![],
        };
        let out = patch_frontmatter("---\ntitle: x\n---\n", &patch).unwrap();
        assert_eq!(out, "---\ntitle: second\n---\n");
    }

    #[test]
    fn set_wins_over_remove_of_same_key() {
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::Str("kept".into()))],
            remove: vec!["title".into()],
        };
        let out = patch_frontmatter("---\ntitle: x\nstatus: draft\n---\n", &patch).unwrap();
        assert_eq!(out, "---\ntitle: kept\nstatus: draft\n---\n");
    }

    #[test]
    fn validation_rejects_list_on_scalar_field() {
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::List(vec!["a".into()]))],
            remove: vec![],
        };
        let err = patch_frontmatter("---\ntitle: x\n---\n", &patch).unwrap_err();
        assert!(matches!(err, FmError::Validation(_)), "{err}");
    }

    #[test]
    fn invalid_keys_are_rejected() {
        for bad in [
            "",
            "has space",
            "colon:key",
            "hash#key",
            "-dash",
            "new\nline",
        ] {
            let patch = FmPatch {
                set: vec![(bad.into(), FmValue::Str("v".into()))],
                remove: vec![],
            };
            let err = patch_frontmatter("---\ntitle: x\n---\n", &patch).unwrap_err();
            assert!(matches!(err, FmError::InvalidKey(_)), "key {bad:?}: {err}");
        }
    }

    #[test]
    fn trailing_comments_survive_and_unknown_keys_insert_before_them() {
        let doc = "---\ntitle: x\n# trailing note\n---\nbody";
        let patch = FmPatch {
            set: vec![("customKey".into(), FmValue::Str("v".into()))],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(
            out,
            "---\ntitle: x\ncustomKey: v\n# trailing note\n---\nbody"
        );
    }

    #[test]
    fn comment_between_list_items_belongs_to_that_span() {
        let doc = "---\ntags:\n  - a\n# middle comment\n  - b\nstatus: draft\n---\n";
        // No-op stays identical.
        assert_eq!(patch_frontmatter(doc, &FmPatch::default()).unwrap(), doc);
        // Replacing tags replaces the interleaved comment too (it is part of
        // the span), while status is untouched.
        let patch = FmPatch {
            set: vec![("tags".into(), FmValue::List(vec!["z".into()]))],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(out, "---\ntags:\n  - z\nstatus: draft\n---\n");
    }

    #[test]
    fn body_with_fake_delimiters_is_never_touched() {
        let doc = "---\ntitle: x\n---\nintro\n---\nfake: frontmatter\n---\nend\n";
        let patch = FmPatch {
            set: vec![("title".into(), FmValue::Str("y".into()))],
            remove: vec![],
        };
        let out = patch_frontmatter(doc, &patch).unwrap();
        assert_eq!(
            out,
            "---\ntitle: y\n---\nintro\n---\nfake: frontmatter\n---\nend\n"
        );
    }

    #[test]
    fn empty_set_on_missing_block_changes_nothing() {
        let body = "no fm";
        let patch = FmPatch {
            set: vec![],
            remove: vec!["title".into()],
        };
        assert_eq!(patch_frontmatter(body, &patch).unwrap(), body);
    }

    #[test]
    fn extra_key_values_validate_round_trip() {
        let patch = FmPatch {
            set: vec![
                ("customStr".into(), FmValue::Str("42".into())),
                (
                    "customList".into(),
                    FmValue::List(vec!["a".into(), "b".into()]),
                ),
                ("customNull".into(), FmValue::Null),
            ],
            remove: vec![],
        };
        let out = patch_frontmatter("---\ntitle: x\n---\n", &patch).unwrap();
        let parsed = parse(&out);
        assert_eq!(parsed.fm.extra.get("customStr"), Some(&"42".to_string()));
        assert_eq!(
            parsed.fm.extra.get("customList"),
            Some(&r#"["a","b"]"#.to_string())
        );
        assert_eq!(parsed.fm.extra.get("customNull"), Some(&String::new()));
    }
}
