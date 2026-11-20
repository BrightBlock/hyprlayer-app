use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use hyprlayer_models::{
    ClientRef, GoalStatus, RevisedTurn, RuntimeEvent, SessionOrigin, SessionTurns, UiRequest,
};

use crate::agent::AgentDriver;
use crate::claude::{ExitOutcome, SessionEvent, SessionState};
use crate::error::{CoreError, Result};
use crate::runtime::attribution::{self, DocAttribution};
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
    KeepStreamsAcrossExit {
        reply: oneshot::Sender<()>,
    },
    SettleOnDetach {
        reply: oneshot::Sender<()>,
    },
    EndKeptStreams {
        reply: oneshot::Sender<()>,
    },
    ReserveUserTurn {
        client_message_id: String,
        base_rev: Option<u64>,
        author: Option<ClientRef>,
        reply: oneshot::Sender<Result<()>>,
    },
    ReleaseUserTurn {
        client_message_id: String,
    },
    ListTurns {
        cursor: LogCursor,
        limit: u32,
        caller: Option<String>,
        reply: oneshot::Sender<Result<SessionTurns>>,
    },
    RevState {
        caller: Option<String>,
        reply: oneshot::Sender<ActorRevState>,
    },
    #[cfg(any(test, feature = "test-support"))]
    AttachmentCount {
        reply: oneshot::Sender<usize>,
    },
}

