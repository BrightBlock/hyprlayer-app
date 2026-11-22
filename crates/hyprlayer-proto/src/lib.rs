//! Wire contracts between `hyprlayer-server` and its clients (ADR-0009).
//!
//! `serde` is the only normal dependency. Nothing here reaches TypeScript, so
//! there is no `specta`; the desktop keeps its own `AppError` and converts.

pub mod commands;
pub mod frame;

use serde::{Deserialize, Serialize};

/// Strict equality on the handshake: the frame format and envelope, not the
/// command table. A client needing a newer server raises its minimum server
/// version instead (ADR-0009).
pub const PROTOCOL_VERSION: u32 = 1;

/// A hello feature: part frames, stream credit and event gaps. A side uses them only once its peer announced it.
pub const FLOW_CONTROL: &str = "flow_control";

/// A hello feature: terminal output as raw bytes in its stream frame (`frame::TAG_TERMINAL_OUTPUT`), not base64 in JSON.
pub const RAW_TERMINAL: &str = "raw_terminal";

/// A hello feature: the client reports what it has read (`received`), and the server keeps no more unread on the link than its measured rate and round trip need.
pub const LINK_CREDIT: &str = "link_credit";

/// A hello feature: the peer answers `ping` with `pong`, so a link gone silent can be given up.
pub const HEARTBEAT: &str = "heartbeat";

/// A hello feature: the client renders a terminal at the size its stream gives (`resized`), so
/// only the active client's fit sizes a shared terminal.
pub const TERMINAL_SIZE: &str = "terminal_size";

/// A hello feature: the client renders the server's screen of a terminal, so the server answers
/// the program's queries for it. A server that lists it takes `terminal_theme`.
pub const SCREEN_FRAMES: &str = "screen_frames";

/// A hello feature: a shared terminal's size has one owner, which typing doesn't move. A client
/// that lists it asks for the size with `terminal_take_size`; a server that lists it takes that,
/// and tells each frame stream's client whether it owns the size (`resized.owner`).
pub const SIZE_OWNER: &str = "size_owner";

/// The hello features a peer announced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    pub flow_control: bool,
    pub raw_terminal: bool,
    pub link_credit: bool,
    pub heartbeat: bool,
    pub terminal_size: bool,
    pub screen_frames: bool,
    pub size_owner: bool,
}

impl Features {
    pub const ALL: Features = Features {
        flow_control: true,
        raw_terminal: true,
        link_credit: true,
        heartbeat: true,
        terminal_size: true,
        screen_frames: true,
        size_owner: true,
    };

    /// Unknown names are a newer peer's, so they are ignored.
    pub fn parse(names: &[String]) -> Self {
        let has = |feature: &str| names.iter().any(|name| name == feature);
        Self {
            flow_control: has(FLOW_CONTROL),
            raw_terminal: has(RAW_TERMINAL),
            link_credit: has(LINK_CREDIT),
            heartbeat: has(HEARTBEAT),
            terminal_size: has(TERMINAL_SIZE),
            screen_frames: has(SCREEN_FRAMES),
            size_owner: has(SIZE_OWNER),
        }
    }

    pub fn names(self) -> Vec<String> {
        [
            (self.flow_control, FLOW_CONTROL),
            (self.raw_terminal, RAW_TERMINAL),
            (self.link_credit, LINK_CREDIT),
            (self.heartbeat, HEARTBEAT),
            (self.terminal_size, TERMINAL_SIZE),
            (self.screen_frames, SCREEN_FRAMES),
            (self.size_owner, SIZE_OWNER),
        ]
        .into_iter()
        .filter(|&(on, _)| on)
        .map(|(_, name)| name.to_string())
        .collect()
    }
}

/// Event bytes a stream may have unread at the client before the server waits for `Credit`.
pub const STREAM_WINDOW: u64 = 512 * 1024;

/// The v0.7 `AppError` envelope, pinned by
/// `src-tauri/tests/fixtures/compat/v0_7/app-errors.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_transcript: Option<StaleTranscript>,
}

/// The numbers a `stale_transcript` refusal carries; absent from every other error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleTranscript {
    pub rev: u64,
    pub committed_rev: u64,
    pub missing_turns: u64,
}

