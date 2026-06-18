//! Pull-request status + merge via the GitHub CLI.
//!
//! PR state is derived live from `gh` rather than stored: `gh` is already a
//! managed, authenticated binary (see [`crate::process::binaries`]), and a
//! branch's PR (its checks, mergeability, review state) changes server-side
//! independently of anything we persist. Every call runs `gh` in the
//! workstream's worktree so it resolves the repo + remote without extra flags.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::Exec;

/// Open/merged/closed — the PR's lifecycle state from `gh`'s `state` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

/// The rolled-up state of the PR's CI checks, derived from `statusCheckRollup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ChecksState {
    /// No checks configured/reported for the head commit.
    None,
    /// At least one check still queued or running.
    Pending,
    /// Every check completed successfully (or neutral/skipped).
    Passing,
    /// At least one check failed, errored, or timed out.
    Failing,
}

/// How the Merge button merges — picked per-PR by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

impl MergeMethod {
    fn flag(self) -> &'static str {
        match self {
            MergeMethod::Squash => "--squash",
            MergeMethod::Merge => "--merge",
            MergeMethod::Rebase => "--rebase",
        }
    }
}

/// One CI check on the PR's head commit, for the per-check breakdown popover.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    /// Per-check outcome — only `Pending`/`Passing`/`Failing` (never `None`).
    pub state: ChecksState,
    /// Link to the check's run/details page, when GitHub provides one.
    pub url: Option<String>,
}

/// A branch's pull request as the PR status bar renders it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub number: u32,
    pub url: String,
    pub title: String,
    pub state: PrState,
    /// GitHub reports the PR cleanly mergeable (no conflicts).
    pub mergeable: bool,
    pub checks: ChecksState,
    pub checks_total: u32,
    pub checks_passed: u32,
    pub checks_failed: u32,
    /// The individual checks, for the "which ran?" breakdown popover.
    pub checks_list: Vec<PrCheck>,
    /// `APPROVED` | `CHANGES_REQUESTED` | `REVIEW_REQUIRED`, or None when the PR
    /// requires no review / has no decision yet.
    pub review_decision: Option<String>,
}

/// The `gh pr view --json` fields we request. Mirrors [`PrStatus`] plus the raw
/// `statusCheckRollup` we fold into [`ChecksState`].
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u32,
    url: String,
    title: String,
    state: String,
    mergeable: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    status_check_rollup: Vec<RollupEntry>,
}

/// One element of `statusCheckRollup`. GitHub returns two shapes: a `CheckRun`
/// (Actions-style, with `status` + `conclusion`) or a `StatusContext` (legacy
/// commit statuses, with a single `state`). We accept both via flattened
/// optionals and normalize in [`classify_entry`].
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollupEntry {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
    /// CheckRun's display name.
    #[serde(default)]
    name: Option<String>,
    /// StatusContext's name lives in `context` instead.
    #[serde(default)]
    context: Option<String>,
    /// CheckRun's run page.
    #[serde(default)]
    details_url: Option<String>,
    /// StatusContext's link.
    #[serde(default)]
    target_url: Option<String>,
}

/// The per-check breakdown: name + outcome + link for each rollup entry, in the
/// order GitHub returns them. Consumes the rollup so the entry strings move into
/// the result rather than being cloned.
fn build_checks(rollup: Vec<RollupEntry>) -> Vec<PrCheck> {
    rollup
        .into_iter()
        .map(|e| {
            let state = classify_entry(&e);
            PrCheck {
                name: e.name.or(e.context).unwrap_or_else(|| "check".into()),
                state,
                url: e.details_url.or(e.target_url),
            }
        })
        .collect()
}

/// Normalize one rollup entry to a per-check [`ChecksState`] across both shapes
/// (never yields `None` — that's reserved for "no checks at all").
fn classify_entry(entry: &RollupEntry) -> ChecksState {
    // CheckRun: not COMPLETED → still running; else read `conclusion`.
    if let Some(status) = entry.status.as_deref() {
        if !status.eq_ignore_ascii_case("COMPLETED") {
            return ChecksState::Pending;
        }
        return match entry.conclusion.as_deref().unwrap_or("") {
            c if c.eq_ignore_ascii_case("SUCCESS")
                || c.eq_ignore_ascii_case("NEUTRAL")
                || c.eq_ignore_ascii_case("SKIPPED") =>
            {
                ChecksState::Passing
            }
            "" => ChecksState::Pending, // completed but no conclusion yet
            _ => ChecksState::Failing,  // FAILURE / TIMED_OUT / CANCELLED / ACTION_REQUIRED
        };
    }
    // StatusContext: a single `state` field.
    match entry.state.as_deref().unwrap_or("") {
        s if s.eq_ignore_ascii_case("SUCCESS") => ChecksState::Passing,
        s if s.eq_ignore_ascii_case("PENDING") || s.eq_ignore_ascii_case("EXPECTED") => {
            ChecksState::Pending
        }
        _ => ChecksState::Failing, // FAILURE / ERROR
    }
}

