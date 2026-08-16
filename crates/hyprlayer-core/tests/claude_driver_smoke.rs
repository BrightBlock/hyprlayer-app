//! End-to-end driver smoke test against the REAL `claude` binary. Ignored by
//! default — needs a logged-in claude and makes a (tiny) billed API call. Run:
//!   cargo test -p hyprlayer-core --test claude_driver_smoke -- --ignored --nocapture

use std::time::Duration;

use hyprlayer_core::claude::{
    ClaudeDriver, ExitOutcome, PermissionMode, SessionEvent, SpawnConfig,
};

#[tokio::test]
#[ignore = "requires a logged-in claude; makes a real API call"]
async fn drives_a_real_no_tool_turn_to_completion() {
    let claude = which::which("claude").expect("claude on PATH");
    let cwd = std::env::temp_dir();
    let session_id = uuid::Uuid::new_v4().to_string();

    let (driver, mut events) = ClaudeDriver::spawn(SpawnConfig {
        claude_bin: claude,
        cwd,
        session_id,
        permission_mode: PermissionMode::Default,
        allowed_tools: vec![],
        disallowed_tools: vec![],
        add_dirs: vec![],
        resume: None,
        fork: false,
        max_budget_usd: Some(1.0),
        model: None,
        effort: None,
        chrome: false,
        env: std::env::vars().collect(),
    })
    .expect("spawn claude");

    driver
        .send_user_turn(
            "Reply with exactly the word: pong. Nothing else.",
            None,
            None,
        )
        .expect("send turn");

    // A stream-json session stays alive between turns, so the turn completes at
    // TurnDone (not Exited). Collect text + partials until the turn finishes.
    let mut text = String::new();
    let mut got_init = false;
    let mut turn_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    while !turn_done {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(SessionEvent::Init { model, .. })) => {
                got_init = true;
                println!("init: model={model:?}");
            }
            Ok(Ok(SessionEvent::AssistantText { text: t, .. })) => text.push_str(&t),
            Ok(Ok(SessionEvent::PartialText { text: t, .. })) => print!("{t}"),
            Ok(Ok(SessionEvent::TurnDone { cost_usd, .. })) => {
                println!("\nturn done: cost={cost_usd:?}");
                turn_done = true;
            }
            Ok(Ok(SessionEvent::Exited { outcome, code, .. })) => {
                panic!("session exited before completing the turn: {outcome:?} code={code:?}");
            }
            Ok(Ok(other)) => println!("event: {other:?}"),
            Ok(Err(_)) => break, // channel closed
            Err(_) => panic!("turn did not finish within 60s"),
        }
    }

    assert!(got_init, "should have seen an init event");
    assert!(turn_done, "the turn should have completed");
    assert!(
        text.to_lowercase().contains("pong"),
        "assistant should have said pong, got: {text:?}"
    );

    // A hard kill should drive the session to a terminal Exited event.
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

/// Send `text` as a turn and drain events until the turn completes, returning the
/// assistant's coalesced text. Panics if the turn doesn't finish in time.
async fn run_turn(
    driver: &ClaudeDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    text: &str,
) -> String {
    driver.send_user_turn(text, None, None).expect("send turn");
    let mut out = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(SessionEvent::AssistantText { text: t, .. })) => out.push_str(&t),
            Ok(Ok(SessionEvent::TurnDone { .. })) => return out,
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited mid-turn: {outcome:?}")
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("event channel closed mid-turn"),
            Err(_) => panic!("turn did not finish within 60s: {text:?}"),
        }
    }
}

/// End-to-end: three turns, then `rewind_to_ordinal(2)` must drive claude's own
/// `rewind_conversation` so the model genuinely forgets turns 2–3 while keeping
/// turn 1. Exercises the whole integration: reading the turn uuids from claude's
/// on-disk transcript, control-response routing, and tail-to-target peeling.
#[tokio::test]
#[ignore = "requires a logged-in claude; makes real API calls"]
async fn rewind_to_ordinal_truncates_model_memory() {
    let claude = which::which("claude").expect("claude on PATH");
    let (driver, mut events) = ClaudeDriver::spawn(SpawnConfig {
        claude_bin: claude,
        cwd: std::env::temp_dir(),
        session_id: uuid::Uuid::new_v4().to_string(),
        permission_mode: PermissionMode::BypassPermissions,
        allowed_tools: vec![],
        disallowed_tools: vec![],
        add_dirs: vec![],
        resume: None,
        fork: false,
        max_budget_usd: Some(1.0),
        model: Some("haiku".into()),
        effort: None,
        chrome: false,
        env: std::env::vars().collect(),
    })
    .expect("spawn claude");

    run_turn(
        &driver,
        &mut events,
        "Remember: my favorite color is blue. Reply OK.",
    )
    .await;
    run_turn(
        &driver,
        &mut events,
        "Remember: my favorite number is seven. Reply OK.",
    )
    .await;
    run_turn(
        &driver,
        &mut events,
        "Remember: my favorite animal is the otter. Reply OK.",
    )
    .await;

    // Rewind to turn 2: drop turns 2 (number) and 3 (animal), keep turn 1 (color).
    driver
        .rewind_to_ordinal(2)
        .await
        .expect("rewind_to_ordinal should truncate claude's conversation");

    let animal = run_turn(
        &driver,
        &mut events,
        "What is my favorite animal? If I never told you in this conversation, reply exactly: UNKNOWN.",
    )
    .await;
    println!("post-rewind animal answer: {animal:?}");
    assert!(
        !animal.to_lowercase().contains("otter"),
        "model should have FORGOTTEN the otter (turn 3 was rewound away), got: {animal:?}"
    );

    // Turn 1 survived the rewind, so the color must still be known.
    let color = run_turn(
        &driver,
        &mut events,
        "What is my favorite color? If I never told you, reply UNKNOWN.",
    )
    .await;
    println!("post-rewind color answer: {color:?}");
    assert!(
        color.to_lowercase().contains("blue"),
        "model should STILL remember blue (turn 1 was kept), got: {color:?}"
    );

    driver.kill();
}

