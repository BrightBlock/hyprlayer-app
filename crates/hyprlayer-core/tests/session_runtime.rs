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
use hyprlayer_core::models::session::{ClientRef, QuestionAnswers, RevisedTurn, TurnKind};
use hyprlayer_core::models::RuntimeEvent;
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::runtime::sessions::{
    run_codex_followup_for_test, SessionRuntimeHandle, SessionStreamError,
};
use hyprlayer_core::store::{
    Db, LogCursor, NewSession, NewWorkstream, SessionEventGateAction, StoredEvent, WorktreeMode,
};

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
        thread_tokens: None,
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

fn client_ref(id: &str) -> ClientRef {
    ClientRef {
        client_id: id.to_string(),
        device: format!("{id}-device"),
    }
}

/// The fields of a `SessionRevised` worth asserting on.
struct Revised {
    rev: u64,
    author: Option<ClientRef>,
    working: bool,
    turns: Vec<RevisedTurn>,
}

/// Drains `events` up to and including the next `SessionRevised`, skipping any
/// `SessionLifecycle` in between.
async fn next_session_revised(events: &mut broadcast::Receiver<RuntimeEvent>) -> Revised {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("timed out waiting for a SessionRevised")
            .expect("events channel closed")
        {
            RuntimeEvent::SessionRevised {
                rev,
                author,
                working,
                turns,
                ..
            } => {
                return Revised {
                    rev,
                    author,
                    working,
                    turns,
                }
            }
            _ => continue,
        }
    }
}

/// Drains `events` up to and including the next `SessionReset`, returning its rev.
async fn next_session_reset(events: &mut broadcast::Receiver<RuntimeEvent>) -> u64 {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("timed out waiting for a SessionReset")
            .expect("events channel closed")
        {
            RuntimeEvent::SessionReset { rev, .. } => return rev,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn reading_an_unknown_session_leaves_no_actor() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let handle = SessionRuntimeHandle::new(db);
    let metrics = tokio::runtime::Handle::current().metrics();
    let before = metrics.num_alive_tasks();
    for n in 0..100 {
        let read = handle
            .list_turns(&format!("no-such-session-{n}"), LogCursor::Tail, 10, None)
            .await;
        assert!(
            matches!(read, Err(CoreError::SessionNotFound { .. })),
            "{read:?}"
        );
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(metrics.num_alive_tasks(), before);
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
        mcp_servers: None,
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
            auth_required: false,
        },
        turn_done(2.0),
        SessionEvent::AssistantText {
            text: "b".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
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
            author: None,
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
            thread_tokens: None,
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
    let mut events = handle.subscribe_events();
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
    // None of the turnDone rows advance rev, so the boundary itself is the first.
    assert_eq!(next_session_reset(&mut events).await, 1);

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

    // The restore has no row of its own, so it must bump rev/reset_rev directly
    // and still emit a SessionReset (case 14).
    assert_eq!(next_session_reset(&mut events).await, 2);
    let turns = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(turns.rev, 2);
    assert_eq!(turns.reset_rev, 2);

    let ingress = handle.ingress("s1").await;
    ingress.send(turn_done(42.0)).await.unwrap();
    wait_for_log_len(&db, "s1", original.len() + 1).await;
    let after = db.session_log("s1").unwrap();
    assert_eq!(after.last().unwrap().seq, original.len() as i64);
}

// --- Revisions, reservations and authorship (Phase 1 §5/§6) ---
//
// Each test below is a failure mode from the plan's failure list: the
// assertions verify the bad outcome named in its doc comment does *not*
// happen. An E2E scenario cannot reach any of them (time-based expiry,
// forced persist failures, races only one actor's serialization prevents,
// mid-turn attribution the fake harness can't drive, and undecodable rows).

/// 1. A duplicate `(seq, payload)` (`AlreadyApplied`) must not bump rev again.
#[tokio::test]
async fn already_applied_does_not_bump_rev_again() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    // Two actors sharing one db, both spawned before either writes, so both
    // start at seq 0: the second's write collides with the first's row.
    let handle1 = SessionRuntimeHandle::new(db.clone());
    let handle2 = SessionRuntimeHandle::new(db.clone());
    let ingress1 = handle1.ingress("s1").await;
    let ingress2 = handle2.ingress("s1").await;

    let event = SessionEvent::UserMessage {
        text: "hi".into(),
        stage: None,
        prompt_name: None,
        client_message_id: None,
        author: None,
    };
    ingress1.send(event.clone()).await.unwrap();
    wait_for_log_len(&db, "s1", 1).await;
    ingress2.send(event).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A fresh third actor reads the db's rev directly.
    let handle3 = SessionRuntimeHandle::new(db.clone());
    let turns = handle3
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(
        turns.rev, 1,
        "a duplicate (seq, payload) must not bump rev again"
    );
}

/// 2. `thinking`, `toolUse`, `turnDone`, `usage` and `partialText` must not bump rev.
#[tokio::test]
async fn bookkeeping_kinds_do_not_bump_rev() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;

    ingress
        .send(SessionEvent::Thinking {
            text: "t".into(),
            parent_id: None,
        })
        .await
        .unwrap();
    ingress
        .send(SessionEvent::ToolUse {
            id: "t1".into(),
            name: "grep".into(),
            input: "{}".into(),
            parent_id: None,
        })
        .await
        .unwrap();
    ingress.send(turn_done(1.0)).await.unwrap();
    ingress
        .send(SessionEvent::Usage {
            context_tokens: 10,
            thread_tokens: None,
        })
        .await
        .unwrap();
    ingress
        .send(SessionEvent::PartialText {
            text: "p".into(),
            parent_id: None,
        })
        .await
        .unwrap();
    // PartialText is ephemeral; the other four persist.
    wait_for_log_len(&db, "s1", 4).await;

    let turns = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(turns.rev, 0);
}

