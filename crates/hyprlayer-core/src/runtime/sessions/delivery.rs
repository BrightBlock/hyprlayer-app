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
    pub event: SessionEvent,
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
    pub live: mpsc::Receiver<Result<SessionEvent, SessionStreamError>>,
}

enum Delivered {
    Ok,
    Full,
    Closed,
}

pub(crate) struct LiveAttachment {
    sender: mpsc::Sender<Result<SessionEvent, SessionStreamError>>,
    pending_partial: Option<SessionDelivery>,
    last_persisted_seq: Option<i64>,
    detached: bool,
}

impl LiveAttachment {
    pub(crate) fn new() -> (
        Self,
        mpsc::Receiver<Result<SessionEvent, SessionStreamError>>,
    ) {
        let (sender, receiver) = mpsc::channel(ATTACH_QUEUE_CAPACITY + 1);
        (
            Self {
                sender,
                pending_partial: None,
                last_persisted_seq: None,
                detached: false,
            },
            receiver,
        )
    }

    pub(crate) fn detached(&self) -> bool {
        self.detached
    }

    // One slot of the channel's capacity is always kept free so a `Lagged`
    // signal can be enqueued the moment this attachment falls behind.
    fn try_deliver(&mut self, event: SessionEvent) -> Delivered {
        if self.sender.capacity() < 2 {
            return Delivered::Full;
        }
        match self.sender.try_send(Ok(event)) {
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

        if let Some(pending) = self.pending_partial.take() {
            if let Delivered::Ok = self.try_deliver(pending.event) {
                if let Some(seq) = pending.persisted_seq {
                    self.last_persisted_seq = Some(seq);
                }
            }
            // A pending partial that still can't be flushed is ephemeral and
            // about to be superseded either way, so it is dropped rather than
            // re-queued.
        }

        let is_partial = matches!(delivery.event, SessionEvent::PartialText { .. });
        match self.try_deliver(delivery.event.clone()) {
            Delivered::Ok => {
                if let Some(seq) = delivery.persisted_seq {
                    self.last_persisted_seq = Some(seq);
                }
            }
            Delivered::Closed => self.detached = true,
            Delivered::Full => {
                if is_partial {
                    self.pending_partial = Some(delivery);
                } else {
                    self.mark_lagged(delivery.epoch);
                }
            }
        }
    }
}
