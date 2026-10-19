pub mod account;
pub mod agents;
pub mod attachments;
mod attribution;
pub mod builder;
mod doc_status;
pub mod goals;
pub mod library;
pub mod migrations;
mod operation_locks;
pub mod ports;
pub mod pr_watcher;
mod profile_lock;
pub mod projects;
pub mod prompts;
pub mod providers;
pub mod reviews;
pub mod script_registry;
pub mod scripts;
pub mod sessions;
pub mod settings_service;
pub mod shutdown;
pub mod skills;
pub mod terminal_registry;
pub mod terminals;
pub mod workstreams;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, watch, Mutex as AsyncMutex, Notify};

use hyprlayer_models::runtime::ThoughtsIndexUpdateKind;
use hyprlayer_models::RuntimeEvent;

pub use crate::env::{RuntimeEnvironment, RuntimePaths};

use crate::account::endpoints::CloudEndpoints;
use crate::config::ConfigService;
use crate::error::Result;
use crate::index::watcher::WatchHandle;
use crate::index::ThoughtsIndex;
use crate::opencode::runtime::OpenCodeManager;
use crate::process::binaries::BinaryResolver;
use crate::settings::SettingsStore;
use crate::store::Db;
use crate::worktree::WorktreeManager;

use attribution::DocAttribution;
use operation_locks::{SkillLocks, WorkstreamLocks};
use ports::{CliConfigCli, ConfigCli, LifecycleRunner, ProviderHttp, ReqwestProviderHttp};
use pr_watcher::{PrWatcherContext, WorktreeWatch};
use profile_lock::ProfileLock;
use script_registry::ScriptRegistry;
use sessions::registry::SessionRegistry;
use sessions::SessionActors;
use shutdown::{ShutdownCoordinator, ShutdownReport, TrackedTask};
use terminal_registry::TerminalRegistry;

const EVENT_CAP: usize = 256;
const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ARCHIVE_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const PROVIDER_HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const ORPHAN_EXIT_WAIT: Duration = Duration::from_secs(2);
const ORPHAN_POLL: Duration = Duration::from_millis(50);

pub struct RuntimeOptions {
    pub environment: RuntimeEnvironment,
    pub paths: RuntimePaths,
}

impl RuntimeOptions {
    pub fn new(environment: RuntimeEnvironment, paths: RuntimePaths) -> Self {
        Self { environment, paths }
    }
}

struct Watchers {
    index_watch: AsyncMutex<Option<WatchHandle>>,
    worktree_watch: WorktreeWatch,
}

pub struct HyprlayerRuntime {
    environment: RuntimeEnvironment,
    paths: RuntimePaths,
    config: Arc<ConfigService>,
    binaries: Arc<BinaryResolver>,
    settings: Arc<SettingsStore>,
    db: Arc<Db>,
    worktrees: Arc<WorktreeManager>,
    index: Arc<ThoughtsIndex>,
    session_registry: Arc<SessionRegistry>,
    session_actors: Arc<SessionActors>,
    goal_runners: Arc<goals::GoalRunners>,
    attribution: Arc<DocAttribution>,
    session_titler: Arc<sessions::titles::SessionTitler>,
    terminals: Arc<TerminalRegistry>,
    scripts: Arc<ScriptRegistry>,
    opencode: Arc<OpenCodeManager>,
    watchers: Watchers,
    focused: Arc<AtomicBool>,
    /// The last full scan's document count, so a rescan that changes nothing
    /// logs at debug.
    indexed_docs: AtomicU32,
    focus_changed: Arc<Notify>,
    events: broadcast::Sender<RuntimeEvent>,
    /// Set by the first client's environment overlay; the goal resumer waits on it.
    overlay_arrived: watch::Sender<bool>,
    config_cli: Arc<dyn ConfigCli>,
    provider_http: Arc<dyn ProviderHttp + Send + Sync>,
    lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
    workstream_locks: WorkstreamLocks,
    skill_locks: SkillLocks,
    account_state: account::AccountState,
    coordinator: ShutdownCoordinator,
    tasks: AsyncMutex<Vec<TrackedTask>>,
    _profile_lock: ProfileLock,
}