/// 3. A reservation whose `UserMessage` was persisted must still count, so
/// another author's next send at the older base is refused.
#[tokio::test]
async fn a_committed_reservation_still_makes_a_foreign_send_stale() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let a = client_ref("client-a");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();
    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    let b = client_ref("client-b");
    let err = handle
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(b))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 1,
                committed_rev: 1,
                missing_turns: 1,
                ..
            }
        ),
        "expected StaleTranscript(1, 1, 1), got {err:?}"
    );
}

/// 4. A reservation whose `UserMessage` failed to persist must not leak.
#[tokio::test]
async fn a_reservation_whose_user_message_failed_to_persist_does_not_leak() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    db.set_session_event_gate(|_, seq| {
        if seq == 0 {
            SessionEventGateAction::FailRowMutation
        } else {
            SessionEventGateAction::Proceed
        }
    });

    let handle = SessionRuntimeHandle::new(db.clone());
    let a = client_ref("client-a");
    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();
    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    // The gated persist fails; give the actor a moment to process it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    db.clear_session_event_gate();

    let b = client_ref("client-b");
    handle
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(b))
        .await
        .expect("a reservation settled by a failed persist must not leak");
}

/// 5. A reservation whose `UserMessage` never arrives must expire after the
/// TTL (not linger forever, and not need another send to notice), and its
/// expiry must emit a same-rev `SessionRevised`.
/// 31s mirrors `RESERVATION_TTL` (30s in forwarder.rs, private to that module).
#[tokio::test]
async fn a_leaked_reservation_expires_after_the_ttl() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    // Spawn the actor (and its startup blocking read) before pausing time.
    let _ingress = handle.ingress("s1").await;
    let mut events = handle.subscribe_events();

    tokio::time::pause();

    let a = client_ref("client-a");
    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();

    tokio::time::advance(Duration::from_secs(31)).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.rev, 0, "expiry announces the unchanged rev");
    assert!(revised.turns.is_empty());

    let b = client_ref("client-b");
    handle
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(b))
        .await
        .expect("an expired reservation must not still block a foreign sender");
}

