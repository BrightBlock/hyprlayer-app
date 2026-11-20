//! One connection's outbound frames, written by one task in priority order:
//! responses, then runtime events, then large responses and stream data taking
//! turns (ADR-0009). A client that reads part frames gets anything over
//! `PART_BYTES` a part at a time, so a small frame never waits behind all of it.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use hyprlayer_proto::{frame, Features};
use tokio::io::AsyncWrite;
use tokio::sync::{oneshot, Notify, OwnedSemaphorePermit};

use crate::transport::{encode_control, encode_event, Control, FrameWriter};

/// The most a small frame waits behind a large one once that one has started.
pub const PART_BYTES: usize = 16 * 1024;
/// Past either, a client that stopped reading misses events rather than queueing them.
const EVENT_QUEUE_BYTES: usize = 64 * 1024;
const EVENT_QUEUE_FRAMES: usize = 256;
/// Past this the connection stops reading requests until its responses drain.
const RESPONSE_QUEUE_BYTES: usize = 4 * 1024 * 1024;
const BULK_LANE: u8 = 0;
const DATA_LANE: u8 = 1;

/// What the writer releases once a frame's last byte is written.
enum Done {
    Nothing,
    Signal(oneshot::Sender<()>),
    /// A stream's one frame in flight.
    Slot(OwnedSemaphorePermit),
}

impl Done {
    fn finish(self) {
        match self {
            Done::Nothing => {}
            Done::Signal(tx) => {
                let _ = tx.send(());
            }
            Done::Slot(slot) => drop(slot),
        }
    }
}

struct Queued {
    bytes: Vec<u8>,
    sent: usize,
    done: Done,
}

impl Queued {
    fn new(bytes: Vec<u8>, done: Done) -> Self {
        Self {
            bytes,
            sent: 0,
            done,
        }
    }
}

enum EventItem {
    Frame(Vec<u8>),
    Missed { first_seq: u64, count: u64 },
}

struct Unit {
    bytes: Vec<u8>,
    done: Done,
}

#[derive(Default)]
struct Lanes {
    closed: bool,
    responses: VecDeque<Queued>,
    /// Responses over `PART_BYTES`.
    bulk: VecDeque<Queued>,
    events: VecDeque<EventItem>,
    data: VecDeque<Queued>,
    response_bytes: usize,
    event_bytes: usize,
    event_frames: usize,
    next_seq: u64,
    data_turn: bool,
}

impl Lanes {
    fn next_unit(&mut self, flow_control: bool) -> Option<Unit> {
        if let Some(queued) = self.responses.pop_front() {
            self.response_bytes -= queued.bytes.len();
            return Some(Unit {
                bytes: queued.bytes,
                done: queued.done,
            });
        }
        while let Some(item) = self.events.pop_front() {
            match item {
                EventItem::Frame(bytes) => {
                    self.event_bytes -= bytes.len();
                    self.event_frames -= 1;
                    return Some(Unit {
                        bytes,
                        done: Done::Nothing,
                    });
                }
                EventItem::Missed { first_seq, count } if flow_control => {
                    let bytes = encode_control(&Control::EventsMissed { first_seq, count })
                        .expect("a gap marker always serializes");
                    return Some(Unit {
                        bytes,
                        done: Done::Nothing,
                    });
                }
                EventItem::Missed { .. } => {}
            }
        }
        let order = if self.data_turn {
            [DATA_LANE, BULK_LANE]
        } else {
            [BULK_LANE, DATA_LANE]
        };
        for lane in order {
            if let Some(unit) = self.take(lane, flow_control) {
                self.data_turn = lane == BULK_LANE;
                return Some(unit);
            }
        }
        None
    }

