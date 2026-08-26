use crate::claude::PermissionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

pub struct StagePolicy {
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

const READ_ONLY: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "Task",
    "Agent",
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

pub fn policy_for(stage: Stage) -> StagePolicy {
    match stage {
        Stage::General => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&[]),
            disallowed_tools: deny(&[]),
        },
        Stage::Research => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&["Write", "Edit"]),
            disallowed_tools: deny(&["Bash(git push:*)"]),
        },
        Stage::Plan => StagePolicy {
            permission_mode: PermissionMode::AcceptEdits,
            allowed_tools: allow(&["Write", "Edit"]),
            disallowed_tools: deny(&["Bash(git push:*)"]),
        },
        Stage::Implement => StagePolicy {
            permission_mode: PermissionMode::AcceptEdits,
            allowed_tools: allow(&["Write", "Edit", "Bash(git add:*)", "Bash(git commit:*)"]),
            disallowed_tools: deny(&["Bash(git push:*)"]),
        },
        Stage::Validate => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&[]),
            disallowed_tools: deny(&["Write", "Edit", "Bash(git commit:*)"]),
        },
        Stage::Commit => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&["Bash(git add:*)", "Bash(git commit:*)"]),
            disallowed_tools: deny(&["Bash(gh:*)"]),
        },
        Stage::Ship => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&SHIP_TOOLS),
            disallowed_tools: deny(&["Skill"]),
        },
        Stage::Revise => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&SHIP_TOOLS),
            disallowed_tools: deny(&[]),
        },
    }
}

