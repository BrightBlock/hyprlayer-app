//! The perf harness's fixture: a sandboxed server with one project, one workstream holding a
//! long transcript and four live shells. Prints one JSON line, then serves until killed.
#[cfg(not(unix))]
fn main() {
    eprintln!("perf_fixture needs a unix sandbox");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() {
    fixture::run();
}

#[cfg(unix)]
mod fixture {
    use std::time::Duration;

    use hyprlayer_core::{Harness, SessionEvent, Stage};
    use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
    use hyprlayer_e2e::{json, text, Journey, Sandbox, Server};
    use hyprlayer_server::streams::{
        SessionEvents, SessionFrame, StreamRequest, StreamResponse, StreamStartOpts,
    };

    const IO: Duration = Duration::from_secs(120);
    const STEPS_PER_TURN: usize = 125;
    const TERMINALS: usize = 4;

    /// Replays `<sandbox>/turn.jsonl` for every message it reads; PERF_STREAM paces text deltas instead.
    const FAKE_CLAUDE: &str = r#"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cfg="$(dirname "$0")/.."
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-perf","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  case "$line" in
    *PERF_STREAM*)
      i=0
      while [ $i -lt 400 ]; do
        printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"tok%s "}},"session_id":"%s","parent_tool_use_id":null}\n' "$i" "$sid"
        i=$((i + 1))
        sleep 0.025
      done
      printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"streamed PERF_STREAM_END"}]},"session_id":"%s"}\n' "$sid"
      printf '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"%s","num_turns":1,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4}}\n' "$sid"
      continue ;;
  esac
  sed -e "s/%SID%/$sid/g" -e "s/%TURN%/$turn/g" "$cfg/turn.jsonl"
done
"#;

    fn turn() -> String {
        let mut out = String::new();
        for n in 0..STEPS_PER_TURN {
            let id = format!("toolu_%TURN%_{n}");
            let prose = format!("step {n} reads the next file. ").repeat(12);
            let code = format!("fn step_{n}() -> u64 {{ {n} }}\n").repeat(12);
            for line in [
                json!({"type":"assistant","message":{"role":"assistant","content":[
                {"type":"text","text":prose},
                {"type":"tool_use","id":id,"name":"Read","input":{"file_path":format!("src/file_{n}.rs")}}]},
                "session_id":"%SID%"}),
                json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":id,"content":code,"is_error":false}]},
                "session_id":"%SID%"}),
            ] {
                out += &(line.to_string() + "\n");
            }
        }
        out + &json!({"type":"result","subtype":"success","is_error":false,"result":"done",
        "session_id":"%SID%","num_turns":1,"total_cost_usd":0.01,
        "usage":{"input_tokens":11,"output_tokens":4}})
        .to_string()
            + "\n"
    }

    async fn until_turn_done(events: &mut SessionEvents) {
        let done = tokio::time::timeout(IO, async {
            while let Some(frame) = events.recv().await {
                let event = match frame {
                    SessionFrame::Item(item) => item.event,
                    SessionFrame::Bare(event) => event,
                };
                if matches!(event, SessionEvent::TurnDone { .. }) {
                    return true;
                }
            }
            false
        })
        .await;
        assert_eq!(done, Ok(true), "the turn never finished");
    }

    #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
    pub async fn run() {
        let turns: usize = std::env::args()
            .nth(1)
            .and_then(|n| n.parse().ok())
            .unwrap_or(6);
        let sandbox = Sandbox::new();
        sandbox.stage_tool("claude", FAKE_CLAUDE);
        std::fs::write(sandbox.root.join("turn.jsonl"), turn()).expect("write the turn");
        let repo = sandbox.fixture_repo("perf-demo", &[("README.md", "# perf demo\n")]);
        let mut j = Journey::new("perf_fixture", &sandbox);
        let server = Server::start(&sandbox).await;
        let desktop = server.client();
        let project = onboard_and_import(&mut j, &desktop, &sandbox, &repo).await;
        let view = create_workstream(&mut j, &desktop, &project, "Perf").await;
        let ws = text(&view, "/item/id");

        let (response, _, mut events) = desktop
            .open_session_stream(StreamRequest::StartStageSession {
                workstream_id: ws.clone(),
                stage: Stage::General,
                input: "read".into(),
                attached_docs: vec![],
                opts: StreamStartOpts {
                    harness: Some(Harness::Claude),
                    ..Default::default()
                },
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
            })
            .await
            .expect("start_stage_session");
        let StreamResponse::StartStageSession { session, .. } = response else {
            panic!("start_stage_session answered with another response");
        };
        let sid = session.id.clone();
        desktop
            .launch_and_converse(sid.clone())
            .await
            .expect("launch_and_converse");
        until_turn_done(&mut events).await;
        for _ in 1..turns {
            desktop
                .send_user_message(sid.clone(), "again".into(), vec![], None, None, None)
                .await
                .expect("send_user_message");
            until_turn_done(&mut events).await;
        }
        let log_rows = desktop
            .get_session_log(sid.clone())
            .await
            .expect("get_session_log")
            .len();

        let big = sandbox.root.join("big.txt");
        {
            use std::io::Write;
            let mut out =
                std::io::BufWriter::new(std::fs::File::create(&big).expect("create big.txt"));
            let mut n = 0u64;
            while out.get_ref().metadata().map_or(0, |m| m.len()) < 50 * 1024 * 1024 {
                for _ in 0..4096 {
                    writeln!(
                        out,
                        "perf line {n} the quick brown fox jumps over the lazy dog"
                    )
                    .expect("write big.txt");
                    n += 1;
                }
                out.flush().expect("flush big.txt");
            }
        }

        // Held open: a terminal's stream is what keeps its end of the pty attached.
        let mut terminals = Vec::new();
        for _ in 0..TERMINALS {
            let (id, _, stream) = desktop
                .terminal_open_stream(ws.clone(), 200, 50)
                .await
                .expect("terminal_open");
            terminals.push((id, stream));
        }

        let report = json!({
            "socket": server.socket(),
            "sandbox": sandbox.root,
            "appData": sandbox.app_data,
            "appConfig": sandbox.app_config,
            "bigFile": big,
            "serverPid": server.pid(),
            "project": project,
            "workstreamId": ws,
            "sessionId": sid,
            "logRows": log_rows,
            "terminals": terminals.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        });
        println!("{report}");
        // Drain the terminals so their rings don't back up while the clients measure.
        for (_, mut stream) in terminals {
            tokio::spawn(async move { while stream.recv().await.is_some() {} });
        }
        // The harness ends the fixture by creating this file, so the server stops with it.
        let stop = sandbox.root.join("stop");
        while !stop.exists() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        drop(j);
    }
}