    /// The next part of `lane`'s head frame, or all of it when it is small or the client reads no parts.
    fn take(&mut self, lane: u8, parts: bool) -> Option<Unit> {
        let queue = match lane {
            BULK_LANE => &mut self.bulk,
            _ => &mut self.data,
        };
        let head = queue.front_mut()?;
        let len = head.bytes.len();
        let unit = if parts && len > PART_BYTES {
            let end = (head.sent + PART_BYTES).min(len);
            let bytes = frame::encode_part(lane, end == len, &head.bytes[head.sent..end]);
            head.sent = end;
            if end < len {
                return Some(Unit {
                    bytes,
                    done: Done::Nothing,
                });
            }
            let queued = queue.pop_front()?;
            Unit {
                bytes,
                done: queued.done,
            }
        } else {
            let queued = queue.pop_front()?;
            Unit {
                bytes: queued.bytes,
                done: queued.done,
            }
        };
        if lane == BULK_LANE {
            self.response_bytes -= len;
        }
        Some(unit)
    }

    fn note_missed(&mut self, count: u64) {
        let first_seq = self.next_seq;
        self.next_seq += count;
        match self.events.back_mut() {
            Some(EventItem::Missed { count: missed, .. }) => *missed += count,
            _ => self
                .events
                .push_back(EventItem::Missed { first_seq, count }),
        }
    }
}

pub enum EventPush {
    Queued,
    Dropped,
    Closed,
}

struct Shared {
    features: Features,
    lanes: Mutex<Lanes>,
    /// Wakes the writer.
    wake: Notify,
    /// Wakes a request reader waiting for response room.
    room: Notify,
    gone: Notify,
}

#[derive(Clone)]
pub struct Outbound(Arc<Shared>);

impl Outbound {
    pub fn new(features: Features) -> Self {
        Self(Arc::new(Shared {
            features,
            lanes: Mutex::default(),
            wake: Notify::new(),
            room: Notify::new(),
            gone: Notify::new(),
        }))
    }

    /// Whether the client announced `FLOW_CONTROL`: parts, stream credit, gap markers.
    pub fn flow_control(&self) -> bool {
        self.0.features.flow_control
    }

    /// Whether the client announced `RAW_TERMINAL`.
    pub fn raw_terminal(&self) -> bool {
        self.0.features.raw_terminal
    }

    /// `false` once the connection is gone.
    pub fn push_response(&self, bytes: Vec<u8>) -> bool {
        self.push_response_with(bytes, Done::Nothing)
    }

    /// Resolves once the frame is written, so nothing queued later can reach the wire first.
    pub async fn push_response_written(&self, bytes: Vec<u8>) -> bool {
        let (tx, rx) = oneshot::channel();
        self.push_response_with(bytes, Done::Signal(tx)) && rx.await.is_ok()
    }

    fn push_response_with(&self, bytes: Vec<u8>, done: Done) -> bool {
        let mut lanes = self.lock();
        if lanes.closed {
            return false;
        }
        lanes.response_bytes += bytes.len();
        let large = bytes.len() > PART_BYTES;
        let queue = if large {
            &mut lanes.bulk
        } else {
            &mut lanes.responses
        };
        queue.push_back(Queued::new(bytes, done));
        drop(lanes);
        self.0.wake.notify_one();
        true
    }

