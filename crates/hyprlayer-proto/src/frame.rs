//! Two planes on one connection, split by a one-byte tag: a control frame is
//! one JSON line, a data frame a `u32` little-endian length then the payload.
//! A part frame carries a slice of a larger frame, so smaller ones can go
//! between its slices. Pure bytes in, frames out; the async I/O lives beside
//! the socket.

pub const TAG_CONTROL: u8 = b'C';
pub const TAG_DATA: u8 = b'D';
pub const TAG_PART: u8 = b'P';

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

/// One slice of a whole frame's encoding. A lane carries one frame at a time, so
/// its slices concatenate to that frame; `last` closes it. Payload: lane byte,
/// `last` byte, slice.
pub fn encode_part(lane: u8, last: bool, slice: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(slice.len() + 7);
    out.push(TAG_PART);
    out.extend_from_slice(&(slice.len() as u32 + 2).to_le_bytes());
    out.push(lane);
    out.push(u8::from(last));
    out.extend_from_slice(slice);
    out
}

/// Leads a terminal stream event that is raw output: then the output's absolute
/// offset as a `u64` little-endian, then its bytes. Other events are JSON, which
/// never starts with this byte.
pub const TAG_TERMINAL_OUTPUT: u8 = b'O';

/// A data frame carrying one client-facing stream's event: the payload is an
/// 8-byte little-endian stream id followed by the encoded event. One data
/// plane serves every open stream on the connection; this is the sub-framing
/// that tells them apart.
pub fn encode_stream_frame(stream_id: u64, event: &[u8]) -> Vec<u8> {
    encode_stream_event(stream_id, &[event])
}

/// A stream frame whose event is raw terminal output at `offset`.
pub fn encode_terminal_output_frame(stream_id: u64, offset: u64, output: &[u8]) -> Vec<u8> {
    encode_stream_event(
        stream_id,
        &[&[TAG_TERMINAL_OUTPUT], &offset.to_le_bytes(), output],
    )
}

/// What a whole stream frame carries ahead of its event: tag, length, stream id.
pub const STREAM_FRAME_HEADER: usize = 13;

/// What a raw terminal output frame carries ahead of its bytes: the stream frame's header, tag, offset.
pub const TERMINAL_OUTPUT_HEADER: usize = STREAM_FRAME_HEADER + 9;

/// A raw terminal output event's offset and bytes; `None` for any other event.
pub fn decode_terminal_output(event: &[u8]) -> Option<(u64, &[u8])> {
    let [TAG_TERMINAL_OUTPUT, rest @ ..] = event else {
        return None;
    };
    let (offset, output) = rest.split_first_chunk::<8>()?;
    Some((u64::from_le_bytes(*offset), output))
}

