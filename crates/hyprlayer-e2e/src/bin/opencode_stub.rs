//! `opencode serve` for E2E: the routes the OpenCode driver calls, shaped as 1.18.32 answers them.
//! Sessions live under `$XDG_DATA_HOME`, so they survive a restart and vanish with a data wipe.
//! A turn replies `model=<model> earlier=[<this session's earlier prompts>]`.
//!
//! `$XDG_DATA_HOME/opencode-stub/control.json` delays, in ms, the events 1.18.32 sends late:
//! `start` (the prompt's user message), `reply`, `echo` (the user message again after the idle),
//! `abortIdle` (an idle session's abort) and `late` (an aborted turn's reply and second idle).
//! `"ask": true` makes each turn ask permission for bash first. `"ownIds": true` ignores the
//! prompt's messageID; `"bare": true` also mints ids that sort below the driver's and omits
//! `time.created`; `"fail": "<message>"` fails each turn before any reply. `"nested": true`
//! names the session only inside each message and part, as 1.0.224 does.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const MODELS: [&str; 2] = ["stub-free", "stub-other-free"];

struct Request {
    method: String,
    path: String,
    body: Value,
}

struct Running {
    user: String,
    assistant: String,
    aborted: AtomicBool,
}

struct Stub {
    root: PathBuf,
    subscribers: Mutex<Vec<TcpStream>>,
    running: Mutex<HashMap<String, Arc<Running>>>,
    permissions: Mutex<HashMap<String, Sender<bool>>>,
    last_id: Mutex<u64>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("1.18.32");
        return;
    }
    let port: u16 = args
        .windows(2)
        .find(|pair| pair[0] == "--port")
        .and_then(|pair| pair[1].parse().ok())
        .expect("--port");
    let data = std::env::var_os("XDG_DATA_HOME").expect("XDG_DATA_HOME");
    let root = PathBuf::from(data).join("opencode-stub");
    std::fs::create_dir_all(root.join("sessions")).expect("create the session store");
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the port");
    let stub = Arc::new(Stub {
        root,
        subscribers: Mutex::new(Vec::new()),
        running: Mutex::new(HashMap::new()),
        permissions: Mutex::new(HashMap::new()),
        last_id: Mutex::new(0),
    });
    for stream in listener.incoming().flatten() {
        let stub = stub.clone();
        std::thread::spawn(move || stub.serve(stream));
    }
}