/// 6. Two reservations at one base_rev from different authors, with no
/// `UserMessage` between them, must not both pass: the second is refused
/// with `rev = committed + 1`.
#[tokio::test]
async fn a_second_authors_reservation_is_refused_while_the_first_is_still_in_flight() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let a = client_ref("client-a");
    let b = client_ref("client-b");

    handle
        .reserve_user_turn("s1", "a1".into(), Some(0), Some(a))
        .await
        .unwrap();
    let err = handle
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(b))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 1,
                committed_rev: 0,
                missing_turns: 1,
                ..
            }
        ),
        "expected StaleTranscript(1, 0, 1), got {err:?}"
    );
}

/// 7. None of a client's own in-flight reservation, its own committed turn,
/// or its own turn's app follow-up may make its own next send stale.
#[tokio::test]
async fn a_clients_own_activity_never_makes_its_own_next_send_stale() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let a = client_ref("client-a");

    // (a) an own in-flight reservation
    handle
        .reserve_user_turn("s1", "a1".into(), Some(0), Some(a.clone()))
        .await
        .unwrap();
    handle
        .reserve_user_turn("s1", "a2".into(), Some(0), Some(a.clone()))
        .await
        .expect("a's own in-flight reservation must not make a's next send stale");

    // Commit "a1" as A's turn (rev 1), so (b) can check an own *committed* turn.
    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    // (b) an own committed turn
    handle
        .reserve_user_turn("s1", "a3".into(), Some(1), Some(a.clone()))
        .await
        .expect("a's own committed turn must not make a's next send stale");

    // (c) an own turn's app follow-up, reserved under its author
    handle
        .reserve_user_turn("s1", "a-followup".into(), None, Some(a.clone()))
        .await
        .unwrap();
    handle
        .reserve_user_turn("s1", "a4".into(), Some(1), Some(a))
        .await
        .expect("a's own follow-up reservation must not make a's next send stale");
}

/// 8. Another author's reservation must still make this client's send stale
/// when this client committed after that author's last commit: A drives revs
/// to 8, B reserves at base 8, then A's own send at base 8 is refused with rev 9.
#[tokio::test]
async fn another_authors_in_flight_reservation_counts_from_the_committed_rev() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    let a = client_ref("client-a");
    let b = client_ref("client-b");

    for i in 0..4 {
        handle
            .reserve_user_turn("s1", format!("a-{i}"), None, Some(a.clone()))
            .await
            .unwrap();
        ingress
            .send(SessionEvent::UserMessage {
                text: format!("turn {i}"),
                stage: None,
                prompt_name: None,
                client_message_id: Some(format!("a-{i}")),
                author: None,
            })
            .await
            .unwrap();
        ingress
            .send(SessionEvent::AssistantText {
                text: "ok".into(),
                parent_id: None,
                injected: false,
                auth_required: false,
            })
            .await
            .unwrap();
    }
    wait_for_log_len(&db, "s1", 8).await;

    handle
        .reserve_user_turn("s1", "b-pull".into(), Some(8), Some(b))
        .await
        .expect("B's own read is caught up, so its reservation at base 8 must be accepted");

    let err = handle
        .reserve_user_turn("s1", "a-4".into(), Some(8), Some(a))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 9,
                committed_rev: 8,
                missing_turns: 1,
                ..
            }
        ),
        "expected StaleTranscript(9, 8, 1), got {err:?}"
    );
}

/// 9. Neither an author-less commit (a released desktop's turn) nor the revs
/// that existed when a fresh actor spawned may be taken as any one client's own.
#[tokio::test]
async fn authorless_and_pre_spawn_history_are_foreign_to_every_client() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");

    // (a) an author-less commit
    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    ingress
        .send(SessionEvent::UserMessage {
            text: "legacy".into(),
            stage: None,
            prompt_name: None,
            client_message_id: None,
            author: None,
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    let a = client_ref("client-a");
    let err = handle
        .reserve_user_turn("s1", "a1".into(), Some(0), Some(a))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 1,
                committed_rev: 1,
                missing_turns: 1,
                ..
            }
        ),
        "an author-less commit must be foreign to every client: {err:?}"
    );

    // (b) history that predates a fresh actor's spawn
    let fresh = SessionRuntimeHandle::new(db.clone());
    let b = client_ref("client-b");
    let err = fresh
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(b))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 1,
                committed_rev: 1,
                missing_turns: 1,
                ..
            }
        ),
        "history predating a fresh actor must be foreign to every client: {err:?}"
    );
}

