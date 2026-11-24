#![cfg(feature = "test-support")]

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use hyprlayer_core::claude::memory::{project_key, MemoryKeyCheck};
use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::repo_settings::ScriptSettings;
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::runtime::ports::LifecycleRunner;
use hyprlayer_core::runtime::script_registry::RunPhase;
use hyprlayer_core::runtime::scripts::{RunLifecycleRequest, ScriptSettingsRequest};
use hyprlayer_core::runtime::sessions::StartStageSessionRequest;
use hyprlayer_core::runtime::workstreams::{
    ArchiveWorkstreamRequest, CreateWorkstreamRequest, HeadBranchObservation,
    ListWorkstreamsRequest, PurgeWorkstreamRequest, RestoreWorkstreamRequest,
};
use hyprlayer_core::runtime::HyprlayerRuntime;
use hyprlayer_core::scripts::ScriptKind;
use hyprlayer_core::store::{Workstream, WorkstreamStatus, WorktreeMode};
use hyprlayer_core::worktree::archive_dir_for;
use hyprlayer_core::{CoreError, Harness, Stage};
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

// A fake `hyprlayer` binary, standing in for `thoughts init`: its whole job
// is to succeed or fail on cue so create()'s CLI-then-manual-fallback branch
// is deterministic regardless of whether the real CLI is installed on the
// machine running the test (same technique as project_runtime.rs).
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

// The manual-provisioning fallback (`thoughts_links::provision_thoughts`)
// requires a non-empty user; leaving it empty here is a deliberate,
// deterministic way to make that fallback fail on cue, exactly like an
// unconfigured profile would (ADR-0005: real filesystem/git, not a port).
fn write_config(paths: &RuntimePaths, thoughts_root: &Path, mappings: &[(&Path, &str)]) {
    let mut repo_mappings = serde_json::Map::new();
    for (path, mapped) in mappings {
        repo_mappings.insert(
            path.display().to_string(),
            serde_json::Value::String((*mapped).to_string()),
        );
    }
    let config = serde_json::json!({
        "version": 4,
        "thoughts": {
            "user": "",
            "backend": {
                "kind": "git",
                "thoughtsRepo": thoughts_root.display().to_string(),
                "reposDir": "repos",
                "globalDir": "global",
            },
            "repoMappings": repo_mappings,
        }
    });
    std::fs::write(
        &paths.hyprlayer_config_file,
        serde_json::to_string_pretty(&config).unwrap(),
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

fn branch_exists(repo: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", branch])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Fixture {
    _dir: tempfile::TempDir,
    paths: RuntimePaths,
    runtime: Arc<HyprlayerRuntime>,
}

/// A `LifecycleRunner` double: reports a fixed phase for the requested kind
/// and `None` (unconfigured) for every other kind, matching the desktop
/// script runner's own "no command configured" contract.
struct FixedLifecycleRunner {
    kind: ScriptKind,
    phase: RunPhase,
}

impl LifecycleRunner for FixedLifecycleRunner {
    fn run<'a>(
        &'a self,
        _item: &'a Workstream,
        kind: ScriptKind,
    ) -> Pin<Box<dyn Future<Output = Option<RunPhase>> + Send + 'a>> {
        let result = (kind == self.kind).then_some(self.phase);
        Box::pin(async move { result })
    }
}

async fn fixture(hyprlayer_exit: Option<i32>) -> (Fixture, PathBuf) {
    fixture_with_runner(hyprlayer_exit, None).await
}

async fn fixture_with_runner(
    hyprlayer_exit: Option<i32>,
    lifecycle_runner: Option<Arc<dyn LifecycleRunner + Send + Sync>>,
) -> (Fixture, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let paths = test_paths(&root.join("profile"));
    let thoughts_root = root.join("thoughts");
    std::fs::create_dir_all(&thoughts_root).unwrap();
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();

    let repo = fixture_repo(&root, "project");
    write_config(&paths, &thoughts_root, &[(&repo, "project")]);
    if let Some(code) = hyprlayer_exit {
        write_hyprlayer_override(&paths, &write_fake_hyprlayer(&root, code));
    }

    let mut builder = RuntimeBuilder::new(test_environment(&root), paths.clone());
    if let Some(runner) = lifecycle_runner {
        builder = builder.with_lifecycle_runner(runner);
    }
    let runtime = builder.open().await.unwrap();

    (
        Fixture {
            _dir: dir,
            paths,
            runtime,
        },
        repo,
    )
}

