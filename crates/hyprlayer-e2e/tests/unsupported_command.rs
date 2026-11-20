//! A client and server built apart: a command the server lacks, a frame it can't
//! read, or an answer the client can't read fails at most that one call; the
//! connection, and every session it carries, keeps serving.
#![cfg(unix)]

use std::time::Duration;

use hyprlayer_e2e::{json, Journey, Sandbox, Server};
use hyprlayer_proto::frame::encode_control;
use hyprlayer_proto::{Outcome, PROTOCOL_VERSION};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::rpc::{Request, Response};
use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};
use tokio::net::{UnixListener, UnixStream};

const IO: Duration = Duration::from_secs(15);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_command() {
    let sandbox = Sandbox::new();
    let mut j = Journey::new("unsupported_command", &sandbox);
    let _server = Server::start(&sandbox).await;

    j.step("connect over the raw socket");
    let stream = UnixStream::connect(sandbox.socket())
        .await
        .expect("connect");
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);
    let hello = next(&mut reader).await;
    j.guard(
        "the server greets first",
        matches!(hello, Control::Hello { .. }),
        json!({}),
    );
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: None,
            version: None,
            features: Vec::new(),
            stream_window: None,
        })
        .await
        .expect("hello");

    j.step("send a command this server does not have");
    writer
        .write_bytes(&encode_control(
            r#"{"request":{"id":7,"body":{"from_a_newer_client":{"x":1}}}}"#,
        ))
        .await
        .expect("write unknown request");
    let refused = match next(&mut reader).await {
        Control::Response {
            id,
            outcome: Outcome::Err(error),
        } => Some((id, error)),
        _ => None,
    };
    j.guard(
        "it is answered with an error carrying the request's id",
        refused.as_ref().is_some_and(|(id, _)| *id == 7),
        json!({ "id": refused.as_ref().map(|(id, _)| id) }),
    );
    let error = refused.map(|(_, e)| e);
    j.guard(
        "the error is one plain sentence that says to update the server",
        error.as_ref().is_some_and(|e| {
            e.code == "unsupported_command"
                && !e.message.contains("from_a_newer_client")
                && e.message.matches(". ").count() == 0
                && e.remedy.is_some()
        }),
        json!({ "error": error.as_ref().map(|e| (&e.code, &e.message, &e.remedy)) }),
    );

    j.step("send a stream request this server does not have");
    writer
        .write_bytes(&encode_control(
            r#"{"stream_request":{"id":8,"body":{"from_a_newer_client":{}}}}"#,
        ))
        .await
        .expect("write unknown stream request");
    let stream_refused = matches!(
        next(&mut reader).await,
        Control::StreamResponse {
            id: 8,
            outcome: Outcome::Err(_)
        }
    );
    j.guard(
        "a stream request is refused the same way",
        stream_refused,
        json!({ "refused": stream_refused }),
    );

    j.step("send frames this server cannot read");
    for line in [
        "not json at all",
        r#"{"request":{"body":{"get_app_settings":{}}}}"#,
    ] {
        writer
            .write_bytes(&encode_control(line))
            .await
            .expect("write an unreadable frame");
    }
    writer
        .write_bytes(&encode_control(r#"{"request":{"id":10,"body":42}}"#))
        .await
        .expect("write a request with an unreadable body");
    let unreadable = match next(&mut reader).await {
        Control::Response {
            id,
            outcome: Outcome::Err(error),
        } => Some((id, error.code)),
        _ => None,
    };
    j.guard(
        "a frame with no id is skipped, and one with an id is answered with an error",
        unreadable
            .as_ref()
            .is_some_and(|(id, code)| *id == 10 && code == "unreadable_request"),
        json!({ "reply": unreadable }),
    );

    j.step("the same connection keeps serving");
    writer
        .write_bytes(&encode_control(
            r#"{"request":{"id":9,"body":{"get_app_settings":{}}}}"#,
        ))
        .await
        .expect("write known request");
    let answered = matches!(
        next(&mut reader).await,
        Control::Response {
            id: 9,
            outcome: Outcome::Ok(_)
        }
    );
    j.guard(
        "a known command on that connection still succeeds",
        answered,
        json!({ "answered": answered }),
    );
    let log = sandbox.server_log();
    let dropped = log.lines().any(|l| l.contains(": dropped after"));
    j.guard(
        "the server never drops the connection",
        !dropped,
        json!({ "dropped": dropped }),
    );

    j.step("an answer the client cannot read fails only that call");
    let socket = sandbox.root.join("stale-server.sock");
    let listener = UnixListener::bind(&socket).expect("bind the stale server");
    let stale = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept the client");
        let (rd, wr) = tokio::io::split(stream);
        let (mut reader, mut writer) = (FrameReader::new(rd), FrameWriter::new(wr));
        writer
            .write_control(&Control::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: None,
                version: None,
                features: Vec::new(),
                stream_window: None,
            })
            .await
            .expect("greet");
        let _ = reader.next_control().await;
        for answer in 0.. {
            let Ok(Some(Control::Request { id, .. })) = reader.next_control().await else {
                return;
            };
            if answer == 0 {
                let garbled = format!(
                    r#"{{"response":{{"id":{id},"outcome":{{"ok":{{"get_app_settings":"not settings"}}}}}}}}"#
                );
                writer
                    .write_bytes(&encode_control(&garbled))
                    .await
                    .expect("answer with a garbled response");
            } else {
                writer
                    .write_control(&Control::Response {
                        id,
                        outcome: Outcome::Ok(Response::ResolveReviewNote(())),
                    })
                    .await
                    .expect("answer normally");
            }
        }
    });
    let client = RpcClient::connect_socket(&socket)
        .await
        .expect("connect to the stale server");
    let garbled = client
        .call(Request::GetAppSettings {})
        .await
        .err()
        .map(|e| e.code);
    let after = client
        .call(Request::ResolveReviewNote {
            workstream_id: "w".into(),
            note_id: "n".into(),
            resolved: true,
        })
        .await;
    j.guard(
        "the unreadable answer fails its call and the next call still succeeds",
        garbled.as_deref() == Some("unreadable_response") && after.is_ok(),
        json!({ "garbled": garbled, "next_ok": after.is_ok() }),
    );
    stale.abort();

    j.finish();
}

/// The next control frame, skipping events the runtime broadcasts.
async fn next<R: tokio::io::AsyncRead + Unpin>(reader: &mut FrameReader<R>) -> Control {
    loop {
        let control = tokio::time::timeout(IO, reader.next_control())
            .await
            .expect("the server answers in time")
            .expect("read a frame")
            .expect("the server keeps the connection open");
        if !matches!(control, Control::Event { .. }) {
            return control;
        }
    }
}