/// 10. A `UserMessage` arriving mid-turn must keep its own author on its own
/// revision and leave the running turn's author until `TurnDone`. `TurnDone`
/// closes the turn; the parked author opens the next one when its first item lands.
#[tokio::test]
async fn a_mid_turn_user_message_keeps_the_running_turns_author_until_turndone() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let ingress = handle.ingress("s1").await;

    let a = client_ref("client-a");
    let b = client_ref("client-b");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a.clone()))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::UserMessage {
            text: "from a".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.author, Some(a.clone()));
    assert_eq!(revised.rev, 1);

    // The turn is still running: an assistant block is attributed to A.
    ingress
        .send(SessionEvent::AssistantText {
            text: "partial reply".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.author, Some(a.clone()));
    assert!(revised.working);

    // B's message arrives mid-turn: its own revision is attributed to B...
    handle
        .reserve_user_turn("s1", "b1".into(), None, Some(b.clone()))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::UserMessage {
            text: "from b".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("b1".into()),
            author: None,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(
        revised.author,
        Some(b.clone()),
        "a mid-turn message keeps its own author"
    );
    assert!(revised.working);

    // ...but the still-running turn's own next item stays attributed to A.
    ingress
        .send(SessionEvent::AssistantText {
            text: "still a's turn".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(
        revised.author,
        Some(a),
        "the running turn keeps its own author until TurnDone"
    );

    // TurnDone always closes: a harness may have answered B's message already.
    ingress.send(turn_done(1.0)).await.unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.rev, 4, "TurnDone itself does not advance rev");
    assert!(!revised.working, "TurnDone closes the turn");

    // B's turn opens under B when its first item lands.
    ingress
        .send(SessionEvent::AssistantText {
            text: "b's reply".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(
        revised.author,
        Some(b),
        "the parked author opens the next turn"
    );
    assert!(revised.working);
}

/// 21. A harness that answers a mid-turn message together with the running
/// turn sends one `TurnDone` for both: the turn must close (`working: false`),
/// and the next send must open a turn under its own author, not the parked one.
#[tokio::test]
async fn a_harness_merging_a_mid_turn_message_leaves_the_session_idle() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let ingress = handle.ingress("s1").await;
    let a = client_ref("client-a");
    let b = client_ref("client-b");

    for (nonce, author) in [("b1", &b), ("a1", &a)] {
        handle
            .reserve_user_turn("s1", nonce.into(), None, Some(author.clone()))
            .await
            .unwrap();
        ingress
            .send(SessionEvent::UserMessage {
                text: nonce.into(),
                stage: None,
                prompt_name: None,
                client_message_id: Some(nonce.into()),
                author: None,
            })
            .await
            .unwrap();
        let _ = next_session_revised(&mut events).await;
    }
    // One reply and one TurnDone answer both messages.
    ingress
        .send(SessionEvent::AssistantText {
            text: "both".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.author, Some(b.clone()));
    ingress.send(turn_done(1.0)).await.unwrap();
    let revised = next_session_revised(&mut events).await;
    assert!(
        !revised.working,
        "one TurnDone for a merged turn must close it"
    );
    let turns = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert!(!turns.working, "list_turns must not report a phantom turn");

    // A's next send opens its own turn; the parked author is not charged to it.
    handle
        .reserve_user_turn("s1", "a2".into(), Some(3), Some(a.clone()))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::UserMessage {
            text: "a2".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a2".into()),
            author: None,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.author, Some(a.clone()));
    assert!(revised.working);
    ingress
        .send(SessionEvent::AssistantText {
            text: "reply".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.author, Some(a), "the reply belongs to A's own turn");
    ingress.send(turn_done(2.0)).await.unwrap();
    let revised = next_session_revised(&mut events).await;
    assert!(!revised.working);
}

/// 11. `ReleaseUserTurn` must emit a same-rev `SessionRevised` only when a
/// reservation actually existed; releasing an unknown nonce must not.
#[tokio::test]
async fn release_user_turn_announces_only_when_a_reservation_actually_existed() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let a = client_ref("client-a");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();
    handle.release_user_turn("s1", "a1".into()).await;
    let revised = next_session_revised(&mut events).await;
    assert_eq!(revised.rev, 0);
    assert!(revised.turns.is_empty());

    handle.release_user_turn("s1", "unknown".into()).await;
    let nothing = tokio::time::timeout(Duration::from_millis(300), events.recv()).await;
    assert!(
        nothing.is_err(),
        "releasing an unknown nonce must not announce a revision"
    );
}

/// 12. A `TurnDone` or `Exited` whose persist fails must still close the turn,
/// clear its reservations, and announce the turn's end.
#[tokio::test]
async fn a_failed_persist_still_closes_the_turn_and_announces_its_end() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let ingress = handle.ingress("s1").await;
    let a = client_ref("client-a");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    let _ = next_session_revised(&mut events).await;

    // TurnDone's own row-write fails.
    db.set_session_event_gate(|_, _| SessionEventGateAction::FailRowMutation);
    ingress.send(turn_done(1.0)).await.unwrap();
    let revised = next_session_revised(&mut events).await;
    assert!(
        !revised.working,
        "the turn must close even though its TurnDone failed to persist"
    );
    db.clear_session_event_gate();

    let b = client_ref("client-b");
    handle
        .reserve_user_turn("s1", "b1".into(), Some(1), Some(b))
        .await
        .expect("a closed turn must not leave a stale in-flight reservation");

    // Exited's own row-write also fails: its reservations must still clear
    // and its end must still be announced.
    handle
        .reserve_user_turn("s1", "b2".into(), None, Some(client_ref("client-b")))
        .await
        .unwrap();
    db.set_session_event_gate(|_, _| SessionEventGateAction::FailRowMutation);
    ingress
        .send(SessionEvent::Exited {
            outcome: ExitOutcome::Crashed,
            code: Some(1),
            stderr_tail: String::new(),
        })
        .await
        .unwrap();
    let revised = next_session_revised(&mut events).await;
    assert!(!revised.working);
    db.clear_session_event_gate();

    let c = client_ref("client-c");
    handle
        .reserve_user_turn("s1", "c1".into(), Some(1), Some(c))
        .await
        .expect("Exited must clear reservations even when its own persist fails");
}

/// 13. `Exited` must clear any reservations left in flight.
#[tokio::test]
async fn exited_clears_any_reservations_left_in_flight() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    let a = client_ref("client-a");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::Exited {
            outcome: ExitOutcome::Crashed,
            code: Some(1),
            stderr_tail: String::new(),
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    let b = client_ref("client-b");
    handle
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(b))
        .await
        .expect("Exited must clear any reservations left in flight");
}

/// An exit is news to every viewer, so it advances rev; it is no turn a send could be stale against.
#[tokio::test]
async fn an_exit_bumps_rev_without_making_a_send_stale() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let ingress = handle.ingress("s1").await;

    ingress
        .send(SessionEvent::Exited {
            outcome: ExitOutcome::SpawnFailed,
            code: None,
            stderr_tail: "no such model".into(),
        })
        .await
        .unwrap();

    assert_eq!(next_session_revised(&mut events).await.rev, 1);
    let turns = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(turns.rev, 1);
    handle
        .reserve_user_turn("s1", "b1".into(), Some(0), Some(client_ref("client-b")))
        .await
        .expect("an exit must not refuse a send from before it");
}

/// 14a. A rewind must bump rev, set reset_rev, raise the foreign floor, and
/// emit `SessionReset`. (`restore_after_failed_relaunch`'s half of case 14 is
/// covered by the extended `cold_rewind_relaunch_failure_restores_the_removed_rows`.)
#[tokio::test]
async fn a_rewind_bumps_rev_sets_reset_rev_raises_the_foreign_floor_and_emits_session_reset() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let ingress = handle.ingress("s1").await;
    let a = client_ref("client-a");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a.clone()))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    ingress
        .send(SessionEvent::AssistantText {
            text: "reply".into(),
            parent_id: None,
            injected: false,
            auth_required: false,
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 2).await;
    let _ = next_session_revised(&mut events).await;
    let _ = next_session_revised(&mut events).await;

    let outcome = handle.rewind("s1", 1, None).await.unwrap();
    assert!(!outcome.live_rewound, "no live driver was supplied");

    // Cutting at seq 1 keeps the userMessage; the boundary lands at seq 1 and
    // is itself a rev-advancing kind, so rev goes from 2 to 3.
    let reset_rev = next_session_reset(&mut events).await;
    assert_eq!(reset_rev, 3);

    let turns = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(turns.rev, 3);
    assert_eq!(turns.reset_rev, 3);

    // The foreign floor is raised: A's own pre-rewind base no longer excuses it.
    let err = handle
        .reserve_user_turn("s1", "a2".into(), Some(2), Some(a))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 3,
                committed_rev: 3,
                missing_turns: 1,
                ..
            }
        ),
        "expected StaleTranscript(3, 3, 1), got {err:?}"
    );
}