const SHIP_TOOLS: [&str; 7] = [
    "Write",
    "Edit",
    "Bash(git add:*)",
    "Bash(git commit:*)",
    "Bash(git push:*)",
    "Bash(gh pr:*)",
    "Bash(gh:*)",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAgent {
    pub name: &'static str,
    pub description: &'static str,
    pub prompt: &'static str,
}

pub const SHIP_AGENT: SessionAgent = SessionAgent {
    name: "hyprlayer-ship",
    description: "Commits, pushes, and opens the pull request for one workstream branch.",
    prompt: "You publish one branch: commit the worktree, push it, open the pull request, \
then write its description. Nothing else. Do not review, refactor, or fix code on the way \
through, and do not improvise a PR body when a later step will write it.\n\n\
The two skills you need (`/commit`, `/describe_pr`) are handed to you as slash commands \
already expanded into the conversation. You cannot invoke skills yourself and never need to.",
};

pub fn agent_for(stage: Stage) -> Option<SessionAgent> {
    match stage {
        Stage::Ship => Some(SHIP_AGENT),
        _ => None,
    }
}

pub const SHIP_PROMPT_TEMPLATE: &str = "/commit\n\nOnce the commit lands, push the branch to \
its remote. Only after the commit and push succeed, open a pull request against \
`{{base_branch}}` with a clear title. Leave the body as a short placeholder, since the description \
is written for you in a follow-up step, so do not improvise one.";

pub const SHIP_DESCRIBE_PROMPT: &str = "/describe_pr";

pub const COMMIT_PROMPT_DEFAULT: &str = "/commit\n\nAfter the commit lands, use the \
AskUserQuestion tool to ask whether to also push the branch and its new commits to the \
remote, and push only if the user confirms. Never open a pull request from here. That's \
Ship.";

pub fn ship_prompt(base: &str) -> String {
    let base = base.strip_prefix("origin/").unwrap_or(base);
    SHIP_PROMPT_TEMPLATE.replace("{{base_branch}}", base)
}

#[derive(Debug, Clone, Default)]
pub struct PromptVars {
    pub branch: String,
    pub base_branch: String,
    pub changed_files: String,
    pub diff: String,
    pub last_commit: String,
}

pub fn substitute_vars(body: &str, vars: &PromptVars) -> String {
    let tokens: [(&str, &str); 5] = [
        ("{{branch}}", &vars.branch),
        ("{{base_branch}}", &vars.base_branch),
        ("{{changed_files}}", &vars.changed_files),
        ("{{diff}}", &vars.diff),
        ("{{last_commit}}", &vars.last_commit),
    ];
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    loop {
        let next = tokens
            .iter()
            .filter_map(|&(tok, val)| rest.find(tok).map(|i| (i, tok, val)))
            .min_by_key(|&(i, _, _)| i);
        match next {
            Some((i, tok, val)) => {
                out.push_str(&rest[..i]);
                out.push_str(val);
                rest = &rest[i + tok.len()..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

pub const REVISE_PROMPT: &str =
    "A pull request already exists for this branch. Address the outstanding feedback, then \
commit and push the fixes to the same branch. Do not open a new pull request.";

fn skill_prompt(skill: &str, input: &str) -> String {
    if input.is_empty() {
        skill.to_string()
    } else {
        format!("{skill} {input}")
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionPromptBodies {
    pub commit: Option<String>,
    pub ship: Option<String>,
}

fn action_prompt(body: &str, input: &str) -> String {
    if input.is_empty() {
        body.to_string()
    } else if body.starts_with('/') && !body.contains('\n') {
        format!("{body} {input}")
    } else {
        format!("{body}\n\n{input}")
    }
}

pub fn compose_prompt(
    stage: Stage,
    input: &str,
    base_branch: &str,
    bodies: &ActionPromptBodies,
) -> String {
    let input = input.trim();
    match stage {
        Stage::General => input.to_string(),
        Stage::Ship => {
            let body = bodies
                .ship
                .clone()
                .unwrap_or_else(|| ship_prompt(base_branch));
            action_prompt(&body, input)
        }
        Stage::Revise => action_prompt(REVISE_PROMPT, input),
        Stage::Commit => action_prompt(
            bodies.commit.as_deref().unwrap_or(COMMIT_PROMPT_DEFAULT),
            input,
        ),
        _ => skill_prompt(
            stage.command().expect("skill-driven stage has a command"),
            input,
        ),
    }
}

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
        assert_eq!(Stage::try_parse("general"), Some(Stage::General));
        assert_eq!(Stage::try_parse("ship"), Some(Stage::Ship));
        assert_eq!(Stage::try_parse("nonsense"), None);

        assert_eq!(Stage::Research.command(), Some("/research_codebase"));
        assert_eq!(Stage::Validate.command(), Some("/validate_plan"));
        assert_eq!(Stage::General.command(), None);
        assert_eq!(Stage::Ship.command(), None);
        let composed = compose_prompt(Stage::Plan, "x", "main", &Default::default());
        assert!(composed.starts_with(Stage::Plan.command().unwrap()));
    }

    #[test]
    fn general_passes_input_through_verbatim() {
        assert_eq!(
            compose_prompt(Stage::General, "  hello  ", "main", &Default::default()),
            "hello"
        );
        assert_eq!(
            append_docs(
                compose_prompt(Stage::General, "hello", "main", &Default::default()),
                &["a.png".into()],
            ),
            "hello\n\nRelevant documents:\n- a.png"
        );
    }

    #[test]
    fn research_invokes_the_skill() {
        assert_eq!(
            compose_prompt(
                Stage::Research,
                "how does auth work?",
                "main",
                &Default::default()
            ),
            "/research_codebase how does auth work?"
        );
    }

    #[test]
    fn plan_lists_attached_docs() {
        let prompt = append_docs(
            compose_prompt(Stage::Plan, "build it", "main", &Default::default()),
            &["repos/p/shared/research/2026-auth.md".into()],
        );
        assert!(prompt.starts_with("/create_plan build it"));
        assert!(prompt.contains("Relevant documents:"));
        assert!(prompt.contains("- repos/p/shared/research/2026-auth.md"));
    }

    #[test]
    fn empty_input_still_invokes_the_skill() {
        assert_eq!(
            compose_prompt(Stage::Research, "", "main", &Default::default()),
            "/research_codebase"
        );
    }

    #[test]
    fn ship_sends_the_directive_not_a_skill() {
        assert_eq!(
            compose_prompt(Stage::Ship, "", "main", &Default::default()),
            ship_prompt("main")
        );
        assert!(
            compose_prompt(Stage::Ship, "", "develop", &Default::default()).contains("`develop`")
        );
        assert_eq!(ship_prompt("origin/develop"), ship_prompt("develop"));
        assert!(ship_prompt("origin/main").contains("`main`"));
        assert_eq!(
            ship_prompt("main"),
            "/commit\n\nOnce the commit lands, push the branch to its remote. Only after the \
commit and push succeed, open a pull request against `main` with a clear title. Leave the body \
as a short placeholder, since the description is written for you in a follow-up step, so do not \
improvise one."
        );
        assert_eq!(
            compose_prompt(
                Stage::Ship,
                "skip the changelog",
                "main",
                &Default::default()
            ),
            format!("{}\n\nskip the changelog", ship_prompt("main")),
        );
        assert!(ship_prompt("main").starts_with("/commit\n"));
        assert!(ship_prompt("main").contains('\n'));
        assert!(!ship_prompt("main").contains("/describe_pr"));
        assert_eq!(SHIP_DESCRIBE_PROMPT, "/describe_pr");
        assert!(SHIP_DESCRIBE_PROMPT.starts_with('/'));
        assert!(!SHIP_DESCRIBE_PROMPT.contains('\n'));
    }

    #[test]
    fn ship_runs_as_an_agent_that_cannot_invoke_skills() {
        let ship = policy_for(Stage::Ship);
        assert!(ship.disallowed_tools.iter().any(|t| t == "Skill"));
        let revise = policy_for(Stage::Revise);
        assert!(revise.disallowed_tools.is_empty());
        assert_eq!(ship.allowed_tools, revise.allowed_tools);

        assert_eq!(agent_for(Stage::Ship), Some(SHIP_AGENT));
        assert_eq!(agent_for(Stage::Commit), None);
        assert_eq!(agent_for(Stage::General), None);
        assert!(!SHIP_AGENT.name.is_empty());
        assert!(!SHIP_AGENT.prompt.is_empty());
    }

    #[test]
    fn commit_invokes_the_skill_then_offers_a_push_but_never_a_pr() {
        assert_eq!(Stage::parse("commit"), Stage::Commit);
        assert_eq!(Stage::Commit.as_str(), "commit");
        assert_eq!(Stage::Commit.command(), Some("/commit"));
        assert!(COMMIT_PROMPT_DEFAULT.starts_with("/commit\n"));
        assert!(COMMIT_PROMPT_DEFAULT.contains("AskUserQuestion"));
        assert_eq!(
            compose_prompt(Stage::Commit, "", "main", &Default::default()),
            COMMIT_PROMPT_DEFAULT
        );
        assert_eq!(
            compose_prompt(
                Stage::Commit,
                "split into two commits",
                "main",
                &Default::default()
            ),
            format!("{COMMIT_PROMPT_DEFAULT}\n\nsplit into two commits"),
        );
        let commit = policy_for(Stage::Commit);
        assert!(commit
            .allowed_tools
            .iter()
            .any(|t| t.contains("git commit")));
        assert!(!commit.allowed_tools.iter().any(|t| t.contains("git push")));
        assert!(!commit
            .disallowed_tools
            .iter()
            .any(|t| t.contains("git push")));
        assert!(commit.disallowed_tools.iter().any(|t| t == "Bash(gh:*)"));
    }

    #[test]
    fn revise_addresses_feedback_without_a_skill() {
        assert_eq!(Stage::parse("revise"), Stage::Revise);
        assert_eq!(Stage::Revise.as_str(), "revise");
        assert_eq!(Stage::Revise.command(), None);
        assert_eq!(
            compose_prompt(Stage::Revise, "", "main", &Default::default()),
            REVISE_PROMPT
        );
        assert_eq!(
            compose_prompt(Stage::Revise, "CI is failing", "main", &Default::default()),
            format!("{REVISE_PROMPT}\n\nCI is failing"),
        );
        let revise = policy_for(Stage::Revise);
        assert!(revise.allowed_tools.iter().any(|t| t.contains("git push")));
        assert!(revise.allowed_tools.iter().any(|t| t.contains("gh pr")));
    }

    #[test]
    fn substitute_vars_replaces_known_tokens_only() {
        let vars = PromptVars {
            branch: "feature/x".into(),
            base_branch: "main".into(),
            changed_files: "4 files (+182 −40)".into(),
            diff: "diff --git a b".into(),
            last_commit: "fix(tabs): debounce".into(),
        };
        assert_eq!(
            substitute_vars("On {{branch}} → {{base_branch}}: {{changed_files}}", &vars),
            "On feature/x → main: 4 files (+182 −40)"
        );
        assert_eq!(
            substitute_vars("{{last_commit}}\n{{diff}}", &vars),
            "fix(tabs): debounce\ndiff --git a b"
        );
        assert_eq!(
            substitute_vars("{{ticket}} on {{branch}}", &PromptVars::default()),
            "{{ticket}} on "
        );
        let tricky = PromptVars {
            diff: "+ body.contains(\"{{last_commit}}\")".into(),
            last_commit: "SHOULD NOT APPEAR".into(),
            ..Default::default()
        };
        assert_eq!(
            substitute_vars("{{diff}}", &tricky),
            "+ body.contains(\"{{last_commit}}\")"
        );
    }

    #[test]
    fn overridden_commit_body_replaces_the_skill_invocation() {
        let bodies = ActionPromptBodies {
            commit: Some("Review the tree on feature/x and commit.".into()),
            ship: None,
        };
        assert_eq!(
            compose_prompt(Stage::Commit, "", "main", &bodies),
            "Review the tree on feature/x and commit."
        );
        assert_eq!(
            compose_prompt(Stage::Commit, "split into two", "main", &bodies),
            "Review the tree on feature/x and commit.\n\nsplit into two"
        );
        let slash = ActionPromptBodies {
            commit: Some("/commit --strict".into()),
            ship: None,
        };
        assert_eq!(
            compose_prompt(Stage::Commit, "now", "main", &slash),
            "/commit --strict now"
        );
    }

    #[test]
    fn overridden_ship_body_replaces_the_directive() {
        let bodies = ActionPromptBodies {
            commit: None,
            ship: Some("Push and open a PR against `develop`. Link HYP-1.".into()),
        };
        assert_eq!(
            compose_prompt(Stage::Ship, "", "main", &bodies),
            "Push and open a PR against `develop`. Link HYP-1."
        );
        assert_eq!(
            compose_prompt(Stage::Commit, "", "main", &bodies),
            COMMIT_PROMPT_DEFAULT
        );
        assert_eq!(
            substitute_vars(
                SHIP_PROMPT_TEMPLATE,
                &PromptVars {
                    base_branch: "main".into(),
                    ..Default::default()
                }
            ),
            ship_prompt("main")
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
        assert_eq!(ship.disallowed_tools, vec!["Skill".to_string()]);

        for stage in [Stage::General, Stage::Research, Stage::Implement] {
            assert!(policy_for(stage).allowed_tools.iter().any(|t| t == "Read"));
        }
    }
}
