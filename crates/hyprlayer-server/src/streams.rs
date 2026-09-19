//! The streaming half of the wire (Phase 4 §1): a response carries a stream
//! id, and its events follow as tagged data-plane frames
//! (`frame::encode_stream_frame`). Outside the `commands!` table because
//! dispatching one needs the connection's outbound sender and stream
//! registry, which `lib.rs::read_requests` has and a plain `Dispatch` method
//! never sees.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use hyprlayer_core::runtime::scripts as core_scripts;
use hyprlayer_core::runtime::sessions::{
    LaunchAndConverseParams, LaunchSessionParams, RewindSessionRequest, SessionStreamError,
    StartStageSessionRequest,
};
use hyprlayer_core::runtime::terminals as core_terminals;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_core::{
    CoreError, Harness, HyprlayerRuntime, PermissionMode, ScriptKind, Session, SessionEvent,
    SessionLogEntry, Stage,
};
use hyprlayer_proto::{frame, WireError};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::client::{EventStream, RpcClient};
use crate::dispatch::{wire_error, Server, Wire};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamStartOpts {
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub prompt_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamRequest {
    TerminalOpen {
        workstream_id: String,
        cols: u16,
        rows: u16,
    },
    TerminalAttach {
        terminal_id: String,
    },
    SpawnCliTerminal {
        args: Vec<String>,
        cols: u16,
        rows: u16,
    },
    SpawnShellTerminal {
        command: String,
        cols: u16,
        rows: u16,
    },
    AgentOpen {
        workstream_id: String,
        command: String,
        args: Vec<String>,
        cols: u16,
        rows: u16,
    },
    ScriptRun {
        workstream_id: String,
        kind: ScriptKind,
        cols: u16,
        rows: u16,
    },
    StartStageSession {
        workstream_id: String,
        stage: Stage,
        input: String,
        attached_docs: Vec<String>,
        opts: StreamStartOpts,
    },
    ResumeSession {
        session_id: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
    },
    RestartSession {
        session_id: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
    },
    RewindSession {
        session_id: String,
        user_turn: u32,
        permission_mode: Option<PermissionMode>,
    },
    AttachSession {
        session_id: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamResponse {
    Terminal {
        terminal_id: String,
        stream_id: u64,
    },
    TerminalAttach {
        attached: bool,
        stream_id: Option<u64>,
    },
    Session {
        session: Session,
        stream_id: u64,
    },
    StartStageSession {
        session: Session,
        prompt: String,
        stream_id: u64,
    },
    RewindSession {
        session: Session,
        live_rewound: bool,
        stream_id: u64,
    },
    AttachSession {
        stream_id: u64,
    },
}

impl StreamRequest {
    pub fn command(&self) -> &'static str {
        match self {
            Self::TerminalOpen { .. } => "terminal_open",
            Self::TerminalAttach { .. } => "terminal_attach",
            Self::SpawnCliTerminal { .. } => "spawn_cli_terminal",
            Self::SpawnShellTerminal { .. } => "spawn_shell_terminal",
            Self::AgentOpen { .. } => "agent_open",
            Self::ScriptRun { .. } => "script_run",
            Self::StartStageSession { .. } => "start_stage_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::RestartSession { .. } => "restart_session",
            Self::RewindSession { .. } => "rewind_session",
            Self::AttachSession { .. } => "attach_session",
        }
    }
}

// Inputs and results carry prompts and command lines, so `{:?}` names the
// variant and nothing else.
impl std::fmt::Debug for StreamRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(self.command()).finish_non_exhaustive()
    }
}

impl std::fmt::Debug for StreamResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let variant = match self {
            Self::Terminal { .. } => "Terminal",
            Self::TerminalAttach { .. } => "TerminalAttach",
            Self::Session { .. } => "Session",
            Self::StartStageSession { .. } => "StartStageSession",
            Self::RewindSession { .. } => "RewindSession",
            Self::AttachSession { .. } => "AttachSession",
        };
        f.debug_struct(variant)
            .field("stream_id", &self.stream_id())
            .finish_non_exhaustive()
    }
}

