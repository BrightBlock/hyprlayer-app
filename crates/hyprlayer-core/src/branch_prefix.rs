use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BranchPrefix {
    pub label: String,
    pub prefix: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PrefixSource {
    Builtin,
    Global,
    Repo,
}

pub const BUILTIN_PREFIXES: &[(&str, &str, &[&str])] = &[
    (
        "Feature",
        "feat/",
        &["feature", "add", "new", "implement", "support", "introduce"],
    ),
    (
        "Bug fix",
        "fix/",
        &[
            "fix",
            "bug",
            "crash",
            "broken",
            "regression",
            "error",
            "fails",
            "failing",
        ],
    ),
    (
        "Hotfix",
        "hotfix/",
        &["hotfix", "urgent", "revert", "emergency", "critical"],
    ),
    (
        "Chore",
        "chore/",
        &[
            "chore",
            "bump",
            "upgrade",
            "cleanup",
            "deps",
            "dependency",
            "release",
        ],
    ),
    (
        "Docs",
        "docs/",
        &[
            "docs",
            "doc",
            "documentation",
            "readme",
            "changelog",
            "comment",
        ],
    ),
    (
        "Refactor",
        "refactor/",
        &[
            "refactor",
            "rename",
            "extract",
            "simplify",
            "restructure",
            "tidy",
        ],
    ),
];

pub fn builtin_prefixes() -> Vec<BranchPrefix> {
    BUILTIN_PREFIXES
        .iter()
        .map(|&(label, prefix, keywords)| BranchPrefix {
            label: label.to_string(),
            prefix: prefix.to_string(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
        })
        .collect()
}

impl BranchPrefix {
    pub fn match_words(&self) -> Vec<String> {
        if !self.keywords.is_empty() {
            let mut seen = std::collections::HashSet::new();
            let mut words = Vec::new();
            for k in &self.keywords {
                let word = k.trim().to_lowercase();
                if word.is_empty() || !seen.insert(word.clone()) {
                    continue;
                }
                words.push(word);
            }
            return words;
        }
        self.label
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_string())
            .collect()
    }

    pub fn with_effective_keywords(self) -> Self {
        let keywords = self.match_words();
        Self { keywords, ..self }
    }
}

pub fn prefix_source(global: Option<&[BranchPrefix]>, repo: &[BranchPrefix]) -> PrefixSource {
    if !repo.is_empty() {
        PrefixSource::Repo
    } else if global.is_some_and(|g| !g.is_empty()) {
        PrefixSource::Global
    } else {
        PrefixSource::Builtin
    }
}

pub fn resolve_prefixes(
    global: Option<&[BranchPrefix]>,
    repo: &[BranchPrefix],
) -> Vec<BranchPrefix> {
    match prefix_source(global, repo) {
        PrefixSource::Repo => repo.to_vec(),
        PrefixSource::Global => global.unwrap_or_default().to_vec(),
        PrefixSource::Builtin => builtin_prefixes(),
    }
}

pub fn validate_prefix(prefix: &str) -> Result<()> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let Some(caps) = INVALID_PREFIX.captures(trimmed) else {
        return Ok(());
    };
    let detail = if caps.name("lead_slash").is_some() {
        format!("branch prefix {prefix:?} must not start with '/'")
    } else if caps.name("lead_dash").is_some() {
        format!("branch prefix {prefix:?} must not start with '-'")
    } else if caps.name("double_slash").is_some() {
        format!("branch prefix {prefix:?} must not contain '//'")
    } else if caps.name("dot_dot").is_some() {
        format!("branch prefix {prefix:?} must not contain '..'")
    } else if caps.name("reflog").is_some() {
        format!("branch prefix {prefix:?} must not contain '@{{'")
    } else if caps.name("ws_ctl").is_some() {
        format!("branch prefix {prefix:?} must not contain whitespace or control characters")
    } else if let Some(m) = caps.name("forbidden") {
        format!("branch prefix {prefix:?} must not contain '{}'", m.as_str())
    } else if caps.name("dot_segment").is_some() {
        format!("branch prefix {prefix:?} has a path segment starting with '.'")
    } else {
        debug_assert!(caps.name("lock_segment").is_some());
        format!("branch prefix {prefix:?} has a path segment ending in '.lock'")
    };
    Err(CoreError::BranchPrefixInvalid { detail })
}

