//! The harness-agnostic driver interface.
//!
//! A live session is driven the same way regardless of which harness backs it
//! (Claude Code, OpenCode, Codex): send a turn, answer a permission or a
//! question, interrupt, kill, and subscribe to its [`SessionEvent`] stream. The
//! session registry and command handlers hold `Arc<dyn AgentDriver>` so they
//! never name a concrete harness.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::Value;
use tokio::sync::broadcast;

use crate::claude::{ClaudeDriver, ExitOutcome, SessionEvent};
use crate::codex::CodexDriver;
use crate::error::{CoreError, Result};

const KILL_NONE: u8 = 0;
const KILL_USER: u8 = 1;
const KILL_DETACH: u8 = 2;

/// Why the app ended a session's process — shared between a driver's kill entry
/// points and the waiter that classifies the exit, so the reason survives the
/// hop. First writer wins: the request that actually took the process down is
/// the honest reason, and a later racing call can't relabel it.
#[derive(Debug, Default)]
pub struct KillFlag(AtomicU8);

impl KillFlag {
    /// The user ended this session (Stop escalating to a force-stop, clearing
    /// the context). Returns whether this call is the one that set it — drivers
    /// use that to stay idempotent.
    pub fn mark_killed(&self) -> bool {
        self.set(KILL_USER)
    }

    /// The app is tearing the process down as its own bookkeeping — quitting, or
    /// relaunching the session under new settings. Reported as
    /// [`ExitOutcome::Detached`], which leaves no trace in the transcript.
    pub fn mark_detached(&self) -> bool {
        self.set(KILL_DETACH)
    }

    /// The outcome an app-initiated kill forces, or `None` to classify from the
    /// process's own wait status.
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

    /// Tolerant row-read: unknown ⇒ `Claude` (the default harness).
    pub fn parse(s: &str) -> Self {
        match s {
            "opencode" => Harness::Opencode,
            "codex" => Harness::Codex,
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

    /// Switch the reasoning effort used for subsequent turns. Codex sends effort
    /// with every `turn/start`, so it can switch live; Claude binds it at launch
    /// and OpenCode manages its own, so this is a no-op there.
    fn set_effort(&self, _effort: Option<String>) {}

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

    /// Terminate the session because the APP is tearing it down — quitting, or
    /// relaunching it under new settings — rather than the user ending it. Same
    /// teardown as [`kill`](Self::kill), but the exit reports
    /// [`ExitOutcome::Detached`]: nothing is persisted, so the transcript stays
    /// clean and the session replays resumable.
    fn kill_detached(&self);

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
    fn kill_detached(&self) {
        self.kill_detached()
    }
    fn pid(&self) -> Option<u32> {
        self.pid()
    }
}

/// The Codex harness. Like the Claude impl, each method forwards to
/// `CodexDriver`'s inherent method of the same name.
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
        // Codex takes a model per turn, so a mid-session switch just changes what
        // the next `turn/start` sends — no relaunch (unlike Claude).
        self.set_model(model)
    }
    fn set_effort(&self, effort: Option<String>) {
        self.set_effort(effort)
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
    fn kill_detached(&self) {
        self.kill_detached()
    }
    fn pid(&self) -> Option<u32> {
        self.pid()
    }
}
