use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::agent::{AgentDriver, KillFlag};
use hyprlayer_core::claude::{ExitOutcome, SessionEvent};
use hyprlayer_core::error::Result as CoreResult;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

const EVENT_CAP: usize = 1024;

#[derive(Clone)]
struct OcEvent {
    kind: String,
    properties: Value,
}

#[derive(Default)]
pub struct OpenCodeManager {
    inner: Mutex<Option<Arc<OpenCodeServer>>>,
}

impl OpenCodeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn kill_blocking(&self) {
        if let Ok(guard) = self.inner.try_lock() {
            if let Some(server) = guard.as_ref() {
                server.kill_blocking();
            }
        }
    }

    pub async fn ensure(
        &self,
        bin: PathBuf,
        path: OsString,
        data_dir: PathBuf,
    ) -> Result<Arc<OpenCodeServer>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_ref() {
            if s.is_healthy().await {
                return Ok(s.clone());
            }
        }
        if let Some(dead) = guard.take() {
            dead.kill_blocking();
        }
        let server = Arc::new(OpenCodeServer::start(bin, path, data_dir).await?);
        *guard = Some(server.clone());
        Ok(server)
    }

    pub async fn invalidate(&self) {
        *self.inner.lock().await = None;
    }
}

pub struct OpenCodeServer {
    base_url: String,
    client: reqwest::Client,
    events: broadcast::Sender<OcEvent>,
    model_context: std::collections::HashMap<String, u32>,
    _child: Mutex<hyprlayer_core::process::SupervisedChild>,
}

