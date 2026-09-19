//! Scrubbing for text bound for a log: argv, error messages, stderr tails.

/// Masks the userinfo of every `scheme://user[:secret]@host` URL in `text`;
/// clone URLs and remotes can embed a token, and git echoes them in errors.
pub fn url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme_end) = rest.find("://") {
        let authority_start = scheme_end + 3;
        out.push_str(&rest[..authority_start]);
        let after = &rest[authority_start..];
        let authority_len = after
            .find(|c: char| matches!(c, '/' | '?' | '#' | '"' | '\'') || c.is_whitespace())
            .unwrap_or(after.len());
        let authority = &after[..authority_len];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &after[authority_len..];
    }
    out.push_str(rest);
    out
}

/// The last `max` bytes of `text`, cut on a char boundary, so a log line
/// carries the end of a long stderr (where the error usually is), not all of it.
pub fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_only_the_userinfo_of_each_url() {
        assert_eq!(
            url_credentials("clone https://me:ghp_abc@github.com/a/b.git failed"),
            "clone https://***@github.com/a/b.git failed"
        );
        assert_eq!(
            url_credentials("http://tok@host and ssh://git@example.com:22/r"),
            "http://***@host and ssh://***@example.com:22/r"
        );
        assert_eq!(
            url_credentials("https://github.com/a/b?x=y@z"),
            "https://github.com/a/b?x=y@z"
        );
        assert_eq!(
            url_credentials("git@github.com:a/b.git"),
            "git@github.com:a/b.git"
        );
    }

    #[test]
    fn tail_keeps_the_end_on_a_char_boundary() {
        assert_eq!(tail("short", 10), "short");
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("aé", 1), "");
        assert_eq!(tail("aéb", 2), "b");
    }
}