impl HyprlayerRuntime {
    pub async fn open(options: RuntimeOptions) -> Result<Arc<Self>> {
        Self::open_with(options, None, None).await
    }

    pub(crate) async fn open_with(
        options: RuntimeOptions,
        config_cli: Option<Arc<dyn ConfigCli>>,
        lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
    ) -> Result<Arc<Self>> {
        let RuntimeOptions { environment, paths } = options;

        let profile_lock = ProfileLock::acquire(&paths.app_data_dir)?;

        migrations::apply_pending(&paths)?;

        let config = Arc::new(ConfigService::new(paths.hyprlayer_config_file.clone()));
        let settings = Arc::new(SettingsStore::load(paths.app_config_dir.clone()));
        let binary_overrides: std::collections::HashMap<_, _> = settings
            .get()
            .await
            .binary_overrides
            .iter()
            .map(|(k, v)| (*k, std::path::PathBuf::from(v)))
            .collect();
        let binaries = Arc::new(BinaryResolver::initialize(&environment, binary_overrides));
        crate::process::orphan::reap(
            &paths
                .app_data_dir
                .join(crate::opencode::runtime::OPENCODE_PID_FILE),
        );

        let db_path = paths.app_data_dir.join("hyprlayer.db");
        let db = Arc::new(Db::open(&db_path)?);

        if let Ok(orphans) = db.list_orphans() {
            if !orphans.is_empty() {
                let reaped = reap_goal_orphans(&db, &orphans).await;
                // A pid may since belong to something else; the program says.
                let described: Vec<String> = orphans
                    .iter()
                    .filter(|(pid, ..)| !reaped.contains(pid))
                    .map(|(pid, session, _)| {
                        match crate::process::orphan::running_program(*pid as u32) {
                            Some(program) => {
                                format!("{pid} (session {session}) still runs {program}")
                            }
                            None => format!("{pid} (session {session}) has exited"),
                        }
                    })
                    .collect();
                if !described.is_empty() {
                    tracing::warn!(
                        "{} agent process(es) outlived a previous run: {}",
                        described.len(),
                        described.join("; ")
                    );
                }
                let _ = db.clear_all_orphans();
            }
        }

        // Nothing is live before this runtime spawns it, so no row may claim to be.
        match db.settle_stale_sessions() {
            Ok(0) => {}
            Ok(n) => tracing::info!("{n} session(s) a previous run left mid-turn are now idle"),
            Err(e) => tracing::warn!("could not settle sessions a previous run left mid-turn: {e}"),
        }

        let settings_snapshot = settings.get().await;
        let worktree_root = settings_snapshot
            .worktree_root
            .as_deref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| paths.default_worktree_root.clone());

        tracing::info!(
            "runtime opened: data {}, worktrees {}, config {}",
            paths.app_data_dir.display(),
            worktree_root.display(),
            paths.hyprlayer_config_file.display()
        );

        let (events, _) = broadcast::channel(EVENT_CAP);
        let attribution = Arc::new(DocAttribution::new(
            db.clone(),
            config.clone(),
            events.clone(),
        ));
        let session_actors = Arc::new(SessionActors::new(
            db.clone(),
            events.clone(),
            Some(attribution.clone()),
        ));

        let config_cli =
            config_cli.unwrap_or_else(|| Arc::new(CliConfigCli::new(binaries.clone())));
        let provider_http: Arc<dyn ProviderHttp + Send + Sync> =
            Arc::new(ReqwestProviderHttp::new(PROVIDER_HTTP_TIMEOUT));
        let cloud = CloudEndpoints::resolve(&environment);
        tracing::info!(
            "Cloud account: {} pool, sign-in at {}",
            cloud.pool.as_str(),
            cloud.login_url
        );
        let worktrees = Arc::new(
            WorktreeManager::new(worktree_root)
                .with_env_overlay(environment.env_overlay.clone())
                .with_binaries(binaries.clone()),
        );

