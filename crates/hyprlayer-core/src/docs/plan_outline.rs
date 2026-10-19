//! A plan's phases and checkboxes, read with the grammar `planDoc.ts` uses.

use std::sync::LazyLock;

use regex::Regex;

use super::frontmatter;

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*#*\s*$").expect("heading regex"));
static PHASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:.*?\s)?Phase\s+(\d+[a-z]?)\b\s*(?:[:—–-]\s*)?(.*)$").expect("phase regex")
});
static CHECKBOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[([ xX~])\]\s+").expect("checkbox regex"));
static FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(```|~~~)").expect("fence regex"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanOutline {
    pub title: Option<String>,
    pub phases: Vec<PlanPhase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanPhase {
    pub id: String,
    pub name: String,
    pub boxes: Vec<PlanBox>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSection {
    Task,
    Automated,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanBox {
    pub text: String,
    pub ticked: bool,
    pub section: BoxSection,
}

impl PlanBox {
    /// The last backtick span: fits "Types check: `pnpm typecheck`" and "`cargo test` passes" alike.
    pub fn command(&self) -> Option<&str> {
        let end = self.text.rfind('`')?;
        let start = self.text[..end].rfind('`')?;
        let span = self.text[start + 1..end].trim();
        (!span.is_empty()).then_some(span)
    }
}

/// Boxes outside a phase or inside a fence are not part of the outline.
pub fn plan_outline(content: &str) -> PlanOutline {
    let parsed_title = frontmatter::parse(content).fm.title;
    let body_offset = frontmatter::extract_block(content).map_or(0, |(_, offset)| offset);
    let mut title = parsed_title.filter(|t| !t.trim().is_empty());
    let mut phases: Vec<PlanPhase> = Vec::new();
    let mut phase: Option<usize> = None;
    // The criteria heading's level, and whether its current subgroup is manual.
    let mut criteria: Option<(usize, bool)> = None;
    let mut in_fence = false;

    for line in content[body_offset..].lines() {
        if FENCE.is_match(line) {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            if let Some(heading) = HEADING.captures(line) {
                let level = heading[1].len();
                let text = &heading[2];
                if level <= 3 {
                    if let Some(found) = PHASE.captures(text) {
                        phase = Some(level);
                        criteria = None;
                        phases.push(PlanPhase {
                            id: found[1].to_string(),
                            name: found[2].trim().to_string(),
                            boxes: Vec::new(),
                        });
                        continue;
                    }
                }
                if level == 1 && title.is_none() {
                    title = Some(text.to_string());
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
        let Some(mark) = CHECKBOX.captures(line) else {
            continue;
        };
        let section = match criteria {
            Some((_, true)) => BoxSection::Manual,
            Some((_, false)) => BoxSection::Automated,
            None => BoxSection::Task,
        };
        let plan_box = PlanBox {
            text: line[mark.get(0).map_or(0, |m| m.end())..]
                .trim()
                .to_string(),
            ticked: mark[1].eq_ignore_ascii_case("x"),
            section,
        };
        if let Some(current) = phases.last_mut() {
            current.boxes.push(plan_box);
        }
    }
    PlanOutline { title, phases }
}
