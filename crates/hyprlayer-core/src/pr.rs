use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::process::Exec;

pub use hyprlayer_models::workstream::PrState;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum CheckBucket {
    Pass,
    Fail,
    Pending,
    Skipping,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    pub bucket: CheckBucket,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrChecks {
    pub state: ChecksState,
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
    pub skipped: u32,
    pub cancelled: u32,
    pub list: Vec<PrCheck>,
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
    pub checks: PrChecks,
    pub review_decision: Option<ReviewDecision>,
}

macro_rules! pr_fragment {
    () => {
        "fragment Pr on PullRequest {
  number url title state mergeable reviewDecision headRefName isCrossRepository
  commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes { __typename
      ... on CheckRun { name status conclusion startedAt detailsUrl
                        checkSuite { workflowRun { event workflow { name } } } }
      ... on StatusContext { context state startedAt: createdAt targetUrl } } } } } } }
}
"
    };
}

const BY_NUMBER: &str = concat!(
    pr_fragment!(),
    "query ByNumber($owner: String!, $repo: String!, $number: Int!, $after: String) {
  rateLimit { limit remaining resetAt }
  repository(owner: $owner, name: $repo) { pullRequest(number: $number) { ...Pr } }
}
"
);

const BY_BRANCH: &str = concat!(
    pr_fragment!(),
    "query ByBranch($owner: String!, $repo: String!, $branch: String!, $after: String) {
  rateLimit { limit remaining resetAt }
  repository(owner: $owner, name: $repo) {
    defaultBranchRef { name }
    pullRequests(headRefName: $branch, first: 30, orderBy: { field: CREATED_AT, direction: DESC }) { nodes { ...Pr } }
  }
}
"
);

#[derive(Deserialize)]
struct GqlBody {
    #[serde(default)]
    data: Option<serde_json::Value>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Deserialize)]