fn isolated_request(project_path: &Path, title: &str) -> CreateWorkstreamRequest {
    CreateWorkstreamRequest {
        project_path: project_path.display().to_string(),
        title: title.into(),
        base: None,
        mode: WorktreeMode::Isolated,
        branch: None,
        branch_prefix: None,
        parent_workstream_id: None,
    }
}

fn inplace_request(project_path: &Path, title: &str) -> CreateWorkstreamRequest {
    CreateWorkstreamRequest {
        project_path: project_path.display().to_string(),
        title: title.into(),
        base: None,
        mode: WorktreeMode::Inplace,
        branch: Some("main".into()),
        branch_prefix: None,
        parent_workstream_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_in_place_creates_yield_one_row() {
    let (f, repo) = fixture(Some(0)).await;

    let mut handles = Vec::new();
    for i in 0..5 {
        let runtime = f.runtime.clone();
        let req = inplace_request(&repo, &format!("Same Title {i}"));
        handles.push(tokio::spawn(async move {
            runtime.workstreams().create(req).await
        }));
    }

    let mut ok = 0;
    let mut guard_errors = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => ok += 1,
            Err(CoreError::InplaceWorkstreamExists) => guard_errors += 1,
            Err(e) => panic!("unexpected error racing in-place creates: {e}"),
        }
    }
    assert_eq!(ok, 1, "exactly one concurrent in-place create must win");
    assert_eq!(
        guard_errors, 4,
        "the rest must see the one-active-in-place guard"
    );

    let rows = f
        .runtime
        .workstreams()
        .list(ListWorkstreamsRequest {
            project_path: repo.display().to_string(),
            include_archived: false,
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "only one row must land in the database");
    assert!(rows[0].is_inplace());

    f.runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_creates_in_different_projects_do_not_block_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let paths = test_paths(&root.join("profile"));
    let thoughts_root = root.join("thoughts");
    std::fs::create_dir_all(&thoughts_root).unwrap();
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();

    let repo_a = fixture_repo(&root, "project-a");
    let repo_b = fixture_repo(&root, "project-b");
    write_config(
        &paths,
        &thoughts_root,
        &[(&repo_a, "project-a"), (&repo_b, "project-b")],
    );
    write_hyprlayer_override(&paths, &write_fake_hyprlayer(&root, 0));

    let runtime = RuntimeBuilder::new(test_environment(&root), paths)
        .open()
        .await
        .unwrap();

    let ra = runtime.clone();
    let req_a = isolated_request(&repo_a, "Workstream A");
    let a = tokio::spawn(async move { ra.workstreams().create(req_a).await });
    let rb = runtime.clone();
    let req_b = isolated_request(&repo_b, "Workstream B");
    let b = tokio::spawn(async move { rb.workstreams().create(req_b).await });

    let (a, b) = tokio::join!(a, b);
    let a = a.unwrap().expect("project A create must succeed");
    let b = b.unwrap().expect("project B create must succeed");
    assert_ne!(a.item.id, b.item.id);
    assert_eq!(a.item.project_path, repo_a.display().to_string());
    assert_eq!(b.item.project_path, repo_b.display().to_string());

    runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_project_create_and_archive_serialize_without_corruption() {
    let (f, repo) = fixture(Some(0)).await;

    let first = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "First"))
        .await
        .unwrap();

    let runtime_a = f.runtime.clone();
    let id_a = first.item.id.clone();
    let archive = tokio::spawn(async move {
        runtime_a
            .workstreams()
            .archive(ArchiveWorkstreamRequest {
                id: id_a,
                skip_scripts: true,
            })
            .await
    });

    let runtime_b = f.runtime.clone();
    let create = tokio::spawn(async move {
        runtime_b
            .workstreams()
            .create(isolated_request(&repo, "Second"))
            .await
    });

    let (archived, created) = tokio::join!(archive, create);
    archived
        .unwrap()
        .expect("archiving the first workstream must succeed");
    let second = created
        .unwrap()
        .expect("creating the second workstream concurrently must succeed");

    let active = f.runtime.workstreams().list_all().await.unwrap();
    assert_eq!(active.len(), 1, "only the second workstream stays active");
    assert_eq!(active[0].id, second.item.id);

    let archived_rows = f.runtime.workstreams().list_archived().await.unwrap();
    assert_eq!(archived_rows.len(), 1);
    assert_eq!(archived_rows[0].id, first.item.id);
    assert_eq!(archived_rows[0].status, WorkstreamStatus::Archived);

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn thoughts_failure_removes_only_the_app_created_worktree_and_branch() {
    // exit 1: thoughts_init fails and create() falls into the manual
    // provisioning fallback, which itself fails because `user` is empty.
    let (f, repo) = fixture(Some(1)).await;

    let result = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Doomed"))
        .await;
    assert!(
        result.is_err(),
        "a failed thoughts provisioning must fail create()"
    );

    let rows = f.runtime.workstreams().list_all().await.unwrap();
    assert!(rows.is_empty(), "no row may be inserted on rollback");

    let worktrees_root = f.paths.default_worktree_root.join("project");
    assert!(
        !worktrees_root.exists() || std::fs::read_dir(&worktrees_root).unwrap().next().is_none(),
        "the worktree directory this call created must be removed"
    );

    assert!(branch_exists(&repo, "main"));

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn failed_archive_script_leaves_disk_and_row_active() {
    let (f, repo) = fixture_with_runner(
        Some(0),
        Some(Arc::new(FixedLifecycleRunner {
            kind: ScriptKind::Archive,
            phase: RunPhase::Failed,
        })),
    )
    .await;
    let created = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Guarded"))
        .await
        .unwrap();
    let worktree_path = PathBuf::from(&created.item.worktree_path);
    assert!(worktree_path.is_dir());
    f.runtime
        .workstreams()
        .watch(HashSet::from([created.item.id.clone()]))
        .await
        .unwrap();

    let result = f
        .runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: created.item.id.clone(),
            skip_scripts: false,
        })
        .await;
    assert!(
        matches!(result, Err(CoreError::ArchiveScriptFailed)),
        "a failed Archive script must be the fatal pre-destructive gate: {result:?}"
    );

    assert!(worktree_path.is_dir(), "the worktree must not have moved");
    assert_eq!(
        f.runtime.test_watched_workstreams().await,
        vec![created.item.id.clone()],
        "the UI stays on this workstream and only re-arms on navigation"
    );
    let row = f
        .runtime
        .workstreams()
        .list_all()
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.id == created.item.id)
        .expect("the row must still be active");
    assert_eq!(row.status, WorkstreamStatus::Active);

    f.runtime.shutdown().await;
}

