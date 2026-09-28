#![cfg(feature = "test-support")]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};

use hyprlayer_core::agent::{AgentDriver, Harness};
use hyprlayer_core::claude::{ExitOutcome, SessionEvent, SessionState};
use hyprlayer_core::codex::CollabMode;
use hyprlayer_core::error::{CoreError, Result as CoreResult};
use hyprlayer_core::models::session::QuestionAnswers;
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::runtime::sessions::{
    run_codex_followup_for_test, SessionRuntimeHandle, SessionStreamError,
};
use hyprlayer_core::store::{Db, NewSession, NewWorkstream, SessionEventGateAction, WorktreeMode};

fn seed_session(db: &Db, session_id: &str) {
    db.insert_workstream(NewWorkstream {
        project_path: "/p".into(),
        title: "t".into(),
        slug: format!("t-{session_id}"),
        branch: format!("hyprlayer/{session_id}"),
        worktree_path: "/wt".into(),
        base_branch: "main".into(),
        worktree_mode: WorktreeMode::Isolated,
    })
    .unwrap();
    let ws = db.list_active_workstreams().unwrap().pop().unwrap();
    db.insert_session(NewSession {
        id: session_id.into(),
        workstream_id: ws.id,
        stage: Stage::General,
        harness: Harness::Claude,
        cwd: "/wt".into(),
        first_input: None,
    })
    .unwrap();
}

fn turn_done(cost: f64) -> SessionEvent {
    SessionEvent::TurnDone {
        cost_usd: Some(cost),
        num_turns: None,
        context_tokens: None,
        turn_status: None,
    }
}

async fn wait_until(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_log_len(db: &Db, session_id: &str, len: usize) {
    let ok = wait_until(Duration::from_secs(5), || {
        db.session_log(session_id).unwrap().len() >= len
    })
    .await;
    assert!(ok, "session log never reached {len} entries");
}

#[test]
fn legacy_raw_row_decodes_to_the_canonical_log() {
    let db = Db::open_in_memory().unwrap();
    seed_session(&db, "s1");
    let legacy = r#"{"kind":"questionAsk","requestId":"q-1","questions":"[{\"header\":\"Header\",\"multiSelect\":false,\"options\":[{\"label\":\"Yes\",\"description\":\"Do it\"}],\"question\":\"Ship it?\"}]"}"#;
    db.append_session_event("s1", 0, "questionAsk", legacy)
        .unwrap();

    let log = db.session_log("s1").unwrap();
    assert_eq!(log.len(), 1);
    match &log[0].event {
        SessionEvent::QuestionAsk {
            request_id,
            questions,
        } => {
            assert_eq!(request_id, "q-1");
            assert_eq!(questions.len(), 1);
            assert_eq!(questions[0].question, "Ship it?");
        }
        other => panic!("expected QuestionAsk, got {other:?}"),
    }
}

#[tokio::test]
async fn legacy_raw_row_reaches_attach_replay() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let legacy = r#"{"kind":"questionAsk","requestId":"q-1","questions":"[{\"header\":\"Header\",\"multiSelect\":false,\"options\":[{\"label\":\"Yes\",\"description\":\"Do it\"}],\"question\":\"Ship it?\"}]"}"#;
    db.append_session_event("s1", 0, "questionAsk", legacy)
        .unwrap();

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();

    assert_eq!(attachment.replay.len(), 1);
    match &attachment.replay[0].event {
        SessionEvent::QuestionAsk { questions, .. } => {
            assert_eq!(questions.len(), 1);
            assert_eq!(questions[0].question, "Ship it?");
        }
        other => panic!("expected QuestionAsk, got {other:?}"),
    }
}