/// 15. A `SessionRevised` must carry its riding `toolUse`/`thinking` seqs, a
/// fresh (non-stale) rev, and `working: false` once a `TurnDone` leaves
/// nothing queued.
#[tokio::test]
async fn a_session_revised_carries_its_riders_a_fresh_rev_and_working_state() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut events = handle.subscribe_events();
    let ingress = handle.ingress("s1").await;

    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: None,
            author: None,
        })
        .await
        .unwrap();
    let user_revised = next_session_revised(&mut events).await;
    assert_eq!(user_revised.rev, 1);

    ingress
        .send(SessionEvent::Thinking {
            text: "hmm".into(),
            parent_id: None,
        })
        .await
        .unwrap();
    ingress
        .send(SessionEvent::ToolUse {
            id: "t1".into(),
            name: "grep".into(),
            input: "{}".into(),
            parent_id: None,
        })
        .await
        .unwrap();
    ingress
        .send(SessionEvent::ToolResult {
            tool_use_id: "t1".into(),
            content: "ok".into(),
            is_error: false,
            parent_id: None,
        })
        .await
        .unwrap();
    let reply_revised = next_session_revised(&mut events).await;
    assert_eq!(
        reply_revised.rev, 2,
        "only toolResult advances rev; the reported rev must not be stale"
    );
    assert_eq!(
        reply_revised.turns.len(),
        3,
        "thinking and toolUse must ride with the commit that follows them"
    );
    let kinds: Vec<TurnKind> = reply_revised.turns.iter().map(|t| t.kind).collect();
    assert!(kinds.contains(&TurnKind::Assistant));
    assert!(kinds.contains(&TurnKind::Tool));

    ingress.send(turn_done(1.0)).await.unwrap();
    let done_revised = next_session_revised(&mut events).await;
    assert!(
        !done_revised.working,
        "no queued turn: TurnDone must report working: false"
    );
    assert!(done_revised.turns.is_empty());
}

