//! The Hyprlayer GPUI desktop client: `hyprlayer-client` wired into `hyprlayer-ui` views (ADR-0013).

pub mod config;
pub mod connection;
pub mod desktop;
#[cfg(all(debug_assertions, unix))]
pub mod drive;
pub mod error;
pub mod events;
pub mod io;
pub mod logging;
pub mod notify;
pub mod query;
pub mod recovery;
pub mod shell;
pub mod single_instance;
pub mod terminal_theme;
