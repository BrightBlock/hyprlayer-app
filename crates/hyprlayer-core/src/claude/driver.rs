use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::events::{ExitOutcome, SessionEvent};
use super::protocol::{self, Block, ParsedLine};
use crate::agent::KillFlag;
use crate::error::{CoreError, Result};
use crate::store::session_event_codec::questions_from_value;
use hyprlayer_models::session::QuestionAnswers;

pub use hyprlayer_models::agent::PermissionMode;

fn permission_mode_flag(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
        PermissionMode::BypassPermissions => "bypassPermissions",
    }
}

pub struct SpawnConfig {
    pub claude_bin: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
    pub permission_mode: PermissionMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub add_dirs: Vec<PathBuf>,
    pub resume: Option<String>,
    pub fork: bool,
    pub max_budget_usd: Option<f64>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub chrome: bool,
    pub agent: Option<crate::pipeline::SessionAgent>,
    pub env: Vec<(String, String)>,
    pub claude_config_dir: Option<PathBuf>,
}

const STDERR_RING_LINES: usize = 200;
const EVENT_CHANNEL_CAP: usize = 1024;

struct RewindOutcome {
    rewound: bool,
    error: Option<String>,
}

pub struct ClaudeDriver {
    session_id: String,
    stdin_tx: mpsc::UnboundedSender<String>,
    events: broadcast::Sender<SessionEvent>,
    control_seq: std::sync::atomic::AtomicU64,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    pending_permissions: Arc<StdMutex<HashMap<String, Value>>>,
    pending_controls: Arc<StdMutex<HashMap<String, oneshot::Sender<Value>>>>,
    killer: Killer,
    claude_config_dir: Option<PathBuf>,
}

impl ClaudeDriver {
    pub fn spawn(config: SpawnConfig) -> Result<(Self, broadcast::Receiver<SessionEvent>)> {
        let mut cmd = Command::new(&config.claude_bin);
        cmd.arg("-p")
            .args(["--output-format", "stream-json"])
            .args(["--input-format", "stream-json"])
            .arg("--verbose")
            .arg("--include-partial-messages")
            .args(["--permission-prompt-tool", "stdio"])
            .args([
                "--permission-mode",
                permission_mode_flag(config.permission_mode),
            ]);

        if let Some(resume) = &config.resume {
            cmd.args(["--resume", resume]);
            if config.fork {
                cmd.arg("--fork-session");
            }
        } else {
            cmd.args(["--session-id", &config.session_id]);
        }
        if !config.allowed_tools.is_empty() {
            cmd.args(["--allowedTools", &config.allowed_tools.join(" ")]);
        }
        if !config.disallowed_tools.is_empty() {
            cmd.args(["--disallowedTools", &config.disallowed_tools.join(" ")]);
        }
        for dir in &config.add_dirs {
            cmd.arg("--add-dir").arg(dir);
        }
        if let Some(budget) = config.max_budget_usd {
            cmd.args(["--max-budget-usd", &budget.to_string()]);
        }
        if let Some(model) = &config.model {
            cmd.args(["--model", model]);
        }
        if let Some(effort) = &config.effort {
            if effort == "ultracode" {
                cmd.args(["--effort", "xhigh"])
                    .args(["--settings", r#"{"ultracode":true,"enableWorkflows":true}"#]);
            } else {
                cmd.args(["--effort", effort]);
            }
        }
        if config.chrome {
            cmd.arg("--chrome");
        }
        if let Some(agent) = &config.agent {
            let defs = serde_json::json!({
                agent.name: {
                    "description": agent.description,
                    "prompt": agent.prompt,
                }
            });
            cmd.args(["--agents", &defs.to_string()])
                .args(["--agent", agent.name]);
        }

        crate::process::appimage::scrub_tokio(&mut cmd);
        cmd.current_dir(&config.cwd)
            .env("CLAUDE_CODE_ENTRYPOINT", "hyprlayer-app")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        #[cfg(unix)]
        cmd.process_group(0);

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = crate::process::spawn_supervised(&mut cmd).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::BinaryMissing {
                    name: "claude".into(),
                }
            } else {
                CoreError::Io(e)
            }
        })?;

