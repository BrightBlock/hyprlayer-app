//! Conductor-style isolation: every work item gets its own git worktree and
//! branch, provisioned with working `thoughts/` links and a hyprlayer repo
//! mapping so agent skills resolve the project correctly from inside it.

mod manager;
pub mod thoughts_links;

pub use manager::*;
