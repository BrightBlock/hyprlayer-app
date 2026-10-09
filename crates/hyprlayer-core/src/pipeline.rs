use crate::agent::Harness;
use crate::claude::PermissionMode;

pub use hyprlayer_models::agent::Stage;

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
        // The plan skill has its own approval gate; Claude's plan mode would move the plan out of thoughts.
        Stage::Plan => StagePolicy {
            permission_mode: PermissionMode::AcceptEdits,
            allowed_tools: allow(&["Write", "Edit"]),
            disallowed_tools: deny(&["Bash(git push:*)", "EnterPlanMode"]),
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

pub const COMMIT_PROMPT_NO_ASK_TOOL: &str = "/commit\n\nAfter the commit lands, ask whether to \
also push the branch and its new commits to the remote, and push only if the user confirms. \
Never open a pull request from here. That's Ship.";

pub const COMMIT_PROMPT_CODEX_PLAN: &str = "/commit\n\nThis is a planning turn: do not \
stage, commit, or modify anything — no `git add`, no `git commit`, no file writes. Review \
the working tree with a small number of read-only commands (`git status --short`, \
`git diff --stat`, and at most a few targeted diffs), then propose how you would split and \
word the commit. Finish by asking the user, in a single structured question tool call, two \
things: whether to proceed with the commit as proposed, and whether to also push the branch \
and its new commits to the remote afterwards. Stop once the answers arrive — the next turn \
executes them. Never open a pull request. That's Ship.";

pub const COMMIT_PROMPT_CODEX_EXECUTE: &str = "Now execute the commit. If you have not yet \
reviewed the working tree, review it now with read-only git commands first. Stage and commit \
per the plan and the user's answers. Push only if the user approved a push; if they declined, \
or if no answer was recorded, do not push. Never open a pull request. That's Ship.";

pub const PLAN_PROMPT_CODEX_SUFFIX: &str = "\n\nThis is a collaboration turn: you cannot \
write files and must not try. Read whatever you need, then ask every open question in a \
single structured question tool call, with concrete options, and draft the plan in the \
conversation for the user to react to. Do not write the plan document this turn — the next \
turn does that.";

pub const PLAN_PROMPT_CODEX_WRITE: &str = "Now write the plan document exactly as \
/create_plan specifies — same path, same frontmatter, same structure — incorporating the \
answers above. You are the only agent in this session: there is no parent to hand a draft \
back to and no subagent to delegate the writing to, so create the file yourself with your \
own tools rather than printing the plan in the conversation. If the previous turn produced \
no draft, do the research now and write the plan anyway. Ask no further questions and \
implement nothing.";

pub const RESEARCH_PROMPT_CODEX_SUFFIX: &str = "\n\nThis is a collaboration turn: you cannot \
write files and must not try. Explore whatever you need, then ask every open question in a \
single structured question tool call, with concrete options, and summarise what you found in \
the conversation for the user to react to. Do not write the research document this turn — the \
next turn does that.";

pub const RESEARCH_PROMPT_CODEX_WRITE: &str = "Now write the research document exactly as \
/research_codebase specifies — same path, same frontmatter, same structure — incorporating the \
answers above. You are the only agent in this session: there is no parent to hand findings \
back to and no subagent to delegate the writing to, so create the file yourself with your own \
tools rather than printing the document in the conversation. If the previous turn produced no \
findings, do the research now and write the document anyway. Ask no further questions and \
change no code.";

pub fn review_notes_path(workstream_id: &str) -> String {
    format!(".hyprlayer/review/{workstream_id}.jsonl")
}

/// One paragraph: it travels as a single argv value and is handed to subagents verbatim.
pub fn review_notes_instruction(workstream_id: &str) -> String {
    let path = review_notes_path(workstream_id);
    format!(
        "As you implement, leave review notes for the human who will review this diff: append one \
JSON object per line to `{path}` (create the directory if needed) with keys `path` \
(repo-relative), `line` (1-based, in the file as you leave it), `kind` (`note` or \
`outside_plan_scope`), `body` (one or two sentences, markdown), and `task` (the plan phase or \
task you were on). The reviewer reads each note beside its line without the plan or this \
conversation, so a note must stand on its own: name what it is about, and never lean on plan \
shorthand, earlier messages, or other notes. Leave a note only where the diff hides something \
the reviewer needs: why the code takes a non-obvious approach (a constraint, a gotcha, a \
rejected alternative), or an effect that reaches beyond the lines shown. Do not restate what \
the code does, narrate your steps, or leave a TODO. Use `outside_plan_scope` for any change the \
plan did not ask for, and say why it was needed. A note is never a question: settle each \
decision while you implement, and if one is genuinely the human's to make, stop and ask in the \
conversation before you build it. Few notes is normal; none is fine. Never stage or commit this \
file. When you delegate work to a subagent, include this paragraph verbatim in its prompt."
    )
}

/// Claude carries the instruction in its system prompt and Codex in `developer_instructions`.
pub fn implement_notes_suffix(
    harness: Harness,
    stage: Stage,
    workstream_id: &str,
) -> Option<String> {
    (harness == Harness::Opencode && stage == Stage::Implement)
        .then(|| format!("\n\n{}", review_notes_instruction(workstream_id)))
}

pub fn commit_prompt_default(harness: Harness) -> &'static str {
    match harness {
        Harness::Claude => COMMIT_PROMPT_DEFAULT,
        Harness::Codex => COMMIT_PROMPT_CODEX_PLAN,
        Harness::Opencode => COMMIT_PROMPT_NO_ASK_TOOL,
    }
}

