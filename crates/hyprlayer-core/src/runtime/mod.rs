pub mod agents;
pub mod attachments;
pub mod builder;
pub mod library;
pub mod migrations;
mod operation_locks;
pub mod ports;
pub mod pr_watcher;
mod profile_lock;
pub mod projects;
pub mod prompts;
pub mod providers;
pub mod script_registry;
pub mod scripts;
pub mod sessions;
pub mod settings_service;
pub mod shutdown;
pub mod terminal_registry;
pub mod terminals;
pub mod workstreams;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};

use hyprlayer_models::runtime::ThoughtsIndexUpdateKind;
use hyprlayer_models::RuntimeEvent;

pub use crate::env::{RuntimeEnvironment, RuntimePaths};

use crate::config::ConfigService;
use crate::error::Result;
use crate::index::watcher::WatchHandle;
use crate::index::ThoughtsIndex;
use crate::opencode::runtime::OpenCodeManager;
use crate::process::binaries::BinaryResolver;
use crate::settings::SettingsStore;
use crate::store::Db;
use crate::worktree::WorktreeManager;

use operation_locks::WorkstreamLocks;
use ports::{
    CliConfigCli, ConfigCli, CredentialLookup, LifecycleRunner, NoCredentialLookup, ProviderHttp,
    ReqwestProviderHttp,
};
use pr_watcher::{PrWatcherContext, WorktreeWatch};
use profile_lock::ProfileLock;
use script_registry::ScriptRegistry;
use sessions::registry::SessionRegistry;
use sessions::SessionActors;
use shutdown::{ShutdownCoordinator, ShutdownReport, TrackedTask};
use terminal_registry::TerminalRegistry;

const EVENT_CAP: usize = 256;
const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROVIDER_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

pub struct RuntimeOptions {
    pub environment: RuntimeEnvironment,
    pub paths: RuntimePaths,
    pub credential_lookup: Arc<dyn CredentialLookup + Send + Sync>,
}

impl RuntimeOptions {
    pub fn new(environment: RuntimeEnvironment, paths: RuntimePaths) -> Self {
        Self {
            environment,
            paths,
            credential_lookup: Arc::new(NoCredentialLookup),
        }
    }

    pub fn with_credential_lookup(
        mut self,
        credential_lookup: Arc<dyn CredentialLookup + Send + Sync>,
    ) -> Self {
        self.credential_lookup = credential_lookup;
        self
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
    terminals: Arc<TerminalRegistry>,
    scripts: Arc<ScriptRegistry>,
    opencode: Arc<OpenCodeManager>,
    watchers: Watchers,
    focused: Arc<AtomicBool>,
    focus_changed: Arc<Notify>,
    events: broadcast::Sender<RuntimeEvent>,
    config_cli: Arc<dyn ConfigCli>,
    credential_lookup: Arc<dyn CredentialLookup + Send + Sync>,
    provider_http: Arc<dyn ProviderHttp + Send + Sync>,
    lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
    workstream_locks: WorkstreamLocks,
    coordinator: ShutdownCoordinator,
    tasks: AsyncMutex<Vec<TrackedTask>>,
    _profile_lock: ProfileLock,
}

impl HyprlayerRuntime {
    pub async fn open(options: RuntimeOptions) -> Result<Arc<Self>> {
        Self::open_with(options, None, None, None).await
    }

    pub(crate) async fn open_with(
        options: RuntimeOptions,
        config_cli: Option<Arc<dyn ConfigCli>>,
        provider_http: Option<Arc<dyn ProviderHttp + Send + Sync>>,
        lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
    ) -> Result<Arc<Self>> {
        let RuntimeOptions {
            environment,
            paths,
            credential_lookup,
        } = options;

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
        let db_path = paths.app_data_dir.join("hyprlayer.db");
        let db = Arc::new(Db::open(&db_path)?);

        if let Ok(orphans) = db.list_orphans() {
            if !orphans.is_empty() {
                tracing::warn!(
                    "found {} orphan session PID(s) from a prior run: {:?}",
                    orphans.len(),
                    orphans
                );
                let _ = db.clear_all_orphans();
            }
        }

        let settings_snapshot = settings.get().await;
        let worktree_root = settings_snapshot
            .worktree_root
            .as_deref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| paths.default_worktree_root.clone());

        let (events, _) = broadcast::channel(EVENT_CAP);
        let session_actors = Arc::new(SessionActors::new(db.clone(), events.clone()));

        let config_cli =
            config_cli.unwrap_or_else(|| Arc::new(CliConfigCli::new(binaries.clone())));
        let provider_http = provider_http
            .unwrap_or_else(|| Arc::new(ReqwestProviderHttp::new(PROVIDER_HTTP_TIMEOUT)));

        Ok(Arc::new(Self {
            config_cli,
            credential_lookup,
            provider_http,
            lifecycle_runner,
            workstream_locks: WorkstreamLocks::new(),
            config,
            binaries,
            settings,
            db,
            worktrees: Arc::new(WorktreeManager::new(worktree_root)),
            index: Arc::new(ThoughtsIndex::new()),
            session_registry: Arc::new(SessionRegistry::new()),
            session_actors,
            terminals: Arc::new(TerminalRegistry::new()),
            scripts: Arc::new(ScriptRegistry::new()),
            opencode: Arc::new(OpenCodeManager::new()),
            watchers: Watchers {
                index_watch: AsyncMutex::new(None),
                worktree_watch: Arc::new(AsyncMutex::new(None)),
            },
            focused: Arc::new(AtomicBool::new(true)),
            focus_changed: Arc::new(Notify::new()),
            events,
            environment,
            paths,
            coordinator: ShutdownCoordinator::new(),
            tasks: AsyncMutex::new(Vec::new()),
            _profile_lock: profile_lock,
        }))
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        self.coordinator.try_start()?;

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
                let Some(runtime) = startup.upgrade() else {
                    return;
                };
                if runtime.migrate_config_if_needed().await {
                    let _ = startup_events.send(RuntimeEvent::ConfigChanged);
                }
                runtime.rebuild_thoughts_index().await;
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
        });

        self.tasks
            .lock()
            .await
            .extend([startup_task, config_task, pr_watcher_task]);

        Ok(())
    }

    pub fn set_foreground(&self, foreground: bool) {
        self.focused.store(foreground, Ordering::Relaxed);
        self.focus_changed.notify_waiters();
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

                self.terminals.kill_all_blocking();
                self.session_registry.kill_all_blocking();
                self.opencode.shutdown().await;

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
            Err(_) => Vec::new(),
        };

        let index = self.index.clone();
        let scan_roots = roots.clone();
        let stats = tokio::task::spawn_blocking(move || index.full_scan(&scan_roots)).await;
        if let Ok(stats) = &stats {
            tracing::info!(
                "thoughts index: {} docs ({} skipped, {} errors) across {} roots",
                stats.indexed,
                stats.skipped,
                stats.errors,
                roots.len()
            );
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
}
