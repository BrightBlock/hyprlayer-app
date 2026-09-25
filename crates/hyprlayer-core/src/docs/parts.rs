//! Body/frontmatter split and recompose, BOM-aware. `split` is the read side
//! of the save contract; `compose` is the write side. Together they let a
//! save with no edits leave the file byte-identical.

use serde::{Deserialize, Serialize};

use super::frontmatter::{
    bom_len, extract_block, patch_frontmatter, tokenize_keys, FmError, FmPatch,
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
    let eol = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let body = match_eol(original, body);
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
fn match_eol<'b>(original: &str, body: &'b str) -> std::borrow::Cow<'b, str> {
    let uniformly_crlf = original.contains("\r\n") && !original.replace("\r\n", "").contains('\n');
    if uniformly_crlf && !body.contains('\r') {
        body.replace('\n', "\r\n").into()
    } else {
        body.into()
    }
}