#[test]
fn malformed_and_missing_questions_normalize_without_failing_the_row() {
    let db = Db::open_in_memory().unwrap();
    seed_session(&db, "s1");
    let rows = [
        r#"{"kind":"questionAsk","requestId":"q-1"}"#,
        r#"{"kind":"questionAsk","requestId":"q-2","questions":"not-valid-json{"}"#,
        r#"{"kind":"questionAsk","requestId":"q-3","questions":17}"#,
    ];
    for (seq, payload) in rows.iter().enumerate() {
        db.append_session_event("s1", seq as i64, "questionAsk", payload)
            .unwrap();
    }

    let log = db.session_log("s1").unwrap();
    assert_eq!(log.len(), rows.len(), "no row should be dropped");
    for entry in &log {
        match &entry.event {
            SessionEvent::QuestionAsk { questions, .. } => assert!(questions.is_empty()),
            other => panic!("expected QuestionAsk, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn attach_observes_a_consistent_view_around_an_in_flight_durable_commit() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    db.set_session_event_gate(|_, seq| {
        if seq == 0 {
            SessionEventGateAction::Delay(Duration::from_millis(200))
        } else {
            SessionEventGateAction::Proceed
        }
    });

    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    ingress.send(turn_done(1.0)).await.unwrap();

    let attachment = handle.attach("s1", false).await.unwrap();
    let mut live = attachment.live;

    let seen_in_replay = !attachment.replay.is_empty();
    let seen_live = tokio::time::timeout(Duration::from_millis(500), live.recv())
        .await
        .ok()
        .flatten()
        .is_some();

    assert!(
        seen_in_replay ^ seen_live,
        "the durable event must appear exactly once, in replay xor live (replay={seen_in_replay}, live={seen_live})"
    );

    db.clear_session_event_gate();
}

#[tokio::test]
async fn an_ephemeral_event_generated_after_attach_reaches_the_live_receiver() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();
    assert!(attachment.replay.is_empty());
    let mut live = attachment.live;

    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::PartialText {
            text: "partial".into(),
            parent_id: None,
        })
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), live.recv())
        .await
        .expect("live receiver timed out")
        .expect("live receiver closed early");
    match received {
        Ok(SessionEvent::PartialText { text, .. }) => assert_eq!(text, "partial"),
        other => panic!("expected a partial-text delivery, got {other:?}"),
    }

    assert!(db.session_log("s1").unwrap().is_empty());
}

#[tokio::test]
async fn replay_never_duplicates_already_committed_durable_events() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    for i in 0..5 {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", 5).await;

    let attachment = handle.attach("s1", false).await.unwrap();
    assert_eq!(attachment.replay.len(), 5);
    for (i, entry) in attachment.replay.iter().enumerate() {
        match entry.event {
            SessionEvent::TurnDone { cost_usd, .. } => assert_eq!(cost_usd, Some(i as f64)),
            ref other => panic!("expected TurnDone, got {other:?}"),
        }
    }

    let mut live = attachment.live;
    let redelivered = tokio::time::timeout(Duration::from_millis(200), live.recv()).await;
    assert!(
        redelivered.is_err(),
        "no already-replayed event may repeat live"
    );
}