        Ok(Arc::new(Self {
            config_cli,
            provider_http,
            lifecycle_runner,
            workstream_locks: WorkstreamLocks::new(),
            skill_locks: SkillLocks::default(),
            account_state: account::AccountState::new(cloud, &paths.app_config_dir),
            config,
            binaries,
            settings,
            db,
            worktrees,
            index: Arc::new(ThoughtsIndex::new()),
            session_registry: Arc::new(SessionRegistry::new()),
            session_actors,
            goal_runners: Arc::default(),
            attribution,
            session_titler: Arc::default(),
            terminals: Arc::new(TerminalRegistry::new()),
            scripts: Arc::new(ScriptRegistry::new()),
            opencode: Arc::new(OpenCodeManager::new()),
            watchers: Watchers {
                index_watch: AsyncMutex::new(None),
                worktree_watch: Arc::new(AsyncMutex::new(None)),
            },
            focused: Arc::new(AtomicBool::new(true)),
            indexed_docs: AtomicU32::new(u32::MAX),
            focus_changed: Arc::new(Notify::new()),
            events,
            overlay_arrived: watch::Sender::new(false),
            environment,
            paths,
            coordinator: ShutdownCoordinator::new(),
            tasks: AsyncMutex::new(Vec::new()),
            _profile_lock: profile_lock,
        }))
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        self.coordinator.try_start()?;
        self.goal_runners.bind(Arc::downgrade(self));

        // Tracked, and holding a `Weak` rather than an `Arc`, because the login-shell
        // probe below carries an eight-second timeout. An untracked task holding a
        // strong reference kept the whole runtime — and so the profile lock — alive
        // for that long after the caller had dropped it, which breaks the contract
        // that dropping a runtime frees its profile for the next one.
        let startup_binaries = self.binaries.clone();
        let startup_events = self.events.clone();
        let startup = Arc::downgrade(self);
        let startup_task = TrackedTask::spawn(move |stop| async move {
            let work = async move {
                if startup_binaries.enrich_with_login_shell().await {
                    let _ = startup_events.send(RuntimeEvent::BinariesChanged);
                }
                log_binaries(&startup_binaries).await;
                let Some(runtime) = startup.upgrade() else {
                    return;
                };
                if runtime.migrate_config_if_needed().await {
                    let _ = startup_events.send(RuntimeEvent::ConfigChanged);
                }
                runtime.rebuild_thoughts_index().await;
                // After the rebuild: tool paths resolve against the loaded roots.
                runtime.attribution.backfill().await;
            };
            tokio::select! {
                _ = work => {}
                _ = stop.notified() => {}
            }
        });