// On Windows the archive itself fails with "Permission denied" if the watcher is still armed.
#[tokio::test]
async fn archive_releases_its_own_worktree_watch_and_no_other() {
    let (f, repo) = fixture(Some(0)).await;
    let watched = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Watched"))
        .await
        .unwrap();
    let other = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Other"))
        .await
        .unwrap();
    f.runtime
        .workstreams()
        .watch(HashSet::from([
            watched.item.id.clone(),
            other.item.id.clone(),
        ]))
        .await
        .unwrap();

    f.runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: other.item.id,
            skip_scripts: true,
        })
        .await
        .unwrap();
    assert_eq!(
        f.runtime.test_watched_workstreams().await,
        vec![watched.item.id.clone()],
        "archiving another workstream must leave the watch alone"
    );

    let worktree = PathBuf::from(&watched.item.worktree_path);
    f.runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: watched.item.id,
            skip_scripts: true,
        })
        .await
        .unwrap();
    assert!(!worktree.exists(), "the worktree must have moved");
    assert_eq!(
        f.runtime.test_watched_workstreams().await,
        Vec::<String>::new(),
        "nothing may hold the moved worktree open"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn failed_archive_move_keeps_the_worktree_watched() {
    let (f, repo) = fixture(Some(0)).await;
    let created = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Blocked"))
        .await
        .unwrap();
    f.runtime
        .workstreams()
        .watch(HashSet::from([created.item.id.clone()]))
        .await
        .unwrap();
    std::fs::create_dir_all(archive_dir_for(&repo, &created.item.slug)).unwrap();

    let result = f
        .runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: created.item.id.clone(),
            skip_scripts: true,
        })
        .await;
    assert!(
        result.is_err(),
        "an occupied destination must fail the move"
    );
    assert_eq!(
        f.runtime.test_watched_workstreams().await,
        vec![created.item.id],
        "the UI stays on this workstream and only re-arms on navigation"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn restore_failure_leaves_the_row_archived() {
    let (f, repo) = fixture(Some(0)).await;
    let created = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Fragile"))
        .await
        .unwrap();
    f.runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: created.item.id.clone(),
            skip_scripts: true,
        })
        .await
        .unwrap();

    let archive_path = archive_dir_for(&repo, &created.item.slug);
    assert!(archive_path.exists());
    std::fs::remove_dir_all(&archive_path).unwrap();

    let result = f
        .runtime
        .workstreams()
        .restore(RestoreWorkstreamRequest {
            id: created.item.id.clone(),
            target: None,
        })
        .await;
    match result {
        Err(CoreError::WorkstreamArchiveMissing { .. }) => {}
        Err(e) => panic!("expected WorkstreamArchiveMissing, got: {e}"),
        Ok(_) => panic!("a missing archive directory must fail restore"),
    }

    let row = f
        .runtime
        .workstreams()
        .list_archived()
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.id == created.item.id)
        .expect("the row must still be listed as archived");
    assert_eq!(row.status, WorkstreamStatus::Archived);

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn purge_deletes_the_branch_only_when_the_app_owns_it() {
    let (f, repo) = fixture(Some(0)).await;

    // Isolated mode: the app creates the branch, so purge deletes it.
    let owned = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "App Owned"))
        .await
        .unwrap();
    assert!(owned.item.app_owns_branch());
    let owned_branch = owned.item.branch;

    f.runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: owned.item.id.clone(),
            skip_scripts: true,
        })
        .await
        .unwrap();
    f.runtime
        .workstreams()
        .purge(PurgeWorkstreamRequest { id: owned.item.id })
        .await
        .unwrap();
    assert!(
        !branch_exists(&repo, &owned_branch),
        "purge must delete a branch the app created"
    );

    // Existing mode: the app adopts a pre-existing branch, so purge must
    // leave it alone.
    sh(&repo, &["branch", "adopted-feature"]);
    let adopted = f
        .runtime
        .workstreams()
        .create(CreateWorkstreamRequest {
            project_path: repo.display().to_string(),
            title: "Adopted".into(),
            base: None,
            mode: WorktreeMode::Existing,
            branch: Some("adopted-feature".into()),
            branch_prefix: None,
            parent_workstream_id: None,
        })
        .await
        .unwrap();
    assert!(!adopted.item.app_owns_branch());

    f.runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: adopted.item.id.clone(),
            skip_scripts: true,
        })
        .await
        .unwrap();
    f.runtime
        .workstreams()
        .purge(PurgeWorkstreamRequest {
            id: adopted.item.id,
        })
        .await
        .unwrap();
    assert!(
        branch_exists(&repo, "adopted-feature"),
        "purge must never delete a branch the app did not create"
    );

    f.runtime.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn head_branch_adoption_moves_only_the_workstreams_that_follow_head() {
    let (f, repo) = fixture(Some(0)).await;
    let mut events = f.runtime.subscribe();

    let inplace = f
        .runtime
        .workstreams()
        .create(inplace_request(&repo, "Inplace"))
        .await
        .unwrap()
        .item;
    let isolated = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Isolated"))
        .await
        .unwrap()
        .item;

    let adoptions = f
        .runtime
        .workstreams()
        .adopt_head_branches(vec![
            HeadBranchObservation {
                item: inplace.clone(),
                head_branch: Some("feature".into()),
            },
            HeadBranchObservation {
                item: isolated.clone(),
                head_branch: Some("feature".into()),
            },
        ])
        .await
        .unwrap();

    assert_eq!(
        adoptions.len(),
        1,
        "only the in-place workstream follows head"
    );
    assert_eq!(adoptions[0].workstream_id, inplace.id);
    assert_eq!(adoptions[0].branch, "feature");

    let ws = f.runtime.workstreams().find(&inplace.id).await.unwrap();
    assert_eq!(ws.branch, "feature");
    let untouched = f.runtime.workstreams().find(&isolated.id).await.unwrap();
    assert_eq!(untouched.branch, isolated.branch);

    let mut announced = None;
    while let Ok(event) = events.try_recv() {
        if let RuntimeEvent::WorkstreamBranchChanged {
            workstream_id,
            branch,
            previous,
        } = event
        {
            announced = Some((workstream_id, branch, previous));
        }
    }
    assert_eq!(
        announced,
        Some((inplace.id, "feature".into(), inplace.branch)),
        "the adoption must reach the runtime event stream"
    );

    f.runtime.shutdown().await;
}

