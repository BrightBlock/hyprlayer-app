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
        agent: None,
        append_system_prompt: None,
        env: std::env::vars().collect(),
        claude_config_dir: dirs::home_dir().map(|h| h.join(".claude")),
    })
    .expect("spawn claude");

    driver
        .send_user_turn(
            "Reply with exactly the word: pong. Nothing else.",
            None,
            None,
            None,
        )
        .expect("send turn");

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
            Ok(Err(_)) => break,
            Err(_) => panic!("turn did not finish within 60s"),
        }
    }

    assert!(got_init, "should have seen an init event");
    assert!(turn_done, "the turn should have completed");
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

async fn run_turn(
    driver: &ClaudeDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    text: &str,
) -> String {
    driver
        .send_user_turn(text, None, None, None)
        .expect("send turn");
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
        agent: None,
        append_system_prompt: None,
        env: std::env::vars().collect(),
        claude_config_dir: dirs::home_dir().map(|h| h.join(".claude")),
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
        agent: None,
        append_system_prompt: None,
        env: std::env::vars().collect(),
        claude_config_dir: dirs::home_dir().map(|h| h.join(".claude")),
    })
    .expect("spawn claude")
}

#[tokio::test]
#[ignore = "requires a logged-in claude; makes real API calls"]
async fn rewind_across_resume_truncates_pre_resume_turns() {
    let cwd = std::env::temp_dir();
    let session_id = uuid::Uuid::new_v4().to_string();

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

    let (driver, mut events) = spawn_haiku(&cwd, &session_id, true);
    run_turn(&driver, &mut events, "Reply READY.").await;

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

#[derive(Debug, PartialEq)]
enum Seen {
    User(Option<String>),
    ToolResult,
    Text(String),
    TurnDone,
    Lifecycle(String, String),
}

/// Sends `first`; at its first tool call runs `mid`. Collects through `turns` turn ends, then
/// keeps listening for `settle` so a stray extra turn shows up.
async fn collect_turns(
    driver: &ClaudeDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    first: &str,
    turns: usize,
    settle: Duration,
    mut mid: impl FnMut(&ClaudeDriver),
) -> Vec<Seen> {
    driver
        .send_user_turn(first, None, None, Some("first"))
        .expect("send first");
    let mut seen = Vec::new();
    let mut fired = false;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let mut done = 0;
    loop {
        let event = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => panic!("event channel closed"),
            Err(_) if done >= turns => return seen,
            Err(_) => panic!("only {done} of {turns} turns ended within 120s: {seen:?}"),
        };
        match event {
            SessionEvent::ToolUse { .. } if !fired => {
                fired = true;
                mid(driver);
            }
            SessionEvent::UserMessage {
                client_message_id, ..
            } => seen.push(Seen::User(client_message_id)),
            SessionEvent::ToolResult { .. } => seen.push(Seen::ToolResult),
            SessionEvent::AssistantText { text, .. } => seen.push(Seen::Text(text)),
            SessionEvent::CommandLifecycle {
                command_uuid,
                state,
            } => seen.push(Seen::Lifecycle(command_uuid, state)),
            SessionEvent::TurnDone { .. } => {
                seen.push(Seen::TurnDone);
                done += 1;
                if done == turns {
                    deadline = tokio::time::Instant::now() + settle;
                }
            }
            SessionEvent::Exited { outcome, .. } => panic!("session exited: {outcome:?}"),
            _ => {}
        }
    }
}

fn position(seen: &[Seen], want: &Seen) -> usize {
    seen.iter()
        .position(|s| s == want)
        .unwrap_or_else(|| panic!("{want:?} never arrived: {seen:?}"))
}