pub fn codex_phase_two(
    harness: Harness,
    stage: Stage,
    bodies: &ActionPromptBodies,
) -> Option<&'static str> {
    if harness != Harness::Codex {
        return None;
    }
    match stage {
        Stage::Commit if bodies.commit.is_none() => Some(COMMIT_PROMPT_CODEX_EXECUTE),
        Stage::Plan => Some(PLAN_PROMPT_CODEX_WRITE),
        Stage::Research => Some(RESEARCH_PROMPT_CODEX_WRITE),
        _ => None,
    }
}

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
    harness: Harness,
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
            bodies
                .commit
                .as_deref()
                .unwrap_or_else(|| commit_prompt_default(harness)),
            input,
        ),
        Stage::Plan if harness == Harness::Codex => {
            let mut p = skill_prompt(Stage::Plan.command().expect("plan has a command"), input);
            p.push_str(PLAN_PROMPT_CODEX_SUFFIX);
            p
        }
        Stage::Research if harness == Harness::Codex => {
            let mut p = skill_prompt(
                Stage::Research.command().expect("research has a command"),
                input,
            );
            p.push_str(RESEARCH_PROMPT_CODEX_SUFFIX);
            p
        }
        _ => skill_prompt(
            stage.command().expect("skill-driven stage has a command"),
            input,
        ),
    }
}

/// The user's own text inside a first turn `compose_prompt` + `append_docs`
/// wrapped, for rows that predate `first_input`. Action-body stages are not
/// recoverable.
pub fn recover_input(stage: Stage, prompt: &str) -> Option<String> {
    let prompt = prompt.split(DOCS_HEADER).next().unwrap_or(prompt);
    let input = match (stage, stage.command()) {
        (Stage::General, _) => prompt,
        (Stage::Commit | Stage::Ship | Stage::Revise, _) | (_, None) => return None,
        (_, Some(command)) => {
            let rest = prompt.strip_prefix(command)?.strip_prefix(' ')?;
            [PLAN_PROMPT_CODEX_SUFFIX, RESEARCH_PROMPT_CODEX_SUFFIX]
                .iter()
                .find_map(|suffix| rest.strip_suffix(suffix))
                .unwrap_or(rest)
        }
    };
    let input = input.trim();
    (!input.is_empty()).then(|| input.to_string())
}

const DOCS_HEADER: &str = "\n\nRelevant documents:";

