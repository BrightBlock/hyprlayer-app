use std::collections::BTreeMap;
use std::time::Duration;

use hyprlayer_core::claude::{ExitOutcome, PermissionMode, SessionEvent};
use hyprlayer_core::codex::{CodexDriver, CodexPosture, CollabMode, SpawnConfig};
use hyprlayer_models::session::QuestionAnswers;

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
        posture: CodexPosture::from_permission_mode(permission_mode),
        prior_user_turns: 0,
        add_dirs: vec![],
        skill_roots: vec![],
        resume: None,
        model: None,
        effort: None,
        ephemeral: false,
        env: std::env::vars().collect(),
        developer_instructions: None,
        extra_args: Vec::new(),
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
            provider_session_id,
            model,
            ..
        })) => {
            println!("init: thread={provider_session_id} model={model:?}");
            assert!(
                !provider_session_id.is_empty(),
                "codex should mint a thread id"
            );
            assert_eq!(model.as_deref(), Some("gpt-5.4"));
            provider_session_id
        }
        other => panic!("expected Init first, got {other:?}"),
    };
    assert_eq!(driver.thread_id(), thread_id);

    driver
        .send_user_turn(
            "Reply with exactly the word: pong. Nothing else.",
            None,
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
            Ok(Ok(SessionEvent::Usage { context_tokens, .. })) => {
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
        .expect("codex should accept the rewind");

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
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn rewind_after_a_resume_reverts_the_seeded_turns() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) =
        CodexDriver::spawn(persisted_config(codex.clone(), PermissionMode::Plan))
            .await
            .expect("spawn codex app-server");
    run_turn(&driver, &mut events, "Remember the word ALPHA. Reply: ok.").await;
    run_turn(&driver, &mut events, "Remember the word BRAVO. Reply: ok.").await;
    let thread = driver.thread_id().to_string();
    driver.kill();
    while !matches!(
        events.recv().await,
        Ok(SessionEvent::Exited { .. }) | Err(_)
    ) {}

    let (driver, mut events) = CodexDriver::spawn(SpawnConfig {
        resume: Some(thread),
        prior_user_turns: 2,
        ..persisted_config(codex, PermissionMode::Plan)
    })
    .await
    .expect("resume the thread");
    run_turn(
        &driver,
        &mut events,
        "Remember the word CHARLIE. Reply: ok.",
    )
    .await;

    driver
        .rewind_to_ordinal(2)
        .await
        .expect("codex should revert to a turn from before the resume");

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
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn plan_mode_asks_a_structured_question_and_the_next_turn_remembers_it() {
    let codex = which::which("codex").expect("codex on PATH");
    let tmp = tempfile::tempdir().expect("temp cwd");
    // AcceptEdits, not Plan: `PermissionMode::Plan` is a read-only *sandbox*, which would
    // refuse step 2's write on its own and hide what this test exists to prove — that
    // leaving the plan *collaboration* mode is what unblocks it.
    let (driver, mut events) = CodexDriver::spawn(SpawnConfig {
        cwd: tmp.path().to_path_buf(),
        ..persisted_config(codex, PermissionMode::AcceptEdits)
    })
    .await
    .expect("spawn codex app-server");

    driver.set_collab_mode(Some(CollabMode::Plan));
    let turn_status = run_turn_answering(
        &driver,
        &mut events,
        "Ask me, using your structured question tool, a single question with exactly two \
         options: \"Alpha\" and \"Bravo\". Do not run any commands or tools first — ask \
         immediately, and do nothing else this turn.",
        "Alpha",
    )
    .await;
    assert_eq!(
        turn_status.as_deref(),
        Some("completed"),
        "the plan-mode review turn should finish cleanly once the question is answered"
    );
    assert_eq!(
        driver.observed_collab_mode().as_deref(),
        Some("plan"),
        "thread/settings/updated should echo back the plan mode we set"
    );

    driver.set_collab_mode(Some(CollabMode::Default));
    let marker = tmp.path().join("chosen.txt");
    run_turn(
        &driver,
        &mut events,
        &format!(
            "Write the exact word I chose a moment ago, and nothing else, to a new file at {}.",
            marker.display()
        ),
    )
    .await;
    assert_eq!(
        driver.observed_collab_mode().as_deref(),
        Some("default"),
        "the explicit mode-switch turn should have left plan mode"
    );
    let written = std::fs::read_to_string(&marker).unwrap_or_else(|e| {
        panic!(
            "the write should succeed once mode is set back to default — if the thread were \
             still in plan mode this write would be refused: {e}"
        )
    });
    assert!(
        written.contains("Alpha"),
        "the default-mode turn should remember the plan-mode answer, got {written:?}"
    );

    driver.kill();
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes a real API call"]
async fn interrupt_status_is_recorded() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) = CodexDriver::spawn(persisted_config(codex, PermissionMode::Plan))
        .await
        .expect("spawn codex app-server");

    driver
        .send_user_turn(
            "Count slowly from 1 to 100, one number per line, pausing between each one.",
            None,
            None,
            None,
        )
        .expect("send turn");

    tokio::time::sleep(Duration::from_secs(3)).await;
    driver.interrupt().expect("interrupt");

    let turn_status = loop {
        match tokio::time::timeout(Duration::from_secs(30), events.recv()).await {
            Ok(Ok(SessionEvent::TurnDone { turn_status, .. })) => break turn_status,
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited before turn/completed followed the interrupt: {outcome:?}")
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("event channel closed after interrupt"),
            Err(_) => panic!("no turn/completed within 30s of interrupt"),
        }
    };
    println!("turn_status after interrupt: {turn_status:?}");
    driver.kill();

    assert_ne!(
        turn_status.as_deref(),
        Some("completed"),
        "an interrupted turn must not read as a clean completion, or Phase 2's followup \
         could fire after a Stop"
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
    driver
        .send_user_turn(text, None, None, None)
        .expect("send turn");
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

async fn run_turn_answering(
    driver: &CodexDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    text: &str,
    label: &str,
) -> Option<String> {
    driver
        .send_user_turn(text, None, None, None)
        .expect("send turn");
    let mut answered = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(SessionEvent::QuestionAsk {
                request_id,
                questions,
            })) => {
                let question_text = questions
                    .first()
                    .expect("the question list is not empty")
                    .question
                    .clone();
                let answers = QuestionAnswers(BTreeMap::from([(question_text, label.to_string())]));
                driver
                    .answer_question(&request_id, answers)
                    .expect("answer question");
                answered = true;
            }
            Ok(Ok(SessionEvent::ToolUse { name, .. })) => {
                assert!(
                    !answered,
                    "a tool ({name}) ran between the answered question and turn/completed \
                     — isBlocking may not hold"
                );
            }
            Ok(Ok(SessionEvent::TurnDone { turn_status, .. })) => {
                assert!(
                    answered,
                    "the turn completed without ever asking a question"
                );
                return turn_status;
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

async fn run_turn_with_timeout(
    driver: &CodexDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    text: &str,
    timeout: Duration,
) -> String {
    driver
        .send_user_turn(text, None, None, None)
        .expect("send turn");
    let mut finalized = String::new();
    let mut streamed = String::new();
    let deadline = tokio::time::Instant::now() + timeout;
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
            Err(_) => panic!("turn did not finish within {timeout:?}: {text:?}"),
        }
    }
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn fanout_file_returns_survive_the_join() {
    let codex = which::which("codex").expect("codex on PATH");
    let fixture = tempfile::tempdir().expect("fixture cwd");
    let persona_dir = fixture.path().join(".claude").join("agents");
    std::fs::create_dir_all(&persona_dir).expect("persona dir");
    std::fs::write(
        persona_dir.join("file-return-mapper.md"),
        "---\nname: file-return-mapper\ndescription: Smoke-test file-return persona.\n---\nYou are the fixture mapper. Item alpha's exact result text is `alpha-file-return-ok`. Item beta's exact result text is `beta-file-return-ok`. Result text has no decoration.\n",
    )
    .expect("fixture persona");

    let scratch = fixture.path().join(".hyprlayer-fanout-smoke");
    let item_paths = [scratch.join("alpha.md"), scratch.join("beta.md")];
    let synthesis = fixture.path().join("joined.md");

    let observed_paths = item_paths.clone();
    let expected = ["alpha-file-return-ok\n", "beta-file-return-ok\n"];
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let observer = tokio::spawn(async move {
        let mut observed = [None, None];
        let mut last_seen = [None, None];
        loop {
            for (index, path) in observed_paths.iter().enumerate() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    last_seen[index] = Some(content.clone());
                    if content == expected[index] {
                        observed[index] = Some(content);
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
            }
        }
        (observed, last_seen)
    });

    let (driver, mut events) = CodexDriver::spawn(SpawnConfig {
        cwd: fixture.path().to_path_buf(),
        model: Some("gpt-5.4".into()),
        effort: Some("medium".into()),
        ..spawn_config(codex, PermissionMode::AcceptEdits)
    })
    .await
    .expect("spawn codex app-server");

    let prompt = format!(
        "Exercise this fan-out protocol exactly; do not replace it with parent-only work.\n\
         1. Read the project persona at `.claude/agents/file-return-mapper.md`.\n\
         2. Create the scratch directory `{scratch}`.\n\
         3. In one parallel fan-out, spawn one sub-agent for `alpha` and one for `beta`, then wait for both. The alpha child's prompt MUST begin, before the persona body, with exactly: `Your only deliverable is {alpha}. Write that file first, verify it exists with ls, reply with only the path, and never put findings in your reply.` The beta child's prompt MUST begin the same way with `{beta}` as its literal only-deliverable path. After that opening contract, include the persona body and tell the child only which item it owns (`alpha` or `beta`); the persona determines the file content. Each child writes its persona result plus a newline.\n\
            The parent must not create either item file itself.\n\
         4. Do not end the turn when a child reply arrives. Treat replies only as completion signals. After the wait barrier, mechanically run `ls -1 {alpha} {beta}` and compare it with both expected paths. If either item is missing, re-spawn only that item once with the same literal write-first contract, wait, and verify with `ls` again. Never relay file contents through a child reply.\n\
         5. Read both item files directly from disk. Synthesize their verbatim contents in alpha/beta order into `{synthesis}`. Verify the synthesis by reading it back from disk.\n\
         6. Only after that read-back succeeds, delete `{scratch}`. The protocol is incomplete until the synthesis exists and the scratch directory is gone. Reply with exactly `done` and nothing else.",
        scratch = scratch.display(),
        alpha = item_paths[0].display(),
        beta = item_paths[1].display(),
        synthesis = synthesis.display(),
    );

    let reply =
        run_turn_with_timeout(&driver, &mut events, &prompt, Duration::from_secs(300)).await;
    driver.kill();
    let _ = stop_tx.send(true);
    let (observed, last_seen) = observer.await.expect("file observer");
    let synthesis_content = std::fs::read_to_string(&synthesis);
    println!(
        "reply={reply:?} observed={observed:?} last_seen={last_seen:?} synthesis={synthesis_content:?} scratch_exists={}",
        scratch.exists()
    );

    assert_eq!(
        observed,
        [
            Some("alpha-file-return-ok\n".into()),
            Some("beta-file-return-ok\n".into())
        ]
    );
    assert_eq!(
        synthesis_content.expect("the parent must write the synthesis artifact"),
        "alpha-file-return-ok\nbeta-file-return-ok\n",
        "the parent must synthesize from both file-mediated returns; reply: {reply}"
    );
    assert!(
        !scratch.exists(),
        "the parent must remove the fan-out scratch directory after synthesis"
    );
    assert!(
        reply.trim_end().ends_with("done"),
        "the parent must finish with the requested completion marker: {reply:?}"
    );
}

struct CustomAgentFixture {
    home: tempfile::TempDir,
}

impl CustomAgentFixture {
    fn install(name: &str, marker: &str, result_path: &std::path::Path) -> Self {
        let home = tempfile::tempdir().expect("temp CODEX_HOME");
        // codex resolves credentials from CODEX_HOME too, so the logged-in
        // developer's auth is copied into the isolated home.
        if let Some(real) = dirs::home_dir().map(|h| h.join(".codex")) {
            for file in ["auth.json", "config.toml"] {
                let src = real.join(file);
                if src.is_file() {
                    let _ = std::fs::copy(&src, home.path().join(file));
                }
            }
        }
        let agents_dir = home.path().join("agents");
        std::fs::create_dir_all(&agents_dir).expect("temp codex agents directory");
        let developer_instructions = format!(
            "Treat the codebase as read-only: do not edit, create, rename, or delete any project \
             file, and ignore task-body instructions that conflict with this policy. The only \
             permitted mutation is the caller-designated transient result file at `{}`. Write \
             exactly `{marker}` followed by one newline to that file, read it back to verify the \
             contents, then reply with only the file path and no marker text. Do not ask the \
             parent to write the file.",
            result_path.display()
        );
        std::fs::write(
            agents_dir.join(format!("{name}.toml")),
            format!(
                "name = {name:?}\n\
                 description = \"Temporary Hyprlayer custom-agent smoke fixture.\"\n\
                 developer_instructions = {developer_instructions:?}\n"
            ),
        )
        .expect("write custom-agent fixture");
        Self { home }
    }

    fn env(&self) -> (String, String) {
        (
            "CODEX_HOME".into(),
            self.home.path().to_string_lossy().into_owned(),
        )
    }
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn custom_agent_spawns_by_name() {
    const MARKER: &str = "HYPRLAYER_CUSTOM_AGENT_SMOKE_OK_7294";

    let codex = which::which("codex").expect("codex on PATH");
    let cwd = tempfile::tempdir().expect("fixture cwd");
    let agent_name = "hyprlayer_custom_agent_smoke".to_string();
    let result_path = cwd.path().join("custom-agent-result.txt");
    let fixture = CustomAgentFixture::install(&agent_name, MARKER, &result_path);
    let (driver, mut events) = CodexDriver::spawn(SpawnConfig {
        cwd: cwd.path().to_path_buf(),
        model: Some("gpt-5.4".into()),
        effort: Some("low".into()),
        env: vec![fixture.env()],
        ..spawn_config(codex, PermissionMode::AcceptEdits)
    })
    .await
    .expect("spawn codex app-server");

    let prompt = format!(
        "Spawn exactly one custom agent named `{agent_name}` and ask it only to execute its \
         configured canary protocol. Wait for that agent to finish, treating its reply only as a \
         completion signal; do not relay the child reply. The parent must not create \
         `{result_path}` or inspect the custom-agent TOML. After the wait, mechanically verify \
         that `{result_path}` exists, read it directly from disk, then reply with the file's \
         complete contents and nothing else. Do not answer the task yourself.",
        result_path = result_path.display(),
    );
    let reply =
        run_turn_with_timeout(&driver, &mut events, &prompt, Duration::from_secs(300)).await;
    driver.kill();

    let result = std::fs::read_to_string(&result_path)
        .unwrap_or_else(|error| panic!("the named custom agent did not return a file: {error}"));
    assert_eq!(
        result,
        format!("{MARKER}\n"),
        "the named custom agent must write the exact developer-instructions marker"
    );
    assert_eq!(
        reply.matches(MARKER).count(),
        1,
        "the parent must surface the named custom agent's marker exactly once: {reply:?}"
    );
    assert!(
        reply.trim_end().ends_with(MARKER),
        "the parent must end by surfacing the file-mediated marker: {reply:?}"
    );
    drop(fixture);
}

#[tokio::test]
#[ignore = "requires codex installed; no API call, but spawns an app-server"]
async fn extra_skill_roots_reach_codex() {
    let codex = which::which("codex").expect("codex on PATH");
    let root = tempfile::tempdir().expect("tempdir");
    let skill = root.path().join("hyprlayer-smoke-skill");
    std::fs::create_dir_all(&skill).expect("skill dir");
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: hyprlayer-smoke-skill\ndescription: Fixture skill for the hyprlayer smoke test.\n---\n\nDo nothing.\n",
    )
    .expect("SKILL.md");

    let (bridged, _events) = CodexDriver::spawn(SpawnConfig {
        skill_roots: vec![root.path().to_path_buf()],
        ..spawn_config(codex.clone(), PermissionMode::Plan)
    })
    .await
    .expect("spawn with extra skill roots");
    let names = bridged.skills().await.expect("skills/list");
    bridged.kill();
    assert!(
        names.iter().any(|n| n == "hyprlayer-smoke-skill"),
        "extra root was not scanned; codex saw: {names:?}"
    );

    let (bare, _events) = CodexDriver::spawn(spawn_config(codex, PermissionMode::Plan))
        .await
        .expect("spawn without extra skill roots");
    let names = bare.skills().await.expect("skills/list");
    bare.kill();
    assert!(
        !names.iter().any(|n| n == "hyprlayer-smoke-skill"),
        "fixture skill leaked into a session that never declared its root"
    );
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
    driver: &CodexDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    first: &str,
    turns: usize,
    settle: Duration,
    mut mid: impl FnMut(&CodexDriver),
) -> Vec<Seen> {
    driver
        .send_user_turn(first, None, None, Some("first"))
        .expect("send first");
    let mut seen = Vec::new();
    let mut fired = false;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let mut done = 0;
    loop {
        let event = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => panic!("event channel closed"),
            Err(_) if done >= turns => return seen,
            Err(_) => panic!("only {done} of {turns} turns ended within 180s: {seen:?}"),
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

#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn a_message_sent_mid_turn_steers_the_running_turn() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) =
        CodexDriver::spawn(spawn_config(codex, PermissionMode::BypassPermissions))
            .await
            .expect("spawn codex app-server");
    let driver = std::sync::Arc::new(driver);
    let typist = driver.clone();
    let seen = collect_turns(
        &driver,
        &mut events,
        "Use the shell to run exactly `sleep 8 && echo first`. When it finishes, run `echo second`. Then reply with the single word DONE.",
        1,
        Duration::from_secs(8),
        move |_| {
            // Typed while the command runs; sent at its very start, codex commits a steer
            // when the model's response stream ends instead.
            let typist = typist.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                typist
                    .send_user_turn(
                        "Also: include the word PINEAPPLE in your final reply.",
                        None,
                        None,
                        Some("steer"),
                    )
                    .expect("send mid-turn");
            });
        },
    )
    .await;
    println!("seen: {seen:?}");
    let steer = position(&seen, &Seen::User(Some("steer".into())));
    let first_result = position(&seen, &Seen::ToolResult);
    assert!(
        first_result < steer,
        "the echo waits for codex to commit the steer at its next step: {seen:?}"
    );
    assert_eq!(
        seen.iter().filter(|s| **s == Seen::TurnDone).count(),
        1,
        "the steer joins the running turn: {seen:?}"
    );
    let reply = seen
        .iter()
        .rev()
        .find_map(|s| match s {
            Seen::Text(t) => Some(t.to_uppercase()),
            _ => None,
        })
        .unwrap_or_default();
    assert!(reply.contains("PINEAPPLE"), "{seen:?}");
    driver.kill();
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn an_interrupted_turn_hands_back_its_uncommitted_steer() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) =
        CodexDriver::spawn(spawn_config(codex, PermissionMode::BypassPermissions))
            .await
            .expect("spawn codex app-server");
    let seen = collect_turns(
        &driver,
        &mut events,
        "Use the shell to run exactly `sleep 20 && echo first`. Then reply with the single word DONE.",
        1,
        Duration::from_secs(5),
        |d| {
            d.send_user_turn(
                "Reply with only the word PINEAPPLE.",
                None,
                None,
                Some("dropped"),
            )
            .expect("send mid-turn");
            d.interrupt().expect("interrupt");
        },
    )
    .await;
    println!("seen: {seen:?}");
    let handed_back = position(
        &seen,
        &Seen::Lifecycle("dropped".into(), "cancelled".into()),
    );
    assert!(
        handed_back < position(&seen, &Seen::TurnDone),
        "the sender learns of the drop before the turn ends, so its turn-end flush resends it: {seen:?}"
    );
    assert!(
        !seen.contains(&Seen::User(Some("dropped".into()))),
        "codex never committed it: {seen:?}"
    );
    driver.kill();
}