/// A live actor's revision state as `caller` sees it; "working" is its `in_turn` (R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorRevState {
    pub rev: u64,
    pub reset_rev: u64,
    pub working: bool,
    pub in_flight: u32,
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

    pub(crate) async fn keep_streams_across_exit(&self) -> Result<()> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::KeepStreamsAcrossExit { reply })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())
    }

    pub(crate) async fn settle_on_detach(&self) -> Result<()> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::SettleOnDetach { reply })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())
    }

    pub(crate) async fn end_kept_streams(&self) -> Result<()> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::EndKeptStreams { reply })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())
    }

    pub(crate) async fn reserve_user_turn(
        &self,
        client_message_id: String,
        base_rev: Option<u64>,
        author: Option<ClientRef>,
    ) -> Result<()> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::ReserveUserTurn {
                client_message_id,
                base_rev,
                author,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    /// Fire-and-forget: a release is advisory, and the caller has no state
    /// worth blocking on (R5 — the TTL is the backstop if this is lost).
    pub(crate) async fn release_user_turn(&self, client_message_id: String) {
        let _ = self
            .control
            .send(Control::ReleaseUserTurn { client_message_id });
    }

    pub(crate) async fn list_turns(
        &self,
        cursor: LogCursor,
        limit: u32,
        caller: Option<String>,
    ) -> Result<SessionTurns> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::ListTurns {
                cursor,
                limit,
                caller,
                reply,
            })
            .map_err(|_| actor_gone())?;
        reply_rx.await.map_err(|_| actor_gone())?
    }

    pub(crate) async fn rev_state(&self, caller: Option<String>) -> Result<ActorRevState> {
        let (reply, reply_rx) = oneshot::channel();
        self.control
            .send(Control::RevState { caller, reply })
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
    attribution: Option<Arc<DocAttribution>>,
) -> ActorHandle {
    let (ingress_tx, ingress_rx) = mpsc::channel(INGRESS_CAPACITY);
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let join = tokio::spawn(run(
        session_id,
        db,
        events,
        attribution,
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
    attribution: Option<Arc<DocAttribution>>,
    ingress_sender: mpsc::Sender<SessionEvent>,
    mut ingress_rx: mpsc::Receiver<SessionEvent>,
    mut control_rx: mpsc::UnboundedReceiver<Control>,
) {
    let mut state = ActorState::new(session_id, db, events, attribution, ingress_sender).await;
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
                    Some(Control::KeepStreamsAcrossExit { reply }) => {
                        state.keep_streams = true;
                        let _ = reply.send(());
                    }
                    Some(Control::SettleOnDetach { reply }) => {
                        state.settle_on_detach = true;
                        let _ = reply.send(());
                    }
                    Some(Control::EndKeptStreams { reply }) => {
                        state.end_kept_streams().await;
                        let _ = reply.send(());
                    }
                    Some(Control::ReserveUserTurn { client_message_id, base_rev, author, reply }) => {
                        let _ = reply.send(state.reserve(client_message_id, base_rev, author));
                    }
                    Some(Control::ReleaseUserTurn { client_message_id }) => {
                        state.release_reservation(&client_message_id);
                    }
                    Some(Control::ListTurns { cursor, limit, caller, reply }) => {
                        let _ = reply.send(state.list_turns(cursor, limit, caller).await);
                    }
                    Some(Control::RevState { caller, reply }) => {
                        let _ = reply.send(state.rev_state(caller.as_deref()));
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
            _ = tokio::time::sleep_until(state.next_expiry()), if !state.reservations.is_empty() => {
                state.expire_reservations();
            }
        }
    }
}

struct ActorState {
    session_id: String,
    workstream_id: String,
    /// An agent spawned it: its asks are announced server-side, since its tab may never have attached.
    agent_spawned: bool,
    db: Arc<Db>,
    events: broadcast::Sender<RuntimeEvent>,
    ingress_sender: mpsc::Sender<SessionEvent>,
    seq: i64,
    epoch: u64,
    ordinal: u64,
    next_attach_id: u64,
    attachments: HashMap<u64, LiveAttachment>,
    workflow: Option<WorkflowChannel>,
    rev: u64,
    reset_rev: u64,
    /// The rev every unattributed commit counts as; raised by a reset (R4).
    foreign_floor: u64,
    rev_by_author: HashMap<String, u64>,
    in_turn: bool,
    turn_author: Option<ClientRef>,
    /// A message that arrived mid-turn, and its author; opens the next turn on its first item.
    next_turn: Option<Option<ClientRef>>,
    /// Rows visible since the last announced revision; flushed onto it.
    riders: Vec<RevisedTurn>,
    reservations: HashMap<String, Reservation>,
    /// Authors of reservations that lapsed unechoed: a steer can wait behind a long tool.
    lapsed: VecDeque<(String, Option<ClientRef>)>,
    attribution: Option<Arc<DocAttribution>>,
    /// Write tool id → its paths, until the result says whether it landed.
    pending_writes: HashMap<String, Vec<String>>,
    /// The main-thread text block streamed so far and not yet stored.
    in_flight: InFlightText,
    /// The next detach is a server relaunch: its exit would end every client's stream.
    keep_streams: bool,
    /// A detach was kept from the streams, which now wait on the relaunch.
    streams_kept: bool,
    /// The next detach relaunches nothing, so its row must stop claiming a live harness.
    settle_on_detach: bool,
}

#[derive(Default)]
struct InFlightText {
    text: String,
    author: Option<ClientRef>,
}

impl ActorState {
    async fn new(
        session_id: String,
        db: Arc<Db>,
        events: broadcast::Sender<RuntimeEvent>,
        attribution: Option<Arc<DocAttribution>>,
        ingress_sender: mpsc::Sender<SessionEvent>,
    ) -> Self {
        let sid = session_id.clone();
        let handle = db.clone();
        let (seq, workstream_id, agent_spawned, rev, reset_rev) = blocking(move || {
            let seq = handle.next_session_seq(&sid)?;
            let (workstream_id, agent_spawned) = handle
                .get_session(&sid)?
                .map(|s| (s.workstream_id, s.origin == SessionOrigin::Agent))
                .unwrap_or_default();
            let (rev, reset_rev) = handle.session_rev(&sid)?;
            Ok((seq, workstream_id, agent_spawned, rev, reset_rev))
        })
        .await
        .unwrap_or((0, String::new(), false, 0, 0));
        Self {
            session_id,
            workstream_id,
            agent_spawned,
            db,
            events,
            ingress_sender,
            seq,
            epoch: 0,
            ordinal: 0,
            next_attach_id: 0,
            attachments: HashMap::new(),
            workflow: None,
            rev,
            reset_rev,
            foreign_floor: rev,
            rev_by_author: HashMap::new(),
            in_turn: false,
            turn_author: None,
            next_turn: None,
            riders: Vec::new(),
            reservations: HashMap::new(),
            lapsed: VecDeque::new(),
            attribution,
            pending_writes: HashMap::new(),
            in_flight: InFlightText::default(),
            keep_streams: false,
            streams_kept: false,
            settle_on_detach: false,
        }
    }

    async fn attach(&mut self, echo_user_messages: bool) -> Result<SessionAttachment> {
        let sid = self.session_id.clone();
        let db = self.db.clone();
        let replay = blocking(move || db.session_log(&sid)).await?;

        let (mut attachment, live) = LiveAttachment::new(echo_user_messages);
        self.seed_in_flight(&mut attachment);
        let id = self.next_attach_id;
        self.next_attach_id += 1;
        self.attachments.insert(id, attachment);

        Ok(SessionAttachment {
            replay,
            live,
            rev: self.rev,
        })
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
        // The lagged reader still holds its open block; a seed would repeat it.
        Ok(self.register_after(replay, after_seq, echo_user_messages, false))
    }

    /// A client's own cursor, not a caller epoch: the check is the row's
    /// `(seq, created_at)` still matching what the client folded, so a
    /// rewind that reused the seq is caught but a failed relaunch (which
    /// restores the original timestamp) is not a false positive.
    async fn verify_cursor(&self, cursor: LogCursor) -> Result<Option<i64>> {
        match cursor {
            LogCursor::Empty => Ok(None),
            LogCursor::After { seq, created_at } => {
                let (sid, db) = (self.session_id.clone(), self.db.clone());
                let held = blocking(move || db.session_event_created_at(&sid, seq)).await?;
                if held != Some(created_at) {
                    return Err(CoreError::SessionEpochAdvanced {
                        session_id: self.session_id.clone(),
                        epoch: self.epoch,
                    });
                }
                Ok(Some(seq))
            }
        }
    }

    async fn attach_from(
        &mut self,
        cursor: LogCursor,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let after_seq = self.verify_cursor(cursor).await?;
        let (sid, db) = (self.session_id.clone(), self.db.clone());
        let replay = blocking(move || db.session_log_after(&sid, after_seq)).await?;
        Ok(self.register_after(replay, after_seq, echo_user_messages, true))
    }

    /// The tail `attach_after` and `attach_from` share: register a new
    /// attachment that resumes exactly where its replay ends.
    fn register_after(
        &mut self,
        replay: Vec<SessionLogEntry>,
        after_seq: Option<i64>,
        echo_user_messages: bool,
        seed: bool,
    ) -> SessionAttachment {
        let resume_seq = replay.last().map(|entry| entry.seq).or(after_seq);
        let (mut attachment, live) = LiveAttachment::resuming_after(resume_seq, echo_user_messages);
        if seed {
            self.seed_in_flight(&mut attachment);
        }
        let id = self.next_attach_id;
        self.next_attach_id += 1;
        self.attachments.insert(id, attachment);
        SessionAttachment {
            replay,
            live,
            rev: self.rev,
        }
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
            match outcome {
                Ok(applied) => {
                    self.seq = seq + 1;
                    self.ordinal += 1;
                    let created_at = applied.created_at();
                    let rev = match applied {
                        ApplySessionEventOutcome::Applied { rev: Some(rev), .. } => rev,
                        // Already applied, or the session row was missing: bump directly.
                        _ => self.bump_reset_rev().await,
                    };
                    let delivery = SessionDelivery {
                        epoch: self.epoch,
                        delivery_ordinal: self.ordinal,
                        persisted_seq: Some(seq),
                        created_at: Some(created_at),
                        rev,
                        author: None,
                        event: boundary,
                    };
                    self.attachments.retain(|_, attachment| {
                        attachment.deliver(delivery.clone());
                        !attachment.detached()
                    });
                    self.reset(rev);
                }
                Err(err) => {
                    tracing::warn!(
                        "session {}: failed to persist the rewind boundary at seq {seq}: {err}",
                        self.session_id
                    );
                    self.bump_and_reset().await;
                }
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
        self.bump_and_reset().await;
        Ok(())
    }

    async fn list_turns(
        &mut self,
        cursor: LogCursor,
        limit: u32,
        caller: Option<String>,
    ) -> Result<SessionTurns> {
        let after_seq = self.verify_cursor(cursor).await?;
        let (sid, db) = (self.session_id.clone(), self.db.clone());
        let limit = limit.clamp(1, TURNS_PAGE_MAX);
        let page =
            blocking(move || db.session_log_page(&sid, after_seq, limit, TURNS_PAGE_MAX_BYTES))
                .await?;
        Ok(SessionTurns {
            entries: page.entries,
            next_cursor: page
                .last
                .map_or(cursor, |(seq, created_at)| LogCursor::After {
                    seq,
                    created_at,
                }),
            remaining: page.remaining,
            rev: self.rev,
            reset_rev: self.reset_rev,
            working: self.in_turn,
            in_flight: self.foreign_in_flight(caller.as_deref()) as u32,
        })
    }

    /// Bumps `sessions.rev`/`reset_rev` directly when the boundary row itself
    /// didn't, so a reset still reaches watchers.
    async fn bump_and_reset(&mut self) {
        let rev = self.bump_reset_rev().await;
        self.reset(rev);
    }

    async fn bump_reset_rev(&self) -> u64 {
        let sid = self.session_id.clone();
        let db = self.db.clone();
        match blocking(move || db.bump_session_reset(&sid)).await {
            Ok(rev) => rev,
            Err(err) => {
                tracing::warn!(
                    "session {}: failed to bump rev for a reset: {err}",
                    self.session_id
                );
                self.rev
            }
        }
    }

    fn rev_state(&self, caller: Option<&str>) -> ActorRevState {
        ActorRevState {
            rev: self.rev,
            reset_rev: self.reset_rev,
            working: self.in_turn,
            in_flight: self.foreign_in_flight(caller) as u32,
        }
    }

    /// The relaunch failed: a kept exit is delivered after all, or a pending one is not kept.
    async fn end_kept_streams(&mut self) {
        if std::mem::take(&mut self.keep_streams) || !std::mem::take(&mut self.streams_kept) {
            return;
        }
        self.handle_event(SessionEvent::Exited {
            outcome: ExitOutcome::Detached,
            code: None,
            stderr_tail: String::new(),
        })
        .await;
    }

    async fn handle_event(&mut self, mut event: SessionEvent) {
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
        self.track_writes(&event);

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
        let exited = matches!(event, SessionEvent::Exited { .. });
        let kept_streams = exited && std::mem::take(&mut self.keep_streams) && unreportable_exit;
        self.streams_kept = kept_streams;
        let settle = exited && std::mem::take(&mut self.settle_on_detach) && unreportable_exit;
        log_lifecycle(&self.session_id, &event);

        // Nothing is persisted for a detach, so the row mutation that clears
        // the pid never runs: the next start would report a live orphan.
        if unreportable_exit {
            let db = self.db.clone();
            let sid = self.session_id.clone();
            let written = blocking(move || {
                db.clear_orphans_for(&sid)?;
                if settle {
                    db.settle_session(&sid)?;
                }
                Ok(())
            })
            .await;
            if let Err(e) = written {
                tracing::warn!(
                    "session {}: its detach was not recorded: {e}",
                    self.session_id
                );
            }
        }

        let author = self.author_for(&event);
        if let SessionEvent::UserMessage {
            author: stamped, ..
        } = &mut event
        {
            *stamped = author.clone();
        }
        let turn_kind = event.turn_kind();
        let closes_turn = exited || matches!(event, SessionEvent::TurnDone { .. });
        let announce_close =
            closes_turn && (self.in_turn || exited && !self.reservations.is_empty());
        if closes_turn {
            // Ahead of the persist, whose early return below must not leave the turn open.
            self.close_turn(exited);
        }

        let (persisted_seq, created_at, committed_rev) = if ephemeral {
            (None, None, None)
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
                Ok(ApplySessionEventOutcome::Applied { rev, created_at }) => {
                    self.seq = seq + 1;
                    (Some(seq), Some(created_at), rev)
                }
                Ok(ApplySessionEventOutcome::AlreadyApplied { created_at }) => {
                    self.seq = seq + 1;
                    (Some(seq), Some(created_at), None)
                }
                Err(err) => {
                    tracing::warn!(
                        "session {}: failed to persist {kind} event at seq {seq}: {err}",
                        self.session_id
                    );
                    if announce_close {
                        self.announce_revision(self.current_author());
                    }
                    return;
                }
            }
        };

        if let (Some(seq), Some(kind)) = (persisted_seq, turn_kind) {
            self.riders.push(RevisedTurn { seq, kind });
        }
        match committed_rev {
            Some(rev) => {
                self.record_commit(rev, &author);
                self.announce_revision(author.clone());
            }
            None if announce_close => {
                self.announce_revision(self.current_author());
            }
            None => {}
        }

        if let Some(state) = lifecycle_for(&event) {
            let _ = self.events.send(RuntimeEvent::SessionLifecycle {
                session_id: self.session_id.clone(),
                workstream_id: self.workstream_id.clone(),
                state,
            });
        }
        if self.agent_spawned {
            self.announce_ask(&event);
        }

        self.track_in_flight(&event, &author);
        let delivery = SessionDelivery {
            epoch: self.epoch,
            delivery_ordinal: ordinal,
            persisted_seq,
            created_at,
            rev: self.rev,
            author,
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
            if (!suppress_live || attachment.echo_user_messages) && !kept_streams {
                attachment.deliver(delivery.clone());
            }
            !attachment.detached()
        });
    }

    /// Who a delivery is attributed to; a UserMessage also settles its reservation and joins a turn.
    fn author_for(&mut self, event: &SessionEvent) -> Option<ClientRef> {
        let SessionEvent::UserMessage {
            client_message_id, ..
        } = event
        else {
            // A parked message's turn opens with its first item, if the harness didn't merge it.
            if !self.in_turn && event.turn_kind().is_some() {
                if let Some(parked) = self.next_turn.take() {
                    self.in_turn = true;
                    self.turn_author = parked;
                }
            }
            return self.current_author();
        };
        // Follow-ups are reserved like sends; an unmatched message is author-less, nonce or not.
        let author = client_message_id
            .as_ref()
            .and_then(|id| {
                let reserved = self.reservations.remove(id).map(|r| r.author);
                reserved.or_else(|| self.take_lapsed(id))
            })
            .flatten();
        if self.in_turn {
            // The running turn keeps its author; the harness may answer this after it or with it.
            self.next_turn = Some(author.clone());
        } else {
            self.in_turn = true;
            self.turn_author = author.clone();
            self.next_turn = None;
        }
        author
    }

    /// Recording runs in its own task: it reads config and disk, which the actor never waits on.
    fn track_writes(&mut self, event: &SessionEvent) {
        let Some(attribution) = &self.attribution else {
            return;
        };
        match event {
            SessionEvent::ToolUse {
                id, name, input, ..
            } => {
                let paths = attribution::write_paths(name, input);
                if !paths.is_empty() {
                    self.pending_writes.insert(id.clone(), paths);
                }
            }
            SessionEvent::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                if let Some(paths) = self.pending_writes.remove(tool_use_id) {
                    if !is_error {
                        tokio::spawn(attribution.clone().record(self.session_id.clone(), paths));
                    }
                }
            }
            SessionEvent::Exited { .. } => self.pending_writes.clear(),
            _ => {}
        }
    }

    fn current_author(&self) -> Option<ClientRef> {
        self.turn_author.clone().filter(|_| self.in_turn)
    }

    /// Always closes: a parked mid-turn author waits for its turn's first item.
    fn close_turn(&mut self, exited: bool) {
        self.in_turn = false;
        self.turn_author = None;
        if exited {
            self.next_turn = None;
            self.reservations.clear();
            self.lapsed.clear();
        }
    }

    fn record_commit(&mut self, rev: u64, author: &Option<ClientRef>) {
        self.rev = rev;
        match client_id(author) {
            Some(id) => {
                self.rev_by_author.insert(id.to_string(), rev);
            }
            // An unattributed turn is foreign to everyone.
            None => self.foreign_floor = rev,
        }
    }

    fn announce_revision(&mut self, author: Option<ClientRef>) {
        let _ = self.events.send(RuntimeEvent::SessionRevised {
            session_id: self.session_id.clone(),
            workstream_id: self.workstream_id.clone(),
            rev: self.rev,
            turns: std::mem::take(&mut self.riders),
            author,
            working: self.in_turn,
            at: crate::store::now_ms(),
        });
    }

    fn track_in_flight(&mut self, event: &SessionEvent, author: &Option<ClientRef>) {
        match event {
            SessionEvent::PartialText {
                text,
                parent_id: None,
            } => {
                self.in_flight.text.push_str(text);
                self.in_flight.author = author.clone();
            }
            SessionEvent::AssistantText {
                parent_id: None, ..
            }
            | SessionEvent::TurnDone { .. }
            | SessionEvent::Exited { .. } => self.in_flight = InFlightText::default(),
            _ => {}
        }
    }

    /// Partial text is never stored, so a reader attaching mid-block would otherwise start mid-sentence.
    fn seed_in_flight(&self, attachment: &mut LiveAttachment) {
        if self.in_flight.text.is_empty() {
            return;
        }
        attachment.deliver(SessionDelivery {
            epoch: self.epoch,
            delivery_ordinal: self.ordinal,
            persisted_seq: None,
            created_at: None,
            rev: self.rev,
            author: self.in_flight.author.clone(),
            event: SessionEvent::PartialText {
                text: self.in_flight.text.clone(),
                parent_id: None,
            },
        });
    }

    /// A rewind or restore: every reader reloads, so no earlier commit is anyone's own.
    fn reset(&mut self, rev: u64) {
        self.in_flight = InFlightText::default();
        self.rev = rev;
        self.reset_rev = rev;
        self.foreign_floor = rev;
        self.rev_by_author.clear();
        self.riders.clear();
        self.in_turn = false;
        self.turn_author = None;
        self.next_turn = None;
        let _ = self.events.send(RuntimeEvent::SessionReset {
            session_id: self.session_id.clone(),
            workstream_id: self.workstream_id.clone(),
            rev,
        });
    }
}