/// Spawn a haiku session (fresh or `--resume`) in `cwd` under `session_id`.
fn spawn_haiku(
    cwd: &std::path::Path,
    session_id: &str,
    resume: bool,
) -> (ClaudeDriver, tokio::sync::broadcast::Receiver<SessionEvent>) {
    let claude = which::which("claude").expect("claude on PATH");
    ClaudeDriver::spawn(SpawnConfig {
        claude_bin: claude,
        cwd: cwd.to_path_buf(),
        session_id: session_id.to_string(),
        permission_mode: PermissionMode::BypassPermissions,
        allowed_tools: vec![],
        disallowed_tools: vec![],
        add_dirs: vec![],
        resume: resume.then(|| session_id.to_string()),
        fork: false,
        max_budget_usd: Some(1.0),
        model: Some("haiku".into()),
        effort: None,
        chrome: false,
        env: std::env::vars().collect(),
    })
    .expect("spawn claude")
}

/// THE REGRESSION (issue #1011): rewinding to a turn from BEFORE a `--resume`
/// boundary. The driver reads the target uuids from claude's on-disk transcript,
/// not the live process's replay echoes, so a resumed session can still
/// truly-rewind its earlier turns. Every app restart resumes sessions, so before
/// this fix a rewind of any pre-restart turn silently fell back to a log-only prune
/// ("the agent still remembers the removed turns") — now the model actually forgets.
#[tokio::test]
#[ignore = "requires a logged-in claude; makes real API calls"]
async fn rewind_across_resume_truncates_pre_resume_turns() {
    let cwd = std::env::temp_dir();
    let session_id = uuid::Uuid::new_v4().to_string();

    // Generation 1: three turns, then kill the process — i.e. an app restart.
    {
        let (driver, mut events) = spawn_haiku(&cwd, &session_id, false);
        run_turn(
            &driver,
            &mut events,
            "Remember: my favorite color is blue. Reply OK.",
        )
        .await;
        run_turn(
            &driver,
            &mut events,
            "Remember: my favorite number is seven. Reply OK.",
        )
        .await;
        run_turn(
            &driver,
            &mut events,
            "Remember: my favorite animal is the otter. Reply OK.",
        )
        .await;
        driver.kill();
        // Let it exit so the resume can't race the still-dying process.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(SessionEvent::Exited { .. }) = events.recv().await {
                    return;
                }
            }
        })
        .await
        .expect("generation 1 should exit after kill");
    }

    // Generation 2: `--resume` the SAME session id. One neutral turn first, exactly
    // as the app does — a resumed session is re-attached and interacted with (so
    // claude has fully loaded `z`) well before the user clicks rewind. (Rewinding in
    // the very first millisecond after `--resume`, mid-load, is racy on claude's
    // side and never happens in the human-driven UI.)
    let (driver, mut events) = spawn_haiku(&cwd, &session_id, true);
    run_turn(&driver, &mut events, "Reply READY.").await;

    // Rewind to turn 2 — turns 2 and 3 are from the PREVIOUS generation, whose uuids
    // this process never saw echoed; the driver reads them from claude's transcript.
    driver
        .rewind_to_ordinal(2)
        .await
        .expect("rewind across a resume should truncate claude's conversation");

    let animal = run_turn(
        &driver,
        &mut events,
        "What is my favorite animal? If I never told you in this conversation, reply exactly: UNKNOWN.",
    )
    .await;
    println!("post-resume-rewind animal answer: {animal:?}");
    assert!(
        !animal.to_lowercase().contains("otter"),
        "model should have FORGOTTEN the otter (a pre-resume turn was rewound), got: {animal:?}"
    );

    let color = run_turn(
        &driver,
        &mut events,
        "What is my favorite color? If I never told you, reply UNKNOWN.",
    )
    .await;
    println!("post-resume-rewind color answer: {color:?}");
    assert!(
        color.to_lowercase().contains("blue"),
        "model should STILL remember blue (turn 1 was kept across the resume), got: {color:?}"
    );

    driver.kill();
}
