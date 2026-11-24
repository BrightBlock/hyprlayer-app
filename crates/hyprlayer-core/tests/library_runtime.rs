#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::docs::frontmatter::{FmPatch, FmValue};
use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::runtime::HyprlayerRuntime;
use hyprlayer_core::CoreError;
use hyprlayer_models::runtime::ThoughtsIndexUpdateKind;
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

const PROJECT_PATH: &str = "/tmp/hyprlayer-library-runtime-project";
const MAPPED_PROJECT: &str = "example-project";

fn git_backend(thoughts_root: &Path) -> serde_json::Value {
    serde_json::json!({
        "kind": "git",
        "thoughtsRepo": thoughts_root.display().to_string(),
        "reposDir": "repos",
        "globalDir": "global",
    })
}

fn obsidian_backend(thoughts_root: &Path) -> serde_json::Value {
    serde_json::json!({
        "kind": "obsidian",
        "vaultPath": thoughts_root.display().to_string(),
        "reposDir": "repos",
        "globalDir": "global",
    })
}

struct Fixture {
    _dir: tempfile::TempDir,
    thoughts_root: PathBuf,
    runtime: Arc<HyprlayerRuntime>,
}

async fn fixture() -> Fixture {
    fixture_with(git_backend).await
}

async fn fixture_with(backend: fn(&Path) -> serde_json::Value) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir.path().join("profile"));
    let thoughts_root = dir.path().join("thoughts");
    std::fs::create_dir_all(&thoughts_root).unwrap();
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();

    let config = serde_json::json!({
        "version": 4,
        "thoughts": {
            "user": "tester",
            "backend": backend(&thoughts_root),
            "repoMappings": { PROJECT_PATH: MAPPED_PROJECT },
        }
    });
    std::fs::write(
        &paths.hyprlayer_config_file,
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();

    let runtime = RuntimeBuilder::new(test_environment(dir.path()), paths)
        .open()
        .await
        .unwrap();

    Fixture {
        _dir: dir,
        thoughts_root,
        runtime,
    }
}

fn plan_request(title: &str) -> CreateDocumentRequest {
    CreateDocumentRequest {
        project_repo_path: None,
        scope: "global".into(),
        doc_type: "plan".into(),
        title: title.into(),
        date: "2026-08-30".into(),
        ..Default::default()
    }
}

async fn base_mtime(runtime: &HyprlayerRuntime, id: &str) -> f64 {
    runtime.library().get(id).await.unwrap().mtime_ms
}

