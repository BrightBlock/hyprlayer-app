use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::index::ThoughtsIndex;
use hyprlayer_core::process::binaries::BinaryResolver;
use hyprlayer_core::store::Db;
use hyprlayer_core::worktree::WorktreeManager;

use crate::script_registry::ScriptRegistry;
use crate::session_registry::SessionRegistry;
use crate::settings::SettingsStore;
use crate::terminal_registry::TerminalRegistry;

pub struct AppState {
    pub config: Arc<ConfigService>,
    pub binaries: Arc<BinaryResolver>,
    pub settings: Arc<SettingsStore>,
    pub db: Arc<Db>,
    pub worktrees: Arc<WorktreeManager>,
    pub index: Arc<ThoughtsIndex>,
    pub sessions: Arc<SessionRegistry>,
    pub opencode: Arc<crate::opencode::OpenCodeManager>,
    pub terminals: Arc<TerminalRegistry>,
    pub scripts: Arc<ScriptRegistry>,
    pub index_watch: tokio::sync::Mutex<Option<hyprlayer_core::index::watcher::WatchHandle>>,
    pub worktree_watch:
        tokio::sync::Mutex<Option<(String, hyprlayer_core::index::watcher::WatchHandle)>>,
    pub focused: Arc<AtomicBool>,
    pub focus_changed: Arc<tokio::sync::Notify>,
}
