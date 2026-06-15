//! Thought document model: tolerant frontmatter parsing, line-surgical
//! frontmatter patching (body bytes are never touched), and new-doc templates.

pub mod frontmatter;
pub mod templates;

pub use frontmatter::*;
