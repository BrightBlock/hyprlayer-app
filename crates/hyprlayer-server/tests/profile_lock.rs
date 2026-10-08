//! A real second `hyprlayer-server` on a held profile. An integration test, so cargo builds
//! the binary before any test runs: a nested build would re-link it under parallel tests.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use hyprlayer_core::{HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths};

// The same hermetic shape as crates/hyprlayer-core/tests/headless_runtime.rs:
// an unspawnable SHELL keeps the login-shell probe off the developer's rc files.
fn environment(home: &Path) -> RuntimeEnvironment {
    let mut process_env = BTreeMap::new();
    process_env.insert(
        OsString::from("SHELL"),
        OsString::from(home.join("no-such-shell")),
    );
    RuntimeEnvironment {
        home_dir: home.to_path_buf(),
        process_cwd: home.to_path_buf(),
        process_env,
        env_overlay: Default::default(),
    }
}

fn paths(root: &Path) -> RuntimePaths {
    RuntimePaths {
        hyprlayer_config_file: root.join("config/hyprlayer.json"),
        app_config_dir: root.join("app-config"),
        app_data_dir: root.join("app-data"),
        default_worktree_root: root.join("worktrees"),
        default_repos_root: root.join("repos"),
        opencode_config_file: root.join("opencode/opencode.json"),
        opencode_auth_file: root.join("opencode/auth.json"),
        opencode_data_dir: root.join("opencode/data"),
        claude_config_dir: root.join("claude"),
        claude_managed_settings_files: Vec::new(),
        codex_home: root.join("codex"),
        agent_roots: Vec::new(),
        download_dir: root.join("downloads"),
    }
}

async fn open_runtime(root: &Path, home: &Path) -> Arc<HyprlayerRuntime> {
    let paths = paths(root);
    for dir in [
        paths.hyprlayer_config_file.parent().unwrap(),
        &paths.app_config_dir,
        &paths.app_data_dir,
        paths.opencode_config_file.parent().unwrap(),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&paths.hyprlayer_config_file, r#"{"version": 4}"#).unwrap();
    std::fs::write(paths.app_config_dir.join("settings.json"), "{}").unwrap();
    std::fs::write(&paths.opencode_config_file, "{}").unwrap();
    std::fs::write(&paths.opencode_auth_file, "{}").unwrap();
    let options = RuntimeOptions::new(environment(home), paths);
    let runtime = HyprlayerRuntime::open(options).await.expect("open");
    runtime.start().await.expect("start");
    runtime
}

#[tokio::test]
async fn a_second_server_process_on_the_same_profile_exits_with_runtime_in_use() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let runtime = open_runtime(root.path(), &home).await;

    // The log dir as well: the shell may export the release profile's.
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_hyprlayer-server"))
        .env("HOME", &home)
        .env("HYPRLAYER_APP_CONFIG_DIR", root.path().join("app-config"))
        .env("HYPRLAYER_APP_DATA_DIR", root.path().join("app-data"))
        .env("HYPRLAYER_APP_LOG_DIR", root.path().join("logs"))
        .kill_on_drop(true)
        .output()
        .await
        .expect("spawn a second hyprlayer-server");

    assert!(
        !output.status.success(),
        "a second server on the same profile must not start"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("runtime_in_use"),
        "expected runtime_in_use on stderr, got: {stderr}"
    );

    runtime.shutdown().await;
}
