use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    #[default]
    Claude,
    Opencode,
    Codex,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Opencode => "opencode",
            Harness::Codex => "codex",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "opencode" => Harness::Opencode,
            "codex" => Harness::Codex,
            _ => Harness::Claude,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    BypassPermissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Research,
    Plan,
    Implement,
    Validate,
    Commit,
    Ship,
    Revise,
    General,
}

impl Stage {
    pub fn try_parse(s: &str) -> Option<Stage> {
        match s {
            "general" => Some(Stage::General),
            "research" => Some(Stage::Research),
            "plan" => Some(Stage::Plan),
            "implement" => Some(Stage::Implement),
            "validate" => Some(Stage::Validate),
            "commit" => Some(Stage::Commit),
            "ship" => Some(Stage::Ship),
            "revise" => Some(Stage::Revise),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Stage {
        Stage::try_parse(s).unwrap_or(Stage::General)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Stage::General => "general",
            Stage::Research => "research",
            Stage::Plan => "plan",
            Stage::Implement => "implement",
            Stage::Validate => "validate",
            Stage::Commit => "commit",
            Stage::Ship => "ship",
            Stage::Revise => "revise",
        }
    }

    pub fn command(self) -> Option<&'static str> {
        match self {
            Stage::Research => Some("/research_codebase"),
            Stage::Plan => Some("/create_plan"),
            Stage::Implement => Some("/implement_plan"),
            Stage::Validate => Some("/validate_plan"),
            Stage::Commit => Some("/commit"),
            Stage::General | Stage::Ship | Stage::Revise => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_round_trips_through_its_wire_string() {
        for harness in [Harness::Claude, Harness::Opencode, Harness::Codex] {
            assert_eq!(Harness::parse(harness.as_str()), harness);
        }
        assert_eq!(Harness::parse("something-else"), Harness::Claude);
    }

    #[test]
    fn stage_round_trips_and_falls_back_to_general() {
        for stage in [
            Stage::Research,
            Stage::Plan,
            Stage::Implement,
            Stage::Validate,
            Stage::Commit,
            Stage::Ship,
            Stage::Revise,
            Stage::General,
        ] {
            assert_eq!(Stage::parse(stage.as_str()), stage);
        }
        assert_eq!(Stage::try_parse("nope"), None);
        assert_eq!(Stage::parse("nope"), Stage::General);
    }
}
