//! One end's outbound frames, written by one task in priority order: small
//! frames, then runtime events, then large frames and stream data taking turns
//! (ADR-0009). Events lead the last two by at most `PART_BYTES` while they
//! wait. Within each of those two, every frame or stream gets up to
//! `PART_BYTES` a turn; a peer that reads part frames gets anything larger a
//! part at a time, each frame on a lane of its own.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hyprlayer_proto::frame::Frame;
use hyprlayer_proto::{frame, Features};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, watch, Notify, OwnedSemaphorePermit, Semaphore};

use crate::link::LinkWindow;
use crate::transport::{encode_control, encode_event, Body, Control, FrameReader, FrameWriter};

/// The most a small frame waits behind a large one once that one has started.
pub const PART_BYTES: usize = 16 * 1024;
/// Past either, a client that stopped reading misses events rather than queueing them.
const EVENT_QUEUE_BYTES: usize = 64 * 1024;
const EVENT_QUEUE_FRAMES: usize = 256;
/// Requests dispatched and not yet answered on the wire; they bound what a reader that
/// stopped reading holds. Bulk answers have their own, so no other call waits behind them.
const BULK_IN_FLIGHT: usize = 16;
const CALLS_IN_FLIGHT: usize = 64;
/// Also keeps a carrier NAT's mapping alive.
const PING_AFTER: Duration = Duration::from_secs(10);
/// Outlasts a cell handover and TCP's retransmit backoff across one.
const DEAD_AFTER: Duration = Duration::from_secs(30);

/// Bytes a sender may still add; may go negative by one frame.
pub(crate) struct Credit(watch::Sender<i64>);

impl Credit {
    pub(crate) fn new(bytes: i64) -> Self {
        Self(watch::Sender::new(bytes))
    }

    pub(crate) fn grant(&self, bytes: u64) {
        let bytes = i64::try_from(bytes).unwrap_or(i64::MAX);
        self.0
            .send_modify(|left| *left = left.saturating_add(bytes));
    }

    pub(crate) fn take(&self, bytes: usize) {
        self.0.send_modify(|left| *left -= bytes as i64);
    }

    pub(crate) async fn available(&self) {
        self.at_least(1).await;
    }

    pub(crate) async fn at_least(&self, bytes: i64) {
        let _ = self.0.subscribe().wait_for(|left| *left >= bytes).await;
    }
}

/// What the writer releases once a frame's last byte is written.
enum Done {
    Nothing,
    Signal(oneshot::Sender<()>),
    /// Gives a stream's queued bytes back.
    Grant(Arc<Credit>, u64),
}

impl Done {
    fn finish(self) {
        match self {
            Done::Nothing => {}
            Done::Signal(tx) => {
                let _ = tx.send(());
            }
            Done::Grant(queue, bytes) => queue.grant(bytes),
        }
    }
}

struct Queued {
    body: Body,
    sent: usize,
    /// Held from a split frame's first part to its last.
    lane: Option<u8>,
    done: Done,
}

impl Queued {
    fn new(body: Body, done: Done) -> Self {
        Self {
            body,
            sent: 0,
            lane: None,
            done,
        }
    }
}

/// What an event says about one session, so a newer revision can take a queued one's place.
#[derive(Debug, PartialEq, Eq)]
pub struct EventScope {
    pub session: String,
    /// Author and whether it has turns, so a turn's end or a released send never folds into a commit.
    pub revision: Option<(Option<String>, bool)>,
}

struct QueuedEvent {
    bytes: Vec<u8>,
    seq: u64,
    scope: Option<Arc<EventScope>>,
}

enum EventItem {
    Frame(QueuedEvent),
    Missed { first_seq: u64, count: u64 },
}

struct Unit {
    bytes: Vec<u8>,
    done: Done,
}

/// A stream's frames, in order, or one frame of its own.
struct Flow {
    stream: Option<u64>,
    frames: VecDeque<Queued>,
}

/// Flows taking turns of up to `PART_BYTES`: deficit round-robin over parts.
#[derive(Default)]
struct Fair {
    flows: VecDeque<Flow>,
    /// Bytes the front flow has sent this turn.
    turn: usize,
}

impl Fair {
    fn push(&mut self, stream: Option<u64>, queued: Queued) {
        let flow = stream.and_then(|id| self.flows.iter_mut().find(|f| f.stream == Some(id)));
        match flow {
            Some(flow) => flow.frames.push_back(queued),
            None => self.flows.push_back(Flow {
                stream,
                frames: VecDeque::from([queued]),
            }),
        }
    }

