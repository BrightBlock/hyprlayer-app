//! With app actions in a CLI, an agent's spawn_agent and approve_plan start a PTY agent in the
//! default engine: the stage's prompt is composed and delivered with the caller's header after it,
//! and the stage's deny list still applies. A pinned harness, a stage on an engine without its
//! guardrails, an unhooked TUI for a non-bypass caller, or the setting off keeps a session.
//!
//! The lead is `goal_stub`'s claude stand-in and the engine `agent_stub`'s PTY claude.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::{AgentControlLevel, PermissionMode, PtyAgentState, PtyLaunchSession};
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::agent_stub::{
    agent_env, delivered, env_var, install, launch_log, lines, set_engine, set_level, tool_failed,
    tool_text, type_line, update_settings, McpDriver, PTY_CLAUDE,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::GOAL_CLAUDE;
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(30);
const PLAN: &str =
    "---\ntitle: Spawned plan\ntype: plan\nstatus: draft\nauthor: e2e\n---\n\n# Spawned plan\n\n## Phase 1\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_spawn_in_cli() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("agent_spawn_in_cli", &sandbox);
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Spawns").await;
    let ws = text(&view, "/item/id");
    set_level(&mut j, &c, AgentControlLevel::Full).await;
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    update_settings(&mut j, &c, |s| {
        s.multiplexer.enabled = true;
        s.agent_control.max_live_children = 10;
    })
    .await;
    let lead = start_lead(&mut j, &c, &ws).await;
    let mut mcp = driver(&mut j, &sandbox, &lead).await;
    let header = format!("[from agent \"Lead\" · {lead}]");

    j.step("spawn_agent opens the default engine");
    let spawned = parsed(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_REVIEW", "title": "Reviewer" }),
        )
        .await,
    );
    let terminal = spawned["terminalId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    j.alias(&terminal, "$REVIEWER");
    let log = launch_log(&sandbox, &terminal);
    let note = format!(
        "Your spawner is Lead (session {lead}). Your final reply is your report. To message it \
         mid-task, use mcp__hyprlayer__message_agent with session_id {lead}, not SendMessage. \
         hyprlayer refuses the message if that session runs in a higher permission mode than \
         yours."
    );
    let expected = delivered(&format!("E2E_REVIEW\n\n{header}\n\n{note}"));
    let stdin = log.join("stdin.log");
    let typed = eventually(IO, || lines(&stdin) == expected).await;
    j.guard(
        "a PTY agent, not a session, typed the prompt with the caller's header and spawner note after it",
        !terminal.is_empty() && spawned.get("sessionId").is_none() && typed,
        json!({ "engineId": spawned["engineId"], "hasSessionId": spawned.get("sessionId").is_some(), "matchesThePaste": typed }),
    );

    j.step("archiving a workstream ends the PTY agent spawned into it, and only that one");
    let shelf = create_workstream(&mut j, &c, &project, "Shelved").await;
    let shelf_slug = text(&shelf, "/item/slug");
    let shelf_tree = PathBuf::from(text(&shelf, "/item/worktreePath"));
    let shelf = text(&shelf, "/item/id");
    let shelved = parsed(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_SHELVED", "title": "Shelved", "workstream_id": shelf }),
        )
        .await,
    );
    let shelved = shelved["terminalId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    j.alias(&shelved, "$SHELVED");
    let pid_file = launch_log(&sandbox, &shelved).join("pid");
    let started = eventually(IO, || pid_file.is_file()).await;
    let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
    let shelved_session = c
        .list_pty_agents(Some(shelf.clone()))
        .await
        .expect("list_pty_agents")
        .into_iter()
        .find_map(|a| a.provider_session_id)
        .unwrap_or_default();
    j.alias(&shelved_session, "$SHELVED_SESSION");

    let other = OtherFilesystem::mount(&sandbox);
    j.alias(other.root.display().to_string(), "$OTHER_FS");
    let archive_root = repo.join("archived-contexts");
    std::os::unix::fs::symlink(&other.root, &archive_root).expect("link archived-contexts");
    let archived_tree = archive_root.join(&shelf_slug);
    std::fs::create_dir_all(&archived_tree).expect("block the archive destination");
    let refused = j
        .call(
            &c,
            Request::ArchiveWorkstream {
                id: shelf.clone(),
                skip_scripts: true,
            },
        )
        .await
        .err()
        .map(|e| e.code);
    let status = workstream_status(&mut j, &c, &project, &shelf).await;
    let still_running = alive(pid.trim())
        && c.list_pty_agents(Some(shelf.clone()))
            .await
            .is_ok_and(|a| a.iter().any(|a| a.terminal_id == shelved));
    j.guard(
        "an archive that cannot move the worktree fails before stopping anything: the workstream stays active, its agent running, its worktree in place",
        refused.is_some() && status == "active" && still_running && shelf_tree.join(".git").exists(),
        json!({ "refused": refused.is_some(), "status": status, "agentRunning": still_running, "worktreeInPlace": shelf_tree.join(".git").exists() }),
    );

    std::fs::remove_dir(&archived_tree).expect("clear the archive destination");
    j.call(
        &c,
        Request::ArchiveWorkstream {
            id: shelf.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let status = workstream_status(&mut j, &c, &project, &shelf).await;
    let listed_at = git_worktrees(&repo);
    let tracked = listed_at.iter().any(|p| same_dir(p, &archived_tree));
    j.note("archive_filesystem", json!(other.kind));
    j.guard(
        "archiving onto another filesystem copies the worktree, repoints git at the copy and removes the original",
        status == "archived"
            && other.distinct
            && archived_tree.join(".git").is_file()
            && !shelf_tree.exists()
            && tracked,
        json!({ "status": status, "otherFilesystem": other.distinct, "archived": archived_tree.join(".git").is_file(), "originalGone": !shelf_tree.exists(), "gitTracksTheCopy": tracked }),
    );
    let exited = eventually(IO, || !alive(pid.trim())).await;
    let listed: Vec<String> = c
        .list_pty_agents(None)
        .await
        .expect("list_pty_agents")
        .into_iter()
        .map(|a| a.terminal_id)
        .collect();
    let lead_live = c
        .list_active_sessions()
        .await
        .expect("list_active_sessions")
        .iter()
        .any(|a| a.session.id == lead);
    j.guard(
        "the shelved agent's process exits and leaves the list; the lead and its reviewer in another workstream stay up",
        started
            && exited
            && !listed.contains(&shelved)
            && listed.contains(&terminal)
            && lead_live,
        json!({ "started": started, "exited": exited, "shelvedListed": listed.contains(&shelved), "reviewerListed": listed.contains(&terminal), "leadLive": lead_live }),
    );
    j.step(
        "a spawn into a folder claude has yet to trust waits on the person, then sends its prompt",
    );
    set_engine(&mut j, &c, "claude-code", &pty_claude, &["--stub-trust"]).await;
    let truster = terminal_of(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_TRUSTED", "title": "Truster" }),
        )
        .await,
    );
    j.alias(&truster, "$TRUSTER");
    let wake = wake_id(
        &mcp.call(
            "wake_when",
            json!({ "condition": { "kind": "terminal", "terminalId": truster, "until": "needs_you" } }),
        )
        .await,
    );
    let woke = lead_turn(&c, &lead, &format!("wake {wake}]")).await;
    let state = pty_state(&c, &ws, &truster).await;
    let stdin = launch_log(&sandbox, &truster).join("stdin.log");
    let untouched = lines(&stdin).is_empty();
    type_line(&c, &truster, "trust").await;
    let mut expected = vec!["trust".to_string()];
    expected.extend(delivered(&format!("E2E_TRUSTED\n\n{header}\n\n{note}")));
    let typed = eventually(IO, || lines(&stdin) == expected).await;
    j.guard(
        "at the trust prompt the agent awaits permission and the spawner's needs_you wake fires; nothing is typed until the person trusts the folder, then the prompt arrives",
        woke.as_deref()
            .is_some_and(|t| t.ends_with(&format!("terminal {truster} \"Truster\" needs you")))
            && state == Some(PtyAgentState::AwaitingPermission)
            && untouched
            && typed,
        json!({
            "wakeFired": woke.is_some(),
            "wakeSaysNeedsYou": woke.as_deref().is_some_and(|t| t.ends_with("needs you")),
            "state": state,
            "nothingTypedWhileAsked": untouched,
            "promptAfterTrust": typed,
        }),
    );

    mcp.call("kill_agent", json!({ "terminal_id": truster }))
        .await;
    assert!(
        reaches(&c, &ws, &truster, None).await,
        "the truster outlived kill_agent"
    );

    j.step("an agent that exits at the trust prompt is reported to its spawner");
    let quitter = terminal_of(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_QUIT", "title": "Quitter" }),
        )
        .await,
    );
    j.alias(&quitter, "$QUITTER");
    let asked = reaches(&c, &ws, &quitter, Some(PtyAgentState::AwaitingPermission)).await;
    type_line(&c, &quitter, "no").await;
    let report = lead_turn(
        &c,
        &lead,
        &format!("terminal {quitter} exited before it was ready"),
    )
    .await;
    let stdin = lines(&launch_log(&sandbox, &quitter).join("stdin.log"));
    j.guard(
        "the spawner reads a hyprlayer turn saying its prompt was never delivered, and the prompt was never typed",
        asked
            && report.as_deref().is_some_and(|t| {
                t.starts_with("[from hyprlayer · wake ")
                    && t.ends_with("so the prompt you spawned it with was never delivered")
            })
            && stdin == ["no"],
        json!({
            "asked": asked,
            "reported": report.as_deref().and_then(|t| t.split_once("\n\n")).map(|(_, body)| body.replace(&quitter, "$QUITTER")),
            "typed": stdin,
        }),
    );
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    j.step("an unhooked engine falls back to a session for a non-bypass caller");
    set_engine(
        &mut j,
        &c,
        "claude-code",
        &pty_claude,
        &["--settings", "{}"],
    )
    .await;
    let fallback = parsed(
        &mcp.call("spawn_agent", json!({ "prompt": "E2E_FALLBACK" }))
            .await,
    );
    let ptys = c.list_pty_agents(Some(ws)).await.expect("list_pty_agents");
    // Only the reviewer's: the unhooked one was closed before the spawn returned.
    let closed = ptys.len() == 1;
    j.guard(
        "a claude that came up unhooked is closed, and the spawn becomes a session instead of a refusal",
        fallback["sessionId"].is_string() && fallback.get("terminalId").is_none() && closed,
        json!({ "result": fallback.get("sessionId").map(|_| "session"), "error": fallback.get("error"), "livePtys": ptys.len() }),
    );
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;

    j.step("approve_plan builds the plan in the default engine");
    set_mode(&mut j, &c, &lead, PermissionMode::BypassPermissions).await;
    let plan = write_plan(&mut j, &c, &project).await;
    c.agent_resize(terminal.clone(), 150, 45)
        .await
        .expect("agent_resize");
    let approved = parsed(&mcp.call("approve_plan", json!({ "plan": plan })).await);
    let implement = approved["terminalId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    j.alias(&implement, "$IMPLEMENT");
    let log = launch_log(&sandbox, &implement);
    let stdin = log.join("stdin.log");
    let opened = eventually(IO, || {
        lines(&stdin)
            .first()
            .is_some_and(|line| line.starts_with("/implement_plan \x1b[200~"))
    })
    .await;
    let argv = lines(&log.join("argv"));
    let denies_push = argv
        .windows(2)
        .any(|w| w[0] == "--disallowedTools" && w[1] == "Bash(git push:*)");
    let bypasses = argv.iter().any(|a| a == "--dangerously-skip-permissions");
    let active = eventually(IO, || plan_status(&plan) == "active").await;
    j.guard(
        "/implement_plan is typed with the rest pasted as its arguments, under Implement's deny list, in the bypass caller's mode, and the plan turns active",
        opened && denies_push && bypasses && active,
        json!({ "opensWithTheSkill": opened, "deniesPush": denies_push, "bypasses": bypasses, "planActive": active }),
    );

    j.step("a spawn after a desktop fitted an agent tab starts at that tab's size");
    let first = lines(&launch_log(&sandbox, &terminal).join("size"));
    let fitted = lines(&log.join("size"));
    j.guard(
        "the reviewer, spawned before any fit, starts at 120x32; the implementer, spawned after a 150x45 tab fit, starts at 150x45",
        first == ["32 120"] && fitted == ["45 150"],
        json!({ "reviewer": first, "implementer": fitted }),
    );

    j.step("a stage on an engine that can't apply its guardrails keeps a session");
    update_settings(&mut j, &c, |s| {
        s.multiplexer.default_engine_id = "codex".into()
    })
    .await;
    let codex = parsed(&mcp.call("validate_plan", json!({ "plan": plan })).await);
    j.guard(
        "validate_plan with codex as the default engine starts a validate session, whose posture is read-only",
        codex["sessionId"].is_string() && codex["stage"] == json!("validate"),
        json!({ "hasSessionId": codex["sessionId"].is_string(), "stage": codex["stage"], "error": codex.get("error") }),
    );
    update_settings(&mut j, &c, |s| {
        s.multiplexer.default_engine_id = "claude-code".into()
    })
    .await;

    j.step("a pinned harness keeps a session");
    let pinned = parsed(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_PINNED", "harness": "claude" }),
        )
        .await,
    );
    j.guard(
        "the caller's harness choice outranks the engine",
        pinned["sessionId"].is_string() && pinned.get("terminalId").is_none(),
        json!({ "hasSessionId": pinned["sessionId"].is_string(), "hasTerminalId": pinned.get("terminalId").is_some() }),
    );

    j.step("with app actions in a chat, spawns stay sessions");
    update_settings(&mut j, &c, |s| s.multiplexer.actions_in_cli = false).await;
    let chat = parsed(
        &mcp.call("spawn_agent", json!({ "prompt": "E2E_CHAT" }))
            .await,
    );
    j.guard(
        "spawn_agent starts a composer session",
        chat["sessionId"].is_string() && chat.get("terminalId").is_none(),
        json!({ "hasSessionId": chat["sessionId"].is_string(), "hasTerminalId": chat.get("terminalId").is_some() }),
    );

    j.step("an archived workstream's agents never start again, across a server restart");
    let relaunch = c
        .pty_launch_stream(
            shelf.clone(),
            "claude-code".into(),
            PtyLaunchSession {
                session_id: Some(shelved_session.clone()),
                resume: true,
                bypass: true,
                model: None,
                permission_mode: None,
                effort: None,
                first_prompt: None,
            },
            80,
            24,
        )
        .await
        .err()
        .map(|e| e.code);
    j.guard(
        "a desktop tab's resume of the archived agent is refused",
        relaunch.as_deref() == Some("workstream_archived"),
        json!({ "refusal": relaunch }),
    );
    let launched_before = launch_count(&sandbox);
    drop(mcp);
    let stopped = server.shutdown(&c).await;
    let server = Server::start(&sandbox).await;
    let c = server.client();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let ptys = c
        .list_pty_agents(Some(shelf.clone()))
        .await
        .expect("list_pty_agents");
    let sessions = c
        .list_active_sessions()
        .await
        .expect("list_active_sessions")
        .iter()
        .filter(|a| a.session.workstream_id == shelf)
        .count();
    let relaunch = c
        .pty_launch_stream(
            shelf.clone(),
            "claude-code".into(),
            PtyLaunchSession {
                session_id: Some(shelved_session),
                resume: true,
                bypass: true,
                model: None,
                permission_mode: None,
                effort: None,
                first_prompt: None,
            },
            80,
            24,
        )
        .await
        .err()
        .map(|e| e.code);
    let launched = launch_count(&sandbox) - launched_before;
    j.guard(
        "after the restart nothing runs in the archived workstream and a resume is still refused",
        stopped.status.success()
            && ptys.is_empty()
            && sessions == 0
            && relaunch.as_deref() == Some("workstream_archived")
            && launched == 0,
        json!({ "cleanStop": stopped.status.success(), "ptyAgents": ptys.len(), "liveSessions": sessions, "refusal": relaunch, "newLaunches": launched }),
    );

    j.step("restoring from the other filesystem moves the worktree back");
    let restored = j
        .call(
            &c,
            Request::RestoreWorkstream {
                id: shelf,
                target: None,
            },
        )
        .await
        .map(|v| v["item"]["status"].clone());
    let tracked = git_worktrees(&repo)
        .iter()
        .any(|p| same_dir(p, &shelf_tree));
    j.guard(
        "the workstream is active again, its worktree back in place and known to git, and the copy gone",
        restored.as_ref().is_ok_and(|s| s == "active")
            && shelf_tree.join(".git").is_file()
            && !archived_tree.exists()
            && tracked,
        json!({ "status": restored.ok(), "back": shelf_tree.join(".git").is_file(), "copyGone": !archived_tree.exists(), "gitTracksIt": tracked }),
    );

    j.finish();
}

