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
    /// Live agent sessions (Claude or OpenCode drivers), keyed by session id.
    pub sessions: Arc<SessionRegistry>,
    /// The shared `opencode serve` process + global event stream, started on
    /// first use of the OpenCode harness.
    pub opencode: Arc<crate::opencode::OpenCodeManager>,
    /// Live PTY terminals, keyed by terminal id.
    pub terminals: Arc<TerminalRegistry>,
    /// Per-workstream lifecycle-script state: the live Run process, its injected
    /// `$HL_PORT`, and each script's last-run status.
    pub scripts: Arc<ScriptRegistry>,
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