/// The gap Phase 6's manual `git push` check could not catch: that one went
/// through a terminal, the single path that already merged the overlay. A
/// lifecycle script is a different spawn, and had none of it (ADR-0009 §7).
#[tokio::test]
async fn the_attach_overlay_reaches_a_lifecycle_script() {
    let (f, repo) = fixture_with_runner(Some(0), None).await;
    let created = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Overlay"))
        .await
        .unwrap();

    let probe = f.paths.app_data_dir.join("agent-sock-probe");
    f.runtime
        .scripts()
        .settings(ScriptSettingsRequest {
            project_path: repo.display().to_string(),
            set: Some(ScriptSettings {
                setup: format!("printf '%s' \"$SSH_AUTH_SOCK\" > {}", probe.display()),
                run: String::new(),
                archive: String::new(),
            }),
        })
        .await
        .unwrap();

    // A real socket, kept alive for the test: the overlay now drops an
    // SSH_AUTH_SOCK that names nothing here.
    #[cfg(unix)]
    let agent_sock = f.paths.app_data_dir.join("a.sock");
    #[cfg(unix)]
    let _agent = std::os::unix::net::UnixListener::bind(&agent_sock).unwrap();
    #[cfg(not(unix))]
    let agent_sock = PathBuf::from("/tmp/overlay-probe.sock");

    f.runtime.set_env_overlay(BTreeMap::from([(
        "SSH_AUTH_SOCK".to_string(),
        agent_sock.display().to_string(),
    )]));

    let result = f
        .runtime
        .scripts()
        .run_lifecycle(RunLifecycleRequest {
            workstream: created.item,
            kind: ScriptKind::Setup,
        })
        .await
        .unwrap();
    assert!(result.is_some(), "the Setup script must have run");

    let seen = std::fs::read_to_string(&probe).unwrap_or_default();
    assert_eq!(
        seen,
        agent_sock.display().to_string(),
        "a lifecycle script must inherit the attaching client's SSH_AUTH_SOCK"
    );

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn project_memory_lists_the_projects_claude_memory() {
    let (f, repo) = fixture(Some(0)).await;
    let created = f
        .runtime
        .workstreams()
        .create(inplace_request(&repo, "Memory"))
        .await
        .unwrap();

    let key = project_key(&dunce::canonicalize(&repo).unwrap().to_string_lossy());
    let dir = f
        .paths
        .claude_config_dir
        .join("projects")
        .join(&key)
        .join("memory");

    let listing = f
        .runtime
        .workstreams()
        .project_memory(&created.item.id)
        .await
        .unwrap();
    assert_eq!(listing.dir, dir.to_string_lossy().into_owned());
    assert_eq!(listing.key_check, MemoryKeyCheck::Unverified);
    assert!(listing.files.is_empty());

    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("topic.md"), "topic").unwrap();
    std::fs::write(dir.join("MEMORY.md"), "index").unwrap();

    let listing = f
        .runtime
        .workstreams()
        .project_memory(&created.item.id)
        .await
        .unwrap();
    let names: Vec<&str> = listing
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    assert_eq!(names, vec!["MEMORY.md", "topic.md"]);

    let started = f
        .runtime
        .sessions()
        .prepare_start(StartStageSessionRequest {
            workstream: created.item.clone(),
            stage: Stage::General,
            input: String::new(),
            attached_docs: Vec::new(),
            harness: Some(Harness::Claude),
            model: None,
            effort: None,
            permission_mode: None,
            prompt_name: None,
            client_message_id: None,
            parent_session_id: None,
        })
        .await
        .unwrap();
    let session_id = started.session.id;

    let transcript_dir = f.paths.claude_config_dir.join("projects").join(&key);
    std::fs::create_dir_all(&transcript_dir).unwrap();
    std::fs::write(transcript_dir.join(format!("{session_id}.jsonl")), "").unwrap();

    let listing = f
        .runtime
        .workstreams()
        .project_memory(&created.item.id)
        .await
        .unwrap();
    assert_eq!(listing.key_check, MemoryKeyCheck::Verified);

    let moved_dir = f.paths.claude_config_dir.join("projects").join("-moved");
    std::fs::create_dir_all(&moved_dir).unwrap();
    std::fs::rename(
        transcript_dir.join(format!("{session_id}.jsonl")),
        moved_dir.join(format!("{session_id}.jsonl")),
    )
    .unwrap();

    let listing = f
        .runtime
        .workstreams()
        .project_memory(&created.item.id)
        .await
        .unwrap();
    assert_eq!(listing.key_check, MemoryKeyCheck::Mismatch);
    assert!(listing.files.is_empty());

    let missing = f.runtime.workstreams().project_memory("missing").await;
    assert!(matches!(missing, Err(CoreError::WorkstreamNotFound { .. })));

    f.runtime.shutdown().await;
}

