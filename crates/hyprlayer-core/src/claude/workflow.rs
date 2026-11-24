use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::driver::parsed_to_events;
use super::events::SessionEvent;
use super::protocol::{self, ParsedLine};
use crate::error::{CoreError, Result};
use crate::index::watcher::{notify_to_core, signals_change};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLaunch {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub task_id: String,
}

pub fn workflow_launch_from_result(content: &str) -> Option<WorkflowLaunch> {
    let mut task_id = None;
    let mut run_dir = None;
    let mut run_id = None;
    for line in content.lines() {
        if let Some((_, rest)) = line.split_once("Task ID:") {
            task_id.get_or_insert_with(|| rest.trim().to_string());
        }
        if let Some((_, rest)) = line.split_once("Transcript dir:") {
            run_dir.get_or_insert_with(|| PathBuf::from(rest.trim()));
        }
        if let Some((_, rest)) = line.split_once("Run ID:") {
            run_id.get_or_insert_with(|| rest.trim().to_string());
        }
    }
    let run_dir = run_dir?;
    let run_id = run_id?;
    let task_id = task_id?;
    if run_dir.file_name().and_then(|n| n.to_str()) != Some(run_id.as_str()) {
        return None;
    }
    Some(WorkflowLaunch {
        run_dir,
        run_id,
        task_id,
    })
}

#[derive(Debug)]
pub struct WorkflowRun {
    pub run_dir: PathBuf,
    pub workflow_tool_use_id: String,
    pub armed_seq: i64,
    offsets: HashMap<PathBuf, u64>,
    opened: HashSet<String>,
    pending_open: HashSet<String>,
    seen_uses: HashSet<String>,
    seen_results: HashSet<String>,
}

#[derive(Debug, Default)]
pub struct WorkflowRuns {
    runs: HashMap<String, WorkflowRun>,
}

impl WorkflowRuns {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        launch: WorkflowLaunch,
        workflow_tool_use_id: String,
        armed_seq: i64,
    ) {
        self.register_seeded(
            launch,
            workflow_tool_use_id,
            armed_seq,
            HashSet::new(),
            HashSet::new(),
        );
    }

    pub fn register_seeded(
        &mut self,
        launch: WorkflowLaunch,
        workflow_tool_use_id: String,
        armed_seq: i64,
        seen_uses: HashSet<String>,
        seen_results: HashSet<String>,
    ) -> bool {
        let mut inserted = false;
        self.runs.entry(launch.run_id).or_insert_with(|| {
            inserted = true;
            WorkflowRun {
                run_dir: launch.run_dir,
                workflow_tool_use_id,
                armed_seq,
                offsets: HashMap::new(),
                opened: HashSet::new(),
                pending_open: HashSet::new(),
                seen_uses,
                seen_results,
            }
        });
        inserted
    }

    pub fn retain_armed_below(&mut self, cutoff: i64) {
        self.runs.retain(|_, run| run.armed_seq < cutoff);
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub fn read_tick(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        for (run_id, run) in self.runs.iter_mut() {
            run.read_tick(run_id, &mut events);
        }
        events
    }
}

impl WorkflowRun {
    fn read_tick(&mut self, run_id: &str, events: &mut Vec<SessionEvent>) {
        self.read_journal(run_id, events);
        self.drain_known_agents(run_id, events);
    }

