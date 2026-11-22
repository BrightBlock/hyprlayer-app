//! Body/frontmatter split and recompose, BOM-aware. `split` is the read side
//! of the save contract; `compose` is the write side. Together they let a
//! save with no edits leave the file byte-identical.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::frontmatter::{
    bom_len, extract_block, first_eol, patch_frontmatter, tokenize_keys, FmError, FmPatch,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrontmatterEdit {
    Patch(FmPatch),
    Raw(String),
}

pub struct DocParts<'a> {
    pub bom_len: usize,
    /// YAML between the delimiters.
    pub block: Option<&'a str>,
    /// Absolute: includes the BOM.
    pub body_offset: usize,
    pub fm_error: Option<String>,
    pub keys: Vec<String>,
}

impl DocParts<'_> {
    pub fn body<'c>(&self, content: &'c str) -> &'c str {
        &content[self.body_offset..]
    }
}

// extract_block already skips a BOM and returns an absolute body_offset.
pub fn split(content: &str) -> DocParts<'_> {
    let bom = bom_len(content);
    match extract_block(content) {
        None => DocParts {
            bom_len: bom,
            block: None,
            body_offset: bom,
            fm_error: None,
            keys: vec![],
        },
        Some((block, body_offset)) => DocParts {
            bom_len: bom,
            block: Some(block),
            body_offset,
            fm_error: strict_error(block),
            keys: tokenize_keys(block),
        },
    }
}

pub fn strict_error(yaml: &str) -> Option<String> {
    match serde_yaml::from_str::<serde_yaml::Value>(yaml) {
        Ok(serde_yaml::Value::Mapping(_) | serde_yaml::Value::Null) => None,
        Ok(_) => Some("frontmatter is not a mapping".into()),
        Err(e) => Some(e.to_string()),
    }
}

// The BOM stays in `rebuilt`; patch_frontmatter and replace_block keep it in front.
pub fn compose(original: &str, body: &str, edit: &FrontmatterEdit) -> Result<String, FmError> {
    let parts = split(original);
    let eol = first_eol(original);
    let body = match_eol(original, parts.body(original), body);
    let rebuilt = format!("{}{}", &original[..parts.body_offset], body);
    match edit {
        FrontmatterEdit::Patch(p) if p.set.is_empty() && p.remove.is_empty() => Ok(rebuilt),
        // A malformed block changes only through an explicit Raw.
        FrontmatterEdit::Patch(_) if parts.fm_error.is_some() => Err(FmError::Malformed),
        FrontmatterEdit::Patch(p) => patch_frontmatter(&rebuilt, p),
        FrontmatterEdit::Raw(yaml) => replace_block(&rebuilt, yaml, eol),
    }
}

pub fn replace_block(text: &str, yaml: &str, eol: &str) -> Result<String, FmError> {
    if !yaml.trim().is_empty() {
        if let Some(msg) = strict_error(yaml) {
            return Err(FmError::InvalidRaw(msg));
        }
    }

    let bom = bom_len(text);
    let head = &text[..bom];

    match extract_block(text) {
        Some((block, body_offset)) => {
            let open_len = if text[bom..].starts_with("---\r\n") {
                5
            } else {
                4
            };
            let opening = &text[bom..bom + open_len];
            let closing = &text[bom + open_len + block.len()..body_offset];
            let body = &text[body_offset..];

            if yaml.trim().is_empty() {
                return Ok(format!("{head}{body}"));
            }

            let normalised = normalise_yaml(yaml, eol, block);
            Ok(format!("{head}{opening}{normalised}{closing}{body}"))
        }
        None => {
            if yaml.trim().is_empty() {
                return Ok(text.to_string());
            }
            let normalised = normalise_yaml(yaml, eol, "");
            Ok(format!(
                "{head}---{eol}{normalised}---{eol}{}",
                &text[bom..]
            ))
        }
    }
}

