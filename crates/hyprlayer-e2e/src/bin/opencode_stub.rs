//! `opencode serve` for E2E: the OpenCode 2 routes the driver calls, shaped as 2.4.1 answers
//! them, behind HTTP Basic auth with the password from `OPENCODE_SERVER_PASSWORD`. Sessions and
//! credentials live under `$XDG_DATA_HOME`, so they survive a restart and vanish with a data wipe.
//! A run answers each message it reads with `model=<model> earlier=[<this session's earlier
//! prompts>]`. Besides the `opencode` models it serves `<provider>/stub-keyed` for each provider
//! holding a credential, and fails a run on any other model.
//!
//! `$XDG_DATA_HOME/opencode-stub/control.json` shapes each run: `start` and `reply` delay, in ms,
//! the delivery of a message and its reply; `abortDelay` delays a stop's handling. `"ask": true`
//! asks permission for bash before each reply, and `"question": true` a form before that;
//! `"aside": true` follows each ask with a text. `"child": true` asks from a subagent's session,
//! announced with `session.created` naming its parent, and answered only under that session.
//! `"fail": "<message>"` fails each run before any reply. `"lateAsk": true` asks for permission
//! as a stop lands; the stop drops the ask with no reply event, as 2.4.1 does. `"cut": "<event
//! type>"` closes every event stream in place of the next event of that type; the events sent
//! before the driver reconnects are lost, and each new stream starts with `server.connected`.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const VERSION: &str = "2.4.1";
const MODELS: [&str; 2] = ["stub-free", "stub-other-free"];
const KEYED_MODEL: &str = "stub-keyed";

struct Request {
    method: String,
    path: String,
    query: String,
    authorized: bool,
    body: Value,
}

/// An ask a run waits on: `Some(true)` allowed or answered, `Some(false)` refused.
struct Ask {
    owner: String,
    answer: Sender<bool>,
    request: Value,
    form: bool,
}

#[derive(Default)]
struct Live {
    inbox: VecDeque<(String, String)>,
    running: bool,
    stop: Arc<AtomicBool>,
}

struct Stub {
    root: PathBuf,
    subscribers: Mutex<Vec<TcpStream>>,
    live: Mutex<HashMap<String, Live>>,
    ended: Condvar,
    asks: Mutex<HashMap<String, Ask>>,
    cut: Mutex<Option<String>>,
    ids: AtomicU64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("opencode v{VERSION}");
        return;
    }
    let port: u16 = args
        .windows(2)
        .find(|pair| pair[0] == "--port")
        .and_then(|pair| pair[1].parse().ok())
        .expect("--port");
    let data = std::env::var_os("XDG_DATA_HOME").expect("XDG_DATA_HOME");
    let password = std::env::var("OPENCODE_SERVER_PASSWORD").expect("a server password");
    let root = PathBuf::from(data).join("opencode-stub");
    std::fs::create_dir_all(root.join("sessions")).expect("create the session store");
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the port");
    let stub = Arc::new(Stub {
        root,
        subscribers: Mutex::new(Vec::new()),
        live: Mutex::new(HashMap::new()),
        ended: Condvar::new(),
        asks: Mutex::new(HashMap::new()),
        cut: Mutex::new(None),
        ids: AtomicU64::new(0),
    });
    let auth = format!("Basic {}", base64(&format!("opencode:{password}")));
    for stream in listener.incoming().flatten() {
        let (stub, auth) = (stub.clone(), auth.clone());
        std::thread::spawn(move || {
            if let Some(request) = read_request(&stream, &auth) {
                stub.serve(stream, request);
            }
        });
    }
}