        let config_poll = self.clone();
        let config_task = TrackedTask::spawn(move |stop| async move {
            let path = config_poll.config.path().to_path_buf();
            let key = |p: &Path| {
                std::fs::metadata(p)
                    .ok()
                    .map(|m| (m.modified().ok(), m.len()))
            };
            let mut last = key(&path);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(CONFIG_POLL_INTERVAL) => {}
                    _ = stop.notified() => return,
                }
                let current = key(&path);
                if current != last {
                    last = current;
                    tracing::info!("{} changed; reloading it", path.display());
                    config_poll.config.invalidate().await;
                    let _ = config_poll.events.send(RuntimeEvent::ConfigChanged);
                    config_poll.rebuild_thoughts_index().await;
                }
            }
        });

        let pr_watcher_task = pr_watcher::spawn(PrWatcherContext {
            binaries: self.binaries.clone(),
            db: self.db.clone(),
            config: self.config.clone(),
            worktree_watch: self.watchers.worktree_watch.clone(),
            focused: self.focused.clone(),
            focus_changed: self.focus_changed.clone(),
            events: self.events.clone(),
            env_overlay: self.environment.env_overlay.clone(),
        });

        // A `Weak` for the same reason as startup; a stop mid-sweep drops it
        // between steps, and the next sweep finishes whatever was left.
        let sweeper = Arc::downgrade(self);
        let sweep_task = TrackedTask::spawn(move |stop| async move {
            loop {
                {
                    let Some(runtime) = sweeper.upgrade() else {
                        return;
                    };
                    let workstreams = runtime.workstreams();
                    tokio::select! {
                        swept = workstreams.sweep_expired_archives() => {
                            if let Err(e) = swept {
                                tracing::warn!("archive sweep failed: {e}");
                            }
                        }
                        _ = stop.notified() => return,
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(ARCHIVE_SWEEP_INTERVAL) => {}
                    _ = stop.notified() => return,
                }
            }
        });

        // A `Weak` like startup; a stored session is checked off the open path, then on a timer.
        let account_check = Arc::downgrade(self);
        let account_task = TrackedTask::spawn(move |stop| async move {
            loop {
                let Some(runtime) = account_check.upgrade() else {
                    return;
                };
                let account = runtime.account();
                tokio::select! {
                    _ = account.check() => {}
                    _ = stop.notified() => return,
                }
                drop(runtime);
                tokio::select! {
                    _ = tokio::time::sleep(account::CHECK_INTERVAL) => {}
                    _ = stop.notified() => return,
                }
            }
        });

        let title_task =
            sessions::titles::spawn_worker(self.session_titler.clone(), Arc::downgrade(self));

        let goal_task = goals::spawn_resumer(self.goal_runners.clone(), Arc::downgrade(self));

        self.tasks.lock().await.extend([
            startup_task,
            config_task,
            pr_watcher_task,
            sweep_task,
            account_task,
            title_task,
            goal_task,
        ]);

        Ok(())
    }

    pub fn set_foreground(&self, foreground: bool) {
        self.focused.store(foreground, Ordering::Relaxed);
        self.focus_changed.notify_waiters();
    }

    /// Merges a client's overlay (`SSH_AUTH_SOCK` and friends) over every child spawn, per key (ADR-0009).
    pub fn set_env_overlay(&self, mut overlay: std::collections::BTreeMap<String, String>) {
        if overlay
            .get("SSH_AUTH_SOCK")
            .is_some_and(|sock| !is_agent_socket(std::path::Path::new(sock)))
        {
            // A client-side path (a remote laptop's agent) would replace one that works here.
            tracing::info!(
                "dropping a client's SSH_AUTH_SOCK: no socket at that path on this host"
            );
            overlay.remove("SSH_AUTH_SOCK");
        }
        // Names only; whether an agent socket arrived decides if git and gh
        // can authenticate over SSH (ADR-0005).
        tracing::info!(
            "environment overlay from a client: {} ({})",
            overlay.keys().cloned().collect::<Vec<_>>().join(", "),
            if overlay.contains_key("SSH_AUTH_SOCK") {
                "with an SSH agent"
            } else {
                "no SSH agent"
            }
        );
        self.environment.merge_env_overlay(overlay);
        self.overlay_arrived.send_replace(true);
    }

    pub(crate) async fn child_env(&self) -> Vec<(String, String)> {
        self.binaries
            .child_env(self.environment.overlay_env())
            .await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub fn worktree_root(&self) -> &Path {
        self.worktrees.root()
    }

    pub async fn shutdown(&self) -> ShutdownReport {
        self.coordinator
            .shutdown(async move {
                let start = std::time::Instant::now();
                let tasks = std::mem::take(&mut *self.tasks.lock().await);
                let (forced_tasks, _) = shutdown::stop_tasks(tasks).await;
                self.goal_runners.stop_all().await;

                let terminals = self.terminals.kill_all_blocking();
                let sessions = self.session_registry.kill_all_blocking();
                self.opencode.shutdown().await;
                let count = |killed: Option<usize>| {
                    killed.map_or("an unknown number of (registry busy)".to_string(), |n| {
                        n.to_string()
                    })
                };
                tracing::info!(
                    "shutdown ended {} terminal(s) and {} agent session(s)",
                    count(terminals),
                    count(sessions)
                );

                ShutdownReport {
                    elapsed: start.elapsed(),
                    forced_process_count: forced_tasks,
                    task_failures: Vec::new(),
                }
            })
            .await
    }

    pub fn shutdown_now(&self) {
        self.coordinator.shutdown_now(|| {
            self.terminals.kill_all_blocking();
            self.session_registry.kill_all_blocking();
            self.opencode.kill_blocking();
        });
    }

    pub async fn migrate_config_if_needed(&self) -> bool {
        if !self.config.exists() {
            return false;
        }
        let needs = matches!(
            self.config.load().await,
            Err(ref e) if e.code() == "config_needs_migration"
        );
        if !needs {
            return false;
        }
        match self
            .config_cli
            .storage_info(&self.environment.home_dir)
            .await
        {
            Ok(_) => {
                self.config.invalidate().await;
                true
            }
            Err(e) => {
                tracing::warn!("config migration nudge failed: {e}");
                false
            }
        }
    }

    pub async fn rebuild_thoughts_index(&self) {
        let roots = match self.config.load().await {
            Ok(loaded) => crate::index::thoughts_roots(&loaded.config),
            Err(e) => {
                tracing::warn!("no thoughts roots to index: {e}");
                Vec::new()
            }
        };

        let index = self.index.clone();
        let scan_roots = roots.clone();
        let started = std::time::Instant::now();
        let stats = tokio::task::spawn_blocking(move || index.full_scan(&scan_roots)).await;
        if let Ok(stats) = &stats {
            // Rescans follow every config change; only news reaches info.
            let changed = self.indexed_docs.swap(stats.indexed, Ordering::Relaxed) != stats.indexed;
            let summary = format!(
                "thoughts index: {} docs ({} skipped, {} errors) across {} roots in {:?}",
                stats.indexed,
                stats.skipped,
                stats.errors,
                roots.len(),
                started.elapsed()
            );
            if changed || stats.errors > 0 {
                tracing::info!("{summary}");
            } else {
                tracing::debug!("{summary}");
            }
        }

        // Per root path, so a profile added or repointed later also starts read.
        let db = self.db.clone();
        let stamp_roots = roots.clone();
        let stamped = tokio::task::spawn_blocking(move || {
            let at = crate::store::now_ms();
            for root in stamp_roots.iter().filter(|r| r.root.is_dir()) {
                if let Err(e) = db.ensure_read_baseline(&root.root.to_string_lossy(), at) {
                    tracing::warn!(root = %root.root.display(), "read baseline not stamped: {e}");
                }
            }
        })
        .await;
        if let Err(e) = stamped {
            tracing::warn!("read baseline task failed: {e}");
        }

        let watch = if roots.is_empty() {
            None
        } else {
            let events = self.events.clone();
            let callback = Box::new(move |event: crate::index::watcher::IndexEvent| {
                use crate::index::watcher::IndexEvent;
                let (update, ids) = match event {
                    IndexEvent::Updated { ids } => (ThoughtsIndexUpdateKind::Updated, ids),
                    IndexEvent::Removed { ids } => (ThoughtsIndexUpdateKind::Removed, ids),
                    IndexEvent::Rescanned => (ThoughtsIndexUpdateKind::Rescanned, Vec::new()),
                };
                let _ = events.send(RuntimeEvent::ThoughtsIndexUpdated { update, ids });
            });
            let watch_index = self.index.clone();
            let armed = tokio::task::spawn_blocking(move || {
                crate::index::watcher::watch_roots(watch_index, roots, callback)
            })
            .await;
            match armed {
                Ok(Ok(handle)) => Some(handle),
                Ok(Err(e)) => {
                    tracing::warn!("thoughts watcher failed to start: {e}");
                    None
                }
                Err(e) => {
                    tracing::warn!("thoughts watcher task failed: {e}");
                    None
                }
            }
        };
        *self.watchers.index_watch.lock().await = watch;

        let _ = self.events.send(RuntimeEvent::ThoughtsIndexUpdated {
            update: ThoughtsIndexUpdateKind::Rescanned,
            ids: Vec::new(),
        });
    }
}

