//! The supported facade, used the way a headless Rust consumer would: nothing
//! but `hyprlayer_core`'s crate-root exports, no `test-support`, no
//! `RuntimeBuilder`, no injected ports, no user home.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::runtime::projects::ImportFolderRequest;
use hyprlayer_core::{
    HyprlayerRuntime, NoCredentialLookup, RuntimeEnvironment, RuntimeEvent, RuntimeOptions,
    RuntimePaths,
};

/// The login-shell PATH probe `start()` runs takes its shell from
/// `RuntimeEnvironment`, so naming one that cannot spawn is what keeps this
/// test off the developer's own rc files — the exact ambient side effect the
/// no-file-outside-the-supplied-paths assertion below is about.
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

/// A schema-current config with no thoughts backend. Current matters: an older
/// one parses as `config_needs_migration`, which is the one condition under
/// which `start()` shells the hyprlayer CLI. `HyprCli::command` sets only
/// `PATH` on its children, so a CLI child would inherit this process's ambient
/// environment rather than `RuntimeEnvironment::process_env` — hermeticity
/// constrains core's own reads, never what a child inherits (ADR-0008). Keeping
/// the config current keeps that path unreached, so the assertion below is
/// about core's own writes and nothing else.
fn write_inputs(paths: &RuntimePaths) {
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
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn a_headless_consumer_can_open_import_observe_and_shut_down() {
    let root = tempfile::tempdir().unwrap();
    let decoy_home = tempfile::tempdir().unwrap();
    std::fs::write(decoy_home.path().join("canary"), b"untouched").unwrap();
    let decoy_before = entries(decoy_home.path());

    let paths = paths(root.path());
    write_inputs(&paths);

    let options = RuntimeOptions::new(
        environment(&decoy_home.path().join("not-the-real-home")),
        paths.clone(),
    )
    .with_credential_lookup(Arc::new(NoCredentialLookup));

    let runtime = HyprlayerRuntime::open(options).await.expect("open");
    let mut events = runtime.subscribe();
    runtime.start().await.expect("start");

    let folder = root.path().join("projects/notes");
    std::fs::create_dir_all(&folder).unwrap();
    let registered = runtime
        .projects()
        .import_folder(ImportFolderRequest {
            path: folder.display().to_string(),
            directory_name: None,
        })
        .await
        .expect("import a plain folder project")
        .path;

    let announced = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::ProjectsChanged) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        announced,
        "importing a project must announce RuntimeEvent::ProjectsChanged"
    );

    let read_back = runtime
        .projects()
        .find(&registered)
        .await
        .expect("registry read")
        .expect("the imported project is registered");
    assert_eq!(read_back.path, registered);
    assert!(read_back.exists_on_disk);

    let (first, second) = tokio::join!(runtime.shutdown(), runtime.shutdown());
    assert_eq!(first.forced_process_count, second.forced_process_count);
    assert_eq!(first.task_failures, second.task_failures);

    assert!(paths.app_data_dir.join("hyprlayer.db").is_file());
    assert_eq!(
        std::fs::read(decoy_home.path().join("canary")).unwrap(),
        b"untouched"
    );
    assert_eq!(
        entries(decoy_home.path()),
        decoy_before,
        "the runtime wrote outside the paths it was given"
    );
}