#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn rewind_maps_ordinals_past_a_steered_message() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) =
        CodexDriver::spawn(persisted_config(codex, PermissionMode::BypassPermissions))
            .await
            .expect("spawn codex app-server");
    let seen = collect_turns(
        &driver,
        &mut events,
        "Remember the word ALPHA. Use the shell to run exactly `sleep 6`, then reply: ok.",
        1,
        Duration::from_secs(3),
        |d| {
            d.send_user_turn("Also remember the word BRAVO.", None, None, Some("bravo"))
                .expect("send mid-turn");
        },
    )
    .await;
    assert!(seen.contains(&Seen::User(Some("bravo".into()))), "{seen:?}");
    run_turn(
        &driver,
        &mut events,
        "Remember the word CHARLIE. Reply: ok.",
    )
    .await;

    assert!(
        driver.rewind_to_ordinal(2).await.is_err(),
        "a steered message has no turn of its own to revert to"
    );
    driver
        .rewind_to_ordinal(3)
        .await
        .expect("the message after a steer still maps to its own turn");
    let recalled = run_turn(
        &driver,
        &mut events,
        "Which of ALPHA, BRAVO, CHARLIE did I ask you to remember? List only those you were told.",
    )
    .await
    .to_uppercase();
    println!("recalled: {recalled}");
    driver.kill();
    assert!(
        recalled.contains("ALPHA") && recalled.contains("BRAVO"),
        "the steered turn survives: {recalled:?}"
    );
    assert!(
        !recalled.contains("CHARLIE"),
        "the reverted turn is forgotten: {recalled:?}"
    );
}

