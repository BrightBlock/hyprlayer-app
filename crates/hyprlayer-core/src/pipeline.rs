//! Stage → skill-prompt composition and per-stage permission policy.
//!
//! Each spec-driven stage maps to (1) a first-prompt that invokes the matching
//! hyprlayer skill and (2) a permission posture: which tools are pre-approved,
//! which are hard-denied, and the claude `--permission-mode`. Anything not
//! pre-approved still works — it just routes through a `can_use_tool` prompt the
//! UI renders. The table is a reviewable starting policy, tightened over time.

use crate::claude::PermissionMode;

/// The pipeline stages. Parsed tolerantly from the workstream's stored stage
/// string; an unknown value falls back to [`Stage::General`] (plain chat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    General,
    Research,
    Plan,
    Implement,
    Validate,
    Ship,
}

impl Stage {
    pub fn parse(s: &str) -> Stage {
        match s {
            "research" => Stage::Research,
            "plan" => Stage::Plan,
            "implement" => Stage::Implement,
            "validate" => Stage::Validate,
            "ship" => Stage::Ship,
            _ => Stage::General,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Stage::General => "general",
            Stage::Research => "research",
            Stage::Plan => "plan",
            Stage::Implement => "implement",
            Stage::Validate => "validate",
            Stage::Ship => "ship",
        }
    }
}

/// The permission posture for a stage.
pub struct StagePolicy {
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

/// Read-only tools pre-approved in every stage: inspection, search, planning
/// scaffolding, and non-mutating git/hyprlayer commands.
const READ_ONLY: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "Task",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
    "Bash(git log:*)",
    "Bash(git diff:*)",
    "Bash(git show:*)",
    "Bash(git status:*)",
    "Bash(git blame:*)",
    "Bash(rg:*)",
    "Bash(hyprlayer:*)",
];

fn allow(extra: &[&str]) -> Vec<String> {
    READ_ONLY
        .iter()
        .chain(extra.iter())
        .map(|s| s.to_string())
        .collect()
}

fn deny(specs: &[&str]) -> Vec<String> {
    specs.iter().map(|s| s.to_string()).collect()
}

/// The permission policy for a stage (see module docs for the rationale).
pub fn policy_for(stage: Stage) -> StagePolicy {
    match stage {
        // Plain chat: read-only, everything mutating prompts.
        Stage::General => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&[]),
            disallowed_tools: deny(&[]),
        },
        // Writes the research doc; the worktree is disposable. No pushing.
        Stage::Research => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&["Write", "Edit"]),
            disallowed_tools: deny(&["Bash(git push:*)"]),
        },
        // Same write surface, edits auto-accepted to keep planning flowing.
        Stage::Plan => StagePolicy {
            permission_mode: PermissionMode::AcceptEdits,
            allowed_tools: allow(&["Write", "Edit"]),
            disallowed_tools: deny(&["Bash(git push:*)"]),
        },
        // Implementation can stage and commit, but not push.
        Stage::Implement => StagePolicy {
            permission_mode: PermissionMode::AcceptEdits,
            allowed_tools: allow(&["Write", "Edit", "Bash(git add:*)", "Bash(git commit:*)"]),
            disallowed_tools: deny(&["Bash(git push:*)"]),
        },
        // Validation runs tests but must not mutate the tree or history.
        Stage::Validate => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&[]),
            disallowed_tools: deny(&["Write", "Edit", "Bash(git commit:*)"]),
        },
        // Ship is where pushing and PR creation are unlocked.
        Stage::Ship => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&[
                "Write",
                "Edit",
                "Bash(git add:*)",
                "Bash(git commit:*)",
                "Bash(git push:*)",
                "Bash(gh pr:*)",
                "Bash(gh:*)",
            ]),
            disallowed_tools: deny(&[]),
        },
    }
}

/// Compose the first prompt for a stage: the matching skill invocation plus the
/// user's input, with any attached context docs listed for the skill to pick up
/// (e.g. `/create_plan` consuming the research doc produced by `/research_codebase`).
pub fn compose_prompt(stage: Stage, input: &str, attached_docs: &[String]) -> String {
    let input = input.trim();
    let skill = match stage {
        Stage::General => return input.to_string(),
        Stage::Research => "/research_codebase",
        Stage::Plan => "/create_plan",
        Stage::Implement => "/implement_plan",
        Stage::Validate => "/validate_plan",
        Stage::Ship => "/commit",
    };

    let mut prompt = if input.is_empty() {
        skill.to_string()
    } else {
        format!("{skill} {input}")
    };
    if !attached_docs.is_empty() {
        prompt.push_str("\n\nRelevant documents:");
        for doc in attached_docs {
            prompt.push_str("\n- ");
            prompt.push_str(doc);
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_parse_is_tolerant() {
        assert_eq!(Stage::parse("research"), Stage::Research);
        assert_eq!(Stage::parse("ship"), Stage::Ship);
        assert_eq!(Stage::parse("nonsense"), Stage::General);
        assert_eq!(Stage::Research.as_str(), "research");
    }

    #[test]
    fn general_passes_input_through_verbatim() {
        assert_eq!(compose_prompt(Stage::General, "  hello  ", &[]), "hello");
    }

    #[test]
    fn research_invokes_the_skill() {
        assert_eq!(
            compose_prompt(Stage::Research, "how does auth work?", &[]),
            "/research_codebase how does auth work?"
        );
    }

    #[test]
    fn plan_lists_attached_docs() {
        let prompt = compose_prompt(
            Stage::Plan,
            "build it",
            &["repos/p/shared/research/2026-auth.md".into()],
        );
        assert!(prompt.starts_with("/create_plan build it"));
        assert!(prompt.contains("Relevant documents:"));
        assert!(prompt.contains("- repos/p/shared/research/2026-auth.md"));
    }

    #[test]
    fn empty_input_still_invokes_the_skill() {
        assert_eq!(
            compose_prompt(Stage::Research, "", &[]),
            "/research_codebase"
        );
    }

    #[test]
    fn validate_denies_mutation_ship_allows_push() {
        let validate = policy_for(Stage::Validate);
        assert!(validate.disallowed_tools.iter().any(|t| t == "Write"));
        assert!(validate
            .disallowed_tools
            .iter()
            .any(|t| t.contains("git commit")));

        let ship = policy_for(Stage::Ship);
        assert!(ship.allowed_tools.iter().any(|t| t.contains("git push")));
        assert!(ship.allowed_tools.iter().any(|t| t.contains("gh pr")));
        assert!(ship.disallowed_tools.is_empty());

        // Read-only tools are present in every stage.
        for stage in [Stage::General, Stage::Research, Stage::Implement] {
            assert!(policy_for(stage).allowed_tools.iter().any(|t| t == "Read"));
        }
    }
}
