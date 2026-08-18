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

/// Open/merged/closed, the PR's lifecycle state from `gh`'s `state` field.
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

/// GitHub's mergeability verdict for the PR, from `gh`'s `mergeable` field.
/// Distinguishes a clean merge from a conflicting branch, which we surface as a
/// row in the checks panel, and from GitHub's transient `UNKNOWN` (still being
/// computed server-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum MergeableState {
    /// No conflicts, the PR can merge cleanly.
    Mergeable,
    /// The branch conflicts with its base and must be resolved before merging.
    Conflicting,
    /// GitHub hasn't finished computing mergeability yet (transient).
    Unknown,
}

impl MergeableState {
    fn parse(raw: &str) -> Self {
        match raw {
            s if s.eq_ignore_ascii_case("MERGEABLE") => MergeableState::Mergeable,
            s if s.eq_ignore_ascii_case("CONFLICTING") => MergeableState::Conflicting,
            _ => MergeableState::Unknown,
        }
    }
}

/// GitHub's aggregate review verdict for the PR, from `gh`'s `reviewDecision`
/// field. The `Option` on [`PrStatus`] is `None` when the PR requires no review,
/// then there's nothing to gate on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ReviewDecision {
    /// A required review is satisfied, approved with no changes outstanding.
    Approved,
    /// A reviewer requested changes; the author must address them before merging.
    ChangesRequested,
    /// A required review is still outstanding, the PR is waiting on a reviewer.
    ReviewRequired,
}

impl ReviewDecision {
    /// `None` for an absent/empty decision (no review required) or any value we
    /// don't recognize, callers treat that as "nothing to gate on".
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            s if s.eq_ignore_ascii_case("APPROVED") => Some(ReviewDecision::Approved),
            s if s.eq_ignore_ascii_case("CHANGES_REQUESTED") => {
                Some(ReviewDecision::ChangesRequested)
            }
            s if s.eq_ignore_ascii_case("REVIEW_REQUIRED") => Some(ReviewDecision::ReviewRequired),
            _ => None,
        }
    }
}

/// How the Merge button merges, picked per-PR by the user.
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
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    /// Per-check outcome, only `Pending`/`Passing`/`Failing` (never `None`).
    pub state: ChecksState,
    /// Link to the check's run/details page, when GitHub provides one.
    pub url: Option<String>,
}

/// A lightweight open-PR row for the "Attach PR" picker: enough to display a
/// choice and attach it by number. From `gh pr list --json …`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrSummary {
    pub number: u32,
    pub title: String,
    /// The PR's source branch, shown as a subtitle so two PRs with similar
    /// titles are still distinguishable.
    pub head_ref_name: String,
    pub url: String,
    pub is_draft: bool,
}

/// A branch's pull request as the PR status bar renders it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub number: u32,
    pub url: String,
    pub title: String,
    pub state: PrState,
    /// GitHub reports the PR cleanly mergeable (no conflicts). True only for
    /// `MERGEABLE`; the Merge button gates on this.
    pub mergeable: bool,
    /// GitHub's full mergeability verdict. Carries the `CONFLICTING` distinction
    /// `mergeable` discards, so the checks panel can show a merge-conflict row.
    pub mergeable_state: MergeableState,
    pub checks: ChecksState,
    pub checks_total: u32,
    pub checks_passed: u32,
    pub checks_failed: u32,
    /// The individual checks, for the "which ran?" breakdown popover.
    pub checks_list: Vec<PrCheck>,
    /// GitHub's review verdict, or `None` when the PR requires no review / has no
    /// decision yet. The Merge button blocks while this is `ReviewRequired` or
    /// `ChangesRequested`.
    pub review_decision: Option<ReviewDecision>,
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
fn build_checks(rollup: Vec<RollupEntry>, pr_url: &str) -> Vec<PrCheck> {
    rollup
        .into_iter()
        .map(|e| {
            let state = classify_entry(&e);
            PrCheck {
                name: e.name.or(e.context).unwrap_or_else(|| "check".into()),
                state,
                url: check_url(pr_url, e.details_url.as_deref(), e.target_url.as_deref()),
            }
        })
        .collect()
}

