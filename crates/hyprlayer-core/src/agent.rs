//! The harness-agnostic driver interface.
//!
//! A live session is driven the same way regardless of which harness backs it
//! (Claude Code today, OpenCode next): send a turn, answer a permission or a
//! question, interrupt, kill, and subscribe to its [`SessionEvent`] stream. The
//! session registry and command handlers hold `Arc<dyn AgentDriver>` so they
//! never name a concrete harness.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::claude::{ClaudeDriver, SessionEvent};
use crate::error::{CoreError, Result};

/// Which harness backs a session — the vocabulary the store persists
/// (`sessions.harness`), the IPC surface selects on, and the shell dispatches
/// drivers by. sqlite stores `as_str` TEXT; read back via the tolerant
/// [`Harness::parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    // Last-variant #[serde(other)] isn't needed here — Claude is first so a
    // struct-level default covers it; unknown wire values are a caller bug we
    // want surfaced, while row reads go through `parse`.
    Claude,
    Opencode,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Opencode => "opencode",
        }
    }

    /// Tolerant row-read: unknown ⇒ `Claude` (the default harness).
    pub fn parse(s: &str) -> Self {
        match s {
            "opencode" => Harness::Opencode,
            _ => Harness::Claude,
        }
    }
}

impl Default for Harness {
    fn default() -> Self {
        Harness::Claude
    }
}

/// What the app drives a live agent session through, independent of harness.
pub trait AgentDriver: Send + Sync {
    /// Subscribe to the event stream. Late subscribers miss earlier events;
    /// the persistence layer replays from storage for anything before they join.
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent>;

    /// Send a user turn — the opening prompt (with a `stage` label) or a
    /// follow-up (`stage = None`). `prompt_name` names the custom prompt the
    /// turn was run from, else `None` for a turn the user typed.
    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
    ) -> Result<()>;

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

    /// Rewind the model's OWN conversation to the 1-based `ordinal` human turn,
    /// truncating that turn and everything after it out of the live session (and
    /// its persisted transcript) so a resume reloads the shortened conversation.
    /// Default: unsupported — only Claude implements it (OpenCode owns its own
    /// history). Callers fall back to a log-only truncation on `Err`.
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
        // Inherent async fn of the same name wins resolution — no recursion.
        Box::pin(self.rewind_to_ordinal(ordinal))
    }
    fn kill(&self) {
        self.kill()
    }
    fn pid(&self) -> Option<u32> {
        self.pid()
    }
}
