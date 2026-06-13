use std::sync::Arc;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::index::ThoughtsIndex;
use hyprlayer_core::process::binaries::BinaryResolver;
use hyprlayer_core::store::Db;
use hyprlayer_core::worktree::WorktreeManager;

use crate::session_registry::SessionRegistry;
use crate::settings::SettingsStore;

pub struct AppState {
    pub config: Arc<ConfigService>,
    pub binaries: Arc<BinaryResolver>,
    pub settings: Arc<SettingsStore>,
    pub db: Arc<Db>,
    pub worktrees: Arc<WorktreeManager>,
    pub index: Arc<ThoughtsIndex>,
    /// Live agent sessions (running claude drivers) + concurrency cap.
    pub sessions: Arc<SessionRegistry>,
    /// Live filesystem watcher over the thoughts roots; rebuilt whenever the
    /// hyprlayer config changes (roots may move). Guarded so rebuilds replace
    /// (and thereby drop/stop) the previous watcher.
    pub index_watch: tokio::sync::Mutex<Option<hyprlayer_core::index::watcher::WatchHandle>>,
}
