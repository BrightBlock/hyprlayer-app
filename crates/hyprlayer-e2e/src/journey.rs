use std::path::{Path, PathBuf};

use hyprlayer_proto::WireError;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::rpc::Request;
use serde_json::{json, Map, Value};

use crate::sandbox::Sandbox;

const BLESS: &str = "HYPRLAYER_E2E_BLESS";

/// One scenario's record: the guards each step checked, every call on the
/// wire, and the artifact they become in `target/e2e/<scenario>/`.
pub struct Journey {
    scenario: &'static str,
    steps: Vec<Step>,
    transcript: Vec<Value>,
    aliases: Vec<(String, String)>,
    environment: Map<String, Value>,
    logs: PathBuf,
    app_data: PathBuf,
    finished: bool,
}

struct Step {
    name: String,
    guards: Vec<(String, bool, Value)>,
}

impl Journey {
    pub fn new(scenario: &'static str, sandbox: &Sandbox) -> Self {
        let mut journey = Self {
            scenario,
            steps: Vec::new(),
            transcript: Vec::new(),
            aliases: Vec::new(),
            environment: Map::new(),
            logs: sandbox.logs.clone(),
            app_data: sandbox.app_data.clone(),
            finished: false,
        };
        journey.alias(sandbox.root.display().to_string(), "$SANDBOX");
        journey.alias(sandbox.given.display().to_string(), "$SANDBOX");
        // Both diverge by platform: Linux binds the socket under $XDG_RUNTIME_DIR
        // with a hash of the absolute profile path, macOS in the data dir; the CLI
        // config sits under ~/.config or ~/Library/Application Support. Recording
        // either pins the golden to one platform and one checkout directory.
        journey.alias(sandbox.socket().display().to_string(), "$SOCKET");
        journey.alias(sandbox.cli_config().display().to_string(), "$CLI_CONFIG");
        journey.note("tools", sandbox.tool_versions());
        journey
    }

    /// Rewrites `literal` to `placeholder` throughout the artifact, so ids,
    /// slugs and temp paths compare equal across runs.
    pub fn alias(&mut self, literal: impl Into<String>, placeholder: impl Into<String>) {
        let literal = literal.into();
        if !literal.is_empty() {
            self.aliases.push((literal, placeholder.into()));
            self.aliases
                .sort_by_key(|(literal, _)| std::cmp::Reverse(literal.len()));
        }
    }

    pub fn step(&mut self, name: &str) {
        self.steps.push(Step {
            name: name.to_string(),
            guards: Vec::new(),
        });
    }

    /// Records one failure mode the step rules out, with its evidence.
    pub fn guard(&mut self, guard: &str, ok: bool, evidence: Value) -> bool {
        self.steps
            .last_mut()
            .expect("a guard belongs to a step")
            .guards
            .push((guard.to_string(), ok, evidence));
        ok
    }

    /// Machine-specific facts for `environment.json`, never compared.
    pub fn note(&mut self, key: &str, value: Value) {
        self.environment.insert(key.to_string(), value);
    }

    /// Sends one request and records both halves as they crossed the wire.
    pub async fn call(&mut self, client: &RpcClient, request: Request) -> Result<Value, WireError> {
        let command = request.command();
        let args = tagged(serde_json::to_value(&request), command);
        match client.call(request).await {
            Ok(response) => {
                let value = tagged(serde_json::to_value(&response), command);
                self.transcript
                    .push(json!({ "call": command, "args": args, "ok": value }));
                Ok(value)
            }
            Err(error) => {
                self.transcript
                    .push(json!({ "call": command, "args": args, "err": &error }));
                Err(error)
            }
        }
    }

    /// Records a stream event alongside the calls.
    pub fn event(&mut self, stream: &str, body: Value) {
        self.transcript
            .push(json!({ "stream": stream, "event": body }));
    }