#[tokio::test]
async fn saving_project_memory_guards_names_mtimes_and_links() {
    let (f, repo) = fixture(Some(0)).await;
    let id = f
        .runtime
        .workstreams()
        .create(inplace_request(&repo, "Memory"))
        .await
        .unwrap()
        .item
        .id;
    let key = project_key(&dunce::canonicalize(&repo).unwrap().to_string_lossy());
    let dir = f
        .paths
        .claude_config_dir
        .join("projects")
        .join(&key)
        .join("memory");
    std::fs::create_dir_all(&dir).unwrap();
    let index = dir.join("MEMORY.md");
    std::fs::write(&index, "old").unwrap();
    std::fs::File::open(&index)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    let workstreams = f.runtime.workstreams();
    let base = workstreams
        .project_memory(&id)
        .await
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.name == "MEMORY.md")
        .unwrap()
        .mtime_ms;

    workstreams
        .save_project_memory_file(&id, "MEMORY.md", "new".into(), base)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&index).unwrap(), "new");

    let stale = workstreams
        .save_project_memory_file(&id, "MEMORY.md", "newer".into(), base)
        .await;
    assert!(matches!(stale, Err(CoreError::DocConflict)));
    assert_eq!(std::fs::read_to_string(&index).unwrap(), "new");

    let escape = workstreams
        .save_project_memory_file(&id, "../escape.md", "x".into(), base)
        .await;
    assert!(matches!(escape, Err(CoreError::InvalidPath { .. })));
    assert!(!dir.parent().unwrap().join("escape.md").exists());

    let absent = workstreams
        .save_project_memory_file(&id, "absent.md", "x".into(), base)
        .await;
    assert!(matches!(absent, Err(CoreError::Io(_))));
    assert!(!dir.join("absent.md").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let outside = repo.parent().unwrap().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("victim.txt");
        std::fs::write(&victim, "victim").unwrap();
        let planted = dir.join("MEMORY.md.tmp");
        symlink(&victim, &planted).unwrap();
        let listing = workstreams.project_memory(&id).await.unwrap().files;
        let current = listing
            .iter()
            .find(|file| file.name == "MEMORY.md")
            .unwrap();
        workstreams
            .save_project_memory_file(&id, "MEMORY.md", "past the plant".into(), current.mtime_ms)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        assert!(std::fs::symlink_metadata(&index)
            .unwrap()
            .file_type()
            .is_file());
        assert_eq!(std::fs::read_to_string(&index).unwrap(), "past the plant");
        assert_eq!(std::fs::read_link(&planted).unwrap(), victim);

        let shared = outside.join("shared.md");
        std::fs::write(&shared, "shared").unwrap();
        let linked = dir.join("linked.md");
        symlink(&shared, &linked).unwrap();
        let listing = workstreams.project_memory(&id).await.unwrap().files;
        let linked_base = listing
            .iter()
            .find(|file| file.name == "linked.md")
            .unwrap();
        let refused = workstreams
            .save_project_memory_file(&id, "linked.md", "replaced".into(), linked_base.mtime_ms)
            .await;
        assert!(matches!(refused, Err(CoreError::InvalidPath { .. })));
        assert_eq!(std::fs::read_to_string(&shared).unwrap(), "shared");
        assert!(std::fs::symlink_metadata(&linked)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );

    f.runtime.shutdown().await;
}

