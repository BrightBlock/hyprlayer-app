use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::broadcast;

pub use hyprlayer_models::agent::Harness;
use hyprlayer_models::session::QuestionAnswers;

use crate::claude::{ClaudeDriver, ExitOutcome, PermissionMode, SessionEvent};
use crate::codex::{CodexDriver, CollabMode};
use crate::error::{CoreError, Result};

const KILL_NONE: u8 = 0;
const KILL_USER: u8 = 1;
const KILL_DETACH: u8 = 2;

#[derive(Debug, Default)]
pub struct KillFlag(AtomicU8);

impl KillFlag {
    pub fn mark_killed(&self) -> bool {
        self.set(KILL_USER)
    }

    pub fn mark_detached(&self) -> bool {
        self.set(KILL_DETACH)
    }

    pub fn outcome(&self) -> Option<ExitOutcome> {
        match self.0.load(Ordering::SeqCst) {
            KILL_USER => Some(ExitOutcome::Killed),
            KILL_DETACH => Some(ExitOutcome::Detached),
            _ => None,
        }
    }

    fn set(&self, kind: u8) -> bool {
        self.0
            .compare_exchange(KILL_NONE, kind, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

pub(crate) fn rewind_unsupported() -> CoreError {
    CoreError::OutputParse {
        program: "agent".into(),
        detail: "conversation rewind is not supported by this harness".into(),
    }
}

/// Hands a message the harness never took in back to its sender, who resends it after the turn.
pub(crate) fn declined_message(echo: &SessionEvent) -> Option<SessionEvent> {
    let SessionEvent::UserMessage {
        client_message_id: Some(id),
        ..
    } = echo
    else {
        return None;
    };
    Some(SessionEvent::CommandLifecycle {
        command_uuid: id.clone(),
        state: "cancelled".into(),
    })
}

pub trait AgentDriver: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent>;

    /// Sent mid-turn, the harness takes it in at its next step and the echo
    /// lands then; one it cannot take comes back as a `cancelled` lifecycle.
    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
        client_message_id: Option<&str>,
    ) -> Result<()>;

    /// Withdraws a message waiting for the harness's next step; false once it was taken in.
    fn cancel_queued_message<'a>(
        &'a self,
        _client_message_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

    fn set_model(&self, _model: Option<String>) {}

    fn set_effort(&self, _effort: Option<String>) {}

    fn set_collab_mode(&self, _mode: Option<CollabMode>) {}

    fn set_permission_mode(&self, _mode: PermissionMode) -> Result<()> {
        Err(CoreError::OutputParse {
            program: "agent".into(),
            detail: "this harness cannot change permission mode without a restart".into(),
        })
    }

    /// The collaboration mode the harness last confirmed (codex only): the
    /// ADR-0007 tripwire for a plan mode it silently ignored.
    fn observed_collab_mode(&self) -> Option<String> {
        None
    }

    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()>;

    fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> Result<()>;

    fn interrupt(&self) -> Result<()>;

    fn rewind_to_ordinal<'a>(
        &'a self,
        _ordinal: usize,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async { Err(rewind_unsupported()) })
    }

    fn kill(&self);

    fn kill_detached(&self);

    fn pid(&self) -> Option<u32>;
}

impl AgentDriver for ClaudeDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.subscribe()
    }
    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
        client_message_id: Option<&str>,
    ) -> Result<()> {
        self.send_user_turn(text, stage, prompt_name, client_message_id)
    }
    fn cancel_queued_message<'a>(
        &'a self,
        client_message_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(self.cancel_queued_message(client_message_id))
    }
    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.respond_permission(request_id, allow, message)
    }
    fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> Result<()> {
        self.answer_question(request_id, answers)
    }
    fn interrupt(&self) -> Result<()> {
        self.interrupt()
    }
    fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        self.set_permission_mode(mode)
    }
    fn rewind_to_ordinal<'a>(
        &'a self,
        ordinal: usize,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(self.rewind_to_ordinal(ordinal))
    }
    fn kill(&self) {
        self.kill()
    }
    fn kill_detached(&self) {
        self.kill_detached()
    }
    fn pid(&self) -> Option<u32> {
        self.pid()
    }
}

impl AgentDriver for CodexDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.subscribe()
    }
    fn observed_collab_mode(&self) -> Option<String> {
        self.observed_collab_mode()
    }
    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
        client_message_id: Option<&str>,
    ) -> Result<()> {
        self.send_user_turn(text, stage, prompt_name, client_message_id)
    }
    fn set_model(&self, model: Option<String>) {
        self.set_model(model)
    }
    fn set_effort(&self, effort: Option<String>) {
        self.set_effort(effort)
    }
    fn set_collab_mode(&self, mode: Option<CollabMode>) {
        self.set_collab_mode(mode)
    }
    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.respond_permission(request_id, allow, message)
    }
    fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> Result<()> {
        self.answer_question(request_id, answers)
    }
    fn interrupt(&self) -> Result<()> {
        self.interrupt()
    }
    fn rewind_to_ordinal<'a>(
        &'a self,
        ordinal: usize,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(self.rewind_to_ordinal(ordinal))
    }
    fn kill(&self) {
        self.kill()
    }
    fn kill_detached(&self) {
        self.kill_detached()
    }
    fn pid(&self) -> Option<u32> {
        self.pid()
    }
}
