//! OpenCode harness: reading/writing OpenCode's own on-disk state and (later)
//! driving `opencode serve` to run sessions.
//!
//! OpenCode is a separate, user-owned tool. We integrate by configuring the
//! files it already reads — never inventing a parallel store:
//!   - `auth.json`     — provider credentials (XDG data dir)
//!   - `opencode.json` — config incl. the default model (XDG config dir)
//!
//! All writes are merge-preserving and atomic; we never clobber keys or fields
//! we didn't set.

pub mod config;