/// A send accepted but not yet committed; other authors' sends are checked against it.
struct Reservation {
    author: Option<ClientRef>,
    at: tokio::time::Instant,
}

// A UserMessage the relay dropped on lag never releases its slot.
const RESERVATION_TTL: Duration = Duration::from_secs(30);
const LAPSED_KEPT: usize = 32;

const TURNS_PAGE_MAX: u32 = 2000;
// About a second of a 256 KiB/s link, so a call queued behind a page waits no longer.
const TURNS_PAGE_MAX_BYTES: usize = 256 * 1024;

fn client_id(author: &Option<ClientRef>) -> Option<&str> {
    author.as_ref().map(|a| a.client_id.as_str())
}

impl ActorState {
    /// Other authors' sends in flight as `me` sees them; no identity counts them all.
    fn foreign_in_flight(&self, me: Option<&str>) -> u64 {
        self.reservations
            .values()
            .filter(|r| me.is_none() || client_id(&r.author) != me)
            .count() as u64
    }

    /// The rev `me` must have applied for its send to answer the latest state.
    fn foreign_rev(&self, me: Option<&str>) -> u64 {
        let in_flight = self.foreign_in_flight(me);
        if in_flight > 0 {
            // Their turns land after everything committed so far, `me`'s included.
            return self.rev + in_flight;
        }
        self.rev_by_author
            .iter()
            .filter(|(id, _)| me != Some(id.as_str()))
            .map(|(_, rev)| *rev)
            .fold(self.foreign_floor, u64::max)
    }

