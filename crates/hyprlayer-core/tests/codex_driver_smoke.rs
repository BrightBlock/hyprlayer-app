use std::time::Duration;

use hyprlayer_core::claude::{ExitOutcome, PermissionMode, SessionEvent};
use hyprlayer_core::codex::{CodexDriver, SpawnConfig};

fn spawn_config(bin: std::path::PathBuf, permission_mode: PermissionMode) -> SpawnConfig {
    SpawnConfig {
        ephemeral: true,
        ..persisted_config(bin, permission_mode)
    }
}

fn persisted_config(bin: std::path::PathBuf, permission_mode: PermissionMode) -> SpawnConfig {
    SpawnConfig {
        codex_bin: bin,
        cwd: std::env::temp_dir(),
        permission_mode,
        prior_user_turns: 0,
        add_dirs: vec![],
        resume: None,
        model: None,
        effort: None,
        ephemeral: false,
        env: std::env::vars().collect(),
    }
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes a real API call"]
async fn drives_a_real_no_tool_turn_to_completion() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) = CodexDriver::spawn(SpawnConfig {
        model: Some("gpt-5.4".into()),
        effort: Some("low".into()),
        ..spawn_config(codex, PermissionMode::Plan)
    })
    .await
    .expect("spawn codex app-server");

    let thread_id = match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        Ok(Ok(SessionEvent::Init {
            claude_session_id,
            model,
        })) => {
            println!("init: thread={claude_session_id} model={model:?}");
            assert!(
                !claude_session_id.is_empty(),
                "codex should mint a thread id"
            );
            assert_eq!(model.as_deref(), Some("gpt-5.4"));
            claude_session_id
        }
        other => panic!("expected Init first, got {other:?}"),
    };
    assert_eq!(driver.thread_id(), thread_id);

    driver
        .send_user_turn(
            "Reply with exactly the word: pong. Nothing else.",
            None,
            None,
        )
        .expect("send turn");

    let mut text = String::new();
    let mut streamed = false;
    let mut turn_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);

    while !turn_done {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(SessionEvent::AssistantText { text: t, .. })) => text.push_str(&t),
            Ok(Ok(SessionEvent::PartialText { text: t, .. })) => {
                streamed = true;
                print!("{t}");
            }
            Ok(Ok(SessionEvent::TurnDone { .. })) => {
                println!("\nturn done");
                turn_done = true;
            }
            Ok(Ok(SessionEvent::Exited { outcome, code, .. })) => {
                panic!("session exited before completing the turn: {outcome:?} code={code:?}");
            }
            Ok(Ok(other)) => println!("event: {other:?}"),
            Ok(Err(_)) => break,
            Err(_) => panic!("turn did not finish within 120s"),
        }
    }

    assert!(turn_done, "the turn should have completed");
    assert!(
        streamed,
        "agentMessage deltas should have streamed. If this fails the \
         `item/agentMessage/delta` notification was renamed"
    );
    assert!(
        text.to_lowercase().contains("pong"),
        "assistant should have said pong, got: {text:?}"
    );

    driver.kill();
    let exited = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(SessionEvent::Exited { outcome, .. }) = events.recv().await {
                return outcome;
            }
        }
    })
    .await
    .expect("session should exit after kill");
    println!("exited after kill: {exited:?}");
    assert_eq!(
        exited,
        ExitOutcome::Killed,
        "an app-initiated kill is Killed"
    );
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes a real API call"]
async fn tool_calls_and_usage_reach_the_transcript() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) =
        CodexDriver::spawn(spawn_config(codex, PermissionMode::BypassPermissions))
            .await
            .expect("spawn codex app-server");

    driver
        .send_user_turn(
            "Run the shell command `echo hyprlayer-smoke` and then reply with just: done",
            None,
            None,
        )
        .expect("send turn");

    let mut saw_bash = false;
    let mut saw_result = false;
    let mut saw_usage = false;
    let mut saw_window = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(SessionEvent::ToolUse { name, input, .. })) => {
                println!("tool: {name} {input}");
                if name == "Bash" {
                    saw_bash = true;
                    let v: serde_json::Value = serde_json::from_str(&input).expect("tool input");
                    assert!(v.get("command").is_some(), "Bash input needs a `command`");
                }
            }
            Ok(Ok(SessionEvent::ToolResult {
                content, is_error, ..
            })) => {
                println!("result (error={is_error}): {content}");
                saw_result |= content.contains("hyprlayer-smoke");
            }
            Ok(Ok(SessionEvent::ContextWindow { tokens })) => {
                println!("context window: {tokens}");
                assert!(tokens > 0);
                saw_window = true;
            }
            Ok(Ok(SessionEvent::Usage { context_tokens })) => {
                saw_usage = true;
                assert!(
                    !saw_window || context_tokens > 0,
                    "usage should report a real reading"
                );
            }
            Ok(Ok(SessionEvent::TurnDone { .. })) => break,
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited mid-turn: {outcome:?}")
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("event channel closed mid-turn"),
            Err(_) => panic!("turn did not finish within 180s"),
        }
    }

    driver.kill();
    assert!(
        saw_bash,
        "the shell call should surface as a `Bash` ToolUse"
    );
    assert!(
        saw_result,
        "the command's output should surface as a ToolResult"
    );
    assert!(saw_usage, "thread/tokenUsage/updated should feed the meter");
    assert!(
        saw_window,
        "the model's context window is the meter's denominator"
    );
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn rewind_to_ordinal_truncates_model_memory() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) = CodexDriver::spawn(persisted_config(codex, PermissionMode::Plan))
        .await
        .expect("spawn codex app-server");

    run_turn(&driver, &mut events, "Remember the word ALPHA. Reply: ok.").await;
    run_turn(&driver, &mut events, "Remember the word BRAVO. Reply: ok.").await;
    run_turn(
        &driver,
        &mut events,
        "Remember the word CHARLIE. Reply: ok.",
    )
    .await;

    driver
        .rewind_to_ordinal(2)
        .await
        .expect("codex should accept the rollback");

    let recalled = run_turn(
        &driver,
        &mut events,
        "Which of ALPHA, BRAVO, CHARLIE did I ask you to remember? List only those you were told.",
    )
    .await;
    println!("recalled: {recalled}");
    driver.kill();

    let recalled = recalled.to_uppercase();
    assert!(
        recalled.contains("ALPHA"),
        "turn 1 should survive: {recalled:?}"
    );
    assert!(
        !recalled.contains("BRAVO") && !recalled.contains("CHARLIE"),
        "the rewound turns should be forgotten: {recalled:?}"
    );
}