impl StreamResponse {
    pub(crate) fn stream_id(&self) -> Option<u64> {
        match self {
            Self::Terminal { stream_id, .. }
            | Self::Session { stream_id, .. }
            | Self::StartStageSession { stream_id, .. }
            | Self::RewindSession { stream_id, .. }
            | Self::AttachSession { stream_id } => Some(*stream_id),
            Self::TerminalAttach { stream_id, .. } => *stream_id,
        }
    }
}

/// Per-server streaming state: the id counter (global, not per-connection —
/// uniqueness is all a client-facing stream id needs) and the launch
/// parameters `prepare_stream` stashes between `StartStageSession` opening the
/// stream and the follow-up `launch_and_converse` call actually running it
/// (the shell's fire-and-forget spawn, unchanged from before this phase).
#[derive(Default)]
pub(crate) struct StreamState {
    next_id: AtomicU64,
    pending_launches: Mutex<HashMap<String, LaunchAndConverseParams>>,
}

impl StreamState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

pub(crate) fn take_pending_launch(
    server: &Server,
    session_id: &str,
) -> Option<LaunchAndConverseParams> {
    server
        .streams()
        .pending_launches
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(session_id)
}

fn stash_pending_launch(server: &Server, session_id: String, params: LaunchAndConverseParams) {
    server
        .streams()
        .pending_launches
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session_id, params);
}

/// The abort handles of one connection's open streams. A dropped connection
/// (or an explicit `Cancel`) aborts the forwarder task directly — cooperative
/// cancellation, the same mechanism `SocketServer::run` already uses for
/// whole connections.
#[derive(Clone, Default)]
pub(crate) struct StreamRegistry {
    inner: Arc<Mutex<HashMap<u64, AbortHandle>>>,
}

impl StreamRegistry {
    fn insert(&self, id: u64, handle: AbortHandle) {
        self.lock().insert(id, handle);
    }

