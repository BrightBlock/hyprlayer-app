pub mod delivery;
mod followup;
mod forwarder;
pub(crate) mod goal_loop;
mod recap;
pub mod registry;
mod service;
pub(crate) mod titles;
mod workflow_bridge;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

use hyprlayer_models::{ClientRef, RuntimeEvent, SessionTurns};

use crate::agent::AgentDriver;
use crate::claude::SessionEvent;
use crate::error::Result;
use crate::runtime::attribution::DocAttribution;
use crate::store::{Db, LogCursor, SessionLogEntry};

pub use delivery::{LiveReceiver, SessionAttachment, SessionStreamError};
pub use forwarder::ActorRevState;
pub use service::{
    ActiveSession, LaunchAndConverseParams, LaunchSessionParams, RewindSessionOutcome,
    RewindSessionRequest, SendContext, SessionLineage, SessionService, StartStageSessionRequest,
    StartedStageSession,
};

#[cfg(any(test, feature = "test-support"))]
pub use followup::run_codex_followup_for_test;
#[cfg(any(test, feature = "test-support"))]
pub use goal_loop::{interrupt_then_kill, watch_iteration, Escalation, IterationEnd, Watch};

pub(crate) struct SessionActors {
    db: Arc<Db>,
    events: broadcast::Sender<RuntimeEvent>,
    attribution: Option<Arc<DocAttribution>>,
    handles: AsyncMutex<HashMap<String, forwarder::ActorHandle>>,
}

impl SessionActors {
    pub(crate) fn new(
        db: Arc<Db>,
        events: broadcast::Sender<RuntimeEvent>,
        attribution: Option<Arc<DocAttribution>>,
    ) -> Self {
        Self {
            db,
            events,
            attribution,
            handles: AsyncMutex::new(HashMap::new()),
        }
    }

    fn spawn_actor(&self, session_id: &str) -> forwarder::ActorHandle {
        forwarder::spawn(
            session_id.to_string(),
            self.db.clone(),
            self.events.clone(),
            self.attribution.clone(),
        )
    }

    pub(crate) async fn ingress(&self, session_id: &str) -> mpsc::Sender<SessionEvent> {
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.get(session_id) {
            return handle.ingress.clone();
        }
        let handle = self.spawn_actor(session_id);
        let sender = handle.ingress.clone();
        handles.insert(session_id.to_string(), handle);
        sender
    }

    pub(crate) async fn attach(
        &self,
        session_id: &str,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.attach(echo_user_messages).await
    }