async fn workstream_status(j: &mut Journey, c: &RpcClient, project: &str, id: &str) -> String {
    let all = j
        .call(
            c,
            Request::ListWorkstreams {
                project_path: project.to_string(),
                include_archived: true,
            },
        )
        .await
        .expect("list_workstreams");
    all.as_array()
        .and_then(|rows| rows.iter().find(|r| text(r, "/item/id") == id))
        .map(|r| text(r, "/item/status"))
        .unwrap_or_default()
}

fn launch_count(sandbox: &Sandbox) -> usize {
    std::fs::read_dir(launch_log(sandbox, ""))
        .map(|d| d.count())
        .unwrap_or_default()
}

fn git_worktrees(repo: &Path) -> Vec<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .expect("git worktree list");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// A directory on a filesystem other than the sandbox's: tmpfs on Linux, a disk image on macOS.
struct OtherFilesystem {
    root: PathBuf,
    kind: &'static str,
    distinct: bool,
    image: Option<PathBuf>,
}

impl OtherFilesystem {
    fn mount(sandbox: &Sandbox) -> Self {
        use std::os::unix::fs::MetadataExt;
        let (root, kind, image) = if cfg!(target_os = "macos") {
            let image = sandbox.root.join("other-fs.dmg");
            let root = sandbox.root.join("other-fs");
            let ok = |cmd: &mut std::process::Command| cmd.status().is_ok_and(|s| s.success());
            assert!(
                ok(std::process::Command::new("hdiutil")
                    .args(["create", "-quiet", "-size", "64m", "-fs", "HFS+", "-volname", "hl-e2e"])
                    .arg(&image))
                    && ok(std::process::Command::new("hdiutil")
                        .args(["attach", "-quiet", "-nobrowse", "-mountpoint"])
                        .arg(&root)
                        .arg(&image)),
                "mount a disk image for a second filesystem"
            );
            (root, "a macOS disk image", Some(image))
        } else {
            let root = PathBuf::from(format!("/dev/shm/hl-e2e-{}", std::process::id()));
            std::fs::create_dir_all(&root).expect("a directory on tmpfs");
            (root, "tmpfs", None)
        };
        let dev = |p: &Path| std::fs::metadata(p).map(|m| m.dev()).ok();
        let distinct = dev(&root).is_some() && dev(&root) != dev(&sandbox.root);
        Self {
            root,
            kind,
            distinct,
            image,
        }
    }
}