/// Resolve one check's link. A GitHub Actions CheckRun gets a PR-anchored URL
/// (`…/pull/N/checks?check_run_id=…`) so the row opens that exact check inside
/// the PR's Checks tab rather than the bare Actions job page. Anything else
/// (third-party StatusContexts) keeps its own `targetUrl`, which points at the
/// external CI, the best link GitHub gives us for those.
fn check_url(pr_url: &str, details_url: Option<&str>, target_url: Option<&str>) -> Option<String> {
    if let Some(details) = details_url {
        return match check_run_id(details) {
            Some(id) => Some(format!("{pr_url}/checks?check_run_id={id}")),
            None => Some(details.to_string()),
        };
    }
    target_url.map(str::to_string)
}

/// The check-run id embedded as the `/job/<id>` segment of an Actions
/// `detailsUrl` (e.g. `…/actions/runs/27760243809/job/82132429491` → 82132429491,
/// which equals the run's `databaseId`, the id the PR Checks tab anchors on).
/// `None` for non-Actions URLs, which carry no such id.
fn check_run_id(details_url: &str) -> Option<u64> {
    let (_, rest) = details_url.rsplit_once("/job/")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Normalize one rollup entry to a per-check [`ChecksState`] across both shapes
/// (never yields `None`, that's reserved for "no checks at all").
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
    let checks_list = build_checks(raw.status_check_rollup, &raw.url);
    let (checks, checks_total, checks_passed, checks_failed) = summarize(&checks_list);
    let review_decision = raw
        .review_decision
        .as_deref()
        .and_then(ReviewDecision::parse);
    Ok(PrStatus {
        number: raw.number,
        url: raw.url,
        title: raw.title,
        state: parse_state(&raw.state),
        mergeable: raw.mergeable.eq_ignore_ascii_case("MERGEABLE"),
        mergeable_state: MergeableState::parse(&raw.mergeable),
        checks,
        checks_total,
        checks_passed,
        checks_failed,
        checks_list,
        review_decision,
    })
}

/// The PR identified by `pr_ref`, or `None` when there's no matching pull
/// request. `pr_ref` is whatever `gh pr view` accepts as its argument, a branch
/// name (the default: the branch's open PR), a PR number (a manually attached
/// PR), or a URL. Runs `gh pr view <pr_ref> --json …` in `worktree`. A missing
/// PR is the common, non-error case (`gh` exits non-zero with "no pull requests
/// found"), reported as `Ok(None)`; any other failure is a real error.
pub async fn pr_status(gh: &Path, worktree: &Path, pr_ref: &str) -> Result<Option<PrStatus>> {
    let out = Exec::new(gh)
        .cwd(worktree)
        .args([
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,url,title,state,mergeable,reviewDecision,statusCheckRollup",
        ])
        .timeout(Duration::from_secs(30))
        .run_unchecked()
        .await?;

    if out.exit_code == Some(0) {
        return parse_pr(&out.stdout).map(Some);
    }
    // No PR for this branch is expected (e.g. before the first ship), gh says
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

/// `headRefName` + `isDraft` over [`RawPr`]'s shared fields, for the picker rows
/// (`gh pr list` returns an array of these).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPrSummary {
    number: u32,
    title: String,
    head_ref_name: String,
    url: String,
    #[serde(default)]
    is_draft: bool,
}

impl From<RawPrSummary> for PrSummary {
    fn from(r: RawPrSummary) -> Self {
        PrSummary {
            number: r.number,
            title: r.title,
            head_ref_name: r.head_ref_name,
            url: r.url,
            is_draft: r.is_draft,
        }
    }
}

/// Up to `limit` open PRs in `dir`'s repo for the attach picker, newest first.
/// When `query` is non-empty it's passed to `gh pr list --search`, so the same
/// box does free-text filtering (GitHub search syntax). An unauthenticated or
/// repo-less directory yields an error the caller degrades to an empty list.
pub async fn list_open_prs(
    gh: &Path,
    dir: &Path,
    query: &str,
    limit: u32,
) -> Result<Vec<PrSummary>> {
    let limit = limit.to_string();
    let mut args = vec![
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        &limit,
        "--json",
        "number,title,headRefName,url,isDraft",
    ];
    let query = query.trim();
    if !query.is_empty() {
        args.push("--search");
        args.push(query);
    }
    let out = Exec::new(gh)
        .cwd(dir)
        .args(args)
        .timeout(Duration::from_secs(30))
        .run()
        .await?;
    parse_pr_list(&out.stdout)
}

