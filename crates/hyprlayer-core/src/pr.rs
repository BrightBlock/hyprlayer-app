use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::Exec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ChecksState {
    None,
    Pending,
    Passing,
    Failing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum MergeableState {
    Mergeable,
    Conflicting,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

impl ReviewDecision {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    pub state: ChecksState,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrSummary {
    pub number: u32,
    pub title: String,
    pub head_ref_name: String,
    pub url: String,
    pub is_draft: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub number: u32,
    pub url: String,
    pub title: String,
    pub state: PrState,
    pub mergeable: bool,
    pub mergeable_state: MergeableState,
    pub checks: ChecksState,
    pub checks_total: u32,
    pub checks_passed: u32,
    pub checks_failed: u32,
    pub checks_list: Vec<PrCheck>,
    pub review_decision: Option<ReviewDecision>,
}

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollupEntry {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    details_url: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
}

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

fn check_url(pr_url: &str, details_url: Option<&str>, target_url: Option<&str>) -> Option<String> {
    if let Some(details) = details_url {
        return match check_run_id(details) {
            Some(id) => Some(format!("{pr_url}/checks?check_run_id={id}")),
            None => Some(details.to_string()),
        };
    }
    target_url.map(str::to_string)
}

fn check_run_id(details_url: &str) -> Option<u64> {
    let (_, rest) = details_url.rsplit_once("/job/")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn classify_entry(entry: &RollupEntry) -> ChecksState {
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
            "" => ChecksState::Pending,
            _ => ChecksState::Failing,
        };
    }
    match entry.state.as_deref().unwrap_or("") {
        s if s.eq_ignore_ascii_case("SUCCESS") => ChecksState::Passing,
        s if s.eq_ignore_ascii_case("PENDING") || s.eq_ignore_ascii_case("EXPECTED") => {
            ChecksState::Pending
        }
        _ => ChecksState::Failing,
    }
}

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

pub async fn pr_status(
    gh: &Path,
    worktree: &Path,
    pr_ref: &str,
    overlay: &[(String, String)],
) -> Result<Option<PrStatus>> {
    let out = Exec::new(gh)
        .cwd(worktree)
        .args([
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,url,title,state,mergeable,reviewDecision,statusCheckRollup",
        ])
        .envs(overlay.iter().cloned())
        .timeout(Duration::from_secs(30))
        .run_unchecked()
        .await?;

    if out.exit_code == Some(0) {
        return parse_pr(&out.stdout).map(Some);
    }
    if out.stderr.to_lowercase().contains("no pull requests found") {
        return Ok(None);
    }
    Err(CoreError::Exec {
        program: "gh".into(),
        exit: out.exit_code,
        stderr_tail: out.stderr.trim().to_string(),
    })
}

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

pub async fn list_open_prs(
    gh: &Path,
    dir: &Path,
    query: &str,
    limit: u32,
    overlay: &[(String, String)],
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
        .envs(overlay.iter().cloned())
        .timeout(Duration::from_secs(30))
        .run()
        .await?;
    parse_pr_list(&out.stdout)
}

fn parse_pr_list(stdout: &str) -> Result<Vec<PrSummary>> {
    let raw: Vec<RawPrSummary> =
        serde_json::from_str(stdout).map_err(|e| CoreError::OutputParse {
            program: "gh pr list".into(),
            detail: e.to_string(),
        })?;
    Ok(raw.into_iter().map(PrSummary::from).collect())
}

pub async fn pr_summary(
    gh: &Path,
    dir: &Path,
    pr_ref: &str,
    overlay: &[(String, String)],
) -> Result<Option<PrSummary>> {
    let out = Exec::new(gh)
        .cwd(dir)
        .args([
            "pr",
            "view",
            pr_ref,
            "--json",
            "number,title,headRefName,url,isDraft",
        ])
        .envs(overlay.iter().cloned())
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

pub async fn merge_pr(
    gh: &Path,
    worktree: &Path,
    pr_ref: &str,
    method: MergeMethod,
    auto: bool,
    overlay: &[(String, String)],
) -> Result<()> {
    let mut args = vec!["pr", "merge", pr_ref, method.flag()];
    if auto {
        args.push("--auto");
    }
    Exec::new(gh)
        .cwd(worktree)
        .args(args)
        .envs(overlay.iter().cloned())
        .timeout(Duration::from_secs(60))
        .run()
        .await?;
    Ok(())
}

pub async fn edit_pr_base(
    gh: &Path,
    worktree: &Path,
    pr_ref: &str,
    base: &str,
    overlay: &[(String, String)],
) -> Result<()> {
    let base = base.strip_prefix("origin/").unwrap_or(base);
    Exec::new(gh)
        .cwd(worktree)
        .args(["pr", "edit", pr_ref, "--base", base])
        .envs(overlay.iter().cloned())
        .timeout(Duration::from_secs(30))
        .run()
        .await?;
    Ok(())
}

pub async fn github_login(gh: &Path, overlay: &[(String, String)]) -> Option<String> {
    let out = Exec::new(gh)
        .args(["api", "user", "--jq", ".login"])
        .envs(overlay.iter().cloned())
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
        let raw: RawPrSummary =
            serde_json::from_str(r#"{"number":98,"title":"t","headRefName":"b","url":"u"}"#)
                .unwrap();
        let s = PrSummary::from(raw);
        assert_eq!((s.number, s.head_ref_name.as_str()), (98, "b"));
        assert!(!s.is_draft);
    }

    #[test]
    fn pending_dominates_failure() {
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
        assert_eq!(pr.mergeable_state, MergeableState::Conflicting);
        assert_eq!(pr.review_decision, None);

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
        assert_eq!(
            pr.checks_list[0].url.as_deref(),
            Some("https://github.com/o/r/pull/8/checks?check_run_id=82132429491")
        );
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
