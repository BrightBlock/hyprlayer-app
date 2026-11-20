//! The OpenCode driver against a real `opencode serve`, reached through a runtime on a
//! throwaway profile and run on an OpenCode Zen free model, so a run spends nothing.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::SessionEvent;
use hyprlayer_core::opencode::runtime::{OpenCodeDriver, OpenCodeServer};
use hyprlayer_core::{HyprlayerRuntime, RuntimeEnvironment, RuntimeOptions, RuntimePaths};
use tokio::sync::broadcast;

/// `headless_runtime.rs`'s environment with the real `PATH`, so the resolver finds opencode.
fn environment(home: &Path) -> RuntimeEnvironment {
    let mut process_env = BTreeMap::new();
    process_env.insert(
        OsString::from("SHELL"),
        OsString::from(home.join("no-such-shell")),
    );
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

/// A schema-current config, so `start()` never shells the hyprlayer CLI.
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

/// `HYPRLAYER_OPENCODE_FREE_MODEL`, else the first `opencode/*-free` model the server offers.
async fn free_model(server: &OpenCodeServer) -> String {
    if let Ok(model) = std::env::var("HYPRLAYER_OPENCODE_FREE_MODEL") {
        return model;
    }
    let registry = server
        .provider_models()
        .await
        .expect("opencode lists its models");
    let mut free: Vec<&str> = registry
        .get("opencode")
        .into_iter()
        .flatten()
        .map(|(id, _)| id.as_str())
        .filter(|id| id.ends_with("-free"))
        .collect();
    free.sort_unstable();
    let model = free.first().expect("opencode offers a free model");
    format!("opencode/{model}")
}

/// One OpenCode session on the profile's shared server.
struct Live {
    runtime: Arc<HyprlayerRuntime>,
    server: Arc<OpenCodeServer>,
    cwd: String,
    model: String,
    driver: Arc<OpenCodeDriver>,
    events: broadcast::Receiver<SessionEvent>,
    _root: tempfile::TempDir,
}

impl Live {
    async fn open() -> Self {
        Self::open_with(None).await
    }

    /// `project_config` becomes the work dir's `opencode.json`.
    async fn open_with(project_config: Option<&str>) -> Self {
        let root = tempfile::tempdir().expect("temp profile");
        let paths = paths(root.path());
        write_inputs(&paths);
        let home = root.path().join("home");
        let cwd = root.path().join("work");
        for dir in [&home, &cwd] {
            std::fs::create_dir_all(dir).unwrap();
        }
        if let Some(config) = project_config {
            std::fs::write(cwd.join("opencode.json"), config).unwrap();
        }
        let runtime = HyprlayerRuntime::open(RuntimeOptions::new(environment(&home), paths))
            .await
            .expect("open the runtime");
        runtime.start().await.expect("start the runtime");
        let server = runtime
            .providers()
            .opencode_server()
            .await
            .expect("start opencode serve");
        let model = free_model(&server).await;
        println!("model: {model}");
        let cwd = cwd.to_string_lossy().into_owned();
        let (driver, events) =
            OpenCodeDriver::spawn(server.clone(), &cwd, Some(model.clone()), None)
                .await
                .expect("open an opencode session");
        Self {
            runtime,
            server,
            cwd,
            model,
            driver,
            events,
            _root: root,
        }
    }

    /// Kills the driver and launches another on the same server, as a relaunch in place does.
    async fn relaunch(&mut self, resume: Option<&str>) {
        self.driver.kill();
        let (driver, events) = OpenCodeDriver::spawn(
            self.server.clone(),
            &self.cwd,
            Some(self.model.clone()),
            resume,
        )
        .await
        .expect("relaunch the opencode session");
        self.driver = driver;
        self.events = events;
    }

    async fn close(self) {
        self.driver.kill();
        self.driver.delete_session().await;
        self.runtime.shutdown().await;
    }

    async fn turn(&mut self, text: &str) -> Turn {
        self.driver
            .send_user_turn(text, None, None, None)
            .expect("send turn");
        next_turn(&mut self.events).await
    }
}

#[derive(Debug)]
struct Turn {
    status: Option<String>,
    cost_usd: Option<f64>,
    text: String,
}

async fn next_turn(events: &mut broadcast::Receiver<SessionEvent>) -> Turn {
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(SessionEvent::AssistantText { text: said, .. })) => text.push_str(&said),
            Ok(Ok(SessionEvent::TurnDone {
                turn_status,
                cost_usd,
                ..
            })) => {
                return Turn {
                    status: turn_status,
                    cost_usd,
                    text,
                }
            }
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited mid-turn: {outcome:?}")
            }
            Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(_)) => panic!("event channel closed mid-turn"),
            Err(_) => panic!("the turn did not end within 120s"),
        }
    }
}

