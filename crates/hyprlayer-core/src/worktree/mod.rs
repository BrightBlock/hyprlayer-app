//! Worktree isolation: every work item gets its own git worktree and
//! branch, provisioned with working `thoughts/` links and a hyprlayer repo
//! mapping so agent skills resolve the project correctly from inside it.

mod manager;
pub mod seed;
pub mod thoughts_links;

pub use manager::*;

/// Name of the app-managed links directory inside every worktree/repo.
pub const THOUGHTS_DIR: &str = "thoughts";

/// True for the app-managed `thoughts/` tree, infrastructure, never part of a
/// worktree's change set. Shared by every worktree walk (status, diffs, provisioning).
pub fn is_thoughts_path(path: &str) -> bool {
    path.strip_prefix(THOUGHTS_DIR)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}