    pub(crate) async fn log_after(
        &self,
        session_id: &str,
        caller_epoch: u64,
        after_seq: Option<i64>,
    ) -> Result<Vec<SessionLogEntry>> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.log_after(caller_epoch, after_seq).await
    }

    pub(crate) async fn attach_after(
        &self,
        session_id: &str,
        caller_epoch: u64,
        after_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle
            .attach_after(caller_epoch, after_seq, echo_user_messages)
            .await
    }

    pub(crate) async fn attach_from(
        &self,
        session_id: &str,
        cursor: LogCursor,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.attach_from(cursor, echo_user_messages).await
    }

    pub(crate) async fn rewind(
        &self,
        session_id: &str,
        cutoff: i64,
        live: Option<(Arc<dyn AgentDriver>, usize)>,
    ) -> Result<forwarder::RewindOutcome> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.rewind(cutoff, live).await
    }

    pub(crate) async fn restore_after_failed_relaunch(
        &self,
        session_id: &str,
        cutoff: i64,
        removed: Vec<crate::store::StoredEvent>,
    ) -> Result<()> {
        let handles = self.handles.lock().await;
        match handles.get(session_id) {
            Some(handle) => handle.restore_after_failed_relaunch(cutoff, removed).await,
            None => Ok(()),
        }
    }

    pub(crate) async fn keep_streams_across_exit(&self, session_id: &str) -> Result<()> {
        match self.handles.lock().await.get(session_id) {
            Some(handle) => handle.keep_streams_across_exit().await,
            None => Ok(()),
        }
    }

    pub(crate) async fn settle_on_detach(&self, session_id: &str) -> Result<()> {
        match self.handles.lock().await.get(session_id) {
            Some(handle) => handle.settle_on_detach().await,
            None => Ok(()),
        }
    }

    pub(crate) async fn end_kept_streams(&self, session_id: &str) -> Result<()> {
        match self.handles.lock().await.get(session_id) {
            Some(handle) => handle.end_kept_streams().await,
            None => Ok(()),
        }
    }

    pub(crate) async fn set_workflow(
        &self,
        session_id: &str,
        workflow: workflow_bridge::WorkflowChannel,
    ) -> Result<()> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.set_workflow(workflow).await
    }

    pub(crate) async fn reserve_user_turn(
        &self,
        session_id: &str,
        client_message_id: String,
        base_rev: Option<u64>,
        author: Option<ClientRef>,
        replay: bool,
    ) -> Result<()> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle
            .reserve_user_turn(client_message_id, base_rev, author, replay)
            .await
    }

    pub(crate) async fn release_user_turn(&self, session_id: &str, client_message_id: String) {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.release_user_turn(client_message_id).await;
    }

    pub(crate) async fn list_turns(
        &self,
        session_id: &str,
        cursor: LogCursor,
        limit: u32,
        caller: Option<String>,
    ) -> Result<SessionTurns> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(session_id.to_string(), self.spawn_actor(session_id));
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.list_turns(cursor, limit, caller).await
    }

    /// Stops a deleted session's actor: its control channel closes, ending its open streams.
    pub(crate) async fn remove(&self, session_id: &str) {
        self.handles.lock().await.remove(session_id);
    }

    /// Asks only a running actor: a revision read must not spawn one.
    pub(crate) async fn rev_state(
        &self,
        session_id: &str,
        caller: Option<String>,
    ) -> Option<ActorRevState> {
        let handles = self.handles.lock().await;
        handles.get(session_id)?.rev_state(caller).await.ok()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn attachment_count(&self, session_id: &str) -> usize {
        let handles = self.handles.lock().await;
        match handles.get(session_id) {
            Some(handle) => handle.attachment_count().await,
            None => 0,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct SessionRuntimeHandle {
    actors: Arc<SessionActors>,
    events: broadcast::Sender<RuntimeEvent>,
}

#[cfg(any(test, feature = "test-support"))]
impl SessionRuntimeHandle {
    pub fn new(db: Arc<Db>) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            actors: Arc::new(SessionActors::new(db, events.clone(), None)),
            events,
        }
    }

    /// A fresh receiver on the runtime events broadcast; subscribe before the
    /// actions whose `SessionRevised`/`SessionReset` you want to observe.
    pub fn subscribe_events(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub async fn reserve_user_turn(
        &self,
        session_id: &str,
        client_message_id: String,
        base_rev: Option<u64>,
        author: Option<ClientRef>,
    ) -> Result<()> {
        self.actors
            .reserve_user_turn(session_id, client_message_id, base_rev, author, false)
            .await
    }

    pub async fn release_user_turn(&self, session_id: &str, client_message_id: String) {
        self.actors
            .release_user_turn(session_id, client_message_id)
            .await
    }

    pub async fn list_turns(
        &self,
        session_id: &str,
        cursor: LogCursor,
        limit: u32,
        caller: Option<String>,
    ) -> Result<SessionTurns> {
        self.actors
            .list_turns(session_id, cursor, limit, caller)
            .await
    }

    pub async fn rev_state(
        &self,
        session_id: &str,
        caller: Option<String>,
    ) -> Option<ActorRevState> {
        self.actors.rev_state(session_id, caller).await
    }

    pub async fn ingress(&self, session_id: &str) -> mpsc::Sender<SessionEvent> {
        self.actors.ingress(session_id).await
    }

    pub async fn attach(
        &self,
        session_id: &str,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        self.actors.attach(session_id, echo_user_messages).await
    }

    pub async fn watch(&self, session_id: &str) -> goal_loop::Watch {
        goal_loop::Watch::attach(self.actors.clone(), session_id)
            .await
            .expect("the session's actor is running")
    }

    pub async fn attachment_count(&self, session_id: &str) -> usize {
        self.actors.attachment_count(session_id).await
    }

    pub async fn log_after(
        &self,
        session_id: &str,
        caller_epoch: u64,
        after_seq: Option<i64>,
    ) -> Result<Vec<SessionLogEntry>> {
        self.actors
            .log_after(session_id, caller_epoch, after_seq)
            .await
    }

    pub async fn attach_after(
        &self,
        session_id: &str,
        caller_epoch: u64,
        after_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> Result<SessionAttachment> {
        self.actors
            .attach_after(session_id, caller_epoch, after_seq, echo_user_messages)
            .await
    }

    pub async fn rewind(
        &self,
        session_id: &str,
        cutoff: i64,
        live: Option<(Arc<dyn AgentDriver>, usize)>,
    ) -> Result<SessionRewind> {
        let outcome = self.actors.rewind(session_id, cutoff, live).await?;
        Ok(SessionRewind {
            live_rewound: outcome.live_rewound,
            epoch: outcome.epoch,
            restart_seq: outcome.restart_seq,
            removed: outcome.removed,
        })
    }

    pub async fn restore_after_failed_relaunch(
        &self,
        session_id: &str,
        cutoff: i64,
        removed: Vec<crate::store::StoredEvent>,
    ) -> Result<()> {
        self.actors
            .restore_after_failed_relaunch(session_id, cutoff, removed)
            .await
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct SessionRewind {
    pub live_rewound: bool,
    pub epoch: u64,
    pub restart_seq: i64,
    pub removed: Vec<crate::store::StoredEvent>,
}
