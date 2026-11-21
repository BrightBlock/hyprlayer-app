//! The client half of the connection: correlates each request with the
//! response carrying its id and surfaces the server's unsolicited events. The
//! typed methods (`list_sessions`, …) are generated onto `RpcClient` by the
//! command table in `rpc.rs`.

use std::collections::{HashMap, HashSet};
use std::io;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_models::terminal::TerminalEvent;
use hyprlayer_models::RuntimeEvent;
use hyprlayer_proto::frame::{self, Frame};
use hyprlayer_proto::{check_version, Features, WireError, PROTOCOL_VERSION, STREAM_WINDOW};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot, watch};

use crate::logging::RepeatGate;
use crate::outbound::{self, Heartbeat, Outbound, ReadReports};
use crate::rpc::{Request, Response};
use crate::socket;
use crate::streams::{StreamRequest, StreamResponse};
use crate::transport::{
    encode_control, label, parse_control, undecodable_event, undecodable_id, Control, FrameReader,
    FrameWriter,
};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Response, WireError>>>>>;

/// A stream's raw frames plus the response value that opened it, handed to
/// the awaiting caller together — see `read_loop`'s `StreamResponse` arm for
/// why the receiver is registered before this is ever sent.
type StreamOpened = (StreamResponse, Option<StreamReceiver>);
type PendingStream = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<StreamOpened, WireError>>>>>;
type StreamSenders = Arc<Mutex<HashMap<u64, Inbox>>>;

/// Runtime events held for a consumer that is not keeping up; past this they count as missed.
const EVENT_QUEUE: usize = 1024;
/// Past this a stream whose server ignores credit is closed rather than grown.
const STREAM_QUEUE_LIMIT: usize = 64 * 1024 * 1024;

/// Where the read loop puts one stream's frames without waiting; credit keeps it near the server's window.
struct Inbox {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
}

/// Covers an SSH connect, a remote relay starting `serve`, and its first open.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
/// What this client announces: it renders raw output until it renders the server's frames.
const ANNOUNCED: Features = Features {
    screen_frames: false,
    ..Features::ALL
};
/// A call still unanswered this long is logged, then left to finish: some
/// (a workstream's Setup script, a first agent turn) legitimately take minutes.
const STILL_PENDING: Duration = Duration::from_secs(30);
/// How much of whatever arrived instead of a hello goes into the error.
const STRAY_BYTES_SHOWN: usize = 160;

static CALL_FAILURES: RepeatGate = RepeatGate::new(Duration::from_secs(60));

