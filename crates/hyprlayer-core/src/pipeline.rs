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
    /// Strictly parse a stage string: `None` for anything unrecognized. Use this
    /// to validate caller input; [`Stage::parse`] is the tolerant variant that
    /// falls back to [`Stage::General`] for rendering.
    pub fn try_parse(s: &str) -> Option<Stage> {
        match s {
            "general" => Some(Stage::General),
            "research" => Some(Stage::Research),
            "plan" => Some(Stage::Plan),
            "implement" => Some(Stage::Implement),
            "validate" => Some(Stage::Validate),
            "ship" => Some(Stage::Ship),
            _ => None,
        }
    }

    /// Tolerant parse: an unknown value renders as plain chat ([`Stage::General`]).
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
            Stage::Ship => "ship",
        }
    }

    /// The skill slash-command this stage invokes, or `None` for stages that send
    /// a plain prompt (General chat; Ship sends [`SHIP_PROMPT`], not a skill).
    /// Single source of truth for the command, mirrored by the frontend
    /// `stageCommand` (src/features/session/stages.ts) for its optimistic echo.
    pub fn command(self) -> Option<&'static str> {
        match self {
            Stage::Research => Some("/research_codebase"),
            Stage::Plan => Some("/create_plan"),
            Stage::Implement => Some("/implement_plan"),
            Stage::Validate => Some("/validate_plan"),
            Stage::General | Stage::Ship => None,
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

/// The Ship directive: commit, push, and open a PR. Ship has no single skill,
/// so it sends a plain instruction (the policy pre-approves push + `gh pr`).
/// Kept byte-identical to the frontend `SHIP_PROMPT` (src/features/session/stages.ts)
/// so the optimistic transcript echo matches what claude receives.
pub const SHIP_PROMPT: &str =
    "Commit all changes in this worktree, push the branch to its remote, then open a \
pull request against the default branch with a clear title and a summary of what changed.";

fn skill_prompt(skill: &str, input: &str) -> String {
    if input.is_empty() {
        skill.to_string()
    } else {
        format!("{skill} {input}")
    }
}

/// Compose the first prompt for a stage: the matching skill invocation plus the
/// user's input, with any attached context docs listed for the skill to pick up
/// (e.g. `/create_plan` consuming the research doc produced by `/research_codebase`).
/// General is pure passthrough (plain chat); Ship sends the [`SHIP_PROMPT`]
/// directive rather than a skill.
pub fn compose_prompt(stage: Stage, input: &str, attached_docs: &[String]) -> String {
    let input = input.trim();
    let prompt = match stage {
        // Plain chat: the input verbatim — no skill prefix. Explicitly attached
        // docs/screenshots are still listed below (the user asked for them).
        Stage::General => input.to_string(),
        Stage::Ship if input.is_empty() => SHIP_PROMPT.to_string(),
        Stage::Ship => format!("{SHIP_PROMPT}\n\n{input}"),
        // research/plan/implement/validate each invoke their skill.
        _ => skill_prompt(
            stage
                .command()
                .expect("non-ship/general stage has a command"),
            input,
        ),
    };
    append_docs(prompt, attached_docs)
}

/// Append the `Relevant documents:` block listing `attached_docs` (a no-op when
/// empty). The single source of truth for that wire format — shared by the
/// session-start [`compose_prompt`] and the follow-up turn path
/// (`send_user_message`), so a screenshot dropped mid-conversation reaches the
/// agent the same way one attached at spawn does.
pub fn append_docs(mut prompt: String, attached_docs: &[String]) -> String {
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
    fn try_parse_is_strict_and_command_matches() {
        // Strict: unknown is None, but every real stage round-trips.
        assert_eq!(Stage::try_parse("general"), Some(Stage::General));
        assert_eq!(Stage::try_parse("ship"), Some(Stage::Ship));
        assert_eq!(Stage::try_parse("nonsense"), None);

        // The command table is the single source compose_prompt reads from.
        assert_eq!(Stage::Research.command(), Some("/research_codebase"));
        assert_eq!(Stage::Validate.command(), Some("/validate_plan"));
        assert_eq!(Stage::General.command(), None);
        assert_eq!(Stage::Ship.command(), None);
        // compose_prompt uses the command, so the two never drift.
        let composed = compose_prompt(Stage::Plan, "x", &[]);
        assert!(composed.starts_with(Stage::Plan.command().unwrap()));
    }

    #[test]
    fn general_passes_input_through_verbatim() {
        assert_eq!(compose_prompt(Stage::General, "  hello  ", &[]), "hello");
        // Plain chat sends no skill prefix, but explicitly attached docs (e.g. a
        // pasted screenshot) are still listed so the agent can read them.
        assert_eq!(
            compose_prompt(Stage::General, "hello", &["a.png".into()]),
            "hello\n\nRelevant documents:\n- a.png"
        );
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
    fn ship_sends_the_directive_not_a_skill() {
        // Button-driven ship has no input: the bare directive.
        assert_eq!(compose_prompt(Stage::Ship, "", &[]), SHIP_PROMPT);
        // Extra context is appended below the directive.
        assert_eq!(
            compose_prompt(Stage::Ship, "skip the changelog", &[]),
            format!("{SHIP_PROMPT}\n\nskip the changelog"),
        );
        // It does not invoke /commit as a slash-command.
        assert!(!compose_prompt(Stage::Ship, "", &[]).starts_with('/'));
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