static INVALID_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<lead_slash>^/)
        # `check-ref-format` ALLOWS a leading '-', but the branch name is the
        # argument to `git worktree add -b`, and the `git branch` it runs
        # internally re-parses it as a switch bundle: a `-fix/` prefix
        # reports `error: unknown switch 'x'` over a `git branch` usage dump,
        # naming neither the prefix nor the workstream. Anchored to the
        # START of the whole prefix, not each component — `fix/-x/` creates
        # a branch fine.
        | (?P<lead_dash>^-)
        | (?P<double_slash>//)
        | (?P<dot_dot>\.\.)
        # The reflog selector syntax. Rejected by `check-ref-format` anywhere
        # in the name, so without this a saved `fix@{/` prefix passes every
        # settings write and then fails at every isolated create with a bare
        # `fatal: '...' is not a valid branch name`. A lone '@' is fine.
        | (?P<reflog>@\{)
        | (?P<ws_ctl>[\x00-\x20\x7f])
        | (?P<forbidden>[~^:?*\[\\])
        # A path segment cannot start with '.'. Prepending cannot rescue this,
        # so the rule holds even for the unfinished last segment: `fix/.hidden`
        # becomes the equally illegal `fix/.hidden<slug>`.
        | (?P<dot_segment>(?:^|/)\.)
        # A SUFFIX rule, and — unlike `dot_segment` above — only for a segment
        # the prefix actually FINISHES, i.e. one followed by another '/'. The
        # slug always finishes the last segment: `fix/x.lock` builds
        # `fix/x.lockadd-login-a1b2`, which git creates happily, so matching
        # a trailing `.lock` with nothing after it would be a false positive.
        # `fix/x.lock/` — where the '/' closes the segment — is the shape
        # git really refuses.
        | (?P<lock_segment>\.lock/)
        ",
    )
    .expect("branch-prefix regex is a fixed literal")
});

