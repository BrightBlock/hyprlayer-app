use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::env::RuntimeEnvironment;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryResolver, BinaryState};
use hyprlayer_core::process::spawn_supervised;

fn real_environment() -> RuntimeEnvironment {
    RuntimeEnvironment {
        home_dir: dirs::home_dir().unwrap_or_default(),
        process_cwd: std::env::current_dir().unwrap_or_default(),
        process_env: std::env::vars_os().collect(),
        env_overlay: Default::default(),
    }
}

fn real_config_service() -> ConfigService {
    let path = dirs::config_dir()
        .expect("OS config dir")
        .join("hyprlayer")
        .join("config.json");
    ConfigService::new(path)
}

#[tokio::test]
#[ignore = "requires opencode installed; run in the session the app launches from"]
async fn a_resolved_binary_spawns_directly() {
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;
    let status = resolver.status(BinaryKind::OpenCode).await;
    let path = status.path.expect("opencode should resolve");
    println!("opencode: {path}");

    let out = tokio::process::Command::new(&path)
        .arg("--version")
        .output()
        .await
        .expect(
            "a resolved binary must be spawnable as-is. If this fails on a \
                 .cmd shim, the app needs its own cmd.exe wrapper again",
        );
    println!(
        "exit={:?} out={:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim()
    );
    assert!(out.status.success(), "`opencode --version` should exit 0");
}