/// Kills the harness a live goal's session left running, so its relaunch is the only one; returns the pids signalled.
async fn reap_goal_orphans(db: &Db, orphans: &[(i64, String, f64)]) -> HashSet<i64> {
    use crate::process::orphan;

    let sessions = match db.live_goal_sessions() {
        Ok(sessions) => sessions,
        Err(e) => {
            tracing::warn!("could not read the sessions of live goals: {e}");
            return HashSet::new();
        }
    };
    let mut reaped = HashSet::new();
    for (pid, session, spawned_at) in orphans {
        let Some((_, harness, harness_id)) = sessions.iter().find(|(id, ..)| id == session) else {
            continue;
        };
        let Some(line) = u32::try_from(*pid)
            .ok()
            .filter(|p| *p > 1)
            .and_then(orphan::cmdline)
        else {
            continue;
        };
        // A fork resumes under its own harness session id.
        let ours = std::iter::once(session.as_str())
            .chain(harness_id.as_deref())
            .any(|id| orphan::harness_orphan_matches(&line, *harness, id));
        // A codex argv names no thread, so a pid that started after its record was reused.
        let reused = *harness == crate::agent::Harness::Codex
            && !orphan::started_by(orphan::started_at_ms(*pid as u32), *spawned_at);
        if !ours || reused {
            continue;
        }
        tracing::warn!("reaping goal orphan pid {pid} (session {session})");
        orphan::kill_group(*pid as u32);
        reaped.insert(*pid);
    }
    let deadline = tokio::time::Instant::now() + ORPHAN_EXIT_WAIT;
    while reaped
        .iter()
        .any(|pid| orphan::cmdline(*pid as u32).is_some())
    {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("a reaped goal orphan was still running after {ORPHAN_EXIT_WAIT:?}");
            break;
        }
        tokio::time::sleep(ORPHAN_POLL).await;
    }
    reaped
}