impl OpenCodeServer {
    async fn start(bin: PathBuf, path: OsString, data_dir: PathBuf) -> Result<Self, String> {
        let port = free_port()?;
        let base_url = format!("http://127.0.0.1:{port}");
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let _ = std::fs::create_dir_all(&data_dir);

        let mut cmd = Command::new(&bin);
        hyprlayer_core::process::appimage::scrub_tokio(&mut cmd);
        cmd.args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .current_dir(&home)
        .env("PATH", &path)
        .env("XDG_DATA_HOME", &data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        if let Some(content) = hyprlayer_core::opencode::config::auth_json_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .filter(|c| !c.trim().is_empty())
        {
            cmd.env("OPENCODE_AUTH_CONTENT", content);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = hyprlayer_core::process::spawn_supervised(&mut cmd)
            .map_err(|e| format!("spawn `opencode serve`: {e}"))?;

        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| e.to_string())?;

        let mut healthy = false;
        for _ in 0..60 {
            if let Ok(resp) = client
                .get(format!("{base_url}/global/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                if let Ok(v) = resp.json::<Value>().await {
                    if v.get("healthy").and_then(Value::as_bool) == Some(true) {
                        healthy = true;
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !healthy {
            return Err("`opencode serve` did not become healthy in time".into());
        }

        let model_context = fetch_model_context(&client, &base_url).await;

        let (events, _) = broadcast::channel(EVENT_CAP);
        spawn_sse(base_url.clone(), client.clone(), events.clone());

        Ok(Self {
            base_url,
            client,
            events,
            model_context,
            _child: Mutex::new(child),
        })
    }

    fn kill_blocking(&self) {
        if let Ok(child) = self._child.try_lock() {
            child.kill_blocking();
        }
    }

    fn context_window(&self, model_id: &str) -> Option<u32> {
        self.model_context.get(model_id).copied()
    }

    async fn is_healthy(&self) -> bool {
        self.client
            .get(format!("{}/global/health", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

fn spawn_sse(base_url: String, client: reqwest::Client, tx: broadcast::Sender<OcEvent>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Ok(mut resp) = client.get(format!("{base_url}/global/event")).send().await {
                let mut buf = String::new();
                while let Ok(Some(bytes)) = resp.chunk().await {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(idx) = buf.find("\n\n") {
                        let record: String = buf.drain(..idx + 2).collect();
                        if let Some(ev) = parse_sse_record(&record) {
                            let _ = tx.send(ev);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

fn parse_sse_record(record: &str) -> Option<OcEvent> {
    let mut data = String::new();
    for line in record.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(&data).ok()?;
    let event = v.get("payload").unwrap_or(&v);
    let kind = event.get("type")?.as_str()?.to_string();
    if kind == "server.heartbeat" {
        return None;
    }
    Some(OcEvent {
        kind,
        properties: event.get("properties").cloned().unwrap_or(Value::Null),
    })
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("allocate a local port: {e}"))
}

async fn fetch_model_context(
    client: &reqwest::Client,
    base_url: &str,
) -> std::collections::HashMap<String, u32> {
    let body = match client
        .get(format!("{base_url}/config/providers"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(r) => match r.text().await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("opencode /config/providers read failed: {e}");
                return std::collections::HashMap::new();
            }
        },
        Err(e) => {
            log::warn!("opencode /config/providers fetch failed: {e}");
            return std::collections::HashMap::new();
        }
    };
    parse_model_context(&body)
}

fn parse_model_context(body: &str) -> std::collections::HashMap<String, u32> {
    #[derive(serde::Deserialize)]
    struct Resp {
        #[serde(default)]
        providers: Vec<ProviderEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ProviderEntry {
        id: String,
        #[serde(default)]
        models: std::collections::HashMap<String, ModelEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        #[serde(default)]
        limit: Option<LimitEntry>,
    }
    #[derive(serde::Deserialize)]
    struct LimitEntry {
        #[serde(default)]
        context: Option<u64>,
    }

    let mut out = std::collections::HashMap::new();
    let Ok(parsed) = serde_json::from_str::<Resp>(body) else {
        log::warn!("opencode /config/providers parse failed");
        return out;
    };
    for p in parsed.providers {
        for (model_id, m) in p.models {
            if let Some(ctx) = m.limit.and_then(|l| l.context).filter(|c| *c > 0) {
                out.insert(
                    format!("{}/{model_id}", p.id),
                    ctx.min(u32::MAX as u64) as u32,
                );
            }
        }
    }
    out
}

pub struct OpenCodeDriver {
    server: Arc<OpenCodeServer>,
    session_id: String,
    model: std::sync::Mutex<Option<(String, String)>>,
    events: broadcast::Sender<SessionEvent>,
    kill_flag: Arc<KillFlag>,
    filter_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pending_questions: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>>,
}

impl OpenCodeDriver {
    pub async fn spawn(
        server: Arc<OpenCodeServer>,
        cwd: &str,
        model: Option<String>,
    ) -> Result<(Arc<Self>, broadcast::Receiver<SessionEvent>), String> {
        let created: Value = server
            .client
            .post(format!("{}/session", server.base_url))
            .query(&[("directory", cwd)])
            .json(&json!({ "title": "Hyprlayer session" }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let session_id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or("OpenCode session create returned no id")?
            .to_string();

        let model = model.as_deref().and_then(parse_model);

        let (events, _) = broadcast::channel(EVENT_CAP);
        let primary_rx = events.subscribe();

        let _ = events.send(SessionEvent::Init {
            claude_session_id: session_id.clone(),
            model: model.as_ref().map(|(p, m)| format!("{p}/{m}")),
        });

        if let Some(tokens) = model.as_ref().and_then(|(p, m)| {
            let id = format!("{p}/{m}");
            hyprlayer_core::opencode::config::opencode_model_context_window(&id)
                .or_else(|| server.context_window(&id))
        }) {
            let _ = events.send(SessionEvent::ContextWindow { tokens });
        }

        let mut sse_rx = server.events.subscribe();
        let tx = events.clone();
        let sid = session_id.clone();
        let pending_questions: Arc<std::sync::Mutex<std::collections::HashMap<String, Value>>> =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let pq = pending_questions.clone();
        let task = tauri::async_runtime::spawn(async move {
            let mut xlate = Translator::default();
            loop {
                match sse_rx.recv().await {
                    Ok(ev) => {
                        if event_session_id(&ev).as_deref() != Some(sid.as_str()) {
                            continue;
                        }
                        if ev.kind == "question.asked" {
                            if let Some(qid) = ev.properties.get("id").and_then(Value::as_str) {
                                if let Ok(mut m) = pq.lock() {
                                    m.insert(
                                        qid.to_string(),
                                        ev.properties
                                            .get("questions")
                                            .cloned()
                                            .unwrap_or(json!([])),
                                    );
                                }
                            }
                        }
                        for se in xlate.handle(&ev) {
                            let _ = tx.send(se);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let driver = Arc::new(Self {
            server,
            session_id,
            model: std::sync::Mutex::new(model),
            events,
            kill_flag: Arc::new(KillFlag::default()),
            filter_task: Mutex::new(Some(task)),
            pending_questions,
        });
        Ok((driver, primary_rx))
    }

    fn end(&self, outcome: ExitOutcome) {
        self.post_async(format!("/session/{}/abort", self.session_id), json!({}));
        if let Ok(mut guard) = self.filter_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        let _ = self.events.send(SessionEvent::Exited {
            outcome,
            code: None,
            stderr_tail: String::new(),
        });
    }

    fn post_async(&self, path: String, body: Value) {
        let client = self.server.client.clone();
        let url = format!("{}{}", self.server.base_url, path);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = client.post(url).json(&body).send().await {
                log::warn!("opencode POST failed: {e}");
            }
        });
    }

    fn prompt_async(&self, body: Value) {
        let client = self.server.client.clone();
        let url = format!(
            "{}/session/{}/prompt_async",
            self.server.base_url, self.session_id
        );
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            let result = client
                .post(url)
                .json(&body)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(e) = result {
                log::warn!("opencode prompt failed: {e}");
                let _ = events.send(SessionEvent::AssistantText {
                    text: format!("⚠ OpenCode request failed: {e}"),
                    parent_id: None,
                    injected: false,
                });
                let _ = events.send(SessionEvent::TurnDone {
                    cost_usd: None,
                    num_turns: None,
                    context_tokens: None,
                });
            }
        });
    }

    pub async fn delete_session(&self) {
        let url = format!("{}/session/{}", self.server.base_url, self.session_id);
        if let Err(e) = self
            .server
            .client
            .delete(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            log::warn!("opencode session delete failed: {e}");
        }
    }
}

impl AgentDriver for OpenCodeDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> CoreResult<()> {
        let _ = self.events.send(SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
        });
        let mut body = json!({ "parts": [{ "type": "text", "text": text }] });
        if let Some((provider, model)) = self.model.lock().ok().and_then(|m| m.clone()) {
            body["model"] = json!({ "providerID": provider, "modelID": model });
        }
        self.prompt_async(body);
        Ok(())
    }

    fn set_model(&self, model: Option<String>) {
        if let Ok(mut m) = self.model.lock() {
            *m = model.as_deref().and_then(parse_model);
        }
    }

    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        _message: Option<&str>,
    ) -> CoreResult<()> {
        let response = if allow { "once" } else { "reject" };
        self.post_async(
            format!("/session/{}/permissions/{}", self.session_id, request_id),
            json!({ "response": response }),
        );
        Ok(())
    }

    fn answer_question(&self, request_id: &str, answers: Value) -> CoreResult<()> {
        let questions = self
            .pending_questions
            .lock()
            .ok()
            .and_then(|mut m| m.remove(request_id));
        let empty = answers.as_object().map(|o| o.is_empty()).unwrap_or(true);
        if empty {
            self.post_async(format!("/question/{request_id}/reject"), json!({}));
            return Ok(());
        }
        let ordered: Vec<Vec<String>> = questions
            .as_ref()
            .and_then(Value::as_array)
            .map(|qs| {
                qs.iter()
                    .map(|q| {
                        let text = q
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        answers
                            .get(text)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(|s| s.split(", ").map(str::to_string).collect())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.post_async(
            format!("/question/{request_id}/reply"),
            json!({ "answers": ordered }),
        );
        Ok(())
    }

    fn interrupt(&self) -> CoreResult<()> {
        self.post_async(format!("/session/{}/abort", self.session_id), json!({}));
        Ok(())
    }

    fn kill(&self) {
        if self.kill_flag.mark_killed() {
            self.end(ExitOutcome::Killed);
        }
    }

    fn kill_detached(&self) {
        if self.kill_flag.mark_detached() {
            self.end(ExitOutcome::Detached);
        }
    }

    fn pid(&self) -> Option<u32> {
        None
    }
}

fn parse_model(id: &str) -> Option<(String, String)> {
    id.split_once('/')
        .map(|(p, m)| (p.to_string(), m.to_string()))
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

fn event_session_id(ev: &OcEvent) -> Option<String> {
    ev.properties
        .get("sessionID")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[derive(Default)]
struct Translator {
    roles: std::collections::HashMap<String, String>,
    text_len: std::collections::HashMap<String, usize>,
    text_done: std::collections::HashSet<String>,
    delta_parts: std::collections::HashSet<String>,
    part_types: std::collections::HashMap<String, String>,
    tool_started: std::collections::HashSet<String>,
    tool_done: std::collections::HashSet<String>,
}

impl Translator {
    fn handle(&mut self, ev: &OcEvent) -> Vec<SessionEvent> {
        let p = &ev.properties;
        match ev.kind.as_str() {
            "message.updated" => {
                if let Some(info) = p.get("info") {
                    if let (Some(id), Some(role)) = (
                        info.get("id").and_then(Value::as_str),
                        info.get("role").and_then(Value::as_str),
                    ) {
                        self.roles.insert(id.to_string(), role.to_string());
                    }
                }
                Vec::new()
            }
            "message.part.delta" => {
                let mid = p
                    .get("messageID")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if self.roles.get(mid).map(String::as_str) != Some("assistant") {
                    return Vec::new();
                }
                let pid = p.get("partID").and_then(Value::as_str).unwrap_or_default();
                if !pid.is_empty() {
                    self.delta_parts.insert(pid.to_string());
                }
                if self.part_types.get(pid).map(String::as_str) == Some("reasoning") {
                    return Vec::new();
                }
                let field = p.get("field").and_then(Value::as_str).unwrap_or("text");
                let delta = p.get("delta").and_then(Value::as_str).unwrap_or_default();
                if field == "text" && !delta.is_empty() {
                    vec![SessionEvent::PartialText {
                        text: delta.to_string(),
                        parent_id: None,
                    }]
                } else {
                    Vec::new()
                }
            }
            "message.part.updated" => self.handle_part(p.get("part").unwrap_or(&Value::Null)),
            "permission.asked" => vec![SessionEvent::PermissionAsk {
                request_id: str_field(p, "id"),
                tool_name: {
                    let t = str_field(p, "permission");
                    if t.is_empty() {
                        "tool".to_string()
                    } else {
                        t
                    }
                },
                input: json!({
                    "patterns": p.get("patterns").cloned().unwrap_or(json!([])),
                    "metadata": p.get("metadata").cloned().unwrap_or(json!({})),
                })
                .to_string(),
            }],
            "question.asked" => vec![SessionEvent::QuestionAsk {
                request_id: str_field(p, "id"),
                questions: p.get("questions").cloned().unwrap_or(json!([])).to_string(),
            }],
            "session.idle" => vec![SessionEvent::TurnDone {
                cost_usd: None,
                num_turns: None,
                context_tokens: None,
            }],
            "session.error" => {
                let err = p.get("error");
                let name = err
                    .and_then(|e| e.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let msg = err
                    .and_then(|e| e.get("data"))
                    .and_then(|d| d.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("OpenCode reported an error")
                    .to_string();
                if name == "MessageAbortedError" || msg == "Aborted" {
                    return vec![SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                    }];
                }
                vec![
                    SessionEvent::AssistantText {
                        text: format!("⚠ OpenCode error: {msg}"),
                        parent_id: None,
                        injected: false,
                    },
                    SessionEvent::TurnDone {
                        cost_usd: None,
                        num_turns: None,
                        context_tokens: None,
                    },
                ]
            }
            _ => Vec::new(),
        }
    }

    fn handle_part(&mut self, part: &Value) -> Vec<SessionEvent> {
        let pid = part
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let ptype = part.get("type").and_then(Value::as_str).unwrap_or_default();
        if !pid.is_empty() {
            self.part_types.insert(pid.clone(), ptype.to_string());
        }
        let mid = part
            .get("messageID")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.roles.get(mid).map(String::as_str) != Some("assistant") {
            return Vec::new();
        }
        let mut out = Vec::new();

        match ptype {
            "text" => {
                let full = part.get("text").and_then(Value::as_str).unwrap_or_default();
                if !self.delta_parts.contains(&pid) {
                    let emitted = self.text_len.get(&pid).copied().unwrap_or(0);
                    if full.len() > emitted {
                        out.push(SessionEvent::PartialText {
                            text: full[emitted..].to_string(),
                            parent_id: None,
                        });
                        self.text_len.insert(pid.clone(), full.len());
                    }
                }
                if part_ended(part) && self.text_done.insert(pid) {
                    out.push(SessionEvent::AssistantText {
                        text: full.to_string(),
                        parent_id: None,
                        injected: false,
                    });
                }
            }
            "reasoning" => {
                if part_ended(part) && self.text_done.insert(pid) {
                    let full = part.get("text").and_then(Value::as_str).unwrap_or_default();
                    out.push(SessionEvent::Thinking {
                        text: full.to_string(),
                        parent_id: None,
                    });
                }
            }
            "tool" => {
                let call_id = part
                    .get("callID")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tool = part
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if tool == "question" {
                    return out;
                }
                let state = part.get("state").cloned().unwrap_or(Value::Null);
                let status = state
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !call_id.is_empty()
                    && status != "pending"
                    && self.tool_started.insert(call_id.clone())
                {
                    let (name, input) =
                        normalize_tool(&tool, state.get("input").cloned().unwrap_or(json!({})));
                    out.push(SessionEvent::ToolUse {
                        id: call_id.clone(),
                        name,
                        input: input.to_string(),
                        parent_id: None,
                    });
                }
                match status {
                    "completed" if self.tool_done.insert(call_id.clone()) => {
                        out.push(SessionEvent::ToolResult {
                            tool_use_id: call_id,
                            content: value_to_text(state.get("output")),
                            is_error: false,
                            parent_id: None,
                        });
                    }
                    "error" if self.tool_done.insert(call_id.clone()) => {
                        out.push(SessionEvent::ToolResult {
                            tool_use_id: call_id,
                            content: value_to_text(state.get("error")),
                            is_error: true,
                            parent_id: None,
                        });
                    }
                    _ => {}
                }
            }
            "step-finish" => {
                let cost = part.get("cost").and_then(Value::as_f64);
                let ctx = part.get("tokens").map(|t| {
                    let f = |k: &str| t.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                    let total = f("total");
                    (if total > 0.0 {
                        total
                    } else {
                        f("input") + f("output")
                    }) as u32
                });
                out.push(SessionEvent::TurnDone {
                    cost_usd: cost,
                    num_turns: None,
                    context_tokens: ctx,
                });
            }
            _ => {}
        }
        out
    }
}

fn str_field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn normalize_tool(tool: &str, mut input: Value) -> (String, Value) {
    fn rename(input: &mut Value, from: &str, to: &str) {
        if let Some(obj) = input.as_object_mut() {
            if let Some(v) = obj.remove(from) {
                obj.insert(to.to_string(), v);
            }
        }
    }
    let name = match tool {
        "read" => {
            rename(&mut input, "filePath", "file_path");
            "Read"
        }
        "write" => {
            rename(&mut input, "filePath", "file_path");
            "Write"
        }
        "edit" => {
            rename(&mut input, "filePath", "file_path");
            rename(&mut input, "oldString", "old_string");
            rename(&mut input, "newString", "new_string");
            "Edit"
        }
        "bash" => "Bash",
        "grep" => "Grep",
        "glob" => "Glob",
        "webfetch" => "WebFetch",
        "task" => {
            rename(&mut input, "subagentType", "subagent_type");
            "Task"
        }
        "todowrite" => "TodoWrite",
        _ => return (tool.to_string(), input),
    };
    (name.to_string(), input)
}

fn part_ended(part: &Value) -> bool {
    part.get("time").and_then(|t| t.get("end")).is_some()
}

fn value_to_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_file_tools_to_claude_vocabulary() {
        let (name, input) = normalize_tool("read", json!({ "filePath": "/w/a.txt" }));
        assert_eq!(name, "Read");
        assert_eq!(input, json!({ "file_path": "/w/a.txt" }));

        let (name, input) =
            normalize_tool("write", json!({ "filePath": "/w/b.txt", "content": "x" }));
        assert_eq!(name, "Write");
        assert_eq!(input, json!({ "file_path": "/w/b.txt", "content": "x" }));

        let (name, input) = normalize_tool(
            "edit",
            json!({ "filePath": "/w/a.txt", "oldString": "a", "newString": "b" }),
        );
        assert_eq!(name, "Edit");
        assert_eq!(
            input,
            json!({ "file_path": "/w/a.txt", "old_string": "a", "new_string": "b" })
        );
    }

    #[test]
    fn parse_model_context_indexes_by_provider_slash_model() {
        let body = r#"{
            "providers": [
                {
                    "id": "fireworks-ai",
                    "name": "Fireworks AI",
                    "models": {
                        "accounts/fireworks/routers/kimi-k2p6-turbo": {
                            "id": "accounts/fireworks/routers/kimi-k2p6-turbo",
                            "limit": { "context": 262000, "output": 262000 }
                        }
                    }
                },
                {
                    "id": "openrouter",
                    "models": {
                        "no-limit-model": { "id": "no-limit-model" },
                        "zero-ctx-model": { "id": "zero-ctx-model", "limit": { "context": 0 } }
                    }
                }
            ]
        }"#;
        let map = parse_model_context(body);
        assert_eq!(
            map.get("fireworks-ai/accounts/fireworks/routers/kimi-k2p6-turbo"),
            Some(&262_000)
        );
        assert!(!map.contains_key("openrouter/no-limit-model"));
        assert!(!map.contains_key("openrouter/zero-ctx-model"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_model_context_tolerates_garbage() {
        assert!(parse_model_context("not json").is_empty());
        assert!(parse_model_context(r#"{"providers":[]}"#).is_empty());
    }

    #[test]
    fn normalize_passes_through_unknown_tools() {
        let original = json!({ "anything": 1 });
        let (name, input) = normalize_tool("some_mcp_tool", original.clone());
        assert_eq!(name, "some_mcp_tool");
        assert_eq!(input, original);
    }

    #[test]
    fn normalize_renames_non_file_tools_without_touching_input() {
        let (name, input) = normalize_tool("grep", json!({ "pattern": "DONE", "path": "/w" }));
        assert_eq!(name, "Grep");
        assert_eq!(input, json!({ "pattern": "DONE", "path": "/w" }));

        let (name, _) = normalize_tool("bash", json!({ "command": "echo hi" }));
        assert_eq!(name, "Bash");
    }

    fn oc(kind: &str, properties: Value) -> OcEvent {
        OcEvent {
            kind: kind.to_string(),
            properties,
        }
    }

    #[test]
    fn reasoning_announced_before_role_is_not_leaked_as_text() {
        let mut x = Translator::default();

        let out = x.handle(&oc(
            "message.part.updated",
            json!({ "part": { "id": "r1", "type": "reasoning", "messageID": "m1" } }),
        ));
        assert!(out.is_empty());

        x.handle(&oc(
            "message.updated",
            json!({ "info": { "id": "m1", "role": "assistant" } }),
        ));

        let out = x.handle(&oc(
            "message.part.delta",
            json!({ "messageID": "m1", "partID": "r1", "field": "text", "delta": "secret plan" }),
        ));
        assert!(
            out.is_empty(),
            "reasoning delta leaked as a live event: {out:?}"
        );

        let out = x.handle(&oc(
            "message.part.updated",
            json!({ "part": { "id": "r1", "type": "reasoning", "messageID": "m1", "text": "secret plan", "time": { "end": 1 } } }),
        ));
        assert_eq!(out.len(), 1);
        match &out[0] {
            SessionEvent::Thinking { text, .. } => assert_eq!(text, "secret plan"),
            other => panic!("expected Thinking, got {other:?}"),
        }
        assert!(
            !out.iter().any(|e| matches!(
                e,
                SessionEvent::PartialText { .. } | SessionEvent::AssistantText { .. }
            )),
            "reasoning surfaced as assistant text"
        );
    }

    #[test]
    fn global_event_record_unwraps_payload() {
        let rec = "data: {\"directory\":\"/w\",\"payload\":{\"type\":\"message.updated\",\"properties\":{\"x\":1}}}";
        let ev = parse_sse_record(rec).expect("payload event parses");
        assert_eq!(ev.kind, "message.updated");
        assert_eq!(ev.properties, json!({ "x": 1 }));

        let flat = "data: {\"type\":\"session.idle\",\"properties\":{}}";
        assert_eq!(parse_sse_record(flat).unwrap().kind, "session.idle");

        assert!(parse_sse_record("data: {\"type\":\"server.heartbeat\"}").is_none());
    }
}