    /// A whole frame, or with `parts` the next part of one over `PART_BYTES`.
    fn next(&mut self, parts: bool, lanes: &mut Vec<u8>) -> Option<Unit> {
        let mut tries = self.flows.len();
        while tries > 0 {
            let flow = self.flows.front_mut()?;
            let head = flow.frames.front_mut().expect("a flow holds a frame");
            let len = head.body.len();
            let split = parts && len > PART_BYTES;
            let size = if split {
                (len - head.sent).min(PART_BYTES)
            } else {
                len
            };
            // The next flow starts a fresh turn, so this never spends a try.
            if self.turn > 0 && self.turn + size > PART_BYTES {
                self.rotate();
                continue;
            }
            let lane = match (split, head.lane) {
                (false, _) => None,
                (true, Some(lane)) => Some(lane),
                (true, None) => match lanes.pop() {
                    Some(lane) => Some(lane),
                    // Every lane is mid-frame; this frame starts once one ends.
                    None => {
                        self.rotate();
                        tries -= 1;
                        continue;
                    }
                },
            };
            self.turn += size;
            let unit = match lane {
                Some(lane) => {
                    head.lane = Some(lane);
                    let end = head.sent + size;
                    let last = end == len;
                    let mut bytes = Vec::with_capacity(frame::PART_HEADER + size);
                    bytes.extend_from_slice(&frame::part_header(lane, last, size));
                    head.body.extend(head.sent..end, &mut bytes);
                    head.sent = end;
                    if !last {
                        return Some(Unit {
                            bytes,
                            done: Done::Nothing,
                        });
                    }
                    lanes.push(lane);
                    let queued = flow.frames.pop_front().expect("the head");
                    Unit {
                        bytes,
                        done: queued.done,
                    }
                }
                None => {
                    let queued = flow.frames.pop_front().expect("the head");
                    Unit {
                        bytes: queued.body.into_bytes(),
                        done: queued.done,
                    }
                }
            };
            if flow.frames.is_empty() {
                self.flows.pop_front();
                self.turn = 0;
            }
            return Some(unit);
        }
        None
    }

    fn rotate(&mut self) {
        self.flows.rotate_left(1);
        self.turn = 0;
    }
}

struct Lanes {
    closed: bool,
    /// Takes nothing more, and closes once what it holds is written.
    ending: bool,
    /// Reports and heartbeats, first and uncounted: a counted report would itself be reported, without end.
    reports: VecDeque<Vec<u8>>,
    /// Frames up to `PART_BYTES` that are not events: responses, or a client's requests and credit.
    small: VecDeque<Queued>,
    /// Larger ones.
    bulk: Fair,
    events: VecDeque<EventItem>,
    data: Fair,
    /// Lane ids no split frame holds.
    free_lanes: Vec<u8>,
    /// For a peer that reports what it read.
    link: Option<LinkWindow>,
    event_bytes: usize,
    event_frames: usize,
    next_seq: u64,
    data_turn: bool,
    /// Event bytes written since large frames or stream data last went, while they waited.
    events_ahead: usize,
}

impl Default for Lanes {
    fn default() -> Self {
        Self {
            closed: false,
            ending: false,
            reports: VecDeque::new(),
            small: VecDeque::new(),
            bulk: Fair::default(),
            events: VecDeque::new(),
            data: Fair::default(),
            free_lanes: (0..=u8::MAX).rev().collect(),
            link: None,
            event_bytes: 0,
            event_frames: 0,
            next_seq: 0,
            data_turn: false,
            events_ahead: 0,
        }
    }
}

impl Lanes {
    /// `room`: the link window has room for large frames and stream data; `spare`: for small
    /// frames and events, which may run past it.
    fn next_unit(&mut self, flow_control: bool, room: bool, spare: bool) -> Option<Unit> {
        if !spare {
            return None;
        }
        if let Some(queued) = self.small.pop_front() {
            return Some(Unit {
                bytes: queued.body.into_bytes(),
                done: queued.done,
            });
        }
        let waiting = !self.bulk.flows.is_empty() || !self.data.flows.is_empty();
        if !waiting || self.events_ahead < PART_BYTES {
            if let Some(unit) = self.next_event(flow_control) {
                if waiting {
                    self.events_ahead += unit.bytes.len();
                }
                return Some(unit);
            }
        }
        if !room {
            return None;
        }
        for data in [self.data_turn, !self.data_turn] {
            let fair = if data { &mut self.data } else { &mut self.bulk };
            if let Some(unit) = fair.next(flow_control, &mut self.free_lanes) {
                self.data_turn = !data;
                self.events_ahead = self.events_ahead.saturating_sub(unit.bytes.len());
                return Some(unit);
            }
        }
        // Every lane is mid-frame, so nothing waiting can go in the events' place.
        self.next_event(flow_control)
    }