#[tokio::test]
#[ignore = "spawns a real `opencode serve`; run in the session the app launches from"]
async fn opencode_serve_dies_with_its_handle() {
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;
    let bin = resolver
        .status(BinaryKind::OpenCode)
        .await
        .path
        .expect("opencode should resolve");

    let port = free_port();
    let data_dir = tempfile::tempdir().expect("temp data home");

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ])
    .env("XDG_DATA_HOME", data_dir.path())
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = spawn_supervised(&mut cmd).expect("spawn opencode serve");
    println!("port {port}, held pid {:?}", child.id());

    assert!(
        port_settles(port, true, Duration::from_secs(30)).await,
        "`opencode serve` never listened on {port}; the teardown check below \
         would pass vacuously"
    );

    drop(child);

    assert!(
        port_settles(port, false, Duration::from_secs(15)).await,
        "`opencode serve` is still listening on {port} after its handle dropped \
         the process tree outlived us. On Windows this means the job object \
         did not take: we killed the `cmd.exe` shim and the server underneath \
         it is still running."
    );
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn port_settles(port: u16, want_open: bool, budget: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let open = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok();
        if open == want_open {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
#[ignore = "requires a configured hyprlayer install"]
async fn real_config_parses_and_lists_projects() {
    let service = real_config_service();
    let loaded = service.load().await.expect("config should parse");
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .expect("thoughts configured");

    println!("user: {}", thoughts.user);
    println!("backend: {:?}", thoughts.backend.kind());
    println!("projects: {}", thoughts.repo_mappings.len());
    assert!(!thoughts.user.is_empty());
    assert!(thoughts.is_configured());
}

#[tokio::test]
#[ignore = "requires codex installed; run in the session the app launches from"]
async fn real_codex_resolves() {
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;

    let status = resolver.status(BinaryKind::Codex).await;
    println!("codex: {:?}", status.state);
    println!("  path:    {:?}", status.path);
    println!("  version: {:?}", status.version);
    assert_eq!(
        status.state,
        BinaryState::Ok,
        "codex should resolve AND answer --version; a `.cmd` shim that can't be \
         spawned surfaces here as a non-Ok state"
    );
}

#[tokio::test]
#[ignore = "requires hyprlayer/git/claude on PATH"]
async fn real_binaries_resolve() {
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;

    for kind in [BinaryKind::Hyprlayer, BinaryKind::Git, BinaryKind::Claude] {
        let status = resolver.status(kind).await;
        println!("{}: {:?} {:?}", status.name, status.state, status.path);
        assert_eq!(
            status.state,
            BinaryState::Ok,
            "{} should be ok",
            status.name
        );
    }
}

#[tokio::test]
#[ignore = "requires a configured hyprlayer install with mapped repos"]
async fn real_storage_info_round_trips() {
    let service = real_config_service();
    let loaded = service.load().await.expect("config should parse");
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .expect("thoughts configured");
    let repo = thoughts
        .repo_mappings
        .keys()
        .find(|p| std::path::Path::new(p.as_str()).is_dir())
        .expect("at least one mapped repo exists on disk");

    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;

    let info = HyprCli::new(&resolver)
        .storage_info(std::path::Path::new(repo))
        .await
        .expect("storage info should parse");
    println!("repo: {repo}");
    println!("mapped: {:?} backend: {}", info.mapped_name, info.backend);
    println!("schema fields: {}", info.schema.len());
    assert!(info.mapped_name.is_some());
    assert_eq!(info.schema.len(), 10, "thought schema has 10 fields");
}

#[tokio::test]
#[ignore = "requires the hyprlayer CLI installed; run with --ignored"]
async fn scaffolded_root_is_accepted_by_the_real_cli() {
    let git = which::which("git").expect("git installed");
    let hyprlayer = which::which("hyprlayer").expect("hyprlayer CLI installed");
    let tmp = tempfile::tempdir().unwrap();

    let root = tmp.path().join("thoughts");
    hyprlayer_core::thoughts_repo::scaffold(&git, &root, "tester", "repos", "global", &[])
        .await
        .expect("scaffold should succeed");

    let code = tmp.path().join("proj");
    std::fs::create_dir_all(&code).unwrap();
    for args in [
        vec!["init", "-b", "main", code.to_str().unwrap()],
        vec![
            "-C",
            code.to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-m",
            "init",
            "-c",
            "commit.gpgsign=false",
        ],
    ] {
        std::process::Command::new(&git)
            .args(&args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git should run");
    }

    let cfg = tmp.path().join("config.json");
    std::fs::write(
        &cfg,
        serde_json::json!({
            "version": 4,
            "disableUpdateCheck": true,
            "thoughts": {
                "backend": {
                    "kind": "git",
                    "thoughtsRepo": root.to_str().unwrap(),
                    "reposDir": "repos",
                    "globalDir": "global",
                },
                "profiles": {}, "currentProfile": null, "repoMappings": {}, "user": "tester",
            },
            "ai": {"agentTool": "claude"},
            "telemetry": {"mode": "off"},
        })
        .to_string(),
    )
    .unwrap();

    let out = std::process::Command::new(&hyprlayer)
        .current_dir(&code)
        .args([
            "thoughts",
            "init",
            "--yes",
            "--directory",
            "Proj",
            "--config-file",
            cfg.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .output()
        .expect("hyprlayer should run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("exit={:?}\nstderr: {stderr}", out.status.code());

    assert!(
        out.status.success(),
        "the CLI must adopt an app-scaffolded root, but refused it:\n{stderr}"
    );
    assert!(
        code.join("thoughts").join("global").exists(),
        "the CLI should have linked thoughts/global into the scaffolded root"
    );
    assert!(
        root.join("repos").join("Proj").is_dir(),
        "project tree created"
    );
}

#[tokio::test]
#[ignore = "spawns a real `claude`; run in the session the app launches from"]
async fn claude_answers_the_model_catalog_probe() {
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;
    let bin = resolver
        .status(BinaryKind::Claude)
        .await
        .path
        .expect("claude should resolve");
    let path_env = resolver.augmented_path().await;

    let probe = hyprlayer_core::claude::probe::probe(
        std::path::Path::new(&bin),
        &path_env,
        &hyprlayer_core::claude::probe::neutral_cwd(dirs::home_dir().as_deref()),
        &[],
    )
    .await
    .expect("claude should answer list_models");

    for m in &probe.models {
        println!(
            "{:?} -> {:?} [{}] efforts={:?} fast={}",
            m.flag,
            m.resolved_model,
            m.display_name,
            m.supported_effort_levels,
            m.supports_fast_mode
        );
    }
    assert!(!probe.models.is_empty(), "a signed-in CLI offers models");
    assert_eq!(
        probe.models.iter().filter(|m| m.flag.is_none()).count(),
        1,
        "expected exactly one default (no --model) row"
    );
}

#[tokio::test]
#[ignore = "hits the real GitHub API through the signed-in gh"]
async fn github_owners_lists_the_signed_in_account_then_orgs() {
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;
    let gh = resolver
        .status(BinaryKind::Gh)
        .await
        .path
        .expect("gh should resolve");

    let owners = hyprlayer_core::repos::github_owners(std::path::Path::new(&gh), &[])
        .await
        .expect("gh api user / user/orgs should answer");
    println!("owners: {owners:?}");

    assert!(
        !owners.is_empty(),
        "the signed-in account is always present"
    );
    assert!(matches!(
        owners[0].kind,
        hyprlayer_core::repos::GithubOwnerKind::User
    ));
    assert!(!owners[0].login.is_empty());
    assert!(
        owners[1..]
            .iter()
            .all(|o| matches!(o.kind, hyprlayer_core::repos::GithubOwnerKind::Organization)),
        "every owner after the first is an organization"
    );
}

#[tokio::test]
#[ignore = "uses the real git binary; no network"]
async fn push_upstream_sets_tracking_against_a_local_remote() {
    use hyprlayer_core::repos::{
        commit_empty_initial, init_repo, push_upstream, stage_all, GitignoreTemplate, Scaffold,
    };

    let dir = tempfile::tempdir().expect("temp dir");
    let git = which::which("git").expect("git on PATH");
    let repo = dir.path().join("aurora-api");
    let remote = dir.path().join("remote.git");

    std::process::Command::new(&git)
        .args(["init", "--bare", "-b", "main"])
        .arg(&remote)
        .output()
        .expect("bare remote");

    init_repo(&git, &repo, "main", &[]).await.expect("init");
    let written = hyprlayer_core::repos::write_scaffold(
        &repo,
        "aurora-api",
        &Scaffold {
            readme: true,
            gitignore: Some(GitignoreTemplate::Rust),
        },
    )
    .expect("scaffold");
    stage_all(&git, &repo, &[]).await.expect("add -A");
    commit_empty_initial(&git, &repo, &[])
        .await
        .expect("commit");

    // `gh repo create --remote origin` is what wires this up for real.
    std::process::Command::new(&git)
        .arg("-C")
        .arg(&repo)
        .args(["remote", "add", "origin"])
        .arg(&remote)
        .output()
        .expect("remote add");

    push_upstream(&git, &repo, "main", &[])
        .await
        .expect("push -u");

    let out = std::process::Command::new(&git)
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "--abbrev-ref", "main@{upstream}"])
        .output()
        .expect("upstream lookup");
    let upstream = String::from_utf8_lossy(&out.stdout).trim().to_string();
    println!("scaffold={written:?} upstream={upstream}");
    assert_eq!(upstream, "origin/main", "push -u set the tracking branch");

    let remote_files = std::process::Command::new(&git)
        .arg("-C")
        .arg(&remote)
        .args(["ls-tree", "--name-only", "main"])
        .output()
        .expect("ls-tree on the remote");
    let listed = String::from_utf8_lossy(&remote_files.stdout);
    println!("remote tree: {listed:?}");
    assert!(listed.contains("README.md") && listed.contains(".gitignore"));
}

/// Creates a REAL private repository under $HYPRLAYER_GH_SMOKE_OWNER and pushes to
/// it. Deleting it again needs the `delete_repo` scope, which the app never asks
/// for, so this prints the repo it left behind instead of cleaning up.
#[tokio::test]
#[ignore = "creates a real GitHub repository; set HYPRLAYER_GH_SMOKE_OWNER to run"]
async fn create_github_repo_then_push_lands_the_first_commit() {
    use hyprlayer_core::repos::{
        commit_empty_initial, create_github_repo, init_repo, push_upstream, stage_all,
        write_scaffold, GitignoreTemplate, Scaffold,
    };

    let Ok(owner) = std::env::var("HYPRLAYER_GH_SMOKE_OWNER") else {
        eprintln!("HYPRLAYER_GH_SMOKE_OWNER unset — skipping");
        return;
    };
    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;
    let gh = resolver.status(BinaryKind::Gh).await.path.expect("gh");
    let gh = std::path::PathBuf::from(gh);
    let git = which::which("git").expect("git on PATH");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let name = format!("hyprlayer-create-smoke-{stamp}");
    let dir = tempfile::tempdir().expect("temp dir");
    let repo = dir.path().join(&name);

    init_repo(&git, &repo, "main", &[]).await.expect("init");
    let written = write_scaffold(
        &repo,
        &name,
        &Scaffold {
            readme: true,
            gitignore: Some(GitignoreTemplate::Rust),
        },
    )
    .expect("scaffold");
    stage_all(&git, &repo, &[]).await.expect("add -A");
    commit_empty_initial(&git, &repo, &[])
        .await
        .expect("commit");

    let url = create_github_repo(&gh, &repo, &owner, &name, true, &[])
        .await
        .expect("gh repo create");
    println!("created {url} (scaffold: {written:?})");

    push_upstream(&git, &repo, "main", &[])
        .await
        .expect("push -u");

    let field = |q: &str| {
        let out = std::process::Command::new(&gh)
            .args(["api", &format!("repos/{owner}/{name}"), "--jq", q])
            .output()
            .expect("gh api");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(field(".private"), "true", "created private");
    assert_eq!(
        field(".default_branch"),
        "main",
        "default branch follows the local one"
    );

    let listing = std::process::Command::new(&gh)
        .args([
            "api",
            &format!("repos/{owner}/{name}/contents"),
            "--jq",
            ".[].name",
        ])
        .output()
        .expect("gh api contents");
    let files = String::from_utf8_lossy(&listing.stdout);
    println!("remote contents: {files:?}");
    assert!(
        files.contains("README.md") && files.contains(".gitignore"),
        "the scaffold reached the remote in the first push"
    );

    println!("LEFT BEHIND — delete with: gh repo delete {owner}/{name} --yes");
}

// Drives the real `gh repo create` into a failure (an existing name) and checks
// the classifier still recognises it. Creates nothing.
#[tokio::test]
#[ignore = "hits the real GitHub API; set HYPRLAYER_GH_SMOKE_TAKEN=owner/name to run"]
async fn a_taken_name_is_reported_as_taken() {
    use hyprlayer_core::repos::{classify_github_failure, create_github_repo, init_repo};

    let Ok(slug) = std::env::var("HYPRLAYER_GH_SMOKE_TAKEN") else {
        eprintln!("HYPRLAYER_GH_SMOKE_TAKEN unset — skipping");
        return;
    };
    let (owner, name) = slug.split_once('/').expect("owner/name");

    let resolver = BinaryResolver::initialize(&real_environment(), HashMap::new());
    resolver.enrich_with_login_shell().await;
    let gh = std::path::PathBuf::from(resolver.status(BinaryKind::Gh).await.path.expect("gh"));
    let git = which::which("git").expect("git on PATH");

    let dir = tempfile::tempdir().expect("temp dir");
    let repo = dir.path().join("probe");
    init_repo(&git, &repo, "main", &[]).await.expect("init");
    hyprlayer_core::repos::commit_empty_initial(&git, &repo, &[])
        .await
        .expect("commit");

    let err = create_github_repo(&gh, &repo, owner, name, true, &[])
        .await
        .expect_err("the name is already taken");
    let stderr = match &err {
        hyprlayer_core::CoreError::Exec { stderr_tail, .. } => stderr_tail.clone(),
        other => other.to_string(),
    };
    println!("gh said: {stderr:?}");

    let failure = classify_github_failure(&stderr, owner, name);
    println!("classified: {failure:?}");
    assert_eq!(failure.code, "github_name_taken");
    assert!(failure.message.contains(&slug));
    assert!(failure.remedy.is_some());

    assert!(
        hyprlayer_core::repos::origin_url(&git, &repo, &[])
            .await
            .is_none(),
        "a failed create leaves no origin behind, so a retry is a clean create"
    );
}

// The retry path: a repository that gh already created is pushed to, never
// created again. Pushes one empty commit into $HYPRLAYER_GH_SMOKE_TAKEN.
#[tokio::test]
#[ignore = "pushes to a real repository; set HYPRLAYER_GH_SMOKE_TAKEN=owner/name to run"]
async fn an_existing_remote_is_pushed_to_rather_than_recreated() {
    use hyprlayer_core::repos::{origin_url, plan_publish, push_upstream, PublishPlan};

    let Ok(slug) = std::env::var("HYPRLAYER_GH_SMOKE_TAKEN") else {
        eprintln!("HYPRLAYER_GH_SMOKE_TAKEN unset — skipping");
        return;
    };
    let (owner, name) = slug.split_once('/').expect("owner/name");
    let git = which::which("git").expect("git on PATH");
    let dir = tempfile::tempdir().expect("temp dir");
    let repo = dir.path().join(name);

    let cloned = std::process::Command::new(&git)
        .args(["clone", &format!("https://github.com/{slug}.git")])
        .arg(&repo)
        .output()
        .expect("clone");
    assert!(cloned.status.success(), "clone the existing repo");

    let origin = origin_url(&git, &repo, &[]).await;
    println!("origin: {origin:?}");
    let plan = plan_publish(origin.as_deref(), owner, name);
    println!("plan: {plan:?}");
    assert!(
        matches!(plan, PublishPlan::PushOnly { .. }),
        "a repo gh already created must not be created again"
    );

    std::process::Command::new(&git)
        .arg("-C")
        .arg(&repo)
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "retry probe",
        ])
        .output()
        .expect("commit");
    push_upstream(&git, &repo, "main", &[])
        .await
        .expect("the retry push lands");
    println!("pushed to {slug}");
}