    /// Writes the artifact, fails on any failed guard, then compares the
    /// report with its golden; `HYPRLAYER_E2E_BLESS=1` rewrites the golden.
    pub fn finish(mut self) {
        self.finished = true;
        let dir = self.write(true).expect("write the E2E artifact");
        let mut failed = Vec::new();
        for step in &self.steps {
            for (guard, ok, evidence) in &step.guards {
                if !ok {
                    failed.push(format!(
                        "  [{}] {guard}: {}",
                        step.name,
                        self.normalize(evidence)
                    ));
                }
            }
        }
        assert!(
            failed.is_empty(),
            "{} guard(s) failed; artifact at {}\n{}",
            failed.len(),
            dir.display(),
            failed.join("\n")
        );

        let report = self.report_text(true);
        let golden = golden_path(self.scenario);
        if std::env::var_os(BLESS).is_some() {
            std::fs::create_dir_all(golden.parent().expect("golden dir"))
                .expect("create golden dir");
            std::fs::write(&golden, &report).expect("bless the golden");
            return;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "no golden at {}; run once with {BLESS}=1 and review it",
                golden.display()
            )
        });
        if let Some((line, want, got)) = first_difference(&expected, &report) {
            panic!(
                "the report differs from its golden at line {line}\n  golden: {want}\n  actual: {got}\n\
                 artifact: {}\ngolden:   {}\nrerun with {BLESS}=1 if the change is intended",
                dir.join("report.json").display(),
                golden.display()
            );
        }
    }

    fn report(&self, complete: bool) -> Value {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|step| {
                let guards: Vec<Value> = step
                    .guards
                    .iter()
                    .map(|(guard, ok, evidence)| {
                        json!({ "guard": guard, "ok": ok, "evidence": evidence })
                    })
                    .collect();
                json!({ "step": step.name, "guards": guards })
            })
            .collect();
        let mut report = json!({ "scenario": self.scenario, "steps": steps });
        if !complete {
            report["incomplete"] = json!(true);
        }
        self.normalize(&report)
    }

    fn report_text(&self, complete: bool) -> String {
        serde_json::to_string_pretty(&self.report(complete)).expect("encode the report") + "\n"
    }

    fn write(&self, complete: bool) -> std::io::Result<PathBuf> {
        let dir = artifact_dir(self.scenario);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logs"))?;
        std::fs::write(dir.join("report.json"), self.report_text(complete))?;
        let transcript: String = self
            .transcript
            .iter()
            .map(|entry| self.normalize(entry).to_string() + "\n")
            .collect();
        std::fs::write(dir.join("transcript.jsonl"), transcript)?;
        let environment = Value::Object(self.environment.clone());
        std::fs::write(
            dir.join("environment.json"),
            serde_json::to_string_pretty(&environment)? + "\n",
        )?;
        for entry in std::fs::read_dir(&self.logs)?.flatten() {
            std::fs::copy(entry.path(), dir.join("logs").join(entry.file_name()))?;
        }
        self.write_session_logs(&dir);
        Ok(dir)
    }

    /// One `session-logs/<harness>-<id>.jsonl` per session in the sandbox's DB, for scripts/transcript-diff.
    fn write_session_logs(&self, dir: &std::path::Path) {
        let db = self.app_data.join("hyprlayer.db");
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            return;
        };
        let Ok(mut sessions) =
            conn.prepare("SELECT id, harness FROM sessions ORDER BY started_at, id")
        else {
            return;
        };
        let rows: Vec<(String, String)> = sessions
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        for (id, harness) in rows {
            let Ok(mut events) = conn.prepare(
                "SELECT seq, kind, payload, created_at FROM session_events WHERE session_id = ?1 ORDER BY seq",
            ) else {
                continue;
            };
            let lines: String = events
                .query_map([&id], |r| {
                    let payload: String = r.get(2)?;
                    let event: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
                    Ok(json!({ "seq": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "createdAt": r.get::<_, f64>(3)?, "event": event }))
                })
                .map(|rows| rows.flatten().map(|l| l.to_string() + "\n").collect())
                .unwrap_or_default();
            if lines.is_empty() {
                continue;
            }
            let out = dir.join("session-logs");
            let _ = std::fs::create_dir_all(&out);
            let _ = std::fs::write(
                out.join(format!("{harness}-{}.jsonl", &id[..id.len().min(8)])),
                lines,
            );
        }
    }

    fn normalize(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.scrub(text)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.normalize(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, v)| {
                        let v = if is_time(key) && v.is_number() {
                            json!("$TIME")
                        } else {
                            self.normalize(v)
                        };
                        (self.scrub(key), v)
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn scrub(&self, text: &str) -> String {
        self.aliases
            .iter()
            .fold(text.to_string(), |acc, (literal, placeholder)| {
                acc.replace(literal, placeholder)
            })
    }
}

impl Drop for Journey {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(dir) = self.write(false) {
                eprintln!("incomplete E2E run; partial artifact at {}", dir.display());
            }
        }
    }
}

fn tagged(value: serde_json::Result<Value>, command: &str) -> Value {
    value
        .ok()
        .and_then(|mut v| v.get_mut(command).map(Value::take))
        .unwrap_or(Value::Null)
}

/// Timestamps are the one kind of field that differs on every run by design.
fn is_time(key: &str) -> bool {
    key.ends_with("At")
        || key.ends_with("_at")
        || key.ends_with("Ms")
        || key.ends_with("_ms")
        || key == "mtime"
}

fn artifact_dir(scenario: &str) -> PathBuf {
    target_dir().join("e2e").join(scenario)
}

/// Cargo's target dir, from the running test's `<target>/<profile>/deps/<test>`.
pub(crate) fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("locate the test executable");
    exe.ancestors()
        .nth(3)
        .expect("<target>/<profile>/deps/<test>")
        .to_path_buf()
}

fn golden_path(scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("golden")
        .join(format!("{scenario}.json"))
}

fn first_difference(expected: &str, actual: &str) -> Option<(usize, String, String)> {
    let mut want = expected.lines();
    let mut got = actual.lines();
    let mut line = 1;
    loop {
        match (want.next(), got.next()) {
            (None, None) => return None,
            (w, g) if w == g => line += 1,
            (w, g) => {
                return Some((
                    line,
                    w.unwrap_or("<end>").to_string(),
                    g.unwrap_or("<end>").to_string(),
                ))
            }
        }
    }
}
