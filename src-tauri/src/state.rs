use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::index::ThoughtsIndex;
use hyprlayer_core::process::binaries::BinaryResolver;
use hyprlayer_core::store::Db;
use hyprlayer_core::worktree::WorktreeManager;

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
    /// Live agent sessions (running claude drivers), keyed by session id.
    pub sessions: Arc<SessionRegistry>,
    /// Live PTY terminals, keyed by terminal id.
    pub terminals: Arc<TerminalRegistry>,
    /// Live filesystem watcher over the thoughts roots; rebuilt whenever the
    /// hyprlayer config changes (roots may move). Guarded so rebuilds replace
    /// (and thereby drop/stop) the previous watcher.
    pub index_watch: tokio::sync::Mutex<Option<hyprlayer_core::index::watcher::WatchHandle>>,
    /// Live filesystem watcher over the *active* workstream's worktree, pushing
    /// `WorktreeChanged` so the Changes panel / git status update on external
    /// edits without waiting for window focus. Single-slot keyed by workstream
    /// id: switching workstreams replaces (and drops) the previous watcher.
    pub worktree_watch:
        tokio::sync::Mutex<Option<(String, hyprlayer_core::index::watcher::WatchHandle)>>,
    /// Whether the main window currently has OS focus. Background pollers (the PR
    /// watcher) read this to pause work while the app is unfocused — no reason to
    /// spawn `gh` subprocesses for a merge bar nobody is looking at.
    pub focused: Arc<AtomicBool>,
    /// Notified whenever `focused` flips, so a paused poller wakes promptly on
    /// refocus instead of sleeping out its full backoff.
    pub focus_changed: Arc<tokio::sync::Notify>,
}
