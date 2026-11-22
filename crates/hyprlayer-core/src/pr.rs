use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::gitops::GitHubRemote;
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
pub enum MergeState {
    Clean,
    Blocked,
    Behind,
    Dirty,
    Unstable,
    HasHooks,
    Draft,
    Unknown,
}

impl MergeState {
    fn parse(raw: &str) -> Self {
        match raw.to_ascii_uppercase().as_str() {
            "CLEAN" => MergeState::Clean,
            "BLOCKED" => MergeState::Blocked,
            "BEHIND" => MergeState::Behind,
            "DIRTY" => MergeState::Dirty,
            "UNSTABLE" => MergeState::Unstable,
            "HAS_HOOKS" => MergeState::HasHooks,
            "DRAFT" => MergeState::Draft,
            _ => MergeState::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

impl ReviewState {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_uppercase().as_str() {
            "APPROVED" => Some(ReviewState::Approved),
            "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
            "COMMENTED" => Some(ReviewState::Commented),
            "DISMISSED" => Some(ReviewState::Dismissed),
            "PENDING" => Some(ReviewState::Pending),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PrReview {
    pub author: String,
    pub state: ReviewState,
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
    pub is_draft: bool,
    pub base_ref_name: String,
    pub head_ref_oid: String,
    pub merge_state: MergeState,
    pub checks: PrChecks,
    pub review_decision: Option<ReviewDecision>,
    pub reviews: Vec<PrReview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PrLookup {
    Unknown,
    NoPr,
    NotGitHub,
    Found {
        pr: PrStatus,
    },
    Failed {
        cause: PrLookupFailure,
        last: Option<PrStatus>,
    },
}

impl PrStatus {
    pub(crate) fn recorded(number: u32, state: PrState, url: String, head_ref_oid: String) -> Self {
        PrStatus {
            number,
            url,
            title: String::new(),
            state,
            is_draft: false,
            base_ref_name: String::new(),
            head_ref_oid,
            merge_state: MergeState::Unknown,
            checks: summarize(Vec::new()),
            review_decision: None,
            reviews: Vec::new(),
        }
    }
}

impl PrLookup {
    pub fn known(&self) -> Option<&PrStatus> {
        match self {
            PrLookup::Found { pr } => Some(pr),
            PrLookup::Failed { last, .. } => last.as_ref(),
            PrLookup::Unknown | PrLookup::NoPr | PrLookup::NotGitHub => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PrLookupFailure {
    GhMissing,
    SignedOut,
    RateLimited { resumes_at: Option<f64> },
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quota {
    pub limit: u32,
    pub remaining: u32,
    pub reset_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrAnswer {
    pub pr: Option<PrStatus>,
    pub quota: Option<Quota>,
}

macro_rules! pr_fragment {
    () => {
        "fragment Pr on PullRequest {
  number url title state reviewDecision headRefName isCrossRepository headRepositoryOwner { login }
  isDraft baseRefName headRefOid mergeStateStatus
  latestReviews(first: 50) { nodes { author { login } state } }
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
#[serde(rename_all = "camelCase")]
struct RawRate {
    limit: u32,
    remaining: u32,
    reset_at: String,
}

impl From<RawRate> for Quota {
    fn from(r: RawRate) -> Self {
        Quota {
            limit: r.limit,
            remaining: r.remaining,
            reset_at: crate::store::epoch_ms_of_utc(&r.reset_at),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ByNumberData {
    #[serde(default)]
    rate_limit: Option<RawRate>,
    repository: Option<RawRepoByNumber>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepoByNumber {
    pull_request: Option<RawPr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ByBranchData {
    #[serde(default)]
    rate_limit: Option<RawRate>,
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
    review_decision: Option<String>,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    head_repository_owner: Option<RawLogin>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    head_ref_oid: String,
    #[serde(default)]
    merge_state_status: String,
    #[serde(default)]
    latest_reviews: Nodes<RawReview>,
    #[serde(default)]
    commits: Nodes<RawCommitNode>,
}

#[derive(Deserialize)]
struct RawReview {
    #[serde(default)]
    author: Option<RawLogin>,
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct RawLogin {
    login: String,
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
    let reviews = raw
        .latest_reviews
        .nodes
        .into_iter()
        .filter_map(|r| {
            Some(PrReview {
                state: ReviewState::parse(&r.state)?,
                author: r.author.map_or_else(|| "ghost".into(), |a| a.login),
            })
        })
        .collect();
    PrStatus {
        number: raw.number,
        state: parse_state(&raw.state),
        is_draft: raw.is_draft,
        base_ref_name: raw.base_ref_name,
        head_ref_oid: raw.head_ref_oid,
        merge_state: MergeState::parse(&raw.merge_state_status),
        checks,
        review_decision,
        reviews,
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

fn pr_by_number(stdout: &str) -> Result<(Option<RawPr>, Option<Quota>)> {
    let data: ByNumberData = graphql_data(stdout)?;
    Ok((
        data.repository.and_then(|r| r.pull_request),
        data.rate_limit.map(Quota::from),
    ))
}

fn pr_for_branch(
    stdout: &str,
    branch: &str,
    fork_owner: Option<&str>,
) -> Result<(Option<RawPr>, Option<Quota>)> {
    let data: ByBranchData = graphql_data(stdout)?;
    Ok((
        pick_for_branch(data.repository, branch, fork_owner),
        data.rate_limit.map(Quota::from),
    ))
}

fn pick_for_branch(repo: RawRepoByBranch, branch: &str, fork_owner: Option<&str>) -> Option<RawPr> {
    let default = repo.default_branch_ref.map(|r| r.name);
    let mut prs = repo.pull_requests.nodes;
    prs.sort_by_key(|p| !p.state.eq_ignore_ascii_case("OPEN"));
    prs.into_iter().find(|p| {
        p.head_ref_name == branch
            && match fork_owner {
                Some(owner) => {
                    p.is_cross_repository
                        && p.head_repository_owner
                            .as_ref()
                            .is_some_and(|o| o.login.eq_ignore_ascii_case(owner))
                }
                None => {
                    !p.is_cross_repository
                        && (p.state.eq_ignore_ascii_case("OPEN")
                            || default.as_deref() != Some(branch))
                }
            }
    })
}

#[derive(Clone, Copy)]
struct Target<'a> {
    host: Option<&'a str>,
    repo: Option<(&'a str, &'a str)>,
}

// The first ask leaves the repo to gh, whose own remote pick also skips hosts it has no login
// for. A retry names its remote's repo, so a query never pairs one host with another's repo.
fn target(remotes: &[GitHubRemote], attempt: usize) -> Target<'_> {
    let remote = remotes.get(attempt);
    Target {
        host: remote.and_then(GitHubRemote::api_host),
        repo: remote
            .filter(|_| attempt > 0)
            .and_then(|r| r.path.split_once('/')),
    }
}

async fn graphql(
    gh: &Path,
    dir: &Path,
    query: &str,
    vars: &[(&str, String)],
    target: Target<'_>,
    overlay: &[(String, String)],
) -> Result<String> {
    let mut exec = Exec::new(gh).cwd(dir).args(["api", "graphql"]);
    exec = match target.repo {
        Some((owner, name)) => exec
            .arg("-f")
            .arg(format!("owner={owner}"))
            .arg("-f")
            .arg(format!("repo={name}")),
        None => exec.args(["-F", "owner={owner}", "-F", "repo={repo}"]),
    };
    exec = exec.arg("-f").arg(format!("query={query}"));
    if let Some(host) = target.host {
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

pub(crate) fn gh_found_no_github_remote(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains(
        "none of the git remotes configured for this repository point to a known github host",
    ) || s.contains("no git remotes found")
}

fn another_host_cannot_help(e: &CoreError) -> bool {
    let text = e.to_string();
    crate::runtime::pr_watcher::is_rate_limited(&text) || gh_found_no_github_remote(&text)
}

pub async fn pr_status(
    gh: &Path,
    worktree: &Path,
    pr_ref: &str,
    overlay: &[(String, String)],
) -> Result<PrAnswer> {
    let by_number = pr_ref.parse::<u32>().is_ok();
    let (query, vars) = if by_number {
        (BY_NUMBER, [("-F", format!("number={pr_ref}"))])
    } else {
        (BY_BRANCH, [("-f", format!("branch={pr_ref}"))])
    };
    let remotes = crate::gitops::lookup_remotes(worktree);
    let mut attempt = 0;
    let out = loop {
        let asked = target(&remotes, attempt);
        match graphql(gh, worktree, query, &vars, asked, overlay).await {
            Err(e) if attempt + 1 < remotes.len() && !another_host_cannot_help(&e) => {
                attempt += 1;
            }
            out => break out?,
        }
    };
    let asked = target(&remotes, attempt);
    let (first, mut quota) = if by_number {
        pr_by_number(&out)?
    } else {
        let fork_owner = remotes
            .get(attempt)
            .and_then(|base| crate::gitops::fork_owner(worktree, pr_ref, base));
        pr_for_branch(&out, pr_ref, fork_owner.as_deref())?
    };
    let Some(mut pr) = first else {
        return Ok(PrAnswer { pr: None, quota });
    };
    while let Some(after) = pr.next_page() {
        let vars = [
            ("-F", format!("number={}", pr.number)),
            ("-f", format!("after={after}")),
        ];
        let out = graphql(gh, worktree, BY_NUMBER, &vars, asked, overlay).await?;
        let (page, page_quota) = pr_by_number(&out)?;
        pr.append_page(page);
        quota = page_quota.or(quota);
    }
    Ok(PrAnswer {
        pr: Some(parse_pr(pr)),
        quota,
    })
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
        let mut pr = pr_by_number(&pages[0]).unwrap().0.unwrap();
        for page in &pages[1..] {
            assert!(pr.next_page().is_some());
            pr.append_page(pr_by_number(page).unwrap().0);
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
            "title": "t", "state": state, "reviewDecision": "",
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
        let pick_as = |prs: Vec<Value>, branch: &str, fork_owner: Option<&str>| {
            pr_for_branch(&by_branch(prs), branch, fork_owner)
                .unwrap()
                .0
                .map(|p| p.number)
        };
        let pick = |prs: Vec<Value>, branch: &str| pick_as(prs, branch, None);
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

        let from_fork = |number: u32, state: &str, head: &str, owner: &str| {
            let mut pr = pr_node(number, state, head, json!([]));
            pr["isCrossRepository"] = json!(true);
            pr["headRepositoryOwner"] = json!({ "login": owner });
            pr
        };
        let theirs = from_fork(6, "OPEN", "feat", "another");
        let mine = from_fork(7, "OPEN", "feat", "me");
        let same_repo = pr_node(8, "OPEN", "feat", json!([]));
        assert_eq!(
            pick_as(vec![theirs, mine.clone(), same_repo], "feat", Some("me")),
            Some(7)
        );
        assert_eq!(pick_as(vec![mine], "feat", Some("Me")), Some(7));
        let merged_main = from_fork(9, "MERGED", "main", "me");
        assert_eq!(pick_as(vec![merged_main], "main", Some("me")), Some(9));
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
            pr_for_branch(&by_branch(vec![]), "feat", None),
            Ok((None, _))
        ));
    }

    #[test]
    fn a_rate_limited_body_is_an_error_the_watcher_recognises() {
        let body = r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit already exceeded for user ID 1."}]}"#;
        let err = pr_for_branch(body, "feat", None).err().unwrap();
        assert!(crate::runtime::pr_watcher::is_rate_limited(
            &err.to_string()
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_host_hands_the_lookup_to_the_next_one() {
        use std::os::unix::fs::PermissionsExt;

        const FAKE_GH: &str = r#"#!/bin/sh
d=$(dirname "$0")
line=""; prev=""
for a in "$@"; do
  case "$a" in owner=*|repo=*) line="$line $a" ;; esac
  [ "$prev" = "--hostname" ] && line="$line host=$a"
  prev="$a"
done
echo "${line# }" >> "$d/calls"
case "$line" in
  *host=*) [ -f "$d/other.err" ] && { cat "$d/other.err" >&2; exit 1; } ;;
  *) [ -f "$d/dotcom.err" ] && { cat "$d/dotcom.err" >&2; exit 1; } ;;
esac
cat "$d/answer.json"
"#;
        let ask = |upstream: &'static str, other: Option<&'static str>, dotcom: Option<&str>| {
            let dotcom = dotcom.map(str::to_string);
            async move {
                let tmp = tempfile::tempdir().unwrap();
                let gh = tmp.path().join("gh");
                std::fs::write(&gh, FAKE_GH).unwrap();
                std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
                let answer = by_branch(vec![pr_node(5, "OPEN", "feat", json!([]))]);
                std::fs::write(tmp.path().join("answer.json"), answer).unwrap();
                for (file, err) in [
                    ("other.err", other.map(str::to_string)),
                    ("dotcom.err", dotcom),
                ] {
                    if let Some(err) = err {
                        std::fs::write(tmp.path().join(file), err).unwrap();
                    }
                }
                let repo_dir = tmp.path().join("repo");
                let repo = git2::Repository::init(&repo_dir).unwrap();
                repo.remote("upstream", upstream).unwrap();
                repo.remote("origin", "https://github.com/me/app.git")
                    .unwrap();
                let found = pr_status(&gh, &repo_dir, "feat", &[]).await;
                let calls = std::fs::read_to_string(tmp.path().join("calls")).unwrap();
                let calls: Vec<String> = calls.lines().map(str::to_string).collect();
                (found.map(|a| a.pr.map(|p| p.number)), calls)
            }
        };
        let self_hosted = "https://gitlab.corp.example/acme/app.git";
        let first_then_origin = vec![
            "owner={owner} repo={repo} host=gitlab.corp.example".to_string(),
            "owner=me repo=app".to_string(),
        ];

        for answer in [
            "gh: Forbidden",
            "error connecting to gitlab.corp.example",
            "gh: Field 'repository' doesn't exist on type 'Query'",
            "gh: HTTP 404",
        ] {
            let (found, calls) = ask(self_hosted, Some(answer), None).await;
            assert_eq!(found.unwrap(), Some(5), "{answer}");
            assert_eq!(calls, first_then_origin, "{answer}");
        }

        let (found, calls) = ask(
            self_hosted,
            Some("gh: Forbidden"),
            Some("HTTP 502: Bad Gateway"),
        )
        .await;
        assert!(
            found.unwrap_err().to_string().contains("502"),
            "the last host's error"
        );
        assert_eq!(calls, first_then_origin);

        let none_known = "none of the git remotes configured for this repository point to a known \
                          GitHub host. To tell gh about a new GitHub host, please use `gh auth login`";
        let (found, calls) = ask(self_hosted, Some(none_known), None).await;
        assert!(gh_found_no_github_remote(&found.unwrap_err().to_string()));
        assert_eq!(
            calls.len(),
            1,
            "gh's own remote pick found nothing; no host can change that"
        );

        let ghe = "https://ghe.corp.example/acme/app.git";
        let limited = "gh: API rate limit already exceeded for user ID 1.";
        let (found, calls) = ask(ghe, Some(limited), None).await;
        assert!(crate::runtime::pr_watcher::is_rate_limited(
            &found.unwrap_err().to_string()
        ));
        assert_eq!(
            calls.len(),
            1,
            "a rate limit is GitHub's answer, not a wrong host"
        );
    }

    #[test]
    fn rate_limit_reads_as_quota() {
        let body = json!({ "data": {
            "rateLimit": { "limit": 5000, "remaining": 4935, "resetAt": "2099-01-01T00:00:00Z" },
            "repository": { "pullRequest": pr_node(1, "OPEN", "feat", json!([])) },
        } });
        let (_, quota) = pr_by_number(&body.to_string()).unwrap();
        assert_eq!(
            quota,
            Some(Quota {
                limit: 5000,
                remaining: 4935,
                reset_at: Some(4_070_908_800_000.0),
            })
        );
        assert_eq!(
            crate::store::epoch_ms_of_utc("2099-01-01T00:00:00Z"),
            Some(4_070_908_800_000.0)
        );
        assert_eq!(
            crate::store::epoch_ms_of_utc("2026-11-21T18:13:01Z"),
            Some(1_795_284_781_000.0)
        );
        assert_eq!(crate::store::epoch_ms_of_utc("soon"), None);
    }

    #[test]
    fn a_draft_pr_is_reported_as_a_draft_with_its_base() {
        let status = from_capture(SURICATA_15352);
        assert!(status.is_draft);
        assert_eq!(status.base_ref_name, "main");
        assert_eq!(status.merge_state, MergeState::Blocked);
        assert_eq!(
            status.head_ref_oid,
            "3c639c38d43fe3376c0de536fcb442fcb767fe6b"
        );
        let reviews = from_capture(CLI_14927).reviews;
        assert_eq!(
            reviews,
            vec![PrReview {
                author: "copilot-pull-request-reviewer".into(),
                state: ReviewState::Commented,
            }]
        );
    }

    #[test]
    fn merge_state_and_reviews_parse_tolerantly() {
        let merge_state = |raw: Option<&str>| {
            let mut pr = pr_node(1, "OPEN", "feat", json!([]));
            if let Some(raw) = raw {
                pr["mergeStateStatus"] = json!(raw);
            }
            status_from_pages(&[by_number(pr)]).merge_state
        };
        for (raw, want) in [
            ("CLEAN", MergeState::Clean),
            ("blocked", MergeState::Blocked),
            ("BEHIND", MergeState::Behind),
            ("DIRTY", MergeState::Dirty),
            ("UNSTABLE", MergeState::Unstable),
            ("HAS_HOOKS", MergeState::HasHooks),
            ("DRAFT", MergeState::Draft),
            ("SOMETHING_NEW", MergeState::Unknown),
        ] {
            assert_eq!(merge_state(Some(raw)), want, "{raw}");
        }
        assert_eq!(merge_state(None), MergeState::Unknown);

        let mut pr = pr_node(1, "OPEN", "feat", json!([]));
        pr["latestReviews"] = json!({ "nodes": [
            { "author": null, "state": "APPROVED" },
            { "author": { "login": "a" }, "state": "CHANGES_REQUESTED" },
            { "author": { "login": "b" }, "state": "SOMETHING_NEW" },
        ] });
        let status = status_from_pages(&[by_number(pr)]);
        assert_eq!(
            status.reviews,
            vec![
                PrReview {
                    author: "ghost".into(),
                    state: ReviewState::Approved
                },
                PrReview {
                    author: "a".into(),
                    state: ReviewState::ChangesRequested
                },
            ]
        );
        assert!(!status.is_draft);
        assert_eq!(status.base_ref_name, "");
    }

    #[test]
    fn a_lookup_serialises_with_its_kind() {
        assert_eq!(
            serde_json::to_value(PrLookup::NoPr).unwrap(),
            json!({ "kind": "noPr" })
        );
        assert_eq!(
            serde_json::to_value(PrLookup::NotGitHub).unwrap(),
            json!({ "kind": "notGitHub" })
        );
        let failed = PrLookup::Failed {
            cause: PrLookupFailure::RateLimited {
                resumes_at: Some(1.0),
            },
            last: None,
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            json!({ "kind": "failed", "cause": { "kind": "rateLimited", "resumesAt": 1.0 }, "last": null })
        );
        let found = PrLookup::Found {
            pr: checks_of(json!([])),
        };
        let encoded = serde_json::to_value(&found).unwrap();
        assert_eq!(encoded["kind"], "found");
        assert_eq!(encoded["pr"]["number"], 1);
        assert_eq!(serde_json::from_value::<PrLookup>(encoded).unwrap(), found);
        assert_eq!(found.known().map(|p| p.number), Some(1));
        assert_eq!(failed.known(), None);
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
        pr["mergeStateStatus"] = json!("DIRTY");
        let status = status_from_pages(&[by_number(pr)]);
        let c = &status.checks;
        assert_eq!(c.state, ChecksState::Failing);
        assert_eq!((c.passed, c.failed), (1, 1));
        assert_eq!(status.merge_state, MergeState::Dirty);
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