/// Parse a `gh pr list --json` array into [`PrSummary`] rows. Split out from
/// [`list_open_prs`] so the mapping is unit-testable without `gh`.
fn parse_pr_list(stdout: &str) -> Result<Vec<PrSummary>> {
    let raw: Vec<RawPrSummary> =
        serde_json::from_str(stdout).map_err(|e| CoreError::OutputParse {
            program: "gh pr list".into(),
            detail: e.to_string(),
        })?;
    Ok(raw.into_iter().map(PrSummary::from).collect())
}

/// A one-line summary of the PR identified by `pr_ref` (number/branch/URL), or
/// `None` when no such PR exists, the attach picker's live preview when a user
/// types a number. Like [`pr_status`] but fetches only the fields the row shows.
/// A missing PR is the common, non-error case: gh reports either "no pull
/// requests found" (branch) or "Could not resolve to a PullRequest" (number).
pub async fn pr_summary(gh: &Path, dir: &Path, pr_ref: &str) -> Result<Option<PrSummary>> {
    let out = Exec::new(gh)
        .cwd(dir)
        .args([
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,title,headRefName,url,isDraft",
        ])
        .timeout(Duration::from_secs(30))
        .run_unchecked()
        .await?;

    if out.exit_code == Some(0) {
        let raw: RawPrSummary =
            serde_json::from_str(&out.stdout).map_err(|e| CoreError::OutputParse {
                program: "gh pr view".into(),
                detail: e.to_string(),
            })?;
        return Ok(Some(raw.into()));
    }
    let stderr = out.stderr.to_lowercase();
    if stderr.contains("no pull requests found") || stderr.contains("could not resolve") {
        return Ok(None);
    }
    Err(CoreError::Exec {
        program: "gh".into(),
        exit: out.exit_code,
        stderr_tail: out.stderr.trim().to_string(),
    })
}

