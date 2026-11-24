use std::sync::Once;

use hyprlayer_models::SkillSourceKind;

use crate::env::RuntimeEnvironment;
use crate::error::{CoreError, Result};

use super::SkillSourceRef;

const GIT_BASE_VAR: &str = "HYPRLAYER_SKILLS_GIT_BASE";
const DEFAULT_GIT_BASE: &str = "https://github.com";

/// Accepts the link forms in ADR-0010; the ref is split from the path after `ls-remote`.
pub fn parse(input: &str) -> Result<SkillSourceRef> {
    let invalid = || CoreError::SkillSourceInvalid {
        input: input.to_string(),
    };
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.contains('\\') || trimmed.chars().any(char::is_control) {
        return Err(invalid());
    }

    // A scheme-less `github.com/...` paste is a URL, not the `owner/repo/path` shorthand.
    let url = strip_scheme(trimmed).or_else(|| {
        let bare = trimmed.strip_prefix("www.").unwrap_or(trimmed);
        bare.starts_with("github.com/").then_some(trimmed)
    });
    let (repo, ref_and_path, path) = if let Some(rest) = url {
        let rest = rest.split(['#', '?']).next().unwrap_or_default();
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let rest = rest.strip_prefix("github.com/").ok_or_else(invalid)?;
        let mut segments = segments(rest).ok_or_else(invalid)?.into_iter();
        let (owner, repo) = (segments.next(), segments.next());
        let repo = repo
            .map(|r| r.strip_suffix(".git").unwrap_or(r))
            .ok_or_else(invalid)?;
        let slug = format!("{}/{repo}", owner.ok_or_else(invalid)?);
        let mut rest: Vec<&str> = match segments.next() {
            None => Vec::new(),
            Some("tree" | "blob") => segments.collect(),
            Some(_) => return Err(invalid()),
        };
        if rest.last() == Some(&"SKILL.md") {
            rest.pop();
        }
        (slug, (!rest.is_empty()).then(|| rest.join("/")), None)
    } else {
        if trimmed.contains(':') || trimmed.starts_with('/') {
            return Err(invalid());
        }
        let segs = segments(trimmed).ok_or_else(invalid)?;
        let [owner, repo, tail @ ..] = segs.as_slice() else {
            return Err(invalid());
        };
        match repo.split_once('@') {
            Some((repo, git_ref)) if !git_ref.is_empty() => {
                let mut rest = vec![git_ref];
                rest.extend(tail);
                (format!("{owner}/{repo}"), Some(rest.join("/")), None)
            }
            Some(_) => return Err(invalid()),
            None => (
                format!("{owner}/{repo}"),
                None,
                (!tail.is_empty()).then(|| tail.join("/")),
            ),
        }
    };

    let repo = repo.strip_suffix(".git").unwrap_or(&repo).to_string();
    if parse_owner_repo(&repo).is_none() {
        return Err(invalid());
    }
    Ok(SkillSourceRef {
        kind: SkillSourceKind::Github,
        repo,
        ref_and_path,
        path,
        name_hint: None,
    })
}

/// `owner/repo`, both segments `[A-Za-z0-9._-]+` and not `.`/`..`.
pub fn parse_owner_repo(s: &str) -> Option<String> {
    let (owner, repo) = s.split_once('/')?;
    let ok = |seg: &str| {
        !seg.is_empty()
            && seg != "."
            && seg != ".."
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    (ok(owner) && ok(repo)).then(|| s.to_string())
}

/// The clone URL, plus the overridden base when the test knob is set and valid.
pub fn git_url(env: &RuntimeEnvironment, repo: &str) -> (String, Option<String>) {
    static WARNED: Once = Once::new();
    let Some(raw) = env.var(GIT_BASE_VAR).and_then(|v| v.to_str()) else {
        return (format!("{DEFAULT_GIT_BASE}/{repo}"), None);
    };
    WARNED.call_once(|| tracing::warn!("{GIT_BASE_VAR} is set; skills clone from {raw}"));
    if raw.starts_with("https://") || raw.starts_with("file://") {
        let url = format!("{}/{repo}", raw.trim_end_matches('/'));
        return (url.clone(), Some(url));
    }
    tracing::warn!("{GIT_BASE_VAR} must be https:// or file://; ignoring it");
    (format!("{DEFAULT_GIT_BASE}/{repo}"), None)
}

fn strip_scheme(s: &str) -> Option<&str> {
    s.strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
}

/// Splits on `/`, tolerating one trailing slash; any empty, `.` or `..` segment refuses.
fn segments(path: &str) -> Option<Vec<&str>> {
    let path = path.strip_suffix('/').unwrap_or(path);
    let segs: Vec<&str> = path.split('/').collect();
    segs.iter()
        .all(|s| !s.is_empty() && *s != "." && *s != "..")
        .then_some(segs)
}