    pub(crate) fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }

    pub(crate) fn abort(&self, id: u64) {
        if let Some(handle) = self.lock().remove(&id) {
            handle.abort();
        }
    }

    pub(crate) fn abort_all(&self) {
        let open: Vec<_> = self.lock().drain().collect();
        if !open.is_empty() {
            tracing::debug!("aborting {} stream(s) with their connection", open.len());
        }
        for (_, handle) in open {
            handle.abort();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, AbortHandle>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) enum StreamSource {
    Terminal(mpsc::Receiver<TerminalEvent>),
    Session {
        session_id: String,
        /// Filled only by `attach_session`: the shell never replayed the log
        /// on start/resume/restart/rewind, and doing so duplicates the
        /// transcript a client restored before resuming.
        replay: Vec<SessionLogEntry>,
        live: mpsc::Receiver<Result<SessionEvent, SessionStreamError>>,
    },
}

async fn load_session(runtime: &HyprlayerRuntime, id: &str) -> Result<Session, CoreError> {
    runtime
        .sessions()
        .get(id)
        .await?
        .ok_or_else(|| CoreError::SessionNotFound {
            session_id: id.to_string(),
        })
}

/// Performs the streaming command's work and, on success, mints the stream id
/// its response carries. Does **not** start forwarding — the caller
/// (`lib.rs::read_requests`) must send the response first and only then call
/// `spawn_stream`, or a fast source could race a data frame ahead of the
/// response that names its stream id.
pub(crate) async fn prepare_stream(
    server: &Server,
    body: StreamRequest,
) -> Result<(StreamResponse, Option<StreamSource>), WireError> {
    let runtime = server.runtime();
    match body {
        StreamRequest::TerminalOpen {
            workstream_id,
            cols,
            rows,
        } => {
            let attachment = runtime
                .terminals()
                .open(core_terminals::OpenTerminalRequest {
                    workstream_id,
                    cols,
                    rows,
                })
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::Terminal {
                    terminal_id: attachment.terminal_id,
                    stream_id,
                },
                Some(StreamSource::Terminal(attachment.live)),
            ))
        }
        StreamRequest::TerminalAttach { terminal_id } => {
            let attachment = runtime
                .terminals()
                .attach(core_terminals::AttachTerminalRequest { terminal_id })
                .await
                .wire()?;
            match attachment {
                Some(attachment) => {
                    let stream_id = server.streams().next_id();
                    Ok((
                        StreamResponse::TerminalAttach {
                            attached: true,
                            stream_id: Some(stream_id),
                        },
                        Some(StreamSource::Terminal(attachment.live)),
                    ))
                }
                None => Ok((
                    StreamResponse::TerminalAttach {
                        attached: false,
                        stream_id: None,
                    },
                    None,
                )),
            }
        }
        StreamRequest::SpawnCliTerminal { args, cols, rows } => {
            let attachment = runtime
                .terminals()
                .open_cli(core_terminals::OpenCliTerminalRequest { args, cols, rows })
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::Terminal {
                    terminal_id: attachment.terminal_id,
                    stream_id,
                },
                Some(StreamSource::Terminal(attachment.live)),
            ))
        }
        StreamRequest::SpawnShellTerminal {
            command,
            cols,
            rows,
        } => {
            let attachment = runtime
                .terminals()
                .open_shell(core_terminals::OpenShellTerminalRequest {
                    command,
                    cols,
                    rows,
                })
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::Terminal {
                    terminal_id: attachment.terminal_id,
                    stream_id,
                },
                Some(StreamSource::Terminal(attachment.live)),
            ))
        }
        StreamRequest::AgentOpen {
            workstream_id,
            command,
            args,
            cols,
            rows,
        } => {
            let attachment = runtime
                .terminals()
                .open_agent(core_terminals::OpenAgentTerminalRequest {
                    workstream_id,
                    command,
                    args,
                    cols,
                    rows,
                })
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::Terminal {
                    terminal_id: attachment.terminal_id,
                    stream_id,
                },
                Some(StreamSource::Terminal(attachment.live)),
            ))
        }
        StreamRequest::ScriptRun {
            workstream_id,
            kind,
            cols,
            rows,
        } => {
            let run = runtime
                .scripts()
                .run(core_scripts::RunScriptRequest {
                    workstream_id,
                    kind,
                    cols,
                    rows,
                })
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::Terminal {
                    terminal_id: run.terminal_id,
                    stream_id,
                },
                Some(StreamSource::Terminal(run.live)),
            ))
        }
        StreamRequest::StartStageSession {
            workstream_id,
            stage,
            input,
            attached_docs,
            opts,
        } => {
            let workstream = runtime.workstreams().find(&workstream_id).await.wire()?;
            let StreamStartOpts {
                harness,
                model,
                effort,
                permission_mode,
                prompt_name,
            } = opts;
            let prepared = runtime
                .sessions()
                .prepare_start(StartStageSessionRequest {
                    workstream,
                    stage,
                    input,
                    attached_docs,
                    harness,
                    model,
                    effort,
                    permission_mode,
                    prompt_name,
                })
                .await
                .wire()?;
            let session = prepared.session.clone();
            let prompt = prepared.prompt.clone();
            let session_id = session.id.clone();
            let attachment = runtime.sessions().attach(&session_id).await.wire()?;
            stash_pending_launch(server, session_id.clone(), prepared.launch);
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::StartStageSession {
                    session,
                    prompt,
                    stream_id,
                },
                Some(StreamSource::Session {
                    session_id,
                    replay: Vec::new(),
                    live: attachment.live,
                }),
            ))
        }
        StreamRequest::ResumeSession {
            session_id,
            model,
            effort,
            permission_mode,
        } => {
            let session = load_session(&runtime, &session_id)
                .await
                .map_err(wire_error)?;
            let sessions = runtime.sessions();

            if sessions.is_live(&session_id).await {
                let running = sessions.permission_mode(&session_id).await;
                if permission_mode.is_none() || permission_mode == running {
                    let attachment = sessions.attach(&session_id).await.wire()?;
                    let stream_id = server.streams().next_id();
                    return Ok((
                        StreamResponse::Session { session, stream_id },
                        Some(StreamSource::Session {
                            session_id,
                            replay: Vec::new(),
                            live: attachment.live,
                        }),
                    ));
                }
                sessions.kill_and_await_exit(&session_id).await;
            }

            let workstream = runtime
                .workstreams()
                .find(&session.workstream_id)
                .await
                .wire()?;
            let attachment = sessions.attach(&session_id).await.wire()?;
            let stream_id = server.streams().next_id();
            sessions
                .launch(LaunchSessionParams {
                    session: session.clone(),
                    project_path: workstream.project_path,
                    model,
                    effort,
                    permission_mode,
                    resume: true,
                    boundary_on_relaunch: true,
                })
                .await
                .wire()?;
            Ok((
                StreamResponse::Session { session, stream_id },
                Some(StreamSource::Session {
                    session_id,
                    replay: Vec::new(),
                    live: attachment.live,
                }),
            ))
        }
        StreamRequest::RestartSession {
            session_id,
            model,
            effort,
            permission_mode,
        } => {
            let session = load_session(&runtime, &session_id)
                .await
                .map_err(wire_error)?;
            let sessions = runtime.sessions();
            sessions.kill_and_await_exit(&session_id).await;

            let workstream = runtime
                .workstreams()
                .find(&session.workstream_id)
                .await
                .wire()?;
            let attachment = sessions.attach(&session_id).await.wire()?;
            let stream_id = server.streams().next_id();
            sessions
                .launch(LaunchSessionParams {
                    session: session.clone(),
                    project_path: workstream.project_path,
                    model,
                    effort,
                    permission_mode,
                    resume: true,
                    boundary_on_relaunch: true,
                })
                .await
                .wire()?;
            Ok((
                StreamResponse::Session { session, stream_id },
                Some(StreamSource::Session {
                    session_id,
                    replay: Vec::new(),
                    live: attachment.live,
                }),
            ))
        }
        StreamRequest::RewindSession {
            session_id,
            user_turn,
            permission_mode,
        } => {
            let session = load_session(&runtime, &session_id)
                .await
                .map_err(wire_error)?;
            let workstream = runtime
                .workstreams()
                .find(&session.workstream_id)
                .await
                .wire()?;
            let sessions = runtime.sessions();
            let attachment = sessions.attach(&session_id).await.wire()?;
            let stream_id = server.streams().next_id();
            let outcome = sessions
                .rewind(RewindSessionRequest {
                    session,
                    workstream,
                    user_turn,
                    permission_mode,
                })
                .await
                .wire()?;
            Ok((
                StreamResponse::RewindSession {
                    session: outcome.session,
                    live_rewound: outcome.live_rewound,
                    stream_id,
                },
                Some(StreamSource::Session {
                    session_id,
                    replay: Vec::new(),
                    live: attachment.live,
                }),
            ))
        }
        StreamRequest::AttachSession { session_id } => {
            let attachment = runtime.sessions().attach(&session_id).await.wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::AttachSession { stream_id },
                Some(StreamSource::Session {
                    session_id,
                    replay: attachment.replay,
                    live: attachment.live,
                }),
            ))
        }
    }
}

