//! Stage → skill-prompt composition and per-stage permission policy.
//!
//! Each spec-driven stage maps to (1) a first-prompt that invokes the matching
//! hyprlayer skill and (2) a permission posture: which tools are pre-approved,
//! which are hard-denied, and the claude `--permission-mode`. Anything not
//! pre-approved still works — it just routes through a `can_use_tool` prompt the
//! UI renders. The table is a reviewable starting policy, tightened over time.

use crate::claude::PermissionMode;

/// The pipeline stages — the canonical wire enum for a workstream/session's
/// pipeline position. sqlite still stores its `as_str` TEXT; rows read back
/// through the tolerant [`Stage::parse`]. Wire deserialization is strict — the
/// generated bindings union makes an unknown value a TS type error, and specta
/// cannot render `#[serde(other)]` on string enums.
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
            "commit" => Some(Stage::Commit),
            "ship" => Some(Stage::Ship),
            "revise" => Some(Stage::Revise),
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
            Stage::Commit => "commit",
            Stage::Ship => "ship",
            Stage::Revise => "revise",
        }
    }

    /// The skill slash-command this stage invokes, or `None` for stages that send
    /// a plain prompt (General chat; Ship and Revise send [`ship_prompt`] /
    /// [`REVISE_PROMPT`] prose, not a skill). Single source of truth for the
    /// command, mirrored by the frontend `stageCommand`
    /// (src/features/session/stages.ts) for its optimistic echo.
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
        // Commit checkpoints the worktree — stage + commit — then the stock
        // prompt asks the user whether to also push. Push is deliberately
        // NEITHER pre-approved NOR denied: it routes through the normal
        // permission ask, so nothing pushes without explicit consent (even if
        // an agent skips the prompt's question), but a user who wants the push
        // isn't blocked either. `gh` stays hard-denied so a commit can never
        // grow into a pull request (the explicit half of the split-out Ship flow).
        Stage::Commit => StagePolicy {
            permission_mode: PermissionMode::Default,
            allowed_tools: allow(&["Bash(git add:*)", "Bash(git commit:*)"]),
            disallowed_tools: deny(&["Bash(gh:*)"]),
        },
        // Ship and Revise both push to the branch and talk to `gh`; Revise
        // addresses PR feedback (CI failures, review comments) on a branch that
        // already has a PR, so it shares Ship's posture.
        Stage::Ship | Stage::Revise => StagePolicy {
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

/// The stock Ship body — a prompt template with `{{base_branch}}` resolved at
/// send time. Run in order: commit first, then (only once that succeeds) push,
/// open the PR against the workstream's target branch, and hand the description
/// off to the `/describe_pr` skill so every shipped PR follows the repo's
/// template instead of a free-improvised body. `/describe_pr` edits an existing
/// PR, so the PR must be opened before it runs. The policy pre-approves push +
/// `gh`. Naming the base (rather than "the default branch") lets a re-targeted
/// workstream open its PR against e.g. `develop`. Users can override this body
/// in Settings → Prompts; the stock text is also surfaced there via the
/// bootstrap's `promptDefaults`.
pub const SHIP_PROMPT_TEMPLATE: &str =
    "First run the /commit skill to commit all changes in this worktree, then push the branch \
to its remote. Only after the commit and push succeed, open a pull request against \
`{{base_branch}}` with a clear title and run the /describe_pr skill to write the PR \
description from the repository's template and update the PR.";

/// The stock Commit body: the `/commit` skill invocation, followed by an
/// explicit ask-before-push step (the AskUserQuestion tool renders as the app's
/// question UI). The stage policy leaves `git push` to the normal permission
/// ask — never silent, never blocked — while `gh` stays denied so a commit
/// never becomes a PR. Overridable in the Prompt library like
/// [`SHIP_PROMPT_TEMPLATE`].
pub const COMMIT_PROMPT_DEFAULT: &str = "/commit\n\nAfter the commit lands, use the \
AskUserQuestion tool to ask whether to also push the branch and its new commits to the \
remote, and push only if the user confirms. Never open a pull request from here — that's \
Ship.";

/// The stock Ship directive with the target branch resolved.
pub fn ship_prompt(base: &str) -> String {
    // The stored base is a git rev (e.g. `origin/main`); a PR targets the GitHub
    // branch name, so strip the remote prefix before naming it.
    let base = base.strip_prefix("origin/").unwrap_or(base);
    SHIP_PROMPT_TEMPLATE.replace("{{base_branch}}", base)
}

/// The `{{variable}}` values available to prompt bodies, resolved from the
/// workstream at send time. A missing value substitutes as an empty string;
/// tokens outside this set pass through untouched.
#[derive(Debug, Clone, Default)]
pub struct PromptVars {
    pub branch: String,
    /// The PR target branch, `origin/`-stripped.
    pub base_branch: String,
    /// Working-tree change summary, e.g. `"4 files (+182 −40)"`.
    pub changed_files: String,
    /// Working-tree diff vs HEAD, truncated (see `gitops::prompt_git_vars`).
    pub diff: String,
    /// Most recent commit subject.
    pub last_commit: String,
}

/// Replace the known `{{variable}}` tokens in a prompt body. Deliberately a
/// fixed token set (not a general template engine): unknown tokens stay
/// verbatim so a body that happens to contain `{{...}}` isn't mangled.
/// Single-pass over the ORIGINAL body — substituted values are never rescanned,
/// so token-shaped text inside a value (e.g. a `{{diff}}` hunk touching a file
/// that itself contains `{{last_commit}}`) survives verbatim. Mirrored by the
/// frontend `substitutePromptVars` (src/features/prompts/prompts.ts).
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

/// The Revise directive: address PR feedback on a branch that already has a PR.
/// The specific feedback (failing CI vs. requested changes) rides in the input.
/// Explicitly tells the agent NOT to open a second PR. Kept byte-identical to the
/// frontend `REVISE_PROMPT` (src/features/session/stages.ts).
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

/// The effective (already variable-substituted) bodies for the overridable
/// action prompts, resolved from settings by the caller. `None` = stock text.
#[derive(Debug, Clone, Default)]
pub struct ActionPromptBodies {
    pub commit: Option<String>,
    pub ship: Option<String>,
}

/// Join an action-prompt body with the user's extra input. A single-line
/// slash-command body keeps skill-argument form (`/commit split it` — the stock
/// Commit shape); a prose body gets the input as a trailing paragraph (the
/// stock Ship/Revise shape).
fn action_prompt(body: &str, input: &str) -> String {
    if input.is_empty() {
        body.to_string()
    } else if body.starts_with('/') && !body.contains('\n') {
        format!("{body} {input}")
    } else {
        format!("{body}\n\n{input}")
    }
}

/// Compose the first prompt for a stage: the matching skill invocation plus the
/// user's input. General is pure passthrough (plain chat); Ship, Revise and
/// Commit send their action bodies — the user's override from `bodies` when set
/// (Settings → Prompts), else the stock [`ship_prompt`] / [`REVISE_PROMPT`] /
/// [`COMMIT_PROMPT_DEFAULT`]. Ship names `base_branch` as the PR target; other
/// stages ignore it. Attached docs are appended separately via [`append_docs`]
/// (the caller sends the doc-suffixed prompt to the agent but echoes this one).
pub fn compose_prompt(
    stage: Stage,
    input: &str,
    base_branch: &str,
    bodies: &ActionPromptBodies,
) -> String {
    let input = input.trim();
    match stage {
        // Plain chat: the input verbatim — no skill prefix.
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
        // research/plan/implement/validate each invoke their skill.
        _ => skill_prompt(
            stage.command().expect("skill-driven stage has a command"),
            input,
        ),
    }
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
        let composed = compose_prompt(Stage::Plan, "x", "main", &Default::default());
        assert!(composed.starts_with(Stage::Plan.command().unwrap()));
    }

    #[test]
    fn general_passes_input_through_verbatim() {
        assert_eq!(
            compose_prompt(Stage::General, "  hello  ", "main", &Default::default()),
            "hello"
        );
        // Plain chat sends no skill prefix, but explicitly attached docs (e.g. a
        // pasted screenshot) are still listed via append_docs so the agent can
        // read them (the caller composes, then appends for the wire prompt).
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
        // Button-driven ship has no input: the bare directive, naming the base.
        assert_eq!(
            compose_prompt(Stage::Ship, "", "main", &Default::default()),
            ship_prompt("main")
        );
        // The target branch is named in the directive (git-flow re-targeting).
        assert!(
            compose_prompt(Stage::Ship, "", "develop", &Default::default()).contains("`develop`")
        );
        // The stored base is a git rev; the PR targets the branch, so `origin/`
        // is stripped — and the frontend `shipPrompt` mirrors this.
        assert_eq!(ship_prompt("origin/develop"), ship_prompt("develop"));
        assert!(ship_prompt("origin/main").contains("`main`"));
        // Golden literal — the stock directive users see in the Prompt library
        // and the wire text when no override is set. Committing goes through
        // the /commit skill so Ship's commits follow the same conventions.
        assert_eq!(
            ship_prompt("main"),
            "First run the /commit skill to commit all changes in this worktree, then push \
the branch to its remote. Only after the commit and push succeed, open a pull request \
against `main` with a clear title and run the /describe_pr skill to write the PR \
description from the repository's template and update the PR."
        );
        // Extra context is appended below the directive.
        assert_eq!(
            compose_prompt(
                Stage::Ship,
                "skip the changelog",
                "main",
                &Default::default()
            ),
            format!("{}\n\nskip the changelog", ship_prompt("main")),
        );
        // Ship is a prose directive (commit via the /commit skill, then push,
        // then PR) — not itself a bare slash-command invocation.
        assert!(!compose_prompt(Stage::Ship, "", "main", &Default::default()).starts_with('/'));
        assert!(ship_prompt("main").contains("/commit skill"));
    }

    #[test]
    fn commit_invokes_the_skill_then_offers_a_push_but_never_a_pr() {
        assert_eq!(Stage::parse("commit"), Stage::Commit);
        assert_eq!(Stage::Commit.as_str(), "commit");
        assert_eq!(Stage::Commit.command(), Some("/commit"));
        // The stock body leads with the /commit invocation and follows with the
        // ask-before-push step (the question is the push consent gate).
        assert!(COMMIT_PROMPT_DEFAULT.starts_with("/commit\n"));
        assert!(COMMIT_PROMPT_DEFAULT.contains("AskUserQuestion"));
        assert_eq!(
            compose_prompt(Stage::Commit, "", "main", &Default::default()),
            COMMIT_PROMPT_DEFAULT
        );
        // A multi-line body takes prose semantics: extra input rides below as a
        // paragraph (still reaching the /commit skill as its argument).
        assert_eq!(
            compose_prompt(
                Stage::Commit,
                "split into two commits",
                "main",
                &Default::default()
            ),
            format!("{COMMIT_PROMPT_DEFAULT}\n\nsplit into two commits"),
        );
        // Push is neither pre-approved nor denied — it must route through the
        // permission ask, so a push always needs explicit user consent. `gh`
        // stays hard-denied — a commit can never open a PR, that's Ship.
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
        // Bare directive when there's no extra context...
        assert_eq!(
            compose_prompt(Stage::Revise, "", "main", &Default::default()),
            REVISE_PROMPT
        );
        // ...the specific feedback is appended below it.
        assert_eq!(
            compose_prompt(Stage::Revise, "CI is failing", "main", &Default::default()),
            format!("{REVISE_PROMPT}\n\nCI is failing"),
        );
        // Revise shares Ship's push/gh posture.
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
        // Unknown tokens pass through verbatim; missing values become empty.
        assert_eq!(
            substitute_vars("{{ticket}} on {{branch}}", &PromptVars::default()),
            "{{ticket}} on "
        );
        // Single-pass: token-shaped text INSIDE a substituted value is never
        // re-substituted (a diff hunk may legitimately contain "{{last_commit}}").
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
        // Extra input rides below a prose body as a paragraph…
        assert_eq!(
            compose_prompt(Stage::Commit, "split into two", "main", &bodies),
            "Review the tree on feature/x and commit.\n\nsplit into two"
        );
        // …but a single-line slash-command override keeps skill-argument form.
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
        // The ship override never bleeds into Commit (and vice versa).
        assert_eq!(
            compose_prompt(Stage::Commit, "", "main", &bodies),
            COMMIT_PROMPT_DEFAULT
        );
        // The stock template names the base via the same substitution path.
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
        assert!(ship.disallowed_tools.is_empty());

        // Read-only tools are present in every stage.
        for stage in [Stage::General, Stage::Research, Stage::Implement] {
            assert!(policy_for(stage).allowed_tools.iter().any(|t| t == "Read"));
        }
    }
}