        let pid = child.id();
        let stdin = child.take_stdin().expect("stdin piped");
        let stdout = child.take_stdout().expect("stdout piped");
        let stderr = child.take_stderr().expect("stderr piped");

        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAP);
        let primary_rx = events_tx.subscribe();
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        let stderr_ring = Arc::new(StdMutex::new(Vec::<String>::new()));
        let kill_flag = Arc::new(KillFlag::default());
        let pending_permissions = Arc::new(StdMutex::new(HashMap::<String, Value>::new()));
        let pending_controls = Arc::new(StdMutex::new(
            HashMap::<String, oneshot::Sender<Value>>::new(),
        ));

        tokio::spawn(write_loop(stdin, stdin_rx));

        tokio::spawn(stderr_loop(stderr, stderr_ring.clone()));

        tokio::spawn(read_and_wait(
            child,
            stdout,
            events_tx.clone(),
            stderr_ring.clone(),
            kill_flag.clone(),
            pending_permissions.clone(),
            pending_controls.clone(),
        ));

        Ok((
            Self {
                session_id: config.session_id,
                stdin_tx,
                events: events_tx,
                control_seq: std::sync::atomic::AtomicU64::new(0),
                stderr_ring,
                kill_flag,
                pending_permissions,
                pending_controls,
                killer: Killer { pid },
                claude_config_dir: config.claude_config_dir,
            },
            primary_rx,
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> Result<()> {
        let _ = self.events.send(SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
        });
        self.write_line(protocol::user_turn_json(&self.session_id, text))
    }

    pub fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        let updated_input = self
            .pending_permissions
            .lock()
            .ok()
            .and_then(|mut map| map.remove(request_id));
        self.write_line(protocol::permission_response_json(
            request_id,
            allow,
            updated_input,
            message,
        ))
    }

    pub fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> Result<()> {
        let mut input = self
            .pending_permissions
            .lock()
            .ok()
            .and_then(|mut map| map.remove(request_id))
            .unwrap_or_else(|| json!({}));
        if let Value::Object(map) = &mut input {
            map.insert("answers".to_string(), json!(answers));
        }
        self.write_line(protocol::permission_response_json(
            request_id,
            true,
            Some(input),
            None,
        ))
    }

    pub fn interrupt(&self) -> Result<()> {
        let id = self.next_control_id();
        self.write_line(protocol::control_request_json(&id, "interrupt", json!({})))
    }

    pub fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        let id = self.next_control_id();
        self.write_line(protocol::control_request_json(
            &id,
            "set_permission_mode",
            json!({ "mode": permission_mode_flag(mode) }),
        ))
    }

    pub async fn rewind_to_ordinal(&self, ordinal: usize) -> Result<()> {
        let uuids = self.transcript_user_uuids().await;
        let start =
            rewind_local_start(ordinal, uuids.len()).map_err(|detail| CoreError::OutputParse {
                program: "claude".into(),
                detail,
            })?;
        for uuid in uuids[start..].iter().rev() {
            let outcome = self.rewind_conversation(uuid, true).await?;
            if !outcome.rewound {
                return Err(CoreError::OutputParse {
                    program: "claude".into(),
                    detail: format!(
                        "claude refused rewind_conversation: {}",
                        outcome.error.as_deref().unwrap_or("unknown")
                    ),
                });
            }
        }
        Ok(())
    }

    async fn transcript_user_uuids(&self) -> Vec<String> {
        let session_id = self.session_id.clone();
        let claude_config_dir = self.claude_config_dir.clone();
        tokio::task::spawn_blocking(move || {
            transcript_path(&session_id, claude_config_dir.as_deref())
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|c| parse_transcript_user_uuids(&c))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }

    async fn rewind_conversation(
        &self,
        target_uuid: &str,
        interrupt_if_running: bool,
    ) -> Result<RewindOutcome> {
        let id = self.next_control_id();
        let (tx, rx) = oneshot::channel::<Value>();
        if let Ok(mut map) = self.pending_controls.lock() {
            map.insert(id.clone(), tx);
        }
        if let Err(e) = self.write_line(protocol::control_request_json(
            &id,
            "rewind_conversation",
            json!({
                "target_message_uuid": target_uuid,
                "interrupt_if_running": interrupt_if_running,
                "dry_run": false,
            }),
        )) {
            if let Ok(mut map) = self.pending_controls.lock() {
                map.remove(&id);
            }
            return Err(e);
        }
        let payload = match tokio::time::timeout(std::time::Duration::from_secs(20), rx).await {
            Ok(Ok(v)) => v,
            _ => {
                if let Ok(mut map) = self.pending_controls.lock() {
                    map.remove(&id);
                }
                return Err(CoreError::OutputParse {
                    program: "claude".into(),
                    detail: "rewind_conversation got no reply".into(),
                });
            }
        };
        Ok(RewindOutcome {
            rewound: payload
                .get("rewound")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error: payload
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub fn kill(&self) {
        self.kill_flag.mark_killed();
        self.killer.kill();
    }

    pub fn kill_detached(&self) {
        self.kill_flag.mark_detached();
        self.killer.kill();
    }

    pub fn pid(&self) -> Option<u32> {
        self.killer.pid
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr_ring
            .lock()
            .map(|r| r.join("\n"))
            .unwrap_or_default()
    }

    fn write_line(&self, mut line: String) -> Result<()> {
        line.push('\n');
        self.stdin_tx
            .send(line)
            .map_err(|_| CoreError::OutputParse {
                program: "claude".into(),
                detail: "session stdin is closed".into(),
            })
    }

    fn next_control_id(&self) -> String {
        let n = self
            .control_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("app-{}-{n}", self.session_id)
    }
}

async fn write_loop(
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(line) = rx.recv().await {
        if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
}

async fn stderr_loop(stderr: tokio::process::ChildStderr, ring: Arc<StdMutex<Vec<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(mut ring) = ring.lock() {
            ring.push(line);
            let overflow = ring.len().saturating_sub(STDERR_RING_LINES);
            if overflow > 0 {
                ring.drain(0..overflow);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_and_wait(
    mut child: crate::process::SupervisedChild,
    stdout: tokio::process::ChildStdout,
    events: broadcast::Sender<SessionEvent>,
    stderr_ring: Arc<StdMutex<Vec<String>>>,
    kill_flag: Arc<KillFlag>,
    pending_permissions: Arc<StdMutex<HashMap<String, Value>>>,
    pending_controls: Arc<StdMutex<HashMap<String, oneshot::Sender<Value>>>>,
) {
    let mut saw_result = false;
    let mut saw_error_result = false;
    let mut lines = BufReader::new(stdout).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let parsed = protocol::parse_line(&line);
                match &parsed {
                    ParsedLine::Result { is_error, .. } => {
                        saw_result = true;
                        saw_error_result |= *is_error;
                    }
                    ParsedLine::CanUseTool {
                        request_id, input, ..
                    } => {
                        if let Ok(mut map) = pending_permissions.lock() {
                            map.insert(request_id.clone(), input.clone());
                        }
                    }
                    ParsedLine::User {
                        is_replay: true, ..
                    } => continue,
                    ParsedLine::ControlResponse {
                        request_id,
                        payload,
                    } => {
                        if let Ok(mut map) = pending_controls.lock() {
                            if let Some(tx) = map.remove(request_id) {
                                let _ = tx.send(payload.clone());
                            }
                        }
                        continue;
                    }
                    _ => {}
                }
                for event in parsed_to_events(parsed) {
                    let _ = events.send(event);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    let status = child.wait().await;
    let code = status.as_ref().ok().and_then(|s| s.code());
    let stderr_tail = stderr_ring.lock().map(|r| r.join("\n")).unwrap_or_default();

    let outcome =
        kill_flag
            .outcome()
            .unwrap_or_else(|| match (&status, saw_result, saw_error_result) {
                (Ok(s), true, false) if s.success() => ExitOutcome::Completed,
                (Ok(s), _, _) => crate::process::exit::outcome_from_status(s),
                (Err(_), _, _) => ExitOutcome::SpawnFailed,
            });

    let _ = events.send(SessionEvent::Exited {
        outcome,
        code,
        stderr_tail,
    });
}

pub(crate) fn parsed_to_events(parsed: ParsedLine) -> Vec<SessionEvent> {
    match parsed {
        ParsedLine::Init {
            claude_session_id,
            model,
            ..
        } => vec![SessionEvent::Init {
            provider_session_id: claude_session_id.unwrap_or_default(),
            model,
        }],
        ParsedLine::PartialText { text, parent_id } => {
            vec![SessionEvent::PartialText { text, parent_id }]
        }
        ParsedLine::Assistant {
            blocks,
            context_tokens,
            parent_id,
        } => context_tokens
            .filter(|_| parent_id.is_none())
            .map(|t| SessionEvent::Usage { context_tokens: t })
            .into_iter()
            .chain(
                blocks
                    .into_iter()
                    .filter_map(|b| block_to_event(b, parent_id.as_deref())),
            )
            .collect(),
        ParsedLine::User {
            blocks,
            parent_id,
            is_synthetic,
            ..
        } => blocks
            .into_iter()
            .filter_map(|b| block_to_event(b, parent_id.as_deref()))
            .map(|event| match event {
                SessionEvent::AssistantText {
                    text, parent_id, ..
                } if is_synthetic => SessionEvent::AssistantText {
                    text,
                    parent_id,
                    injected: true,
                },
                other => other,
            })
            .collect(),
        ParsedLine::CanUseTool {
            request_id,
            tool_name,
            input,
        } => {
            let questions = (tool_name == "AskUserQuestion")
                .then(|| input.get("questions"))
                .flatten()
                .filter(|q| q.as_array().is_some_and(|a| !a.is_empty()));
            match questions {
                Some(questions) => vec![SessionEvent::QuestionAsk {
                    request_id,
                    questions: questions_from_value(Some(questions)),
                }],
                None => vec![SessionEvent::PermissionAsk {
                    request_id,
                    tool_name,
                    input: input.to_string(),
                }],
            }
        }
        ParsedLine::Result {
            total_cost_usd,
            num_turns,
            context_tokens,
            ..
        } => vec![SessionEvent::TurnDone {
            cost_usd: total_cost_usd,
            num_turns,
            context_tokens,
            turn_status: None,
        }],
        ParsedLine::CompactBoundary {
            trigger,
            pre_tokens,
        } => {
            vec![SessionEvent::CompactBoundary {
                trigger,
                pre_tokens,
            }]
        }
        ParsedLine::TaskNotification {
            task_id,
            tool_use_id,
            status,
        } => vec![SessionEvent::BackgroundTaskUpdate {
            task_id,
            tool_use_id,
            status,
        }],
        ParsedLine::CommandLifecycle {
            command_uuid,
            state,
        } => vec![SessionEvent::CommandLifecycle {
            command_uuid,
            state,
        }],
        ParsedLine::Unknown { raw } => vec![SessionEvent::ProtocolError {
            detail: "unrecognized stream-json line".into(),
            raw,
        }],
        ParsedLine::ControlResponse { .. } => Vec::new(),
        ParsedLine::System { .. } => Vec::new(),
    }
}

pub(crate) fn block_to_event(block: Block, parent_id: Option<&str>) -> Option<SessionEvent> {
    let parent_id = parent_id.map(str::to_string);
    match block {
        Block::Text { text } => Some(SessionEvent::AssistantText {
            text,
            parent_id,
            injected: false,
        }),
        Block::Thinking { text } => Some(SessionEvent::Thinking { text, parent_id }),
        Block::ToolUse { id, name, input } => Some(SessionEvent::ToolUse {
            id,
            name,
            input: input.to_string(),
            parent_id,
        }),
        Block::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(SessionEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
            parent_id,
        }),
        Block::Other { .. } => None,
    }
}

fn rewind_local_start(ordinal: usize, len: usize) -> std::result::Result<usize, String> {
    if ordinal < 2 {
        return Err("rewind_conversation cannot target the first user turn".into());
    }
    let idx = ordinal - 1;
    if idx >= len {
        return Err(format!(
            "no uuid for user turn {ordinal} (transcript has {len} turns)"
        ));
    }
    Ok(idx)
}

fn transcript_path(session_id: &str, claude_config_dir: Option<&Path>) -> Option<PathBuf> {
    let projects = claude_config_dir?.join("projects");
    let filename = format!("{session_id}.jsonl");
    for entry in std::fs::read_dir(&projects).ok()?.flatten() {
        let candidate = entry.path().join(&filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn parse_transcript_user_uuids(jsonl: &str) -> Vec<String> {
    jsonl
        .lines()
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line.trim()).ok()?;
            if v.get("type").and_then(Value::as_str) != Some("user") {
                return None;
            }
            let flag = |k| v.get(k).and_then(Value::as_bool).unwrap_or(false);
            if flag("isMeta")
                || flag("isCompactSummary")
                || flag("isVisibleInTranscriptOnly")
                || flag("isSidechain")
            {
                return None;
            }
            let content = v.get("message").and_then(|m| m.get("content"));
            let is_tool_result = content
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|b| b.get("type"))
                .and_then(Value::as_str)
                == Some("tool_result");
            if is_tool_result {
                return None;
            }
            if let Some(kind) = v
                .get("origin")
                .and_then(|o| o.get("kind"))
                .and_then(Value::as_str)
            {
                if kind != "human" {
                    return None;
                }
            }
            v.get("uuid").and_then(Value::as_str).map(str::to_string)
        })
        .collect()
}

struct Killer {
    pid: Option<u32>,
}

impl Killer {
    fn kill(&self) {
        let Some(pid) = self.pid else { return };
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000)
                .output();
        }
    }
}

#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rewind_maps_ordinal_to_index() {
        assert_eq!(rewind_local_start(2, 5), Ok(1));
        assert_eq!(rewind_local_start(5, 5), Ok(4));
        assert_eq!(rewind_local_start(3, 7), Ok(2));
        assert!(rewind_local_start(1, 5).is_err());
        assert!(rewind_local_start(6, 5).is_err());
    }

    #[test]
    fn rewind_empty_transcript_reaches_nothing() {
        for ordinal in 1..=8 {
            assert!(rewind_local_start(ordinal, 0).is_err());
        }
    }

    #[test]
    fn parse_transcript_keeps_only_human_turns() {
        let jsonl = [
            r#"{"type":"system","subtype":"init","uuid":"sys"}"#,
            r#"{"type":"user","uuid":"u1","message":{"role":"user","content":"first turn"}}"#,
            r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"user","uuid":"tr","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#,
            r#"{"type":"user","uuid":"u2","message":{"role":"user","content":"second turn"}}"#,
            r#"{"type":"user","uuid":"meta","isMeta":true,"message":{"content":"injected"}}"#,
            r#"{"type":"user","uuid":"side","isSidechain":true,"message":{"content":"subagent"}}"#,
            r#"{"type":"user","uuid":"nonhuman","origin":{"kind":"agent"},"message":{"content":"x"}}"#,
            r#"{"type":"user","uuid":"u3","message":{"role":"user","content":"third turn"}}"#,
        ]
        .join("\n");
        assert_eq!(parse_transcript_user_uuids(&jsonl), vec!["u1", "u2", "u3"]);
    }

    #[test]
    fn parse_transcript_tolerates_junk_lines() {
        let jsonl = "\nnot json\n{}\n{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(parse_transcript_user_uuids(jsonl), vec!["u1"]);
    }

    #[test]
    fn ask_user_question_becomes_a_question_ask() {
        let questions = json!([{
            "question": "Tabs or spaces?",
            "header": "Indentation",
            "multiSelect": false,
            "options": [
                { "label": "Tabs", "description": "Tab characters." },
                { "label": "Spaces", "description": "Space characters." }
            ]
        }]);
        let events = parsed_to_events(ParsedLine::CanUseTool {
            request_id: "req-1".into(),
            tool_name: "AskUserQuestion".into(),
            input: json!({ "questions": questions }),
        });
        match events.as_slice() {
            [SessionEvent::QuestionAsk {
                request_id,
                questions: q,
            }] => {
                assert_eq!(request_id, "req-1");
                assert_eq!(serde_json::to_value(q).unwrap(), questions);
            }
            other => panic!("expected one QuestionAsk, got {other:?}"),
        }
    }

    #[test]
    fn synthetic_user_text_is_flagged_injected() {
        let events = parsed_to_events(ParsedLine::User {
            blocks: vec![Block::Text {
                text: "Approach this as the design lead".into(),
            }],
            parent_id: None,
            uuid: None,
            is_replay: false,
            is_synthetic: true,
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::AssistantText { injected: true, .. }]
        ));
    }

    #[test]
    fn ordinary_user_text_is_not_injected() {
        let events = parsed_to_events(ParsedLine::User {
            blocks: vec![Block::Text {
                text: "plain".into(),
            }],
            parent_id: None,
            uuid: None,
            is_replay: false,
            is_synthetic: false,
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::AssistantText {
                injected: false,
                ..
            }]
        ));
    }

    #[test]
    fn other_tools_remain_permission_asks() {
        let events = parsed_to_events(ParsedLine::CanUseTool {
            request_id: "req-2".into(),
            tool_name: "Bash".into(),
            input: json!({ "command": "ls" }),
        });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::PermissionAsk { tool_name, .. }] if tool_name == "Bash"
        ));
    }

    #[test]
    fn task_notification_becomes_a_background_task_update() {
        let events = parsed_to_events(ParsedLine::TaskNotification {
            task_id: "bxmc1fof8".into(),
            tool_use_id: Some("toolu_01HgG".into()),
            status: "completed".into(),
        });
        match events.as_slice() {
            [SessionEvent::BackgroundTaskUpdate {
                task_id,
                tool_use_id,
                status,
            }] => {
                assert_eq!(task_id, "bxmc1fof8");
                assert_eq!(tool_use_id.as_deref(), Some("toolu_01HgG"));
                assert_eq!(status, "completed");
            }
            other => panic!("expected one BackgroundTaskUpdate, got {other:?}"),
        }
    }

    #[test]
    fn ask_user_question_with_bad_questions_falls_back_to_permission() {
        for bad in [
            json!({}),
            json!({ "questions": null }),
            json!({ "questions": "nope" }),
            json!({ "questions": [] }),
        ] {
            let events = parsed_to_events(ParsedLine::CanUseTool {
                request_id: "req-3".into(),
                tool_name: "AskUserQuestion".into(),
                input: bad.clone(),
            });
            assert!(
                matches!(
                    events.as_slice(),
                    [SessionEvent::PermissionAsk { tool_name, .. }] if tool_name == "AskUserQuestion"
                ),
                "expected PermissionAsk fallback for input {bad}"
            );
        }
    }
}