/// Merge `pr_ref`'s PR with the chosen method (`pr_ref` is a branch, PR number,
/// or URL, see [`pr_status`]). Does not pass `--delete-branch`: gh would try to
/// delete the local branch checked out in this worktree (and switch branches),
/// which fails. Local branch + worktree teardown is the archive step's job; the
/// remote branch is dropped when a merged workstream is archived (see
/// `archive_workstream` / `delete_branch_on_archive`), independent of GitHub's
/// own "auto-delete head branches" setting.
///
/// When `auto` is set, passes `--auto` so GitHub queues the merge until
/// required checks pass (the repo must have auto-merge enabled). With `auto`
/// the call returns as soon as the merge is *queued*, not when it lands.
pub async fn merge_pr(
    gh: &Path,
    worktree: &Path,
    pr_ref: &str,
    method: MergeMethod,
    auto: bool,
) -> Result<()> {
    let mut args = vec!["pr", "merge", pr_ref, method.flag()];
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

/// Re-target an open PR's base branch (`gh pr edit <ref> --base <branch>`).
/// Run when a workstream's target branch changes after its PR is already open,
/// so GitHub merges it into the new base. `base` must exist on the remote.
pub async fn edit_pr_base(gh: &Path, worktree: &Path, pr_ref: &str, base: &str) -> Result<()> {
    // `gh pr edit --base` wants the GitHub branch name; the stored base may be a
    // git rev (e.g. `origin/main`), so strip the remote prefix.
    let base = base.strip_prefix("origin/").unwrap_or(base);
    Exec::new(gh)
        .cwd(worktree)
        .args(["pr", "edit", pr_ref, "--base", base])
        .timeout(Duration::from_secs(30))
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
    fn pr_list_parses_rows_and_defaults_draft() {
        // `isDraft` is omitted on the second row → defaults to false.
        let json = r#"[
            {"number": 12, "title": "Add thing", "headRefName": "feat/thing",
             "url": "https://x/12", "isDraft": true},
            {"number": 9, "title": "Fix bug", "headRefName": "fix/bug", "url": "https://x/9"}
        ]"#;
        let rows = parse_pr_list(json).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].number, 12);
        assert_eq!(rows[0].head_ref_name, "feat/thing");
        assert!(rows[0].is_draft);
        assert_eq!(rows[1].number, 9);
        assert!(!rows[1].is_draft);
    }

    #[test]
    fn pr_list_empty_array_is_empty() {
        assert!(parse_pr_list("[]").unwrap().is_empty());
    }

    #[test]
    fn pr_summary_maps_single_object_and_defaults_draft() {
        // `gh pr view --json` returns one object (not an array); `isDraft` omitted.
        let raw: RawPrSummary =
            serde_json::from_str(r#"{"number":98,"title":"t","headRefName":"b","url":"u"}"#)
                .unwrap();
        let s = PrSummary::from(raw);
        assert_eq!((s.number, s.head_ref_name.as_str()), (98, "b"));
        assert!(!s.is_draft);
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
        assert_eq!(pr.mergeable_state, MergeableState::Mergeable);
        assert_eq!(pr.review_decision, Some(ReviewDecision::Approved));
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
        // CONFLICTING is preserved distinctly so the checks panel can flag it,
        // rather than being flattened into the `mergeable` bool.
        assert_eq!(pr.mergeable_state, MergeableState::Conflicting);
        assert_eq!(pr.review_decision, None); // empty string → None (parse rejects it)

        // The per-check breakdown carries each check's name, state, and link,
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
    fn actions_check_links_into_pr_checks_tab() {
        let json = r#"{
            "number": 8, "url": "https://github.com/o/r/pull/8", "title": "t",
            "state": "OPEN", "mergeable": "MERGEABLE", "reviewDecision": "",
            "statusCheckRollup": [
                {"status": "COMPLETED", "conclusion": "SUCCESS", "name": "build",
                 "detailsUrl": "https://github.com/o/r/actions/runs/27760243809/job/82132429491"},
                {"state": "SUCCESS", "context": "ci/ext", "targetUrl": "https://ext.example/run/9"}
            ]
        }"#;
        let pr = parse_pr(json).unwrap();
        // The Actions CheckRun resolves to its run *inside the PR's Checks tab*,
        // not the bare Actions job page.
        assert_eq!(
            pr.checks_list[0].url.as_deref(),
            Some("https://github.com/o/r/pull/8/checks?check_run_id=82132429491")
        );
        // A third-party status context keeps its own external target URL.
        assert_eq!(
            pr.checks_list[1].url.as_deref(),
            Some("https://ext.example/run/9")
        );
    }

    #[test]
    fn check_run_id_reads_job_segment_only() {
        assert_eq!(
            check_run_id("https://github.com/o/r/actions/runs/1/job/42"),
            Some(42)
        );
        // No `/job/` segment (older/foreign shapes) → no id; caller falls back.
        assert_eq!(check_run_id("https://github.com/o/r/runs/42"), None);
        assert_eq!(check_run_id("https://ext.example/build"), None);
    }

    #[test]
    fn mergeable_state_parses_all_arms() {
        assert_eq!(
            MergeableState::parse("MERGEABLE"),
            MergeableState::Mergeable
        );
        assert_eq!(
            MergeableState::parse("conflicting"),
            MergeableState::Conflicting
        );
        // UNKNOWN and any unexpected value fall back to Unknown (transient).
        assert_eq!(MergeableState::parse("UNKNOWN"), MergeableState::Unknown);
        assert_eq!(MergeableState::parse(""), MergeableState::Unknown);
    }

    #[test]
    fn review_decision_parses_known_and_ignores_the_rest() {
        let mk = |rd: &str| {
            format!(
                r#"{{"number":1,"url":"u","title":"t","state":"OPEN","mergeable":"MERGEABLE","reviewDecision":"{rd}","statusCheckRollup":[]}}"#
            )
        };
        let rd = |raw: &str| parse_pr(&mk(raw)).unwrap().review_decision;
        assert_eq!(rd("REVIEW_REQUIRED"), Some(ReviewDecision::ReviewRequired));
        assert_eq!(
            rd("CHANGES_REQUESTED"),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(rd("APPROVED"), Some(ReviewDecision::Approved));
        // Empty (no required review) and any unrecognized value → None: nothing to
        // gate on, so the Merge button isn't blocked by a verdict we can't read.
        assert_eq!(rd(""), None);
        assert_eq!(rd("SOMETHING_NEW"), None);
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