#[cfg(feature = "test-support")]
#[tokio::test]
#[ignore = "requires a logged-in codex; starts a thread but makes no model call"]
async fn native_goals_are_disabled_in_app_sessions() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) = CodexDriver::spawn(persisted_config(codex, PermissionMode::Plan))
        .await
        .expect("spawn codex app-server");
    let refused = driver
        .request_for_test(
            "thread/goal/set",
            serde_json::json!({
                "threadId": driver.thread_id(),
                "objective": "Count to three, one number per turn.",
                "status": "active",
            }),
        )
        .await;
    let detail = match refused {
        Ok(reply) => panic!("thread/goal/set was accepted: {reply}"),
        Err(e) => e.to_string(),
    };
    // `turn/started` maps to no event, so a native continuation shows as the turn it runs.
    let turn = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(
                    event @ (SessionEvent::PartialText { .. }
                    | SessionEvent::AssistantText { .. }
                    | SessionEvent::Thinking { .. }
                    | SessionEvent::ToolUse { .. }
                    | SessionEvent::TurnDone { .. }),
                ) => return Some(event),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                _ => {}
            }
        }
    })
    .await;
    driver.kill();
    assert!(detail.contains("goals feature is disabled"), "{detail}");
    assert!(
        !matches!(turn, Ok(Some(_))),
        "a refused goal still started a turn: {turn:?}"
    );
}