#[derive(Debug)]
pub enum ConnectError {
    Io(io::Error),
    /// The server answered the hello with a refusal (a version mismatch).
    Refused(WireError),
    Protocol(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Io(e) => write!(f, "{e}"),
            ConnectError::Refused(e) => write!(f, "{}", e.message),
            ConnectError::Protocol(detail) => write!(f, "protocol error: {detail}"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl From<io::Error> for ConnectError {
    fn from(e: io::Error) -> Self {
        ConnectError::Io(e)
    }
}

/// Keeps the writer running: the client holds one, and so does each stream returning credit.
/// Once the last is dropped, what they queued is still written before the writer ends.
struct Holder(Outbound);

impl std::ops::Deref for Holder {
    type Target = Outbound;
    fn deref(&self) -> &Outbound {
        &self.0
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        self.0.finish();
    }
}

pub struct RpcClient {
    next_id: AtomicU64,
    outbound: Arc<Holder>,
    pending: Pending,
    pending_streams: PendingStream,
    events: Mutex<Option<mpsc::Receiver<RuntimeEvent>>>,
    missed: Arc<watch::Sender<u64>>,
    server_version: u32,
    server_build: Option<String>,
    server_semver: Option<String>,
    server_features: Features,
}

impl RpcClient {
    /// Performs the hello exchange, then runs a reader and a writer task for
    /// the connection's lifetime.
    pub async fn connect<S>(stream: S) -> Result<Arc<Self>, ConnectError>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (rd, wr) = tokio::io::split(stream);
        let mut reader = FrameReader::new(rd);
        let mut writer = FrameWriter::new(wr);

        writer
            .write_control(&Control::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: None,
                version: None,
                features: ANNOUNCED.names(),
                stream_window: None,
            })
            .await?;
        let hello = match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.next_control()).await {
            Ok(Ok(hello)) => hello,
            // A remote shell's startup files can print to stdout ahead of the
            // relay; say what arrived, since the frame error alone cannot.
            Ok(Err(e)) if e.kind() == io::ErrorKind::InvalidData => {
                return Err(ConnectError::Protocol(format!("{e}{}", stray(&reader))))
            }
            Ok(Err(e)) => return Err(ConnectError::Io(e)),
            Err(_) => {
                return Err(ConnectError::Protocol(format!(
                    "no hello from the server within {}s{}",
                    HANDSHAKE_TIMEOUT.as_secs(),
                    stray(&reader)
                )))
            }
        };
        let (server_version, server_build, server_semver, server_features, credit_batch) =
            match hello {
                Some(Control::Hello {
                    protocol_version,
                    build,
                    version,
                    features,
                    stream_window,
                }) => {
                    check_version(protocol_version).map_err(ConnectError::Refused)?;
                    let features = Features::parse(&features);
                    // A quarter of whatever window the server keeps, so its window can change alone.
                    let credit_batch = features
                        .flow_control
                        .then(|| (stream_window.unwrap_or(STREAM_WINDOW) / 4).max(1));
                    (protocol_version, build, version, features, credit_batch)
                }
                Some(Control::Refused { error }) => return Err(ConnectError::Refused(error)),
                Some(other) => {
                    return Err(ConnectError::Protocol(format!(
                        "expected hello, got {}",
                        label(&other)
                    )))
                }
                None => return Err(ConnectError::Protocol("closed before hello".into())),
            };

        // The server's own lanes: control frames and small requests first, larger ones in parts.
        // Windowed by the server's reports, so an upload never fills the uplink ahead of ours.
        let outbound = Outbound::new(server_features, server_features.link_credit);
        let (events_tx, events_rx) = mpsc::channel(EVENT_QUEUE);
        let missed = Arc::new(watch::Sender::new(0));
        let pending: Pending = Arc::default();
        let pending_streams: PendingStream = Arc::default();

        let writer_task = tokio::spawn({
            let outbound = outbound.clone();
            async move {
                if let Err(e) = outbound::write(outbound, writer).await {
                    tracing::warn!("writing to the server failed: {e}");
                }
            }
        });
        let reports = server_features
            .link_credit
            .then(|| ReadReports::new(outbound.clone(), reader.consumed()));
        let heartbeat = server_features
            .heartbeat
            .then(|| Heartbeat::new(outbound.clone()));
        let holder = Arc::new(Holder(outbound.clone()));
        tokio::spawn(read_loop(
            reader,
            Loop {
                pending: pending.clone(),
                pending_streams: pending_streams.clone(),
                stream_senders: StreamSenders::default(),
                events: events_tx,
                missed: missed.clone(),
                next_seq: 0,
                outbound,
                holder: Arc::downgrade(&holder),
                credit_batch,
                reports,
                heartbeat,
            },
            writer_task.abort_handle(),
        ));

        Ok(Arc::new(Self {
            next_id: AtomicU64::new(1),
            outbound: holder,
            pending,
            pending_streams,
            events: Mutex::new(Some(events_rx)),
            missed,
            server_version,
            server_build,
            server_semver,
            server_features,
        }))
    }

    pub async fn connect_socket(path: &Path) -> Result<Arc<Self>, ConnectError> {
        Self::connect(socket::connect(path).await?).await
    }

    pub fn server_version(&self) -> u32 {
        self.server_version
    }

    pub fn server_build(&self) -> Option<&str> {
        self.server_build.as_deref()
    }

