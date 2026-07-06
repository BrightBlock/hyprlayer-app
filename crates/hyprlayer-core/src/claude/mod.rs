//! Claude Code headless session driver.
//!
//! `protocol` parses the `stream-json` wire format tolerantly; `driver` owns a
//! long-lived `claude` child and turns its output into `events::SessionEvent`s.

pub mod driver;
pub mod events;
pub mod protocol;

pub use driver::{ClaudeDriver, PermissionMode, SpawnConfig};
pub use events::{ExitOutcome, SessionEvent, SessionState};