/// Fold the per-check states into an overall [`ChecksState`] + counts. Pending
/// dominates (we're still waiting), then failing, then passing; empty → None.
fn summarize(checks: &[PrCheck]) -> (ChecksState, u32, u32, u32) {
    let count = |s| checks.iter().filter(|c| c.state == s).count() as u32;
    let (passed, failed) = (count(ChecksState::Passing), count(ChecksState::Failing));
    let state = if checks.is_empty() {
        ChecksState::None
    } else if checks.iter().any(|c| c.state == ChecksState::Pending) {
        ChecksState::Pending
    } else if failed > 0 {
        ChecksState::Failing
    } else {
        ChecksState::Passing
    };
    (state, checks.len() as u32, passed, failed)
}

fn parse_state(raw: &str) -> PrState {
    match raw {
        s if s.eq_ignore_ascii_case("MERGED") => PrState::Merged,
        s if s.eq_ignore_ascii_case("CLOSED") => PrState::Closed,
        _ => PrState::Open,
    }
}

/// Parse a `gh pr view --json` payload into a [`PrStatus`]. Split out from
/// [`pr_status`] so the rollup classification is unit-testable without `gh`.
fn parse_pr(stdout: &str) -> Result<PrStatus> {
    let raw: RawPr = serde_json::from_str(stdout).map_err(|e| CoreError::OutputParse {
        program: "gh pr view".into(),
        detail: e.to_string(),
    })?;
    let checks_list = build_checks(raw.status_check_rollup);
    let (checks, checks_total, checks_passed, checks_failed) = summarize(&checks_list);
    let review_decision = raw.review_decision.filter(|s| !s.is_empty());
    Ok(PrStatus {
        number: raw.number,
        url: raw.url,
        title: raw.title,
        state: parse_state(&raw.state),
        mergeable: raw.mergeable.eq_ignore_ascii_case("MERGEABLE"),
        checks,
        checks_total,
        checks_passed,
        checks_failed,
        checks_list,
        review_decision,
    })
}

/// The PR for `branch`, or `None` when the branch has no pull request. Runs
/// `gh pr view <branch> --json …` in `worktree`. A missing PR is the common,
/// non-error case (`gh` exits non-zero with "no pull requests found"), reported
/// as `Ok(None)`; any other failure is a real error.
pub async fn pr_status(gh: &Path, worktree: &Path, branch: &str) -> Result<Option<PrStatus>> {
    let out = Exec::new(gh)
        .cwd(worktree)
        .args([
            "pr",
            "view",
            branch,
            "--json",
            "number,url,title,state,mergeable,reviewDecision,statusCheckRollup",
        ])
        .timeout(Duration::from_secs(30))
        .run_unchecked()
        .await?;

    if out.exit_code == Some(0) {
        return parse_pr(&out.stdout).map(Some);
    }
    // No PR for this branch is expected (e.g. before the first ship) — gh says
    // "no pull requests found for branch". Treat only that as None.
    if out.stderr.to_lowercase().contains("no pull requests found") {
        return Ok(None);
    }
    Err(CoreError::Exec {
        program: "gh".into(),
        exit: out.exit_code,
        stderr_tail: out.stderr.trim().to_string(),
    })
}

