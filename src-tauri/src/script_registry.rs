//! In-memory registry for workstream lifecycle scripts: per workstream, the live
//! Run PTY (so ▶ can stop a prior one), its once-allocated `$HL_PORT`, and each
//! script's last-run status (the dock/settings pills). Session-scoped by design.

use std::collections::HashMap;
use std::sync::Arc;

use hyprlayer_core::scripts::{self, ScriptKind};
use hyprlayer_core::store::now_ms;
use hyprlayer_core::terminal::PtySession;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Outcome of a script run, shown as a status pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum RunPhase {
    Running,
    Passed,
    Failed,
}

/// Last-run status for one script in one workstream. `at` is unix-millis.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScriptStatus {
    pub kind: ScriptKind,
    pub phase: RunPhase,
    pub exit_code: Option<i32>,
    pub at: f64,
}

/// Snapshot of a workstream's script runtime, for UI hydration on (re)mount —
/// the dock/rail read this to recover the live Run + last-run pills after a
/// remount or workstream switch.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRuntime {
    /// terminal_id of the live Run PTY, if one is running (else `null`).
    pub run_terminal_id: Option<String>,
    /// Detected dev-server port for the live Run, if any.
    pub port: Option<u16>,
    /// Last-run status per script that has run this session.
    pub statuses: Vec<ScriptStatus>,
}

/// A live Run process for one workstream.
struct RunHandle {
    terminal_id: String,
    session: Arc<PtySession>,
    port: Option<u16>,
}

#[derive(Default)]
pub struct ScriptRegistry {
    /// workstream_id -> live Run.
    runs: Mutex<HashMap<String, RunHandle>>,
    /// (workstream_id, kind) -> last-run status.
    status: Mutex<HashMap<(String, ScriptKind), ScriptStatus>>,
    /// (workstream_id, kind) -> last-run output (captured terminal text, capped).
    outputs: Mutex<HashMap<(String, ScriptKind), String>>,
    /// workstream_id -> `$HL_PORT`, allocated once and reused for that workstream.
    ports: Mutex<HashMap<String, u16>>,
}

/// Cap stored output so a chatty install doesn't balloon memory / the IPC payload.
const OUTPUT_CAP_CHARS: usize = 64 * 1024;