impl Drop for OtherFilesystem {
    fn drop(&mut self) {
        if self.image.is_some() {
            let _ = std::process::Command::new("hdiutil")
                .args(["detach", "-quiet", "-force"])
                .arg(&self.root)
                .status();
        } else {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// A titled composer lead, its first turn done, so the header naming it is fixed.
async fn start_lead(j: &mut Journey, c: &RpcClient, ws: &str) -> String {
    let (session, _prompt, _stream_id, _events) = c
        .start_stage_session_stream(
            ws.to_string(),
            Stage::General,
            "E2E_START".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode: Some(PermissionMode::Default),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$LEAD");
    j.call(
        c,
        Request::SetSessionTitle {
            session_id: session.id.clone(),
            title: Some("Lead".into()),
        },
    )
    .await
    .expect("set_session_title");
    j.call(
        c,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let log = c
            .get_session_log(session.id.clone())
            .await
            .expect("get_session_log");
        if log
            .iter()
            .any(|e| matches!(e.event, SessionEvent::TurnDone { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    session.id
}

/// The MCP face under the lead's launch env, initialized.
async fn driver(j: &mut Journey, sandbox: &Sandbox, session_id: &str) -> McpDriver {
    eventually(IO, || {
        !env_var(&agent_env(sandbox, session_id), "HYPRLAYER_AGENT_TOKEN").is_empty()
    })
    .await;
    let env = agent_env(sandbox, session_id);
    j.alias(env_var(&env, "HYPRLAYER_AGENT_TOKEN"), "$LEAD_TOKEN");
    let mut mcp = McpDriver::start(&env, false);
    mcp.initialize("2025-06-18").await;
    mcp
}

async fn set_mode(j: &mut Journey, c: &RpcClient, session_id: &str, mode: PermissionMode) {
    j.call(
        c,
        Request::SetSessionPermissionMode {
            session_id: session_id.to_string(),
            mode: Some(mode),
        },
    )
    .await
    .expect("set_session_permission_mode");
}

/// A draft plan indexed in the project's thoughts; returns its path.
async fn write_plan(j: &mut Journey, c: &RpcClient, project: &str) -> String {
    let meta = j
        .call(
            c,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.to_string()),
                    scope: "shared".into(),
                    doc_type: "plan".into(),
                    title: "Spawned plan".into(),
                    date: "2026-01-01".into(),
                    ticket: None,
                    tags: vec![],
                },
            },
        )
        .await
        .expect("create_doc");
    let path = j
        .call(
            c,
            Request::ResolveDocPath {
                id: text(&meta, "/id"),
            },
        )
        .await
        .expect("resolve_doc_path");
    let path = text(&path, "");
    if let Some(dir) = Path::new(&path).parent().and_then(Path::parent) {
        j.alias(dir.display().to_string(), "$SHARED");
    }
    std::fs::write(&path, PLAN).expect("write the plan");
    path
}

fn plan_status(plan: &str) -> String {
    std::fs::read_to_string(plan)
        .ok()
        .and_then(|doc| {
            doc.lines()
                .find_map(|l| l.strip_prefix("status: ").map(str::to_string))
        })
        .unwrap_or_default()
}

fn alive(pid: &str) -> bool {
    !pid.is_empty()
        && std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

fn terminal_of(result: &Value) -> String {
    text(&parsed(result), "/terminalId")
}

fn wake_id(result: &Value) -> String {
    text(&parsed(result), "/wake/id")
}

async fn pty_state(c: &RpcClient, ws: &str, terminal: &str) -> Option<PtyAgentState> {
    c.list_pty_agents(Some(ws.to_string()))
        .await
        .expect("list_pty_agents")
        .into_iter()
        .find(|a| a.terminal_id == terminal)
        .map(|a| a.state)
}

async fn reaches(c: &RpcClient, ws: &str, terminal: &str, want: Option<PtyAgentState>) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        if pty_state(c, ws, terminal).await == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// The first user turn in the lead's log that contains `needle`.
async fn lead_turn(c: &RpcClient, lead: &str, needle: &str) -> Option<String> {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let found = c
            .get_session_log(lead.to_string())
            .await
            .expect("get_session_log")
            .into_iter()
            .find_map(|e| match e.event {
                SessionEvent::UserMessage { text, .. } if text.contains(needle) => Some(text),
                _ => None,
            });
        if found.is_some() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

fn parsed(result: &Value) -> Value {
    if tool_failed(result) {
        return json!({ "error": tool_text(result) });
    }
    serde_json::from_str(&tool_text(result)).unwrap_or(Value::Null)
}