#[tokio::test]
async fn a_large_burst_is_bounded_and_fully_committed_in_order() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;

    const TOTAL: usize = 1_500;
    for i in 0..TOTAL {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", TOTAL).await;

    let log = db.session_log("s1").unwrap();
    assert_eq!(log.len(), TOTAL);
    for (i, entry) in log.iter().enumerate() {
        assert_eq!(entry.seq, i as i64, "sequence numbers must be gap-free");
        match entry.event {
            SessionEvent::TurnDone { cost_usd, .. } => assert_eq!(cost_usd, Some(i as f64)),
            ref other => panic!("expected TurnDone, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_stalled_store_backpressures_the_producer_instead_of_buffering_unboundedly() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    db.set_session_event_gate(|_, seq| {
        if seq == 0 {
            SessionEventGateAction::Delay(Duration::from_millis(800))
        } else {
            SessionEventGateAction::Proceed
        }
    });

    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;

    const TOTAL: usize = 2_500;
    let sent = Arc::new(AtomicUsize::new(0));
    let sent_for_task = sent.clone();
    let send_task = tokio::spawn(async move {
        for i in 0..TOTAL {
            ingress.send(turn_done(i as f64)).await.unwrap();
            sent_for_task.fetch_add(1, Ordering::SeqCst);
        }
    });

    tokio::time::sleep(Duration::from_millis(250)).await;
    let progressed = sent.load(Ordering::SeqCst);
    assert!(
        progressed < TOTAL,
        "a stalled consumer must backpressure the producer: {progressed} of {TOTAL} sends completed almost instantly"
    );
    assert!(
        progressed <= 2_048 + 8,
        "the producer must not race ahead of the documented 2,048-entry ingress bound (progressed={progressed})"
    );

    send_task.await.unwrap();
    assert_eq!(sent.load(Ordering::SeqCst), TOTAL);
    wait_for_log_len(&db, "s1", TOTAL).await;
    assert_eq!(db.session_log("s1").unwrap().len(), TOTAL);
}

#[tokio::test]
async fn a_stalled_attachment_lags_detaches_and_the_store_still_has_everything() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();
    let mut live = attachment.live;

    let ingress = handle.ingress("s1").await;
    const TOTAL: usize = 520;
    for i in 0..TOTAL {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", TOTAL).await;

    let mut ok_count = 0usize;
    let lagged = loop {
        match tokio::time::timeout(Duration::from_secs(2), live.recv())
            .await
            .expect("live receiver stalled")
        {
            Some(Ok(_)) => ok_count += 1,
            Some(Err(err)) => break err,
            None => panic!("channel closed before a Lagged signal was observed"),
        }
    };
    assert_eq!(
        ok_count, 512,
        "exactly the queue capacity must land before lag"
    );
    assert!(matches!(lagged, SessionStreamError::Lagged { .. }));
    assert_eq!(
        live.recv().await,
        None,
        "a lagged attachment must be detached (channel closed) afterwards"
    );

    let fresh = handle.attach("s1", false).await.unwrap();
    assert_eq!(fresh.replay.len(), TOTAL);
}

#[tokio::test]
async fn attach_after_resumes_a_lagged_reader_with_no_gap_or_duplicate() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let mut live = handle.attach("s1", false).await.unwrap().live;
    let ingress = handle.ingress("s1").await;
    const TOTAL: usize = 520;
    for i in 0..TOTAL {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", TOTAL).await;

    let mut seen = 0usize;
    let (epoch, last_seq) = loop {
        match live.recv().await {
            Some(Ok(_)) => seen += 1,
            Some(Err(SessionStreamError::Lagged {
                epoch,
                last_persisted_seq,
            })) => break (epoch, last_persisted_seq),
            None => panic!("channel closed before a Lagged signal was observed"),
        }
    };

    let mut resumed = handle
        .attach_after("s1", epoch, last_seq, false)
        .await
        .unwrap();
    assert_eq!(
        seen + resumed.replay.len(),
        TOTAL,
        "the replay must cover exactly what the lagged reader missed"
    );

    ingress.send(turn_done(999.0)).await.unwrap();
    match tokio::time::timeout(Duration::from_secs(2), resumed.live.recv())
        .await
        .expect("the resumed attachment must receive later events")
    {
        Some(Ok(SessionEvent::TurnDone { cost_usd, .. })) => assert_eq!(cost_usd, Some(999.0)),
        other => panic!("expected the next turn, got {other:?}"),
    }
}

#[tokio::test]
async fn a_non_coalescible_ephemeral_event_is_never_silently_dropped() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();
    let mut live = attachment.live;

    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::Exited {
            outcome: ExitOutcome::Detached,
            code: None,
            stderr_tail: String::new(),
        })
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), live.recv())
        .await
        .expect("live receiver timed out")
        .expect("live receiver closed early");
    assert!(matches!(
        received,
        Ok(SessionEvent::Exited {
            outcome: ExitOutcome::Detached,
            ..
        })
    ));
    assert!(
        db.session_log("s1").unwrap().is_empty(),
        "a detached exit is ephemeral and must not be persisted"
    );
}

#[tokio::test]
async fn a_row_mutation_failure_rolls_back_atomically_and_a_retry_is_safe() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let failed_once = Arc::new(AtomicBool::new(false));
    let gate_failed_once = failed_once.clone();
    db.set_session_event_gate(move |_, seq| {
        if seq == 0 && !gate_failed_once.swap(true, Ordering::SeqCst) {
            SessionEventGateAction::FailRowMutation
        } else {
            SessionEventGateAction::Proceed
        }
    });

    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    let event = SessionEvent::Init {
        provider_session_id: "p1".into(),
        model: Some("m1".into()),
    };
    ingress.send(event.clone()).await.unwrap();

    let stayed_empty = wait_until(Duration::from_millis(300), || {
        db.session_log("s1").unwrap().len() == 1
    })
    .await;
    assert!(
        !stayed_empty,
        "the failed commit must not have persisted the event"
    );
    assert!(db.session_log("s1").unwrap().is_empty());
    let unchanged = db.get_session("s1").unwrap().unwrap();
    assert_eq!(unchanged.state, SessionState::Starting);
    assert!(unchanged.provider_session_id.is_none());

    ingress.send(event).await.unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    let log = db.session_log("s1").unwrap();
    assert_eq!(log.len(), 1, "the retry must not create a duplicate row");
    assert_eq!(log[0].seq, 0);
    let updated = db.get_session("s1").unwrap().unwrap();
    assert_eq!(updated.state, SessionState::Running);
    assert_eq!(updated.provider_session_id.as_deref(), Some("p1"));
}