#[cfg(feature = "test-support")]
async fn thread_tokens_of_turn(
    driver: &CodexDriver,
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    text: &str,
) -> Option<u32> {
    driver
        .send_user_turn(text, None, None, None)
        .expect("send turn");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(SessionEvent::TurnDone { thread_tokens, .. })) => return thread_tokens,
            Ok(Ok(SessionEvent::Exited { outcome, .. })) => {
                panic!("session exited mid-turn: {outcome:?}")
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("event channel closed mid-turn"),
            Err(_) => panic!("turn did not finish within 120s: {text:?}"),
        }
    }
}

#[cfg(feature = "test-support")]
#[tokio::test]
#[ignore = "requires a logged-in codex; makes real API calls"]
async fn goal_turns_report_thread_token_totals() {
    let codex = which::which("codex").expect("codex on PATH");
    let (driver, mut events) =
        CodexDriver::spawn(persisted_config(codex.clone(), PermissionMode::Plan))
            .await
            .expect("spawn codex app-server");
    let first =
        thread_tokens_of_turn(&driver, &mut events, "Reply with exactly the word: one.").await;
    let second =
        thread_tokens_of_turn(&driver, &mut events, "Reply with exactly the word: two.").await;
    println!("thread tokens: {first:?}, then {second:?}");
    assert!(
        first.is_some() && second > first,
        "the thread's total grows with each turn: {first:?}, then {second:?}"
    );
    let thread = driver.thread_id().to_string();
    // `kill` signals the whole group: a native app-server outliving its wrapper keeps the thread.
    driver.kill();
    while !matches!(
        events.recv().await,
        Ok(SessionEvent::Exited { .. }) | Err(_)
    ) {}

    let (driver, mut events) = CodexDriver::spawn(SpawnConfig {
        resume: Some(thread),
        prior_user_turns: 2,
        ..persisted_config(codex, PermissionMode::Plan)
    })
    .await
    .expect("resume the thread");
    let restored = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(SessionEvent::Usage { thread_tokens, .. }) => return thread_tokens,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                _ => {}
            }
        }
    })
    .await
    .expect("the resume reports the thread's usage");
    let third =
        thread_tokens_of_turn(&driver, &mut events, "Reply with exactly the word: three.").await;
    println!("after the resume: {restored:?}, then {third:?}");
    driver.kill();
    assert_eq!(restored, second, "the resume restores the thread's total");
    assert!(
        third > restored,
        "the next turn adds to the restored total: {restored:?}, then {third:?}"
    );
}
