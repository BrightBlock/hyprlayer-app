//! JS-compatible string rules the reducer's output depends on (`trim`, `\s`, the `/i` and `/m` regexes).

const SKILL_BODY_PREFIX: &str = "Base directory for this skill:";
const COMPACT_SUMMARY_START: &str = "This session is being continued from a previous conversation";
const STOP_HOOK_FEEDBACK: &str = "Stop hook feedback:\n";

// Rust's `is_whitespace` has U+0085 and lacks U+FEFF; JS is the reverse.
fn js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

pub fn js_trim(s: &str) -> &str {
    s.trim_matches(js_ws)
}

pub fn skill_body_name(text: &str) -> Option<String> {
    let rest = text.strip_prefix(SKILL_BODY_PREFIX)?;
    let line = rest.split('\n').next().unwrap_or(rest);
    let dir = js_trim(line);
    let name = dir
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(dir);
    Some(if name.is_empty() { "skill" } else { name }.to_string())
}

pub fn is_interrupt_notice(text: &str) -> bool {
    text == "[Request interrupted by user]" || text == "[Request interrupted by user for tool use]"
}

// Line-anchored like the `m` flag: JS treats \r, U+2028 and U+2029 as line breaks too.
pub fn compact_summary_body(text: &str) -> Option<&str> {
    let starts = std::iter::once(0).chain(
        text.char_indices()
            .filter(|(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
            .map(|(i, c)| i + c.len_utf8()),
    );
    starts
        .into_iter()
        .find(|&at| text[at..].starts_with(COMPACT_SUMMARY_START))
        .map(|at| &text[at..])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Hook,
    Goal,
}

pub struct NativeContinuation {
    pub reason: String,
    pub native: NativeKind,
}

pub fn native_continuation(text: &str) -> Option<NativeContinuation> {
    let body = js_trim(text.strip_prefix(STOP_HOOK_FEEDBACK)?);
    // `^\[[^\]\n]*\]:\s*`
    let goal = body.strip_prefix('[').and_then(|rest| {
        let close = rest.find([']', '\n'])?;
        let after = rest[close..].strip_prefix("]:")?;
        Some(after.trim_start_matches(js_ws))
    });
    Some(match goal {
        Some(reason) => NativeContinuation {
            reason: js_trim(reason).to_string(),
            native: NativeKind::Goal,
        },
        None => NativeContinuation {
            reason: body.to_string(),
            native: NativeKind::Hook,
        },
    })
}

// `/<\/?think(?:ing)?>/gi`: ASCII-only case folding, as JS does without the `u` flag.
fn think_tag_at(bytes: &[u8], at: usize) -> Option<usize> {
    let mut i = at;
    if bytes.get(i) != Some(&b'<') {
        return None;
    }
    i += 1;
    if bytes.get(i) == Some(&b'/') {
        i += 1;
    }
    let word = |i: usize, w: &[u8]| {
        bytes
            .get(i..i + w.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(w))
    };
    if !word(i, b"think") {
        return None;
    }
    i += 5;
    if word(i, b"ing") && bytes.get(i + 3) == Some(&b'>') {
        return Some(i + 4);
    }
    (bytes.get(i) == Some(&b'>')).then_some(i + 1)
}

fn think_tags(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match think_tag_at(bytes, i) {
            Some(end) => {
                out.push((i, end));
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

pub fn split_inline_thinking(text: &str) -> (Option<String>, String) {
    let tags = think_tags(text);
    if tags.is_empty() {
        return (None, text.to_string());
    }
    let mut reasoning: Vec<String> = Vec::new();
    let mut answer = String::new();
    let mut buf = String::new();
    let mut in_think = false;
    let mut last = 0;
    for (start, end) in tags {
        buf.push_str(&text[last..start]);
        last = end;
        if text.as_bytes()[start + 1] == b'/' {
            reasoning.push(if in_think {
                std::mem::take(&mut buf)
            } else {
                answer.clone() + &std::mem::take(&mut buf)
            });
            if !in_think {
                answer.clear();
            }
            in_think = false;
        } else {
            answer.push_str(&std::mem::take(&mut buf));
            in_think = true;
        }
    }
    buf.push_str(&text[last..]);
    if in_think {
        reasoning.push(buf);
    } else {
        answer.push_str(&buf);
    }
    let joined = reasoning
        .iter()
        .map(|r| js_trim(r))
        .filter(|r| !r.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        (None, text.to_string())
    } else {
        (Some(joined), js_trim(&answer).to_string())
    }
}