    /// The server's release version; `None` from servers up to 0.8.2.
    pub fn server_semver(&self) -> Option<&str> {
        self.server_semver.as_deref()
    }

    pub fn server_features(&self) -> Features {
        self.server_features
    }

    /// The server's runtime events, handed out once; the stream ends when the
    /// connection does.
    pub fn take_events(&self) -> Option<mpsc::Receiver<RuntimeEvent>> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// How many runtime events this connection lost, at the server or here; a
    /// change means re-read whatever they would have invalidated.
    pub fn events_missed(&self) -> watch::Receiver<u64> {
        self.missed.subscribe()
    }

    pub async fn call(&self, request: Request) -> Result<Response, WireError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.lock_pending().insert(id, tx);

        let command = request.command();
        let started = Instant::now();
        let line = serde_json::to_string(&Control::Request { id, body: request })
            .map_err(|e| WireError::protocol(format!("{command}: {e}")))?;
        if !self.outbound.push_frame(frame::encode_control(&line)) {
            self.lock_pending().remove(&id);
            return Err(WireError::disconnected());
        }
        let reply = await_reply(command, rx).await;
        if let Err(e) = &reply {
            note_failure(command, e, started.elapsed());
        }
        reply
    }

    /// Opens a stream: sends the request, and returns the response value
    /// together with the raw frame receiver for whatever stream id it names
    /// (`None` for a command like `terminal_attach` that can decline to open
    /// one). `read_loop` registers the receiver's sender half before this
    /// call ever sees the response, so no frame for the new stream can arrive
    /// before its consumer does.
    pub(crate) async fn call_stream(
        &self,
        request: StreamRequest,
    ) -> Result<StreamOpened, WireError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending_streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);

        let command = request.command();
        let started = Instant::now();
        let line = serde_json::to_string(&Control::StreamRequest { id, body: request })
            .map_err(|e| WireError::protocol(e.to_string()))?;
        if !self.outbound.push_frame(frame::encode_control(&line)) {
            self.pending_streams
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            return Err(WireError::disconnected());
        }
        let reply = await_reply(command, rx).await;
        match &reply {
            Ok((response, _)) => tracing::debug!(
                "{command} opened stream {:?} in {:?}",
                response.stream_id(),
                started.elapsed()
            ),
            Err(e) => note_failure(command, e, started.elapsed()),
        }
        reply
    }

    /// Tells the server to stop forwarding a stream this client no longer
    /// reads — mirrors dropping a local receiver, since nothing here can
    /// drop the server's side of the channel for it.
    pub async fn cancel_stream(&self, stream_id: u64) -> Result<(), WireError> {
        self.send_control(&Control::Cancel { stream_id })
    }

    /// Asks the server to stop. The server answers by closing the connection,
    /// so callers wait for the socket to disappear rather than for a reply.
    pub async fn request_shutdown(&self) -> Result<(), WireError> {
        self.send_control(&Control::Shutdown)
    }

    fn send_control(&self, control: &Control) -> Result<(), WireError> {
        let bytes = encode_control(control).map_err(|e| WireError::protocol(e.to_string()))?;
        if self.outbound.push_frame(bytes) {
            Ok(())
        } else {
            Err(WireError::disconnected())
        }
    }

    fn lock_pending(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<Response, WireError>>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A burst of gaps settles this long before a re-read.
const GAP_QUIET: Duration = Duration::from_millis(500);
/// Re-reads stay this far apart, so a flood's refetches cannot feed it more gaps.
const GAP_INTERVAL: Duration = Duration::from_secs(5);

/// When to re-read after missed events: once a burst settles, and never twice within `GAP_INTERVAL`.
pub struct GapPacer {
    missed: watch::Receiver<u64>,
    last: Option<tokio::time::Instant>,
}

impl GapPacer {
    pub fn new(missed: watch::Receiver<u64>) -> Self {
        Self { missed, last: None }
    }

    /// `false` once the connection is gone.
    pub async fn next(&mut self) -> bool {
        if self.missed.changed().await.is_err() {
            return false;
        }
        let first = tokio::time::Instant::now();
        let not_before = self.last.map_or(first, |last| last + GAP_INTERVAL);
        let deadline = not_before.max(first + GAP_INTERVAL);
        loop {
            let settle = (tokio::time::Instant::now() + GAP_QUIET).max(not_before);
            match tokio::time::timeout_at(settle.min(deadline), self.missed.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => return false,
                Err(_) => break,
            }
        }
        self.last = Some(tokio::time::Instant::now());
        true
    }
}

async fn await_reply<T>(
    command: &str,
    mut rx: oneshot::Receiver<Result<T, WireError>>,
) -> Result<T, WireError> {
    let started = Instant::now();
    match tokio::time::timeout(STILL_PENDING, &mut rx).await {
        Ok(reply) => reply.unwrap_or_else(|_| Err(WireError::disconnected())),
        Err(_) => {
            tracing::warn!("{command} is still waiting on the server after {STILL_PENDING:?}");
            let reply = rx.await.unwrap_or_else(|_| Err(WireError::disconnected()));
            tracing::info!("{command} answered after {:?}", started.elapsed());
            reply
        }
    }
}

/// Command name, code and message only: arguments carry keys, keystrokes and
/// prompts. A lost connection is logged once, by `read_loop`, not per call.
fn note_failure(command: &str, error: &WireError, elapsed: Duration) {
    if error.code == WireError::disconnected().code {
        return;
    }
    let Some(held) = CALL_FAILURES.admit(&format!("{command}:{}", error.code)) else {
        return;
    };
    let repeats = if held > 0 {
        format!(" ({held} more since the last report)")
    } else {
        String::new()
    };
    tracing::warn!(
        "{command} failed after {elapsed:?}: {}: {}{repeats}",
        error.code,
        hyprlayer_core::redact::url_credentials(&error.message)
    );
}

fn stray<R: AsyncRead + Unpin>(reader: &FrameReader<R>) -> String {
    let bytes = reader.unconsumed();
    if bytes.is_empty() {
        return String::new();
    }
    let shown = String::from_utf8_lossy(&bytes[..bytes.len().min(STRAY_BYTES_SHOWN)]);
    format!("; it sent {shown:?} instead")
}

/// Where the read loop routes what arrives. Nothing here waits on a consumer.
struct Loop {
    pending: Pending,
    pending_streams: PendingStream,
    stream_senders: StreamSenders,
    events: mpsc::Sender<RuntimeEvent>,
    missed: Arc<watch::Sender<u64>>,
    /// The seq the next event should carry; a later one means events were lost.
    next_seq: u64,
    outbound: Outbound,
    /// Weak, so dropping the `RpcClient` and its streams still ends the writer.
    holder: Weak<Holder>,
    /// Set when the server honors stream credit: how much is returned at once.
    credit_batch: Option<u64>,
    /// Set when the server keeps a link window.
    reports: Option<ReadReports>,
    /// Set when the server answers pings.
    heartbeat: Option<Heartbeat>,
}

impl Loop {
    fn event(&mut self, event: RuntimeEvent, seq: Option<u64>) {
        let mut lost = seq.map_or(0, |seq| self.skip_to(seq));
        if let Err(TrySendError::Full(_)) = self.events.try_send(event) {
            lost += 1;
        }
        self.note_missed(lost);
    }

    /// Advances past `seq`; returns how many events before it never arrived.
    fn skip_to(&mut self, seq: u64) -> u64 {
        let lost = seq.saturating_sub(self.next_seq);
        self.next_seq = self.next_seq.max(seq + 1);
        lost
    }

    fn events_missed(&mut self, first_seq: u64, count: u64) {
        let end = first_seq.saturating_add(count);
        let lost = end.saturating_sub(self.next_seq);
        self.next_seq = self.next_seq.max(end);
        self.note_missed(lost);
    }

    fn note_missed(&self, lost: u64) {
        if lost > 0 {
            self.missed.send_modify(|missed| *missed += lost);
        }
    }

    fn open_stream(&self, stream_id: u64) -> StreamReceiver {
        let (tx, rx) = mpsc::unbounded_channel();
        let queued = Arc::new(AtomicUsize::new(0));
        self.stream_senders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                stream_id,
                Inbox {
                    tx,
                    queued: queued.clone(),
                },
            );
        StreamReceiver {
            stream_id,
            rx,
            queued,
            credit: self.credit_batch.and(self.holder.upgrade()),
            batch: self.credit_batch.unwrap_or(u64::MAX),
            owed: 0,
        }
    }

    fn data(&self, stream_id: u64, event: &[u8]) {
        let mut senders = self
            .stream_senders
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(inbox) = senders.get(&stream_id) else {
            tracing::debug!("data frame for a closed stream {stream_id}");
            return;
        };
        let queued = inbox.queued.load(Ordering::Relaxed);
        if queued > 0 && queued + event.len() > STREAM_QUEUE_LIMIT {
            senders.remove(&stream_id);
            tracing::warn!(
                "closing stream {stream_id}: {queued} bytes unread and the server kept sending"
            );
            let cancel = Control::Cancel { stream_id };
            self.outbound
                .push_frame(encode_control(&cancel).expect("a cancel serializes"));
            return;
        }
        inbox.queued.fetch_add(event.len(), Ordering::Relaxed);
        if inbox.tx.send(event.to_vec()).is_err() {
            senders.remove(&stream_id);
            tracing::debug!("data frame for a closed stream {stream_id}");
        }
    }
}

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: FrameReader<R>,
    mut routes: Loop,
    writer: tokio::task::AbortHandle,
) {
    let pending = routes.pending.clone();
    let pending_streams = routes.pending_streams.clone();
    let stream_senders = routes.stream_senders.clone();
    let mut skipped = HashSet::new();
    let reason = loop {
        match outbound::next_frame(&mut reader, &mut routes.reports, &mut routes.heartbeat).await {
            Ok(Some(Frame::Control(line))) => match parse_control(&line) {
                Ok(Control::Response { id, outcome }) => {
                    let waiter = pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                    match waiter {
                        Some(tx) => {
                            let _ = tx.send(outcome.into());
                        }
                        None => tracing::warn!("response for unknown request {id}"),
                    }
                }
                Ok(Control::StreamResponse { id, outcome }) => {
                    let waiter = pending_streams
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                    let Some(tx) = waiter else {
                        tracing::warn!("stream response for unknown request {id}");
                        continue;
                    };
                    let result = match outcome {
                        hyprlayer_proto::Outcome::Ok(response) => {
                            // Registered here, before the waiting caller ever
                            // sees the response — see `call_stream`'s doc.
                            let rx = response
                                .stream_id()
                                .map(|stream_id| routes.open_stream(stream_id));
                            Ok((response, rx))
                        }
                        hyprlayer_proto::Outcome::Err(e) => Err(e),
                    };
                    let _ = tx.send(result);
                }
                Ok(Control::Event { event, seq }) => routes.event(event, seq),
                Ok(Control::EventsMissed { first_seq, count }) => {
                    routes.events_missed(first_seq, count)
                }
                Ok(Control::Received { bytes, read_us }) => routes
                    .outbound
                    .received(bytes, read_us.map(Duration::from_micros)),
                Ok(other) => tracing::warn!("unexpected {} frame from server", label(&other)),
                // Never fatal: an unreadable frame fails its own call at most,
                // so the connection and its sessions stay up.
                Err(e) => match undecodable_event(&line) {
                    Some((kind, seq)) => {
                        // Skipped on purpose, so not missed.
                        if let Some(seq) = seq {
                            routes.skip_to(seq);
                        }
                        if skipped.insert(kind.clone()) {
                            tracing::warn!("skipping {kind} events this build cannot decode: {e}");
                        }
                    }
                    None => match undecodable_id(&line) {
                        Some((kind, id)) if kind == "response" => {
                            tracing::warn!("failing request {id}: its response is unreadable: {e}");
                            let waiter = pending
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&id);
                            if let Some(tx) = waiter {
                                let _ = tx.send(Err(WireError::unreadable_response()));
                            }
                        }
                        Some((kind, id)) if kind == "stream_response" => {
                            tracing::warn!(
                                "failing stream request {id}: its response is unreadable: {e}"
                            );
                            let waiter = pending_streams
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&id);
                            if let Some(tx) = waiter {
                                let _ = tx.send(Err(WireError::unreadable_response()));
                            }
                        }
                        _ => tracing::warn!(
                            "skipped an unreadable control frame from the server: {e}"
                        ),
                    },
                },
            },
            Ok(Some(Frame::Data(payload))) => match frame::decode_stream_frame(&payload) {
                Some((stream_id, event)) => routes.data(stream_id, event),
                None => tracing::warn!("data frame too short to carry a stream id"),
            },
            Ok(None) => break "the server closed it".to_string(),
            Err(e) => break format!("reading failed ({e})"),
        }
    };
    // Close the outbound queue before draining: a call that has not sent
    // yet then fails to push, instead of queueing a request nothing will
    // ever answer and awaiting it forever.
    routes.outbound.close();
    writer.abort();
    let waiters: Vec<_> = pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain()
        .map(|(_, tx)| tx)
        .collect();
    let waiters_len = waiters.len();
    for tx in waiters {
        let _ = tx.send(Err(WireError::disconnected()));
    }
    let stream_waiters: Vec<_> = pending_streams
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain()
        .map(|(_, tx)| tx)
        .collect();
    let failed_calls = waiters_len;
    let failed_opens = stream_waiters.len();
    for tx in stream_waiters {
        let _ = tx.send(Err(WireError::disconnected()));
    }
    // Dropping every sender ends each open stream's `recv()` with `None`,
    // same as a local channel closing when its producer goes away.
    let mut senders = stream_senders
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let open_streams = senders.len();
    senders.clear();
    tracing::warn!(
        "server connection ended: {reason}; failed {failed_calls} pending call(s) and \
         {failed_opens} stream open(s), closed {open_streams} stream(s)"
    );
}

