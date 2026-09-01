#![cfg(feature = "test-support")]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::error::Result;
use hyprlayer_core::hyprcli::StorageInfo;
use hyprlayer_core::models::RuntimeEvent;
use hyprlayer_core::process::binaries::BinaryResolver;
#[cfg(unix)]
use hyprlayer_core::process::SupervisedChild;
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::runtime::ports::ConfigCli;
use hyprlayer_core::store::Db;
use hyprlayer_core::CoreError;

fn test_environment(home: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment {
        home_dir: home.to_path_buf(),
        process_cwd: home.to_path_buf(),
        process_env: std::collections::BTreeMap::new(),
    }
}

fn test_paths(root: &Path) -> RuntimePaths {
    RuntimePaths {
        hyprlayer_config_file: root.join("hyprlayer-config.json"),
        app_config_dir: root.join("app-config"),
        app_data_dir: root.join("app-data"),
        default_worktree_root: root.join("worktrees"),
        default_repos_root: root.join("repos"),
        opencode_config_file: root.join("opencode.json"),
        opencode_auth_file: root.join("opencode-auth.json"),
        opencode_data_dir: root.join("opencode-data"),
        claude_config_dir: root.join("claude"),
        claude_managed_settings_files: Vec::new(),
        codex_home: root.join("codex"),
        agent_roots: Vec::new(),
        download_dir: root.join("downloads"),
    }
}

fn valid_v4_config(thoughts_repo: &Path) -> String {
    format!(
        r#"{{
            "version": 4,
            "thoughts": {{
                "user": "tester",
                "backend": {{
                    "kind": "git",
                    "thoughtsRepo": {thoughts_repo:?},
                    "reposDir": "repos",
                    "globalDir": "global"
                }}
            }}
        }}"#
    )
}

// The exact function `HyprlayerRuntime::open` calls to build its search path —
// exercised directly against `BinaryResolver` rather than through a live,
// fully-opened `HyprlayerRuntime`, which this narrow test has no need to pay
// the setup cost of building.
#[tokio::test]
async fn binary_resolver_derives_its_search_path_from_the_supplied_home_not_a_fixed_ambient_value()
{
    let home_a = tempfile::tempdir().unwrap();
    let home_b = tempfile::tempdir().unwrap();

    let path_a = BinaryResolver::initialize(&test_environment(home_a.path()), Default::default())
        .augmented_path()
        .await;
    let path_b = BinaryResolver::initialize(&test_environment(home_b.path()), Default::default())
        .augmented_path()
        .await;

    let a = path_a.to_string_lossy().into_owned();
    let b = path_b.to_string_lossy().into_owned();
    assert!(a.contains(home_a.path().to_str().unwrap()));
    assert!(b.contains(home_b.path().to_str().unwrap()));
    assert!(!a.contains(home_b.path().to_str().unwrap()));
    assert!(!b.contains(home_a.path().to_str().unwrap()));
}