#[tokio::test]
async fn a_persistence_failure_prevents_fanout_to_live_attachments() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    db.set_session_event_gate(|_, _| SessionEventGateAction::FailRowMutation);

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();
    let mut live = attachment.live;

    let ingress = handle.ingress("s1").await;
    ingress.send(turn_done(1.0)).await.unwrap();

    let nothing_arrived = tokio::time::timeout(Duration::from_millis(300), live.recv())
        .await
        .is_err();
    assert!(nothing_arrived, "a failed persist must never fan out");
    assert!(db.session_log("s1").unwrap().is_empty());
}

#[tokio::test]
async fn live_delivery_preserves_the_order_events_were_ingressed() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();
    let mut live = attachment.live;
    let ingress = handle.ingress("s1").await;

    let events = vec![
        turn_done(1.0),
        SessionEvent::AssistantText {
            text: "a".into(),
            parent_id: None,
            injected: false,
        },
        turn_done(2.0),
        SessionEvent::AssistantText {
            text: "b".into(),
            parent_id: None,
            injected: false,
        },
        turn_done(3.0),
    ];
    for event in &events {
        ingress.send(event.clone()).await.unwrap();
    }

    for expected in &events {
        let received = tokio::time::timeout(Duration::from_secs(2), live.recv())
            .await
            .expect("live receiver timed out")
            .expect("live receiver closed early")
            .expect("no lag expected");
        assert_eq!(&received, expected);
    }
}

#[tokio::test]
async fn repeated_attach_and_drop_does_not_leak_attachment_state() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    for _ in 0..20 {
        let attachment = handle.attach("s1", false).await.unwrap();
        drop(attachment.live);
    }

    let ingress = handle.ingress("s1").await;
    ingress.send(turn_done(1.0)).await.unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    let start = Instant::now();
    let mut remaining = handle.attachment_count("s1").await;
    while remaining != 0 && start.elapsed() < Duration::from_secs(2) {
        tokio::time::sleep(Duration::from_millis(5)).await;
        remaining = handle.attachment_count("s1").await;
    }
    assert_eq!(
        remaining, 0,
        "dropped attachments must eventually be pruned"
    );
}

#[tokio::test]
async fn a_user_message_reaches_only_attachments_that_opted_in() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let mut opted_in = handle.attach("s1", true).await.unwrap().live;
    let mut opted_out = handle.attach("s1", false).await.unwrap().live;

    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("m1".into()),
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    let received = tokio::time::timeout(Duration::from_secs(2), opted_in.recv())
        .await
        .expect("an opted-in attachment must receive the message live")
        .expect("live receiver closed early");
    match received {
        Ok(SessionEvent::UserMessage {
            client_message_id, ..
        }) => assert_eq!(client_message_id.as_deref(), Some("m1")),
        other => panic!("expected a user-message delivery, got {other:?}"),
    }

    let nothing_arrived = tokio::time::timeout(Duration::from_millis(300), opted_out.recv())
        .await
        .is_err();
    assert!(
        nothing_arrived,
        "an attachment that did not opt in must never receive a user message live"
    );

    let fresh = handle.attach("s1", false).await.unwrap();
    assert_eq!(fresh.replay.len(), 1);
    assert!(matches!(
        fresh.replay[0].event,
        SessionEvent::UserMessage { .. }
    ));
}

struct FakeDriver {
    sent_turns: Arc<AtomicUsize>,
    collab_modes: Arc<StdMutex<Vec<Option<CollabMode>>>>,
    rewind_started: Arc<Notify>,
    rewind_delay: Duration,
    rewind_ok: bool,
}

impl FakeDriver {
    fn new() -> Self {
        Self {
            sent_turns: Arc::new(AtomicUsize::new(0)),
            collab_modes: Arc::default(),
            rewind_started: Arc::new(Notify::new()),
            rewind_delay: Duration::ZERO,
            rewind_ok: true,
        }
    }

    fn rewinding(rewind_started: Arc<Notify>, rewind_delay: Duration, rewind_ok: bool) -> Self {
        Self {
            sent_turns: Arc::new(AtomicUsize::new(0)),
            collab_modes: Arc::default(),
            rewind_started,
            rewind_delay,
            rewind_ok,
        }
    }
}