#[tokio::test]
#[ignore = "requires a logged-in codex; starts threads but makes no model call"]
async fn every_permission_mode_starts_a_thread() {
    let codex = which::which("codex").expect("codex on PATH");
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::BypassPermissions,
    ] {
        let (driver, _events) = CodexDriver::spawn(spawn_config(codex.clone(), mode))
            .await
            .unwrap_or_else(|e| panic!("{mode:?} should start a thread, got: {e}"));
        assert!(
            !driver.thread_id().is_empty(),
            "{mode:?} produced no thread id"
        );
        println!("{mode:?}: thread {}", driver.thread_id());
        driver.kill();
    }
}

#[tokio::test]
#[ignore = "requires codex installed; no API call, but spawns an app-server"]
async fn probes_report_account_and_models() {
    let codex = which::which("codex").expect("codex on PATH");
    let path = std::env::var_os("PATH").unwrap_or_default();

    let probe = hyprlayer_core::codex::probe::probe(&codex, &path)
        .await
        .expect("account/read + model/list should answer");
    println!("auth: {:?}", probe.auth);
    println!("{} models", probe.models.len());
    for m in &probe.models {
        println!(
            "  {}: {} (efforts: {:?})",
            m.id, m.display_name, m.supported_efforts
        );
    }

    if probe.auth.signed_in() {
        assert!(
            !probe.models.is_empty(),
            "a signed-in codex should offer models"
        );
        assert!(probe
            .models
            .iter()
            .all(|m| !m.id.is_empty() && !m.display_name.is_empty()));
    }
}

#[tokio::test]
#[ignore = "requires a signed-in codex; spawns several app-servers"]
async fn concurrent_probes_never_report_a_false_empty() {
    let codex = which::which("codex").expect("codex on PATH");
    let path = std::env::var_os("PATH").unwrap_or_default();

    let mut tasks = Vec::new();
    for _ in 0..4 {
        let (c, p) = (codex.clone(), path.clone());
        tasks.push(tokio::spawn(async move {
            hyprlayer_core::codex::probe::probe(&c, &p).await
        }));
    }

    for (i, t) in tasks.into_iter().enumerate() {
        let probe = t.await.expect("probe task").expect("probe should answer");
        println!(
            "probe {i}: signed_in={} models={}",
            probe.auth.signed_in(),
            probe.models.len()
        );
        assert!(probe.auth.signed_in(), "probe {i} lost the account");
        assert!(!probe.models.is_empty(), "probe {i} lost the model list");
    }
}

async fn run_turn(
    driver: &CodexDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    text: &str,
) -> String {
    driver.send_user_turn(text, None, None).expect("send turn");
    let mut finalized = String::new();
    let mut streamed = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(SessionEvent::AssistantText { text: t, .. })) => finalized.push_str(&t),
            Ok(Ok(SessionEvent::PartialText { text: t, .. })) => streamed.push_str(&t),
            Ok(Ok(SessionEvent::TurnDone { .. })) => {
                return if finalized.trim().is_empty() {
                    streamed
                } else {
                    finalized
                };
            }
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited mid-turn: {outcome:?}")
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("event channel closed mid-turn"),
            Err(_) => panic!("turn did not finish within 120s: {text:?}"),
        }
    }
}
