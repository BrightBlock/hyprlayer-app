use std::sync::Arc;

use tokio::sync::{broadcast, Mutex as AsyncMutex};

use hyprlayer_models::ClientRef;

use crate::agent::{AgentDriver, Harness};
use crate::claude::{SessionEvent, TurnStatus};
use crate::codex::CollabMode;
use crate::pipeline::{self, Stage};

use super::SessionActors;

#[derive(Debug, PartialEq)]
pub(crate) enum Followup {
    ShipDescribe,
    CodexExecute(&'static str),
}

pub(crate) fn followup_for(
    harness: Harness,
    stage: Stage,
    bodies: &pipeline::ActionPromptBodies,
) -> Option<Followup> {
    if stage == Stage::Ship {
        return Some(Followup::ShipDescribe);
    }
    pipeline::codex_phase_two(harness, stage, bodies).map(Followup::CodexExecute)
}

pub(crate) async fn run_followup(
    session_id: &str,
    driver: &dyn AgentDriver,
    rx: &mut broadcast::Receiver<SessionEvent>,
    followup: Followup,
    op_lock: Arc<AsyncMutex<()>>,
    actors: &SessionActors,
    author: Option<ClientRef>,
) {
    let status = loop {
        match rx.recv().await {
            Ok(SessionEvent::TurnDone { turn_status, .. }) => break turn_status,
            Ok(SessionEvent::Exited { .. }) => {
                tracing::info!(
                    "session {session_id}: exited before its first turn ended; no follow-up"
                );
                return;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    "session {session_id}: follow-up abandoned after missing {n} events"
                );
                if matches!(followup, Followup::CodexExecute(_)) {
                    driver.set_collab_mode(Some(CollabMode::Default));
                }
                return;
            }
            Err(broadcast::error::RecvError::Closed) => return,
            Ok(_) => {}
        }
    };
    match followup {
        Followup::ShipDescribe => {
            tracing::info!("session {session_id}: sending the ship describe follow-up");
            let nonce = reserve_followup_turn(session_id, actors, author).await;
            if let Err(e) =
                driver.send_user_turn(pipeline::SHIP_DESCRIBE_PROMPT, None, None, Some(&nonce))
            {
                tracing::warn!("session {session_id}: the describe follow-up was not sent: {e}");
            }
        }
        Followup::CodexExecute(prompt) => {
            // Destructive follow-up: only after a cleanly completed review turn.
            // Codex reports a Stop as "interrupted" (verified on 0.149.1), but an
            // absent or unrecognised status must fail safe too, so this requires
            // an exact Completed rather than excluding the statuses we know.
            if status.as_deref().map(TurnStatus::parse) != Some(TurnStatus::Completed) {
                tracing::info!(
                    "session {session_id}: phase one ended with status {status:?}; phase two skipped"
                );
                // Plan mode is sticky: the user's next turn would run in it too.
                driver.set_collab_mode(Some(CollabMode::Default));
                return;
            }
            let observed = driver.observed_collab_mode();
            if observed.as_deref() != Some("plan") {
                tracing::warn!(
                    "session {session_id}: codex never confirmed plan mode for phase one \
                     (observed {observed:?}); its questions had no card to answer them"
                );
            }
            let _guard = op_lock.lock().await;
            driver.set_collab_mode(Some(CollabMode::Default));
            tracing::info!("session {session_id}: starting phase two");
            let nonce = reserve_followup_turn(session_id, actors, author).await;
            if let Err(e) = driver.send_user_turn(prompt, None, None, Some(&nonce)) {
                tracing::warn!("session {session_id}: phase two was not sent: {e}");
            }
        }
    }
}

/// Reserves a follow-up's turn under the trigger's author (R3), minting the
/// nonce the actor matches the echoed `UserMessage` by. Reserving even with
/// no author still blocks another client's send until this one commits.
async fn reserve_followup_turn(
    session_id: &str,
    actors: &SessionActors,
    author: Option<ClientRef>,
) -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    if let Err(e) = actors
        .reserve_user_turn(session_id, nonce.clone(), None, author)
        .await
    {
        tracing::warn!("session {session_id}: follow-up reservation failed: {e}");
    }
    nonce
}

#[cfg(any(test, feature = "test-support"))]
pub async fn run_codex_followup_for_test(
    driver: &dyn AgentDriver,
    rx: &mut broadcast::Receiver<SessionEvent>,
    prompt: &'static str,
    op_lock: Arc<AsyncMutex<()>>,
) {
    // No caller reaches the actor from this helper's callers, so it builds its
    // own throwaway one; the reservation it takes for session "test" is never read.
    let actors = SessionActors::new(
        Arc::new(crate::store::Db::open_in_memory().expect("in-memory db for a test followup")),
        broadcast::channel(8).0,
    );
    run_followup(
        "test",
        driver,
        rx,
        Followup::CodexExecute(prompt),
        op_lock,
        &actors,
        None,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followup_dispatch_matrix() {
        let bodies = pipeline::ActionPromptBodies::default();
        for harness in [Harness::Claude, Harness::Codex, Harness::Opencode] {
            assert_eq!(
                followup_for(harness, Stage::Ship, &bodies),
                Some(Followup::ShipDescribe),
                "ship's describe follow-up is harness-independent"
            );
        }
        assert!(matches!(
            followup_for(Harness::Codex, Stage::Commit, &bodies),
            Some(Followup::CodexExecute(_))
        ));
        assert_eq!(followup_for(Harness::Claude, Stage::Commit, &bodies), None);
        assert_eq!(
            followup_for(Harness::Opencode, Stage::Commit, &bodies),
            None
        );
        assert_eq!(
            followup_for(Harness::Codex, Stage::Implement, &bodies),
            None
        );
        for stage in [Stage::Plan, Stage::Research] {
            assert!(
                matches!(
                    followup_for(Harness::Codex, stage, &bodies),
                    Some(Followup::CodexExecute(_))
                ),
                "codex {stage:?} collaborates before it writes"
            );
            assert_eq!(followup_for(Harness::Claude, stage, &bodies), None);
            assert_eq!(followup_for(Harness::Opencode, stage, &bodies), None);
        }
    }
}
