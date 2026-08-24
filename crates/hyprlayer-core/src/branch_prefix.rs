//! Branch types offered at workstream creation ("Bug fix" → `fix/`). Pure
//! vocabulary: no I/O, no settings access. The three levels this resolves
//! between are owned by their own modules (built-ins here, the global list in
//! `src-tauri/src/settings.rs`, the per-repo list in `crate::repo_settings`),
//! and assembled by `src-tauri/src/commands/workstreams.rs`.
//!
//! A picked type REPLACES the global prefix rather than composing with it:
//! `fix/add-login-a1b2`, never `jt/fix/add-login-a1b2`.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BranchPrefix {
    /// What the picker shows ("Bug fix").
    pub label: String,
    /// Used verbatim before the slug — `manager.rs:169` injects no separator,
    /// so `hotfix-` is as valid as `hotfix/`.
    pub prefix: String,
    /// Lowercase single words that suggest this type from a title. Empty means
    /// "derive from the label" (see [`BranchPrefix::match_words`]).
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// Which level of the chain a resolved list came from, so the editors can say
/// what a surface is inheriting instead of leaving an empty list ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PrefixSource {
    Builtin,
    Global,
    Repo,
}

/// The set that applies with zero configuration. `&str` triples rather than
/// `BranchPrefix` values because the record owns `String`s and const-eval
/// cannot build those; [`builtin_prefixes`] is the constructor.
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

/// Owned records built from [`BUILTIN_PREFIXES`], since a `const` cannot hold
/// the `String`s [`BranchPrefix`] needs.
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
    /// The words a title is matched against.
    ///
    /// The split rule is asymmetric, deliberately: **stored keywords are
    /// lowercased and trimmed but never split**; only the label fallback is
    /// split on non-alphanumerics. Splitting stored keywords too would let
    /// `"login crash"` silently become two independent matchers, which
    /// contradicts the whole-word contract `suggestPrefix`
    /// (`src/lib/domain/branchPrefix.ts`) advertises. Keeping the rule here
    /// means one keyword is always one matcher.
    ///
    /// So: non-empty `keywords` → each entry lowercased, trimmed, empties
    /// dropped, de-duplicated. Empty `keywords` → the label lowercased and split
    /// on non-alphanumerics.
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

    /// Self with `keywords` replaced by [`match_words`](Self::match_words).
    /// Applied at the IPC boundary so the frontend matcher never has to know
    /// about the fallback; deliberately NOT applied by `resolve_prefixes`,
    /// which must stay faithful to what is stored so an editor round-trip
    /// cannot silently promote an implicit fallback into explicit saved data.
    ///
    /// Consequence, decided rather than discovered: because empty means "use the
    /// label's words", there is no way to spell "never suggest this type" by
    /// clearing the field — clearing it falls back to the label. The single
    /// keyword `-` is the opt-out. It works not because [`match_words`](Self::match_words)
    /// drops it — under the rule above a stored keyword is never split, so `-`
    /// survives as the literal matcher `"-"` — but because `suggestPrefix` splits
    /// the *title* on non-alphanumerics, so no title word can ever equal `"-"`.
    /// The entry keeps exactly one matcher, and that matcher is unreachable. The
    /// editor's placeholder says so. Without this, a user who clears the box to
    /// stop a false suggestion sees nothing change.
    pub fn with_effective_keywords(self) -> Self {
        let keywords = self.match_words();
        Self { keywords, ..self }
    }
}

/// Which level wins: a non-empty per-repo list, else a non-empty global list,
/// else the built-ins. Wholesale replace, never a merge (records have no
/// identity key to merge on).
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

/// App-side rules for a branch prefix, checked before it can reach
/// `git worktree add -b <prefix><slug>`. A subset of `git check-ref-format`,
/// applied in-process: shelling out for a string check would cost a subprocess
/// per keystroke-commit and still need this message rewritten for the UI.
///
/// Empty after trimming is NOT an error — it means "unset", and callers
/// normalize it away the same way `GitSettings::effective_branch_prefix`
/// (`src-tauri/src/settings.rs:452-460`) treats an empty string as unset.
///
/// No `/` is appended. `manager.rs:169` uses the prefix verbatim, and
/// `hotfix-` is a legitimate thing to want.
///
/// Checks run against the *trimmed* content: incidental leading/trailing
/// whitespace is not part of the prefix that will actually be stored (every
/// write path trims before persisting, see [`validate_prefixes`]), so it is
/// not grounds to reject the value here.
///
/// One alternation, one pass: `Regex::captures` returns the leftmost match,
/// and exactly one named group can be `Some` since the branches are mutually
/// exclusive alternatives, not independent tests. Which group fires (rather
/// than a fixed check order) decides the message when a prefix trips more
/// than one rule — acceptable because callers branch on [`CoreError::code`],
/// not on the detail text.
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

