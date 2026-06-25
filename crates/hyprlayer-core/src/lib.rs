//! Core logic for the Hyprlayer desktop app.
//!
//! This crate is deliberately free of Tauri dependencies so it can be tested
//! headless and fast. The Tauri shell (`src-tauri`) is a thin layer over it.

pub mod agent;
pub mod claude;
pub mod config;
pub mod docs;
pub mod error;
pub mod gitops;
pub mod hyprcli;
pub mod index;
pub mod opencode;
pub mod pipeline;
pub mod pr;
pub mod process;
pub mod repo_settings;
pub mod repos;
pub mod skills;
pub mod store;
pub mod terminal;
pub mod worktree;

pub use error::CoreError;