    fn reserve(
        &mut self,
        id: String,
        base_rev: Option<u64>,
        author: Option<ClientRef>,
    ) -> Result<()> {
        let foreign = self.foreign_rev(client_id(&author));
        if let Some(base_rev) = base_rev.filter(|base| *base < foreign) {
            return Err(CoreError::StaleTranscript {
                session_id: self.session_id.clone(),
                rev: foreign,
                committed_rev: self.rev,
                missing_turns: foreign - base_rev,
            });
        }
        self.reservations.insert(
            id,
            Reservation {
                author,
                at: tokio::time::Instant::now(),
            },
        );
        Ok(())
    }

    fn next_expiry(&self) -> tokio::time::Instant {
        let oldest = self.reservations.values().map(|r| r.at).min();
        oldest.map_or_else(tokio::time::Instant::now, |at| at + RESERVATION_TTL)
    }

    fn expire_reservations(&mut self) {
        let before = self.reservations.len();
        let expired: Vec<String> = self
            .reservations
            .iter()
            .filter(|(_, r)| r.at.elapsed() >= RESERVATION_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(reserved) = self.reservations.remove(&id) {
                self.lapsed.push_back((id, reserved.author));
            }
        }
        while self.lapsed.len() > LAPSED_KEPT {
            self.lapsed.pop_front();
        }
        if self.reservations.len() < before {
            // Same rev, no turns: readers drop the in-flight count it carried.
            self.announce_revision(self.current_author());
        }
    }

