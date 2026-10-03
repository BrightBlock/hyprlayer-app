//! The request-changes message. `src/features/review/reviewMessage.ts` parses it
//! back, so a change here is a change there.

use hyprlayer_models::review::ReviewNoteKind;

const FOOTER: &str =
    "Address each comment, then stop. Files you change go back to unviewed in the review.";

/// The agent note a comment answers.
pub struct MessageNote<'a> {
    pub kind: ReviewNoteKind,
    pub body: &'a str,
}

pub struct MessageComment<'a> {
    pub path: &'a str,
    pub line: u32,
    /// Left on the base side: a line the branch removed.
    pub removed: bool,
    pub body: &'a str,
    pub note: Option<MessageNote<'a>>,
}

/// The body lines of an item: a note reply puts the quoted note, then a blank
/// line, above the answer, so the answer may itself start with `>`.
fn item_lines(c: &MessageComment<'_>) -> Vec<String> {
    let answer = c.body.trim().lines().map(str::to_string);
    let Some(note) = &c.note else {
        return answer.collect();
    };
    let mut lines = vec![format!("Reply to note [{}]:", note.kind.as_str())];
    lines.extend(note.body.trim().lines().map(|l| match l.trim_end() {
        "" => ">".to_string(),
        l => format!("> {l}"),
    }));
    lines.push(String::new());
    lines.extend(answer);
    lines
}

pub fn compose(
    viewed: u32,
    files: u32,
    summary: Option<&str>,
    comments: &[MessageComment<'_>],
) -> String {
    let noun = if comments.len() == 1 {
        "comment"
    } else {
        "comments"
    };
    let mut out = format!(
        "Review · Changes requested · {viewed} / {files} viewed · {} {noun}\n\n",
        comments.len()
    );
    if let Some(summary) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(summary);
        out.push_str("\n\n");
    }
    for c in comments {
        let suffix = if c.removed { " (removed line)" } else { "" };
        let body = item_lines(c);
        let mut lines = body.iter().map(String::as_str);
        out.push_str(&format!(
            "- `{}:{}{suffix}` {}\n",
            c.path,
            c.line,
            lines.next().unwrap_or_default()
        ));
        // Blank body lines keep the indent, so only the list's end is an empty line.
        for line in lines {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if !comments.is_empty() {
        out.push('\n');
    }
    out.push_str(FOOTER);
    out
}
