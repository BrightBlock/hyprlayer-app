//! Two planes on one connection, split by a one-byte tag: a control frame is
//! one JSON line, a data frame a `u32` little-endian length then the payload.
//! Pure bytes in, frames out; the async I/O lives beside the socket.

pub const TAG_CONTROL: u8 = b'C';
pub const TAG_DATA: u8 = b'D';

#[derive(Clone, PartialEq, Eq)]
pub enum Frame {
    /// One JSON document, without its terminating newline.
    Control(String),
    Data(Vec<u8>),
}

// A control line is a whole request or response body: print its size, not its
// contents.
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Frame::Control(line) => write!(f, "Control({} bytes)", line.len()),
            Frame::Data(payload) => write!(f, "Data({} bytes)", payload.len()),
        }
    }
}

pub fn encode(frame: &Frame) -> Vec<u8> {
    match frame {
        Frame::Control(line) => encode_control(line),
        Frame::Data(payload) => encode_data(payload),
    }
}

pub fn encode_control(line: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len() + 2);
    out.push(TAG_CONTROL);
    out.extend_from_slice(line.as_bytes());
    out.push(b'\n');
    out
}

pub fn encode_data(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(TAG_DATA);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// A data frame carrying one client-facing stream's event: the payload is an
/// 8-byte little-endian stream id followed by the encoded event. One data
/// plane serves every open stream on the connection; this is the sub-framing
/// that tells them apart.
pub fn encode_stream_frame(stream_id: u64, event: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + event.len());
    payload.extend_from_slice(&stream_id.to_le_bytes());
    payload.extend_from_slice(event);
    encode_data(&payload)
}

/// Splits a data frame's payload into its stream id and the event bytes.
/// `None` for a payload too short to carry the id, which is always a bug on
/// the sending side rather than a wire condition to recover from.
pub fn decode_stream_frame(payload: &[u8]) -> Option<(u64, &[u8])> {
    if payload.len() < 8 {
        return None;
    }
    let (id_bytes, rest) = payload.split_at(8);
    Some((u64::from_le_bytes(id_bytes.try_into().ok()?), rest))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    UnknownTag(u8),
    ControlNotUtf8,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnknownTag(tag) => write!(f, "unknown frame tag 0x{tag:02x}"),
            DecodeError::ControlNotUtf8 => write!(f, "control frame is not UTF-8"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Incremental decoder: feed bytes as they arrive, take frames as they complete.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    start: usize,
    // Where the newline search resumes, so a partially delivered control line
    // is not rescanned from its first byte on every feed.
    scanned: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// True when no partial frame is buffered, so an EOF here is a clean one.
    pub fn is_empty(&self) -> bool {
        self.start == self.buf.len()
    }

    /// Bytes fed but not yet taken as a frame: what arrived instead of a hello.
    pub fn unconsumed(&self) -> &[u8] {
        &self.buf[self.start..]
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, DecodeError> {
        let pending = &self.buf[self.start..];
        let Some(&tag) = pending.first() else {
            return Ok(None);
        };
        match tag {
            TAG_CONTROL => {
                let from = self.scanned.max(1);
                let Some(offset) = pending[from..].iter().position(|&b| b == b'\n') else {
                    self.scanned = pending.len();
                    return Ok(None);
                };
                let end = from + offset;
                let line = std::str::from_utf8(&pending[1..end])
                    .map_err(|_| DecodeError::ControlNotUtf8)?
                    .to_owned();
                self.consume(end + 1);
                Ok(Some(Frame::Control(line)))
            }
            TAG_DATA => {
                if pending.len() < 5 {
                    return Ok(None);
                }
                let len =
                    u32::from_le_bytes([pending[1], pending[2], pending[3], pending[4]]) as usize;
                if pending.len() < 5 + len {
                    return Ok(None);
                }
                let payload = pending[5..5 + len].to_vec();
                self.consume(5 + len);
                Ok(Some(Frame::Data(payload)))
            }
            other => Err(DecodeError::UnknownTag(other)),
        }
    }

    fn consume(&mut self, n: usize) {
        self.start += n;
        self.scanned = 0;
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_when_delivered_one_byte_at_a_time() {
        let frames = vec![
            Frame::Control(r#"{"hello":{"protocol_version":1}}"#.into()),
            Frame::Data(vec![0, 1, 2, 255]),
            Frame::Control(String::new()),
            Frame::Data(Vec::new()),
            Frame::Control("tail".into()),
        ];
        let wire: Vec<u8> = frames.iter().flat_map(encode).collect();

        let mut decoder = Decoder::new();
        let mut decoded = Vec::new();
        for byte in wire {
            decoder.feed(&[byte]);
            while let Some(frame) = decoder.next_frame().unwrap() {
                decoded.push(frame);
            }
        }
        assert_eq!(decoded, frames);
        assert!(decoder.next_frame().unwrap().is_none());
    }

    #[test]
    fn two_frames_in_one_feed_come_out_in_order() {
        let mut wire = encode_data(b"first");
        wire.extend(encode_control("second"));
        let mut decoder = Decoder::new();
        decoder.feed(&wire);
        assert_eq!(
            decoder.next_frame().unwrap(),
            Some(Frame::Data(b"first".to_vec()))
        );
        assert_eq!(
            decoder.next_frame().unwrap(),
            Some(Frame::Control("second".into()))
        );
        assert_eq!(decoder.next_frame().unwrap(), None);
    }

    #[test]
    fn an_unknown_tag_is_an_error_and_bad_utf8_is_named() {
        let mut decoder = Decoder::new();
        decoder.feed(b"X");
        assert_eq!(decoder.next_frame(), Err(DecodeError::UnknownTag(b'X')));

        let mut decoder = Decoder::new();
        decoder.feed(&[TAG_CONTROL, 0xff, b'\n']);
        assert_eq!(decoder.next_frame(), Err(DecodeError::ControlNotUtf8));
    }

    #[test]
    fn the_data_length_prefix_is_little_endian() {
        let wire = encode_data(&[7; 258]);
        assert_eq!(&wire[..5], &[TAG_DATA, 2, 1, 0, 0]);
    }

    #[test]
    fn a_stream_frame_carries_its_id_ahead_of_the_event_bytes() {
        let wire = encode_stream_frame(42, b"hello");
        let mut decoder = Decoder::new();
        decoder.feed(&wire);
        let Some(Frame::Data(payload)) = decoder.next_frame().unwrap() else {
            panic!("a stream frame is a data frame");
        };
        let (id, event) = decode_stream_frame(&payload).expect("payload carries an id");
        assert_eq!(id, 42);
        assert_eq!(event, b"hello");
    }

    #[test]
    fn a_payload_too_short_for_a_stream_id_decodes_to_none() {
        assert_eq!(decode_stream_frame(&[1, 2, 3]), None);
    }
}