impl AgentDriver for FakeDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        broadcast::channel(1).1
    }
    fn send_user_turn(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
    ) -> CoreResult<()> {
        self.sent_turns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn set_collab_mode(&self, mode: Option<CollabMode>) {
        self.collab_modes.lock().unwrap().push(mode);
    }
    fn respond_permission(&self, _: &str, _: bool, _: Option<&str>) -> CoreResult<()> {
        Ok(())
    }
    fn answer_question(&self, _: &str, _: QuestionAnswers) -> CoreResult<()> {
        Ok(())
    }
    fn interrupt(&self) -> CoreResult<()> {
        Ok(())
    }
    fn rewind_to_ordinal<'a>(
        &'a self,
        _ordinal: usize,
    ) -> Pin<Box<dyn Future<Output = CoreResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.rewind_started.notify_one();
            if !self.rewind_delay.is_zero() {
                tokio::time::sleep(self.rewind_delay).await;
            }
            if self.rewind_ok {
                Ok(())
            } else {
                Err(CoreError::OutputParse {
                    program: "fake-driver".into(),
                    detail: "model-level rewind refused".into(),
                })
            }
        })
    }
    fn kill(&self) {}
    fn kill_detached(&self) {}
    fn pid(&self) -> Option<u32> {
        None
    }
}

#[tokio::test]
async fn interrupted_codex_phase_one_suppresses_phase_two() {
    let driver = FakeDriver::new();
    let sent_turns = driver.sent_turns.clone();
    let (tx, mut rx) = broadcast::channel(8);
    tx.send(SessionEvent::Exited {
        outcome: ExitOutcome::Interrupted,
        code: None,
        stderr_tail: String::new(),
    })
    .unwrap();

    let op_lock = Arc::new(AsyncMutex::new(()));
    run_codex_followup_for_test(&driver, &mut rx, "now execute", op_lock).await;

    assert_eq!(
        sent_turns.load(Ordering::SeqCst),
        0,
        "an interrupted phase one must not trigger a phase-two turn"
    );
}

#[tokio::test]
async fn a_skipped_codex_phase_two_leaves_plan_mode_on_the_next_turn() {
    for (status, turns) in [(Some("interrupted"), 0), (None, 0), (Some("completed"), 1)] {
        let driver = FakeDriver::new();
        let (tx, mut rx) = broadcast::channel(8);
        tx.send(SessionEvent::TurnDone {
            cost_usd: None,
            num_turns: None,
            context_tokens: None,
            turn_status: status.map(str::to_string),
        })
        .unwrap();

        let op_lock = Arc::new(AsyncMutex::new(()));
        run_codex_followup_for_test(&driver, &mut rx, "now execute", op_lock).await;

        assert_eq!(
            driver.sent_turns.load(Ordering::SeqCst),
            turns,
            "{status:?}"
        );
        assert_eq!(
            *driver.collab_modes.lock().unwrap(),
            [Some(CollabMode::Default)],
            "{status:?}: whichever way phase one ends, the next turn runs in default mode"
        );
    }
}

#[tokio::test]
async fn live_rewind_during_a_streaming_turn_commits_queued_ingress_above_the_cutoff_under_the_new_epoch(
) {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = Arc::new(SessionRuntimeHandle::new(db.clone()));
    let ingress = handle.ingress("s1").await;
    for i in 0..3 {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", 3).await;

    let rewind_started = Arc::new(Notify::new());
    let driver: Arc<dyn AgentDriver> = Arc::new(FakeDriver::rewinding(
        rewind_started.clone(),
        Duration::from_millis(200),
        true,
    ));

    let rewind_handle = handle.clone();
    let rewind_task =
        tokio::spawn(async move { rewind_handle.rewind("s1", 1, Some((driver, 1usize))).await });

    rewind_started.notified().await;
    let queued = handle.ingress("s1").await;
    queued.send(turn_done(99.0)).await.unwrap();

    let outcome = rewind_task.await.unwrap().unwrap();
    assert!(outcome.live_rewound);
    assert_eq!(outcome.epoch, 1);

    wait_for_log_len(&db, "s1", 3).await;
    let log = db.session_log("s1").unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].seq, 0);
    assert_eq!(log[1].seq, 1);
    assert_eq!(log[1].kind, "rewound");
    assert_eq!(log[2].seq, 2);
    match log[2].event {
        SessionEvent::TurnDone { cost_usd, .. } => assert_eq!(cost_usd, Some(99.0)),
        ref other => panic!("expected the queued TurnDone, got {other:?}"),
    }
}

