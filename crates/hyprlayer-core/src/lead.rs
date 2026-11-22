use crate::settings::GuardrailRule;

pub fn default_rules() -> Vec<GuardrailRule> {
    [
        (
            "merge-after-validation",
            "Merge only after validation",
            "Merge a PR only after a plan validation says promote.",
        ),
        (
            "git-and-gh",
            "Git through git and gh",
            "Every agent commits, pushes, opens PRs and merges with the git and gh CLIs, never with hyprlayer app actions.",
        ),
        (
            "ask-before-irreversible",
            "Ask before irreversible steps",
            "Force-push, deploys and deleting data wait for the user, even when the user delegated the work.",
        ),
        (
            "own-agents-only",
            "Leave other agents alone",
            "Never stop, kill or archive an agent you didn't start.",
        ),
        (
            "no-ai-attribution",
            "No AI attribution",
            "No co-author trailers or generated-with footers in commits or PR bodies.",
        ),
        (
            "prompts-to-composer",
            "Prompts go to the composer",
            "Ask the user with the AskUserQuestion tool, which shows above your composer. Never ask through notify_user or request_attention.",
        ),
        (
            "briefs-open-with-goal",
            "Briefs open with the goal",
            "Every brief opens with the goal. When the poteto-mode skill is installed, agents run in it.",
        ),
    ]
    .into_iter()
    .map(|(id, title, text)| GuardrailRule {
        id: id.into(),
        title: title.into(),
        text: text.into(),
        enabled: true,
    })
    .collect()
}