#[cfg(unix)]
fn is_agent_socket(path: &std::path::Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

// Windows agents are named pipes; there is nothing to stat.
#[cfg(not(unix))]
fn is_agent_socket(_: &std::path::Path) -> bool {
    true
}

#[cfg(any(test, feature = "test-support"))]
impl HyprlayerRuntime {
    #[doc(hidden)]
    pub async fn install_opencode_child_for_test(&self, child: crate::process::SupervisedChild) {
        self.opencode.install_child_for_test(child).await;
    }

    /// Test-only access to the terminal registry, for asserting that
    /// `WorkstreamService::archive` stops live terminals before moving the
    /// worktree. Not part of the supported facade; production callers use
    /// `HyprlayerRuntime::terminals()` instead.
    #[doc(hidden)]
    pub fn test_terminals(&self) -> Arc<TerminalRegistry> {
        self.terminals.clone()
    }

    /// Test-only raw ingress into a session's actor, for forcing an
    /// attachment past its lag threshold without a real harness process. Not
    /// part of the supported facade.
    #[doc(hidden)]
    pub async fn test_session_ingress(
        &self,
        session_id: &str,
    ) -> tokio::sync::mpsc::Sender<crate::claude::SessionEvent> {
        self.session_actors.ingress(session_id).await
    }

    /// Test-only database access, for seeding a session row directly rather
    /// than paying for a real git-backed workstream. Not part of the
    /// supported facade.
    #[doc(hidden)]
    pub fn test_db(&self) -> Arc<Db> {
        self.db.clone()
    }
}

/// Which binary and version each tool resolved to: a daemon sees a different
/// `PATH` than the user's shell, the usual cause of "works in my terminal".
async fn log_binaries(binaries: &BinaryResolver) {
    for status in binaries.status_all().await {
        tracing::info!(
            "binary {}: {:?} at {} ({}){}",
            status.name,
            status.state,
            status.path.as_deref().unwrap_or("nowhere"),
            status.version.as_deref().unwrap_or("no version"),
            if status.overridden {
                ", overridden"
            } else {
                ""
            }
        );
    }
    tracing::debug!(
        "search PATH: {}",
        binaries.augmented_path().await.to_string_lossy()
    );
}
