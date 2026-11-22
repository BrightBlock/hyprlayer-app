use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::docs::frontmatter::body_offset_bytes;
use crate::pipeline::SessionAgent;
use crate::settings::{GuardrailRule, LeadRuleScope, LeadSettings};

pub const LEAD_AGENT_NAME: &str = "hyprlayer-lead-engineer";

const PROMPT: &str = include_str!("lead_engineer.md");

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
        (
            "no-plan-unless-asked",
            "No plan step unless asked",
            "Plan, with an approval gate, only when the user asks for a plan. When the user says to start the implementation, start it.",
        ),
        (
            "validate-stacks-once",
            "Validate stacks once",
            "Stack multi-PR work. Run the full suite, the goldens, the live and perf checks and one plan validation once, at the top of the stack, then merge bottom-up.",
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

pub fn project_rules_path(checkout: &Path) -> PathBuf {
    checkout
        .join(".claude")
        .join("agents")
        .join("lead-engineer.md")
}

pub fn project_rules(checkout: &Path) -> Option<String> {
    let text = std::fs::read_to_string(project_rules_path(checkout)).ok()?;
    let body = text[body_offset_bytes(text.as_bytes())..].trim();
    (!body.is_empty()).then(|| body.to_string())
}

pub fn rules_in_effect(settings: &LeadSettings, checkout: &str) -> Option<String> {
    match settings.projects.get(checkout).copied().unwrap_or_default() {
        LeadRuleScope::InheritAndFile => project_rules(Path::new(checkout)),
        LeadRuleScope::InheritOnly => None,
    }
}

pub fn agent(settings: &LeadSettings, project_rules: Option<&str>) -> SessionAgent {
    let mut prompt = format!("{}\n\n## Guardrails\n\n", PROMPT.trim_end());
    for rule in settings.rules.iter().filter(|r| r.enabled) {
        prompt.push_str(&format!("- **{}.** {}\n", rule.title, rule.text));
    }
    if let Some(rules) = project_rules {
        prompt.push_str(&format!("\n## Project rules\n\n{rules}\n"));
    }
    SessionAgent {
        name: LEAD_AGENT_NAME,
        description: "Starts, checks and lands work in this project's workstreams.",
        prompt: Cow::Owned(prompt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> LeadSettings {
        let mut settings = LeadSettings::default();
        settings.rules[0].enabled = false;
        settings
    }

    #[test]
    fn the_prompt_carries_enabled_rules_only() {
        let agent = agent(&settings(), None);
        assert_eq!(agent.name, LEAD_AGENT_NAME);
        assert!(agent.prompt.starts_with("You are the Lead Engineer"));
        assert!(agent
            .prompt
            .contains("\n## Guardrails\n\n- **Git through git and gh.** Every agent commits"));
        assert!(!agent.prompt.contains("Merge only after validation"));
        assert!(!agent.prompt.contains("## Project rules"));
    }

    #[test]
    fn project_rules_follow_the_guardrails() {
        let agent = agent(&settings(), Some("Run `make check` before any PR."));
        assert!(agent
            .prompt
            .ends_with("\n## Project rules\n\nRun `make check` before any PR.\n"));
    }

    #[test]
    fn project_rules_drop_the_frontmatter_and_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(project_rules(dir.path()), None);
        let agents = dir.path().join(".claude").join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        let file = agents.join("lead-engineer.md");
        std::fs::write(&file, "---\nname: lead-engineer\n---\n\nShip small PRs.\n").unwrap();
        assert_eq!(
            project_rules(dir.path()).as_deref(),
            Some("Ship small PRs.")
        );
        std::fs::write(&file, "---\nname: lead-engineer\n---\n\n").unwrap();
        assert_eq!(project_rules(dir.path()), None);
    }
}