fn final_text(seen: &[Seen]) -> String {
    seen.iter()
        .rev()
        .find_map(|s| match s {
            Seen::Text(t) => Some(t.to_uppercase()),
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "requires a logged-in claude; makes real API calls"]
async fn a_message_sent_mid_turn_joins_the_running_turn() {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (driver, mut events) = spawn_haiku(&std::env::temp_dir(), &session_id, false);
    let seen = collect_turns(
        &driver,
        &mut events,
        "Use the Bash tool to run exactly `sleep 8 && echo first`. When it finishes, use Bash to run `echo second`. Then reply with the single word DONE.",
        1,
        Duration::from_secs(8),
        |d| {
            d.send_user_turn(
                "Also: include the word PINEAPPLE in your final reply.",
                None,
                None,
                Some("steer"),
            )
            .expect("send mid-turn");
        },
    )
    .await;
    println!("seen: {seen:?}");

    let steer = position(&seen, &Seen::User(Some("steer".into())));
    let first_result = position(&seen, &Seen::ToolResult);
    assert!(
        first_result < steer,
        "the echo waits for claude's next step, after the running tool: {seen:?}"
    );
    assert_eq!(
        seen.iter().filter(|s| **s == Seen::TurnDone).count(),
        1,
        "the message joins the running turn instead of opening its own: {seen:?}"
    );
    assert!(
        !seen.iter().any(|s| matches!(s, Seen::Lifecycle(..))),
        "our own lifecycle frames never reach the transcript: {seen:?}"
    );
    assert!(final_text(&seen).contains("PINEAPPLE"), "{seen:?}");

    // The steered message is a user turn claude can rewind to, so later ordinals stay aligned.
    driver
        .rewind_to_ordinal(2)
        .await
        .expect("claude rewinds to a message that joined a running turn");
    let recalled = run_turn(
        &driver,
        &mut events,
        "Did I ask you to include any special word in a reply? Answer with that word, or NONE.",
    )
    .await;
    println!("post-rewind: {recalled:?}");
    assert!(
        !recalled.to_uppercase().contains("PINEAPPLE"),
        "the rewound steer is forgotten: {recalled:?}"
    );
    driver.kill();
}

#[tokio::test]
#[ignore = "requires a logged-in claude; makes real API calls"]
async fn a_waiting_message_can_be_withdrawn() {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (driver, mut events) = spawn_haiku(&std::env::temp_dir(), &session_id, false);
    let driver = std::sync::Arc::new(driver);
    let canceller = driver.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    let seen = collect_turns(
        &driver,
        &mut events,
        "Use the Bash tool to run exactly `sleep 15 && echo first`. Then reply with the single word DONE.",
        1,
        Duration::from_secs(5),
        move |d| {
            d.send_user_turn(
                "Also: include the word PINEAPPLE in your reply.",
                None,
                None,
                Some("withdrawn"),
            )
            .expect("send mid-turn");
            if let Some(tx) = tx.take() {
                let canceller = canceller.clone();
                tokio::spawn(async move {
                    let _ = tx.send(canceller.cancel_queued_message("withdrawn").await);
                });
            }
        },
    )
    .await;
    println!("seen: {seen:?}");
    let cancelled = rx.await.expect("cancel ran").expect("cancel replied");
    assert!(cancelled, "withdrawn before claude's next step");
    assert!(
        !seen.contains(&Seen::User(Some("withdrawn".into()))),
        "a withdrawn message never echoes: {seen:?}"
    );
    assert!(
        !seen.iter().any(|s| matches!(s, Seen::Lifecycle(..))),
        "a withdrawal the user asked for is not handed back: {seen:?}"
    );
    assert!(!final_text(&seen).contains("PINEAPPLE"), "{seen:?}");
    driver.kill();
}

#[tokio::test]
#[ignore = "requires a logged-in claude; makes real API calls"]
async fn an_interrupt_runs_the_waiting_message_next() {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (driver, mut events) = spawn_haiku(&std::env::temp_dir(), &session_id, false);
    let seen = collect_turns(
        &driver,
        &mut events,
        "Use the Bash tool to run exactly `sleep 20 && echo first`. Then reply with the single word DONE.",
        2,
        Duration::from_secs(5),
        |d| {
            d.send_user_turn("Reply with only the word PINEAPPLE.", None, None, Some("next"))
                .expect("send mid-turn");
            d.interrupt().expect("interrupt");
        },
    )
    .await;
    println!("seen: {seen:?}");
    let interrupted = position(&seen, &Seen::TurnDone);
    let next = position(&seen, &Seen::User(Some("next".into())));
    assert!(
        interrupted < next,
        "the waiting message opens the turn after the interrupted one: {seen:?}"
    );
    assert!(final_text(&seen).contains("PINEAPPLE"), "{seen:?}");
    driver.kill();
}