// `HyprlayerRuntime::open` must touch only the supplied paths — everything it
// creates lands under `app_data_dir`/`app_config_dir`, and a decoy "wrong home"
// the environment does not name is left untouched.
#[tokio::test]
async fn open_writes_only_under_the_supplied_paths() {
    let root = tempfile::tempdir().unwrap();
    let decoy_home = tempfile::tempdir().unwrap();
    std::fs::write(decoy_home.path().join("canary"), b"untouched").unwrap();

    let paths = test_paths(root.path());
    let environment = test_environment(&decoy_home.path().join("not-the-real-home"));
    let runtime = RuntimeBuilder::new(environment, paths.clone())
        .open()
        .await
        .expect("hermetic open");

    assert!(paths.app_data_dir.join("hyprlayer.db").is_file());
    assert!(paths.app_data_dir.join(".runtime.lock").is_file());

    let canary = std::fs::read(decoy_home.path().join("canary")).unwrap();
    assert_eq!(canary, b"untouched");
    assert!(
        !decoy_home.path().join("not-the-real-home").exists(),
        "open() must not materialize a home directory it never reads or writes to"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_second_open_on_the_same_profile_is_rejected_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    let runtime = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("first open succeeds");

    let second = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await;
    let Err(err) = second else {
        panic!("a second runtime must not attach to a locked profile");
    };
    assert!(matches!(err, CoreError::RuntimeInUse { .. }));
    assert_eq!(err.code(), "runtime_in_use");

    runtime.shutdown().await;
    // `shutdown` tears down background work but the profile lock is tied to
    // the runtime's own lifetime (held for as long as the `Arc` is), not to
    // the shutdown state machine — dropping it is what releases the file.
    drop(runtime);

    let reopened = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("the profile is free on the first attempt once its holder is dropped");
    reopened.shutdown().await;
}

/// The handoff above never calls `start()`, so it says nothing about the case an
/// embedder actually hits. Startup work runs on a task that outlives the call,
/// and the login-shell probe it begins with carries an eight-second timeout — so
/// a strong reference held there kept the profile locked long after its owner let
/// go. The slow shell makes that window deterministic rather than a race.
#[cfg(unix)]
#[tokio::test]
async fn a_started_runtime_frees_its_profile_as_soon_as_it_is_dropped() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    let shell = home.path().join("slow-login-shell");
    std::fs::write(&shell, "#!/bin/sh\nsleep 3\n").unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut environment = test_environment(home.path());
    environment.process_env.insert(
        std::ffi::OsString::from("SHELL"),
        std::ffi::OsString::from(&shell),
    );

    let runtime = RuntimeBuilder::new(environment, test_paths(root.path()))
        .open()
        .await
        .expect("open");
    runtime.start().await.expect("start");

    runtime.shutdown().await;
    drop(runtime);

    let started = std::time::Instant::now();
    let reopened = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("a started runtime frees its profile on drop, without waiting for startup work");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "reopen waited {:?} — the profile outlived its runtime",
        started.elapsed()
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn prior_orphan_pids_are_cleared_on_open() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());

    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    {
        let seed = Db::open(&paths.app_data_dir.join("hyprlayer.db")).unwrap();
        seed.record_orphan(999_999, "sess-orphan").unwrap();
        assert_eq!(seed.list_orphans().unwrap().len(), 1);
    }

    let runtime = RuntimeBuilder::new(test_environment(home.path()), paths.clone())
        .open()
        .await
        .expect("open");
    runtime.shutdown().await;

    let verify = Db::open(&paths.app_data_dir.join("hyprlayer.db")).unwrap();
    assert!(
        verify.list_orphans().unwrap().is_empty(),
        "open() must clear orphan PIDs left by a prior run"
    );
}

struct FixedConfigCli {
    result: fn() -> Result<StorageInfo>,
}

impl ConfigCli for FixedConfigCli {
    fn storage_info<'a>(
        &'a self,
        _repo_path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<StorageInfo>> + Send + 'a>> {
        Box::pin(async move { (self.result)() })
    }
}

fn failing_storage_info() -> Result<StorageInfo> {
    Err(CoreError::BinaryMissing {
        name: "hyprlayer".into(),
    })
}

#[tokio::test]
async fn config_migration_is_a_noop_when_the_cli_is_absent_or_fails() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());

    std::fs::create_dir_all(paths.hyprlayer_config_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.hyprlayer_config_file, r#"{"version": 2}"#).unwrap();

    let runtime = RuntimeBuilder::new(test_environment(home.path()), paths.clone())
        .with_config_cli(Arc::new(FixedConfigCli {
            result: failing_storage_info,
        }))
        .open()
        .await
        .expect("open");

    let changed = runtime.migrate_config_if_needed().await;
    assert!(
        !changed,
        "a failing CLI must not be reported as a migration"
    );

    let raw = std::fs::read_to_string(&paths.hyprlayer_config_file).unwrap();
    assert!(
        raw.contains("\"version\": 2"),
        "the config is left untouched"
    );

    runtime.shutdown().await;
}

