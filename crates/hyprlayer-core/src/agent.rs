//! The harness-agnostic driver interface.
//!
//! A live session is driven the same way regardless of which harness backs it
//! (Claude Code today, OpenCode next): send a turn, answer a permission or a
//! question, interrupt, kill, and subscribe to its [`SessionEvent`] stream. The
//! session registry and command handlers hold `Arc<dyn AgentDriver>` so they
//! never name a concrete harness.

use serde_json::Value;
use tokio::sync::broadcast;

use crate::claude::{ClaudeDriver, SessionEvent};
use crate::error::Result;

/// What the app drives a live agent session through, independent of harness.
pub trait AgentDriver: Send + Sync {
    /// Subscribe to the event stream. Late subscribers miss earlier events;
    /// the persistence layer replays from storage for anything before they join.
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent>;

    /// Send a user turn — the opening prompt (with a `stage` label) or a
    /// follow-up (`stage = None`).
    fn send_user_turn(&self, text: &str, stage: Option<&str>) -> Result<()>;

    /// Switch the model used for subsequent turns, if the harness supports
    /// changing it mid-session. OpenCode sends the model with every prompt, so it
    /// can switch live; Claude binds the model at launch, so this is a no-op there
    /// (its model changes by restarting the session). `None` clears any override.
    fn set_model(&self, _model: Option<String>) {}

    /// Answer a `PermissionAsk` (allow/deny, with an optional message).
    fn respond_permission(&self, request_id: &str, allow: bool, message: Option<&str>)
        -> Result<()>;

    /// Answer an `AskUserQuestion` prompt; `answers` is a JSON object mapping
    /// each question's text to the chosen answer(s).
    fn answer_question(&self, request_id: &str, answers: Value) -> Result<()>;

    /// Graceful interrupt of the current turn (the session stays alive).
    fn interrupt(&self) -> Result<()>;

    /// Hard stop: terminate the session. The driver's own machinery emits the
    /// terminal `Exited` event.
    fn kill(&self);

    /// The OS process id backing the session, if any (for the orphan ledger).
    fn pid(&self) -> Option<u32>;
}

/// The Claude Code harness. Each method forwards to `ClaudeDriver`'s inherent
/// method of the same name (inherent methods win in resolution, so there's no
/// recursion).
impl AgentDriver for ClaudeDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.subscribe()
    }
    fn send_user_turn(&self, text: &str, stage: Option<&str>) -> Result<()> {
        self.send_user_turn(text, stage)
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
    fn kill(&self) {
        self.kill()
    }
    fn pid(&self) -> Option<u32> {
        self.pid()
    }
}
