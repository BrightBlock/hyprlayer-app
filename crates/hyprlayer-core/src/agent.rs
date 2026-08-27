use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::Value;
use tokio::sync::broadcast;

use crate::claude::{ClaudeDriver, ExitOutcome, SessionEvent};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    #[default]
    Claude,
    Opencode,
    Codex,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Opencode => "opencode",
            Harness::Codex => "codex",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "opencode" => Harness::Opencode,
            "codex" => Harness::Codex,
            _ => Harness::Claude,
        }
    }
}

pub trait AgentDriver: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent>;

    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> Result<()>;

    fn set_model(&self, _model: Option<String>) {}

    fn set_effort(&self, _effort: Option<String>) {}

    fn set_collab_mode(&self, _mode: Option<CollabMode>) {}

    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()>;

    fn answer_question(&self, request_id: &str, answers: Value) -> Result<()>;

    fn interrupt(&self) -> Result<()>;

    fn rewind_to_ordinal<'a>(
        &'a self,
        _ordinal: usize,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async {
            Err(CoreError::OutputParse {
                program: "agent".into(),
                detail: "conversation rewind is not supported by this harness".into(),
            })
        })
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
    ) -> Result<()> {
        self.send_user_turn(text, stage, prompt_name)
    }
    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
    ) -> Result<()> {
        self.respond_permission(request_id, allow, message)
    }
    fn answer_question(&self, request_id: &str, answers: Value) -> Result<()> {
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

impl AgentDriver for CodexDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.subscribe()
    }
    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> Result<()> {
        self.send_user_turn(text, stage, prompt_name)
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
    fn answer_question(&self, request_id: &str, answers: Value) -> Result<()> {
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
