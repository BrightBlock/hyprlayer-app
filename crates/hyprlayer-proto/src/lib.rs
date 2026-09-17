//! Wire contracts between `hyprlayer-server` and its clients (ADR-0009).
//!
//! `serde` is the only normal dependency. Nothing here reaches TypeScript, so
//! there is no `specta`; the desktop keeps its own `AppError` and converts.

pub mod commands;
pub mod frame;

use serde::{Deserialize, Serialize};

/// Strict equality on the handshake. Stays at 1 until the first release that
/// ships `hyprlayer-server` — nothing has ever shipped with a different
/// number, so wire changes before that release are free. Bump on every
/// wire-affecting change from that release onward.
pub const PROTOCOL_VERSION: u32 = 1;

/// The v0.7 `AppError` envelope, pinned by
/// `src-tauri/tests/fixtures/compat/v0_7/app-errors.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
}

impl WireError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
            remedy: None,
        }
    }

    /// A frame the peer should never have sent.
    pub fn protocol(message: impl Into<String>) -> Self {
        Self {
            code: "protocol_error".into(),
            message: message.into(),
            remedy: None,
        }
    }

    pub fn disconnected() -> Self {
        Self {
            code: "server_disconnected".into(),
            message: "The connection to the Hyprlayer server closed.".into(),
            remedy: None,
        }
    }

    /// `ours` is the version of the side raising the mismatch; `theirs` is the
    /// peer's. Only the side that is *older* gets a remedy — it has no
    /// self-upgrade path — so this reads the same regardless of which side calls
    /// it: a client older than the server it just connected to learns to update
    /// itself, and a newer client evicting an older server gets no remedy text
    /// because it already knows what to do next (Phase 4 §5).
    pub fn version_mismatch(ours: u32, theirs: u32) -> Self {
        let remedy = (ours < theirs)
            .then(|| "This server was started by a newer Hyprlayer. Update this client.".into());
        Self {
            code: "protocol_version_mismatch".into(),
            message: format!(
                "This side speaks Hyprlayer protocol version {ours}; the peer sent version {theirs}."
            ),
            remedy,
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
    },
    /// Client-side cancellation of a stream opened over the data plane.
    Cancel {
        stream_id: u64,
    },
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
                protocol_version: PROTOCOL_VERSION
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
        assert!(matches!(back, ControlFrame::Event { event: 9 }));
    }

    #[test]
    fn a_client_on_the_next_protocol_version_is_refused() {
        let hello = serde_json::to_string(&Envelope::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
        })
        .unwrap();
        let mut decoder = Decoder::new();
        decoder.feed(&encode_control(&hello));
        let Some(Frame::Control(line)) = decoder.next_frame().unwrap() else {
            panic!("the hello line decodes as one control frame");
        };
        let ControlFrame::Hello { protocol_version } =
            serde_json::from_str::<Envelope>(&line).unwrap()
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
            Some("This server was started by a newer Hyprlayer. Update this client.")
        );

        // We are newer than the peer: no remedy — we're about to evict them
        // (Phase 4 §5) and already know what happens next.
        let newer = WireError::version_mismatch(2, 1);
        assert_eq!(newer.remedy, None);
    }
}