/// Every `TurnDone` within `window`, for whatever trails a turn's end.
async fn trailing_turn_ends(
    events: &mut broadcast::Receiver<SessionEvent>,
    window: Duration,
) -> Vec<Option<String>> {
    let mut ends = Vec::new();
    let _ = tokio::time::timeout(window, async {
        loop {
            match events.recv().await {
                Ok(SessionEvent::TurnDone { turn_status, .. }) => ends.push(turn_status),
                Err(broadcast::error::RecvError::Closed) => return,
                _ => {}
            }
        }
    })
    .await;
    ends
}

#[tokio::test]
#[ignore = "requires opencode; runs an opencode/*-free model (or HYPRLAYER_OPENCODE_FREE_MODEL)"]
async fn a_completed_turn_reports_completed_with_cumulative_cost() {
    let mut live = Live::open().await;
    let first = live.turn("Reply with exactly the word: one.").await;
    let second = live.turn("Reply with exactly the word: two.").await;
    let so_far = live.driver.usage_so_far().cost_usd;
    println!("{first:?}\n{second:?}\nusage so far: {so_far:?}");
    live.close().await;
    assert_eq!(first.status.as_deref(), Some("completed"));
    assert_eq!(second.status.as_deref(), Some("completed"));
    let (Some(before), Some(after)) = (first.cost_usd, second.cost_usd) else {
        panic!("a turn reported no cost: {first:?}, {second:?}");
    };
    assert!(
        after >= before,
        "the cost is the session's running total: {before}, then {after}"
    );
    assert_eq!(so_far, Some(after), "usage_so_far reads the same total");
    assert_eq!(after, 0.0, "a free model spends nothing");
}

#[tokio::test]
#[ignore = "requires opencode; runs an opencode/*-free model (or HYPRLAYER_OPENCODE_FREE_MODEL)"]
async fn an_aborted_turn_reports_interrupted() {
    let mut live = Live::open().await;
    live.driver
        .send_user_turn(
            "Without using any tool, count from 1 to 500, one number per line.",
            None,
            None,
            None,
        )
        .expect("send turn");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        match tokio::time::timeout_at(deadline, live.events.recv()).await {
            Ok(Ok(SessionEvent::PartialText { .. })) => break,
            Ok(Ok(SessionEvent::TurnDone { .. })) => panic!("the turn ended before it streamed"),
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited mid-turn: {outcome:?}")
            }
            Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(_)) => panic!("event channel closed mid-turn"),
            Err(_) => panic!("nothing streamed within 120s"),
        }
    }
    live.driver.interrupt().expect("abort");
    let aborted = next_turn(&mut live.events).await;
    // A goal reads an iteration's last status, so anything after the abort matters too.
    let trailing = trailing_turn_ends(&mut live.events, Duration::from_secs(3)).await;
    println!("{aborted:?}, then {trailing:?}");
    live.close().await;
    assert_eq!(aborted.status.as_deref(), Some("interrupted"));
}

#[tokio::test]
#[ignore = "requires opencode; runs an opencode/*-free model (or HYPRLAYER_OPENCODE_FREE_MODEL)"]
async fn a_prompt_after_idle_runs_as_its_own_turn() {
    let mut live = Live::open().await;
    let first = live.turn("Reply with exactly the word: alpha.").await;
    let second = live.turn("Reply with exactly the word: bravo.").await;
    let trailing = trailing_turn_ends(&mut live.events, Duration::from_secs(3)).await;
    println!("{first:?}\n{second:?}\nthen {trailing:?}");
    live.close().await;
    assert_eq!(first.status.as_deref(), Some("completed"));
    assert_eq!(second.status.as_deref(), Some("completed"));
    assert!(
        first.text.to_lowercase().contains("alpha") && second.text.to_lowercase().contains("bravo"),
        "each prompt got its own answer: {first:?}, {second:?}"
    );
    assert!(
        trailing.is_empty(),
        "each prompt ends in one TurnDone: {trailing:?}"
    );
}