/// Starts forwarding a prepared source and registers its abort handle. Must
/// only be called after the response naming `stream_id` is already enqueued
/// on `tx` (see `prepare_stream`'s doc comment).
pub(crate) fn spawn_stream(
    runtime: Arc<HyprlayerRuntime>,
    stream_id: u64,
    source: StreamSource,
    tx: mpsc::Sender<Vec<u8>>,
    registry: StreamRegistry,
) {
    let for_task = registry.clone();
    let handle = tokio::spawn(run_stream(runtime, stream_id, source, tx, for_task)).abort_handle();
    registry.insert(stream_id, handle);
}

async fn run_stream(
    runtime: Arc<HyprlayerRuntime>,
    stream_id: u64,
    source: StreamSource,
    tx: mpsc::Sender<Vec<u8>>,
    registry: StreamRegistry,
) {
    let ended = match source {
        StreamSource::Terminal(live) => forward_terminal(stream_id, live, &tx).await,
        StreamSource::Session {
            session_id,
            replay,
            live,
        } => forward_session(&runtime, stream_id, &session_id, replay, live, &tx).await,
    };
    tracing::debug!("stream {stream_id} ended: {ended}");
    registry.remove(stream_id);
}

/// Returns why the stream ended.
async fn forward_terminal(
    stream_id: u64,
    mut live: mpsc::Receiver<TerminalEvent>,
    tx: &mpsc::Sender<Vec<u8>>,
) -> &'static str {
    while let Some(event) = live.recv().await {
        if !send_event(tx, stream_id, &event).await {
            return "the client stopped reading";
        }
    }
    "the terminal closed"
}

