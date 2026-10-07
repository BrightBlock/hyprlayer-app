use std::sync::Once;

use serde::{Deserialize, Serialize};

use crate::env::{https_or_loopback_http, RuntimeEnvironment};
use crate::error::{CoreError, Result};

use super::github::parse_owner_repo;

pub const DEFAULT_API: &str = "https://skills.sh";
const API_VAR: &str = "HYPRLAYER_SKILLS_API_URL";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillSearchResult {
    pub name: String,
    pub source: String,
    pub skill_id: String,
    pub installs: Option<f64>,
}

/// The skills.sh leaderboards its own site pages through (`/api/skills/{view}/{page}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case")]
pub enum SkillsShView {
    AllTime,
    Trending,
    Hot,
}

impl SkillsShView {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillsShView::AllTime => "all-time",
            SkillsShView::Trending => "trending",
            SkillsShView::Hot => "hot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillLeaderboardPage {
    pub skills: Vec<SkillSearchResult>,
    pub has_more: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    skills: Vec<Row>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    name: String,
    source: String,
    skill_id: String,
    #[serde(default)]
    installs: Option<f64>,
}

/// The test knob accepts `https://` or loopback `http://`; anything else is ignored.
pub fn api_base(env: &RuntimeEnvironment) -> String {
    static WARNED: Once = Once::new();
    let Some(raw) = env.var(API_VAR).and_then(|v| v.to_str()) else {
        return DEFAULT_API.to_string();
    };
    WARNED.call_once(|| tracing::warn!("{API_VAR} is set; skills.sh searches go to {raw}"));
    if https_or_loopback_http(raw) {
        return raw.trim_end_matches('/').to_string();
    }
    tracing::warn!("{API_VAR} must be https:// or loopback http://; ignoring it");
    DEFAULT_API.to_string()
}

pub fn search_url(base: &str, query: &str, limit: u32) -> Result<String> {
    let url = reqwest::Url::parse_with_params(
        &format!("{}/api/search", base.trim_end_matches('/')),
        [("q", query), ("limit", &limit.to_string())],
    )
    .map_err(|e| CoreError::SkillFetchFailed {
        detail: e.to_string(),
    })?;
    Ok(url.to_string())
}

pub fn leaderboard_url(base: &str, view: SkillsShView, page: u32) -> String {
    format!(
        "{}/api/skills/{}/{page}",
        base.trim_end_matches('/'),
        view.as_str()
    )
}

pub fn parse_search(body: &str) -> Result<Vec<SkillSearchResult>> {
    Ok(parse(body)?.skills)
}

pub fn parse_leaderboard(body: &str) -> Result<SkillLeaderboardPage> {
    parse(body)
}

/// Rows whose `source` isn't `owner/repo` can't be cloned from GitHub and drop out.
fn parse(body: &str) -> Result<SkillLeaderboardPage> {
    let parsed: Response = serde_json::from_str(body).map_err(|_| CoreError::SkillFetchFailed {
        detail: "skills.sh sent an unreadable answer.".into(),
    })?;
    Ok(SkillLeaderboardPage {
        skills: parsed
            .skills
            .into_iter()
            .filter(|row| parse_owner_repo(&row.source).is_some())
            .map(|row| SkillSearchResult {
                name: row.name,
                source: row.source,
                skill_id: row.skill_id,
                installs: row.installs,
            })
            .collect(),
        has_more: parsed.has_more,
    })
}