    fn read_journal(&mut self, run_id: &str, events: &mut Vec<SessionEvent>) {
        let path = self.run_dir.join("journal.jsonl");
        let Some(lines) = read_new_lines(&path, &mut self.offsets) else {
            return;
        };
        for line in &lines {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(agent_id) = v.get("agentId").and_then(Value::as_str) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("started") => {
                    if !self.opened.contains(agent_id) {
                        self.pending_open.insert(agent_id.to_string());
                        self.drain_one(run_id, agent_id, events);
                    }
                }
                Some("result") => {
                    self.drain_one(run_id, agent_id, events);
                    let result = v.get("result").and_then(Value::as_str).unwrap_or_default();
                    self.close(run_id, agent_id, result, events);
                }
                _ => {}
            }
        }
    }

    fn drain_known_agents(&mut self, run_id: &str, events: &mut Vec<SessionEvent>) {
        let agent_ids: Vec<String> = self
            .opened
            .iter()
            .chain(self.pending_open.iter())
            .cloned()
            .collect();
        for agent_id in agent_ids {
            self.drain_one(run_id, &agent_id, events);
        }
    }

    fn drain_one(&mut self, run_id: &str, agent_id: &str, events: &mut Vec<SessionEvent>) {
        let path = self.run_dir.join(format!("agent-{agent_id}.jsonl"));
        let Some(lines) = read_new_lines(&path, &mut self.offsets) else {
            return;
        };
        let parent_id = format!("wf:{run_id}:{agent_id}");
        for line in &lines {
            let prompt = is_prompt_line(line);
            if prompt && self.pending_open.contains(agent_id) {
                let label =
                    agent_label_from_prompt_line(line).unwrap_or_else(|| agent_id.to_string());
                self.open(run_id, agent_id, label, events);
            }
            events.extend(keep_new_tool_events(
                parse_agent_line(line, &parent_id, prompt),
                &self.seen_uses,
                &self.seen_results,
            ));
        }
    }

    fn open(
        &mut self,
        run_id: &str,
        agent_id: &str,
        label: String,
        events: &mut Vec<SessionEvent>,
    ) {
        self.pending_open.remove(agent_id);
        let tool_id = format!("wf:{run_id}:{agent_id}");
        if self.opened.insert(agent_id.to_string()) && !self.seen_uses.contains(&tool_id) {
            events.push(SessionEvent::ToolUse {
                id: tool_id,
                name: "Agent".into(),
                input: json!({ "description": label }).to_string(),
                parent_id: Some(self.workflow_tool_use_id.clone()),
            });
        }
    }

    fn close(
        &mut self,
        run_id: &str,
        agent_id: &str,
        result: &str,
        events: &mut Vec<SessionEvent>,
    ) {
        let tool_id = format!("wf:{run_id}:{agent_id}");
        if !self.opened.contains(agent_id) {
            self.open(run_id, agent_id, agent_id.to_string(), events);
        }
        if !self.seen_results.contains(&tool_id) {
            events.push(SessionEvent::ToolResult {
                tool_use_id: tool_id,
                content: result.to_string(),
                is_error: false,
                parent_id: Some(self.workflow_tool_use_id.clone()),
            });
        }
        self.opened.remove(agent_id);
        self.pending_open.remove(agent_id);
    }
}

#[derive(Debug, Default)]
pub struct SessionWorkflows {
    pub runs: WorkflowRuns,
    armed: Option<(WorkflowWatch, Arc<Notify>)>,
}

impl SessionWorkflows {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    pub fn arm(
        &mut self,
        workflows_dir: &Path,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<(Arc<Notify>, bool)> {
        if let Some((_, notify)) = &self.armed {
            return Ok((notify.clone(), false));
        }
        let (watch, notify) = watch_workflows(workflows_dir, on_change)?;
        self.armed = Some((watch, notify.clone()));
        Ok((notify, true))
    }

    pub fn attach_reader(&mut self, handle: JoinHandle<()>) {
        match &mut self.armed {
            Some((watch, _)) => watch.attach_reader(handle),
            None => handle.abort(),
        }
    }
}

pub struct WorkflowWatch {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    reader: Option<JoinHandle<()>>,
}

impl WorkflowWatch {
    pub fn attach_reader(&mut self, handle: JoinHandle<()>) {
        if let Some(old) = self.reader.replace(handle) {
            old.abort();
        }
    }
}

impl Drop for WorkflowWatch {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
    }
}

impl std::fmt::Debug for WorkflowWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowWatch")
            .field("reader", &self.reader.is_some())
            .finish_non_exhaustive()
    }
}

pub fn watch_workflows(
    workflows_dir: &Path,
    on_change: Box<dyn Fn() + Send + Sync>,
) -> Result<(WorkflowWatch, Arc<Notify>)> {
    std::fs::create_dir_all(workflows_dir).map_err(CoreError::Io)?;

    let notify = Arc::new(Notify::new());
    let signal = notify.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| {
            if let Ok(events) = result {
                handle_batch(&events, &signal, on_change.as_ref());
            }
        },
    )
    .map_err(notify_to_core)?;
    debouncer
        .watch(workflows_dir, RecursiveMode::Recursive)
        .map_err(notify_to_core)?;

    Ok((
        WorkflowWatch {
            _debouncer: debouncer,
            reader: None,
        },
        notify,
    ))
}