/// Returns why the stream ended.
async fn forward_session(
    runtime: &HyprlayerRuntime,
    stream_id: u64,
    session_id: &str,
    replay: Vec<SessionLogEntry>,
    mut live: mpsc::Receiver<Result<SessionEvent, SessionStreamError>>,
    tx: &mpsc::Sender<Vec<u8>>,
) -> &'static str {
    for entry in replay {
        if !send_event(tx, stream_id, &entry.event).await {
            return "the client stopped reading during the replay";
        }
    }
    loop {
        match live.recv().await {
            Some(Ok(event)) => {
                let exited = matches!(event, SessionEvent::Exited { .. });
                if !send_event(tx, stream_id, &event).await {
                    return "the client stopped reading";
                }
                if exited {
                    return "the session exited";
                }
            }
            Some(Err(SessionStreamError::Lagged {
                epoch,
                last_persisted_seq,
            })) => {
                match runtime
                    .sessions()
                    .attach_after(session_id, epoch, last_persisted_seq)
                    .await
                {
                    Ok(resumed) => {
                        tracing::warn!(
                            "stream {stream_id}: session {session_id} lagged after seq \
                             {last_persisted_seq:?}; replaying {} events",
                            resumed.replay.len()
                        );
                        live = resumed.live;
                        for entry in resumed.replay {
                            if !send_event(tx, stream_id, &entry.event).await {
                                return "the client stopped reading during a lag replay";
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "stream {stream_id}: session {session_id} could not recover \
                             from a lag: {e}"
                        );
                        let _ = send_event(
                            tx,
                            stream_id,
                            &SessionEvent::ProtocolError {
                                detail: format!("could not recover from a lagged stream: {e}"),
                                raw: String::new(),
                            },
                        )
                        .await;
                        return "a lag could not be recovered";
                    }
                }
            }
            None => return "the session's event source closed",
        }
    }
}

async fn send_event(tx: &mpsc::Sender<Vec<u8>>, stream_id: u64, event: &impl Serialize) -> bool {
    match serde_json::to_vec(event) {
        Ok(bytes) => tx
            .send(frame::encode_stream_frame(stream_id, &bytes))
            .await
            .is_ok(),
        Err(e) => {
            tracing::error!("stream {stream_id}: event did not encode: {e}");
            false
        }
    }
}

fn expect_stream(
    rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    command: &str,
) -> Result<mpsc::UnboundedReceiver<Vec<u8>>, WireError> {
    rx.ok_or_else(|| WireError::protocol(format!("{command}: server opened no stream")))
}

fn unexpected(command: &str, response: &StreamResponse) -> WireError {
    WireError::protocol(format!("{command} was answered with {response:?}"))
}

