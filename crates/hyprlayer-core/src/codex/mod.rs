//! OpenAI Codex headless session driver.
//!
//! `protocol` decodes the `codex app-server` JSON-RPC stream and translates its
//! v2 thread notifications into `SessionEvent`s; `driver` owns a long-lived
//! `codex app-server` child, one per session.

pub mod driver;
pub mod probe;
pub mod protocol;

pub use driver::{CodexDriver, SpawnConfig};
pub use probe::{CodexAuthStatus, CodexModel, CodexProbe};

/// Serializes `codex app-server` STARTUP across the whole app.
///
/// Two app-servers starting at the same time interfere: one loses and exits
/// without answering. Measured on Windows, roughly one collision per three
/// concurrent pairs — enough that opening the Codex settings tab while a Codex
/// chat starts would intermittently fail one of them.
///
/// Held only across spawn + handshake, never for a session's lifetime, so a
/// long-lived driver doesn't block later probes. Both hold sites sit inside a
/// timeout, so a wedged child bounds the wait rather than deadlocking the app.
pub(crate) fn startup_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(Default::default)
}
