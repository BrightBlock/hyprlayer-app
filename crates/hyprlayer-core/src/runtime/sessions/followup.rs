use tokio::sync::broadcast;

use crate::agent::{AgentDriver, Harness};
use crate::claude::{SessionEvent, TurnStatus};
use crate::codex::CollabMode;
use crate::pipeline::{self, Stage};

#[derive(Debug, Clone, PartialEq)]
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

/// What a follow-up sends once its first turn has ended.
pub(crate) struct FollowupTurn {
    pub prompt: &'static str,
    pub collab_mode: Option<CollabMode>,
}

/// Waits for the first turn to end and says what to send next; sending is the caller's.
pub(crate) async fn await_followup(
    session_id: &str,
    driver: &dyn AgentDriver,
    rx: &mut broadcast::Receiver<SessionEvent>,
    followup: Followup,
) -> Option<FollowupTurn> {
    let status = loop {
        match rx.recv().await {
            Ok(SessionEvent::TurnDone { turn_status, .. }) => break turn_status,
            Ok(SessionEvent::Exited { .. }) => {
                tracing::info!(
                    "session {session_id}: exited before its first turn ended; no follow-up"
                );
                return None;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    "session {session_id}: follow-up abandoned after missing {n} events"
                );
                if matches!(followup, Followup::CodexExecute(_)) {
                    driver.set_collab_mode(Some(CollabMode::Default));
                }
                return None;
            }
            Err(broadcast::error::RecvError::Closed) => return None,
            Ok(_) => {}
        }
    };
    match followup {
        Followup::ShipDescribe => {
            tracing::info!("session {session_id}: sending the ship describe follow-up");
            Some(FollowupTurn {
                prompt: pipeline::SHIP_DESCRIBE_PROMPT,
                collab_mode: None,
            })
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
                return None;
            }
            let observed = driver.observed_collab_mode();
            if observed.as_deref() != Some("plan") {
                tracing::warn!(
                    "session {session_id}: codex never confirmed plan mode for phase one \
                     (observed {observed:?}); its questions had no card to answer them"
                );
            }
            tracing::info!("session {session_id}: starting phase two");
            Some(FollowupTurn {
                prompt,
                collab_mode: Some(CollabMode::Default),
            })
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub async fn run_codex_followup_for_test(
    driver: &dyn AgentDriver,
    rx: &mut broadcast::Receiver<SessionEvent>,
    prompt: &'static str,
    op_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
) {
    let Some(turn) = await_followup("test", driver, rx, Followup::CodexExecute(prompt)).await
    else {
        return;
    };
    let _guard = op_lock.lock().await;
    driver.set_collab_mode(turn.collab_mode);
    if let Err(e) = driver.send_user_turn(turn.prompt, None, None, None) {
        tracing::warn!("phase two was not sent: {e}");
    }
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
