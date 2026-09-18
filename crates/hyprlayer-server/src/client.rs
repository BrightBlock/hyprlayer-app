//! The client half of the connection: correlates each request with the
//! response carrying its id and surfaces the server's unsolicited events. The
//! typed methods (`list_sessions`, …) are generated onto `RpcClient` by the
//! command table in `rpc.rs`.

use std::collections::HashMap;
use std::io;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_models::RuntimeEvent;
use hyprlayer_proto::frame::{self, Frame};
use hyprlayer_proto::{check_version, WireError, PROTOCOL_VERSION};
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::rpc::{Request, Response};
use crate::socket;
use crate::streams::{StreamRequest, StreamResponse};
use crate::transport::{
    label, parse_control, Control, FrameReader, FrameWriter, OUTBOUND_QUEUE_FRAMES,
};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Response, WireError>>>>>;

/// A stream's raw frames plus the response value that opened it, handed to
/// the awaiting caller together — see `read_loop`'s `StreamResponse` arm for
/// why the receiver is registered before this is ever sent.
type StreamOpened = (StreamResponse, Option<mpsc::UnboundedReceiver<Vec<u8>>>);
type PendingStream = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<StreamOpened, WireError>>>>>;
type StreamSenders = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>;

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
            })
            .await?;
        let (server_version, server_build) = match reader.next_control().await? {
            Some(Control::Hello {
                protocol_version,
                build,
            }) => {
                check_version(protocol_version).map_err(ConnectError::Refused)?;
                (protocol_version, build)
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
                if writer.write_bytes(&bytes).await.is_err() {
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
        rx.await.unwrap_or_else(|_| Err(WireError::disconnected()))
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
        rx.await.unwrap_or_else(|_| Err(WireError::disconnected()))
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

async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: FrameReader<R>,
    pending: Pending,
    pending_streams: PendingStream,
    stream_senders: StreamSenders,
    events: mpsc::UnboundedSender<RuntimeEvent>,
    writer: tokio::task::AbortHandle,
) {
    loop {
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
                Err(e) => {
                    tracing::warn!("unreadable control frame from server: {e}");
                    break;
                }
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
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("server connection read failed: {e}");
                break;
            }
        }
    }
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
    for tx in waiters {
        let _ = tx.send(Err(WireError::disconnected()));
    }
    let stream_waiters: Vec<_> = pending_streams
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain()
        .map(|(_, tx)| tx)
        .collect();
    for tx in stream_waiters {
        let _ = tx.send(Err(WireError::disconnected()));
    }
    // Dropping every sender ends each open stream's `recv()` with `None`,
    // same as a local channel closing when its producer goes away.
    stream_senders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
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
                tracing::error!("stream event did not decode: {e}");
                None
            }
        }
    }
}
