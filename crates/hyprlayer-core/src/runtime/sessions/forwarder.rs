use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use hyprlayer_models::RuntimeEvent;

use crate::agent::AgentDriver;
use crate::claude::{ExitOutcome, SessionEvent, SessionState};
use crate::error::{CoreError, Result};
use crate::store::session_event_codec::encode_raw_event;
use crate::store::{
    ApplySessionEventOutcome, Db, FieldOp, LogCursor, SessionLogEntry, SessionRowMutation,
    StoredEvent,
};

use super::delivery::{LiveAttachment, SessionAttachment, SessionDelivery, INGRESS_CAPACITY};
use super::workflow_bridge::{self, WorkflowChannel};

pub(crate) struct RewindOutcome {
    pub live_rewound: bool,
    pub epoch: u64,
    pub restart_seq: i64,
    pub removed: Vec<StoredEvent>,
}

pub(crate) enum Control {
    Attach {
        echo_user_messages: bool,
        reply: oneshot::Sender<Result<SessionAttachment>>,
    },
    LogAfter {
        caller_epoch: u64,
        after_seq: Option<i64>,
        reply: oneshot::Sender<Result<Vec<SessionLogEntry>>>,
    },
    AttachAfter {
        caller_epoch: u64,
        after_seq: Option<i64>,
        echo_user_messages: bool,
        reply: oneshot::Sender<Result<SessionAttachment>>,
    },
    AttachFrom {
        cursor: LogCursor,
        echo_user_messages: bool,
        reply: oneshot::Sender<Result<SessionAttachment>>,
    },
    Rewind {
        cutoff: i64,
        live: Option<(Arc<dyn AgentDriver>, usize)>,
        reply: oneshot::Sender<Result<RewindOutcome>>,
    },
    RestoreAfterFailedRelaunch {
        cutoff: i64,
        removed: Vec<StoredEvent>,
        reply: oneshot::Sender<Result<()>>,
    },
    SetWorkflow {
        workflow: WorkflowChannel,
        reply: oneshot::Sender<()>,
    },
    #[cfg(any(test, feature = "test-support"))]
    AttachmentCount { reply: oneshot::Sender<usize> },
}

pub(crate) struct ActorHandle {
    pub ingress: mpsc::Sender<SessionEvent>,
    control: mpsc::UnboundedSender<Control>,
    #[allow(dead_code)]
    join: JoinHandle<()>,
}