/// 16. `list_turns`' rev must match its rows even racing a delayed concurrent
/// commit: the actor serializes control messages against ingress inline.
#[tokio::test]
async fn list_turns_rev_matches_its_rows_even_racing_a_concurrent_commit() {
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
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: None,
            author: None,
        })
        .await
        .unwrap();

    let turns = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    let counted = turns
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.event,
                SessionEvent::UserMessage { .. }
                    | SessionEvent::AssistantText { .. }
                    | SessionEvent::ToolResult { .. }
                    | SessionEvent::Rewound { .. }
            )
        })
        .count() as u64;
    assert_eq!(
        turns.rev, counted,
        "rev must always match the rows a consistent read returns"
    );
    db.clear_session_event_gate();
}

/// 18. `list_turns` must take `next_cursor` from the last raw row, so a page
/// whose trailing rows don't decode is still resumable rather than read forever.
#[tokio::test]
async fn list_turns_resumes_past_an_undecodable_trailing_row() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    ingress.send(turn_done(1.0)).await.unwrap();
    wait_for_log_len(&db, "s1", 1).await;

    db.restore_session_events(
        "s1",
        &[StoredEvent {
            seq: 1.0,
            kind: "turnDone".into(),
            payload: "not valid json".into(),
            created_at: 12345.0,
        }],
    )
    .unwrap();

    let page = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(
        page.entries.len(),
        1,
        "the undecodable row is skipped, not returned"
    );
    assert_eq!(
        page.next_cursor,
        LogCursor::After {
            seq: 1,
            created_at: 12345.0
        },
        "next_cursor must name the raw row even though it didn't decode"
    );
    assert_eq!(page.remaining, 0);

    let next = handle
        .list_turns("s1", page.next_cursor, 100, None)
        .await
        .unwrap();
    assert!(next.entries.is_empty());
    assert_eq!(next.remaining, 0);
}