/// Merge `branch`'s PR with the chosen method. Does not pass `--delete-branch`:
/// gh would try to delete the local branch checked out in this worktree (and
/// switch branches), which fails. Local branch + worktree teardown is the
/// archive step's job; GitHub auto-deletes the remote branch if so configured.
///
/// When `auto` is set, passes `--auto` so GitHub queues the merge until
/// required checks pass (the repo must have auto-merge enabled). With `auto`
/// the call returns as soon as the merge is *queued*, not when it lands.
pub async fn merge_pr(
    gh: &Path,
    worktree: &Path,
    branch: &str,
    method: MergeMethod,
    auto: bool,
) -> Result<()> {
    let mut args = vec!["pr", "merge", branch, method.flag()];
    if auto {
        args.push("--auto");
    }
    Exec::new(gh)
        .cwd(worktree)
        .args(args)
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

/// The authenticated GitHub user's login (`gh api user --jq .login`), or
/// `None` when gh is unavailable / not logged in. Used to offer a
/// "username/" branch prefix in settings; failure is non-fatal.
pub async fn github_login(gh: &Path) -> Option<String> {
    let out = Exec::new(gh)
        .args(["api", "user", "--jq", ".login"])
        .timeout(Duration::from_secs(20))
        .run_unchecked()
        .await
        .ok()?;
    if out.exit_code != Some(0) {
        return None;
    }
    let login = out.stdout.trim();
    (!login.is_empty()).then(|| login.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rollup_is_none() {
        let (state, total, passed, failed) = summarize(&[]);
        assert_eq!(state, ChecksState::None);
        assert_eq!((total, passed, failed), (0, 0, 0));
    }

    #[test]
    fn pending_dominates_failure() {
        // A still-running check plus a failed one => overall Pending (we wait).
        let json = r#"{
            "number": 7, "url": "https://x/7", "title": "t",
            "state": "OPEN", "mergeable": "MERGEABLE", "reviewDecision": "",
            "statusCheckRollup": [
                {"status": "IN_PROGRESS"},
                {"status": "COMPLETED", "conclusion": "FAILURE"}
            ]
        }"#;
        let pr = parse_pr(json).unwrap();
        assert_eq!(pr.checks, ChecksState::Pending);
        assert_eq!((pr.checks_total, pr.checks_failed), (2, 1));
    }

    #[test]
    fn all_success_passes_and_neutral_counts_as_pass() {
        let json = r#"{
            "number": 1, "url": "u", "title": "t",
            "state": "OPEN", "mergeable": "MERGEABLE", "reviewDecision": "APPROVED",
            "statusCheckRollup": [
                {"status": "COMPLETED", "conclusion": "SUCCESS"},
                {"status": "COMPLETED", "conclusion": "SKIPPED"},
                {"state": "SUCCESS"}
            ]
        }"#;
        let pr = parse_pr(json).unwrap();
        assert_eq!(pr.checks, ChecksState::Passing);
        assert_eq!((pr.checks_total, pr.checks_passed), (3, 3));
        assert!(pr.mergeable);
        assert_eq!(pr.review_decision.as_deref(), Some("APPROVED"));
    }

    #[test]
    fn failure_without_pending_is_failing() {
        let json = r#"{
            "number": 2, "url": "u", "title": "t",
            "state": "OPEN", "mergeable": "CONFLICTING", "reviewDecision": "",
            "statusCheckRollup": [
                {"status": "COMPLETED", "conclusion": "SUCCESS", "name": "build", "detailsUrl": "https://x/run/1"},
                {"state": "ERROR", "context": "coverage", "targetUrl": "https://x/cov"}
            ]
        }"#;
        let pr = parse_pr(json).unwrap();
        assert_eq!(pr.checks, ChecksState::Failing);
        assert_eq!((pr.checks_passed, pr.checks_failed), (1, 1));
        assert!(!pr.mergeable);
        assert_eq!(pr.review_decision, None); // empty string filtered out

        // The per-check breakdown carries each check's name, state, and link —
        // from both the CheckRun (name/detailsUrl) and StatusContext
        // (context/targetUrl) shapes.
        assert_eq!(pr.checks_list.len(), 2);
        assert_eq!(pr.checks_list[0].name, "build");
        assert_eq!(pr.checks_list[0].state, ChecksState::Passing);
        assert_eq!(pr.checks_list[0].url.as_deref(), Some("https://x/run/1"));
        assert_eq!(pr.checks_list[1].name, "coverage");
        assert_eq!(pr.checks_list[1].state, ChecksState::Failing);
        assert_eq!(pr.checks_list[1].url.as_deref(), Some("https://x/cov"));
    }

    #[test]
    fn state_and_method_parse() {
        assert_eq!(parse_state("MERGED"), PrState::Merged);
        assert_eq!(parse_state("closed"), PrState::Closed);
        assert_eq!(parse_state("OPEN"), PrState::Open);
        assert_eq!(parse_state("anything"), PrState::Open);
        assert_eq!(MergeMethod::Squash.flag(), "--squash");
        assert_eq!(MergeMethod::Rebase.flag(), "--rebase");
    }
}
