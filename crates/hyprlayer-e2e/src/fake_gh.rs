use std::path::Path;

use serde_json::{json, Value};

// A GitHub-looking remote that never resolves; Fetch only fetches `origin`.
pub const GITHUB_REMOTE: &str = "https://github.invalid/e2e/demo.git";

pub fn script(state_dir: &Path) -> String {
    format!(
        r##"#!/bin/sh
dir="{dir}"
{{ printf '%s ' "$@" | tr '\n' ' '; echo; }} >> "$dir/calls.log"
[ "$1" = "--version" ] && {{ echo "gh version 2.81.0 (2026-01-01)"; exit 0; }}
[ -f "$dir/FAIL" ] && {{ echo "HTTP 502: Bad Gateway" >&2; exit 1; }}
if [ -f "$dir/SECONDARY" ]; then
  echo "gh: You have exceeded a secondary rate limit. Please wait a few minutes before you try again." >&2
  exit 1
fi
if [ -f "$dir/RATE_LIMITED" ]; then
  echo '{{"data":null,"errors":[{{"type":"RATE_LIMITED","message":"API rate limit already exceeded for user ID 1."}}]}}'
  echo "gh: API rate limit already exceeded for user ID 1." >&2
  exit 1
fi
node_for() {{
  key=$(printf '%s' "$1" | tr '/' '_')
  [ -f "$dir/$key.json" ] || return 1
  oid=$(git rev-parse -q --verify "refs/heads/$1" 2>/dev/null || git rev-parse -q --verify HEAD 2>/dev/null)
  sed "s/@HEAD_OID@/$oid/g" "$dir/$key.json"
}}
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  number=""; branch=""
  for arg in "$@"; do
    case "$arg" in
      number=*) number="${{arg#number=}}" ;;
      branch=*) branch="${{arg#branch=}}" ;;
    esac
  done
  rate='{{"limit":5000,"remaining":5000,"resetAt":"2099-01-01T00:00:00Z"}}'
  [ -f "$dir/RATE" ] && rate=$(cat "$dir/RATE")
  if [ -n "$number" ]; then
    if node=$(node_for "$number"); then
      printf '{{"data":{{"rateLimit":%s,"repository":{{"pullRequest":%s}}}}}}\n' "$rate" "$node"
      exit 0
    fi
    printf '{{"data":{{"repository":{{"pullRequest":null}}}},"errors":[{{"type":"NOT_FOUND","path":["repository","pullRequest"],"message":"Could not resolve to a PullRequest with the number of %s."}}]}}\n' "$number"
    echo "gh: Could not resolve to a PullRequest with the number of $number." >&2
    exit 1
  fi
  node=$(node_for "$branch") || node=""
  if [ -f "$dir/SLOW_ONCE" ] && [ "$(cat "$dir/SLOW_ONCE")" = "$branch" ]; then
    rm -f "$dir/SLOW_ONCE"
    sleep 6
  fi
  printf '{{"data":{{"rateLimit":%s,"repository":{{"defaultBranchRef":{{"name":"main"}},"pullRequests":{{"nodes":[%s]}}}}}}}}\n' "$rate" "$node"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  key=$(printf '%s' "$3" | tr '/' '_')
  sed 's/"state":"OPEN"/"state":"MERGED"/' "$dir/$key.json" > "$dir/$key.merging" && mv "$dir/$key.merging" "$dir/$key.json"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  key=$(printf '%s' "$3" | tr '/' '_')
  [ -f "$dir/$key.json" ] && {{ cat "$dir/$key.json"; exit 0; }}
  echo "GraphQL: Could not resolve to a PullRequest with the number of $3." >&2
  exit 1
fi
echo "e2e gh stub: unsupported: $*" >&2
exit 1
"##,
        dir = state_dir.display()
    )
}

pub struct FakePr {
    pub number: u32,
    pub state: &'static str,
    pub is_draft: bool,
    pub base: &'static str,
    pub merge_state: &'static str,
    pub head_oid: Option<String>,
    pub checks: Vec<Value>,
}

impl FakePr {
    pub fn new(number: u32, state: &'static str) -> Self {
        FakePr {
            number,
            state,
            is_draft: false,
            base: "main",
            merge_state: "UNKNOWN",
            head_oid: None,
            checks: Vec::new(),
        }
    }
}

pub fn set_pr(dir: &Path, reference: &str, pr: &FakePr) {
    let head_ref_name = if reference.bytes().all(|b| b.is_ascii_digit()) {
        ""
    } else {
        reference
    };
    let node = json!({
        "number": pr.number,
        "url": format!("https://github.com/e2e/demo/pull/{}", pr.number),
        "title": format!("E2E #{}", pr.number),
        "state": pr.state,
        "reviewDecision": "",
        "isDraft": pr.is_draft,
        "baseRefName": pr.base,
        "headRefName": head_ref_name,
        "headRefOid": pr.head_oid.as_deref().unwrap_or("@HEAD_OID@"),
        "mergeStateStatus": pr.merge_state,
        "isCrossRepository": false,
        "latestReviews": { "nodes": [] },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": {
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": pr.checks,
        } } } }] },
    });
    let path = dir.join(format!("{}.json", reference.replace('/', "_")));
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, node.to_string()).expect("write the gh state");
    std::fs::rename(&tmp, &path).expect("publish the gh state");
}

pub fn calls(dir: &Path, reference: &str) -> usize {
    let wanted = [format!("branch={reference}"), format!("number={reference}")];
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("api graphql "))
        .filter(|line| line.split(' ').any(|arg| wanted.iter().any(|w| w == arg)))
        .count()
}
