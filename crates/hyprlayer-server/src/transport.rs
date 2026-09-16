//! Frames over any `AsyncRead`/`AsyncWrite`, and the control line both ends
//! exchange. The encoding itself is `hyprlayer_proto::frame`.

use std::io;

use hyprlayer_models::RuntimeEvent;
use hyprlayer_proto::frame::{self, Decoder, Frame};
use hyprlayer_proto::ControlFrame;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::rpc::{Request, Response};
use crate::streams::{StreamRequest, StreamResponse};

pub type Control = ControlFrame<Request, Response, StreamRequest, StreamResponse, RuntimeEvent>;

/// Frames queued to one peer before its writer applies backpressure.
pub const OUTBOUND_QUEUE_FRAMES: usize = 1024;

const READ_CHUNK: usize = 64 * 1024;

pub fn encode_control(control: &Control) -> Result<Vec<u8>, serde_json::Error> {
    Ok(frame::encode_control(&serde_json::to_string(control)?))
}

pub fn parse_control(line: &str) -> io::Result<Control> {
    serde_json::from_str(line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn label(control: &Control) -> &'static str {
    match control {
        Control::Hello { .. } => "hello",
        Control::Refused { .. } => "refused",
        Control::Request { .. } => "request",
        Control::Response { .. } => "response",
        Control::StreamRequest { .. } => "stream_request",
        Control::StreamResponse { .. } => "stream_response",
        Control::Event { .. } => "event",
        Control::Cancel { .. } => "cancel",
        Control::Shutdown => "shutdown",
    }
}

pub struct FrameReader<R> {
    inner: R,
    decoder: Decoder,
    chunk: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            decoder: Decoder::new(),
            chunk: vec![0; READ_CHUNK],
        }
    }

    /// `None` at a clean EOF; an EOF inside a frame is `UnexpectedEof`.
    pub async fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        loop {
            if let Some(frame) = self
                .decoder
                .next_frame()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            {
                return Ok(Some(frame));
            }
            let n = self.inner.read(&mut self.chunk).await?;
            if n == 0 {
                return if self.decoder.is_empty() {
                    Ok(None)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            }
            self.decoder.feed(&self.chunk[..n]);
        }
    }

    /// The next control line, parsed; a data frame in its place is invalid.
    pub async fn next_control(&mut self) -> io::Result<Option<Control>> {
        match self.next_frame().await? {
            Some(Frame::Control(line)) => parse_control(&line).map(Some),
            Some(Frame::Data(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected a control frame, got a data frame",
            )),
            None => Ok(None),
        }
    }
}

pub struct FrameWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub async fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes).await?;
        self.inner.flush().await
    }

    pub async fn write_control(&mut self, control: &Control) -> io::Result<()> {
        let bytes =
            encode_control(control).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.write_bytes(&bytes).await
    }
}
