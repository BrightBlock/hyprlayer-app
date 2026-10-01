//! The client half of the connection: correlates each request with the
//! response carrying its id and surfaces the server's unsolicited events. The
//! typed methods (`list_sessions`, …) are generated onto `RpcClient` by the
//! command table in `rpc.rs`.

use std::collections::{HashMap, HashSet};
use std::io;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use hyprlayer_models::RuntimeEvent;
use hyprlayer_proto::frame::{self, Frame};
use hyprlayer_proto::{check_version, WireError, PROTOCOL_VERSION};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::logging::RepeatGate;
use crate::rpc::{Request, Response};
use crate::socket;
use crate::streams::{StreamRequest, StreamResponse};
use crate::transport::{
    label, parse_control, undecodable_event, Control, FrameReader, FrameWriter,
    OUTBOUND_QUEUE_FRAMES,
};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Response, WireError>>>>>;

/// A stream's raw frames plus the response value that opened it, handed to
/// the awaiting caller together — see `read_loop`'s `StreamResponse` arm for
/// why the receiver is registered before this is ever sent.
type StreamOpened = (StreamResponse, Option<mpsc::UnboundedReceiver<Vec<u8>>>);
type PendingStream = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<StreamOpened, WireError>>>>>;
type StreamSenders = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>;

/// Covers an SSH connect, a remote relay starting `serve`, and its first open.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
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

pub struct RpcClient {
    next_id: AtomicU64,
    outbound: mpsc::Sender<Vec<u8>>,
    pending: Pending,
    pending_streams: PendingStream,
    events: Mutex<Option<mpsc::UnboundedReceiver<RuntimeEvent>>>,
    server_version: u32,
    server_build: Option<String>,
    server_semver: Option<String>,
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
        let (server_version, server_build, server_semver) = match hello {
            Some(Control::Hello {
                protocol_version,
                build,
                version,
            }) => {
                check_version(protocol_version).map_err(ConnectError::Refused)?;
                (protocol_version, build, version)
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

        let (outbound, mut rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE_FRAMES);
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let pending: Pending = Arc::default();
        let pending_streams: PendingStream = Arc::default();
        let stream_senders: StreamSenders = Arc::default();

        let writer_task = tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if let Err(e) = writer.write_bytes(&bytes).await {
                    tracing::warn!("writing to the server failed: {e}");
                    break;
                }
            }
        });
        tokio::spawn(read_loop(
            reader,
            pending.clone(),
            pending_streams.clone(),
            stream_senders,
            events_tx,
            writer_task.abort_handle(),
        ));

        Ok(Arc::new(Self {
            next_id: AtomicU64::new(1),
            outbound,
            pending,
            pending_streams,
            events: Mutex::new(Some(events_rx)),
            server_version,
            server_build,
            server_semver,
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

    /// The server's runtime events, handed out once; the stream ends when the
    /// connection does.
    pub fn take_events(&self) -> Option<mpsc::UnboundedReceiver<RuntimeEvent>> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    pub async fn call(&self, request: Request) -> Result<Response, WireError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.lock_pending().insert(id, tx);

        let command = request.command();
        let started = Instant::now();
        let line = serde_json::to_string(&Control::Request { id, body: request })
            .map_err(|e| WireError::protocol(format!("{command}: {e}")))?;
        if self
            .outbound
            .send(hyprlayer_proto::frame::encode_control(&line))
            .await
            .is_err()
        {
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
        if self
            .outbound
            .send(hyprlayer_proto::frame::encode_control(&line))
            .await
            .is_err()
        {
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
        let line = serde_json::to_string(&Control::Cancel { stream_id })
            .map_err(|e| WireError::protocol(e.to_string()))?;
        self.outbound
            .send(hyprlayer_proto::frame::encode_control(&line))
            .await
            .map_err(|_| WireError::disconnected())
    }

    /// Asks the server to stop. The server answers by closing the connection,
    /// so callers wait for the socket to disappear rather than for a reply.
    pub async fn request_shutdown(&self) -> Result<(), WireError> {
        let line = serde_json::to_string(&Control::Shutdown)
            .map_err(|e| WireError::protocol(e.to_string()))?;
        self.outbound
            .send(hyprlayer_proto::frame::encode_control(&line))
            .await
            .map_err(|_| WireError::disconnected())
    }

    fn lock_pending(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<Response, WireError>>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
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

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: FrameReader<R>,
    pending: Pending,
    pending_streams: PendingStream,
    stream_senders: StreamSenders,
    events: mpsc::UnboundedSender<RuntimeEvent>,
    writer: tokio::task::AbortHandle,
) {
    let mut skipped = HashSet::new();
    let reason = loop {
        match reader.next_frame().await {
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
                            let rx = response.stream_id().map(|stream_id| {
                                let (sender, receiver) = mpsc::unbounded_channel();
                                stream_senders
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .insert(stream_id, sender);
                                receiver
                            });
                            Ok((response, rx))
                        }
                        hyprlayer_proto::Outcome::Err(e) => Err(e),
                    };
                    let _ = tx.send(result);
                }
                Ok(Control::Event { event }) => {
                    let _ = events.send(event);
                }
                Ok(other) => tracing::warn!("unexpected {} frame from server", label(&other)),
                Err(e) => match undecodable_event(&line) {
                    // A newer server's event kind; the rest of the connection is still good.
                    Some(kind) => {
                        if skipped.insert(kind.clone()) {
                            tracing::warn!("skipping {kind} events this build cannot decode: {e}");
                        }
                    }
                    None => break format!("an unreadable control frame ({e})"),
                },
            },
            Ok(Some(Frame::Data(payload))) => match frame::decode_stream_frame(&payload) {
                Some((stream_id, event)) => {
                    let sender = stream_senders
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&stream_id)
                        .cloned();
                    match sender {
                        Some(sender) if sender.send(event.to_vec()).is_ok() => {}
                        _ => {
                            stream_senders
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&stream_id);
                            tracing::debug!("data frame for a closed stream {stream_id}");
                        }
                    }
                }
                None => tracing::warn!("data frame too short to carry a stream id"),
            },
            Ok(None) => break "the server closed it".to_string(),
            Err(e) => break format!("reading failed ({e})"),
        }
    };
    // Close the outbound channel before draining: a call that has not sent
    // yet then fails at `send`, instead of queueing a request nothing will
    // ever answer and awaiting it forever.
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

/// A typed, JSON-decoding view onto one stream's raw frames.
pub struct EventStream<E> {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    _marker: PhantomData<fn() -> E>,
}

impl<E: DeserializeOwned> EventStream<E> {
    pub(crate) fn new(rx: mpsc::UnboundedReceiver<Vec<u8>>) -> Self {
        Self {
            rx,
            _marker: PhantomData,
        }
    }

    /// `None` at a clean end of stream or an undecodable frame — the latter
    /// is a protocol bug, not a condition worth retrying past.
    pub async fn recv(&mut self) -> Option<E> {
        let bytes = self.rx.recv().await?;
        match serde_json::from_slice(&bytes) {
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
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use hyprlayer_proto::Outcome;

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
    async fn a_malformed_line_that_is_not_an_event_still_ends_the_connection() {
        let client = scripted_client(vec![r#"{"response":42}"#]).await;

        let err = client.call(any_call()).await.expect_err("the call fails");
        assert_eq!(err.code, WireError::disconnected().code);
    }
}