#[tokio::test]
async fn rewind_concurrent_with_attach_reflects_post_truncation_rows_with_no_stale_delivery() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = Arc::new(SessionRuntimeHandle::new(db.clone()));
    let ingress = handle.ingress("s1").await;
    for i in 0..3 {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", 3).await;

    let rewind_started = Arc::new(Notify::new());
    let driver: Arc<dyn AgentDriver> = Arc::new(FakeDriver::rewinding(
        rewind_started.clone(),
        Duration::from_millis(200),
        true,
    ));

    let rewind_handle = handle.clone();
    let rewind_task =
        tokio::spawn(async move { rewind_handle.rewind("s1", 1, Some((driver, 1usize))).await });

    rewind_started.notified().await;
    let attachment = handle.attach("s1", false).await.unwrap();

    let outcome = rewind_task.await.unwrap().unwrap();
    assert!(outcome.live_rewound);

    assert_eq!(attachment.replay.len(), 2, "kept row plus the boundary");
    assert_eq!(attachment.replay[0].seq, 0);
    match attachment.replay[0].event {
        SessionEvent::TurnDone { cost_usd, .. } => assert_eq!(cost_usd, Some(0.0)),
        ref other => panic!("expected the pre-cutoff TurnDone, got {other:?}"),
    }
    assert!(
        matches!(attachment.replay[1].event, SessionEvent::Rewound { .. }),
        "no stale-ordinal delivery: replay must end at the Rewound boundary, not the old seq 1/2 rows"
    );
}

#[tokio::test]
async fn lag_then_rewind_then_log_after_with_the_stale_epoch_reports_epoch_advanced() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let attachment = handle.attach("s1", false).await.unwrap();
    let mut live = attachment.live;

    let ingress = handle.ingress("s1").await;
    const TOTAL: usize = 520;
    for i in 0..TOTAL {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", TOTAL).await;

    let (stale_epoch, last_seq) = loop {
        match tokio::time::timeout(Duration::from_secs(2), live.recv())
            .await
            .expect("live receiver stalled")
        {
            Some(Ok(_)) => {}
            Some(Err(SessionStreamError::Lagged {
                epoch,
                last_persisted_seq,
            })) => break (epoch, last_persisted_seq),
            None => panic!("channel closed before a Lagged signal was observed"),
        }
    };
    assert_eq!(stale_epoch, 0);

    let outcome = handle.rewind("s1", 5, None).await.unwrap();
    assert_eq!(outcome.epoch, 1);

    let stale = handle.log_after("s1", stale_epoch, last_seq).await;
    assert!(
        matches!(stale, Err(CoreError::SessionEpochAdvanced { epoch, .. }) if epoch == 1),
        "a log_after call at the stale epoch must report SessionEpochAdvanced, got {stale:?}"
    );

    let fresh = handle.attach("s1", false).await.unwrap();
    assert!(
        fresh
            .replay
            .iter()
            .any(|entry| matches!(entry.event, SessionEvent::Rewound { .. })),
        "a fresh attach after the epoch check must recover cleanly at the new epoch"
    );
}

#[tokio::test]
async fn cold_rewind_relaunch_failure_restores_the_removed_rows() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    for i in 0..5 {
        ingress.send(turn_done(i as f64)).await.unwrap();
    }
    wait_for_log_len(&db, "s1", 5).await;

    let original = db.session_log("s1").unwrap();

    let outcome = handle.rewind("s1", 2, None).await.unwrap();
    assert!(!outcome.live_rewound, "no live driver was supplied");
    assert_eq!(
        db.session_log("s1").unwrap().len(),
        3,
        "two kept rows plus the rewind boundary"
    );

    handle
        .restore_after_failed_relaunch("s1", 2, outcome.removed)
        .await
        .unwrap();

    let restored = db.session_log("s1").unwrap();
    assert_eq!(restored.len(), original.len());
    for (restored_entry, original_entry) in restored.iter().zip(original.iter()) {
        assert_eq!(restored_entry.seq, original_entry.seq);
        assert_eq!(restored_entry.kind, original_entry.kind);
    }

    let ingress = handle.ingress("s1").await;
    ingress.send(turn_done(42.0)).await.unwrap();
    wait_for_log_len(&db, "s1", original.len() + 1).await;
    let after = db.session_log("s1").unwrap();
    assert_eq!(after.last().unwrap().seq, original.len() as i64);
}