impl Stub {
    fn serve(self: Arc<Self>, mut stream: TcpStream) {
        let Some(request) = read_request(&stream) else {
            return;
        };
        let segments: Vec<&str> = request.path.trim_matches('/').split('/').collect();
        match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["global", "health"]) => respond(
                stream,
                200,
                &json!({ "healthy": true, "version": "1.18.32" }),
            ),
            ("GET", ["global", "event"]) => {
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                if stream.write_all(head.as_bytes()).is_ok() {
                    self.subscribers.lock().unwrap().push(stream);
                }
            }
            ("GET", ["config", "providers"]) => {
                let models: serde_json::Map<String, Value> = MODELS
                    .iter()
                    .map(|id| {
                        let model = json!({ "id": id, "name": id, "limit": { "context": 200000 } });
                        (id.to_string(), model)
                    })
                    .collect();
                let providers =
                    json!({ "providers": [{ "id": "opencode", "models": models }], "default": {} });
                respond(stream, 200, &providers)
            }
            ("GET", ["command"]) => respond(stream, 200, &json!([])),
            ("POST", ["session"]) => {
                let id = format!("ses_stub{}", nanos());
                self.save(&id, &[]);
                respond(stream, 200, &json!({ "id": id }))
            }
            ("GET", ["session", id]) => match self.load(id) {
                Some(_) => respond(stream, 200, &json!({ "id": id })),
                None => not_found(stream, id),
            },
            ("DELETE", ["session", id]) => {
                let _ = std::fs::remove_file(self.file(id));
                respond(stream, 200, &json!(true))
            }
            ("POST", ["session", id, "prompt_async"]) => {
                let Some(mut prompts) = self.load(id) else {
                    return not_found(stream, id);
                };
                let body = &request.body;
                let text = body.pointer("/parts/0/text").and_then(Value::as_str);
                let model = body.pointer("/model/modelID").and_then(Value::as_str);
                let reply = format!(
                    "model={} earlier=[{}]",
                    model.unwrap_or("default"),
                    prompts.join(" | ")
                );
                prompts.push(text.unwrap_or_default().to_string());
                self.save(id, &prompts);
                let control = self.delays();
                let user = match body.get("messageID").and_then(Value::as_str) {
                    Some(user) if !control.own_ids && !control.bare => user.to_string(),
                    _ => self.new_id(control.bare),
                };
                respond_empty(stream);
                self.turn(id, user, &reply, control);
            }
            ("POST", ["session", _, "permissions", permission]) => {
                let allowed = request.body["response"] != "reject";
                let waiting = self.permissions.lock().unwrap().remove(*permission);
                respond(stream, 200, &json!(true));
                if let Some(waiting) = waiting {
                    let _ = waiting.send(allowed);
                }
            }
            ("POST", ["session", id, "abort"]) => {
                respond(stream, 200, &json!(true));
                self.abort(id);
            }
            _ => respond(stream, 404, &json!({ "name": "NotFoundError" })),
        }
    }

    fn turn(&self, id: &str, user: String, reply: &str, delays: Delays) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.subscribers.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        sleep(delays.start);
        let created = (!delays.bare).then(|| nanos() / 1_000_000);
        let running = Arc::new(Running {
            assistant: self.new_id(delays.bare),
            user,
            aborted: AtomicBool::new(false),
        });
        let (user, assistant) = (&running.user, &running.assistant);
        self.running
            .lock()
            .unwrap()
            .insert(id.to_string(), running.clone());
        self.emit("message.updated", user_info(id, user, created));
        if let Some(message) = &delays.fail {
            // As 1.18.32 fails an unknown model: no reply, and the error again after the idle.
            self.running.lock().unwrap().remove(id);
            let error = json!({ "name": "UnknownError", "data": { "message": message } });
            self.emit("session.error", json!({ "sessionID": id, "error": error }));
            self.emit("session.idle", json!({ "sessionID": id }));
            self.emit("session.error", json!({ "sessionID": id, "error": error }));
            return;
        }
        self.emit(
            "message.updated",
            assistant_info(id, assistant, user, json!({})),
        );
        if delays.ask && !self.permitted(id) {
            // 1.18.32 ends a denied turn on its tool-calls step, with no error.
            self.running.lock().unwrap().remove(id);
            let step = json!({ "finish": "tool-calls", "time": { "created": 1, "completed": 2 } });
            self.emit("message.updated", assistant_info(id, assistant, user, step));
            self.emit("session.idle", json!({ "sessionID": id }));
            return;
        }
        sleep(delays.reply);
        if running.aborted.load(Ordering::SeqCst) {
            return;
        }
        self.running.lock().unwrap().remove(id);
        self.emit(
            "message.part.updated",
            json!({ "sessionID": id, "part": {
                "id": format!("prt_{}", nanos()), "sessionID": id, "messageID": assistant,
                "type": "text", "text": reply, "time": { "start": 1, "end": 2 },
            } }),
        );
        let finished = json!({ "finish": "stop", "time": { "created": 1, "completed": 2 } });
        self.emit(
            "message.updated",
            assistant_info(id, assistant, user, finished),
        );
        self.emit("session.idle", json!({ "sessionID": id }));
        sleep(delays.echo);
        self.emit("message.updated", user_info(id, user, created));
    }

    fn permitted(&self, session: &str) -> bool {
        let permission = format!("per_{}", nanos());
        let (answer, answered) = channel();
        self.permissions
            .lock()
            .unwrap()
            .insert(permission.clone(), answer);
        self.emit(
            "permission.asked",
            json!({ "id": permission, "sessionID": session, "permission": "bash",
                    "patterns": ["echo stub"], "metadata": {} }),
        );
        let allowed = answered
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or(false);
        self.emit(
            "permission.replied",
            json!({ "sessionID": session, "requestID": permission }),
        );
        allowed
    }

    /// As 1.18.32: an idle session reports `session.idle`; a busy one an aborted error and an
    /// idle, then its reply ends in that error and a second idle follows.
    fn abort(&self, id: &str) {
        let delays = self.delays();
        let Some(running) = self.running.lock().unwrap().remove(id) else {
            sleep(delays.abort_idle);
            return self.emit("session.idle", json!({ "sessionID": id }));
        };
        running.aborted.store(true, Ordering::SeqCst);
        let error = json!({ "name": "MessageAbortedError", "data": { "message": "Aborted" } });
        self.emit("session.error", json!({ "sessionID": id, "error": error }));
        self.emit("session.idle", json!({ "sessionID": id }));
        sleep(delays.late);
        let aborted = json!({ "error": error, "time": { "created": 1, "completed": 2 } });
        self.emit(
            "message.updated",
            assistant_info(id, &running.assistant, &running.user, aborted),
        );
        self.emit("session.idle", json!({ "sessionID": id }));
    }

    fn emit(&self, kind: &str, mut properties: Value) {
        let message = matches!(kind, "message.updated" | "message.part.updated");
        if let Some(p) = properties
            .as_object_mut()
            .filter(|_| message && self.delays().nested)
        {
            p.remove("sessionID");
        }
        let record = format!(
            "data: {}\n\n",
            json!({ "type": kind, "properties": properties })
        );
        self.subscribers
            .lock()
            .unwrap()
            .retain_mut(|s| s.write_all(record.as_bytes()).is_ok());
    }

    /// 1.18.32's ascending id: the low 48 bits of `ms * 0x1000 + counter`, counter from 1.
    fn message_id(&self) -> String {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after 1970")
            .as_millis() as u64;
        let mut last = self.last_id.lock().unwrap();
        *last = (ms * 0x1000 + 1).max(*last + 1);
        format!("msg_{:012x}stubstubstubst", *last & 0xffff_ffff_ffff)
    }

    /// Ids from a version whose scheme sorts below the driver's, in place of 1.18.32's.
    fn new_id(&self, bare: bool) -> String {
        if !bare {
            return self.message_id();
        }
        let mut last = self.last_id.lock().unwrap();
        *last += 1;
        format!("msg_0{:025}", *last)
    }

    fn delays(&self) -> Delays {
        let control: Value = std::fs::read(self.root.join("control.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or(Value::Null);
        let ms = |key: &str| Duration::from_millis(control[key].as_u64().unwrap_or(0));
        Delays {
            start: ms("start"),
            reply: ms("reply"),
            echo: ms("echo"),
            abort_idle: ms("abortIdle"),
            late: ms("late"),
            ask: control["ask"].as_bool().unwrap_or(false),
            own_ids: control["ownIds"].as_bool().unwrap_or(false),
            bare: control["bare"].as_bool().unwrap_or(false),
            nested: control["nested"].as_bool().unwrap_or(false),
            fail: control["fail"].as_str().map(str::to_string),
        }
    }

    fn file(&self, id: &str) -> PathBuf {
        self.root.join("sessions").join(format!("{id}.json"))
    }

    fn load(&self, id: &str) -> Option<Vec<String>> {
        serde_json::from_slice(&std::fs::read(self.file(id)).ok()?).ok()
    }

    fn save(&self, id: &str, prompts: &[String]) {
        std::fs::write(self.file(id), json!(prompts).to_string()).expect("save a session");
    }
}

struct Delays {
    start: Duration,
    reply: Duration,
    echo: Duration,
    abort_idle: Duration,
    late: Duration,
    ask: bool,
    own_ids: bool,
    bare: bool,
    nested: bool,
    fail: Option<String>,
}

fn user_info(session: &str, id: &str, created: Option<u128>) -> Value {
    let mut info = json!({ "id": id, "sessionID": session, "role": "user" });
    if let Some(created) = created {
        info["time"] = json!({ "created": created as u64 });
    }
    json!({ "sessionID": session, "info": info })
}

fn assistant_info(session: &str, id: &str, parent: &str, extra: Value) -> Value {
    let mut info = json!({
        "id": id, "sessionID": session, "role": "assistant", "agent": "build", "parentID": parent,
    });
    if let (Some(info), Value::Object(extra)) = (info.as_object_mut(), extra) {
        info.extend(extra);
    }
    json!({ "sessionID": session, "info": info })
}

fn sleep(delay: Duration) {
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut words = line.split_whitespace();
    let method = words.next()?.to_string();
    let target = words.next()?;
    let path = target.split('?').next().unwrap_or_default().to_string();
    let mut length = 0;
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
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    })
}

fn respond(mut stream: TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        if status == 200 { "OK" } else { "Not Found" },
        body.len()
    );
}

fn respond_empty(mut stream: TcpStream) {
    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
}

fn not_found(stream: TcpStream, id: &str) {
    let body = json!({ "name": "NotFoundError", "data": { "message": format!("Session not found: {id}") } });
    respond(stream, 404, &body)
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos()
}