// One allocation for the whole frame, however many pieces the event comes in.
fn encode_stream_event(stream_id: u64, event: &[&[u8]]) -> Vec<u8> {
    let len = 8 + event.iter().map(|piece| piece.len()).sum::<usize>();
    let mut out = Vec::with_capacity(5 + len);
    out.push(TAG_DATA);
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&stream_id.to_le_bytes());
    for piece in event {
        out.extend_from_slice(piece);
    }
    out
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
    /// A lane's parts did not reassemble into exactly one control or data frame.
    BadPart,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnknownTag(tag) => write!(f, "unknown frame tag 0x{tag:02x}"),
            DecodeError::ControlNotUtf8 => write!(f, "control frame is not UTF-8"),
            DecodeError::BadPart => write!(f, "part frames did not reassemble into one frame"),
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
    /// Each lane's frame so far, while its parts arrive.
    partial: Vec<(u8, Vec<u8>)>,
    consumed: u64,
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
        self.start == self.buf.len() && self.partial.is_empty()
    }

    /// Bytes fed but not yet taken as a frame: what arrived instead of a hello.
    pub fn unconsumed(&self) -> &[u8] {
        &self.buf[self.start..]
    }

    /// Bytes taken as whole frames or parts so far.
    pub fn consumed(&self) -> u64 {
        self.consumed
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, DecodeError> {
        loop {
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
                    return Ok(Some(Frame::Control(line)));
                }
                TAG_DATA | TAG_PART => {
                    if pending.len() < 5 {
                        return Ok(None);
                    }
                    let len = u32::from_le_bytes([pending[1], pending[2], pending[3], pending[4]])
                        as usize;
                    if pending.len() < 5 + len {
                        return Ok(None);
                    }
                    let payload = pending[5..5 + len].to_vec();
                    self.consume(5 + len);
                    if tag == TAG_DATA {
                        return Ok(Some(Frame::Data(payload)));
                    }
                    if let Some(frame) = self.take_part(&payload)? {
                        return Ok(Some(frame));
                    }
                }
                other => return Err(DecodeError::UnknownTag(other)),
            }
        }
    }

    fn take_part(&mut self, payload: &[u8]) -> Result<Option<Frame>, DecodeError> {
        let [lane, last, slice @ ..] = payload else {
            return Err(DecodeError::BadPart);
        };
        let index = match self.partial.iter().position(|(l, _)| l == lane) {
            Some(index) => index,
            None => {
                self.partial.push((*lane, Vec::new()));
                self.partial.len() - 1
            }
        };
        self.partial[index].1.extend_from_slice(slice);
        if *last == 0 {
            return Ok(None);
        }
        let (_, whole) = self.partial.swap_remove(index);
        if whole.first() == Some(&TAG_PART) {
            return Err(DecodeError::BadPart);
        }
        let mut inner = Decoder::new();
        inner.feed(&whole);
        match inner.next_frame() {
            Ok(Some(frame)) if inner.is_empty() => Ok(Some(frame)),
            _ => Err(DecodeError::BadPart),
        }
    }

    fn consume(&mut self, n: usize) {
        self.consumed += n as u64;
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

    fn parts(lane: u8, frame: &[u8], size: usize) -> Vec<Vec<u8>> {
        let chunks: Vec<&[u8]> = frame.chunks(size).collect();
        let last = chunks.len() - 1;
        chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| encode_part(lane, i == last, chunk))
            .collect()
    }

    #[test]
    fn two_lanes_of_parts_and_a_whole_frame_interleave_one_byte_at_a_time() {
        let big_control = format!("{{\"response\":\"{}\"}}", "x".repeat(300));
        let big_data = encode_stream_frame(7, &[9; 500]);
        let control_parts = parts(0, &encode_control(&big_control), 64);
        let data_parts = parts(1, &big_data, 100);
        let mut wire = Vec::new();
        for i in 0..control_parts.len().max(data_parts.len()) {
            if let Some(part) = control_parts.get(i) {
                wire.extend(part);
            }
            wire.extend(encode_control("small"));
            if let Some(part) = data_parts.get(i) {
                wire.extend(part);
            }
        }

        let mut decoder = Decoder::new();
        let mut decoded = Vec::new();
        for byte in wire {
            decoder.feed(&[byte]);
            while let Some(frame) = decoder.next_frame().unwrap() {
                decoded.push(frame);
            }
        }
        let whole: Vec<_> = decoded
            .iter()
            .filter(|f| **f != Frame::Control("small".into()))
            .collect();
        assert_eq!(
            whole,
            vec![
                &Frame::Control(big_control),
                &Frame::Data(big_data[5..].to_vec())
            ]
        );
        assert_eq!(
            decoded.len() - whole.len(),
            control_parts.len().max(data_parts.len())
        );
        assert!(decoder.is_empty());
    }

    #[test]
    fn parts_that_are_not_exactly_one_frame_are_an_error() {
        let mut two = encode_control("a");
        two.extend(encode_control("b"));
        for inner in [two, b"Cno newline".to_vec(), encode_part(0, true, b"Cx\n")] {
            let mut decoder = Decoder::new();
            decoder.feed(&encode_part(0, true, &inner));
            assert_eq!(decoder.next_frame(), Err(DecodeError::BadPart));
        }
    }

    #[test]
    fn a_half_assembled_frame_is_not_a_clean_end() {
        let mut decoder = Decoder::new();
        decoder.feed(&encode_part(1, false, b"Cpartial"));
        assert_eq!(decoder.next_frame(), Ok(None));
        assert!(!decoder.is_empty());
    }
}
