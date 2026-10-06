use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum SkillTarget {
    Claude,
    Agents,
}

impl SkillTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillTarget::Claude => "claude",
            SkillTarget::Agents => "agents",
        }
    }

    /// Unknown values are `None` so a newer lock's target is dropped, not fatal.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(SkillTarget::Claude),
            "agents" => Some(SkillTarget::Agents),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum SkillScope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum SkillSourceKind {
    #[serde(rename = "github")]
    Github,
    #[serde(rename = "skills.sh")]
    SkillsSh,
}

impl SkillSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillSourceKind::Github => "github",
            SkillSourceKind::SkillsSh => "skills.sh",
        }
    }

    /// Unknown values are `None`: listed, never updated or restored.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(SkillSourceKind::Github),
            "skills.sh" => Some(SkillSourceKind::SkillsSh),
            _ => None,
        }
    }
}