/// Converts EOLs to `eol` and trims trailing whitespace, then re-appends the
/// original block's trailing whitespace (e.g. a blank line before the
/// closer), so a raw edit to one line changes only that line.
fn normalise_yaml(yaml: &str, eol: &str, original_block: &str) -> String {
    let converted = if eol == "\n" {
        yaml.replace("\r\n", "\n")
    } else {
        let lf = yaml.replace("\r\n", "\n");
        lf.replace('\n', eol)
    };
    let trimmed = converted.trim_end();
    let trail_start = original_block
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let trailing = &original_block[trail_start..];
    if trailing.is_empty() {
        format!("{trimmed}{eol}")
    } else {
        format!("{trimmed}{trailing}")
    }
}

// CodeMirror hands back LF; a uniformly CRLF file stays CRLF.
pub(crate) fn match_eol<'b>(original: &str, original_text: &str, text: &'b str) -> Cow<'b, str> {
    if text.contains("\r\n") || !original.contains("\r\n") {
        return text.into();
    }
    if !original.replace("\r\n", "").contains('\n') {
        return text.replace('\n', "\r\n").into();
    }
    restore_endings(original_text, text, first_eol(original)).map_or(text.into(), Cow::Owned)
}

fn restore_endings(original: &str, text: &str, fallback: &'static str) -> Option<String> {
    let old: Vec<&str> = original.split_inclusive('\n').collect();
    let new: Vec<&str> = text.split_inclusive('\n').collect();
    let hunks = line_hunks(&original.replace("\r\n", "\n"), text)?;
    let mut endings: Vec<Option<&'static str>> = Vec::with_capacity(new.len());
    let (mut i, mut j) = (0, 0);
    for (old_at, old_len, new_at, new_len) in
        hunks.into_iter().chain([(old.len(), 0, new.len(), 0)])
    {
        if old_at < i || new_at < j || old_at - i != new_at - j || old_at + old_len > old.len() {
            return None;
        }
        endings.extend(old[i..old_at].iter().map(|line| line_ending(line)));
        for k in 0..new_len {
            endings.push(match old_len {
                0 => endings
                    .last()
                    .copied()
                    .flatten()
                    .or_else(|| old.get(old_at).and_then(|line| line_ending(line))),
                _ => line_ending(old[old_at + k.min(old_len - 1)]),
            });
        }
        (i, j) = (old_at + old_len, new_at + new_len);
    }

    let mut out = String::with_capacity(text.len() + new.len());
    let mut last = fallback;
    for (line, ending) in new.iter().zip(endings) {
        match line.strip_suffix('\n') {
            Some(content) => {
                last = ending.unwrap_or(last);
                out.push_str(content);
                out.push_str(last);
            }
            None => out.push_str(line),
        }
    }
    Some(out)
}

fn line_ending(line: &str) -> Option<&'static str> {
    if line.ends_with("\r\n") {
        Some("\r\n")
    } else if line.ends_with('\n') {
        Some("\n")
    } else {
        None
    }
}

fn line_hunks(old: &str, new: &str) -> Option<Vec<(usize, usize, usize, usize)>> {
    let mut opts = git2::DiffOptions::new();
    opts.context_lines(0).interhunk_lines(0).force_text(true);
    let patch =
        git2::Patch::from_buffers(old.as_bytes(), None, new.as_bytes(), None, Some(&mut opts))
            .ok()?;
    // Unified-diff starts are 1-based, except that an empty side names the line before it.
    let at = |start: u32, len: u32| (start as usize).saturating_sub(usize::from(len > 0));
    (0..patch.num_hunks())
        .map(|h| {
            let (hunk, _) = patch.hunk(h).ok()?;
            let (old_len, new_len) = (hunk.old_lines(), hunk.new_lines());
            Some((
                at(hunk.old_start(), old_len),
                old_len as usize,
                at(hunk.new_start(), new_len),
                new_len as usize,
            ))
        })
        .collect()
}
