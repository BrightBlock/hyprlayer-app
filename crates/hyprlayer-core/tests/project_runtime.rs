#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::runtime::projects::{
    CloneProjectRequest, ImportFolderRequest, ImportRepositoryRequest,
};
use hyprlayer_core::store::ProjectSource;
use hyprlayer_core::CoreError;
use hyprlayer_models::RuntimeEvent;

fn test_environment(home: &Path) -> RuntimeEnvironment {
    let mut process_env = BTreeMap::new();
    if let Some(path) = std::env::var_os("PATH") {
        process_env.insert(OsString::from("PATH"), path);
    }
    RuntimeEnvironment {
        home_dir: home.to_path_buf(),
        process_cwd: home.to_path_buf(),
        process_env,
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

// A fake `hyprlayer` binary, standing in for `thoughts init`/`thoughts uninit`:
// its whole job is to succeed or fail on cue so rollback/registration behavior
// is deterministic regardless of whether the real CLI is installed on the
// machine running the test. `BinaryResolver` requires a `--version` probe to
// succeed with a parsable semver before it will resolve `hyprlayer` at all,
// so the script answers that specially and defers everything else to `exit_code`.
fn write_fake_hyprlayer(root: &Path, exit_code: i32) -> PathBuf {
    let script = root.join(format!("fake-hyprlayer-{exit_code}.sh"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"hyprlayer 9.9.9\"\n  exit 0\nfi\nexit {exit_code}\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    script
}

fn write_hyprlayer_override(paths: &RuntimePaths, bin: &Path) {
    std::fs::create_dir_all(&paths.app_config_dir).unwrap();
    let body = serde_json::json!({
        "binaryOverrides": { "hyprlayer": bin.display().to_string() }
    });
    std::fs::write(
        paths.app_config_dir.join("settings.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

fn sh(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-c")
        .arg("commit.gpgsign=false")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ADR-0005: filesystem/Git tests use real temporary repositories, not a port.
fn fixture_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    sh(&repo, &["add", "."]);
    sh(&repo, &["commit", "-m", "init"]);
    repo
}

async fn wait_for_event(
    rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>,
    mut matches: impl FnMut(&RuntimeEvent) -> bool,
) -> bool {
    let deadline = tokio::time::sleep(Duration::from_secs(2));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return false,
            event = rx.recv() => match event {
                Ok(e) if matches(&e) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            },
        }
    }
}

#[tokio::test]
async fn clone_rollback_leaves_no_db_row_and_no_directory() {
    let root = tempfile::tempdir().unwrap();
    let origin = fixture_repo(root.path(), "origin");
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 1));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();

    let result = runtime
        .projects()
        .clone(CloneProjectRequest {
            url: origin.display().to_string(),
            location: None,
            name: Some("cloned".into()),
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await;

    assert!(
        result.is_err(),
        "a failed thoughts-init must fail the clone"
    );
    let dest = paths.default_repos_root.join("cloned");
    assert!(
        !dest.exists(),
        "the clone destination must be rolled back on failure"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn import_rollback_removes_the_git_init_it_created_for_a_new_folder() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 1));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    let folder = root.path().join("plain-folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "hi\n").unwrap();

    let result = runtime
        .projects()
        .import_repository(ImportRepositoryRequest {
            path: folder.display().to_string(),
            branch: None,
            directory_name: None,
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await;

    assert!(
        result.is_err(),
        "a failed thoughts-init must fail the import"
    );
    assert!(
        folder.exists(),
        "the folder itself predates the call and must survive"
    );
    assert!(
        !folder.join(".git").exists(),
        "the .git this call created must be rolled back"
    );
    let canonical = std::fs::canonicalize(&folder).unwrap_or(folder);
    assert!(runtime
        .projects()
        .find(&canonical.display().to_string())
        .await
        .unwrap()
        .is_none());

    runtime.shutdown().await;
}

#[tokio::test]
async fn import_refuses_a_repository_git2_cannot_open_and_leaves_it_intact() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 1));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    // Shaped like a blobless clone: format v1 naming an extension libgit2 lacks.
    let repo = fixture_repo(root.path(), "unsupported-repo");
    sh(
        &repo,
        &["config", "extensions.hyprlayerunsupported", "true"],
    );
    sh(&repo, &["config", "core.repositoryformatversion", "1"]);
    let head = std::fs::read_to_string(repo.join(".git/HEAD")).unwrap();

    let err = runtime
        .projects()
        .import_repository(ImportRepositoryRequest {
            path: repo.display().to_string(),
            branch: None,
            directory_name: None,
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await
        .expect_err("an unopenable repository must not import");

    assert_eq!(err.code(), "repo_unreadable");
    assert_eq!(
        std::fs::read_to_string(repo.join(".git/HEAD")).unwrap(),
        head,
        "the repository must be left exactly as it was"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn import_rollback_never_deletes_a_pre_existing_repository() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 1));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    let repo = fixture_repo(root.path(), "existing-repo");

    let result = runtime
        .projects()
        .import_repository(ImportRepositoryRequest {
            path: repo.display().to_string(),
            branch: None,
            directory_name: None,
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await;

    assert!(
        result.is_err(),
        "a failed thoughts-init must fail the import"
    );
    assert!(
        repo.exists() && repo.join(".git").exists(),
        "a pre-existing repository must never be deleted on failure"
    );
    let canonical = std::fs::canonicalize(&repo).unwrap();
    assert!(runtime
        .projects()
        .find(&canonical.display().to_string())
        .await
        .unwrap()
        .is_none());

    runtime.shutdown().await;
}

#[tokio::test]
async fn folder_import_emits_projects_changed() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();
    let mut events = runtime.subscribe();

    let folder = root.path().join("plain-folder");
    std::fs::create_dir_all(&folder).unwrap();

    let view = runtime
        .projects()
        .import_folder(ImportFolderRequest {
            path: folder.display().to_string(),
            directory_name: None,
        })
        .await
        .unwrap();
    assert_eq!(view.source, ProjectSource::Folder);

    let saw = wait_for_event(&mut events, |e| matches!(e, RuntimeEvent::ProjectsChanged)).await;
    assert!(saw, "folder import must emit RuntimeEvent::ProjectsChanged");

    runtime.shutdown().await;
}

#[tokio::test]
async fn importing_a_folder_that_is_already_a_git_repository_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 0));
    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    let folder = root.path().join("becomes-a-repo");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "hi\n").unwrap();

    runtime
        .projects()
        .import_repository(ImportRepositoryRequest {
            path: folder.display().to_string(),
            branch: None,
            directory_name: None,
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await
        .unwrap();

    let result = runtime
        .projects()
        .import_folder(ImportFolderRequest {
            path: folder.display().to_string(),
            directory_name: None,
        })
        .await;

    assert!(
        matches!(result, Err(CoreError::FolderIsGitRepo { .. })),
        "a folder already registered as a repository must be rejected: {result:?}"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn importing_a_registered_folder_as_a_repository_is_a_project_kind_conflict() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    let folder = root.path().join("stays-a-folder");
    std::fs::create_dir_all(&folder).unwrap();

    runtime
        .projects()
        .import_folder(ImportFolderRequest {
            path: folder.display().to_string(),
            directory_name: None,
        })
        .await
        .unwrap();

    let result = runtime
        .projects()
        .import_repository(ImportRepositoryRequest {
            path: folder.display().to_string(),
            branch: None,
            directory_name: None,
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await;

    assert!(
        matches!(
            result,
            Err(CoreError::ProjectKindConflict {
                existing_kind: "local folder",
                ..
            })
        ),
        "a folder already registered must reject a repository import: {result:?}"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn delete_files_true_deletes_an_imported_projects_directory_it_did_not_create() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 0));
    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    // A pre-existing repository Hyprlayer never created: proves delete_files=true
    // carries no ownership restriction, exactly as ADR-0002/the plan require.
    let repo = fixture_repo(root.path(), "existing-repo");
    let view = runtime
        .projects()
        .import_repository(ImportRepositoryRequest {
            path: repo.display().to_string(),
            branch: None,
            directory_name: None,
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await
        .unwrap();

    runtime.projects().remove(&view.path, true).await.unwrap();

    assert!(
        !Path::new(&view.path).exists(),
        "delete_files=true must delete the directory regardless of who created it"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn delete_files_true_deletes_a_cloned_projects_directory() {
    let root = tempfile::tempdir().unwrap();
    let origin = fixture_repo(root.path(), "origin");
    let paths = test_paths(&root.path().join("profile"));
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(root.path(), 0));
    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .open()
        .await
        .unwrap();

    let view = runtime
        .projects()
        .clone(CloneProjectRequest {
            url: origin.display().to_string(),
            location: None,
            name: Some("cloned".into()),
            thoughts_dir: None,
            profile: None,
            bootstrap_user: Some("tester".into()),
        })
        .await
        .unwrap();

    runtime.projects().remove(&view.path, true).await.unwrap();

    assert!(
        !Path::new(&view.path).exists(),
        "delete_files=true must delete a cloned project's directory"
    );

    runtime.shutdown().await;
}