struct Nodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Nodes { nodes: Vec::new() }
    }
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct ByNumberData {
    repository: Option<RawRepoByNumber>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepoByNumber {
    pull_request: Option<RawPr>,
}

#[derive(Deserialize)]
struct ByBranchData {
    repository: RawRepoByBranch,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepoByBranch {
    #[serde(default)]
    default_branch_ref: Option<Named>,
    pull_requests: Nodes<RawPr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u32,
    url: String,
    title: String,
    state: String,
    #[serde(default)]
    mergeable: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    commits: Nodes<RawCommitNode>,
}

#[derive(Deserialize)]
struct RawCommitNode {
    commit: RawCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommit {
    #[serde(default)]
    status_check_rollup: Option<RawRollup>,
}

#[derive(Deserialize)]
struct RawRollup {
    contexts: RawContexts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawContexts {
    #[serde(default)]
    page_info: PageInfo,
    #[serde(default)]
    nodes: Vec<RollupEntry>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    #[serde(default)]
    has_next_page: bool,
    #[serde(default)]
    end_cursor: Option<String>,
}

impl RawPr {
    fn contexts(&mut self) -> Option<&mut RawContexts> {
        let commit = &mut self.commits.nodes.first_mut()?.commit;
        commit.status_check_rollup.as_mut().map(|r| &mut r.contexts)
    }

    fn next_page(&mut self) -> Option<String> {
        let info = &self.contexts()?.page_info;
        info.has_next_page
            .then(|| info.end_cursor.clone())
            .flatten()
    }

    fn append_page(&mut self, mut page: Option<RawPr>) {
        let from = page.as_mut().and_then(RawPr::contexts);
        let Some(into) = self.contexts() else { return };
        match from {
            Some(from) => {
                into.nodes.append(&mut from.nodes);
                into.page_info = std::mem::take(&mut from.page_info);
            }
            None => into.page_info = PageInfo::default(),
        }
    }
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
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    check_suite: Option<RawSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSuite {
    #[serde(default)]
    workflow_run: Option<RawRun>,
}

#[derive(Deserialize)]
struct RawRun {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    workflow: Option<Named>,
}

#[derive(PartialEq, Eq, Hash)]
enum CheckKey {
    Status(String),
    Run {
        name: String,
        workflow: String,
        event: String,
    },
}

impl RollupEntry {
    fn key(&self) -> CheckKey {
        if let Some(context) = &self.context {
            return CheckKey::Status(context.clone());
        }
        let run = self
            .check_suite
            .as_ref()
            .and_then(|s| s.workflow_run.as_ref());
        CheckKey::Run {
            name: self.name.clone().unwrap_or_default(),
            workflow: run
                .and_then(|r| r.workflow.as_ref())
                .map(|w| w.name.clone())
                .unwrap_or_default(),
            event: run.and_then(|r| r.event.clone()).unwrap_or_default(),
        }
    }
}

fn latest_runs(mut rollup: Vec<RollupEntry>) -> Vec<RollupEntry> {
    rollup.sort_by_cached_key(|e| {
        (
            std::cmp::Reverse(e.started_at.clone()),
            bucket(e) != CheckBucket::Fail,
        )
    });
    let mut seen = HashSet::new();
    rollup.retain(|e| seen.insert(e.key()));
    rollup
}

fn bucket(e: &RollupEntry) -> CheckBucket {
    let raw = match (&e.state, &e.status) {
        (Some(state), _) => state.as_str(),
        (None, Some(status)) if status.eq_ignore_ascii_case("COMPLETED") => {
            e.conclusion.as_deref().unwrap_or("")
        }
        (None, status) => status.as_deref().unwrap_or(""),
    };
    match raw.to_ascii_uppercase().as_str() {
        "SUCCESS" => CheckBucket::Pass,
        "SKIPPED" | "NEUTRAL" => CheckBucket::Skipping,
        "ERROR" | "FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" => CheckBucket::Fail,
        "CANCELLED" => CheckBucket::Cancel,
        _ => CheckBucket::Pending,
    }
}

fn build_checks(rollup: Vec<RollupEntry>, pr_url: &str) -> Vec<PrCheck> {
    rollup
        .into_iter()
        .map(|e| {
            let bucket = bucket(&e);
            PrCheck {
                name: e.name.or(e.context).unwrap_or_else(|| "check".into()),
                bucket,
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

fn summarize(list: Vec<PrCheck>) -> PrChecks {
    let count = |b| list.iter().filter(|c| c.bucket == b).count() as u32;
    let (passed, failed, pending) = (
        count(CheckBucket::Pass),
        count(CheckBucket::Fail),
        count(CheckBucket::Pending),
    );
    let (skipped, cancelled) = (count(CheckBucket::Skipping), count(CheckBucket::Cancel));
    let state = if list.is_empty() {
        ChecksState::None
    } else if failed > 0 {
        ChecksState::Failing
    } else if pending > 0 {
        ChecksState::Pending
    } else {
        ChecksState::Passing
    };
    PrChecks {
        state,
        total: list.len() as u32,
        passed,
        failed,
        pending,
        skipped,
        cancelled,
        list,
    }
}

fn parse_state(raw: &str) -> PrState {
    match raw {
        s if s.eq_ignore_ascii_case("MERGED") => PrState::Merged,
        s if s.eq_ignore_ascii_case("CLOSED") => PrState::Closed,
        _ => PrState::Open,
    }
}

fn parse_pr(mut raw: RawPr) -> PrStatus {
    let contexts = raw
        .contexts()
        .map(|c| std::mem::take(&mut c.nodes))
        .unwrap_or_default();
    let checks = summarize(build_checks(latest_runs(contexts), &raw.url));
    let review_decision = raw
        .review_decision
        .as_deref()
        .and_then(ReviewDecision::parse);
    PrStatus {
        number: raw.number,
        state: parse_state(&raw.state),
        mergeable: raw.mergeable.eq_ignore_ascii_case("MERGEABLE"),
        mergeable_state: MergeableState::parse(&raw.mergeable),
        checks,
        review_decision,
        url: raw.url,
        title: raw.title,
    }
}

fn graphql_parse_error(e: serde_json::Error) -> CoreError {
    CoreError::OutputParse {
        program: "gh api graphql".into(),
        detail: e.to_string(),
    }
}

fn graphql_data<T: DeserializeOwned>(stdout: &str) -> Result<T> {
    let body: GqlBody = serde_json::from_str(stdout).map_err(graphql_parse_error)?;
    if !body.errors.is_empty() {
        let messages: Vec<_> = body.errors.into_iter().map(|e| e.message).collect();
        return Err(CoreError::Exec {
            program: "gh".into(),
            exit: Some(0),
            stderr_tail: messages.join("; "),
        });
    }
    serde_json::from_value(body.data.unwrap_or_default()).map_err(graphql_parse_error)
}

fn pr_by_number(stdout: &str) -> Result<Option<RawPr>> {
    let data: ByNumberData = graphql_data(stdout)?;
    Ok(data.repository.and_then(|r| r.pull_request))
}

fn pr_for_branch(stdout: &str, branch: &str) -> Result<Option<RawPr>> {
    let data: ByBranchData = graphql_data(stdout)?;
    Ok(pick_for_branch(data.repository, branch))
}

fn pick_for_branch(repo: RawRepoByBranch, branch: &str) -> Option<RawPr> {
    let default = repo.default_branch_ref.map(|r| r.name);
    let mut prs = repo.pull_requests.nodes;
    prs.sort_by_key(|p| !p.state.eq_ignore_ascii_case("OPEN"));
    prs.into_iter().find(|p| {
        !p.is_cross_repository
            && p.head_ref_name == branch
            && (p.state.eq_ignore_ascii_case("OPEN") || default.as_deref() != Some(branch))
    })
}

async fn graphql(
    gh: &Path,
    dir: &Path,
    query: &str,
    vars: &[(&str, String)],
    overlay: &[(String, String)],
) -> Result<String> {
    let mut exec = Exec::new(gh)
        .cwd(dir)
        .args([
            "api",
            "graphql",
            "-F",
            "owner={owner}",
            "-F",
            "repo={repo}",
            "-f",
        ])
        .arg(format!("query={query}"));
    if let Some(host) = crate::gitops::gh_api_hostname(dir) {
        exec = exec.arg("--hostname").arg(host);
    }
    for (flag, var) in vars {
        exec = exec.arg(*flag).arg(var);
    }
    let out = exec
        .envs(overlay.iter().cloned())
        .timeout(Duration::from_secs(30))
        .run_unchecked()
        .await?;
    if out.exit_code == Some(0) {
        return Ok(out.stdout);
    }
    Err(CoreError::Exec {
        program: "gh".into(),
        exit: out.exit_code,
        stderr_tail: out.stderr.trim().to_string(),
    })
}

pub async fn pr_status(
    gh: &Path,
    worktree: &Path,
    pr_ref: &str,
    overlay: &[(String, String)],
) -> Result<Option<PrStatus>> {
    let first = if pr_ref.parse::<u32>().is_ok() {
        let vars = [("-F", format!("number={pr_ref}"))];
        pr_by_number(&graphql(gh, worktree, BY_NUMBER, &vars, overlay).await?)?
    } else {
        let vars = [("-f", format!("branch={pr_ref}"))];
        pr_for_branch(
            &graphql(gh, worktree, BY_BRANCH, &vars, overlay).await?,
            pr_ref,
        )?
    };
    let Some(mut pr) = first else {
        return Ok(None);
    };
    while let Some(after) = pr.next_page() {
        let vars = [
            ("-F", format!("number={}", pr.number)),
            ("-f", format!("after={after}")),
        ];
        let out = graphql(gh, worktree, BY_NUMBER, &vars, overlay).await?;
        pr.append_page(pr_by_number(&out)?);
    }
    Ok(Some(parse_pr(pr)))
}

/// Just the PR's state, for sweeps that need nothing else; `None` when the number resolves to no PR.
pub async fn pr_state(
    gh: &Path,
    dir: &Path,
    pr_ref: &str,
    overlay: &[(String, String)],
) -> Result<Option<PrState>> {
    #[derive(Deserialize)]
    struct RawState {
        state: String,
    }
    let out = Exec::new(gh)
        .cwd(dir)
        .args(["pr", "view", pr_ref, "--json", "state"])
        .envs(overlay.iter().cloned())
        .timeout(Duration::from_secs(30))
        .run_unchecked()
        .await?;
    if out.exit_code == Some(0) {
        let raw: RawState =
            serde_json::from_str(&out.stdout).map_err(|e| CoreError::OutputParse {
                program: "gh pr view".into(),
                detail: e.to_string(),
            })?;
        return Ok(Some(parse_state(&raw.state)));
    }
    let stderr = out.stderr.to_lowercase();
    if stderr.contains("no pull requests found")
        || stderr.contains("could not resolve to a pullrequest")
    {
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
    use serde_json::{json, Value};

    macro_rules! capture {
        ($name:literal) => {
            (
                include_str!(concat!("../tests/fixtures/pr/", $name, ".graphql.json")),
                include_str!(concat!("../tests/fixtures/pr/", $name, ".checks.json")),
            )
        };
    }

    const REACT_38314: (&str, &str) = capture!("facebook-react-38314");
    const CLI_15117: (&str, &str) = capture!("cli-cli-15117");
    const CLI_14927: (&str, &str) = capture!("cli-cli-14927");
    const SURICATA_15352: (&str, &str) = capture!("OISF-suricata-15352");
    const BLACK_5447: (&str, &str) = capture!("psf-black-5447");
    const BLACK_5590: (&str, &str) = capture!("psf-black-5590");

    fn status_from_pages(pages: &[String]) -> PrStatus {
        let mut pr = pr_by_number(&pages[0]).unwrap().unwrap();
        for page in &pages[1..] {
            assert!(pr.next_page().is_some());
            pr.append_page(pr_by_number(page).unwrap());
        }
        assert_eq!(pr.next_page(), None);
        parse_pr(pr)
    }

    fn pages_of(capture: &str) -> Vec<String> {
        let pages: Vec<Value> = serde_json::from_str(capture).unwrap();
        pages.iter().map(Value::to_string).collect()
    }

    fn assert_matches_gh(status: &PrStatus, checks_json: &str) {
        let gh: Vec<Value> = serde_json::from_str(checks_json).unwrap();
        let count = |bucket: &str| gh.iter().filter(|c| c["bucket"] == bucket).count() as u32;
        let c = &status.checks;
        assert_eq!(c.total as usize, gh.len());
        assert_eq!(
            (c.passed, c.failed, c.pending, c.skipped, c.cancelled),
            (
                count("pass"),
                count("fail"),
                count("pending"),
                count("skipping"),
                count("cancel")
            )
        );
        let mut ours: Vec<(String, String)> = c
            .list
            .iter()
            .map(|k| {
                (
                    k.name.clone(),
                    serde_json::to_value(k.bucket).unwrap().to_string(),
                )
            })
            .collect();
        let mut theirs: Vec<(String, String)> = gh
            .iter()
            .map(|k| {
                (
                    k["name"].as_str().unwrap().to_string(),
                    k["bucket"].to_string(),
                )
            })
            .collect();
        ours.sort();
        theirs.sort();
        assert_eq!(ours, theirs);
    }

    fn from_capture((graphql, checks): (&str, &str)) -> PrStatus {
        let status = status_from_pages(&pages_of(graphql));
        assert_matches_gh(&status, checks);
        status
    }

    fn pr_node(number: u32, state: &str, head: &str, nodes: Value) -> Value {
        json!({
            "number": number, "url": format!("https://github.com/o/r/pull/{number}"),
            "title": "t", "state": state, "mergeable": "MERGEABLE", "reviewDecision": "",
            "headRefName": head, "isCrossRepository": false,
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": nodes,
            } } } }] },
        })
    }

    fn by_number(pr: Value) -> String {
        json!({ "data": { "repository": { "pullRequest": pr } } }).to_string()
    }

    fn by_branch(prs: Vec<Value>) -> String {
        json!({ "data": { "repository": {
            "defaultBranchRef": { "name": "main" },
            "pullRequests": { "nodes": prs },
        } } })
        .to_string()
    }

    fn checks_of(nodes: Value) -> PrStatus {
        status_from_pages(&[by_number(pr_node(1, "OPEN", "feat", nodes))])
    }

    fn run(name: &str, status: &str, conclusion: Option<&str>, started: &str) -> Value {
        json!({
            "__typename": "CheckRun", "name": name, "status": status,
            "conclusion": conclusion, "startedAt": started,
            "checkSuite": { "workflowRun": { "event": "pull_request", "workflow": { "name": "ci" } } },
        })
    }

    fn context(name: &str, state: &str, target: &str) -> Value {
        json!({
            "__typename": "StatusContext", "context": name, "state": state,
            "startedAt": "2026-01-01T00:00:00Z", "targetUrl": target,
        })
    }

    #[test]
    fn superseded_cancelled_runs_do_not_turn_a_green_pr_red() {
        assert_eq!(pages_of(REACT_38314.0).len(), 3);
        let status = from_capture(REACT_38314);
        assert_eq!(status.checks.state, ChecksState::Passing);
        assert_eq!((status.checks.failed, status.checks.cancelled), (0, 0));
    }

    #[test]
    fn check_count_matches_github() {
        from_capture(CLI_15117);
        from_capture(SURICATA_15352);
    }

    #[test]
    fn real_rollup_counts_superseded_runs() {
        let status = from_capture(CLI_14927);
        assert_eq!(status.checks.state, ChecksState::Failing);
        assert_eq!(status.checks.failed, 1);
    }

    #[test]
    fn paged_rollups_count_like_gh_and_a_same_second_tie_reads_failing() {
        assert_eq!(pages_of(BLACK_5447.0).len(), 1);
        assert_eq!(from_capture(BLACK_5447).checks.state, ChecksState::Failing);
        assert_eq!(pages_of(BLACK_5590.0).len(), 2);
        let status = from_capture(BLACK_5590);
        assert_eq!(status.checks.state, ChecksState::Failing);
        let changelog: Vec<_> = status
            .checks
            .list
            .iter()
            .filter(|c| c.name == "check")
            .collect();
        assert_eq!(changelog.len(), 1);
        assert_eq!(changelog[0].bucket, CheckBucket::Fail);
    }

    #[test]
    fn a_same_second_tie_between_runs_of_one_check_reads_failing() {
        let at = "2026-01-01T00:00:00Z";
        let status = checks_of(json!([
            run("check", "COMPLETED", Some("SUCCESS"), at),
            run("check", "COMPLETED", Some("FAILURE"), at),
        ]));
        assert_eq!((status.checks.total, status.checks.failed), (1, 1));
        assert_eq!(status.checks.state, ChecksState::Failing);
    }

    #[test]
    fn a_status_and_a_check_run_with_one_name_are_both_counted() {
        let mut app_run = run(
            "codecov/patch",
            "COMPLETED",
            Some("FAILURE"),
            "2026-01-01T00:00:00Z",
        );
        app_run["checkSuite"] = json!({ "workflowRun": null });
        let mut status = context("codecov/patch", "SUCCESS", "https://codecov.example");
        status["startedAt"] = json!("2026-01-02T00:00:00Z");
        let pr = checks_of(json!([app_run, status]));
        assert_eq!((pr.checks.total, pr.checks.failed), (2, 1));
        assert_eq!(pr.checks.state, ChecksState::Failing);
    }

    #[test]
    fn newer_passing_run_still_reads_failing() {
        let mut page = serde_json::from_str::<Vec<Value>>(CLI_14927.0).unwrap()[0].clone();
        let nodes = page
            .pointer_mut(
                "/data/repository/pullRequest/commits/nodes/0/commit/statusCheckRollup/contexts/nodes",
            )
            .and_then(Value::as_array_mut)
            .unwrap();
        let mut rerun = nodes
            .iter()
            .find(|n| n["conclusion"] == "FAILURE")
            .unwrap()
            .clone();
        rerun["conclusion"] = json!("SUCCESS");
        rerun["startedAt"] = json!("2099-01-01T00:00:00Z");
        nodes.push(rerun);
        let status = status_from_pages(&[page.to_string()]);
        assert_eq!(status.checks.state, ChecksState::Passing);
        assert_eq!((status.checks.total, status.checks.failed), (20, 0));
    }

    #[test]
    fn cancelled_and_skipped_runs_have_their_own_buckets() {
        let at = "2026-01-01T00:00:00Z";
        let cancelled = checks_of(json!([run("a", "COMPLETED", Some("CANCELLED"), at)]));
        assert_eq!(cancelled.checks.list[0].bucket, CheckBucket::Cancel);
        assert_eq!(cancelled.checks.state, ChecksState::Passing);
        assert_eq!(
            (cancelled.checks.cancelled, cancelled.checks.failed),
            (1, 0)
        );

        let skipped = checks_of(json!([run("a", "COMPLETED", Some("SKIPPED"), at)]));
        assert_eq!(skipped.checks.list[0].bucket, CheckBucket::Skipping);
        assert_eq!(skipped.checks.state, ChecksState::Passing);
    }

    #[test]
    fn failure_outranks_a_running_check() {
        let at = "2026-01-01T00:00:00Z";
        let status = checks_of(json!([
            run("build", "IN_PROGRESS", None, at),
            run("test", "COMPLETED", Some("FAILURE"), at),
        ]));
        let c = &status.checks;
        assert_eq!(c.state, ChecksState::Failing);
        assert_eq!((c.total, c.failed, c.pending), (2, 1, 1));
    }

    #[test]
    fn branch_lookup_picks_the_pr_gh_pr_view_would() {
        let pick = |prs: Vec<Value>, branch: &str| {
            pr_for_branch(&by_branch(prs), branch)
                .unwrap()
                .map(|p| p.number)
        };
        let merged = pr_node(2, "MERGED", "feat", json!([]));
        let open = pr_node(1, "OPEN", "feat", json!([]));
        assert_eq!(pick(vec![merged.clone(), open], "feat"), Some(1));

        let mut fork = pr_node(3, "OPEN", "feat", json!([]));
        fork["isCrossRepository"] = json!(true);
        assert_eq!(pick(vec![fork, merged], "feat"), Some(2));

        let merged_main = pr_node(4, "MERGED", "main", json!([]));
        assert_eq!(pick(vec![merged_main.clone()], "main"), None);
        let open_main = pr_node(5, "OPEN", "main", json!([]));
        assert_eq!(pick(vec![merged_main, open_main], "main"), Some(5));

        assert_eq!(pick(vec![], "feat"), None);
    }

    #[test]
    fn graphql_errors_are_failures_not_missing_prs() {
        let partial = json!({
            "data": { "repository": { "pullRequest": pr_node(1, "OPEN", "feat", json!([])) } },
            "errors": [{ "type": "SOMETHING", "message": "partial" }],
        });
        assert!(matches!(
            pr_by_number(&partial.to_string()),
            Err(CoreError::Exec { exit: Some(0), .. })
        ));
        let not_found = r#"{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND","path":["repository","pullRequest"],"message":"Could not resolve to a PullRequest with the number of 99999999."}]}"#;
        let err = pr_by_number(not_found).err().unwrap();
        assert!(err
            .to_string()
            .contains("Could not resolve to a PullRequest"));
        assert!(matches!(
            pr_for_branch(&by_branch(vec![]), "feat"),
            Ok(None)
        ));
    }

    #[test]
    fn a_pr_without_checks_reads_none() {
        let mut pr = pr_node(1, "OPEN", "feat", json!([]));
        pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = Value::Null;
        let status = status_from_pages(&[by_number(pr)]);
        assert_eq!(status.checks.state, ChecksState::None);
        assert_eq!(status.checks.total, 0);
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
    fn all_success_passes_and_skipped_counts_as_skipping() {
        let at = "2026-01-01T00:00:00Z";
        let mut pr = pr_node(
            1,
            "OPEN",
            "feat",
            json!([
                run("build", "COMPLETED", Some("SUCCESS"), at),
                run("lint", "COMPLETED", Some("SKIPPED"), at),
                context("ci/ext", "SUCCESS", "https://ext.example"),
            ]),
        );
        pr["reviewDecision"] = json!("APPROVED");
        let status = status_from_pages(&[by_number(pr)]);
        let c = &status.checks;
        assert_eq!(c.state, ChecksState::Passing);
        assert_eq!((c.total, c.passed, c.skipped), (3, 2, 1));
        assert!(status.mergeable);
        assert_eq!(status.mergeable_state, MergeableState::Mergeable);
        assert_eq!(status.review_decision, Some(ReviewDecision::Approved));
    }

    #[test]
    fn failure_without_pending_is_failing() {
        let mut build = run(
            "build",
            "COMPLETED",
            Some("SUCCESS"),
            "2026-01-02T00:00:00Z",
        );
        build["detailsUrl"] = json!("https://x/run/1");
        let mut pr = pr_node(
            2,
            "OPEN",
            "feat",
            json!([build, context("coverage", "ERROR", "https://x/cov")]),
        );
        pr["mergeable"] = json!("CONFLICTING");
        let status = status_from_pages(&[by_number(pr)]);
        let c = &status.checks;
        assert_eq!(c.state, ChecksState::Failing);
        assert_eq!((c.passed, c.failed), (1, 1));
        assert!(!status.mergeable);
        assert_eq!(status.mergeable_state, MergeableState::Conflicting);
        assert_eq!(status.review_decision, None);

        assert_eq!(c.list.len(), 2);
        assert_eq!(c.list[0].name, "build");
        assert_eq!(c.list[0].bucket, CheckBucket::Pass);
        assert_eq!(c.list[0].url.as_deref(), Some("https://x/run/1"));
        assert_eq!(c.list[1].name, "coverage");
        assert_eq!(c.list[1].bucket, CheckBucket::Fail);
        assert_eq!(c.list[1].url.as_deref(), Some("https://x/cov"));
    }

    #[test]
    fn actions_check_links_into_pr_checks_tab() {
        let mut build = run(
            "build",
            "COMPLETED",
            Some("SUCCESS"),
            "2026-01-02T00:00:00Z",
        );
        build["detailsUrl"] =
            json!("https://github.com/o/r/actions/runs/27760243809/job/82132429491");
        let pr = pr_node(
            8,
            "OPEN",
            "feat",
            json!([
                build,
                context("ci/ext", "SUCCESS", "https://ext.example/run/9")
            ]),
        );
        let list = status_from_pages(&[by_number(pr)]).checks.list;
        assert_eq!(
            list[0].url.as_deref(),
            Some("https://github.com/o/r/pull/8/checks?check_run_id=82132429491")
        );
        assert_eq!(list[1].url.as_deref(), Some("https://ext.example/run/9"));
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
        let rd = |raw: &str| {
            let mut pr = pr_node(1, "OPEN", "feat", json!([]));
            pr["reviewDecision"] = json!(raw);
            status_from_pages(&[by_number(pr)]).review_decision
        };
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
