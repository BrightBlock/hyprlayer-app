//! Frames over any `AsyncRead`/`AsyncWrite`, and the control line both ends
//! exchange. The encoding itself is `hyprlayer_proto::frame`.

use std::io;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_models::RuntimeEvent;
use hyprlayer_proto::frame::{self, Decoder, Frame};
use hyprlayer_proto::{ControlFrame, Outcome, WireError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::dispatch::Answer;
use crate::rpc::{Request, Response};
use crate::streams::{StreamRequest, StreamResponse};

pub type Control = ControlFrame<Request, Response, StreamRequest, StreamResponse, RuntimeEvent>;

const READ_CHUNK: usize = 64 * 1024;

/// Serialized straight into the frame: a large response is never held twice.
pub fn encode_control(control: &Control) -> Result<Vec<u8>, serde_json::Error> {
    let mut out = vec![frame::TAG_CONTROL];
    serde_json::to_writer(&mut out, control)?;
    out.push(b'\n');
    Ok(out)
}

const QUAD_BYTES: usize = 4096;

/// A frame's bytes, or base64 of `raw` set in its envelope, encoded a part at a time as the
/// writer reaches it: the base64 is never whole in memory.
pub enum Body {
    Bytes(Vec<u8>),
    Base64 {
        head: Vec<u8>,
        raw: Vec<u8>,
        tail: Vec<u8>,
    },
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Body::Bytes(bytes)
    }
}

impl Body {
    pub(crate) fn len(&self) -> usize {
        match self {
            Body::Bytes(bytes) => bytes.len(),
            Body::Base64 { head, raw, tail } => head.len() + raw.len().div_ceil(3) * 4 + tail.len(),
        }
    }

    pub(crate) fn extend(&self, range: std::ops::Range<usize>, out: &mut Vec<u8>) {
        let (head, raw, tail) = match self {
            Body::Bytes(bytes) => return out.extend_from_slice(&bytes[range]),
            Body::Base64 { head, raw, tail } => (head, raw, tail),
        };
        let encoded = head.len()..head.len() + raw.len().div_ceil(3) * 4;
        out.extend_from_slice(&head[range.start.min(encoded.start)..range.end.min(encoded.start)]);
        let mut at = range.start.clamp(encoded.start, encoded.end) - encoded.start;
        let to = range.end.clamp(encoded.start, encoded.end) - encoded.start;
        // Whole quads through the stack, so a part costs the writer a copy and no fill.
        let mut quads = [0u8; QUAD_BYTES];
        while at < to {
            let first = at / 4;
            let last = to.div_ceil(4).min(first + QUAD_BYTES / 4);
            B64.encode_slice(&raw[first * 3..(last * 3).min(raw.len())], &mut quads)
                .expect("sized to the quads");
            let end = to.min(last * 4);
            out.extend_from_slice(&quads[at - first * 4..end - first * 4]);
            at = end;
        }
        let past =
            range.start.max(encoded.end) - encoded.end..range.end.max(encoded.end) - encoded.end;
        out.extend_from_slice(&tail[past]);
    }

    /// The whole frame, for a peer that reads no parts.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        match self {
            Body::Bytes(bytes) => bytes,
            body => {
                let mut out = Vec::with_capacity(body.len());
                body.extend(0..body.len(), &mut out);
                out
            }
        }
    }
}

/// The response frame for `outcome`; an attachment's bytes go in raw, for `Body::Base64`.
pub(crate) fn encode_answer(id: u64, outcome: Outcome<Answer>) -> Result<Body, serde_json::Error> {
    let outcome = match outcome {
        Outcome::Ok(Answer::Attachment(raw)) => {
            // The envelope as serde writes it, split at its empty string.
            let empty = Outcome::Ok(Response::ReadAttachment(String::new()));
            let mut head = encode_control(&Control::Response { id, outcome: empty })?;
            let at = head
                .windows(2)
                .position(|pair| pair == b"\"\"")
                .expect("the envelope holds the empty string")
                + 1;
            let tail = head.split_off(at);
            return Ok(Body::Base64 { head, raw, tail });
        }
        Outcome::Ok(Answer::Response(response)) => Outcome::Ok(response),
        Outcome::Err(error) => Outcome::Err(error),
    };
    encode_control(&Control::Response { id, outcome }).map(Body::Bytes)
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
        Control::Received { .. } => "received",
        Control::Ping => "ping",
        Control::Pong => "pong",
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

    /// Bytes taken as frames or parts so far, the hello included.
    pub fn consumed(&self) -> u64 {
        self.decoder.consumed()
    }

    /// `None` at a clean EOF; an EOF inside a frame is `UnexpectedEof`.
    pub async fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        loop {
            if let Some(frame) = self.decoded()? {
                return Ok(Some(frame));
            }
            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    /// The next frame already read, without reading more.
    pub fn decoded(&mut self) -> io::Result<Option<Frame>> {
        self.decoder
            .next_frame()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Reads once; `false` at a clean EOF, and an EOF inside a frame is `UnexpectedEof`.
    pub async fn fill(&mut self) -> io::Result<bool> {
        let n = self.inner.read(&mut self.chunk).await?;
        if n == 0 {
            return if self.decoder.is_empty() {
                Ok(false)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        self.decoder.feed(&self.chunk[..n]);
        Ok(true)
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