fn existing_request(project_path: &Path, title: &str, branch: &str) -> CreateWorkstreamRequest {
    CreateWorkstreamRequest {
        project_path: project_path.display().to_string(),
        title: title.into(),
        base: None,
        mode: WorktreeMode::Existing,
        branch: Some(branch.into()),
        branch_prefix: None,
        parent_workstream_id: None,
    }
}

fn git_out(repo: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test]
async fn create_refuses_a_base_that_git_would_read_as_an_option() {
    let (f, repo) = fixture(Some(0)).await;
    let mut req = isolated_request(&repo, "Option Injection");
    req.base = Some("--lock".into());
    let created = f.runtime.workstreams().create(req).await;

    let Ok(created) = created else {
        f.runtime.shutdown().await;
        return;
    };
    let listing = git_out(&repo, &["worktree", "list", "--porcelain"]);
    let archived = f
        .runtime
        .workstreams()
        .archive(ArchiveWorkstreamRequest {
            id: created.item.id.clone(),
            skip_scripts: true,
        })
        .await;
    f.runtime.shutdown().await;
    panic!(
        "create accepted base \"--lock\": row base_branch={:?} branch={:?}\n\
         git worktree list --porcelain:\n{listing}\narchive of it -> {:?}",
        created.item.base_branch,
        created.item.branch,
        archived.map(|w| w.status)
    );
}