pub fn validate_prefixes(list: &[BranchPrefix]) -> Result<()> {
    let invalid = |detail: String| CoreError::BranchPrefixInvalid { detail };
    let mut seen_prefixes = std::collections::HashSet::new();
    for entry in list {
        validate_prefix(&entry.prefix)?;
        if entry.label.trim().is_empty() {
            return Err(invalid("every branch type needs a label".to_string()));
        }
        let prefix = entry.prefix.trim();
        if prefix.is_empty() {
            return Err(invalid(format!(
                "\"{}\" needs a prefix",
                entry.label.trim()
            )));
        }
        if !seen_prefixes.insert(prefix) {
            return Err(invalid(format!(
                "more than one branch type uses the prefix {prefix:?}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefix(label: &str, p: &str, keywords: &[&str]) -> BranchPrefix {
        BranchPrefix {
            label: label.to_string(),
            prefix: p.to_string(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
        }
    }

    #[test]
    fn builtins_are_valid_and_unique() {
        validate_prefixes(&builtin_prefixes()).unwrap();
    }

    #[test]
    fn resolution_prefers_repo_then_global_then_builtin() {
        let repo = vec![prefix("Release", "release/", &["release"])];
        let global = vec![prefix("Spike", "spike/", &["spike"])];

        assert_eq!(prefix_source(Some(&global), &repo), PrefixSource::Repo);
        assert_eq!(resolve_prefixes(Some(&global), &repo), repo);

        assert_eq!(prefix_source(None, &repo), PrefixSource::Repo);
        assert_eq!(resolve_prefixes(None, &repo), repo);

        assert_eq!(prefix_source(Some(&global), &[]), PrefixSource::Global);
        assert_eq!(resolve_prefixes(Some(&global), &[]), global);

        assert_eq!(prefix_source(None, &[]), PrefixSource::Builtin);
        assert_eq!(resolve_prefixes(None, &[]), builtin_prefixes());
    }

    #[test]
    fn source_agrees_with_resolution() {
        let repo = vec![prefix("Release", "release/", &["release"])];
        let global = vec![prefix("Spike", "spike/", &["spike"])];
        let empty: Vec<BranchPrefix> = vec![];

        let cases: [(Option<&[BranchPrefix]>, &[BranchPrefix]); 4] = [
            (Some(&global), &repo),
            (None, &repo),
            (Some(&global), &empty),
            (None, &empty),
        ];
        for (global, repo) in cases {
            let source = prefix_source(global, repo);
            let resolved = resolve_prefixes(global, repo);
            let expected = match source {
                PrefixSource::Repo => repo.to_vec(),
                PrefixSource::Global => global.unwrap().to_vec(),
                PrefixSource::Builtin => builtin_prefixes(),
            };
            assert_eq!(resolved, expected, "source={source:?}");
        }
    }

    #[test]
    fn empty_prefix_is_unset_not_invalid() {
        validate_prefix("").unwrap();
        validate_prefix("   ").unwrap();
    }

    #[test]
    fn validate_rejects_git_hostile_prefixes() {
        let cases = [
            "/fix/",
            "a//b/",
            "my feature/",
            "a..b/",
            "fix~/",
            "fix^/",
            "fix:/",
            "fix?/",
            "fix*/",
            "fix[/",
            "fix\\/",
            "fix/\u{7f}/",
            "./fix/",
            "fix/.hidden/",
            "fix.lock/",
            "fix/x.lock/",
            "-fix/",
            "-/",
            "fix@{/",
            "@{/",
            "a/b@{c/",
        ];
        for case in cases {
            let err = validate_prefix(case).unwrap_err();
            assert_eq!(err.code(), "invalid_branch_prefix", "for {case:?}");
        }
    }

    #[test]
    fn validate_accepts_the_shapes_users_want() {
        for case in [
            "fix/",
            "hotfix-",
            "jt/fix/",
            "release_1/",
            "fix/-x/",
            "my-fix/",
            "fix@x/",
            "fix@/",
            "fix/x.lock",
            "topic.lock",
        ] {
            validate_prefix(case).unwrap_or_else(|e| panic!("{case:?} should be valid: {e}"));
        }
    }

    #[test]
    fn validate_prefixes_rejects_empty_label_blank_prefix_and_duplicates() {
        let blank_label = vec![prefix("", "fix/", &[])];
        assert_eq!(
            validate_prefixes(&blank_label).unwrap_err().code(),
            "invalid_branch_prefix"
        );

        let blank_prefix = vec![prefix("Bug fix", "   ", &[])];
        assert_eq!(
            validate_prefixes(&blank_prefix).unwrap_err().code(),
            "invalid_branch_prefix"
        );

        let duplicate = vec![
            prefix("Bug fix", "fix/", &[]),
            prefix("Bugfix", "fix/", &[]),
        ];
        assert_eq!(
            validate_prefixes(&duplicate).unwrap_err().code(),
            "invalid_branch_prefix"
        );
    }

    #[test]
    fn validate_prefixes_compares_after_trimming() {
        let list = vec![prefix("Bug fix", " fix/ ", &[]), prefix("Fix", "fix/", &[])];
        assert_eq!(
            validate_prefixes(&list).unwrap_err().code(),
            "invalid_branch_prefix"
        );
    }

    #[test]
    fn keywords_fall_back_to_the_label_words() {
        let no_keywords = prefix("Bug fix", "fix/", &[]);
        assert_eq!(no_keywords.match_words(), vec!["bug", "fix"]);

        let with_keywords = prefix("Bug fix", "fix/", &["crash", "broken"]);
        assert_eq!(with_keywords.match_words(), vec!["crash", "broken"]);

        let filled = no_keywords.clone().with_effective_keywords();
        assert_eq!(filled.keywords, vec!["bug", "fix"]);
        let resolved = resolve_prefixes(None, &[no_keywords]);
        assert_eq!(resolved[0].keywords, Vec::<String>::new());
    }

    #[test]
    fn stored_keywords_are_not_split() {
        let entry = prefix("Login issue", "fix/", &["login crash"]);
        assert_eq!(entry.match_words(), vec!["login crash"]);
    }

    #[test]
    fn dash_keyword_never_suggests() {
        let entry = prefix("Nothing", "nothing/", &["-"]);
        assert_eq!(entry.match_words(), vec!["-"]);
        let filled = entry.with_effective_keywords();
        assert_eq!(filled.keywords, vec!["-"]);
        assert_eq!(filled.clone().with_effective_keywords(), filled);
    }
}
