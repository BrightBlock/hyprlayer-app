//! Where a thoughts doc's `status` settles as its workstream moves through the pipeline.

use std::path::Path;

use super::frontmatter::{self, FmPatch, FmValue};
use super::plan_outline::{plan_outline, BoxSection};
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

/// Every non-manual checkbox under a phase is ticked. No boxes is no evidence.
pub fn plan_built(content: &str) -> bool {
    let outline = plan_outline(content);
    let mut required = outline
        .phases
        .iter()
        .flat_map(|phase| &phase.boxes)
        .filter(|b| b.section != BoxSection::Manual)
        .peekable();
    required.peek().is_some() && required.all(|b| b.ticked)
}
