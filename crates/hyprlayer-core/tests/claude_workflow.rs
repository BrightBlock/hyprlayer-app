use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use hyprlayer_core::claude::events::SessionEvent;
use hyprlayer_core::claude::workflow::{WorkflowLaunch, WorkflowRuns};

const WORKFLOW_TOOL_USE_ID: &str = "toolu_workflow_launch_1";

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/workflow")
        .join(name)
}

fn register(run_dir: PathBuf, run_id: &str) -> WorkflowRuns {
    let mut runs = WorkflowRuns::new();
    runs.register(
        WorkflowLaunch {
            run_dir,
            run_id: run_id.to_string(),
            task_id: "whjv5o0xq".to_string(),
        },
        WORKFLOW_TOOL_USE_ID.to_string(),
        1,
    );
    runs
}

fn parent_id_of(event: &SessionEvent) -> Option<&str> {
    match event {
        SessionEvent::ToolUse { parent_id, .. } => parent_id.as_deref(),
        SessionEvent::ToolResult { parent_id, .. } => parent_id.as_deref(),
        _ => None,
    }
}

fn synthetic_parents(events: &[SessionEvent], run_id: &str) -> Vec<(usize, String, String)> {
    let prefix = format!("wf:{run_id}:");
    events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            SessionEvent::ToolUse {
                id, name, input, ..
            } if name == "Agent" && id.starts_with(&prefix) => {
                let parsed: Value = serde_json::from_str(input).expect("input is JSON");
                let label = parsed["description"]
                    .as_str()
                    .expect("the synthetic parent carries a description")
                    .to_string();
                Some((i, id.clone(), label))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn workflow_reader_parses_the_complete_fixture_into_paired_agent_events() {
    let mut runs = register(fixture_dir("wf_89a9bc27-4f1"), "wf_89a9bc27-4f1");
    let events = runs.read_tick();
    assert!(
        !events.is_empty(),
        "expected events from the complete fixture"
    );

    let mut opened_agents = HashSet::new();
    let mut closed_agents = HashSet::new();
    for event in &events {
        match event {
            SessionEvent::Usage { .. } => {
                panic!("no Usage event should ever come from a workflow file")
            }
            SessionEvent::ProtocolError { raw, .. } => {
                panic!("no ProtocolError expected from a real workflow tree, got: {raw}")
            }
            SessionEvent::ToolUse {
                id,
                name,
                parent_id,
                ..
            } => {
                assert!(
                    parent_id.is_some(),
                    "every emitted event must carry a parent_id"
                );
                if name == "Agent" && id.starts_with("wf:wf_89a9bc27-4f1:") {
                    opened_agents.insert(id.clone());
                }
            }
            SessionEvent::ToolResult {
                tool_use_id,
                parent_id,
                ..
            } => {
                assert!(
                    parent_id.is_some(),
                    "every emitted event must carry a parent_id"
                );
                if tool_use_id.starts_with("wf:wf_89a9bc27-4f1:") {
                    closed_agents.insert(tool_use_id.clone());
                }
            }
            other => panic!("unexpected event kind from the workflow reader: {other:?}"),
        }
    }

    assert_eq!(
        opened_agents.len(),
        8,
        "all 8 agents should have opened: {opened_agents:?}"
    );
    assert_eq!(
        closed_agents, opened_agents,
        "every opened agent should also have a matching close"
    );

    assert!(
        runs.read_tick().is_empty(),
        "a second tick over an unchanged tree must emit nothing"
    );
}

#[test]
fn workflow_reader_surfaces_each_agents_own_tool_calls() {
    let mut runs = register(fixture_dir("wf_89a9bc27-4f1"), "wf_89a9bc27-4f1");
    let events = runs.read_tick();

    let inner_tool_use = events.iter().any(|e| {
        matches!(e, SessionEvent::ToolUse { name, parent_id, .. }
            if name != "Agent" && parent_id.as_deref().is_some_and(|p| p.starts_with("wf:wf_89a9bc27-4f1:")))
    });
    assert!(
        inner_tool_use,
        "expected at least one agent-owned ToolUse nested under a synthetic parent"
    );
}

#[test]
fn workflow_reader_opens_a_synthetic_parent_before_any_of_its_children() {
    let mut runs = register(fixture_dir("wf_89a9bc27-4f1"), "wf_89a9bc27-4f1");
    let events = runs.read_tick();

    let parents = synthetic_parents(&events, "wf_89a9bc27-4f1");
    assert_eq!(parents.len(), 8);
    assert_eq!(
        parents[0].0, 0,
        "the journal's first `started` line opens the very first event of the tick"
    );

    for (open_at, id, _) in &parents {
        for (i, event) in events.iter().enumerate() {
            if parent_id_of(event) == Some(id.as_str()) {
                assert!(
                    i > *open_at,
                    "{id}'s child at index {i} precedes its own parent at {open_at}"
                );
            }
        }
    }
}

#[test]
fn workflow_reader_labels_the_eight_fixture_agents_distinctly() {
    let mut runs = register(fixture_dir("wf_89a9bc27-4f1"), "wf_89a9bc27-4f1");
    let events = runs.read_tick();

    let parents = synthetic_parents(&events, "wf_89a9bc27-4f1");
    assert_eq!(parents.len(), 8);

    let labels: Vec<&String> = parents.iter().map(|(_, _, label)| label).collect();
    let distinct: HashSet<&&String> = labels.iter().collect();
    assert_eq!(distinct.len(), 8, "eight distinct labels, got {labels:?}");

    for (_, id, label) in &parents {
        assert!(
            !label.starts_with("Repo root:"),
            "a label must be the agent's assignment, not the shared preamble: {label}"
        );
        let agent_id = id.rsplit(':').next().unwrap();
        assert_ne!(label, agent_id, "label fell back to the bare agent id");
    }
}

#[test]
fn workflow_reader_leaves_unfinished_agents_open_on_the_partial_fixture() {
    let mut runs = register(
        fixture_dir("wf_89a9bc27-4f1-partial"),
        "wf_89a9bc27-4f1-partial",
    );
    let events = runs.read_tick();

    let mut opened = HashSet::new();
    let mut closed = HashSet::new();
    for event in &events {
        match event {
            SessionEvent::ToolUse { id, name, .. } if name == "Agent" => {
                opened.insert(id.clone());
            }
            SessionEvent::ToolResult { tool_use_id, .. } if tool_use_id.starts_with("wf:") => {
                closed.insert(tool_use_id.clone());
            }
            _ => {}
        }
    }

    assert!(
        opened.len() > closed.len(),
        "expected more opened agents ({}) than closed ones ({}) in the partial fixture",
        opened.len(),
        closed.len()
    );
    let unmatched: Vec<&String> = opened.difference(&closed).collect();
    assert!(
        !unmatched.is_empty(),
        "expected at least one agent left open with no ToolResult"
    );
}

#[test]
fn a_seeded_re_arm_emits_exactly_what_the_persisted_log_is_missing() {
    let dir = fixture_dir("wf_89a9bc27-4f1");

    let complete = register(dir.clone(), "wf_89a9bc27-4f1").read_tick();
    assert!(complete.len() > 20, "fixture should be substantial");

    let cut = (1..complete.len())
        .rev()
        .find(|&i| leaves_both_row_kinds_open(&complete, i))
        .expect("the fixture must hold a cut leaving both row kinds open");
    let (persisted, missing) = complete.split_at(cut);
    let mut seen_uses = HashSet::new();
    let mut seen_results = HashSet::new();
    for event in persisted {
        match event {
            SessionEvent::ToolUse { id, .. } => {
                seen_uses.insert(id.clone());
            }
            SessionEvent::ToolResult { tool_use_id, .. } => {
                seen_results.insert(tool_use_id.clone());
            }
            _ => {}
        }
    }

    let mut runs = WorkflowRuns::new();
    assert!(
        runs.register_seeded(
            WorkflowLaunch {
                run_dir: dir,
                run_id: "wf_89a9bc27-4f1".to_string(),
                task_id: "whjv5o0xq".to_string(),
            },
            WORKFLOW_TOOL_USE_ID.to_string(),
            1,
            seen_uses,
            seen_results,
        ),
        "a fresh map must actually insert, or the seeds were silently dropped"
    );

    let replayed = runs.read_tick();
    assert_eq!(
        ids_of(&replayed),
        ids_of(missing),
        "a re-arm must emit the tail of the run and nothing else"
    );
}

fn leaves_both_row_kinds_open(events: &[SessionEvent], cut: usize) -> bool {
    let (head, tail) = events.split_at(cut);
    let opened: HashSet<&str> = head
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut synthetic = false;
    let mut agent_owned = false;
    for event in tail {
        if let SessionEvent::ToolResult { tool_use_id, .. } = event {
            if opened.contains(tool_use_id.as_str()) {
                if tool_use_id.starts_with("wf:") {
                    synthetic = true;
                } else {
                    agent_owned = true;
                }
            }
        }
    }
    synthetic && agent_owned
}

fn ids_of(events: &[SessionEvent]) -> Vec<(&'static str, &str)> {
    events
        .iter()
        .map(|e| match e {
            SessionEvent::ToolUse { id, .. } => ("toolUse", id.as_str()),
            SessionEvent::ToolResult { tool_use_id, .. } => ("toolResult", tool_use_id.as_str()),
            other => panic!("unexpected event kind: {other:?}"),
        })
        .collect()
}