    /// Waits while queued responses exceed `RESPONSE_QUEUE_BYTES`.
    pub async fn response_room(&self) {
        loop {
            let notified = self.0.room.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let lanes = self.lock();
                if lanes.closed || lanes.response_bytes <= RESPONSE_QUEUE_BYTES {
                    return;
                }
            }
            notified.await;
        }
    }

    /// Numbers the event for this connection; once a gap opens, later events
    /// join it until the writer reaches it. An empty queue takes any size, and
    /// `event_json` runs only when the queue may have room.
    pub fn push_event<'a>(&self, event_json: impl FnOnce() -> Option<&'a str>) -> EventPush {
        let mut lanes = self.lock();
        if lanes.closed {
            return EventPush::Closed;
        }
        let gap_open = matches!(lanes.events.back(), Some(EventItem::Missed { .. }));
        let busy = lanes.event_frames > 0;
        let json = (!gap_open && !(busy && lanes.event_frames >= EVENT_QUEUE_FRAMES))
            .then(event_json)
            .flatten()
            .filter(|json| !busy || lanes.event_bytes + json.len() <= EVENT_QUEUE_BYTES);
        let Some(json) = json else {
            lanes.note_missed(1);
            return EventPush::Dropped;
        };
        let bytes = encode_event(json, lanes.next_seq);
        lanes.next_seq += 1;
        lanes.event_bytes += bytes.len();
        lanes.event_frames += 1;
        lanes.events.push_back(EventItem::Frame(bytes));
        drop(lanes);
        self.0.wake.notify_one();
        EventPush::Queued
    }

    /// Events lost before they reached this connection's queue.
    pub fn missed(&self, count: u64) {
        let mut lanes = self.lock();
        if !lanes.closed {
            lanes.note_missed(count);
            drop(lanes);
            self.0.wake.notify_one();
        }
    }

    /// `slot` is the stream's one frame in flight; the writer releases it once the frame is written.
    pub fn push_data(&self, bytes: Vec<u8>, slot: OwnedSemaphorePermit) -> bool {
        let mut lanes = self.lock();
        if lanes.closed {
            return false;
        }
        lanes.data.push_back(Queued::new(bytes, Done::Slot(slot)));
        drop(lanes);
        self.0.wake.notify_one();
        true
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    pub async fn closed(&self) {
        loop {
            let notified = self.0.gone.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_closed() {
                return;
            }
            notified.await;
        }
    }

    /// Drops everything queued: its slots and write signals release.
    pub fn close(&self) {
        let dropped = {
            let mut lanes = self.lock();
            let dropped = std::mem::take(&mut *lanes);
            lanes.closed = true;
            dropped
        };
        drop(dropped);
        self.0.wake.notify_one();
        self.0.room.notify_waiters();
        self.0.gone.notify_waiters();
    }

    async fn next(&self) -> Option<Unit> {
        loop {
            let notified = self.0.wake.notified();
            {
                let mut lanes = self.lock();
                if lanes.closed {
                    return None;
                }
                let before = lanes.response_bytes;
                if let Some(unit) = lanes.next_unit(self.0.features.flow_control) {
                    if before > RESPONSE_QUEUE_BYTES && lanes.response_bytes <= RESPONSE_QUEUE_BYTES
                    {
                        self.0.room.notify_waiters();
                    }
                    return Some(unit);
                }
            }
            notified.await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, Lanes> {
        self.0.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The connection's one writer. A failed write closes the queue, so producers stop.
pub async fn write<W: AsyncWrite + Unpin>(
    outbound: Outbound,
    mut writer: FrameWriter<W>,
) -> io::Result<()> {
    while let Some(unit) = outbound.next().await {
        if let Err(e) = writer.write_bytes(&unit.bytes).await {
            outbound.close();
            return Err(e);
        }
        unit.done.finish();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use hyprlayer_models::runtime::ThoughtsIndexUpdateKind;
    use hyprlayer_models::RuntimeEvent;

    use super::*;

    fn index_update(ids: usize) -> String {
        serde_json::to_string(&RuntimeEvent::ThoughtsIndexUpdated {
            update: ThoughtsIndexUpdateKind::Updated,
            ids: (0..ids)
                .map(|i| format!("thoughts/shared/plans/doc-{i}.md"))
                .collect(),
        })
        .unwrap()
    }

    #[test]
    fn an_event_larger_than_the_queue_still_goes_when_the_queue_is_empty() {
        let out = Outbound::new(Features::ALL);
        let large = index_update(4000);
        assert!(large.len() > EVENT_QUEUE_BYTES);
        for _ in 0..3 {
            assert!(matches!(out.push_event(|| Some(&large)), EventPush::Queued));
            while out.lock().next_unit(true).is_some() {}
        }
    }
}