/// 19. `list_turns`' `in_flight` must exclude the caller's own reservations.
#[tokio::test]
async fn list_turns_in_flight_excludes_the_callers_own_reservations() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let a = client_ref("client-a");
    let b = client_ref("client-b");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a))
        .await
        .unwrap();
    handle
        .reserve_user_turn("s1", "b1".into(), None, Some(b))
        .await
        .unwrap();

    let as_a = handle
        .list_turns("s1", LogCursor::Empty, 100, Some("client-a".into()))
        .await
        .unwrap();
    assert_eq!(as_a.in_flight, 1, "only B's reservation is foreign to A");

    let as_unidentified = handle
        .list_turns("s1", LogCursor::Empty, 100, None)
        .await
        .unwrap();
    assert_eq!(
        as_unidentified.in_flight, 2,
        "an unidentified caller counts every reservation"
    );
}

/// 20. A nonce-less `UserMessage` right after an announced client's turn must
/// not be attributed to that client: it must be author-less, raising the
/// foreign floor so that same client's next send at its own older base is stale.
#[tokio::test]
async fn a_nonceless_message_after_an_announced_clients_turn_is_not_attributed_to_it() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let ingress = handle.ingress("s1").await;
    let a = client_ref("client-a");

    handle
        .reserve_user_turn("s1", "a1".into(), None, Some(a.clone()))
        .await
        .unwrap();
    ingress
        .send(SessionEvent::UserMessage {
            text: "hi".into(),
            stage: None,
            prompt_name: None,
            client_message_id: Some("a1".into()),
            author: None,
        })
        .await
        .unwrap();
    ingress.send(turn_done(1.0)).await.unwrap();
    wait_for_log_len(&db, "s1", 2).await;

    // A nonce-less message right after A's turn: no reservation to match.
    ingress
        .send(SessionEvent::UserMessage {
            text: "raw rpc".into(),
            stage: None,
            prompt_name: None,
            client_message_id: None,
            author: None,
        })
        .await
        .unwrap();
    wait_for_log_len(&db, "s1", 3).await;

    // It is author-less and foreign to everyone, A included: A's own last
    // known base (rev 1, from its own turn) is now stale.
    let err = handle
        .reserve_user_turn("s1", "a2".into(), Some(1), Some(a))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CoreError::StaleTranscript {
                rev: 2,
                committed_rev: 2,
                missing_turns: 1,
                ..
            }
        ),
        "expected StaleTranscript(2, 2, 1), got {err:?}"
    );
}
