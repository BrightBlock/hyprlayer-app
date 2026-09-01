use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::scripts::{self, ScriptKind};
use crate::store::now_ms;
use crate::terminal::PtySession;

pub use hyprlayer_models::script::{RunPhase, ScriptRuntime, ScriptStatus};

struct RunHandle {
    terminal_id: String,
    session: Arc<PtySession>,
    port: Option<u16>,
}

#[derive(Default)]
pub struct ScriptRegistry {
    runs: Mutex<HashMap<String, RunHandle>>,
    status: Mutex<HashMap<(String, ScriptKind), ScriptStatus>>,
    outputs: Mutex<HashMap<(String, ScriptKind), String>>,
    ports: Mutex<HashMap<String, u16>>,
}

const OUTPUT_CAP_CHARS: usize = 64 * 1024;

impl ScriptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn take_port(&self, ws: &str) -> u16 {
        let mut ports = self.ports.lock().await;
        if let Some(p) = ports.get(ws) {
            return *p;
        }
        let port = scripts::free_port().unwrap_or(3000);
        ports.insert(ws.to_string(), port);
        port
    }

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

    pub async fn set_output(&self, ws: &str, kind: ScriptKind, output: String) {
        let n = output.chars().count();
        let capped = if n > OUTPUT_CAP_CHARS {
            output.chars().skip(n - OUTPUT_CAP_CHARS).collect()
        } else {
            output
        };
        self.outputs
            .lock()
            .await
            .insert((ws.to_string(), kind), capped);
    }

    pub async fn get_output(&self, ws: &str, kind: ScriptKind) -> Option<String> {
        self.outputs
            .lock()
            .await
            .get(&(ws.to_string(), kind))
            .cloned()
    }

    pub async fn clear_output(&self, ws: &str, kind: ScriptKind) {
        self.outputs.lock().await.remove(&(ws.to_string(), kind));
    }

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

    pub async fn set_run_port(&self, ws: &str, terminal_id: &str, port: u16) {
        if let Some(h) = self.runs.lock().await.get_mut(ws) {
            if h.terminal_id == terminal_id {
                h.port = Some(port);
            }
        }
    }

    pub async fn live_run(&self, ws: &str) -> Option<(String, Option<u16>)> {
        self.runs
            .lock()
            .await
            .get(ws)
            .map(|h| (h.terminal_id.clone(), h.port))
    }

    pub async fn stop_run(&self, ws: &str) -> bool {
        if let Some(h) = self.runs.lock().await.remove(ws) {
            h.session.kill();
            true
        } else {
            false
        }
    }

    pub async fn clear_run(&self, ws: &str, terminal_id: &str) {
        let mut runs = self.runs.lock().await;
        if runs.get(ws).is_some_and(|h| h.terminal_id == terminal_id) {
            runs.remove(ws);
        }
    }

    pub async fn running_ids(&self) -> Vec<String> {
        self.runs.lock().await.keys().cloned().collect()
    }

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
        assert!(reg.get_runtime("ws2").await.statuses.is_empty());

        reg.clear_workstream("ws1").await;
        let cleared = reg.get_runtime("ws1").await;
        assert!(cleared.statuses.is_empty());
        assert!(reg.ports.lock().await.get("ws1").is_none());
    }

    #[tokio::test]
    async fn clear_output_drops_only_that_script() {
        let reg = ScriptRegistry::new();
        reg.set_output("ws1", ScriptKind::Run, "run log".into())
            .await;
        reg.set_output("ws1", ScriptKind::Setup, "setup log".into())
            .await;
        reg.set_status("ws1", ScriptKind::Run, RunPhase::Passed, Some(0))
            .await;

        reg.clear_output("ws1", ScriptKind::Run).await;

        assert_eq!(reg.get_output("ws1", ScriptKind::Run).await, None);
        assert_eq!(
            reg.get_output("ws1", ScriptKind::Setup).await.as_deref(),
            Some("setup log")
        );
        assert_eq!(reg.get_runtime("ws1").await.statuses.len(), 1);
    }
}
