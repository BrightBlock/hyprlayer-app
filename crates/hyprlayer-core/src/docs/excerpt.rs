/// Two clamped lines at the 344px list width, with headroom.
pub const EXCERPT_CHARS: usize = 200;

/// The body's leading prose as plain text, for list rows.
pub fn excerpt(body: &str) -> String {
    let mut out = String::new();
    let mut fence: Option<&str> = None;
    let mut in_comment = false;
    for raw in body.lines() {
        let line = raw.trim();
        if let Some(close) = fence {
            if line.starts_with(close) {
                fence = None;
            }
            continue;
        }
        if in_comment {
            in_comment = !line.contains("-->");
            continue;
        }
        if let Some(open) = ["```", "~~~"].into_iter().find(|f| line.starts_with(f)) {
            fence = Some(open);
            continue;
        }
        if line.starts_with("<!--") {
            in_comment = !line.contains("-->");
            continue;
        }
        if line.is_empty() || is_heading(line) || line.starts_with('|') || is_rule(line) {
            continue;
        }
        let text: String = strip_links(strip_prefixes(line))
            .chars()
            .filter(|c| !matches!(c, '*' | '`'))
            .collect();
        // Intraword `_` is not emphasis (CommonMark), so `file_path` keeps it.
        for word in text.split_whitespace().map(|w| w.trim_matches('_')) {
            if word.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
        if out.chars().count() > EXCERPT_CHARS {
            break;
        }
    }
    truncate(out)
}

fn is_heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    (1..=6).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

fn is_rule(line: &str) -> bool {
    let mut marks = line.chars().filter(|c| !c.is_whitespace());
    let Some(first) = marks.next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_') && {
        let rest: Vec<char> = marks.collect();
        rest.len() >= 2 && rest.iter().all(|c| *c == first)
    }
}

fn strip_prefixes(line: &str) -> &str {
    let mut rest = line;
    while let Some(quoted) = rest.strip_prefix('>') {
        rest = quoted.trim_start();
    }
    if let Some(item) = ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|m| rest.strip_prefix(m))
    {
        rest = item.trim_start();
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            if let Some(item) = rest[digits..].strip_prefix(". ") {
                rest = item.trim_start();
            }
        }
    }
    for task in ["[ ] ", "[x] ", "[X] "] {
        if let Some(item) = rest.strip_prefix(task) {
            return item.trim_start();
        }
    }
    rest
}

fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix('!').and_then(link) {
            rest = after.1;
            continue;
        }
        if let Some(inner) = rest.strip_prefix("[[") {
            if let Some(end) = inner.find("]]") {
                out.push_str(&inner[..end]);
                rest = &inner[end + 2..];
                continue;
            }
        }
        if let Some((text, after)) = link(rest) {
            out.push_str(text);
            rest = after;
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// `[text](url)` at the start of `s`: the text and what follows the `)`.
fn link(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix('[')?;
    let close = inner.find(']')?;
    let url = inner[close + 1..].strip_prefix('(')?;
    let end = url.find(')')?;
    Some((&inner[..close], &url[end + 1..]))
}

fn truncate(mut out: String) -> String {
    let Some((limit, _)) = out.char_indices().nth(EXCERPT_CHARS) else {
        return out;
    };
    let cut = if out[limit..].starts_with(' ') {
        limit
    } else {
        out[..limit].rfind(' ').filter(|&i| i > 0).unwrap_or(limit)
    };
    out.truncate(cut);
    out.push('…');
    out
}
