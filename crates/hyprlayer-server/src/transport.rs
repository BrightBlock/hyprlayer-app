//! Frames over any `AsyncRead`/`AsyncWrite`, and the control line both ends
//! exchange. The encoding itself is `hyprlayer_proto::frame`.

use std::io;

use hyprlayer_models::RuntimeEvent;
use hyprlayer_proto::frame::{self, Decoder, Frame};
use hyprlayer_proto::{ControlFrame, Outcome, WireError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::rpc::{Request, Response};
use crate::streams::{StreamRequest, StreamResponse};

pub type Control = ControlFrame<Request, Response, StreamRequest, StreamResponse, RuntimeEvent>;

/// Requests a client queues before its callers wait.
pub const OUTBOUND_QUEUE_FRAMES: usize = 1024;

const READ_CHUNK: usize = 64 * 1024;

pub fn encode_control(control: &Control) -> Result<Vec<u8>, serde_json::Error> {
    Ok(frame::encode_control(&serde_json::to_string(control)?))
}

/// `Control::Event` around an event serialized once for every connection; only `seq` differs.
pub fn encode_event(event_json: &str, seq: u64) -> Vec<u8> {
    let seq = seq.to_string();
    let mut out = Vec::with_capacity(event_json.len() + seq.len() + 30);
    out.push(frame::TAG_CONTROL);
    out.extend_from_slice(br#"{"event":{"event":"#);
    out.extend_from_slice(event_json.as_bytes());
    out.extend_from_slice(br#","seq":"#);
    out.extend_from_slice(seq.as_bytes());
    out.extend_from_slice(b"}}\n");
    out
}

pub fn parse_control(line: &str) -> io::Result<Control> {
    serde_json::from_str(line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The error reply to a request `parse_control` rejected, when the line still
/// names its id; `None` for a frame too broken to answer.
pub fn reject_undecodable(line: &str) -> Option<(String, Control)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let (kind, envelope) = value.as_object()?.iter().next()?;
    let id = envelope.get("id")?.as_u64()?;
    let body = envelope.get("body")?;
    let command = match body.as_object() {
        Some(tagged) => tagged.keys().next()?.clone(),
        None => body.as_str()?.to_string(),
    };
    let error = WireError::unsupported_command();
    let reply = match kind.as_str() {
        "request" => Control::Response {
            id,
            outcome: Outcome::Err(error),
        },
        "stream_request" => Control::StreamResponse {
            id,
            outcome: Outcome::Err(error),
        },
        _ => return None,
    };
    Some((command, reply))
}

/// The envelope kind and id of a control line this build cannot decode, so the
/// reader can answer or fail that one call instead of the connection.
pub fn undecodable_id(line: &str) -> Option<(String, u64)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let (kind, envelope) = value.as_object()?.iter().next()?;
    Some((kind.clone(), envelope.get("id")?.as_u64()?))
}

/// The kind and seq of an event this build cannot decode, so the reader can skip it.
pub fn undecodable_event(line: &str) -> Option<(String, Option<u64>)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let envelope = value.get("event")?;
    let kind = match envelope.get("event")? {
        serde_json::Value::String(unit) => unit.clone(),
        serde_json::Value::Object(tagged) => tagged.keys().next()?.clone(),
        _ => return None,
    };
    Some((
        kind,
        envelope.get("seq").and_then(serde_json::Value::as_u64),
    ))
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
        Control::EventsMissed { .. } => "events_missed",
        Control::Cancel { .. } => "cancel",
        Control::Credit { .. } => "credit",
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

    pub fn unconsumed(&self) -> &[u8] {
        self.decoder.unconsumed()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spliced_event_line_is_the_one_serde_writes() {
        for event in [
            RuntimeEvent::BinariesChanged,
            RuntimeEvent::WorktreeChanged {
                workstream_id: "ws \"quoted\"\n".into(),
            },
        ] {
            let json = serde_json::to_string(&event).unwrap();
            let seq = 18_446_744_073_709_551_615;
            let control = Control::Event {
                event,
                seq: Some(seq),
            };
            assert_eq!(encode_event(&json, seq), encode_control(&control).unwrap());
        }
    }
}
