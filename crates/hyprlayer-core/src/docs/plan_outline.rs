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
    LazyLock::new(|| Regex::new(r"^(\s*)(?:```|~~~)\s*([A-Za-z0-9_+-]*)").expect("fence regex"));
/// `#### Automated Verification (from `tools/web/`):`
static HEADING_DIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:from|in)\s+`([^`]+)`").expect("heading dir regex"));
/// `Run these from `tools/web/`:` on its own line under a criteria heading.
static LINE_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(?:all\s+(?:commands|checks)\s+)?run(?:\s+(?:these|them|the\s+commands|the\s+checks))?\s+(?:from|in)\s+`([^`]+)`",
    )
    .expect("line dir regex")
});
const SHELL_FENCES: [&str; 5] = ["", "sh", "bash", "zsh", "shell"];

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
    /// The directory its criteria heading or a `Run from` line names, relative to the repo.
    pub dir: Option<String>,
    /// A shell fence right under a box that ends in a colon.
    pub block: Option<String>,
}

impl PlanBox {
    /// The box's fence, else its last backtick span: fits "Types check: `pnpm typecheck`" and
    /// "`cargo test` passes" alike.
    pub fn command(&self) -> Option<&str> {
        if let Some(block) = &self.block {
            return Some(block);
        }
        let end = self.text.rfind('`')?;
        let start = self.text[..end].rfind('`')?;
        let span = self.text[start + 1..end].trim();
        (!span.is_empty()).then_some(span)
    }
}

/// `./app/` → `app`; the repo root itself is none.
fn normalize_dir(raw: &str) -> Option<String> {
    let mut dir = raw.trim();
    while let Some(rest) = dir.strip_prefix("./") {
        dir = rest;
    }
    let dir = dir.trim_end_matches('/');
    (!dir.is_empty() && dir != ".").then(|| dir.to_string())
}

fn dir_in(regex: &Regex, text: &str) -> Option<String> {
    regex.captures(text).and_then(|c| normalize_dir(&c[1]))
}

/// An open fence, and whether it may be the command of the box just above it.
struct Fence {
    indent: usize,
    command_for_box: bool,
    lines: Vec<String>,
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
    // The directory the criteria heading names, and the one its current subgroup names.
    let mut criteria_dir: Option<String> = None;
    let mut group_dir: Option<String> = None;
    let mut fence: Option<Fence> = None;
    // The last box ends in a colon and only blank lines have followed it.
    let mut box_awaits_block = false;

    for line in content[body_offset..].lines() {
        if let Some(found) = FENCE.captures(line) {
            match fence.take() {
                Some(closed) if closed.command_for_box => {
                    let block = closed.lines.join("\n");
                    let block = block.trim_start_matches(['\n', '\r']).trim_end();
                    let last = phases.last_mut().and_then(|p| p.boxes.last_mut());
                    if let (false, Some(last)) = (block.is_empty(), last) {
                        last.block = Some(block.to_string());
                    }
                }
                Some(_) => {}
                None => {
                    let lang = found[2].to_ascii_lowercase();
                    fence = Some(Fence {
                        indent: found[1].len(),
                        command_for_box: box_awaits_block && SHELL_FENCES.contains(&lang.as_str()),
                        lines: Vec::new(),
                    });
                }
            }
            box_awaits_block = false;
            continue;
        }
        if let Some(open) = fence.as_mut() {
            if open.command_for_box {
                open.lines.push(dedent(line, open.indent));
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(heading) = HEADING.captures(line) {
            box_awaits_block = false;
            let level = heading[1].len();
            let text = &heading[2];
            if level <= 3 {
                if let Some(found) = PHASE.captures(text) {
                    phase = Some(level);
                    criteria = None;
                    (criteria_dir, group_dir) = (None, None);
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
                (criteria_dir, group_dir) = (None, None);
            }
            let lower = text.to_lowercase();
            if phase.is_some() && lower.contains("success criteria") {
                criteria = Some((level, false));
                (criteria_dir, group_dir) = (dir_in(&HEADING_DIR, text), None);
            } else if let Some((_, manual)) = criteria.as_mut() {
                *manual = !lower.contains("automated") && lower.contains("manual");
                group_dir = dir_in(&HEADING_DIR, text);
            }
            continue;
        }
        box_awaits_block = false;
        if phase.is_none() {
            continue;
        }
        let Some(mark) = CHECKBOX.captures(line) else {
            if let (Some((_, false)), Some(dir)) = (criteria, dir_in(&LINE_DIR, line)) {
                group_dir = Some(dir);
            }
            continue;
        };
        let section = match criteria {
            Some((_, true)) => BoxSection::Manual,
            Some((_, false)) => BoxSection::Automated,
            None => BoxSection::Task,
        };
        let text = line[mark.get(0).map_or(0, |m| m.end())..]
            .trim()
            .to_string();
        box_awaits_block = text.ends_with(':');
        let plan_box = PlanBox {
            text,
            ticked: mark[1].eq_ignore_ascii_case("x"),
            dir: (section == BoxSection::Automated)
                .then(|| group_dir.clone().or_else(|| criteria_dir.clone()))
                .flatten(),
            section,
            block: None,
        };
        if let Some(current) = phases.last_mut() {
            current.boxes.push(plan_box);
        }
    }
    PlanOutline { title, phases }
}

/// Drops up to `indent` leading whitespace, the fence's own indent.
fn dedent(line: &str, indent: usize) -> String {
    let cut: usize = line
        .chars()
        .take(indent)
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum();
    line[cut..].to_string()
}
