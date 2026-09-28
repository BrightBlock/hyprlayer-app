pub mod delivery;
mod followup;
mod forwarder;
pub mod registry;
mod service;
mod workflow_bridge;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

use hyprlayer_models::RuntimeEvent;

use crate::agent::AgentDriver;
use crate::claude::SessionEvent;
use crate::error::Result;
use crate::store::{Db, LogCursor, SessionLogEntry};

pub use delivery::{SessionAttachment, SessionStreamError};
pub use service::{
    ActiveSession, LaunchAndConverseParams, LaunchSessionParams, RewindSessionOutcome,
    RewindSessionRequest, SessionService, StartStageSessionRequest, StartedStageSession,
};

#[cfg(any(test, feature = "test-support"))]
pub use followup::run_codex_followup_for_test;

pub(crate) struct SessionActors {
    db: Arc<Db>,
    events: broadcast::Sender<RuntimeEvent>,
    handles: AsyncMutex<HashMap<String, forwarder::ActorHandle>>,
}

impl SessionActors {
    pub(crate) fn new(db: Arc<Db>, events: broadcast::Sender<RuntimeEvent>) -> Self {
        Self {
            db,
            events,
            handles: AsyncMutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn ingress(&self, session_id: &str) -> mpsc::Sender<SessionEvent> {
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.get(session_id) {
            return handle.ingress.clone();
        }
        let handle = forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone());
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
            handles.insert(
                session_id.to_string(),
                forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone()),
            );
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
            handles.insert(
                session_id.to_string(),
                forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone()),
            );
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
            handles.insert(
                session_id.to_string(),
                forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone()),
            );
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
            handles.insert(
                session_id.to_string(),
                forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone()),
            );
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
            handles.insert(
                session_id.to_string(),
                forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone()),
            );
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

    pub(crate) async fn set_workflow(
        &self,
        session_id: &str,
        workflow: workflow_bridge::WorkflowChannel,
    ) -> Result<()> {
        let mut handles = self.handles.lock().await;
        if !handles.contains_key(session_id) {
            handles.insert(
                session_id.to_string(),
                forwarder::spawn(session_id.to_string(), self.db.clone(), self.events.clone()),
            );
        }
        let handle = handles.get(session_id).expect("just inserted above");
        handle.set_workflow(workflow).await
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
}

#[cfg(any(test, feature = "test-support"))]
impl SessionRuntimeHandle {
    pub fn new(db: Arc<Db>) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            actors: Arc::new(SessionActors::new(db, events)),
        }
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
