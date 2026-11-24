use hyprlayer_models::{ClientRef, SessionStreamItem};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use crate::claude::SessionEvent;
use crate::store::SessionLogEntry;

pub(crate) const INGRESS_CAPACITY: usize = 2048;
pub(crate) const ATTACH_QUEUE_CAPACITY: usize = 512;

#[derive(Debug, Clone)]
pub(crate) struct SessionDelivery {
    pub epoch: u64,
    pub delivery_ordinal: u64,
    pub persisted_seq: Option<i64>,
    /// The persisted row's; with `persisted_seq` it is a client's resume cursor.
    pub created_at: Option<f64>,
    pub rev: u64,
    pub author: Option<ClientRef>,
    pub event: SessionEvent,
}

impl SessionDelivery {
    fn item(&self) -> SessionStreamItem {
        SessionStreamItem {
            seq: self.persisted_seq,
            created_at: self.created_at,
            rev: self.rev,
            author: self.author.clone(),
            event: self.event.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStreamError {
    Lagged {
        epoch: u64,
        last_persisted_seq: Option<i64>,
    },
}

pub struct SessionAttachment {
    pub replay: Vec<SessionLogEntry>,
    pub live: LiveReceiver,
    pub rev: u64,
    /// When the turn open after `replay` began; None when none is.
    pub turn_started_at: Option<f64>,
}

pub struct LiveReceiver(mpsc::Receiver<Result<SessionStreamItem, SessionStreamError>>);

impl LiveReceiver {
    /// Events only: most readers never need the log position.
    pub async fn recv(&mut self) -> Option<Result<SessionEvent, SessionStreamError>> {
        Some(self.0.recv().await?.map(|item| item.event))
    }

    pub async fn recv_item(&mut self) -> Option<Result<SessionStreamItem, SessionStreamError>> {
        self.0.recv().await
    }
}

enum Delivered {
    Ok,
    Full,
    Closed,
}

pub(crate) struct LiveAttachment {
    sender: mpsc::Sender<Result<SessionStreamItem, SessionStreamError>>,
    pending_partial: Option<SessionDelivery>,
    last_persisted_seq: Option<i64>,
    detached: bool,
    /// Only a client built from this branch sets this; a released desktop
    /// renders its own prompt optimistically and never dedupes an echo.
    pub(crate) echo_user_messages: bool,
}

impl LiveAttachment {
    pub(crate) fn new(echo_user_messages: bool) -> (Self, LiveReceiver) {
        Self::resuming_after(None, echo_user_messages)
    }

    /// Starts from the last seq the reader already holds, so lagging again
    /// before any persisted delivery resumes there instead of replaying the log.
    pub(crate) fn resuming_after(
        last_persisted_seq: Option<i64>,
        echo_user_messages: bool,
    ) -> (Self, LiveReceiver) {
        let (sender, receiver) = mpsc::channel(ATTACH_QUEUE_CAPACITY + 1);
        (
            Self {
                sender,
                pending_partial: None,
                last_persisted_seq,
                detached: false,
                echo_user_messages,
            },
            LiveReceiver(receiver),
        )
    }

    pub(crate) fn detached(&self) -> bool {
        self.detached || self.sender.is_closed()
    }

    // One slot of the channel's capacity is always kept free so a `Lagged`
    // signal can be enqueued the moment this attachment falls behind.
    fn try_deliver(&mut self, item: SessionStreamItem) -> Delivered {
        if self.sender.capacity() < 2 {
            return Delivered::Full;
        }
        match self.sender.try_send(Ok(item)) {
            Ok(()) => Delivered::Ok,
            Err(TrySendError::Full(_)) => Delivered::Full,
            Err(TrySendError::Closed(_)) => Delivered::Closed,
        }
    }

    fn mark_lagged(&mut self, epoch: u64) {
        if self.detached {
            return;
        }
        let _ = self.sender.try_send(Err(SessionStreamError::Lagged {
            epoch,
            last_persisted_seq: self.last_persisted_seq,
        }));
        self.detached = true;
    }

    pub(crate) fn deliver(&mut self, delivery: SessionDelivery) {
        if self.detached {
            return;
        }

        if let Some(mut pending) = self.pending_partial.take() {
            match self.try_deliver(pending.item()) {
                Delivered::Ok => {}
                Delivered::Closed => {
                    self.detached = true;
                    return;
                }
                // Partials are deltas: dropping one would cut words out of the block.
                Delivered::Full if is_partial(&delivery) => {
                    absorb(&mut pending, delivery);
                    self.pending_partial = Some(pending);
                    return;
                }
                Delivered::Full => {
                    self.mark_lagged(delivery.epoch);
                    return;
                }
            }
        }

        match self.try_deliver(delivery.item()) {
            Delivered::Ok => {
                if let Some(seq) = delivery.persisted_seq {
                    self.last_persisted_seq = Some(seq);
                }
            }
            Delivered::Closed => self.detached = true,
            Delivered::Full if is_partial(&delivery) => self.pending_partial = Some(delivery),
            Delivered::Full => self.mark_lagged(delivery.epoch),
        }
    }
}

fn is_partial(delivery: &SessionDelivery) -> bool {
    matches!(delivery.event, SessionEvent::PartialText { .. })
}

fn absorb(pending: &mut SessionDelivery, next: SessionDelivery) {
    let SessionEvent::PartialText {
        text: more,
        parent_id: next_parent,
    } = &next.event
    else {
        return;
    };
    match &mut pending.event {
        SessionEvent::PartialText { text, parent_id } if parent_id == next_parent => {
            text.push_str(more);
            pending.rev = next.rev;
            pending.author = next.author;
        }
        // Clients render only main-thread partials, so those win the single slot.
        SessionEvent::PartialText {
            parent_id: None, ..
        } => {}
        _ => *pending = next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpersisted_turn() -> SessionDelivery {
        SessionDelivery {
            epoch: 0,
            delivery_ordinal: 0,
            persisted_seq: None,
            created_at: None,
            rev: 0,
            author: None,
            event: SessionEvent::TurnDone {
                cost_usd: None,
                num_turns: None,
                context_tokens: None,
                turn_status: None,
                thread_tokens: None,
            },
        }
    }

    #[test]
    fn a_resumed_attachment_that_lags_before_any_persisted_delivery_keeps_its_resume_point() {
        let (mut attachment, mut rx) = LiveAttachment::resuming_after(Some(41), false);
        for _ in 0..=ATTACH_QUEUE_CAPACITY {
            attachment.deliver(unpersisted_turn());
        }
        let mut last = None;
        while let Ok(item) = rx.0.try_recv() {
            last = Some(item);
        }
        assert_eq!(
            last,
            Some(Err(SessionStreamError::Lagged {
                epoch: 0,
                last_persisted_seq: Some(41),
            }))
        );
    }
}