/// One stream's raw frames. Reading them returns credit to a server that waits for it.
pub struct StreamReceiver {
    stream_id: u64,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    /// The writer's queue, when the server honors credit.
    credit: Option<Arc<Holder>>,
    batch: u64,
    owed: u64,
}

impl StreamReceiver {
    /// Cancel-safe: credit goes out without awaiting, so a timed-out read loses no frame.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        let bytes = self.rx.recv().await?;
        self.queued.fetch_sub(bytes.len(), Ordering::Relaxed);
        if let Some(outbound) = &self.credit {
            self.owed += bytes.len() as u64;
            if self.owed >= self.batch {
                let credit = Control::Credit {
                    stream_id: self.stream_id,
                    bytes: std::mem::take(&mut self.owed),
                };
                outbound.push_frame(encode_control(&credit).expect("credit serializes"));
            }
        }
        Some(bytes)
    }
}

impl Drop for StreamReceiver {
    fn drop(&mut self) {
        // A server waiting on this stream's credit would otherwise hold it until the connection closes.
        if let Some(outbound) = &self.credit {
            let cancel = Control::Cancel {
                stream_id: self.stream_id,
            };
            outbound.push_frame(encode_control(&cancel).expect("a cancel serializes"));
        }
    }
}

/// A typed, JSON-decoding view onto one stream's raw frames.
pub struct EventStream<E> {
    rx: StreamReceiver,
    _marker: PhantomData<fn() -> E>,
}