#[tokio::test]
async fn set_base_branch_refuses_a_base_that_git_would_read_as_an_option() {
    let (f, repo) = fixture(Some(0)).await;
    let root = repo.parent().unwrap().to_path_buf();
    let bare = root.join("origin.git");
    sh(&root, &["clone", "--bare", "project", "origin.git"]);
    sh(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sh(&repo, &["fetch", "origin"]);

    let created = f
        .runtime
        .workstreams()
        .create(isolated_request(&repo, "Injected Base"))
        .await
        .unwrap();
    let marker = root.join("PWNED-by-base-branch");
    let base = format!("--upload-pack=touch {}", marker.display());
    let set = f
        .runtime
        .workstreams()
        .set_base_branch(&created.item.id, &base)
        .await;
    let pulled = f
        .runtime
        .workstreams()
        .update_from_base(&created.item.id)
        .await;
    let ran = marker.exists();
    f.runtime.shutdown().await;
    assert!(
        !ran,
        "set_base_branch({base:?}) -> {set:?}; update_from_base -> {pulled:?}; \
         the command in the base branch name ran: {} exists",
        marker.display()
    );
}

fn write_fake_gh_answering(
    root: &Path,
    number: u32,
    state: &str,
    head: &str,
) -> (PathBuf, PathBuf) {
    let log = root.join("gh-calls.log");
    let script = root.join("fake-gh.sh");
    let body = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "gh version 2.99.0 (2026-01-01)"; exit 0; fi
line="gh"
for a in "$@"; do case "$a" in query=*) line="$line query=<graphql>";; *) line="$line $a";; esac; done
echo "$line" >> "LOGFILE"
case "$*" in
  "pr merge"*) exit 0;;
  *"number=NUMBER"*) cat <<'JSON'
{"data":{"repository":{"pullRequest":{"number":NUMBER,"url":"https://ghe.invalid/acme/widgets/pull/NUMBER","title":"A teammate's unrelated PR","state":"STATE","headRefName":"HEAD","baseRefName":"main","headRefOid":"1111111111111111111111111111111111111111","isCrossRepository":false,"commits":{"nodes":[]}}}}}
JSON
  exit 0;;
  *) echo '{"data":{"repository":{"defaultBranchRef":{"name":"main"},"pullRequests":{"nodes":[]}}}}'; exit 0;;
esac
"#
    .replace("LOGFILE", &log.display().to_string())
    .replace("NUMBER", &number.to_string())
    .replace("STATE", state)
    .replace("HEAD", head);
    std::fs::write(&script, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (script, log)
}