/// A subset of `git check-ref-format`, plus two app-specific rules
/// (`lead_dash`, `lock_segment`) `check-ref-format` itself does not enforce.
/// Compiled once: this runs per keystroke-commit in the settings editor, and
/// shelling out to `git check-ref-format` for a string check would cost a
/// subprocess per keystroke and still need every message rewritten for the
/// UI.
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

/// A whole list, as an editor saves it. Rules, each with its own message:
///
/// - every `prefix` passes [`validate_prefix`];
/// - no blank `label`;
/// - **no blank `prefix`** — note this is deliberately STRICTER than
///   [`validate_prefix`], which treats blank as "unset". Inside a list a blank
///   prefix is not "unset", it is an entry that would collide with the Type
///   picker's own `value: ""` sentinel for the None row (`DropdownSelect` keys
///   options by `` `${group ?? ""}${value}` ``, `src/components/ui.tsx:387`),
///   producing a duplicate React key and an unselectable option;
/// - no two entries sharing a `prefix`, for the same reason.
///
/// `label` and `prefix` are compared after trimming. Trimming itself is the
/// *caller's* job, not this function's: both write paths normalize before
/// storing and hand the normalized list back, so what the editor sends and what
/// it reads back are the same bytes. `set_worktree_files`
/// (`src-tauri/src/commands/projects.rs:1100-1116`) is the precedent for exactly
/// that shape — it trims, drops empties, and returns `Ok(saved)`.
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

        // repo present, global present → repo wins.
        assert_eq!(prefix_source(Some(&global), &repo), PrefixSource::Repo);
        assert_eq!(resolve_prefixes(Some(&global), &repo), repo);

        // repo present, global absent → repo still wins.
        assert_eq!(prefix_source(None, &repo), PrefixSource::Repo);
        assert_eq!(resolve_prefixes(None, &repo), repo);

        // repo absent, global present → global wins.
        assert_eq!(prefix_source(Some(&global), &[]), PrefixSource::Global);
        assert_eq!(resolve_prefixes(Some(&global), &[]), global);

        // repo absent, global absent → built-ins.
        assert_eq!(prefix_source(None, &[]), PrefixSource::Builtin);
        assert_eq!(resolve_prefixes(None, &[]), builtin_prefixes());
    }

    /// The anti-drift device for the pair: whatever level `prefix_source`
    /// names, `resolve_prefixes` must have pulled its list from there too.
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
            // `git branch` re-parses a leading '-' as a switch bundle even
            // though `check-ref-format` accepts it.
            "-fix/",
            "-/",
            // The reflog selector, rejected by `check-ref-format` anywhere.
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
        // The near-misses of the two rules above are all legal: a '-' that is
        // not the first character (git creates `fix/-x-slug` fine), and an '@'
        // that is not followed by '{'.
        for case in [
            "fix/",
            "hotfix-",
            "jt/fix/",
            "release_1/",
            "fix/-x/",
            "my-fix/",
            "fix@x/",
            "fix@/",
            // The slug finishes this component, so it is never really `.lock`.
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

        // Blank alone is "unset" to `validate_prefix`, but a list entry with a
        // blank prefix would collide with the branch-type picker's own `""` sentinel
        // for "None" — stricter here on purpose.
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

        // `with_effective_keywords` fills the fallback in...
        let filled = no_keywords.clone().with_effective_keywords();
        assert_eq!(filled.keywords, vec!["bug", "fix"]);
        // ...but `resolve_prefixes` never applies it: the stored (empty) list
        // survives resolution untouched.
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
        // Idempotent: filling again changes nothing.
        assert_eq!(filled.clone().with_effective_keywords(), filled);
    }
}