/// The id the driver's `Init` reports.
async fn opened_session(events: &mut broadcast::Receiver<SessionEvent>) -> String {
    loop {
        if let SessionEvent::Init {
            provider_session_id,
            ..
        } = events.recv().await.expect("the driver reports its session")
        {
            return provider_session_id;
        }
    }
}

#[tokio::test]
#[ignore = "requires opencode; runs an opencode/*-free model (or HYPRLAYER_OPENCODE_FREE_MODEL)"]
async fn a_resumed_session_keeps_its_earlier_turns() {
    const NONCE: &str = "NONCE-5H8M";
    let mut live = Live::open().await;
    let opened = opened_session(&mut live.events).await;
    let told = live
        .turn(&format!("Remember the nonce {NONCE}. Reply with just: OK."))
        .await;
    live.relaunch(Some(&opened)).await;
    let resumed = opened_session(&mut live.events).await;
    let asked = live
        .turn("What was the nonce I asked you to remember? Reply with just the nonce.")
        .await;
    let trailing = trailing_turn_ends(&mut live.events, Duration::from_secs(3)).await;
    println!("{opened} -> {resumed}\n{told:?}\n{asked:?}\nthen {trailing:?}");
    live.close().await;
    assert_eq!(resumed, opened, "the relaunch continues the same session");
    assert_eq!(asked.status.as_deref(), Some("completed"));
    assert!(
        asked.text.contains(NONCE),
        "the model kept the first turn: {asked:?}"
    );
    assert!(trailing.is_empty(), "one TurnDone per turn: {trailing:?}");
}

#[tokio::test]
#[ignore = "requires opencode; runs an opencode/*-free model (or HYPRLAYER_OPENCODE_FREE_MODEL)"]
async fn a_send_right_after_a_mid_turn_restart_gets_its_own_reply() {
    const NONCE: &str = "NONCE-3K9W";
    let mut live = Live::open().await;
    let opened = opened_session(&mut live.events).await;
    live.turn(&format!("Remember the nonce {NONCE}. Reply with just: OK."))
        .await;
    live.driver
        .send_user_turn(
            "Without using any tool, count from 1 to 500, one number per line.",
            None,
            None,
            None,
        )
        .expect("send turn");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while !matches!(
        tokio::time::timeout_at(deadline, live.events.recv()).await,
        Ok(Ok(SessionEvent::PartialText { .. }))
    ) {
        assert!(tokio::time::Instant::now() < deadline, "nothing streamed");
    }
    // The aborted turn's late idle lands while this one runs.
    live.relaunch(Some(&opened)).await;
    let asked = live
        .turn("What was the nonce I asked you to remember? Reply with just the nonce.")
        .await;
    let trailing = trailing_turn_ends(&mut live.events, Duration::from_secs(3)).await;
    println!("{asked:?}\nthen {trailing:?}");
    live.close().await;
    assert_eq!(asked.status.as_deref(), Some("completed"));
    assert!(asked.text.contains(NONCE), "its own reply: {asked:?}");
    assert!(trailing.is_empty(), "one TurnDone per turn: {trailing:?}");
}

#[tokio::test]
#[ignore = "requires opencode; runs an opencode/*-free model (or HYPRLAYER_OPENCODE_FREE_MODEL)"]
async fn a_turn_whose_permission_is_denied_ends() {
    let mut live = Live::open_with(Some(r#"{"permission":{"bash":"ask"}}"#)).await;
    live.driver
        .send_user_turn(
            "Run the bash command `echo denied-probe` and tell me its output.",
            None,
            None,
            None,
        )
        .expect("send turn");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let request_id = loop {
        match tokio::time::timeout_at(deadline, live.events.recv()).await {
            Ok(Ok(SessionEvent::PermissionAsk { request_id, .. })) => break request_id,
            Ok(Ok(SessionEvent::TurnDone { .. })) => panic!("the turn ended before it asked"),
            Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(_)) => panic!("event channel closed"),
            Err(_) => panic!("no permission ask within 120s"),
        }
    };
    live.driver
        .respond_permission(&request_id, false, None)
        .expect("deny");
    let denied = next_turn(&mut live.events).await;
    let trailing = trailing_turn_ends(&mut live.events, Duration::from_secs(3)).await;
    println!("{denied:?}\nthen {trailing:?}");
    live.close().await;
    assert_eq!(denied.status.as_deref(), Some("completed"));
    assert!(trailing.is_empty(), "one TurnDone per turn: {trailing:?}");
}