impl ScriptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The workstream's `$HL_PORT`: a free port allocated once and then reused so
    /// Setup and Run agree. Falls back to a fixed port only if allocation fails.
    pub async fn take_port(&self, ws: &str) -> u16 {
        let mut ports = self.ports.lock().await;
        if let Some(p) = ports.get(ws) {
            return *p;
        }
        let port = scripts::free_port().unwrap_or(3000);
        ports.insert(ws.to_string(), port);
        port
    }

    /// Record a script's last-run status (timestamp stamped now).
    pub async fn set_status(
        &self,
        ws: &str,
        kind: ScriptKind,
        phase: RunPhase,
        exit_code: Option<i32>,
    ) {
        self.status.lock().await.insert(
            (ws.to_string(), kind),
            ScriptStatus {
                kind,
                phase,
                exit_code,
                at: now_ms(),
            },
        );
    }

    /// Store a script's last-run output (captured text), keeping only the tail.
    pub async fn set_output(&self, ws: &str, kind: ScriptKind, output: String) {
        let n = output.chars().count();
        let capped = if n > OUTPUT_CAP_CHARS {
            output.chars().skip(n - OUTPUT_CAP_CHARS).collect()
        } else {
            output
        };
        self.outputs.lock().await.insert((ws.to_string(), kind), capped);
    }

    /// The last-run output for one script, if captured this session.
    pub async fn get_output(&self, ws: &str, kind: ScriptKind) -> Option<String> {
        self.outputs.lock().await.get(&(ws.to_string(), kind)).cloned()
    }

    /// Mark a workstream's Run process as live (called when ▶ spawns one).
    pub async fn register_run(&self, ws: &str, terminal_id: String, session: Arc<PtySession>) {
        self.runs.lock().await.insert(
            ws.to_string(),
            RunHandle {
                terminal_id,
                session,
                port: None,
            },
        );
    }

    /// Attach a detected dev-server port to the live Run — but only if it's still
    /// the same run that opened (guards against a stale monitor from a prior run).
    pub async fn set_run_port(&self, ws: &str, terminal_id: &str, port: u16) {
        if let Some(h) = self.runs.lock().await.get_mut(ws) {
            if h.terminal_id == terminal_id {
                h.port = Some(port);
            }
        }
    }

    /// The live Run's (terminal_id, detected port), if any.
    pub async fn live_run(&self, ws: &str) -> Option<(String, Option<u16>)> {
        self.runs
            .lock()
            .await
            .get(ws)
            .map(|h| (h.terminal_id.clone(), h.port))
    }

    /// Stop the live Run (kill its process group via the PTY). Returns whether
    /// one was actually running.
    pub async fn stop_run(&self, ws: &str) -> bool {
        if let Some(h) = self.runs.lock().await.remove(ws) {
            h.session.kill();
            true
        } else {
            false
        }
    }

    /// Drop the live-run record IF it still names `terminal_id` — called from the
    /// monitor on exit so a freshly-started run (restart) isn't clobbered.
    pub async fn clear_run(&self, ws: &str, terminal_id: &str) {
        let mut runs = self.runs.lock().await;
        if runs.get(ws).is_some_and(|h| h.terminal_id == terminal_id) {
            runs.remove(ws);
        }
    }

    /// Workstreams with a live Run — drives the rail's green indicator.
    pub async fn running_ids(&self) -> Vec<String> {
        self.runs.lock().await.keys().cloned().collect()
    }

    /// Assemble the runtime snapshot for one workstream.
    pub async fn get_runtime(&self, ws: &str) -> ScriptRuntime {
        let (run_terminal_id, port) = match self.live_run(ws).await {
            Some((tid, port)) => (Some(tid), port),
            None => (None, None),
        };
        let statuses = self
            .status
            .lock()
            .await
            .iter()
            .filter(|((id, _), _)| id == ws)
            .map(|(_, v)| v.clone())
            .collect();
        ScriptRuntime {
            run_terminal_id,
            port,
            statuses,
        }
    }

    /// Forget everything for a workstream (archive path): kill its Run, drop its
    /// statuses + cached port.
    pub async fn clear_workstream(&self, ws: &str) {
        if let Some(h) = self.runs.lock().await.remove(ws) {
            h.session.kill();
        }
        self.status.lock().await.retain(|(id, _), _| id != ws);
        self.outputs.lock().await.retain(|(id, _), _| id != ws);
        self.ports.lock().await.remove(ws);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprlayer_core::scripts::ScriptKind;

    #[tokio::test]
    async fn port_allocated_once_and_reused_per_workstream() {
        let reg = ScriptRegistry::new();
        let p1 = reg.take_port("ws1").await;
        let p2 = reg.take_port("ws1").await;
        assert_eq!(p1, p2, "a workstream reuses its allocated port");
        assert!(p1 > 0);
    }

    #[tokio::test]
    async fn statuses_record_per_workstream_and_clear() {
        let reg = ScriptRegistry::new();
        reg.set_status("ws1", ScriptKind::Setup, RunPhase::Passed, Some(0))
            .await;
        reg.set_status("ws1", ScriptKind::Archive, RunPhase::Failed, Some(1))
            .await;

        let rt = reg.get_runtime("ws1").await;
        assert_eq!(rt.statuses.len(), 2);
        assert!(rt.run_terminal_id.is_none());
        // Other workstreams are isolated.
        assert!(reg.get_runtime("ws2").await.statuses.is_empty());

        reg.clear_workstream("ws1").await;
        let cleared = reg.get_runtime("ws1").await;
        assert!(cleared.statuses.is_empty());
        assert!(reg.ports.lock().await.get("ws1").is_none());
    }
}
