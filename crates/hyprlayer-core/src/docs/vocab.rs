use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum DocType {
    Plan,
    Research,
    Handoff,
    Note,
    Pr,
}

impl DocType {
    pub const ALL: [DocType; 5] = [
        DocType::Plan,
        DocType::Research,
        DocType::Handoff,
        DocType::Note,
        DocType::Pr,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DocType::Plan => "plan",
            DocType::Research => "research",
            DocType::Handoff => "handoff",
            DocType::Note => "note",
            DocType::Pr => "pr",
        }
    }

    // ASCII case-insensitive: `passes_filters` matches types that way.
    pub fn try_parse(s: &str) -> Option<DocType> {
        DocType::ALL
            .into_iter()
            .find(|t| t.as_str().eq_ignore_ascii_case(s))
    }

    pub fn dir_name(self) -> &'static str {
        match self {
            DocType::Plan => "plans",
            DocType::Research => "research",
            DocType::Handoff => "handoffs",
            DocType::Note => "notes",
            DocType::Pr => "prs",
        }
    }

    pub fn from_dir_name(dir: &str) -> Option<DocType> {
        DocType::ALL.into_iter().find(|t| t.dir_name() == dir)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum DocStatus {
    Draft,
    Active,
    Implemented,
    Superseded,
    Archived,
}

impl DocStatus {
    pub const ALL: [DocStatus; 5] = [
        DocStatus::Draft,
        DocStatus::Active,
        DocStatus::Implemented,
        DocStatus::Superseded,
        DocStatus::Archived,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DocStatus::Draft => "draft",
            DocStatus::Active => "active",
            DocStatus::Implemented => "implemented",
            DocStatus::Superseded => "superseded",
            DocStatus::Archived => "archived",
        }
    }

    pub fn try_parse(s: &str) -> Option<DocStatus> {
        DocStatus::ALL
            .into_iter()
            .find(|v| v.as_str().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum DocScope {
    User,
    Shared,
    Global,
}

impl DocScope {
    pub const ALL: [DocScope; 3] = [DocScope::User, DocScope::Shared, DocScope::Global];

    pub fn as_str(self) -> &'static str {
        match self {
            DocScope::User => "user",
            DocScope::Shared => "shared",
            DocScope::Global => "global",
        }
    }

    pub fn try_parse(s: &str) -> Option<DocScope> {
        DocScope::ALL
            .into_iter()
            .find(|v| v.as_str().eq_ignore_ascii_case(s))
    }
}
