use std::collections::HashSet;
use std::sync::{Arc, Weak};

use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::claude::workflow::{self, SessionWorkflows, WorkflowLaunch};
use crate::claude::SessionEvent;
use crate::store::Db;

#[derive(Clone)]
pub(crate) struct WorkflowChannel {
    pub tx: mpsc::UnboundedSender<SessionEvent>,
    pub rx: Arc<Mutex<mpsc::UnboundedReceiver<SessionEvent>>>,
    pub workflows: Arc<Mutex<SessionWorkflows>>,
}

impl WorkflowChannel {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            workflows: Arc::new(Mutex::new(SessionWorkflows::new())),
        }
    }
}

pub(crate) fn spawn_workflow_ingress_pump(
    workflow: &WorkflowChannel,
    ingress: mpsc::Sender<SessionEvent>,
) -> JoinHandle<()> {
    let rx = workflow.rx.clone();
    tokio::spawn(async move {
        loop {
            let event = rx.lock().await.recv().await;
            match event {
                Some(event) => {
                    if ingress.send(event).await.is_err() {
                        return;
                    }
                }
                None => return,
            }
        }
    })
}

pub(crate) fn spawn_workflow_reader(
    workflows: Weak<Mutex<SessionWorkflows>>,
    notify: Arc<Notify>,
    tx: mpsc::UnboundedSender<SessionEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            notify.notified().await;
            let Some(handle) = workflows.upgrade() else {
                return;
            };
            let events =
                match tokio::task::spawn_blocking(move || handle.blocking_lock().runs.read_tick())
                    .await
                {
                    Ok(events) => events,
                    Err(err) => {
                        tracing::warn!("workflow reader tick failed: {err}");
                        return;
                    }
                };
            for event in events {
                if tx.send(event).is_err() {
                    return;
                }
            }
        }
    })
}

pub(crate) async fn arm_workflow_run(
    w: &WorkflowChannel,
    launch: WorkflowLaunch,
    workflow_tool_use_id: String,
    armed_seq: i64,
    seen_uses: HashSet<String>,
    seen_results: HashSet<String>,
) {
    let Some(workflows_dir) = launch.run_dir.parent().map(|p| p.to_path_buf()) else {
        tracing::warn!(
            "workflow run dir {} has no parent; not arming",
            launch.run_dir.display()
        );
        return;
    };

    let workflows = w.workflows.clone();
    let armed = tokio::task::spawn_blocking(move || {
        let mut guard = workflows.blocking_lock();
        guard.runs.register_seeded(
            launch,
            workflow_tool_use_id,
            armed_seq,
            seen_uses,
            seen_results,
        );
        guard.arm(&workflows_dir, Box::new(|| {}))
    })
    .await;
    let (notify, newly_armed) = match armed {
        Ok(Ok(armed)) => armed,
        Ok(Err(err)) => {
            tracing::warn!("could not arm the workflow watch: {err}");
            return;
        }
        Err(err) => {
            tracing::warn!("workflow arming task failed: {err}");
            return;
        }
    };

    if newly_armed {
        let reader =
            spawn_workflow_reader(Arc::downgrade(&w.workflows), notify.clone(), w.tx.clone());
        let workflows = w.workflows.clone();
        let _ = tokio::task::spawn_blocking(move || {
            workflows.blocking_lock().attach_reader(reader);
        })
        .await;
    }
    notify.notify_one();
}

pub(crate) async fn recover_workflow_runs(db: &Arc<Db>, session_id: &str, w: &WorkflowChannel) {
    let sid = session_id.to_string();
    let handle = db.clone();
    let events = match tokio::task::spawn_blocking(move || handle.session_log(&sid)).await {
        Ok(Ok(events)) => events,
        Ok(Err(err)) => {
            tracing::warn!("session {session_id}: could not scan the log for workflow runs: {err}");
            return;
        }
        Err(err) => {
            tracing::warn!("session {session_id}: workflow recovery scan failed: {err}");
            return;
        }
    };

    let mut workflow_tool_ids: HashSet<String> = HashSet::new();
    let mut seen_uses: HashSet<String> = HashSet::new();
    let mut seen_results: HashSet<String> = HashSet::new();
    let mut ended: HashSet<String> = HashSet::new();
    let mut launches: Vec<(WorkflowLaunch, String, i64)> = Vec::new();

    for stored in &events {
        match &stored.event {
            SessionEvent::ToolUse { id, name, .. } => {
                if name == "Workflow" {
                    workflow_tool_ids.insert(id.clone());
                }
                seen_uses.insert(id.clone());
            }
            SessionEvent::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                if workflow_tool_ids.contains(tool_use_id) {
                    if let Some(launch) = workflow::workflow_launch_from_result(content) {
                        launches.push((launch, tool_use_id.clone(), stored.seq));
                    }
                }
                seen_results.insert(tool_use_id.clone());
            }
            SessionEvent::BackgroundTaskUpdate {
                tool_use_id,
                status,
                ..
            } => {
                if let Some(id) = tool_use_id.clone().filter(|_| status != "running") {
                    ended.insert(id);
                }
            }
            _ => {}
        }
    }

    for (launch, tool_use_id, armed_seq) in launches {
        if ended.contains(&tool_use_id) {
            continue;
        }
        arm_workflow_run(
            w,
            launch,
            tool_use_id,
            armed_seq,
            seen_uses.clone(),
            seen_results.clone(),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_dropped_session_reaps_its_workflow_watch_and_reader() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workflows = Arc::new(Mutex::new(SessionWorkflows::new()));
        let (tx, rx) = mpsc::unbounded_channel::<SessionEvent>();

        let notify = {
            let mut guard = workflows.lock().await;
            let (notify, armed) = guard
                .arm(&tmp.path().join("subagents/workflows"), Box::new(|| {}))
                .expect("arming creates the directory it watches");
            assert!(armed, "the session's watch must really be armed");
            let reader =
                spawn_workflow_reader(Arc::downgrade(&workflows), notify.clone(), tx.clone());
            guard.attach_reader(reader);
            notify
        };

        let weak = Arc::downgrade(&workflows);
        drop(workflows);
        drop(tx);
        drop(rx);

        assert!(
            weak.upgrade().is_none(),
            "the reader task is still holding the session's workflows alive, so the debouncer and the task itself can never be stopped"
        );

        for _ in 0..200 {
            if Arc::strong_count(&notify) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            Arc::strong_count(&notify),
            1,
            "the reader task outlived the watch that was supposed to abort it"
        );
    }

    #[tokio::test]
    async fn dropping_the_workflow_channel_closes_its_ingress_pump() {
        let (ingress_tx, mut ingress_rx) = mpsc::channel::<SessionEvent>(8);
        let workflow = WorkflowChannel::new();
        let pump = spawn_workflow_ingress_pump(&workflow, ingress_tx);

        workflow
            .tx
            .send(SessionEvent::Rewound {})
            .expect("channel is open");
        let received = ingress_rx.recv().await.expect("pump forwards the event");
        assert!(matches!(received, SessionEvent::Rewound {}));

        drop(workflow);

        for _ in 0..200 {
            if pump.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            pump.is_finished(),
            "the pump must end once its channel's last sender is dropped"
        );
    }
}