impl WireError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
            remedy: None,
            stale_transcript: None,
        }
    }

    /// A frame the peer should never have sent.
    pub fn protocol(message: impl Into<String>) -> Self {
        Self {
            code: "protocol_error".into(),
            message: message.into(),
            remedy: None,
            stale_transcript: None,
        }
    }

    /// A well-formed request this server cannot decode: a newer client's
    /// command, or arguments this build does not accept.
    pub fn unreadable_request() -> Self {
        Self {
            code: "unreadable_request".into(),
            message: "The server couldn't read that request.".into(),
            remedy: Some("Update the Hyprlayer server on this host.".into()),
            stale_transcript: None,
        }
    }

    pub fn unreadable_response() -> Self {
        Self {
            code: "unreadable_response".into(),
            message: "The app couldn't read the server's reply.".into(),
            remedy: Some("Update the Hyprlayer app.".into()),
            stale_transcript: None,
        }
    }

    pub fn unsupported_command() -> Self {
        Self {
            code: "unsupported_command".into(),
            message: "The server doesn't support this yet.".into(),
            remedy: Some("Update the Hyprlayer server on this host.".into()),
            stale_transcript: None,
        }
    }

    pub fn disconnected() -> Self {
        Self {
            code: "server_disconnected".into(),
            message: "The connection to the Hyprlayer server closed.".into(),
            remedy: None,
            stale_transcript: None,
        }
    }

    /// `ours` is the version of the side raising the mismatch; `theirs` is the
    /// peer's. Only the side that is *older* gets a remedy — it has no
    /// self-upgrade path — so this reads the same regardless of which side calls
    /// it: a client older than the server it just connected to learns to update
    /// itself, and a newer client evicting an older server gets no remedy text
    /// because it already knows what to do next (Phase 4 §5).
    pub fn version_mismatch(ours: u32, theirs: u32) -> Self {
        let remedy = (ours < theirs).then(|| "Update this app to match the newer server.".into());
        Self {
            code: "protocol_version_mismatch".into(),
            message: format!(
                "The app and server use different protocol versions ({ours} and {theirs})."
            ),
            remedy,
            stale_transcript: None,
        }
    }
}

/// Strict equality, not a range; the refusal names both versions.
pub fn check_version(peer: u32) -> Result<(), WireError> {
    if peer == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(WireError::version_mismatch(PROTOCOL_VERSION, peer))
    }
}