/// The eleven streaming commands' client entry points. Hand-written, not
/// macro-generated — each one's post-processing (unwrap a `StreamResponse`
/// variant, wrap the receiver in a typed `EventStream`) differs too much for
/// a shared body. Each also returns the stream id, for `cancel_stream(id)`
/// once a caller stops reading.
impl RpcClient {
    pub async fn terminal_open_stream(
        &self,
        workstream_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, EventStream<TerminalEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::TerminalOpen {
                workstream_id,
                cols,
                rows,
            })
            .await?;
        match response {
            StreamResponse::Terminal {
                terminal_id,
                stream_id,
            } => Ok((
                terminal_id,
                stream_id,
                EventStream::new(expect_stream(rx, "terminal_open")?),
            )),
            other => Err(unexpected("terminal_open", &other)),
        }
    }

    pub async fn terminal_attach_stream(
        &self,
        terminal_id: String,
    ) -> Result<Option<(u64, EventStream<TerminalEvent>)>, WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::TerminalAttach { terminal_id })
            .await?;
        match response {
            StreamResponse::TerminalAttach {
                attached: true,
                stream_id: Some(stream_id),
            } => Ok(Some((
                stream_id,
                EventStream::new(expect_stream(rx, "terminal_attach")?),
            ))),
            StreamResponse::TerminalAttach {
                attached: false, ..
            } => Ok(None),
            other => Err(unexpected("terminal_attach", &other)),
        }
    }

    pub async fn spawn_cli_terminal_stream(
        &self,
        args: Vec<String>,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, EventStream<TerminalEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::SpawnCliTerminal { args, cols, rows })
            .await?;
        match response {
            StreamResponse::Terminal {
                terminal_id,
                stream_id,
            } => Ok((
                terminal_id,
                stream_id,
                EventStream::new(expect_stream(rx, "spawn_cli_terminal")?),
            )),
            other => Err(unexpected("spawn_cli_terminal", &other)),
        }
    }

    pub async fn spawn_shell_terminal_stream(
        &self,
        command: String,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, EventStream<TerminalEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::SpawnShellTerminal {
                command,
                cols,
                rows,
            })
            .await?;
        match response {
            StreamResponse::Terminal {
                terminal_id,
                stream_id,
            } => Ok((
                terminal_id,
                stream_id,
                EventStream::new(expect_stream(rx, "spawn_shell_terminal")?),
            )),
            other => Err(unexpected("spawn_shell_terminal", &other)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn agent_open_stream(
        &self,
        workstream_id: String,
        command: String,
        args: Vec<String>,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, EventStream<TerminalEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::AgentOpen {
                workstream_id,
                command,
                args,
                cols,
                rows,
            })
            .await?;
        match response {
            StreamResponse::Terminal {
                terminal_id,
                stream_id,
            } => Ok((
                terminal_id,
                stream_id,
                EventStream::new(expect_stream(rx, "agent_open")?),
            )),
            other => Err(unexpected("agent_open", &other)),
        }
    }

    pub async fn script_run_stream(
        &self,
        workstream_id: String,
        kind: ScriptKind,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, EventStream<TerminalEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::ScriptRun {
                workstream_id,
                kind,
                cols,
                rows,
            })
            .await?;
        match response {
            StreamResponse::Terminal {
                terminal_id,
                stream_id,
            } => Ok((
                terminal_id,
                stream_id,
                EventStream::new(expect_stream(rx, "script_run")?),
            )),
            other => Err(unexpected("script_run", &other)),
        }
    }

    pub async fn start_stage_session_stream(
        &self,
        workstream_id: String,
        stage: Stage,
        input: String,
        attached_docs: Vec<String>,
        opts: StreamStartOpts,
    ) -> Result<(Session, String, u64, EventStream<SessionEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::StartStageSession {
                workstream_id,
                stage,
                input,
                attached_docs,
                opts,
            })
            .await?;
        match response {
            StreamResponse::StartStageSession {
                session,
                prompt,
                stream_id,
            } => Ok((
                session,
                prompt,
                stream_id,
                EventStream::new(expect_stream(rx, "start_stage_session")?),
            )),
            other => Err(unexpected("start_stage_session", &other)),
        }
    }

    pub async fn resume_session_stream(
        &self,
        session_id: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<(Session, u64, EventStream<SessionEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::ResumeSession {
                session_id,
                model,
                effort,
                permission_mode,
            })
            .await?;
        match response {
            StreamResponse::Session { session, stream_id } => Ok((
                session,
                stream_id,
                EventStream::new(expect_stream(rx, "resume_session")?),
            )),
            other => Err(unexpected("resume_session", &other)),
        }
    }

    pub async fn restart_session_stream(
        &self,
        session_id: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<(Session, u64, EventStream<SessionEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::RestartSession {
                session_id,
                model,
                effort,
                permission_mode,
            })
            .await?;
        match response {
            StreamResponse::Session { session, stream_id } => Ok((
                session,
                stream_id,
                EventStream::new(expect_stream(rx, "restart_session")?),
            )),
            other => Err(unexpected("restart_session", &other)),
        }
    }

    pub async fn rewind_session_stream(
        &self,
        session_id: String,
        user_turn: u32,
        permission_mode: Option<PermissionMode>,
    ) -> Result<(Session, bool, u64, EventStream<SessionEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::RewindSession {
                session_id,
                user_turn,
                permission_mode,
            })
            .await?;
        match response {
            StreamResponse::RewindSession {
                session,
                live_rewound,
                stream_id,
            } => Ok((
                session,
                live_rewound,
                stream_id,
                EventStream::new(expect_stream(rx, "rewind_session")?),
            )),
            other => Err(unexpected("rewind_session", &other)),
        }
    }

    pub async fn attach_session_stream(
        &self,
        session_id: String,
    ) -> Result<(u64, EventStream<SessionEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::AttachSession { session_id })
            .await?;
        match response {
            StreamResponse::AttachSession { stream_id } => Ok((
                stream_id,
                EventStream::new(expect_stream(rx, "attach_session")?),
            )),
            other => Err(unexpected("attach_session", &other)),
        }
    }
}