impl<E: DeserializeOwned> EventStream<E> {
    pub(crate) fn new(rx: StreamReceiver) -> Self {
        Self {
            rx,
            _marker: PhantomData,
        }
    }

    /// `None` at a clean end of stream or an undecodable frame — the latter
    /// is a protocol bug, not a condition worth retrying past.
    pub async fn recv(&mut self) -> Option<E> {
        decode_json(&self.rx.recv().await?)
    }
}

fn decode_json<E: DeserializeOwned>(bytes: &[u8]) -> Option<E> {
    match serde_json::from_slice(bytes) {
        Ok(event) => Some(event),
        Err(e) => {
            tracing::error!(
                "{} stream event did not decode: {e}",
                std::any::type_name::<E>()
            );
            None
        }
    }
}

/// A terminal stream's events, whether the server sent raw output or base64 in JSON.
pub struct TerminalStream {
    rx: StreamReceiver,
}

impl TerminalStream {
    pub(crate) fn new(rx: StreamReceiver) -> Self {
        Self { rx }
    }

    /// `None` at a clean end of stream or an undecodable frame.
    pub async fn recv(&mut self) -> Option<TerminalEvent> {
        let bytes = self.rx.recv().await?;
        match frame::decode_terminal_output(&bytes) {
            Some((offset, output)) => Some(TerminalEvent::Output {
                data: B64.encode(output),
                offset: Some(offset),
            }),
            None => decode_json(&bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use hyprlayer_proto::{Outcome, FLOW_CONTROL};

    use super::*;
    use crate::transport::encode_control;

    const FUTURE_EVENT: &str = r#"{"event":{"event":{"sessionFromTheFuture":{"sessionId":"s1"}}}}"#;

    /// A peer that says hello, answers the first request after writing `lines`, then holds on.
    async fn scripted_client(lines: Vec<&'static str>) -> Arc<RpcClient> {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let (rd, wr) = tokio::io::split(theirs);
            let mut reader = FrameReader::new(rd);
            let mut writer = FrameWriter::new(wr);
            let _ = reader.next_control().await;
            let _ = writer
                .write_control(&Control::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    build: None,
                    version: None,
                    features: Vec::new(),
                    stream_window: None,
                })
                .await;
            let Ok(Some(Control::Request { id, .. })) = reader.next_control().await else {
                return;
            };
            for line in lines {
                let _ = writer.write_bytes(&frame::encode_control(line)).await;
            }
            let answer = Control::Response {
                id,
                outcome: Outcome::Ok(Response::ReorderProjects(())),
            };
            let _ = writer.write_bytes(&encode_control(&answer).unwrap()).await;
            while matches!(reader.next_frame().await, Ok(Some(_))) {}
        });
        RpcClient::connect(ours).await.expect("the peer says hello")
    }