fn migrated_v4_body() -> Result<StorageInfo> {
    Ok(StorageInfo {
        backend: "git".into(),
        settings: Default::default(),
        project_path: String::new(),
        mapped_name: None,
        profile: None,
        user: Some("tester".into()),
        schema: Vec::new(),
    })
}

#[tokio::test]
async fn config_migration_reloads_once_the_cli_rewrites_the_file() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    let thoughts_dir = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(paths.hyprlayer_config_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.hyprlayer_config_file, r#"{"version": 2}"#).unwrap();

    struct RewritingConfigCli {
        config_path: PathBuf,
        body: String,
    }
    impl ConfigCli for RewritingConfigCli {
        fn storage_info<'a>(
            &'a self,
            _repo_path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<StorageInfo>> + Send + 'a>> {
            Box::pin(async move {
                std::fs::write(&self.config_path, &self.body).unwrap();
                migrated_v4_body()
            })
        }
    }

    let runtime = RuntimeBuilder::new(test_environment(home.path()), paths.clone())
        .with_config_cli(Arc::new(RewritingConfigCli {
            config_path: paths.hyprlayer_config_file.clone(),
            body: valid_v4_config(thoughts_dir.path()),
        }))
        .open()
        .await
        .expect("open");

    let changed = runtime.migrate_config_if_needed().await;
    assert!(changed, "a successful CLI probe is reported as a migration");

    runtime.shutdown().await;
}

#[tokio::test]
async fn config_and_index_changes_are_mapped_to_runtime_events() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    let thoughts_dir = tempfile::tempdir().unwrap();
    std::fs::write(thoughts_dir.path().join("note.md"), "# hello\n").unwrap();

    std::fs::create_dir_all(paths.hyprlayer_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.hyprlayer_config_file,
        valid_v4_config(thoughts_dir.path()),
    )
    .unwrap();

    let runtime = RuntimeBuilder::new(test_environment(home.path()), paths.clone())
        .open()
        .await
        .expect("open");
    let mut events = runtime.subscribe();
    runtime.start().await.expect("start");

    let saw_index_update = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::ThoughtsIndexUpdated { .. }) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_index_update,
        "starting the runtime with a configured thoughts root must rescan and announce it"
    );

    // Touching the config file (as the CLI's own writer would) is picked up by
    // the runtime's poll loop and re-announced as a config change.
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(
        &paths.hyprlayer_config_file,
        valid_v4_config(thoughts_dir.path()),
    )
    .unwrap();

    let saw_config_change = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::ConfigChanged) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_config_change,
        "the background poll loop must notice the on-disk change and re-announce it"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn foreground_toggles_the_pr_watcher_without_touching_binaries() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());

    let runtime = RuntimeBuilder::new(test_environment(home.path()), paths)
        .open()
        .await
        .expect("open");
    runtime.start().await.expect("start");

    // No panic, no deadlock, from either direction of the transition; the pr
    // watcher's own cadence tests (`runtime::pr_watcher::tests`) cover the
    // duration math this flag feeds.
    runtime.set_foreground(false);
    tokio::time::sleep(Duration::from_millis(20)).await;
    runtime.set_foreground(true);
    tokio::time::sleep(Duration::from_millis(20)).await;

    runtime.shutdown().await;
}

#[tokio::test]
async fn shutdown_is_idempotent_under_repeated_calls() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("open");
    runtime.start().await.expect("start");

    let first = runtime.shutdown().await;
    let second = runtime.shutdown().await;
    let third = runtime.shutdown().await;

    assert_eq!(first.forced_process_count, second.forced_process_count);
    assert_eq!(second.forced_process_count, third.forced_process_count);
}