impl Stub {
    fn serve(self: Arc<Self>, mut stream: TcpStream, request: Request) {
        if !request.authorized {
            return respond(stream, 401, &json!({ "_tag": "UnauthorizedError" }));
        }
        let path = request.path.strip_prefix("/api").unwrap_or("");
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["info"]) => respond(
                stream,
                200,
                &json!({ "version": VERSION, "pid": std::process::id(), "urls": [], "paths": { "tmp": "/tmp" } }),
            ),
            ("GET", ["event"]) => {
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                let connected = "data: {\"type\":\"server.connected\",\"data\":{}}\n\n";
                if stream.write_all(head.as_bytes()).is_ok()
                    && stream.write_all(connected.as_bytes()).is_ok()
                {
                    self.subscribers.lock().unwrap().push(stream);
                }
            }
            ("GET", ["model"]) => {
                let model = |provider: &str, id: &str| json!({ "id": id, "providerID": provider, "name": id, "limit": { "context": 200000, "output": 8192 } });
                let mut models: Vec<Value> =
                    MODELS.iter().map(|id| model("opencode", id)).collect();
                for provider in self.keyed() {
                    models.push(model(&provider, KEYED_MODEL));
                }
                respond(stream, 200, &json!({ "data": models }))
            }
            ("GET", ["command" | "skill"]) => respond(stream, 200, &json!({ "data": [] })),
            ("GET", ["credential"]) => respond(stream, 200, &json!({ "data": self.credentials() })),
            ("POST", ["credential"]) => {
                let mut all = self.credentials();
                let id = format!("crd_{}", self.next());
                let entry = json!({
                    "id": id, "integrationID": request.body["integrationID"],
                    "label": request.body["label"], "active": true, "value": request.body["value"],
                });
                all.push(entry.clone());
                self.save_credentials(&all);
                respond(stream, 200, &json!({ "data": entry }))
            }
            ("DELETE", ["credential", id]) => {
                let mut all = self.credentials();
                all.retain(|c| c["id"] != *id);
                self.save_credentials(&all);
                respond_empty(stream)
            }
            ("POST", ["session"]) => {
                let id = format!("ses_stub{}", self.next());
                let model = request.body.pointer("/model/id").and_then(Value::as_str);
                let provider = request
                    .body
                    .pointer("/model/providerID")
                    .and_then(Value::as_str);
                let session = json!({
                    "model": model.map(|m| json!({ "id": m, "providerID": provider })),
                    "directory": request.body.pointer("/location/directory"),
                    "prompts": [], "messages": [],
                });
                self.save(&id, &session);
                respond(stream, 200, &json!({ "data": { "id": id } }))
            }
            ("GET", ["session", "active"]) => {
                let live = self.live.lock().unwrap();
                let active: serde_json::Map<String, Value> = live
                    .iter()
                    .filter(|(_, l)| l.running)
                    .map(|(id, _)| (id.clone(), json!({ "type": "running" })))
                    .collect();
                respond(stream, 200, &json!({ "data": active }))
            }
            ("GET", ["session", id]) => match self.load(id) {
                Some(_) => respond(stream, 200, &json!({ "data": { "id": id } })),
                None => not_found(stream, id),
            },
            ("DELETE", ["session", id]) => {
                let _ = std::fs::remove_file(self.file(id));
                respond_empty(stream)
            }
            ("POST", ["session", id, "model"]) => {
                let Some(mut session) = self.load(id) else {
                    return not_found(stream, id);
                };
                session["model"] = request.body["model"].clone();
                self.save(id, &session);
                respond_empty(stream)
            }
            ("GET", ["session", id, "message"]) => {
                let Some(session) = self.load(id) else {
                    return not_found(stream, id);
                };
                let mut messages = session["messages"].as_array().cloned().unwrap_or_default();
                if param(&request.query, "order") == Some("desc") {
                    messages.reverse();
                }
                let limit = param(&request.query, "limit").and_then(|n| n.parse().ok());
                messages.truncate(limit.unwrap_or(messages.len()));
                respond(stream, 200, &json!({ "data": messages, "cursor": {} }))
            }
            ("POST", ["session", id, "prompt"]) => {
                if self.load(id).is_none() {
                    return not_found(stream, id);
                }
                let message = request.body["id"].as_str().unwrap_or_default().to_string();
                let text = request.body["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let entry = json!({ "id": message, "sessionID": id, "type": "user",
                    "payload": { "text": text }, "delivery": "steer" });
                respond(stream, 200, &json!({ "data": entry }));
                self.emit(
                    "session.inbox.enqueued",
                    json!({ "sessionID": id, "inboxID": message,
                    "item": { "type": "user", "payload": { "text": text }, "delivery": "steer" } }),
                );
                self.admit(id, message, text);
            }
            ("GET", ["session", id, "inbox"]) => {
                let live = self.live.lock().unwrap();
                let inbox: Vec<Value> = live
                    .get(*id)
                    .map(|l| {
                        l.inbox
                            .iter()
                            .map(|(m, _)| json!({ "id": m, "type": "user" }))
                            .collect()
                    })
                    .unwrap_or_default();
                respond(stream, 200, &json!({ "data": inbox }))
            }
            ("DELETE", ["session", id, "inbox", message]) => {
                let removed = {
                    let mut live = self.live.lock().unwrap();
                    let inbox = &mut live.entry(id.to_string()).or_default().inbox;
                    let before = inbox.len();
                    inbox.retain(|(m, _)| m != message);
                    before != inbox.len()
                };
                respond_empty(stream);
                if removed {
                    self.emit(
                        "session.inbox.cancelled",
                        json!({ "sessionID": id, "inboxID": message }),
                    );
                }
            }
            ("PATCH", ["session", id, "inbox", _]) => {
                respond_empty(stream);
                self.wake(id);
            }
            ("POST", ["session", id, "interrupt"]) => self.interrupt(stream, id),
            ("GET", ["session", id, kind @ ("permission" | "form")]) => {
                let form = *kind == "form";
                let asks = self.asks.lock().unwrap();
                let open: Vec<&Value> = asks
                    .values()
                    .filter(|ask| ask.form == form && ask.owner == *id)
                    .map(|ask| &ask.request)
                    .collect();
                respond(stream, 200, &json!({ "data": open }))
            }
            ("POST", ["session", owner, "permission", ask, "reply"]) => {
                let allowed = request.body["decision"] != "reject";
                self.answer(
                    stream,
                    owner,
                    ask,
                    allowed,
                    "permission.replied",
                    json!({
                        "sessionID": owner, "requestID": ask, "reply": request.body["decision"],
                    }),
                );
            }
            ("POST", ["session", owner, "form", ask, "reply"]) => {
                self.answer(
                    stream,
                    owner,
                    ask,
                    true,
                    "form.replied",
                    json!({
                        "sessionID": owner, "id": ask, "answer": request.body["answer"],
                    }),
                );
            }
            ("DELETE", ["session", owner, "form", ask]) => {
                self.answer(
                    stream,
                    owner,
                    ask,
                    false,
                    "form.cancelled",
                    json!({ "sessionID": owner, "id": ask }),
                );
            }
            _ => respond(stream, 404, &json!({ "_tag": "NotFoundError" })),
        }
    }

    /// Admitted into an idle session, a message starts its run at once.
    fn admit(self: &Arc<Self>, id: &str, message: String, text: String) {
        let start = {
            let mut live = self.live.lock().unwrap();
            let session = live.entry(id.to_string()).or_default();
            session.inbox.push_back((message, text));
            !std::mem::replace(&mut session.running, true)
        };
        if start {
            self.start(id);
        }
    }

    fn wake(self: &Arc<Self>, id: &str) {
        let start = {
            let mut live = self.live.lock().unwrap();
            let session = live.entry(id.to_string()).or_default();
            !session.inbox.is_empty() && !std::mem::replace(&mut session.running, true)
        };
        if start {
            self.start(id);
        }
    }

    fn start(self: &Arc<Self>, id: &str) {
        let stop = Arc::new(AtomicBool::new(false));
        self.live.lock().unwrap().get_mut(id).unwrap().stop = stop.clone();
        self.emit("session.execution.started", json!({ "sessionID": id }));
        let (stub, id) = (self.clone(), id.to_string());
        std::thread::spawn(move || stub.run(&id, &stop));
    }

    /// Reads the inbox until it is empty, answering each message, then ends.
    fn run(&self, id: &str, stop: &AtomicBool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.subscribers.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let control = self.control();
        self.cut.lock().unwrap().clone_from(&control.cut);
        loop {
            if !pause(control.start, stop) {
                return self.stopped(id, stop);
            }
            let next = self
                .live
                .lock()
                .unwrap()
                .get_mut(id)
                .and_then(|l| l.inbox.pop_front());
            let Some((message, text)) = next else {
                break;
            };
            let Some(mut session) = self.load(id) else {
                break;
            };
            self.emit(
                "session.inbox.delivered",
                json!({ "sessionID": id, "inboxID": message }),
            );
            let model = session["model"].clone();
            let prompts: Vec<String> =
                serde_json::from_value(session["prompts"].clone()).unwrap_or_default();
            let reply = format!(
                "model={} earlier=[{}]",
                model["id"].as_str().unwrap_or("default"),
                prompts.join(" | ")
            );
            let mut all = prompts;
            all.push(text.clone());
            session["prompts"] = json!(all);
            push(
                &mut session,
                json!({ "id": message, "type": "user", "text": text, "time": { "created": now() } }),
            );
            self.save(id, &session);
            let failure = control.fail.clone().or_else(|| self.unknown_model(&model));
            if let Some(failure) = failure {
                let error = json!({ "type": "provider.unknown", "message": failure });
                self.end(id, "failed", json!({ "sessionID": id, "error": error }));
                return;
            }
            let assistant = format!("msg_stub{}", self.next());
            self.emit(
                "session.step.started",
                json!({ "sessionID": id, "assistantMessageID": assistant,
                "agent": "build", "model": model, "started": now() }),
            );
            for (on, form) in [(control.question, true), (control.ask, false)] {
                match on.then(|| self.asked(id, &assistant, form, &control, stop)) {
                    Some(Asked::Stopped) => return self.stopped(id, stop),
                    Some(Asked::Refused) => {
                        return self.end(
                            id,
                            "interrupted",
                            json!({ "sessionID": id, "reason": "shutdown" }),
                        );
                    }
                    _ => {}
                }
            }
            if !pause(control.reply, stop) {
                return self.stopped(id, stop);
            }
            let content = json!([{ "type": "text", "text": reply }]);
            self.emit("session.text.delta", json!({ "sessionID": id, "assistantMessageID": assistant, "ordinal": 0, "delta": reply }));
            self.emit("session.text.ended", json!({ "sessionID": id, "assistantMessageID": assistant, "ordinal": 0, "text": reply }));
            self.emit("session.step.ended", json!({ "sessionID": id, "assistantMessageID": assistant, "finish": "stop",
                "cost": 0.0, "tokens": { "input": 10, "output": 5, "reasoning": 0, "cache": { "read": 0, "write": 0 } } }));
            if let Some(mut session) = self.load(id) {
                push(
                    &mut session,
                    json!({ "id": assistant, "type": "assistant", "agent": "build", "content": content, "time": { "created": now() } }),
                );
                self.save(id, &session);
            }
        }
        self.end(id, "succeeded", json!({ "sessionID": id }));
    }

    /// Whether the run goes on: a refusal or a stop ends it.
    fn asked(
        &self,
        session: &str,
        assistant: &str,
        form: bool,
        control: &Control,
        stop: &AtomicBool,
    ) -> Asked {
        let ask = format!("{}_{}", if form { "frm" } else { "per" }, self.next());
        let owner = if control.child {
            let child = format!("ses_stubchild{}", self.next());
            self.emit(
                "session.created",
                json!({ "sessionID": child, "parentID": session }),
            );
            child
        } else {
            session.to_string()
        };
        let (answer, answered) = channel();
        let request = if form {
            json!({ "id": ask, "sessionID": owner, "title": "Questions", "fields": [{
                "key": "q0", "title": "Stub", "description": "Proceed?", "type": "string",
                "options": [{ "value": "yes", "label": "Yes" }],
            }] })
        } else {
            json!({ "id": ask, "sessionID": owner, "action": "bash", "resources": ["echo stub"],
                    "source": { "type": "tool", "messageID": assistant, "id": format!("call_{ask}") } })
        };
        let asked = request.clone();
        self.asks.lock().unwrap().insert(
            ask.clone(),
            Ask {
                owner,
                answer,
                request,
                form,
            },
        );
        if form {
            self.emit("form.created", json!({ "form": asked }));
        } else {
            self.emit("permission.asked", asked);
        }
        if control.aside {
            self.emit(
                "session.text.ended",
                json!({ "sessionID": session, "assistantMessageID": assistant,
                "ordinal": self.next(), "text": format!("waiting on {ask}") }),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match answered.recv_timeout(Duration::from_millis(50)) {
                Ok(true) => return Asked::Allowed,
                // As 2.4.1: a refusal fails the tool and ends the run, interrupted.
                Ok(false) => {
                    self.emit("session.tool.failed", json!({ "sessionID": session, "assistantMessageID": assistant,
                        "id": format!("call_{ask}"), "error": { "type": "aborted", "message": "The user declined this tool call" } }));
                    return Asked::Refused;
                }
                Err(RecvTimeoutError::Disconnected) => return Asked::Stopped,
                Err(RecvTimeoutError::Timeout) if stop.load(Ordering::SeqCst) => {
                    self.asks.lock().unwrap().remove(&ask);
                    return Asked::Stopped;
                }
                Err(RecvTimeoutError::Timeout) if Instant::now() > deadline => {
                    return Asked::Stopped
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn answer(
        &self,
        stream: TcpStream,
        owner: &str,
        ask: &str,
        allowed: bool,
        kind: &str,
        event: Value,
    ) {
        let waiting = {
            let mut asks = self.asks.lock().unwrap();
            match asks.get(ask) {
                Some(waiting) if waiting.owner == owner => asks.remove(ask),
                _ => None,
            }
        };
        let Some(waiting) = waiting else {
            let body = json!({ "_tag": "NotFoundError", "message": format!("No pending request {ask} in {owner}") });
            return respond(stream, 404, &body);
        };
        respond_empty(stream);
        self.emit(kind, event);
        let _ = waiting.answer.send(allowed);
    }

    /// As 2.4.1: a running session's run stops, dropping its asks with no reply; an idle one's
    /// stop is a no-op. Answered once the run has ended.
    fn interrupt(&self, stream: TcpStream, id: &str) {
        let control = self.control();
        std::thread::sleep(control.abort_delay);
        let stop = {
            let live = self.live.lock().unwrap();
            live.get(id).filter(|l| l.running).map(|l| l.stop.clone())
        };
        let Some(stop) = stop else {
            return respond(stream, 200, &json!({ "interrupted": false }));
        };
        if control.late_ask {
            self.emit(
                "permission.asked",
                json!({ "id": format!("per_{}", self.next()), "sessionID": id,
                "action": "bash", "resources": ["echo stub"] }),
            );
        }
        stop.store(true, Ordering::SeqCst);
        let mut live = self.live.lock().unwrap();
        while live.get(id).is_some_and(|l| l.running) {
            live = self
                .ended
                .wait_timeout(live, Duration::from_secs(5))
                .unwrap()
                .0;
        }
        drop(live);
        respond(stream, 200, &json!({ "interrupted": true }))
    }

    fn stopped(&self, id: &str, stop: &AtomicBool) {
        if stop.load(Ordering::SeqCst) {
            self.asks.lock().unwrap().retain(|_, ask| ask.owner != id);
            self.end(
                id,
                "interrupted",
                json!({ "sessionID": id, "reason": "user" }),
            );
        } else {
            self.end(
                id,
                "failed",
                json!({ "sessionID": id, "error": { "type": "timeout", "message": "no answer" } }),
            );
        }
    }

    fn end(&self, id: &str, outcome: &str, event: Value) {
        if let Some(mut session) = self.load(id) {
            push(
                &mut session,
                json!({ "id": format!("msg_stub{}", self.next()), "type": "idle", "outcome": outcome, "time": { "created": now() } }),
            );
            self.save(id, &session);
        }
        {
            let mut live = self.live.lock().unwrap();
            if let Some(l) = live.get_mut(id) {
                l.running = false;
            }
        }
        self.emit(&format!("session.execution.{outcome}"), event);
        self.ended.notify_all();
    }

    fn emit(&self, kind: &str, data: Value) {
        let record = format!(
            "data: {}\n\n",
            json!({ "id": format!("evt_{}", self.next()), "type": kind, "data": data })
        );
        let mut subscribers = self.subscribers.lock().unwrap();
        let mut cut = self.cut.lock().unwrap();
        if cut.as_deref() == Some(kind) {
            *cut = None;
            for stream in subscribers.drain(..) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            return;
        }
        subscribers.retain_mut(|s| s.write_all(record.as_bytes()).is_ok());
    }

    fn next(&self) -> u64 {
        let n = self.ids.fetch_add(1, Ordering::SeqCst);
        (now() * 1000) + n
    }

    fn keyed(&self) -> Vec<String> {
        self.credentials()
            .iter()
            .filter_map(|c| c["integrationID"].as_str().map(str::to_string))
            .collect()
    }

    fn unknown_model(&self, model: &Value) -> Option<String> {
        let provider = model.get("providerID")?.as_str()?;
        let id = model.get("id")?.as_str()?;
        let served = match provider {
            "opencode" => MODELS.contains(&id),
            _ => self.keyed().iter().any(|p| p == provider) && id == KEYED_MODEL,
        };
        (!served).then(|| format!("Model not found: {provider}/{id}"))
    }

    fn control(&self) -> Control {
        let control: Value = std::fs::read(self.root.join("control.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or(Value::Null);
        let ms = |key: &str| Duration::from_millis(control[key].as_u64().unwrap_or(0));
        let on = |key: &str| control[key].as_bool().unwrap_or(false);
        Control {
            start: ms("start"),
            reply: ms("reply"),
            abort_delay: ms("abortDelay"),
            ask: on("ask"),
            question: on("question"),
            aside: on("aside"),
            child: on("child"),
            late_ask: on("lateAsk"),
            fail: control["fail"].as_str().map(str::to_string),
            cut: control["cut"].as_str().map(str::to_string),
        }
    }

    fn credentials(&self) -> Vec<Value> {
        std::fs::read(self.root.join("credentials.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    fn save_credentials(&self, all: &[Value]) {
        std::fs::write(self.root.join("credentials.json"), json!(all).to_string())
            .expect("save credentials");
    }

    fn file(&self, id: &str) -> PathBuf {
        self.root.join("sessions").join(format!("{id}.json"))
    }

    fn load(&self, id: &str) -> Option<Value> {
        serde_json::from_slice(&std::fs::read(self.file(id)).ok()?).ok()
    }

    fn save(&self, id: &str, session: &Value) {
        std::fs::write(self.file(id), session.to_string()).expect("save a session");
    }
}

enum Asked {
    Allowed,
    Refused,
    Stopped,
}

struct Control {
    start: Duration,
    reply: Duration,
    abort_delay: Duration,
    ask: bool,
    question: bool,
    aside: bool,
    child: bool,
    late_ask: bool,
    fail: Option<String>,
    cut: Option<String>,
}

fn push(session: &mut Value, message: Value) {
    if let Some(messages) = session["messages"].as_array_mut() {
        messages.push(message);
    }
}

/// Sleeps unless the run is stopped first; false once stopped.
fn pause(delay: Duration, stop: &AtomicBool) -> bool {
    let until = Instant::now() + delay;
    while Instant::now() < until {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    !stop.load(Ordering::SeqCst)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_millis() as u64
}

fn base64(raw: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = raw.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn read_request(stream: &TcpStream, auth: &str) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut words = line.split_whitespace();
    let method = words.next()?.to_string();
    let target = words.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let mut length = 0;
    let mut authorized = false;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
            if name.eq_ignore_ascii_case("authorization") && value.trim() == auth {
                authorized = true;
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        query,
        authorized,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    })
}

fn param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
}

fn respond(mut stream: TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        _ => "Not Found",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn respond_empty(mut stream: TcpStream) {
    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
}

fn not_found(stream: TcpStream, id: &str) {
    let body = json!({ "_tag": "SessionNotFoundError", "sessionID": id, "message": format!("Session not found: {id}") });
    respond(stream, 404, &body)
}
