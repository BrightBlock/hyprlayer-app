use std::time::Duration;

use crate::claude::SessionEvent;

pub(crate) const RECAP_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) const RECAP_SYSTEM: &str =
    "You recap a coding agent's last turn for the developer who ran it.";

const RECAP_INSTRUCTIONS: &str = "Recap this turn for the developer in one or two short sentences: what they asked and what got done, then their next step if one is needed; if none is, don't mention next steps. Address the developer as \"you\" and never write \"I\" or \"me\". Plain text, no preamble.";

const TOOL_INPUT_CHARS: usize = 120;
const EXCERPT_BYTES: usize = 8_000;

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The last turn, from its user message on, as lines a model can recap; `None` when nothing happened.
pub(crate) fn recap_excerpt(events: &[SessionEvent]) -> Option<String> {
    let start = events
        .iter()
        .rposition(|e| matches!(e, SessionEvent::UserMessage { .. }))?;
    let SessionEvent::UserMessage { text: asked, .. } = &events[start] else {
        return None;
    };

    let mut steps = Vec::new();
    let mut subagent_steps = 0usize;
    for event in &events[start + 1..] {
        match event {
            SessionEvent::ToolUse {
                parent_id: Some(_), ..
            } => subagent_steps += 1,
            SessionEvent::ToolUse { name, input, .. } => {
                steps.push(format!("Tool {name}: {}", clip(input, TOOL_INPUT_CHARS)));
            }
            SessionEvent::ToolResult {
                is_error: true,
                parent_id: None,
                content,
                ..
            } => steps.push(format!("Tool failed: {}", clip(content, TOOL_INPUT_CHARS))),
            SessionEvent::AssistantText {
                text,
                parent_id: None,
                injected: false,
            } if !text.trim().is_empty() => steps.push(format!("Agent: {}", text.trim())),
            _ => {}
        }
    }
    // Over budget: keep the ask and the final reply, drop steps from the middle.
    let last = clip(&steps.pop()?, EXCERPT_BYTES / 2);
    if subagent_steps > 0 {
        steps.push(format!("({subagent_steps} subagent tool calls)"));
    }

    let head = format!("User asked: {}", clip(asked, 2_000));
    let mut budget = EXCERPT_BYTES.saturating_sub(head.len() + last.len());
    let mut kept = Vec::new();
    let mut omitted = 0usize;
    for step in steps {
        if step.len() < budget {
            budget -= step.len() + 1;
            kept.push(step);
        } else {
            omitted += 1;
        }
    }
    if omitted > 0 {
        kept.push(format!("({omitted} steps omitted)"));
    }
    let mut lines = vec![head];
    lines.extend(kept);
    lines.push(last);
    Some(lines.join("\n"))
}

pub(crate) fn recap_prompt(excerpt: &str) -> String {
    format!("{RECAP_INSTRUCTIONS}\n\n<turn>\n{excerpt}\n</turn>")
}