pub fn append_docs(mut prompt: String, attached_docs: &[String]) -> String {
    if !attached_docs.is_empty() {
        prompt.push_str(DOCS_HEADER);
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

    fn compose(stage: Stage, input: &str, base: &str, bodies: &ActionPromptBodies) -> String {
        compose_prompt(stage, input, base, bodies, Harness::Claude)
    }

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
        let composed = compose(Stage::Plan, "x", "main", &Default::default());
        assert!(composed.starts_with(Stage::Plan.command().unwrap()));
    }

    #[test]
    fn general_passes_input_through_verbatim() {
        assert_eq!(
            compose(Stage::General, "  hello  ", "main", &Default::default()),
            "hello"
        );
        assert_eq!(
            append_docs(
                compose(Stage::General, "hello", "main", &Default::default()),
                &["a.png".into()],
            ),
            "hello\n\nRelevant documents:\n- a.png"
        );
    }

    #[test]
    fn research_invokes_the_skill() {
        assert_eq!(
            compose(
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
            compose(Stage::Plan, "build it", "main", &Default::default()),
            &["repos/p/shared/research/2026-auth.md".into()],
        );
        assert!(prompt.starts_with("/create_plan build it"));
        assert!(prompt.contains("Relevant documents:"));
        assert!(prompt.contains("- repos/p/shared/research/2026-auth.md"));
    }

    #[test]
    fn empty_input_still_invokes_the_skill() {
        assert_eq!(
            compose(Stage::Research, "", "main", &Default::default()),
            "/research_codebase"
        );
    }

    #[test]
    fn ship_sends_the_directive_not_a_skill() {
        assert_eq!(
            compose(Stage::Ship, "", "main", &Default::default()),
            ship_prompt("main")
        );
        assert!(compose(Stage::Ship, "", "develop", &Default::default()).contains("`develop`"));
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
            compose(
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
            compose(Stage::Commit, "", "main", &Default::default()),
            COMMIT_PROMPT_DEFAULT
        );
        assert_eq!(
            compose(
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
    fn claude_commit_still_names_the_ask_tool() {
        assert_eq!(
            commit_prompt_default(Harness::Claude),
            COMMIT_PROMPT_DEFAULT
        );
        assert!(COMMIT_PROMPT_DEFAULT.contains("AskUserQuestion"));
        assert!(compose_prompt(
            Stage::Commit,
            "",
            "main",
            &Default::default(),
            Harness::Claude
        )
        .contains("AskUserQuestion"));
    }

    #[test]
    fn opencode_commit_keeps_the_prose_prompt() {
        assert!(COMMIT_PROMPT_NO_ASK_TOOL.starts_with("/commit\n"));
        assert!(!COMMIT_PROMPT_NO_ASK_TOOL.contains("AskUserQuestion"));
        assert_eq!(
            commit_prompt_default(Harness::Opencode),
            COMMIT_PROMPT_NO_ASK_TOOL
        );
        assert_eq!(
            compose_prompt(
                Stage::Commit,
                "",
                "main",
                &Default::default(),
                Harness::Opencode
            ),
            COMMIT_PROMPT_NO_ASK_TOOL
        );
    }

    #[test]
    fn codex_commit_asks_before_it_commits() {
        assert_eq!(
            commit_prompt_default(Harness::Codex),
            COMMIT_PROMPT_CODEX_PLAN
        );
        let composed = compose_prompt(
            Stage::Commit,
            "",
            "main",
            &Default::default(),
            Harness::Codex,
        );
        assert!(composed.starts_with("/commit"));
        assert!(!composed.contains("AskUserQuestion"));
        assert!(composed.contains("no `git add`"));
        assert!(composed.contains("no `git commit`"));
        assert!(composed.contains("whether to also push the branch"));
    }

    #[test]
    fn codex_commit_phase_two_stands_alone() {
        let phase_two = codex_phase_two(Harness::Codex, Stage::Commit, &Default::default());
        assert_eq!(phase_two, Some(COMMIT_PROMPT_CODEX_EXECUTE));
        assert!(phase_two
            .unwrap()
            .contains("If you have not yet reviewed the working tree"));
    }

    #[test]
    fn a_custom_commit_body_stays_single_turn() {
        let bodies = ActionPromptBodies {
            commit: Some("do it".into()),
            ship: None,
        };
        assert_eq!(
            codex_phase_two(Harness::Codex, Stage::Commit, &bodies),
            None
        );
        assert_eq!(
            compose_prompt(Stage::Commit, "", "main", &bodies, Harness::Codex),
            "do it"
        );
    }

    #[test]
    fn no_other_harness_two_phases() {
        for harness in [Harness::Claude, Harness::Opencode] {
            for stage in [Stage::Commit, Stage::Plan, Stage::Research] {
                assert_eq!(
                    codex_phase_two(harness, stage, &Default::default()),
                    None,
                    "{harness:?} {stage:?} must stay a single turn"
                );
            }
        }
        for stage in [
            Stage::General,
            Stage::Implement,
            Stage::Validate,
            Stage::Ship,
            Stage::Revise,
        ] {
            assert_eq!(
                codex_phase_two(Harness::Codex, stage, &Default::default()),
                None
            );
        }
    }

    #[test]
    fn claude_plan_is_still_a_bare_skill_invocation() {
        assert_eq!(
            compose(Stage::Plan, "x", "main", &Default::default()),
            "/create_plan x"
        );
    }

    #[test]
    fn codex_plan_collaborates_before_it_writes() {
        let composed = compose_prompt(
            Stage::Plan,
            "x",
            "main",
            &Default::default(),
            Harness::Codex,
        );
        assert!(composed.starts_with("/create_plan x"));
        assert!(composed.contains("you cannot write files"));
        assert!(composed.contains("single structured question tool call"));
        assert!(composed.contains("Do not write the plan document this turn"));

        let phase_two = codex_phase_two(Harness::Codex, Stage::Plan, &Default::default());
        assert_eq!(phase_two, Some(PLAN_PROMPT_CODEX_WRITE));
        assert!(phase_two
            .unwrap()
            .contains("If the previous turn produced no draft"));
        assert!(phase_two
            .unwrap()
            .contains("no subagent to delegate the writing to"));
    }

    #[test]
    fn opencode_plan_is_unchanged() {
        assert_eq!(
            compose_prompt(
                Stage::Plan,
                "x",
                "main",
                &Default::default(),
                Harness::Opencode
            ),
            compose(Stage::Plan, "x", "main", &Default::default())
        );
    }

    #[test]
    fn claude_research_is_still_a_bare_skill_invocation() {
        assert_eq!(
            compose(Stage::Research, "x", "main", &Default::default()),
            "/research_codebase x"
        );
    }

    #[test]
    fn codex_research_collaborates_before_it_writes() {
        let composed = compose_prompt(
            Stage::Research,
            "x",
            "main",
            &Default::default(),
            Harness::Codex,
        );
        assert!(composed.starts_with("/research_codebase x"));
        assert!(composed.contains("you cannot write files"));
        assert!(composed.contains("single structured question tool call"));
        assert!(composed.contains("Do not write the research document this turn"));

        let phase_two = codex_phase_two(Harness::Codex, Stage::Research, &Default::default());
        assert_eq!(phase_two, Some(RESEARCH_PROMPT_CODEX_WRITE));
        assert!(phase_two
            .unwrap()
            .contains("If the previous turn produced no findings"));
        assert!(phase_two
            .unwrap()
            .contains("no subagent to delegate the writing to"));
    }

    #[test]
    fn opencode_research_is_unchanged() {
        assert_eq!(
            compose_prompt(
                Stage::Research,
                "x",
                "main",
                &Default::default(),
                Harness::Opencode
            ),
            compose(Stage::Research, "x", "main", &Default::default())
        );
    }

    #[test]
    fn revise_addresses_feedback_without_a_skill() {
        assert_eq!(Stage::parse("revise"), Stage::Revise);
        assert_eq!(Stage::Revise.as_str(), "revise");
        assert_eq!(Stage::Revise.command(), None);
        assert_eq!(
            compose(Stage::Revise, "", "main", &Default::default()),
            REVISE_PROMPT
        );
        assert_eq!(
            compose(Stage::Revise, "CI is failing", "main", &Default::default()),
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
            compose(Stage::Commit, "", "main", &bodies),
            "Review the tree on feature/x and commit."
        );
        assert_eq!(
            compose(Stage::Commit, "split into two", "main", &bodies),
            "Review the tree on feature/x and commit.\n\nsplit into two"
        );
        let slash = ActionPromptBodies {
            commit: Some("/commit --strict".into()),
            ship: None,
        };
        assert_eq!(
            compose(Stage::Commit, "now", "main", &slash),
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
            compose(Stage::Ship, "", "main", &bodies),
            "Push and open a PR against `develop`. Link HYP-1."
        );
        assert_eq!(
            compose(Stage::Commit, "", "main", &bodies),
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
    fn only_the_plan_stage_denies_claudes_own_plan_mode() {
        let plan = policy_for(Stage::Plan);
        assert!(plan.disallowed_tools.iter().any(|t| t == "EnterPlanMode"));
        // A plan-mode session must still be able to present its plan and leave.
        assert!(!plan.disallowed_tools.iter().any(|t| t == "ExitPlanMode"));
        for stage in [Stage::General, Stage::Research, Stage::Implement] {
            assert!(!policy_for(stage)
                .disallowed_tools
                .iter()
                .any(|t| t == "EnterPlanMode"));
        }
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
