//! Thought document model: tolerant frontmatter parsing, line-surgical
//! frontmatter patching (body bytes are never touched), new-doc templates, and
//! the PR-description records the PR watcher promotes on merge.

pub mod frontmatter;
pub mod pr_record;
pub mod templates;

pub use frontmatter::*;