    fn any_call() -> Request {
        Request::ReorderProjects { paths: Vec::new() }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn credit_follows_the_window_the_server_announced() {
        let window = 64 * 1024;
        let (ours, theirs) = tokio::io::duplex(1024 * 1024);
        let peer = tokio::spawn(async move {
            let (rd, wr) = tokio::io::split(theirs);
            let mut reader = FrameReader::new(rd);
            let mut writer = FrameWriter::new(wr);
            let _ = reader.next_control().await;
            let hello = format!(
                r#"{{"hello":{{"protocol_version":{PROTOCOL_VERSION},"features":["{FLOW_CONTROL}"],"stream_window":{window}}}}}"#
            );
            writer
                .write_bytes(&frame::encode_control(&hello))
                .await
                .unwrap();
            let Ok(Some(Control::StreamRequest { id, .. })) = reader.next_control().await else {
                return false;
            };
            let opened = Control::StreamResponse {
                id,
                outcome: Outcome::Ok(StreamResponse::AttachSession { stream_id: 9 }),
            };
            writer.write_control(&opened).await.unwrap();
            let event = format!("\"{}\"", "x".repeat(4094));
            for _ in 0..window / event.len() {
                let frame = frame::encode_stream_frame(9, event.as_bytes());
                writer.write_bytes(&frame).await.unwrap();
            }
            while let Ok(Some(control)) = reader.next_control().await {
                if matches!(control, Control::Credit { stream_id: 9, .. }) {
                    return true;
                }
            }
            false
        });
        let client = RpcClient::connect(ours).await.expect("hello");
        let (_, stream) = client
            .call_stream(StreamRequest::AttachSession {
                session_id: "s".into(),
                echo_user_messages: false,
                with_meta: false,
                all_partials: false,
            })
            .await
            .expect("the stream opens");
        let mut stream = stream.expect("a stream id");
        for _ in 0..window / 4096 {
            stream.recv().await.expect("a frame");
        }
        let credited = tokio::time::timeout(Duration::from_secs(5), peer).await;
        assert!(
            matches!(credited, Ok(Ok(true))),
            "a window smaller than the client's batch must not stall the stream"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_stream_is_cancelled_even_behind_a_full_queue() {
        let (ours, theirs) = tokio::io::duplex(4 * 1024);
        let (resume, resumed) = oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let (rd, wr) = tokio::io::split(theirs);
            let mut reader = FrameReader::new(rd);
            let mut writer = FrameWriter::new(wr);
            let _ = reader.next_control().await;
            let hello = Control::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: None,
                version: None,
                features: vec![FLOW_CONTROL.to_string()],
                stream_window: None,
            };
            writer.write_control(&hello).await.unwrap();
            let Ok(Some(Control::StreamRequest { id, .. })) = reader.next_control().await else {
                return false;
            };
            let opened = Control::StreamResponse {
                id,
                outcome: Outcome::Ok(StreamResponse::AttachSession { stream_id: 5 }),
            };
            writer.write_control(&opened).await.unwrap();
            let _ = resumed.await;
            while let Ok(Some(control)) = reader.next_control().await {
                if matches!(control, Control::Cancel { stream_id: 5 }) {
                    return true;
                }
            }
            false
        });
        let client = RpcClient::connect(ours).await.expect("hello");
        let (_, stream) = client
            .call_stream(StreamRequest::AttachSession {
                session_id: "s".into(),
                echo_user_messages: false,
                with_meta: false,
                all_partials: false,
            })
            .await
            .expect("the stream opens");
        // Far more than the stalled pipe takes.
        for id in 0..1500 {
            client.cancel_stream(1000 + id).await.expect("queued");
        }

        drop(stream);
        let _ = resume.send(());
        let cancelled = tokio::time::timeout(Duration::from_secs(10), peer).await;
        assert!(
            matches!(cancelled, Ok(Ok(true))),
            "the dropped stream's cancel must reach the server"
        );
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Every test reaching the skip's `warn!` holds one: a thread with no scoped
    /// subscriber can cache that callsite's interest as never for all threads.
    /// Current-thread runtime, so the spawned reader logs under this default.
    fn capture_logs() -> (Captured, tracing::subscriber::DefaultGuard) {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        (captured, tracing::subscriber::set_default(subscriber))
    }

    #[tokio::test]
    async fn an_unknown_event_kind_keeps_the_connection_and_its_pending_call() {
        let (_logs, _guard) = capture_logs();
        let client =
            scripted_client(vec![FUTURE_EVENT, r#"{"event":{"event":"configChanged"}}"#]).await;
        let mut events = client.take_events().unwrap();

        assert!(client.call(any_call()).await.is_ok());
        assert!(matches!(
            events.recv().await,
            Some(RuntimeEvent::ConfigChanged)
        ));
    }

    #[tokio::test]
    async fn a_skipped_event_kind_is_logged_once() {
        let (captured, _guard) = capture_logs();
        let client = scripted_client(vec![FUTURE_EVENT, FUTURE_EVENT]).await;

        assert!(client.call(any_call()).await.is_ok());
        let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        let lines = logs.lines().filter(|l| l.contains("sessionFromTheFuture"));
        assert_eq!(lines.count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn a_malformed_line_with_no_id_is_skipped_and_the_connection_keeps_serving() {
        let client = scripted_client(vec![r#"{"response":42}"#, "not json"]).await;

        client
            .call(any_call())
            .await
            .expect("the real answer still arrives after the unreadable lines");
    }
}
