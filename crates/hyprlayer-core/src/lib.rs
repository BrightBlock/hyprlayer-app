//! Core logic for the Hyprlayer desktop app.
//!
//! This crate is deliberately free of Tauri dependencies so it can be tested
//! headless and fast. The Tauri shell (`src-tauri`) is a thin layer over it.

pub mod config;
pub mod error;
pub mod hyprcli;
pub mod process;

pub use error::CoreError;