impl ActorHandle {
    pub(crate) async fn attach(&self, echo_user_messages: bool) -> Result<SessionAttachment> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::Attach {
                echo_user_messages,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn log_after(
        &self,
        caller_epoch: u64,
        after_seq: Option<i64>,
    ) -> Result<Vec<SessionLogEntry>> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::LogAfter {
                caller_epoch,
                after_seq,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn attach_after(
        &self,
        caller_epoch: u64,
        after_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::AttachAfter {
                caller_epoch,
                after_seq,
                echo_user_messages,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn attach_from(
        &self,
        cursor: LogCursor,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::AttachFrom {
                cursor,
                echo_user_messages,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn rewind(
        &self,
        cutoff: i64,
        live: Option<(Arc<dyn AgentDriver>, usize)>,
    ) -> Result<RewindOutcome> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::Rewind {
                cutoff,
                live,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn restore_after_failed_relaunch(
        &self,
        cutoff: i64,
        removed: Vec<StoredEvent>,
    ) -> Result<()> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::RestoreAfterFailedRelaunch {
                cutoff,
                removed,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn set_workflow(&self, workflow: WorkflowChannel) -> Result<()> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::SetWorkflow { workflow, reply })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn attachment_count(&self) -> usize {
        let (reply, reply_rx) = oneshot::channel();
        if self
            .control
            .send(Control::AttachmentCount { reply })
            .is_err()
        {
            return 0;
        }
        reply_rx.await.unwrap_or(0)
    }
}

fn actor_gone() -> CoreError {
    CoreError::Io(std::io::Error::other("session actor is no longer running"))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

pub(crate) fn spawn(
    session_id: String,
    db: Arc<Db>,
    events: broadcast::Sender<RuntimeEvent>,
) -> ActorHandle {
    let (ingress_tx, ingress_rx) = mpsc::channel(INGRESS_CAPACITY);
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let join = tokio::spawn(run(
        session_id,
        db,
        events,
        ingress_tx.clone(),
        ingress_rx,
        control_rx,
    ));
    ActorHandle {
        ingress: ingress_tx,
        control: control_tx,
        join,
    }
}

async fn run(
    session_id: String,
    db: Arc<Db>,
    events: broadcast::Sender<RuntimeEvent>,
    ingress_sender: mpsc::Sender<SessionEvent>,
    mut ingress_rx: mpsc::Receiver<SessionEvent>,
    mut control_rx: mpsc::UnboundedReceiver<Control>,
) {
    let mut state = ActorState::new(session_id, db, events, ingress_sender).await;
    loop {
        tokio::select! {
            biased;
            ctrl = control_rx.recv() => {
                match ctrl {
                    Some(Control::Attach { echo_user_messages, reply }) => {
                        let _ = reply.send(state.attach(echo_user_messages).await);
                    }
                    Some(Control::LogAfter { caller_epoch, after_seq, reply }) => {
                        let _ = reply.send(state.log_after(caller_epoch, after_seq).await);
                    }
                    Some(Control::AttachAfter { caller_epoch, after_seq, echo_user_messages, reply }) => {
                        let _ = reply.send(state.attach_after(caller_epoch, after_seq, echo_user_messages).await);
                    }
                    Some(Control::AttachFrom { cursor, echo_user_messages, reply }) => {
                        let _ = reply.send(state.attach_from(cursor, echo_user_messages).await);
                    }
                    Some(Control::Rewind { cutoff, live, reply }) => {
                        let _ = reply.send(state.rewind(cutoff, live).await);
                    }
                    Some(Control::RestoreAfterFailedRelaunch { cutoff, removed, reply }) => {
                        let _ = reply.send(state.restore_after_failed_relaunch(cutoff, removed).await);
                    }
                    Some(Control::SetWorkflow { workflow, reply }) => {
                        state.set_workflow(workflow);
                        let _ = reply.send(());
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    Some(Control::AttachmentCount { reply }) => {
                        let _ = reply.send(state.attachments.len());
                    }
                    None => break,
                }
            }
            event = ingress_rx.recv() => {
                match event {
                    Some(event) => state.handle_event(event).await,
                    None => break,
                }
            }
        }
    }
}

struct ActorState {
    session_id: String,
    workstream_id: String,
    db: Arc<Db>,
    events: broadcast::Sender<RuntimeEvent>,
    ingress_sender: mpsc::Sender<SessionEvent>,
    seq: i64,
    epoch: u64,
    ordinal: u64,
    next_attach_id: u64,
    attachments: HashMap<u64, LiveAttachment>,
    workflow: Option<WorkflowChannel>,
}

impl ActorState {
    async fn new(
        session_id: String,
        db: Arc<Db>,
        events: broadcast::Sender<RuntimeEvent>,
        ingress_sender: mpsc::Sender<SessionEvent>,
    ) -> Self {
        let sid = session_id.clone();
        let handle = db.clone();
        let (seq, workstream_id) = blocking(move || {
            let seq = handle.next_session_seq(&sid)?;
            let workstream_id = handle
                .get_session(&sid)?
                .map(|s| s.workstream_id)
                .unwrap_or_default();
            Ok((seq, workstream_id))
        })
        .await
        .unwrap_or((0, String::new()));
        Self {
            session_id,
            workstream_id,
            db,
            events,
            ingress_sender,
            seq,
            epoch: 0,
            ordinal: 0,
            next_attach_id: 0,
            attachments: HashMap::new(),
            workflow: None,
        }
    }

    async fn attach(&mut self, echo_user_messages: bool) -> Result<SessionAttachment> {
        let sid = self.session_id.clone();
        let db = self.db.clone();
        let replay = blocking(move || db.session_log(&sid)).await?;

        let (attachment, live) = LiveAttachment::new(echo_user_messages);
        let id = self.next_attach_id;
        self.next_attach_id += 1;
        self.attachments.insert(id, attachment);

        Ok(SessionAttachment { replay, live })
    }

    async fn log_after(
        &mut self,
        caller_epoch: u64,
        after_seq: Option<i64>,
    ) -> Result<Vec<SessionLogEntry>> {
        if caller_epoch != self.epoch {
            return Err(CoreError::SessionEpochAdvanced {
                session_id: self.session_id.clone(),
                epoch: self.epoch,
            });
        }
        let sid = self.session_id.clone();
        let db = self.db.clone();
        blocking(move || db.session_log_after(&sid, after_seq)).await
    }

    /// The catch-up read and the new attachment in one actor turn: as two
    /// messages, events persisted between them reached neither.
    async fn attach_after(
        &mut self,
        caller_epoch: u64,
        after_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let replay = self.log_after(caller_epoch, after_seq).await?;
        Ok(self.register_after(replay, after_seq, echo_user_messages))
    }

    /// A client's own cursor, not a caller epoch: the check is the row's
    /// `(seq, created_at)` still matching what the client folded, so a
    /// rewind that reused the seq is caught but a failed relaunch (which
    /// restores the original timestamp) is not a false positive.
    async fn attach_from(
        &mut self,
        cursor: LogCursor,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let after_seq = match cursor {
            LogCursor::Empty => None,
            LogCursor::After { seq, created_at } => {
                let (sid, db) = (self.session_id.clone(), self.db.clone());
                let held = blocking(move || db.session_event_created_at(&sid, seq)).await?;
                if held != Some(created_at) {
                    return Err(CoreError::SessionEpochAdvanced {
                        session_id: self.session_id.clone(),
                        epoch: self.epoch,
                    });
                }
                Some(seq)
            }
        };
        let (sid, db) = (self.session_id.clone(), self.db.clone());
        let replay = blocking(move || db.session_log_after(&sid, after_seq)).await?;
        Ok(self.register_after(replay, after_seq, echo_user_messages))
    }

    /// The tail `attach_after` and `attach_from` share: register a new
    /// attachment that resumes exactly where its replay ends.
    fn register_after(
        &mut self,
        replay: Vec<SessionLogEntry>,
        after_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> SessionAttachment {
        let resume_seq = replay.last().map(|entry| entry.seq).or(after_seq);
        let (attachment, live) = LiveAttachment::resuming_after(resume_seq, echo_user_messages);
        let id = self.next_attach_id;
        self.next_attach_id += 1;
        self.attachments.insert(id, attachment);
        SessionAttachment { replay, live }
    }

    fn set_workflow(&mut self, workflow: WorkflowChannel) {
        let pump =
            workflow_bridge::spawn_workflow_ingress_pump(&workflow, self.ingress_sender.clone());
        drop(pump);
        self.workflow = Some(workflow);
    }

    async fn rewind(
        &mut self,
        cutoff: i64,
        live: Option<(Arc<dyn AgentDriver>, usize)>,
    ) -> Result<RewindOutcome> {
        let mut live_rewound = false;
        if let Some((driver, ordinal)) = live {
            match driver.rewind_to_ordinal(ordinal).await {
                Ok(()) => live_rewound = true,
                Err(e) => {
                    tracing::info!(
                        "session {}: model-level rewind unavailable ({e}); truncating the app log only",
                        self.session_id
                    );
                }
            }
        }

        let sid = self.session_id.clone();
        let db = self.db.clone();
        let removed = blocking(move || db.cut_session_events_from(&sid, cutoff)).await?;

        if let Some(workflow) = &self.workflow {
            let workflows = workflow.workflows.clone();
            let _ = tokio::task::spawn_blocking(move || {
                workflows.blocking_lock().runs.retain_armed_below(cutoff);
            })
            .await;
        }

        self.epoch += 1;
        self.seq = cutoff;

        let boundary = SessionEvent::Rewound {};
        let kind = boundary.kind();
        if let Ok(payload) = encode_raw_event(&boundary) {
            let seq = self.seq;
            let mutation = row_mutation_for(&boundary);
            let sid = self.session_id.clone();
            let db = self.db.clone();
            let outcome =
                blocking(move || db.apply_session_event(&sid, seq, kind, &payload, mutation)).await;
            if let Ok(
                ApplySessionEventOutcome::Applied | ApplySessionEventOutcome::AlreadyApplied,
            ) = outcome
            {
                self.seq = seq + 1;
                self.ordinal += 1;
                let delivery = SessionDelivery {
                    epoch: self.epoch,
                    delivery_ordinal: self.ordinal,
                    persisted_seq: Some(seq),
                    event: boundary,
                };
                self.attachments.retain(|_, attachment| {
                    attachment.deliver(delivery.clone());
                    !attachment.detached()
                });
            } else if let Err(err) = outcome {
                tracing::warn!(
                    "session {}: failed to persist the rewind boundary at seq {seq}: {err}",
                    self.session_id
                );
            }
        }

        Ok(RewindOutcome {
            live_rewound,
            epoch: self.epoch,
            restart_seq: self.seq,
            removed,
        })
    }

    async fn restore_after_failed_relaunch(
        &mut self,
        cutoff: i64,
        removed: Vec<StoredEvent>,
    ) -> Result<()> {
        let sid = self.session_id.clone();
        let db = self.db.clone();
        let restore = removed.clone();
        blocking(move || {
            db.cut_session_events_from(&sid, cutoff)?;
            db.restore_session_events(&sid, &restore)?;
            db.set_session_state(&sid, SessionState::Idle)
        })
        .await?;
        self.seq = cutoff + removed.len() as i64;
        Ok(())
    }

    async fn handle_event(&mut self, event: SessionEvent) {
        self.ordinal += 1;
        let ordinal = self.ordinal;

        if let SessionEvent::ToolResult {
            tool_use_id,
            content,
            ..
        } = &event
        {
            if let Some(launch) = crate::claude::workflow::workflow_launch_from_result(content) {
                if let Some(workflow) = &self.workflow {
                    workflow_bridge::arm_workflow_run(
                        workflow,
                        launch,
                        tool_use_id.clone(),
                        self.seq,
                        Default::default(),
                        Default::default(),
                    )
                    .await;
                }
            }
        }

        let unreportable_exit =
            matches!(&event, SessionEvent::Exited { outcome, .. } if !outcome.is_reportable());
        // Modes are live state: a replayed one would be stale, and an older app
        // cannot decode the row.
        let ephemeral = matches!(
            event,
            SessionEvent::PartialText { .. }
                | SessionEvent::PermissionModeChanged { .. }
                | SessionEvent::PlanModeChanged { .. }
        ) || unreportable_exit;
        let suppress_live = matches!(event, SessionEvent::UserMessage { .. });
        log_lifecycle(&self.session_id, &event);

        // Nothing is persisted for a detach, so the row mutation that clears
        // the pid never runs: the next start would report a live orphan.
        if unreportable_exit {
            let db = self.db.clone();
            let sid = self.session_id.clone();
            if let Err(e) = blocking(move || db.clear_orphans_for(&sid)).await {
                tracing::warn!(
                    "session {}: could not clear its orphan record: {e}",
                    self.session_id
                );
            }
        }

        let persisted_seq = if ephemeral {
            None
        } else {
            let seq = self.seq;
            let kind = event.kind();
            let payload = match encode_raw_event(&event) {
                Ok(payload) => payload,
                Err(err) => {
                    tracing::warn!(
                        "session {}: failed to encode {kind} event at seq {seq}: {err}",
                        self.session_id
                    );
                    return;
                }
            };
            let mutation = row_mutation_for(&event);
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let outcome = blocking(move || {
                db.apply_session_event(&session_id, seq, kind, &payload, mutation)
            })
            .await;
            match outcome {
                Ok(
                    ApplySessionEventOutcome::Applied | ApplySessionEventOutcome::AlreadyApplied,
                ) => {
                    self.seq = seq + 1;
                    Some(seq)
                }
                Err(err) => {
                    tracing::warn!(
                        "session {}: failed to persist {kind} event at seq {seq}: {err}",
                        self.session_id
                    );
                    return;
                }
            }
        };

        if let Some(state) = lifecycle_for(&event) {
            let _ = self.events.send(RuntimeEvent::SessionLifecycle {
                session_id: self.session_id.clone(),
                workstream_id: self.workstream_id.clone(),
                state,
            });
        }

        let delivery = SessionDelivery {
            epoch: self.epoch,
            delivery_ordinal: ordinal,
            persisted_seq,
            event,
        };
        tracing::trace!(
            session_id = %self.session_id,
            ordinal = delivery.delivery_ordinal,
            persisted_seq = ?delivery.persisted_seq,
            "fanning out session delivery"
        );
        self.attachments.retain(|_, attachment| {
            // Released clients render their own prompt optimistically and never dedupe an echo.
            if !suppress_live || attachment.echo_user_messages {
                attachment.deliver(delivery.clone());
            }
            !attachment.detached()
        });
    }
}

fn lifecycle_for(event: &SessionEvent) -> Option<SessionState> {
    match event {
        SessionEvent::Init { .. } => Some(SessionState::Running),
        SessionEvent::PermissionAsk { .. } => Some(SessionState::AwaitingPermission),
        SessionEvent::QuestionAsk { .. } => Some(SessionState::AwaitingQuestion),
        SessionEvent::Exited { outcome, .. } if outcome.is_reportable() => Some(outcome.state()),
        _ => None,
    }
}

/// The session timeline a support log needs; payloads stay out of it.
fn log_lifecycle(session_id: &str, event: &SessionEvent) {
    match event {
        SessionEvent::Exited {
            outcome,
            code,
            stderr_tail,
        } => {
            let stderr =
                crate::redact::url_credentials(crate::redact::tail(stderr_tail.trim(), 500));
            match outcome {
                ExitOutcome::Crashed | ExitOutcome::SpawnFailed => tracing::warn!(
                    "session {session_id} exited: {outcome:?}, code {code:?}: {stderr}"
                ),
                _ => tracing::info!("session {session_id} exited: {outcome:?}, code {code:?}"),
            }
        }
        SessionEvent::Relaunched {} => {
            tracing::info!("session {session_id}: harness relaunched")
        }
        SessionEvent::ProtocolError { detail, .. } => {
            tracing::debug!("session {session_id}: unrecognised harness output: {detail}")
        }
        _ => {}
    }
}

fn row_mutation_for(event: &SessionEvent) -> SessionRowMutation {
    match event {
        SessionEvent::Init {
            provider_session_id,
            model,
        } => SessionRowMutation {
            provider_session_id: FieldOp::Set(provider_session_id.clone()),
            model: model.clone().map(FieldOp::Set).unwrap_or(FieldOp::Clear),
            state: FieldOp::Set(SessionState::Running),
            ..Default::default()
        },
        SessionEvent::PermissionAsk { .. } => SessionRowMutation {
            state: FieldOp::Set(SessionState::AwaitingPermission),
            ..Default::default()
        },
        SessionEvent::QuestionAsk { .. } => SessionRowMutation {
            state: FieldOp::Set(SessionState::AwaitingQuestion),
            ..Default::default()
        },
        SessionEvent::Rewound {} => SessionRowMutation {
            state: FieldOp::Set(SessionState::Running),
            ..Default::default()
        },
        SessionEvent::TurnDone { cost_usd, .. } => SessionRowMutation {
            cost_usd: cost_usd.map(FieldOp::Set).unwrap_or(FieldOp::Unchanged),
            state: FieldOp::Set(SessionState::Running),
            ..Default::default()
        },
        SessionEvent::Exited { outcome, .. } if !outcome.is_reportable() => SessionRowMutation {
            clear_orphans: true,
            ..Default::default()
        },
        SessionEvent::Exited {
            outcome,
            stderr_tail,
            ..
        } => {
            let summary = (!stderr_tail.is_empty()).then(|| stderr_tail.clone());
            SessionRowMutation {
                state: FieldOp::Set(outcome.state()),
                ended_at: FieldOp::Set(crate::store::now_ms()),
                exit_summary: summary.map(FieldOp::Set).unwrap_or(FieldOp::Clear),
                clear_orphans: true,
                ..Default::default()
            }
        }
        _ => SessionRowMutation::default(),
    }
}
