#![cfg(feature = "test-support")]

use std::path::{Path, PathBuf};

use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::CoreError;

fn test_environment(home: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment {
        home_dir: home.to_path_buf(),
        process_cwd: home.to_path_buf(),
        process_env: std::collections::BTreeMap::new(),
        env_overlay: Default::default(),
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

// --- Change group 2: the tracked OpenCode migration ledger ---

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/compat/runtime-migrations")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

fn ledger_path(paths: &RuntimePaths) -> PathBuf {
    paths.app_data_dir.join("runtime-migrations.json")
}

#[tokio::test]
async fn open_runs_the_migration_against_the_supplied_opencode_config_only() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(paths.opencode_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.opencode_config_file,
        r#"{"model":"github-copilot/gpt-4"}"#,
    )
    .unwrap();

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&paths.opencode_config_file).unwrap())
            .unwrap();
    assert!(
        config.get("model").is_none(),
        "the retired default must be cleared"
    );

    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ledger_path(&paths)).unwrap()).unwrap();
    assert_eq!(ledger["version"], 1);
    assert!(ledger["completed"]["opencode_remove_github_copilot_default_v1"].is_object());

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_malformed_ledger_fails_open_without_touching_the_config() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(ledger_path(&paths), fixture("malformed.json")).unwrap();
    std::fs::create_dir_all(paths.opencode_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.opencode_config_file,
        r#"{"model":"github-copilot/gpt-4"}"#,
    )
    .unwrap();

    let result = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await;
    let Err(err) = result else {
        panic!("a malformed ledger must fail open");
    };
    assert!(matches!(
        err,
        CoreError::RuntimeMigrationLedgerInvalid { .. }
    ));

    let config = std::fs::read_to_string(&paths.opencode_config_file).unwrap();
    assert!(
        config.contains("github-copilot"),
        "must not mutate the config"
    );
}

#[tokio::test]
async fn a_too_new_ledger_fails_open_without_touching_the_config() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(ledger_path(&paths), fixture("version-too-new.json")).unwrap();
    std::fs::create_dir_all(paths.opencode_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.opencode_config_file,
        r#"{"model":"github-copilot/gpt-4"}"#,
    )
    .unwrap();

    let result = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await;
    let Err(err) = result else {
        panic!("a too-new ledger must fail open");
    };
    assert!(matches!(
        err,
        CoreError::RuntimeMigrationLedgerTooNew {
            found: 2,
            max: 1,
            ..
        }
    ));

    let config = std::fs::read_to_string(&paths.opencode_config_file).unwrap();
    assert!(
        config.contains("github-copilot"),
        "must not mutate the config"
    );
}

#[tokio::test]
async fn unknown_migration_ids_in_the_ledger_are_preserved_across_an_open() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(ledger_path(&paths), fixture("unknown-id.json")).unwrap();

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();

    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ledger_path(&paths)).unwrap()).unwrap();
    assert_eq!(
        ledger["completed"]["some_future_migration_v7"]["completedAt"],
        1_700_000_000_000.0_f64
    );
    assert!(ledger["completed"]["opencode_remove_github_copilot_default_v1"].is_object());

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_concurrent_open_on_the_same_profile_is_rejected_and_the_ledger_is_untouched() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());

    let first = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();
    let ledger_after_first_open = std::fs::read_to_string(ledger_path(&paths)).unwrap();

    let result = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await;
    let Err(err) = result else {
        panic!("a concurrent open on the same profile must be rejected");
    };
    assert!(matches!(err, CoreError::RuntimeInUse { .. }));
    assert_eq!(
        std::fs::read_to_string(ledger_path(&paths)).unwrap(),
        ledger_after_first_open,
        "a rejected second open must not touch the migration ledger at all"
    );

    first.shutdown().await;
}
