pub mod driver;
pub mod probe;
pub mod protocol;

pub use driver::{config_sets_developer_instructions, CodexDriver, SpawnConfig};
pub use probe::{CodexAuthStatus, CodexModel, CodexProbe};
pub use protocol::{CodexPosture, CollabMode};

pub(crate) fn startup_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(Default::default)
}