fn handle_batch(events: &[DebouncedEvent], signal: &Notify, on_change: &(dyn Fn() + Send + Sync)) {
    if batch_signals_change(events) {
        signal.notify_one();
        on_change();
    }
}

fn batch_signals_change(events: &[DebouncedEvent]) -> bool {
    events.iter().any(|event| signals_change(&event.kind))
}

fn read_new_lines(path: &Path, offsets: &mut HashMap<PathBuf, u64>) -> Option<Vec<String>> {
    let mut file = File::open(path).ok()?;
    let start = offsets.get(path).copied().unwrap_or(0);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let Some(last_newline) = buf.iter().rposition(|&b| b == b'\n') else {
        return Some(Vec::new());
    };
    let consumed = last_newline as u64 + 1;
    let text = String::from_utf8_lossy(&buf[..consumed as usize]);
    let lines = text
        .lines()
        .map(str::to_string)
        .filter(|l| !l.trim().is_empty())
        .collect();
    offsets.insert(path.to_path_buf(), start + consumed);
    Some(lines)
}

fn parse_agent_line(line: &str, parent_id: &str, is_prompt_line: bool) -> Vec<SessionEvent> {
    if is_prompt_line || is_attachment_line(line) {
        return Vec::new();
    }
    parsed_to_events(stamp_parent(protocol::parse_line(line), parent_id))
}

fn keep_new_tool_events(
    events: Vec<SessionEvent>,
    seen_uses: &HashSet<String>,
    seen_results: &HashSet<String>,
) -> Vec<SessionEvent> {
    events
        .into_iter()
        .filter(|event| match event {
            SessionEvent::ToolUse { id, .. } => !seen_uses.contains(id),
            SessionEvent::ToolResult { tool_use_id, .. } => !seen_results.contains(tool_use_id),
            _ => false,
        })
        .collect()
}

fn is_attachment_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|v| v.get("type").and_then(Value::as_str) == Some("attachment"))
}

fn is_prompt_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|v| matches!(v.get("parentUuid"), Some(Value::Null)))
}

fn stamp_parent(parsed: ParsedLine, parent_id: &str) -> ParsedLine {
    match parsed {
        ParsedLine::Assistant {
            blocks,
            context_tokens,
            error,
            ..
        } => ParsedLine::Assistant {
            blocks,
            context_tokens,
            parent_id: Some(parent_id.to_string()),
            error,
        },
        ParsedLine::User {
            blocks,
            uuid,
            is_replay,
            is_synthetic,
            ..
        } => ParsedLine::User {
            blocks,
            parent_id: Some(parent_id.to_string()),
            uuid,
            is_replay,
            is_synthetic,
        },
        other => other,
    }
}

fn agent_label_from_prompt_line(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    let content = v.get("message")?.get("content")?.as_str()?;
    Some(agent_label(content))
}

fn agent_label(prompt: &str) -> String {
    let prompt = prompt.trim();
    let mut lines = prompt.lines();
    let first = lines.next().unwrap_or("");
    let after_blank = lines
        .skip_while(|l| !l.trim().is_empty())
        .find(|l| !l.trim().is_empty());
    truncate_label(after_blank.unwrap_or(first))
}

const LABEL_MAX_CHARS: usize = 200;

