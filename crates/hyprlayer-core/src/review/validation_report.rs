//! A port of `src/features/review/validationReport.ts`: the bare parse a validation report
//! falls back to when its summary is unavailable. The two parsers must agree.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::runtime::providers::{CheckOutcome, ValidationCheck, ValidationSummary};

static REPORT_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^#{1,4}\s*validation report\b").expect("report heading regex")
});
static HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:#{2,5}\s+(.+?)|\*\*(.+?):?\*\*:?\s*(.*))$").expect("heading regex")
});
static BULLET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\s*)(?:[-*]|\d+\.)\s+(?:\[[ xX]\]\s+)?(.*)$").expect("bullet regex")
});
static OUTCOME_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(pass(ed)?|fail(ed)?)\s*:\s*").expect("label regex"));
static NONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(none|n/a|no (deviations|issues|manual))\b").expect("none regex")
});
static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").expect("bold regex"));
static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("code regex"));
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("space regex"));
static FAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bfail(s|ed|ing)?\b").expect("fail regex"));
static PASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bpass(es|ed)?\b|\bexit(s|ed)? 0\b").expect("pass regex"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Status,
    Checks,
    Findings,
    Matches,
    Deviations,
    Issues,
    Manual,
    Recs,
    Gate,
}

fn section_of(title: &str) -> Option<Section> {
    let t = title.to_lowercase();
    let section = if t.contains("gate") {
        Section::Gate
    } else if t.contains("automated") {
        Section::Checks
    } else if t.contains("deviation") {
        Section::Deviations
    } else if t.contains("issue") || t.contains("risk") {
        Section::Issues
    } else if t.contains("matches") {
        Section::Matches
    } else if t.contains("manual") {
        Section::Manual
    } else if t.contains("recommend") {
        Section::Recs
    } else if t.contains("review finding") || t.contains("findings") {
        Section::Findings
    } else if t.contains("implementation") {
        Section::Status
    } else {
        return None;
    };
    Some(section)
}

fn plain(text: &str) -> String {
    let text = BOLD.replace_all(text, "$1");
    let text = CODE.replace_all(&text, "$1");
    SPACE.replace_all(&text, " ").trim().to_string()
}

fn outcome_of(text: &str) -> Option<CheckOutcome> {
    if FAIL.is_match(text) {
        Some(CheckOutcome::Fail)
    } else if PASS.is_match(text) {
        Some(CheckOutcome::Pass)
    } else {
        None
    }
}

/// Top-level bullets, except a group label ending in `:` yields its nested items instead.
fn items(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<&str> = Vec::new();
    let mut group = false;
    let mut prose: Vec<&str> = Vec::new();
    for line in lines {
        let Some(m) = BULLET.captures(line) else {
            if !line.trim().is_empty() {
                prose.push(line.trim());
            }
            continue;
        };
        let nested = !m[1].is_empty();
        let text = m.get(2).map_or("", |t| t.as_str()).trim();
        if !nested {
            group = plain(text).ends_with(':');
            if !group {
                out.push(text);
            }
        } else if group {
            out.push(text);
        }
    }
    let joined = prose.join(" ");
    if out.is_empty() && !prose.is_empty() {
        out.push(&joined);
    }
    out.into_iter()
        .map(plain)
        .filter(|t| !t.is_empty() && !NONE.is_match(t))
        .collect()
}

fn report_start(text: &str) -> Option<usize> {
    REPORT_HEADING.find(text).map(|m| m.start())
}

pub fn parse_validation_report(text: &str) -> ValidationSummary {
    let start = report_start(text).unwrap_or(0);
    let mut sections: HashMap<Section, Vec<&str>> = HashMap::new();
    let mut current = None;
    for line in text[start..].split('\n').skip(1) {
        if let Some(h) = HEADING.captures(line.trim()) {
            let title = h.get(1).or_else(|| h.get(2)).map_or("", |t| t.as_str());
            if let Some(key) = section_of(title) {
                current = Some(key);
                let rest = h.get(3).map_or("", |r| r.as_str().trim());
                sections.insert(
                    key,
                    if rest.is_empty() {
                        Vec::new()
                    } else {
                        vec![rest]
                    },
                );
                continue;
            }
        }
        if let Some(key) = current {
            sections.entry(key).or_default().push(line);
        }
    }
    let of = |key| items(sections.get(&key).map_or(&[][..], Vec::as_slice));
    let gate = sections
        .get(&Section::Gate)
        .and_then(|lines| lines.iter().map(|l| l.trim()).find(|l| !l.is_empty()))
        .map(plain);
    let mut issues = of(Section::Deviations);
    issues.extend(of(Section::Issues));
    ValidationSummary {
        gate,
        checks: of(Section::Checks)
            .into_iter()
            .map(|t| ValidationCheck {
                text: OUTCOME_LABEL.replace(&t, "").into_owned(),
                outcome: outcome_of(&t),
            })
            .collect(),
        issues,
        notes: Vec::new(),
        manual: of(Section::Manual),
    }
}

/// The newest reply that carries a report, from its heading on; `replies` run oldest first.
pub fn find_validation_report<'a>(
    replies: impl DoubleEndedIterator<Item = &'a str>,
) -> Option<&'a str> {
    replies
        .rev()
        .find_map(|text| report_start(text).map(|start| &text[start..]))
}
