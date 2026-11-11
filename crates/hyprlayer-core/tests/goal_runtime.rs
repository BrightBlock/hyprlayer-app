#![cfg(feature = "test-support")]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use hyprlayer_core::agent::{AgentDriver, Harness};
use hyprlayer_core::claude::SessionEvent;
use hyprlayer_core::error::Result as CoreResult;
use hyprlayer_core::models::session::QuestionAnswers;
use hyprlayer_core::pipeline::Stage;
use hyprlayer_core::runtime::sessions::{
    interrupt_then_kill, watch_iteration, SessionRuntimeHandle,
};
use hyprlayer_core::store::{Db, NewSession, NewWorkstream, WorktreeMode};

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

fn user_message(nonce: &str) -> SessionEvent {
    SessionEvent::UserMessage {
        text: "go".into(),
        stage: None,
        prompt_name: Some("hyprlayer:goal".into()),
        client_message_id: Some(nonce.into()),
        author: None,
    }
}

fn assistant(text: &str) -> SessionEvent {
    SessionEvent::AssistantText {
        text: text.into(),
        parent_id: None,
        injected: false,
        auth_required: false,
    }
}

fn turn_done(cost: f64, status: &str) -> SessionEvent {
    SessionEvent::TurnDone {
        cost_usd: Some(cost),
        num_turns: None,
        context_tokens: None,
        turn_status: Some(status.into()),
        thread_tokens: None,
    }
}

fn idle() -> Pin<Box<dyn Future<Output = bool> + Send>> {
    Box::pin(async { true })
}

async fn wait_for_log_len(db: &Db, session_id: &str, len: usize) {
    let start = Instant::now();
    while db.session_log(session_id).unwrap().len() < len {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "session log never reached {len} entries"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[derive(Default)]
struct CountingDriver {
    interrupts: AtomicUsize,
    kills: AtomicUsize,
}

impl AgentDriver for CountingDriver {
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
        Ok(())
    }
    fn respond_permission(&self, _: &str, _: bool, _: Option<&str>) -> CoreResult<()> {
        Ok(())
    }
    fn answer_question(&self, _: &str, _: QuestionAnswers) -> CoreResult<()> {
        Ok(())
    }
    fn interrupt(&self) -> CoreResult<()> {
        self.interrupts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn kill(&self) {
        self.kills.fetch_add(1, Ordering::SeqCst);
    }
    fn kill_detached(&self) {}
    fn pid(&self) -> Option<u32> {
        None
    }
}

#[tokio::test]
async fn a_turn_done_before_the_echo_is_ignored() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut watch = handle.watch("s1").await;
    let ingress = handle.ingress("s1").await;

    for event in [
        turn_done(9.0, "failed"),
        user_message("n1"),
        assistant("answer"),
        turn_done(0.4, "completed"),
    ] {
        ingress.send(event).await.unwrap();
    }
    wait_for_log_len(&db, "s1", 4).await;

    let end = watch_iteration(&mut watch, "n1", Duration::from_millis(100), &idle).await;
    assert_eq!(end.last_text.as_deref(), Some("answer"));
    assert_eq!(end.turn_status.as_deref(), Some("completed"));
    assert_eq!(end.cost_usd, Some(0.4));
    assert!(!end.steered);
}

#[tokio::test]
async fn a_foreign_message_in_the_quiet_window_extends_the_iteration() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut watch = handle.watch("s1").await;
    let ingress = handle.ingress("s1").await;

    let quiet = Duration::from_millis(600);
    let started = Instant::now();
    let steering = async {
        for event in [
            user_message("n1"),
            assistant("first"),
            turn_done(1.0, "completed"),
        ] {
            ingress.send(event).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        for event in [
            user_message("steer"),
            assistant("second"),
            turn_done(2.0, "completed"),
        ] {
            ingress.send(event).await.unwrap();
        }
    };
    let (end, ()) = tokio::join!(watch_iteration(&mut watch, "n1", quiet, &idle), steering);

    assert!(end.steered);
    assert_eq!(end.last_text.as_deref(), Some("second"));
    assert!(
        started.elapsed() >= Duration::from_millis(750),
        "the window restarts after the steered turn: ended after {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_lagged_watch_recovers_from_the_log() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    let mut watch = handle.watch("s1").await;
    let ingress = handle.ingress("s1").await;

    const FILLER: usize = 520;
    for i in 0..FILLER {
        ingress
            .send(turn_done(i as f64, "completed"))
            .await
            .unwrap();
    }
    for event in [
        user_message("n1"),
        assistant("recovered"),
        turn_done(7.0, "completed"),
    ] {
        ingress.send(event).await.unwrap();
    }
    wait_for_log_len(&db, "s1", FILLER + 3).await;

    let end = watch_iteration(&mut watch, "n1", Duration::from_millis(100), &idle).await;
    assert_eq!(end.last_text.as_deref(), Some("recovered"));
    assert_eq!(end.cost_usd, Some(7.0));
}

#[tokio::test]
async fn the_cap_interrupts_then_kills() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    seed_session(&db, "s1");
    let handle = SessionRuntimeHandle::new(db.clone());
    // Spawn the actor and take the attachment before pausing time.
    let mut watch = handle.watch("s1").await;
    let driver = CountingDriver::default();

    tokio::time::pause();
    let observe = async {
        tokio::task::yield_now().await;
        assert_eq!(
            driver.interrupts.load(Ordering::SeqCst),
            1,
            "interrupted at once"
        );
        assert_eq!(driver.kills.load(Ordering::SeqCst), 0);
        tokio::time::advance(Duration::from_secs(9)).await;
        assert_eq!(
            driver.kills.load(Ordering::SeqCst),
            0,
            "the harness keeps its 10 s"
        );
        tokio::time::advance(Duration::from_secs(2)).await;
    };
    let (_, _escalation) = tokio::join!(
        observe,
        interrupt_then_kill(&driver, &mut watch, Duration::from_secs(10))
    );

    assert_eq!(driver.interrupts.load(Ordering::SeqCst), 1);
    assert_eq!(driver.kills.load(Ordering::SeqCst), 1);
}