/// One control-plane line. The first line each way is `Hello`; a `Request` is
/// answered by the `Response` carrying its id; an `Event` is unsolicited.
/// `StreamRequest`/`StreamResponse` are the streaming form (Phase 4 §1): the
/// response carries a stream id, and the events that follow are data-plane
/// frames tagged with it (`frame::encode_stream_frame`) rather than more
/// control-plane lines — a second request/response type parameter, not a
/// variant of `Req`/`Resp`, because a streaming command's response shape
/// (a value plus a stream id) differs from a plain command's.
///
/// Externally tagged so a large body deserializes straight into its type
/// instead of through serde's buffered `Content`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlFrame<Req, Resp, StreamReq, StreamResp, Ev> {
    Hello {
        protocol_version: u32,
        /// SHA-256 of the server's own executable. Same-protocol builds are
        /// wire-compatible but not behaviour-identical, so this is what lets a
        /// client notice it is attached to an older server. Optional so that
        /// peers from before the field ignore it and are read as unknown.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        build: Option<String>,
        /// The server's own semver, which clients compare against their
        /// minimum. Absent from servers up to 0.8.2.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// What this side can read beyond the base protocol, such as `FLOW_CONTROL`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        features: Vec<String>,
        /// The server's per-stream credit window; the client returns credit in step with it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream_window: Option<u64>,
    },
    Refused {
        error: WireError,
    },
    Request {
        id: u64,
        body: Req,
    },
    Response {
        id: u64,
        outcome: Outcome<Resp>,
    },
    StreamRequest {
        id: u64,
        body: StreamReq,
    },
    StreamResponse {
        id: u64,
        outcome: Outcome<StreamResp>,
    },
    Event {
        event: Ev,
        /// Per connection and contiguous, so a jump is a gap. Absent from older servers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
    /// The server dropped `count` events from `first_seq` for a client that stopped reading.
    EventsMissed {
        first_seq: u64,
        count: u64,
    },
    /// Client-side cancellation of a stream opened over the data plane.
    Cancel {
        stream_id: u64,
    },
    /// The client consumed `bytes` of a stream's events; the server may send that many more.
    Credit {
        stream_id: u64,
        bytes: u64,
    },
    /// The sender read `bytes` more of the connection, the last of them `read_us` after its
    /// hello; its peer may have that many more in flight.
    Received {
        bytes: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_us: Option<u64>,
    },
    /// To a `HEARTBEAT` peer gone quiet; it answers `Pong`.
    Ping,
    Pong,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome<T> {
    Ok(T),
    Err(WireError),
}

impl<T> From<Result<T, WireError>> for Outcome<T> {
    fn from(result: Result<T, WireError>) -> Self {
        match result {
            Ok(value) => Outcome::Ok(value),
            Err(error) => Outcome::Err(error),
        }
    }
}

impl<T> From<Outcome<T>> for Result<T, WireError> {
    fn from(outcome: Outcome<T>) -> Self {
        match outcome {
            Outcome::Ok(value) => Ok(value),
            Outcome::Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{encode_control, Decoder, Frame};

    type Envelope = ControlFrame<u8, u8, u8, u8, u8>;

    #[test]
    fn wire_error_keeps_the_envelope_field_names() {
        let err = WireError {
            code: "session_not_found".into(),
            message: "No live session with id sess-fixture".into(),
            remedy: None,
            stale_transcript: None,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(
            json,
            r#"{"code":"session_not_found","message":"No live session with id sess-fixture","remedy":null}"#
        );
        let back: WireError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, err.code);
        assert_eq!(back.message, err.message);
        assert_eq!(back.remedy, err.remedy);
    }

    #[test]
    fn control_frames_are_externally_tagged_lines() {
        assert_eq!(
            serde_json::to_string(&Envelope::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: None,
                version: None,
                features: Vec::new(),
                stream_window: None,
            })
            .unwrap(),
            format!(r#"{{"hello":{{"protocol_version":{}}}}}"#, PROTOCOL_VERSION)
        );
        assert_eq!(
            serde_json::to_string(&Envelope::Request { id: 7, body: 3 }).unwrap(),
            r#"{"request":{"id":7,"body":3}}"#
        );
        assert_eq!(
            serde_json::to_string(&Envelope::Response {
                id: 7,
                outcome: Outcome::Ok(4)
            })
            .unwrap(),
            r#"{"response":{"id":7,"outcome":{"ok":4}}}"#
        );
        assert_eq!(
            serde_json::to_string(&Envelope::Shutdown).unwrap(),
            r#""shutdown""#
        );
        let back: Envelope = serde_json::from_str(r#"{"event":{"event":9}}"#).unwrap();
        assert!(matches!(
            back,
            ControlFrame::Event {
                event: 9,
                seq: None
            }
        ));
    }

    #[test]
    fn a_hello_without_a_version_still_parses_as_an_unversioned_server() {
        let old: Envelope =
            serde_json::from_str(r#"{"hello":{"protocol_version":1,"build":"abc"}}"#).unwrap();
        assert!(matches!(old, ControlFrame::Hello { version: None, .. }));

        let json = serde_json::to_string(&Envelope::Hello {
            protocol_version: 1,
            build: None,
            version: Some("0.8.3".into()),
            features: Vec::new(),
            stream_window: None,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"hello":{"protocol_version":1,"version":"0.8.3"}}"#
        );
    }

    #[test]
    fn a_client_on_the_next_protocol_version_is_refused() {
        let hello = serde_json::to_string(&Envelope::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
            build: None,
            version: None,
            features: Vec::new(),
            stream_window: None,
        })
        .unwrap();
        let mut decoder = Decoder::new();
        decoder.feed(&encode_control(&hello));
        let Some(Frame::Control(line)) = decoder.next_frame().unwrap() else {
            panic!("the hello line decodes as one control frame");
        };
        let ControlFrame::Hello {
            protocol_version, ..
        } = serde_json::from_str::<Envelope>(&line).unwrap()
        else {
            panic!("the first line is a hello");
        };

        let refused = check_version(protocol_version).unwrap_err();
        assert_eq!(refused.code, "protocol_version_mismatch");
        assert!(refused.message.contains(&PROTOCOL_VERSION.to_string()));
        assert!(refused
            .message
            .contains(&(PROTOCOL_VERSION + 1).to_string()));
        assert!(check_version(PROTOCOL_VERSION).is_ok());
    }

    #[test]
    fn only_the_older_side_of_a_mismatch_gets_a_remedy() {
        // We are older than the peer: we have no self-upgrade path, so the
        // error we construct about ourselves carries the remedy.
        let older = WireError::version_mismatch(1, 2);
        assert_eq!(
            older.remedy.as_deref(),
            Some("Update this app to match the newer server.")
        );

        // We are newer than the peer: no remedy — we're about to evict them
        // (Phase 4 §5) and already know what happens next.
        let newer = WireError::version_mismatch(2, 1);
        assert_eq!(newer.remedy, None);
    }
}