    fn next_event(&mut self, flow_control: bool) -> Option<Unit> {
        while let Some(item) = self.events.pop_front() {
            match item {
                EventItem::Frame(event) => {
                    self.event_bytes -= event.bytes.len();
                    self.event_frames -= 1;
                    return Some(Unit {
                        bytes: event.bytes,
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
        None
    }

    /// The queued event a revision in `scope` takes the place of: its session's latest, not past a gap.
    fn superseded(&self, scope: &EventScope) -> Option<usize> {
        scope.revision.as_ref()?;
        for (at, item) in self.events.iter().enumerate().rev() {
            match item {
                EventItem::Missed { .. } => return None,
                EventItem::Frame(QueuedEvent {
                    scope: Some(queued),
                    ..
                }) if queued.session == scope.session => return (**queued == *scope).then_some(at),
                EventItem::Frame(_) => {}
            }
        }
        None
    }

    fn has_queued(&self) -> bool {
        !self.small.is_empty()
            || !self.events.is_empty()
            || !self.bulk.flows.is_empty()
            || !self.data.flows.is_empty()
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
    /// Queued, or in the place of a revision it supersedes.
    Queued,
    Dropped,
    Closed,
}

struct Shared {
    features: Features,
    lanes: Mutex<Lanes>,
    bulk: Arc<Semaphore>,
    calls: Arc<Semaphore>,
    /// Wakes the writer.
    wake: Notify,
    gone: Notify,
    reported: Notify,
}

#[derive(Clone)]
pub struct Outbound(Arc<Shared>);

impl Outbound {
    /// `features` are what the peer reads; `windowed` when it reports what it read.
    pub fn new(features: Features, windowed: bool) -> Self {
        let lanes = Lanes {
            link: windowed.then(LinkWindow::new),
            ..Lanes::default()
        };
        Self(Arc::new(Shared {
            features,
            lanes: Mutex::new(lanes),
            bulk: Arc::new(Semaphore::new(BULK_IN_FLIGHT)),
            calls: Arc::new(Semaphore::new(CALLS_IN_FLIGHT)),
            wake: Notify::new(),
            gone: Notify::new(),
            reported: Notify::new(),
        }))
    }

    /// Whether the peer announced `FLOW_CONTROL`: parts, stream credit, gap markers.
    pub fn flow_control(&self) -> bool {
        self.0.features.flow_control
    }

    /// Whether the peer announced `LINK_CREDIT`: it reports what it reads and windows its writes.
    pub fn link_credit(&self) -> bool {
        self.0.features.link_credit
    }

    /// Whether the peer announced `RAW_TERMINAL`.
    pub fn raw_terminal(&self) -> bool {
        self.0.features.raw_terminal
    }

    /// Whether the peer announced `TERMINAL_SIZE`.
    pub fn terminal_size(&self) -> bool {
        self.0.features.terminal_size
    }

    /// Whether the peer announced `HEARTBEAT`: it answers pings, so its silence means something.
    pub fn heartbeat(&self) -> bool {
        self.0.features.heartbeat
    }

    pub fn screen_frames(&self) -> bool {
        self.0.features.screen_frames
    }

    /// A request's turn to run, from the bulk pool or the other; hold it until its answer is
    /// written. `None` once the connection is gone: what it sent still runs, since a launch or
    /// a failed start's record may hang on it, and only the answer is lost.
    pub async fn request_slot(&self, bulk: bool) -> Option<OwnedSemaphorePermit> {
        let pool = if bulk { &self.0.bulk } else { &self.0.calls };
        pool.clone().acquire_owned().await.ok()
    }

    /// A control frame that is not an event. `false` once the connection is gone.
    pub fn push_frame(&self, bytes: Vec<u8>) -> bool {
        self.push_frame_with(bytes.into(), Done::Nothing)
    }

    /// Resolves once the frame is written, so nothing queued later can reach the wire first.
    pub async fn push_frame_written(&self, body: impl Into<Body>) -> bool {
        let (tx, rx) = oneshot::channel();
        self.push_frame_with(body.into(), Done::Signal(tx)) && rx.await.is_ok()
    }

    fn push_frame_with(&self, body: Body, done: Done) -> bool {
        let mut lanes = self.lock();
        if lanes.closed || lanes.ending {
            return false;
        }
        let queued = Queued::new(body, done);
        if queued.body.len() > PART_BYTES {
            lanes.bulk.push(None, queued);
        } else {
            lanes.small.push_back(queued);
        }
        drop(lanes);
        self.0.wake.notify_one();
        true
    }

    /// Numbers the event for this connection; once a gap opens, later events
    /// join it until the writer reaches it. An empty queue takes any size, and
    /// `event_json` runs only when the queue may have room. A superseding revision takes its place and seq.
    pub fn push_event<'a>(
        &self,
        scope: Option<&Arc<EventScope>>,
        event_json: impl FnOnce() -> Option<&'a str>,
    ) -> EventPush {
        let mut lanes = self.lock();
        if lanes.closed {
            return EventPush::Closed;
        }
        let superseded = scope.and_then(|scope| lanes.superseded(scope));
        let gap_open = matches!(lanes.events.back(), Some(EventItem::Missed { .. }));
        let busy = lanes.event_frames > 0;
        let room = superseded.is_some()
            || (!gap_open && !(busy && lanes.event_frames >= EVENT_QUEUE_FRAMES));
        let json = room.then(event_json).flatten().filter(|json| {
            superseded.is_some() || !busy || lanes.event_bytes + json.len() <= EVENT_QUEUE_BYTES
        });
        let Some(json) = json else {
            lanes.note_missed(1);
            return EventPush::Dropped;
        };
        if let Some(at) = superseded {
            let lanes = &mut *lanes;
            if let Some(EventItem::Frame(queued)) = lanes.events.get_mut(at) {
                let bytes = encode_event(json, queued.seq);
                lanes.event_bytes = lanes.event_bytes - queued.bytes.len() + bytes.len();
                queued.bytes = bytes;
            }
            return EventPush::Queued;
        }
        let seq = lanes.next_seq;
        let bytes = encode_event(json, seq);
        lanes.next_seq += 1;
        lanes.event_bytes += bytes.len();
        lanes.event_frames += 1;
        lanes.events.push_back(EventItem::Frame(QueuedEvent {
            bytes,
            seq,
            scope: scope.cloned(),
        }));
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

    /// `queue` gets the frame's bytes back once the writer has written it.
    pub(crate) fn push_data(&self, stream_id: u64, bytes: Vec<u8>, queue: Arc<Credit>) -> bool {
        let mut lanes = self.lock();
        if lanes.closed || lanes.ending {
            return false;
        }
        let len = bytes.len() as u64;
        lanes.data.push(
            Some(stream_id),
            Queued::new(bytes.into(), Done::Grant(queue, len)),
        );
        drop(lanes);
        self.0.wake.notify_one();
        true
    }

    fn push_report(&self, bytes: Vec<u8>) {
        let mut lanes = self.lock();
        if !lanes.closed && !lanes.ending {
            lanes.reports.push_back(bytes);
            drop(lanes);
            self.0.wake.notify_one();
        }
    }

    /// The peer read `bytes` more of what was written, `read` after its hello by its clock.
    pub fn received(&self, bytes: u64, read: Option<Duration>) {
        if let Some(link) = &mut self.lock().link {
            link.received(bytes, read, Instant::now());
        }
        self.0.wake.notify_one();
        self.0.reported.notify_waiters();
    }

    pub(crate) fn link_written(&self) -> Option<u64> {
        self.lock().link.as_ref().map(|link| link.written())
    }

    pub(crate) async fn read_through(&self, offset: u64) {
        loop {
            let notified = self.0.reported.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let lanes = self.lock();
                if lanes.closed || lanes.link.as_ref().is_none_or(|l| l.acked() >= offset) {
                    return;
                }
            }
            notified.await;
        }
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

    /// Takes no more frames; the writer ends once it has written the ones queued.
    pub fn finish(&self) {
        self.lock().ending = true;
        self.0.wake.notify_one();
    }

    /// Drops everything queued and every request still waiting for its turn.
    pub fn close(&self) {
        let dropped = {
            let mut lanes = self.lock();
            let dropped = std::mem::take(&mut *lanes);
            lanes.closed = true;
            dropped
        };
        drop(dropped);
        self.0.bulk.close();
        self.0.calls.close();
        self.0.wake.notify_one();
        self.0.gone.notify_waiters();
        self.0.reported.notify_waiters();
    }

    async fn next(&self) -> Option<Unit> {
        loop {
            let notified = self.0.wake.notified();
            {
                let mut lanes = self.lock();
                if lanes.closed {
                    return None;
                }
                if let Some(bytes) = lanes.reports.pop_front() {
                    return Some(Unit {
                        bytes,
                        done: Done::Nothing,
                    });
                }
                let room = lanes.link.as_ref().is_none_or(LinkWindow::has_room);
                let spare = lanes.link.as_ref().is_none_or(LinkWindow::has_spare);
                if let Some(unit) = lanes.next_unit(self.0.features.flow_control, room, spare) {
                    let waiting = lanes.has_queued();
                    if let Some(link) = &mut lanes.link {
                        link.wrote(unit.bytes.len(), waiting, Instant::now());
                    }
                    return Some(unit);
                }
                if room && lanes.ending {
                    drop(lanes);
                    self.close();
                    return None;
                }
            }
            notified.await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, Lanes> {
        self.0.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What this end has read and not yet told a peer that keeps a link window.
pub(crate) struct ReadReports {
    outbound: Outbound,
    told: u64,
    since: Instant,
}

impl ReadReports {
    /// `consumed`: what the reader had taken by its hello.
    pub(crate) fn new(outbound: Outbound, consumed: u64) -> Self {
        Self {
            outbound,
            told: consumed,
            since: Instant::now(),
        }
    }

    fn report(&mut self, consumed: u64) {
        let bytes = consumed - self.told;
        if bytes > 0 {
            self.told = consumed;
            let read_us = Some(self.since.elapsed().as_micros() as u64);
            let received =
                encode_control(&Control::Received { bytes, read_us }).expect("received serializes");
            self.outbound.push_report(received);
        }
    }
}

const PING: &str = r#""ping""#;
const PONG: &str = r#""pong""#;

/// Neither end counts these, so they are never reported back.
fn is_uncounted(line: &str) -> bool {
    line.starts_with(r#"{"received":"#) || line == PING || line == PONG
}

/// When a `HEARTBEAT` peer last sent anything: quiet for `PING_AFTER` it is pinged, for `DEAD_AFTER` given up.
pub(crate) struct Heartbeat {
    outbound: Outbound,
    heard: Instant,
    pinged: bool,
}

impl Heartbeat {
    pub(crate) fn new(outbound: Outbound) -> Self {
        Self {
            outbound,
            heard: Instant::now(),
            pinged: false,
        }
    }

    fn send(&self, control: &Control) {
        self.outbound
            .push_report(encode_control(control).expect("a heartbeat serializes"));
    }

    /// `FrameReader::fill`, which is cancel-safe, bounded by the peer's silence.
    async fn fill<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut FrameReader<R>,
    ) -> io::Result<bool> {
        loop {
            let quiet = if self.pinged { DEAD_AFTER } else { PING_AFTER };
            match tokio::time::timeout_at((self.heard + quiet).into(), reader.fill()).await {
                Ok(filled) => {
                    self.heard = Instant::now();
                    self.pinged = false;
                    return filled;
                }
                Err(_) if self.pinged => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("the peer sent nothing for {}s", DEAD_AFTER.as_secs()),
                    ))
                }
                Err(_) => {
                    self.pinged = true;
                    self.send(&Control::Ping);
                }
            }
        }
    }
}

/// The next frame already read, else reads more. Reports what was read whenever nothing is
/// left to decode, so a small frame times a bare round trip, and before a large frame, so its
/// parse never reads as a queue on the link. Answers pings and returns no heartbeat.
pub(crate) async fn next_frame<R: AsyncRead + Unpin>(
    reader: &mut FrameReader<R>,
    reports: &mut Option<ReadReports>,
    heartbeat: &mut Option<Heartbeat>,
) -> io::Result<Option<Frame>> {
    loop {
        if let Some(frame) = reader.decoded()? {
            let len = match &frame {
                Frame::Control(line) => line.len(),
                Frame::Data(payload) => payload.len(),
            };
            if let Some(reports) = reports {
                match &frame {
                    Frame::Control(line) if is_uncounted(line) => reports.told += len as u64 + 2,
                    _ if len > PART_BYTES => reports.report(reader.consumed()),
                    _ => {}
                }
            }
            if let (Some(heartbeat), Frame::Control(line)) = (heartbeat.as_ref(), &frame) {
                match line.as_str() {
                    PING => {
                        heartbeat.send(&Control::Pong);
                        continue;
                    }
                    PONG => continue,
                    _ => {}
                }
            }
            return Ok(Some(frame));
        }
        if let Some(reports) = reports {
            reports.report(reader.consumed());
        }
        let filled = match heartbeat {
            Some(heartbeat) => heartbeat.fill(reader).await?,
            None => reader.fill().await?,
        };
        if !filled {
            return Ok(None);
        }
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
        let out = Outbound::new(Features::ALL, false);
        let large = index_update(4000);
        assert!(large.len() > EVENT_QUEUE_BYTES);
        for _ in 0..3 {
            assert!(matches!(
                out.push_event(None, || Some(&large)),
                EventPush::Queued
            ));
            while out.lock().next_unit(true, true, true).is_some() {}
        }
    }
}