async fn runtime_with_gh(
    number: u32,
    state: &str,
    head: &str,
) -> (tempfile::TempDir, Arc<HyprlayerRuntime>, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let paths = test_paths(&root.join("profile"));
    let thoughts_root = root.join("thoughts");
    std::fs::create_dir_all(&thoughts_root).unwrap();
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    let repo = fixture_repo(&root, "project");
    write_config(&paths, &thoughts_root, &[(&repo, "project")]);
    let (gh, log) = write_fake_gh_answering(&root, number, state, head);
    std::fs::create_dir_all(&paths.app_config_dir).unwrap();
    std::fs::write(
        paths.app_config_dir.join("settings.json"),
        serde_json::json!({ "binaryOverrides": {
            "hyprlayer": write_fake_hyprlayer(&root, 0).display().to_string(),
            "gh": gh.display().to_string(),
        } })
        .to_string(),
    )
    .unwrap();
    let runtime = RuntimeBuilder::new(test_environment(&root), paths)
        .open()
        .await
        .unwrap();
    (dir, runtime, repo, log)
}

#[tokio::test]
async fn an_all_digit_branch_is_not_read_as_a_pr_number() {
    let (_dir, runtime, repo, log) =
        runtime_with_gh(2024, "OPEN", "teammate/unrelated-feature").await;
    let ws = runtime.workstreams();

    sh(&repo, &["branch", "2024"]);
    let created = ws
        .create(existing_request(&repo, "Issue 2024", "2024"))
        .await
        .unwrap();
    sh(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://ghe.invalid/acme/widgets.git",
        ],
    );

    let status = ws
        .pr_status(
            hyprlayer_core::runtime::workstreams::WorkstreamPrStatusRequest {
                id: created.item.id.clone(),
            },
        )
        .await
        .unwrap();
    let merged = ws
        .merge_pr(&created.item.id, hyprlayer_core::pr::MergeMethod::Squash)
        .await;
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    runtime.shutdown().await;
    let found = status.known().map(|pr| (pr.number, pr.title.clone()));
    assert!(
        found.is_none() && !calls.contains("pr merge"),
        "workstream branch {:?} (pr_number: {:?})\npr_status -> {found:?}\nmerge_pr -> {merged:?}\ngh calls:\n{calls}",
        created.item.branch,
        created.item.pr_number
    );
}

#[tokio::test]
async fn archive_keeps_a_pushed_branch_whose_commits_no_merged_pr_holds() {
    let (dir, runtime, repo, _log) = runtime_with_gh(77, "MERGED", "teammate/other").await;
    let root = dir.path().to_path_buf();
    let ws = runtime.workstreams();
    sh(&root, &["clone", "--bare", "project", "origin.git"]);
    let bare = root.join("origin.git");
    sh(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sh(&repo, &["fetch", "origin"]);

    let created = ws
        .create(isolated_request(&repo, "Unmerged Work"))
        .await
        .unwrap();
    let worktree = PathBuf::from(&created.item.worktree_path);
    let branch = created.item.branch.clone();
    std::fs::write(worktree.join("feature.rs"), "// not merged anywhere\n").unwrap();
    sh(&worktree, &["add", "feature.rs"]);
    sh(
        &worktree,
        &["commit", "-m", "work that only lives on this branch"],
    );
    sh(&worktree, &["push", "origin", &branch]);
    sh(
        &repo,
        &[
            "remote",
            "add",
            "upstream",
            "https://ghe.invalid/acme/widgets.git",
        ],
    );
    let before = git_out(&repo, &["ls-remote", "origin", &branch]);

    let attached = ws.attach_pr(&created.item.id, 77).await;
    let archived = ws
        .archive(ArchiveWorkstreamRequest {
            id: created.item.id.clone(),
            skip_scripts: true,
        })
        .await;
    let after = git_out(&repo, &["ls-remote", "origin", &branch]);
    runtime.shutdown().await;
    assert!(
        after.contains(&branch),
        "attach_pr(77) -> {:?}; archive -> {:?}\nls-remote origin {branch} before archive: {before:?}\nls-remote origin {branch} after archive:  {after:?}",
        attached.map(|pr| pr.map(|pr| (pr.number, pr.state))),
        archived.map(|w| w.status)
    );
}