fn truncate_label(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= LABEL_MAX_CHARS {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(LABEL_MAX_CHARS).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_LAUNCH_RESULT: &str = "Workflow launched in background. Task ID: whjv5o0xq\nSummary: Map how hyprlayer-app drives spec-driven dev stages, and what each transition actually writes\nTranscript dir: /home/jt/.claude/projects/slug/session/subagents/workflows/wf_89a9bc27-4f1\nScript file: /home/jt/.claude/projects/slug/session/workflows/scripts/map-stage-driving-wf_89a9bc27-4f1.js\n(Edit this file with Write/Edit and re-invoke Workflow with {scriptPath: \"…\"} to iterate without resending the script.)\nRun ID: wf_89a9bc27-4f1\nTo resume after editing the script: Workflow({scriptPath: \"…\", resumeFromRunId: \"wf_89a9bc27-4f1\"}) — completed agents return cached results (…).\n\nYou will be notified when it completes. Use /workflows to watch live progress.";

    const PROMPT_LINE: &str = r#"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"Repo root: /repo/checkout. Read-only.\n\nMap what happens when the app launches an agent session."},"uuid":"u1"}"#;
    const TOOL_USE_LINE: &str = r#"{"parentUuid":"u1","type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#;
    const TOOL_RESULT_LINE: &str = r#"{"parentUuid":"u2","type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}}"#;

    #[test]
    fn workflow_launch_from_result_parses_the_real_shape() {
        let launch = workflow_launch_from_result(REAL_LAUNCH_RESULT).expect("should parse");
        assert_eq!(
            launch.run_dir,
            PathBuf::from(
                "/home/jt/.claude/projects/slug/session/subagents/workflows/wf_89a9bc27-4f1"
            )
        );
        assert_eq!(launch.run_id, "wf_89a9bc27-4f1");
        assert_eq!(launch.task_id, "whjv5o0xq");
    }

    #[test]
    fn workflow_launch_from_result_returns_none_for_non_workflow_result() {
        let bash_output = "total 12\ndrwxr-xr-x  3 jt jt 4096 Aug 22 10:00 .\n-rw-r--r--  1 jt jt  123 Aug 22 10:00 Cargo.toml\n";
        assert!(workflow_launch_from_result(bash_output).is_none());
        assert!(workflow_launch_from_result("").is_none());
    }

    #[test]
    fn workflow_launch_from_result_handles_a_wrapped_summary() {
        let wrapped = "Workflow launched in background. Task ID: t1\nSummary: This summary is long enough that the model\nwrapped it across three physical lines before finally\nfinishing the sentence here.\nTranscript dir: /a/b/wf_x1\nRun ID: wf_x1\n\nYou will be notified when it completes.";
        let launch = workflow_launch_from_result(wrapped).expect("should parse despite wrapping");
        assert_eq!(launch.run_dir, PathBuf::from("/a/b/wf_x1"));
        assert_eq!(launch.run_id, "wf_x1");
        assert_eq!(launch.task_id, "t1");
    }

    #[test]
    fn workflow_launch_from_result_returns_none_on_run_id_mismatch() {
        let mismatched = "Workflow launched in background. Task ID: t1\nTranscript dir: /a/b/wf_x1\nRun ID: wf_DIFFERENT\n";
        assert!(workflow_launch_from_result(mismatched).is_none());
    }

    #[test]
    fn workflow_launch_from_result_requires_every_field() {
        assert!(workflow_launch_from_result("Task ID: t1\nRun ID: wf_x1\n").is_none());
        assert!(workflow_launch_from_result("Task ID: t1\nTranscript dir: /a/wf_x1\n").is_none());
        assert!(workflow_launch_from_result("Transcript dir: /a/wf_x1\nRun ID: wf_x1\n").is_none());
    }

    #[test]
    fn parse_agent_line_drops_an_attachment_line() {
        let line = r#"{"parentUuid":"p1","isSidechain":true,"agentId":"a1","attachment":{"type":"skill_listing","names":["a","b"]},"type":"attachment","uuid":"u1"}"#;
        assert!(parse_agent_line(line, "wf:run:a1", false).is_empty());
    }

    #[test]
    fn parse_agent_line_still_reports_a_genuinely_unknown_line() {
        let line = r#"{"type":"brand_new_event_kind","x":1}"#;
        let parsed = parse_agent_line(line, "wf:run:a1", false);
        assert!(
            matches!(parsed.as_slice(), [SessionEvent::ProtocolError { .. }]),
            "expected a ProtocolError, got {parsed:?}"
        );
        assert!(keep_new_tool_events(parsed, &HashSet::new(), &HashSet::new()).is_empty());
    }

    #[test]
    fn parse_agent_line_drops_the_prompt_line_whatever_its_content_shape() {
        let array_of_blocks = r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":[{"type":"text","text":"Repo root: /repo. Read-only."}]}}"#;
        for line in [PROMPT_LINE, array_of_blocks] {
            assert!(
                parse_agent_line(line, "wf:run:a1", true).is_empty(),
                "a prompt line must yield no events: {line}"
            );
        }
        assert!(!parse_agent_line(array_of_blocks, "wf:run:a1", false).is_empty());
    }

    #[test]
    fn parse_agent_line_stamps_parent_id_and_emits_no_usage() {
        let assistant = r#"{"parentUuid":"u1","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"thinking out loud"},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":100}}}"#;
        let events = parse_agent_line(assistant, "wf:run:a1", false);
        assert_eq!(events.len(), 2, "text + tool_use, no usage: {events:?}");
        for event in &events {
            let parent_id = match event {
                SessionEvent::AssistantText { parent_id, .. } => parent_id,
                SessionEvent::ToolUse { parent_id, .. } => parent_id,
                other => panic!("unexpected event kind: {other:?}"),
            };
            assert_eq!(parent_id.as_deref(), Some("wf:run:a1"));
        }
    }

    #[test]
    fn is_prompt_line_matches_only_an_explicitly_null_parent_uuid() {
        assert!(is_prompt_line(PROMPT_LINE));
        assert!(!is_prompt_line(TOOL_USE_LINE));
        assert!(!is_prompt_line(
            r#"{"type":"started","key":"k","agentId":"a1"}"#
        ));
        assert!(!is_prompt_line("not json"));
    }

    #[test]
    fn agent_label_takes_the_line_after_the_first_blank() {
        assert_eq!(
            agent_label(
                "Repo root: /repo/checkout. Read-only.\n\nMap the STAGE MODEL of the pipeline."
            ),
            "Map the STAGE MODEL of the pipeline."
        );
        assert_eq!(
            agent_label("  Preamble.\n   \n\n  The assignment.\nMore detail."),
            "The assignment."
        );
        assert_eq!(
            agent_label("One long single-line prompt."),
            "One long single-line prompt."
        );
        assert_eq!(agent_label("Only a preamble.\n\n"), "Only a preamble.");
    }

    #[test]
    fn agent_label_from_prompt_line_is_none_for_non_string_content() {
        let array_content =
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        assert!(agent_label_from_prompt_line(array_content).is_none());
        assert!(agent_label_from_prompt_line("not json").is_none());
    }

    #[test]
    fn truncate_label_caps_a_long_line() {
        let long = "x".repeat(400);
        let out = truncate_label(&long);
        assert_eq!(out.chars().count(), LABEL_MAX_CHARS + 1);
        assert!(out.ends_with('…'));
        assert_eq!(truncate_label("  padded  "), "padded");
    }

    fn write_run(dir: &Path, journal: &[&str], agents: &[(&str, &[&str])]) {
        std::fs::create_dir_all(dir).unwrap();
        let mut journal_text = String::new();
        for line in journal {
            journal_text.push_str(line);
            journal_text.push('\n');
        }
        std::fs::write(dir.join("journal.jsonl"), journal_text).unwrap();
        for (agent_id, lines) in agents {
            let mut text = String::new();
            for line in *lines {
                text.push_str(line);
                text.push('\n');
            }
            std::fs::write(dir.join(format!("agent-{agent_id}.jsonl")), text).unwrap();
        }
    }

    fn append(path: &Path, text: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn tool_use_line(id: &str) -> String {
        format!(
            r#"{{"parentUuid":"u1","type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"Bash","input":{{"command":"ls"}}}}]}}}}"#
        )
    }

    fn ids(events: &[SessionEvent]) -> Vec<&str> {
        events
            .iter()
            .map(|e| match e {
                SessionEvent::ToolUse { id, .. } => id.as_str(),
                SessionEvent::ToolResult { tool_use_id, .. } => tool_use_id.as_str(),
                other => panic!("unexpected event kind: {other:?}"),
            })
            .collect()
    }

    fn launch(run_dir: PathBuf, run_id: &str) -> WorkflowLaunch {
        WorkflowLaunch {
            run_dir,
            run_id: run_id.to_string(),
            task_id: "t1".into(),
        }
    }

    fn registered(run_dir: PathBuf, run_id: &str) -> WorkflowRuns {
        let mut runs = WorkflowRuns::new();
        runs.register(
            WorkflowLaunch {
                run_dir,
                run_id: run_id.to_string(),
                task_id: "t1".into(),
            },
            "toolu_workflow".into(),
            7,
        );
        runs
    }

    #[test]
    fn a_tick_reads_the_journal_before_the_agent_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
            &[("a1", &[PROMPT_LINE, &tool_use_line("toolu_1")])],
        );
        let mut runs = registered(run_dir.clone(), "wf_t");
        assert_eq!(ids(&runs.read_tick()), ["wf:wf_t:a1", "toolu_1"]);

        append(
            &run_dir.join("agent-a1.jsonl"),
            &format!("{}\n", tool_use_line("toolu_2")),
        );
        std::fs::write(
            run_dir.join("agent-a2.jsonl"),
            format!("{PROMPT_LINE}\n{}\n", tool_use_line("toolu_3")),
        )
        .unwrap();
        append(
            &run_dir.join("journal.jsonl"),
            "{\"type\":\"started\",\"key\":\"k2\",\"agentId\":\"a2\"}\n",
        );

        assert_eq!(
            ids(&runs.read_tick()),
            ["wf:wf_t:a2", "toolu_3", "toolu_2"],
            "the journal's new agent opens before an older agent's new content"
        );
    }

    #[test]
    fn a_seen_tool_use_does_not_suppress_its_own_tool_result() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[
                r#"{"type":"started","key":"k1","agentId":"a1"}"#,
                r#"{"type":"result","key":"k1","agentId":"a1","result":"done"}"#,
            ],
            &[("a1", &[PROMPT_LINE, TOOL_USE_LINE, TOOL_RESULT_LINE])],
        );
        let mut runs = registered(run_dir, "wf_t");

        let run = runs.runs.get_mut("wf_t").unwrap();
        run.seen_uses.insert("toolu_1".into());
        run.seen_uses.insert("wf:wf_t:a1".into());

        let events = runs.read_tick();
        let uses: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let results: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            uses.is_empty(),
            "both ToolUses were already persisted: {uses:?}"
        );
        assert!(
            results.contains(&"toolu_1"),
            "the agent's own ToolResult must still close its row: {results:?}"
        );
        assert!(
            results.contains(&"wf:wf_t:a1"),
            "the synthetic parent's ToolResult must still close its row: {results:?}"
        );
    }

    #[test]
    fn register_on_an_already_present_run_is_a_no_op() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
            &[("a1", &[PROMPT_LINE, TOOL_USE_LINE])],
        );
        let mut runs = registered(run_dir.clone(), "wf_t");
        assert!(!runs.read_tick().is_empty());
        let offsets = runs.runs["wf_t"].offsets.clone();
        assert!(!offsets.is_empty(), "the first tick advanced the cursors");

        runs.register(
            WorkflowLaunch {
                run_dir,
                run_id: "wf_t".into(),
                task_id: "t2".into(),
            },
            "toolu_other".into(),
            99,
        );

        let run = &runs.runs["wf_t"];
        assert_eq!(run.offsets, offsets, "cursors must survive a re-register");
        assert_eq!(run.workflow_tool_use_id, "toolu_workflow");
        assert_eq!(run.armed_seq, 7);
        assert!(
            runs.read_tick().is_empty(),
            "a re-register must not replay what the surviving cursors already covered"
        );
    }

    #[test]
    fn a_deferred_open_is_retried_once_the_agent_file_appears() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
            &[],
        );
        let mut runs = registered(run_dir.clone(), "wf_t");
        assert!(
            runs.read_tick().is_empty(),
            "no agent file yet: nothing to label, so nothing to emit"
        );

        std::fs::write(
            run_dir.join("agent-a1.jsonl"),
            format!("{PROMPT_LINE}\n{TOOL_USE_LINE}\n"),
        )
        .unwrap();

        let events = runs.read_tick();
        match events.first() {
            Some(SessionEvent::ToolUse {
                id, name, input, ..
            }) => {
                assert_eq!(id, "wf:wf_t:a1");
                assert_eq!(name, "Agent");
                assert!(
                    input.contains("Map what happens when the app launches an agent session."),
                    "the deferred open carries the real label, not the bare agent id: {input}"
                );
            }
            other => panic!("expected the synthetic parent ToolUse first, got {other:?}"),
        }
        assert!(events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { id, .. } if id == "toolu_1")));
    }

    #[test]
    fn chunked_agent_file_append_matches_a_single_read() {
        fn run(root: &Path, writes: &[&str]) -> Vec<SessionEvent> {
            let run_dir = root.join("wf_t");
            write_run(
                &run_dir,
                &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
                &[("a1", &[])],
            );
            let agent_path = run_dir.join("agent-a1.jsonl");
            let mut runs = registered(run_dir, "wf_t");

            let mut events = Vec::new();
            for chunk in writes {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&agent_path)
                    .unwrap();
                f.write_all(chunk.as_bytes()).unwrap();
                drop(f);
                events.extend(runs.read_tick());
            }
            events
        }

        let full = format!("{PROMPT_LINE}\n{TOOL_USE_LINE}\n");

        let single_dir = tempfile::TempDir::new().unwrap();
        let single = run(single_dir.path(), &[&full]);

        let split_at = PROMPT_LINE.len() + 1 + TOOL_USE_LINE.len() / 2;
        let (first, rest) = full.split_at(split_at);
        let split_dir = tempfile::TempDir::new().unwrap();
        let split = run(split_dir.path(), &[first, rest]);

        assert_eq!(single.len(), 2, "synthetic parent + the agent's tool_use");
        assert_eq!(
            serde_json::to_string_pretty(&single).unwrap(),
            serde_json::to_string_pretty(&split).unwrap(),
            "chunked read must match a single read"
        );
    }

    #[test]
    fn a_long_journal_result_is_emitted_verbatim() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        let long = "x".repeat(30_000);
        write_run(
            &run_dir,
            &[
                r#"{"type":"started","key":"k1","agentId":"a1"}"#,
                &format!(r#"{{"type":"result","key":"k1","agentId":"a1","result":"{long}"}}"#),
            ],
            &[("a1", &[PROMPT_LINE])],
        );
        let events = registered(run_dir, "wf_t").read_tick();
        let closed = events
            .iter()
            .find_map(|e| match e {
                SessionEvent::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "wf:wf_t:a1" => Some(content),
                _ => None,
            })
            .expect("the agent's synthetic parent should close");
        assert_eq!(closed, &long);
    }

    #[test]
    fn a_batch_of_reads_does_not_signal_a_workflow_change() {
        use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind};
        use notify::{Event, EventKind};

        let journal = PathBuf::from("/s/subagents/workflows/wf_x/journal.jsonl");
        let at = std::time::Instant::now();
        let batch = |kinds: &[EventKind]| -> Vec<DebouncedEvent> {
            kinds
                .iter()
                .map(|kind| DebouncedEvent::new(Event::new(*kind).add_path(journal.clone()), at))
                .collect()
        };

        let reads = [
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ];
        assert!(
            !batch_signals_change(&batch(&reads)),
            "a whole batch of reads is this reader's own tail coming back"
        );

        let append = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        assert!(batch_signals_change(&batch(&[append])));
        assert!(batch_signals_change(&batch(&[EventKind::Create(
            CreateKind::File
        )])));
        assert!(batch_signals_change(&batch(&[
            reads[0], reads[1], append, reads[2]
        ])));
        assert!(!batch_signals_change(&batch(&[])));
    }

    #[test]
    fn the_callback_ignores_a_read_only_batch_at_its_call_site() {
        use notify::event::{AccessKind, AccessMode, DataChange, ModifyKind};
        use notify::{Event, EventKind};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let journal = PathBuf::from("/s/subagents/workflows/wf_x/journal.jsonl");
        let at = std::time::Instant::now();
        let batch = |kinds: &[EventKind]| -> Vec<DebouncedEvent> {
            kinds
                .iter()
                .map(|kind| DebouncedEvent::new(Event::new(*kind).add_path(journal.clone()), at))
                .collect()
        };

        let signal = Notify::new();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = fired.clone();
        let on_change: Box<dyn Fn() + Send + Sync> = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let reads = batch(&[
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ]);
        handle_batch(&reads, &signal, on_change.as_ref());
        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "this reader's own reads must not reach on_change — that is the CPU spin"
        );

        let append = batch(&[EventKind::Modify(ModifyKind::Data(DataChange::Any))]);
        handle_batch(&append, &signal, on_change.as_ref());
        assert_eq!(
            fired.load(Ordering::SeqCst),
            1,
            "a real append must still reach on_change"
        );
    }

    #[test]
    fn watch_workflows_creates_the_directory_it_arms() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("session/subagents/workflows");
        assert!(!dir.exists(), "the fixture starts with nothing on disk");

        let (watch, _notify) =
            watch_workflows(&dir, Box::new(|| {})).expect("arming must not fail on a missing dir");
        assert!(dir.is_dir(), "watch_workflows creates what it arms");
        drop(watch);
    }

    #[test]
    fn a_second_run_joins_the_sessions_one_watch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workflows_dir = tmp.path().join("subagents/workflows");
        let mut session = SessionWorkflows::new();
        assert!(!session.is_armed());

        assert!(session.runs.register_seeded(
            launch(workflows_dir.join("wf_a"), "wf_a"),
            "toolu_a".into(),
            3,
            HashSet::new(),
            HashSet::new(),
        ));
        let (first, armed) = session.arm(&workflows_dir, Box::new(|| {})).unwrap();
        assert!(armed, "the first launch arms the session's watch");
        assert!(session.is_armed());

        assert!(session.runs.register_seeded(
            launch(workflows_dir.join("wf_b"), "wf_b"),
            "toolu_b".into(),
            9,
            HashSet::new(),
            HashSet::new(),
        ));
        let (second, armed_again) = session.arm(&workflows_dir, Box::new(|| {})).unwrap();
        assert!(!armed_again, "the second run must not arm a second watcher");
        assert!(
            Arc::ptr_eq(&first, &second),
            "and it must wake the reader the first one already started"
        );
        assert_eq!(session.runs.len(), 2, "both runs are being tailed");
    }

    #[test]
    fn a_rewind_drops_only_the_runs_it_cut() {
        let mut runs = WorkflowRuns::new();
        runs.register(
            launch(PathBuf::from("/a/wf_below"), "wf_below"),
            "t1".into(),
            4,
        );
        runs.register(launch(PathBuf::from("/a/wf_at"), "wf_at"), "t2".into(), 7);
        runs.register(
            launch(PathBuf::from("/a/wf_above"), "wf_above"),
            "t3".into(),
            9,
        );

        runs.retain_armed_below(7);

        assert_eq!(runs.len(), 1);
        assert!(runs.runs.contains_key("wf_below"));
        assert!(!runs.runs.contains_key("wf_at"));
        assert!(!runs.runs.contains_key("wf_above"));
    }

    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn reading_a_watched_file_does_not_fire_the_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let workflows_dir = tmp.path().join("session/subagents/workflows");
        let run_dir = workflows_dir.join("wf_existing");
        std::fs::create_dir_all(&run_dir).unwrap();
        let journal = run_dir.join("journal.jsonl");
        std::fs::write(
            &journal,
            "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\"}\n",
        )
        .unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let (watch, _notify) = watch_workflows(
            &workflows_dir,
            Box::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let baseline = hits.load(Ordering::SeqCst);

        for _ in 0..5 {
            let mut file = File::open(&journal).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
        }

        std::thread::sleep(Duration::from_millis(600));
        drop(watch);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            baseline,
            "reading the watched file must not fire on_change on its own"
        );
    }

    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn an_append_under_a_new_run_directory_fires_the_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let workflows_dir = tmp.path().join("session/subagents/workflows");

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let (watch, _notify) = watch_workflows(
            &workflows_dir,
            Box::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(400));

        let run_dir = workflows_dir.join("wf_new");
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(
            run_dir.join("journal.jsonl"),
            "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\"}\n",
        )
        .unwrap();

        let fired = (0..50).any(|_| {
            if hits.load(Ordering::SeqCst) > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
            false
        });
        drop(watch);
        assert!(
            fired,
            "an append under a newly created wf_* must fire on_change"
        );
    }
}
