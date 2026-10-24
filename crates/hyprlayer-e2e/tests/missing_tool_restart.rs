//! The desktop leaves its server running when it quits, so a server that
//! started before a required tool was installed stops once its last client
//! leaves, unless work is still running on it, and the next one finds the tool.
#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_proto::frame::encode_control;
use hyprlayer_proto::PROTOCOL_VERSION;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::transport::{Control, FrameReader, FrameWriter};
use tokio::net::UnixStream;

/// The check after the last client leaves reprobes every binary first.
const DECIDED: Duration = Duration::from_secs(30);
const STAYING_UP: &str = "the last client left; staying up";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_tool_restart() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("missing_tool_restart", &sandbox);

    j.step("start a server without claude");
    let mut server = Server::start(&sandbox).await;
    let client = server.client();
    let statuses = redetect(&mut j, &client).await;
    j.guard(
        "the sandbox starts without claude",
        state_of(&statuses, "claude") == "missing",
        json!({ "claude": state_of(&statuses, "claude") }),
    );
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Missing tool").await;
    let ws = text(&view, "/item/id");

    j.step("a terminal keeps a server missing claude running");
    let (terminal, _, _events) = client
        .terminal_open_stream(ws, 80, 24)
        .await
        .expect("terminal_open");
    j.alias(&terminal, "$TERMINAL");
    let before = stayed(&sandbox);
    announce_and_leave(&sandbox.socket()).await;
    let decided = eventually(DECIDED, || stayed(&sandbox) > before).await;
    j.guard(
        "the server decides to stay up",
        decided,
        json!({ "decided": decided }),
    );
    let exited = server.exited(Duration::ZERO).await;
    j.guard(
        "the server is still running",
        exited.is_none(),
        json!({ "exit": exited.map(|s| s.code()) }),
    );

    j.step("with the terminal closed, it stops once its last client leaves");
    j.call(
        &client,
        Request::TerminalClose {
            terminal_id: terminal.clone(),
        },
    )
    .await
    .expect("terminal_close");
    announce_and_leave(&sandbox.socket()).await;
    let exited = server.exited(DECIDED).await;
    j.guard(
        "the server exits cleanly",
        exited.is_some_and(|s| s.success()),
        json!({ "exit": exited.map(|s| s.code()) }),
    );
    let log = sandbox.server_log();
    j.guard(
        "its log names the missing tool",
        log.contains("the last client left and claude is still missing"),
        json!({}),
    );
    let listening = RpcClient::connect_socket(&sandbox.socket()).await.is_ok();
    j.guard(
        "nothing listens on the socket",
        !listening,
        json!({ "listening": listening }),
    );

    j.step("the next server finds claude installed meanwhile and stays up");
    sandbox.stage_tool("claude", "#!/bin/sh\nexit 0\n");
    let mut server = Server::start(&sandbox).await;
    let client = server.client();
    let statuses = redetect(&mut j, &client).await;
    j.guard(
        "claude is found",
        state_of(&statuses, "claude") == "ok",
        json!({ "claude": state_of(&statuses, "claude") }),
    );
    let before = stayed(&sandbox);
    announce_and_leave(&sandbox.socket()).await;
    let decided = eventually(DECIDED, || stayed(&sandbox) > before).await;
    j.guard(
        "the server decides to stay up",
        decided,
        json!({ "decided": decided }),
    );
    let exited = server.exited(Duration::ZERO).await;
    j.guard(
        "the server is still running",
        exited.is_none(),
        json!({ "exit": exited.map(|s| s.code()) }),
    );

    j.finish();
}

async fn redetect(j: &mut Journey, client: &RpcClient) -> Value {
    j.call(client, Request::RedetectBinaries {})
        .await
        .expect("redetect_binaries")
}

fn state_of(statuses: &Value, name: &str) -> String {
    statuses
        .as_array()
        .and_then(|all| all.iter().find(|s| text(s, "/name") == name))
        .map(|s| text(s, "/state/state"))
        .unwrap_or_default()
}

fn stayed(sandbox: &Sandbox) -> usize {
    sandbox.server_log().matches(STAYING_UP).count()
}

/// The desktop's lifetime in miniature: connect, announce, quit. A raw socket,
/// because a dropped `RpcClient` keeps its connection open.
async fn announce_and_leave(socket: &Path) {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (rd, wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut writer = FrameWriter::new(wr);
    let hello = reader.next_control().await.expect("read hello");
    assert!(matches!(hello, Some(Control::Hello { .. })), "{hello:?}");
    writer
        .write_control(&Control::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: None,
            version: None,
        })
        .await
        .expect("hello");
    writer
        .write_bytes(&encode_control(
            r#"{"request":{"id":1,"body":{"announce_client":{"client_id":"desktop","device":"e2e","capabilities":[]}}}}"#,
        ))
        .await
        .expect("announce");
    loop {
        match reader
            .next_control()
            .await
            .expect("read the announce reply")
        {
            Some(Control::Response { id: 1, .. }) => return,
            Some(_) => continue,
            None => panic!("the server closed before answering the announce"),
        }
    }
}