    fn take_lapsed(&mut self, id: &str) -> Option<Option<ClientRef>> {
        let at = self.lapsed.iter().position(|(lapsed, _)| lapsed == id)?;
        self.lapsed.remove(at).map(|(_, author)| author)
    }

    /// The `ReleaseUserTurn` arm: releasing an unknown nonce is a silent no-op.
    fn release_reservation(&mut self, id: &str) {
        if self.reservations.remove(id).is_some() {
            self.announce_revision(self.current_author());
        }
    }
}

impl ActorState {
    /// Tells every announced desktop an agent-spawned session is asking; a desktop streaming it skips this.
    fn announce_ask(&self, event: &SessionEvent) {
        let body = match event {
            SessionEvent::PermissionAsk { tool_name, .. } => tool_name.clone(),
            SessionEvent::QuestionAsk { questions, .. } => questions
                .first()
                .map(|q| {
                    if q.header.is_empty() {
                        q.question.clone()
                    } else {
                        q.header.clone()
                    }
                })
                .unwrap_or_default(),
            _ => return,
        };
        let (db, events) = (self.db.clone(), self.events.clone());
        let (session_id, workstream_id) = (self.session_id.clone(), self.workstream_id.clone());
        // Off the actor: a row read must not hold up the event stream.
        tokio::spawn(async move {
            let id = session_id.clone();
            let row = match blocking(move || db.get_session(&id)).await {
                Ok(Some(row)) => row,
                Ok(None) => return,
                Err(e) => {
                    tracing::warn!("session {session_id}: its ask was not announced: {e}");
                    return;
                }
            };
            // A running goal's asks are announced by the goal itself.
            if row.goal.is_some_and(|g| g.status == GoalStatus::Running) {
                return;
            }
            let name = row
                .title
                .unwrap_or_else(|| format!("{} session", row.stage.as_str()));
            let _ = events.send(RuntimeEvent::UiRequested {
                request: UiRequest::Notify {
                    workstream_id,
                    title: format!("{name} needs you"),
                    body,
                    session_id: Some(session_id),
                    terminal_id: None,
                },
            });
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
        SessionEvent::Relaunched { .. } => {
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
            ..
        } => SessionRowMutation {
            provider_session_id: FieldOp::Set(provider_session_id.clone()),
            model: model.clone().map(FieldOp::Set).unwrap_or(FieldOp::Clear),
            state: FieldOp::Set(SessionState::Running),
            // A resumed harness is live again, so an earlier end no longer holds.
            ended_at: FieldOp::Clear,
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
