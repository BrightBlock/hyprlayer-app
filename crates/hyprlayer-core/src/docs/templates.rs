//! New-document templates: deterministic file names plus frontmatter/body
//! skeletons that match the hyprlayer-cli layout conventions.

use serde::{Deserialize, Serialize};

use super::frontmatter::{patch_frontmatter, FmPatch, FmValue};

/// Inputs for a new thought document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DocTemplate {
    pub title: String,
    /// plan | research | handoff | note | pr
    #[serde(rename = "type")]
    pub doc_type: String,
    /// YYYY-MM-DD
    pub date: String,
    pub ticket: Option<String>,
    pub project: Option<String>,
    /// user | shared | global
    pub scope: String,
    pub author: String,
    pub tags: Vec<String>,
}

/// A rendered new document, ready to be written to disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct NewDoc {
    pub file_name: String,
    pub content: String,
}

/// Render a new document: `YYYY-MM-DD[-TICKET]-kebab-slug.md` plus schema-
/// ordered frontmatter (status: draft, empty optionals omitted) and a minimal
/// body skeleton per type.
pub fn new_doc(t: &DocTemplate) -> NewDoc {
    let mut file_name = t.date.clone();
    if let Some(ticket) = t.ticket.as_deref() {
        let ticket = sanitize_ticket(ticket);
        if !ticket.is_empty() {
            file_name.push('-');
            file_name.push_str(&ticket);
        }
    }
    file_name.push('-');
    file_name.push_str(&slugify(&t.title));
    file_name.push_str(".md");

    let body = match t.doc_type.as_str() {
        "plan" => "\n## Overview\n\n## Phases\n",
        "research" => "\n## Question\n\n## Findings\n",
        "handoff" => "\n## Context\n\n## Next steps\n",
        "pr" => "\n## Summary\n",
        _ => "",
    };

    let mut set: Vec<(String, FmValue)> = Vec::new();
    let mut push_scalar = |key: &str, value: &str| {
        if !value.is_empty() {
            set.push((key.to_string(), FmValue::Str(value.to_string())));
        }
    };
    push_scalar("title", &t.title);
    push_scalar("type", &t.doc_type);
    push_scalar("date", &t.date);
    push_scalar("status", "draft");
    push_scalar("ticket", t.ticket.as_deref().unwrap_or_default());
    push_scalar("project", t.project.as_deref().unwrap_or_default());
    push_scalar("scope", &t.scope);
    push_scalar("author", &t.author);
    if !t.tags.is_empty() {
        set.push(("tags".to_string(), FmValue::List(t.tags.clone())));
    }

    let patch = FmPatch {
        set,
        remove: Vec::new(),
    };
    // Synthesizing onto a frontmatter-free body with validated keys cannot
    // fail: every key is a schema key and every value round-trips.
    let content =
        patch_frontmatter(body, &patch).expect("template frontmatter must render and validate");

    NewDoc { file_name, content }
}

/// Lowercase kebab slug: alphanumeric runs joined by `-`, capped at ~60 chars.
fn slugify(title: &str) -> String {
    const MAX: usize = 60;
    let mut out = String::new();
    let mut pending_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        } else {
            pending_dash = true;
        }
        if out.len() >= MAX {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// Tickets go into file names verbatim minus anything filename-hostile.
fn sanitize_ticket(ticket: &str) -> String {
    ticket
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::frontmatter;

    fn template() -> DocTemplate {
        DocTemplate {
            title: "Auth Refactor: Phase One".to_string(),
            doc_type: "plan".to_string(),
            date: "2026-06-12".to_string(),
            ticket: Some("AUTH-12".to_string()),
            project: Some("hyprlayer".to_string()),
            scope: "shared".to_string(),
            author: "jt".to_string(),
            tags: vec!["security".to_string(), "backend".to_string()],
        }
    }

    #[test]
    fn file_name_with_ticket() {
        let doc = new_doc(&template());
        assert_eq!(
            doc.file_name,
            "2026-06-12-AUTH-12-auth-refactor-phase-one.md"
        );
    }

    #[test]
    fn file_name_without_ticket() {
        let mut t = template();
        t.ticket = None;
        assert_eq!(
            new_doc(&t).file_name,
            "2026-06-12-auth-refactor-phase-one.md"
        );
    }

    #[test]
    fn slug_is_capped_and_sanitized() {
        let mut t = template();
        t.title = "Wow!! ".to_string() + &"very long words ".repeat(20);
        t.ticket = None;
        let doc = new_doc(&t);
        let slug = doc
            .file_name
            .strip_prefix("2026-06-12-")
            .and_then(|s| s.strip_suffix(".md"))
            .unwrap();
        assert!(slug.len() <= 62, "slug too long: {slug}");
        assert!(slug.starts_with("wow-very-long-words"));
        assert!(!slug.ends_with('-'));
        assert!(slug.chars().all(|c| c.is_ascii_lowercase() || c == '-'));

        t.title = "???".to_string();
        assert_eq!(new_doc(&t).file_name, "2026-06-12-untitled.md");
    }

    #[test]
    fn ticket_is_sanitized_or_dropped() {
        let mut t = template();
        t.ticket = Some("AUTH/12 ".to_string());
        assert!(new_doc(&t).file_name.starts_with("2026-06-12-AUTH12-"));
        t.ticket = Some("///".to_string());
        assert!(new_doc(&t).file_name.starts_with("2026-06-12-auth-"));
    }

    #[test]
    fn content_has_schema_order_status_draft_and_skeleton() {
        let doc = new_doc(&template());
        let expected = "---\n\
                        title: 'Auth Refactor: Phase One'\n\
                        type: plan\n\
                        date: 2026-06-12\n\
                        status: draft\n\
                        ticket: AUTH-12\n\
                        project: hyprlayer\n\
                        scope: shared\n\
                        tags:\n  - security\n  - backend\n\
                        author: jt\n\
                        ---\n\
                        \n## Overview\n\n## Phases\n";
        assert_eq!(doc.content, expected);
    }

    #[test]
    fn empty_optionals_are_omitted() {
        let mut t = template();
        t.ticket = None;
        t.project = None;
        t.tags.clear();
        let doc = new_doc(&t);
        assert!(!doc.content.contains("ticket"));
        assert!(!doc.content.contains("project"));
        assert!(!doc.content.contains("tags"));
        assert!(doc.content.contains("status: draft"));
    }

    #[test]
    fn body_skeleton_per_type() {
        let mut t = template();
        for (doc_type, needle) in [
            ("plan", "\n## Overview\n\n## Phases\n"),
            ("research", "\n## Question\n\n## Findings\n"),
            ("handoff", "\n## Context\n\n## Next steps\n"),
            ("pr", "\n## Summary\n"),
        ] {
            t.doc_type = doc_type.to_string();
            let doc = new_doc(&t);
            assert!(doc.content.ends_with(needle), "{doc_type}: {}", doc.content);
        }
        t.doc_type = "note".to_string();
        let doc = new_doc(&t);
        assert!(doc.content.ends_with("---\n"), "note body must be empty");
    }

    #[test]
    fn content_round_trips_through_parse() {
        let mut t = template();
        t.title = "edge: #cases 'with' \"quotes\" ".to_string();
        let doc = new_doc(&t);
        let parsed = frontmatter::parse(&doc.content);
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.fm.title.as_deref(), Some(t.title.as_str()));
        assert_eq!(parsed.fm.doc_type.as_deref(), Some("plan"));
        assert_eq!(parsed.fm.status.as_deref(), Some("draft"));
        assert_eq!(parsed.fm.tags, t.tags);
    }
}
