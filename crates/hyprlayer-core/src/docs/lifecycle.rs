//! Where a thoughts doc's `status` settles as its workstream moves through the pipeline.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::frontmatter::{self, FmPatch, FmValue};
use super::vocab::{DocStatus, DocType};
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Milestone {
    ImplementStarted,
    Merged,
    Closed,
}

/// The status a doc moves to at `milestone`, or `None` to leave it alone.
pub fn next_status(
    doc_type: DocType,
    status: Option<&str>,
    content: &str,
    milestone: Milestone,
) -> Option<DocStatus> {
    let status = status.map(str::trim).unwrap_or_default();
    let is = |names: &[&str]| names.iter().any(|n| status.eq_ignore_ascii_case(n));
    match (doc_type, milestone) {
        // `merged`, `closed` and `complete` are the skills' words; the vocabulary has none of them.
        (DocType::Pr, Milestone::Merged) if is(&["draft", "active", "merged", "closed"]) => {
            Some(DocStatus::Implemented)
        }
        (DocType::Pr, Milestone::Closed) if is(&["draft", "active", "closed"]) => {
            Some(DocStatus::Archived)
        }
        (DocType::Plan, Milestone::ImplementStarted) if is(&["draft"]) => Some(DocStatus::Active),
        (DocType::Plan, Milestone::Merged) if is(&["draft", "active"]) && plan_built(content) => {
            Some(DocStatus::Implemented)
        }
        (DocType::Research, Milestone::Merged) if is(&["draft", "active", "complete"]) => {
            Some(DocStatus::Implemented)
        }
        (DocType::Handoff, Milestone::Merged) if is(&["draft", "active", "complete"]) => {
            Some(DocStatus::Archived)
        }
        _ => None,
    }
}

/// A PR record a merge or close could still move.
pub fn record_unsettled(status: Option<&str>) -> bool {
    next_status(DocType::Pr, status, "", Milestone::Merged).is_some()
}

/// `content` with its status moved for `milestone`, or `None` when it stays.
pub fn settle_as(content: &str, doc_type: DocType, milestone: Milestone) -> Result<Option<String>> {
    let status = frontmatter::parse(content).fm.status;
    next_status(doc_type, status.as_deref(), content, milestone)
        .map(|next| restatus(content, next))
        .transpose()
}

/// Settles the doc at `path` in place if `admit` takes its type; a missing file is not an error.
pub fn settle_file(
    path: &Path,
    milestone: Milestone,
    admit: impl Fn(DocType) -> bool,
) -> Result<Option<DocStatus>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut fm = frontmatter::parse(&content).fm;
    frontmatter::apply_path_fallbacks(&mut fm, path);
    let Some(doc_type) = fm
        .doc_type
        .as_deref()
        .and_then(DocType::try_parse)
        .filter(|t| admit(*t))
    else {
        return Ok(None);
    };
    let Some(next) = next_status(doc_type, fm.status.as_deref(), &content, milestone) else {
        return Ok(None);
    };
    std::fs::write(path, restatus(&content, next)?)?;
    Ok(Some(next))
}

fn restatus(content: &str, status: DocStatus) -> Result<String> {
    let patch = FmPatch {
        set: vec![("status".into(), FmValue::Str(status.as_str().into()))],
        remove: Vec::new(),
    };
    Ok(frontmatter::patch_frontmatter(content, &patch)?)
}

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*#*\s*$").expect("heading regex"));
static PHASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:.*?\s)?Phase\s+(\d+[a-z]?)\b\s*(?:[:—–-]\s*)?(.*)$").expect("phase regex")
});
static CHECKBOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[([ xX~])\]\s+").expect("checkbox regex"));
static FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(```|~~~)").expect("fence regex"));

/// Every non-manual checkbox under a phase is ticked; parses as `planDoc.ts` does. No boxes is no evidence.
pub fn plan_built(content: &str) -> bool {
    let body_offset = frontmatter::extract_block(content).map_or(0, |(_, offset)| offset);
    let mut phase: Option<usize> = None;
    // The criteria heading's level, and whether its current subgroup is manual.
    let mut criteria: Option<(usize, bool)> = None;
    let mut in_fence = false;
    let (mut required, mut open) = (0usize, 0usize);

    for line in content[body_offset..].lines() {
        if FENCE.is_match(line) {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            if let Some(heading) = HEADING.captures(line) {
                let level = heading[1].len();
                let text = &heading[2];
                if level <= 3 && PHASE.is_match(text) {
                    phase = Some(level);
                    criteria = None;
                    continue;
                }
                if phase.is_some_and(|p| level <= p) {
                    phase = None;
                }
                if criteria.is_some_and(|(c, _)| phase.is_none() || level <= c) {
                    criteria = None;
                }
                let lower = text.to_lowercase();
                if phase.is_some() && lower.contains("success criteria") {
                    criteria = Some((level, false));
                } else if let Some((_, manual)) = criteria.as_mut() {
                    *manual = !lower.contains("automated") && lower.contains("manual");
                }
                continue;
            }
        }
        if in_fence || phase.is_none() {
            continue;
        }
        let Some(mark) = CHECKBOX.captures(line).map(|c| c[1].to_string()) else {
            continue;
        };
        if criteria.is_some_and(|(_, manual)| manual) {
            continue;
        }
        required += 1;
        if !mark.eq_ignore_ascii_case("x") {
            open += 1;
        }
    }
    required > 0 && open == 0
}