fn body_bytes(raw: &[u8]) -> Vec<u8> {
    let opened = raw
        .strip_prefix(b"---\n".as_slice())
        .expect("the document opens with a frontmatter fence");
    let end = opened
        .windows(5)
        .position(|w| w == b"\n---\n")
        .expect("the frontmatter closes");
    opened[end + 5..].to_vec()
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
async fn create_places_a_global_doc_under_its_scoped_type_directory() {
    let f = fixture().await;

    let meta = f
        .runtime
        .library()
        .create(plan_request("Runtime Facade Rollout"))
        .await
        .unwrap();

    let expected = f
        .thoughts_root
        .join("global")
        .join("shared")
        .join("plans")
        .join("2026-08-30-runtime-facade-rollout.md");
    assert_eq!(PathBuf::from(&meta.id), expected);
    assert!(expected.is_file(), "the document is written to disk");
    assert_eq!(
        meta.rel_path,
        "global/shared/plans/2026-08-30-runtime-facade-rollout.md"
    );
    assert_eq!(meta.scope, "global");

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn create_places_a_project_doc_under_the_mapped_project_and_user_area() {
    let f = fixture().await;

    let meta = f
        .runtime
        .library()
        .create(CreateDocumentRequest {
            project_repo_path: Some(PROJECT_PATH.into()),
            scope: "user".into(),
            doc_type: "research".into(),
            title: "Index Ownership".into(),
            date: "2026-08-30".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let expected = f
        .thoughts_root
        .join("repos")
        .join(MAPPED_PROJECT)
        .join("tester")
        .join("research")
        .join("2026-08-30-index-ownership.md");
    assert_eq!(PathBuf::from(&meta.id), expected);
    assert_eq!(meta.project.as_deref(), Some(MAPPED_PROJECT));

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn a_second_create_of_the_same_title_reports_the_document_already_exists() {
    let f = fixture().await;

    let meta = f
        .runtime
        .library()
        .create(plan_request("Duplicate Title"))
        .await
        .unwrap();
    let first = std::fs::read(&meta.id).unwrap();

    let err = f
        .runtime
        .library()
        .create(plan_request("Duplicate Title"))
        .await
        .expect_err("the second create collides with the first");

    assert!(
        matches!(&err, CoreError::DocExists { path } if path == &meta.id),
        "unexpected error: {err:?}"
    );
    assert_eq!(
        std::fs::read(&meta.id).unwrap(),
        first,
        "the existing document must not be overwritten"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn a_save_replaces_the_content_and_leaves_no_temp_file_behind() {
    let f = fixture().await;

    let meta = f
        .runtime
        .library()
        .create(plan_request("Atomic Save"))
        .await
        .unwrap();
    let base = base_mtime(&f.runtime, &meta.id).await;

    let updated = "---\ntitle: Atomic Save\ntype: plan\n---\n\nRewritten body.\n";
    f.runtime
        .library()
        .save(&meta.id, updated.into(), base)
        .await
        .unwrap();

    assert_eq!(std::fs::read_to_string(&meta.id).unwrap(), updated);

    let dir = Path::new(&meta.id).parent().unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the atomic rename must leave no temp file: {leftovers:?}"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn a_frontmatter_patch_leaves_the_body_byte_identical() {
    let f = fixture().await;

    let meta = f
        .runtime
        .library()
        .create(plan_request("Patch Only Frontmatter"))
        .await
        .unwrap();
    let base = base_mtime(&f.runtime, &meta.id).await;
    let seeded = "---\ntitle: Patch Only Frontmatter\ntype: plan\nstatus: draft\n---\n\n## Overview\n\nBody with ünïcödé, a\ttab, and a trailing space \n";
    f.runtime
        .library()
        .save(&meta.id, seeded.into(), base)
        .await
        .unwrap();

    let before = std::fs::read(&meta.id).unwrap();
    let base = base_mtime(&f.runtime, &meta.id).await;

    let patched = f
        .runtime
        .library()
        .patch_frontmatter(
            &meta.id,
            FmPatch {
                set: vec![
                    ("status".into(), FmValue::Str("in-review".into())),
                    ("tags".into(), FmValue::List(vec!["runtime".into()])),
                ],
                remove: Vec::new(),
            },
            base,
        )
        .await
        .unwrap();

    let after = std::fs::read(&meta.id).unwrap();
    assert_eq!(
        body_bytes(&after),
        body_bytes(&before),
        "a frontmatter patch must not touch a single body byte"
    );
    assert_ne!(before, after, "the frontmatter itself did change");
    assert_eq!(patched.fm.status.as_deref(), Some("in-review"));
    assert_eq!(patched.fm.tags, vec!["runtime".to_string()]);

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn a_save_reindexes_the_document_and_emits_a_thoughts_index_update() {
    let f = fixture().await;

    let meta = f
        .runtime
        .library()
        .create(plan_request("Reindexed On Save"))
        .await
        .unwrap();
    let base = base_mtime(&f.runtime, &meta.id).await;

    let mut rx = f.runtime.subscribe();
    let saved = f
        .runtime
        .library()
        .save(
            &meta.id,
            "---\ntitle: Renamed On Save\ntype: plan\n---\n\nBody.\n".into(),
            base,
        )
        .await
        .unwrap();

    assert_eq!(saved.fm.title.as_deref(), Some("Renamed On Save"));

    let queried = f.runtime.library().query(Default::default()).await.unwrap();
    assert!(
        queried
            .items
            .iter()
            .any(|d| d.id == meta.id && d.fm.title.as_deref() == Some("Renamed On Save")),
        "the save must be visible through the index"
    );

    let id = meta.id;
    assert!(
        wait_for_event(&mut rx, |event| matches!(
            event,
            RuntimeEvent::ThoughtsIndexUpdated {
                update: ThoughtsIndexUpdateKind::Updated,
                ids,
            } if ids.contains(&id)
        ))
        .await,
        "a save must announce the reindexed document"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn getting_an_unknown_id_reports_it_is_not_in_the_index() {
    let f = fixture().await;

    let err = f
        .runtime
        .library()
        .get("not-a-doc")
        .await
        .expect_err("an unindexed non-markdown id has nothing to fall back to");
    assert!(
        matches!(&err, CoreError::DocNotIndexed { id } if id == "not-a-doc"),
        "unexpected error: {err:?}"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn an_unindexed_markdown_file_still_loads_through_the_fallback() {
    let f = fixture().await;

    let loose = f._dir.path().join("outside.md");
    std::fs::write(&loose, "---\ntitle: Outside\n---\n\nNot in any root.\n").unwrap();

    let doc = f
        .runtime
        .library()
        .get(&loose.display().to_string())
        .await
        .unwrap();
    assert_eq!(doc.meta.fm.title.as_deref(), Some("Outside"));
    assert!(doc.content.contains("Not in any root."));

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn thoughts_git_status_reports_the_repository_backing_the_root() {
    let f = fixture().await;

    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&f.thoughts_root)
        .args(["init", "-b", "main"])
        .output()
        .expect("git runs");
    assert!(out.status.success());

    let status = f
        .runtime
        .library()
        .thoughts_git_status(PROJECT_PATH)
        .await
        .unwrap();
    assert!(!status.has_remote);
    assert_eq!(status.ahead, 0);
    assert_eq!(status.behind, 0);

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn thoughts_git_status_declines_a_backend_that_does_not_sync_through_git() {
    let f = fixture_with(obsidian_backend).await;

    let err = f
        .runtime
        .library()
        .thoughts_git_status(PROJECT_PATH)
        .await
        .expect_err("an obsidian vault has no git remote to report on");
    assert!(
        matches!(&err, CoreError::BackendNotGit { backend } if backend == "obsidian"),
        "unexpected error: {err:?}"
    );
    assert_eq!(err.code(), "backend_not_git");

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn create_refuses_a_date_that_escapes_the_thoughts_tree() {
    let fx = fixture().await;
    let outside = fx._dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let mut req = plan_request("Escape");
    req.date = "../../../../outside/pwn".into();

    let result = fx.runtime.library().create(req).await;

    assert!(
        matches!(result, Err(CoreError::DocOutsideRoots { .. })),
        "{result:?}"
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn removing_an_unregistered_path_with_its_files_deletes_nothing() {
    let fx = fixture().await;
    let victim = fx._dir.path().join("never-registered");
    std::fs::create_dir_all(victim.join("src")).unwrap();
    std::fs::write(victim.join("src/precious.txt"), "keep me").unwrap();

    let result = fx
        .runtime
        .projects()
        .remove(&victim.display().to_string(), true)
        .await;

    assert!(
        matches!(result, Err(CoreError::ProjectNotFound { .. })),
        "{result:?}"
    );
    assert!(victim.join("src/precious.txt").exists());
    fx.runtime.shutdown().await;
}