#[tokio::test]
async fn concurrent_shutdown_callers_all_observe_the_same_report() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("open");
    runtime.start().await.expect("start");

    let (a, b, c) = tokio::join!(runtime.shutdown(), runtime.shutdown(), runtime.shutdown());
    assert_eq!(a.forced_process_count, b.forced_process_count);
    assert_eq!(b.forced_process_count, c.forced_process_count);
}

struct CapturingLogger {
    records: &'static std::sync::Mutex<Vec<String>>,
}

impl log::Log for CapturingLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        self.records.lock().unwrap().push(record.args().to_string());
    }

    fn flush(&self) {}
}

fn captured_log_records() -> &'static std::sync::Mutex<Vec<String>> {
    static RECORDS: std::sync::OnceLock<&'static std::sync::Mutex<Vec<String>>> =
        std::sync::OnceLock::new();
    RECORDS.get_or_init(|| {
        let records: &'static std::sync::Mutex<Vec<String>> =
            Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
        let _ = log::set_boxed_logger(Box::new(CapturingLogger { records }));
        log::set_max_level(log::LevelFilter::Trace);
        records
    })
}

#[tokio::test]
async fn core_tracing_events_reach_the_log_facade_with_no_subscriber_installed() {
    let records = captured_log_records();

    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(paths.hyprlayer_config_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.hyprlayer_config_file, r#"{"version": 2}"#).unwrap();

    let runtime = RuntimeBuilder::new(test_environment(home.path()), paths.clone())
        .with_config_cli(Arc::new(FixedConfigCli {
            result: failing_storage_info,
        }))
        .open()
        .await
        .expect("open");

    runtime.migrate_config_if_needed().await;
    runtime.shutdown().await;

    assert!(
        records
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("config migration nudge failed")),
        "a core tracing::warn! must reach the log facade with no tracing subscriber installed"
    );
}

#[tokio::test]
async fn start_is_one_shot() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("open");

    runtime.start().await.expect("first start succeeds");
    let second = runtime.start().await;
    assert!(matches!(second, Err(CoreError::RuntimeAlreadyStarted)));

    runtime.shutdown().await;
}

// Stands in for `opencode serve`: its own process group, so the manager's
// `kill(-pid)` reaches it exactly as it reaches the real server, and a piped
// stdout nobody writes to, so the pipe closing is proof the process is gone.
#[cfg(unix)]
fn sleeping_opencode_child() -> (SupervisedChild, tokio::process::ChildStdout) {
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.args(["-c", "sleep 120"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    let mut child = hyprlayer_core::process::spawn_supervised(&mut cmd).expect("spawn");
    let stdout = child.take_stdout().expect("stdout is piped");
    (child, stdout)
}

#[cfg(unix)]
async fn exited_within(stdout: &mut tokio::process::ChildStdout, budget: Duration) -> bool {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 1];
    matches!(
        tokio::time::timeout(budget, stdout.read(&mut buf)).await,
        Ok(Ok(0))
    )
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_tears_down_the_runtime_owned_opencode_server() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("open");
    runtime.start().await.expect("start");

    let (child, mut stdout) = sleeping_opencode_child();
    runtime.install_opencode_child_for_test(child).await;
    assert!(
        !exited_within(&mut stdout, Duration::from_millis(200)).await,
        "the stand-in server should still be running before shutdown"
    );

    runtime.shutdown().await;

    assert!(
        exited_within(&mut stdout, Duration::from_secs(5)).await,
        "an OpenCode server must not survive HyprlayerRuntime::shutdown"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_now_tears_down_the_runtime_owned_opencode_server() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::new(test_environment(home.path()), test_paths(root.path()))
        .open()
        .await
        .expect("open");
    runtime.start().await.expect("start");

    let (child, mut stdout) = sleeping_opencode_child();
    runtime.install_opencode_child_for_test(child).await;

    runtime.shutdown_now();

    assert!(
        exited_within(&mut stdout, Duration::from_secs(5)).await,
        "an OpenCode server must not survive the synchronous exit path"
    );

    runtime.shutdown().await;
}
