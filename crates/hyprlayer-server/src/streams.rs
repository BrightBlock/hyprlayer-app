//! The streaming half of the wire (Phase 4 §1): a response carries a stream
//! id, and its events follow as tagged data-plane frames
//! (`frame::encode_stream_frame`). Outside the `commands!` table because
//! dispatching one needs the connection's outbound sender and stream
//! registry, which `lib.rs::read_requests` has and a plain `Dispatch` method
//! never sees.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_core::runtime::scripts as core_scripts;
use hyprlayer_core::runtime::sessions::{
    LaunchAndConverseParams, LaunchSessionParams, LiveReceiver, RewindSessionRequest,
    SessionStreamError, StartStageSessionRequest,
};
use hyprlayer_core::runtime::terminals as core_terminals;
use hyprlayer_core::terminal::{RingReader, TerminalChunk};
use hyprlayer_core::{
    CoreError, Harness, HyprlayerRuntime, LogCursor, PermissionMode, ScriptKind, Session,
    SessionEvent, SessionLogEntry, Stage,
};
use hyprlayer_models::{ClientRef, PtyLaunchSession, PtyLaunched, SessionStreamItem};
use hyprlayer_proto::{frame, WireError, STREAM_WINDOW};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::client::{EventStream, RpcClient, StreamReceiver, TerminalStream};
use crate::dispatch::{wire_error, Server, Wire};
use crate::events::{Decoder, StreamedRows};
use crate::outbound::{Credit, Outbound, PART_BYTES};

/// What a stream may have queued in the writer: a turn's worth ready behind the one going out.
const STREAM_QUEUE_BYTES: i64 = 2 * PART_BYTES as i64;
/// Raw output per terminal frame, so one fits a part and needs no lane.
const TERMINAL_READ_BYTES: usize = PART_BYTES - frame::TERMINAL_OUTPUT_HEADER;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamStartOpts {
    pub harness: Option<Harness>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub prompt_name: Option<String>,
    pub client_message_id: Option<String>,
    /// An older server drops it unread, so the session just starts unlinked.
    #[serde(default)]
    pub parent_session_id: Option<String>,
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
        /// Resume after this many bytes of output instead of replaying the scrollback.
        #[serde(default)]
        offset: Option<u64>,
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
    /// The server builds the argv; `AgentOpen` stays for released desktops.
    PtyLaunch {
        workstream_id: String,
        engine_id: String,
        session: PtyLaunchSession,
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
        #[serde(default)]
        echo_user_messages: bool,
        #[serde(default)]
        with_meta: bool,
        /// Also forward partial text this client does not render.
        #[serde(default)]
        all_partials: bool,
    },
    ResumeSession {
        session_id: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
        cursor: Option<LogCursor>,
        #[serde(default)]
        echo_user_messages: bool,
        #[serde(default)]
        with_meta: bool,
        #[serde(default)]
        all_partials: bool,
    },
    RestartSession {
        session_id: String,
        model: Option<String>,
        effort: Option<String>,
        permission_mode: Option<PermissionMode>,
        #[serde(default)]
        echo_user_messages: bool,
        #[serde(default)]
        with_meta: bool,
        #[serde(default)]
        all_partials: bool,
    },
    RewindSession {
        session_id: String,
        user_turn: u32,
        permission_mode: Option<PermissionMode>,
        #[serde(default)]
        echo_user_messages: bool,
        #[serde(default)]
        with_meta: bool,
        #[serde(default)]
        all_partials: bool,
    },
    AttachSession {
        session_id: String,
        #[serde(default)]
        echo_user_messages: bool,
        #[serde(default)]
        with_meta: bool,
        #[serde(default)]
        all_partials: bool,
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
        /// The ring held the requested offset at open; a trim can still beat the first read, so output offsets decide.
        #[serde(default)]
        resumed: bool,
    },
    Session {
        session: Session,
        stream_id: u64,
        #[serde(default)]
        permission_mode: Option<PermissionMode>,
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
    PtyLaunch {
        launched: PtyLaunched,
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
            Self::PtyLaunch { .. } => "pty_launch",
            Self::ScriptRun { .. } => "script_run",
            Self::StartStageSession { .. } => "start_stage_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::RestartSession { .. } => "restart_session",
            Self::RewindSession { .. } => "rewind_session",
            Self::AttachSession { .. } => "attach_session",
        }
    }

    /// `Some` for the session streams, which can carry `SessionStreamItem`s.
    pub fn with_meta(&self) -> Option<bool> {
        match self {
            Self::StartStageSession { with_meta, .. }
            | Self::ResumeSession { with_meta, .. }
            | Self::RestartSession { with_meta, .. }
            | Self::RewindSession { with_meta, .. }
            | Self::AttachSession { with_meta, .. } => Some(*with_meta),
            _ => None,
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
            Self::PtyLaunch { .. } => "PtyLaunch",
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
            | Self::AttachSession { stream_id }
            | Self::PtyLaunch { stream_id, .. } => Some(*stream_id),
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
    pending_launches: Mutex<PendingLaunches>,
}

/// One lock, so a stash and a connection's close can't miss each other.
#[derive(Default)]
struct PendingLaunches {
    /// Connections whose starts can still be launched.
    open: HashSet<u64>,
    /// Keyed by session; the owner is `None` once its launch call has arrived.
    launches: HashMap<String, (Option<u64>, LaunchAndConverseParams)>,
}

const ABANDONED: &str = "The app disconnected before it launched the agent.";

impl StreamState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

fn pending(server: &Server) -> std::sync::MutexGuard<'_, PendingLaunches> {
    server
        .streams()
        .pending_launches
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn take_pending_launch(
    server: &Server,
    session_id: &str,
) -> Option<LaunchAndConverseParams> {
    pending(server)
        .launches
        .remove(session_id)
        .map(|(_, params)| params)
}

/// The launch call has arrived, so its connection closing must not fail the start.
pub(crate) fn claim_pending_launch(server: &Server, session_id: &str) {
    if let Some((owner, _)) = pending(server).launches.get_mut(session_id) {
        *owner = None;
    }
}

pub(crate) fn open_connection(server: &Server, conn: u64) {
    pending(server).open.insert(conn);
}

/// `Some` hands the start back when its connection has already closed.
fn stash_pending_launch(
    server: &Server,
    session_id: String,
    params: LaunchAndConverseParams,
) -> Option<LaunchAndConverseParams> {
    let conn = crate::CONN.try_with(|conn| *conn).ok();
    let mut pending = pending(server);
    if conn.is_some_and(|conn| !pending.open.contains(&conn)) {
        return Some(params);
    }
    pending.launches.insert(session_id, (conn, params));
    None
}

/// Starts a closed connection prepared but never launched end as failed starts, not `starting` rows.
pub(crate) async fn fail_abandoned_launches(server: &Server, conn: u64) {
    let abandoned: Vec<(String, LaunchAndConverseParams)> = {
        let mut pending = pending(server);
        pending.open.remove(&conn);
        let ids: Vec<String> = pending
            .launches
            .iter()
            .filter(|(_, (owner, _))| *owner == Some(conn))
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| pending.launches.remove(&id).map(|(_, params)| (id, params)))
            .collect()
    };
    let runtime = server.runtime();
    let sessions = runtime.sessions();
    for (session_id, params) in abandoned {
        tracing::warn!("session {session_id}: its client left before launching it");
        sessions.fail_unlaunched(params, ABANDONED).await;
    }
}

/// The abort handles of one connection's open streams. A dropped connection
/// (or an explicit `Cancel`) aborts the forwarder task directly — cooperative
/// cancellation, the same mechanism `SocketServer::run` already uses for
/// whole connections.
#[derive(Clone, Default)]
pub(crate) struct StreamRegistry {
    inner: Arc<Mutex<HashMap<u64, Entry>>>,
}

enum Entry {
    /// Named in a response the client may already hold, not yet forwarding.
    Reserved {
        cancelled: bool,
    },
    Open(AbortHandle, Arc<Credit>),
}

impl StreamRegistry {
    pub(crate) fn reserve(&self, id: u64) {
        self.lock().insert(id, Entry::Reserved { cancelled: false });
    }

    pub(crate) fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }

    pub(crate) fn abort(&self, id: u64) {
        let mut open = self.lock();
        match open.get_mut(&id) {
            Some(Entry::Reserved { cancelled }) => *cancelled = true,
            Some(Entry::Open(handle, _)) => {
                handle.abort();
                open.remove(&id);
            }
            None => {}
        }
    }

    pub(crate) fn grant(&self, id: u64, bytes: u64) {
        if let Some(Entry::Open(_, credit)) = self.lock().get(&id) {
            credit.grant(bytes);
        }
    }

    pub(crate) fn abort_all(&self) {
        let open: Vec<_> = self.lock().drain().collect();
        if !open.is_empty() {
            tracing::debug!("aborting {} stream(s) with their connection", open.len());
        }
        for (_, entry) in open {
            if let Entry::Open(handle, _) = entry {
                handle.abort();
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Entry>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One stream's way onto its connection: no more than `STREAM_QUEUE_BYTES`
/// queued in the writer, and no more unread at the client than its credit.
pub(crate) struct StreamTx {
    outbound: Outbound,
    stream_id: u64,
    queue: Arc<Credit>,
    /// Event bytes the client still has room for.
    credit: Arc<Credit>,
}

pub(crate) struct Ready<'a> {
    tx: &'a StreamTx,
}

impl StreamTx {
    /// Waits until the writer and the client both have room for more of this stream.
    async fn ready(&self) -> Option<Ready<'_>> {
        let room = async {
            self.queue.available().await;
            self.credit.available().await;
        };
        tokio::select! {
            () = room => {}
            () = self.outbound.closed() => return None,
        }
        (!self.outbound.is_closed()).then_some(Ready { tx: self })
    }
}

impl Ready<'_> {
    fn send(self, stream_id: u64, event: &impl Serialize) -> bool {
        match serde_json::to_vec(event) {
            Ok(bytes) => self.push(frame::encode_stream_frame(stream_id, &bytes)),
            Err(e) => {
                tracing::error!("stream {stream_id}: event did not encode: {e}");
                false
            }
        }
    }

    // Credit counts the event as it goes out, everything after the stream id, as the client does.
    fn push(self, frame: Vec<u8>) -> bool {
        let tx = self.tx;
        tx.credit.take(frame.len() - frame::STREAM_FRAME_HEADER);
        tx.queue.take(frame.len());
        tx.outbound.push_data(tx.stream_id, frame, tx.queue.clone())
    }
}

/// A terminal event as a client without `RAW_TERMINAL` reads it: base64 in JSON.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum JsonTerminalEvent {
    Output { data: String },
    Exited { code: Option<i32> },
}

pub(crate) enum StreamSource {
    Terminal(RingReader),
    Session {
        session_id: String,
        /// Filled by `attach_session`, and by a resume that names the row
        /// its client folded last.
        replay: Vec<SessionLogEntry>,
        live: LiveReceiver,
        /// Carried into a lag recovery's re-attach, so it keeps this stream's opt-in.
        echo_user_messages: bool,
        with_meta: bool,
        partials: Partials,
        decoder: Decoder,
        /// The session's rev once `replay` is applied.
        rev: u64,
    },
}

impl StreamSource {
    /// The session this stream carries every row and author of, as its revisions would.
    pub(crate) fn rows_of(&self) -> Option<&str> {
        match self {
            StreamSource::Session {
                session_id,
                with_meta: true,
                echo_user_messages: true,
                ..
            } => Some(session_id),
            _ => None,
        }
    }
}

/// Which partial text a session stream forwards.
pub(crate) enum Partials {
    All,
    /// Only what this announced client renders; its route drops the rest on arrival.
    RenderedBy(String),
}

impl Partials {
    /// A bare stream carries no author, so its client renders every main-thread partial.
    fn for_stream(server: &Server, with_meta: bool, all_partials: bool) -> Self {
        match server.caller() {
            Some(me) if with_meta && !all_partials => Partials::RenderedBy(me.client_id),
            _ => Partials::All,
        }
    }

    /// Mirrors the desktop: subagent text is never shown, an agent's turn applies like its own.
    fn forwards(&self, author: Option<&ClientRef>, parent_id: Option<&str>) -> bool {
        match self {
            Partials::All => true,
            Partials::RenderedBy(me) => {
                parent_id.is_none() && author.is_none_or(|a| a.is_agent() || a.client_id == *me)
            }
        }
    }
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
    if server.agent_token().is_some() {
        return Err(wire_error(CoreError::AgentControlDenied {
            detail: format!(
                "{} is a stream, and streams are not open to agents",
                body.command()
            ),
        }));
    }
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
        StreamRequest::TerminalAttach {
            terminal_id,
            offset,
        } => {
            let attachment = runtime
                .terminals()
                .attach(core_terminals::AttachTerminalRequest {
                    terminal_id,
                    offset,
                })
                .await
                .wire()?;
            match attachment {
                Some(attachment) => {
                    let stream_id = server.streams().next_id();
                    Ok((
                        StreamResponse::TerminalAttach {
                            attached: true,
                            stream_id: Some(stream_id),
                            resumed: attachment.resumed,
                        },
                        Some(StreamSource::Terminal(attachment.live)),
                    ))
                }
                None => Ok((
                    StreamResponse::TerminalAttach {
                        attached: false,
                        stream_id: None,
                        resumed: false,
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
        StreamRequest::PtyLaunch {
            workstream_id,
            engine_id,
            session,
            cols,
            rows,
        } => {
            let (attachment, launched) = runtime
                .terminals()
                .launch_agent(core_terminals::LaunchAgentRequest {
                    workstream_id,
                    engine_id,
                    session,
                    cols,
                    rows,
                    lineage: None,
                })
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::PtyLaunch {
                    launched,
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
            echo_user_messages,
            with_meta,
            all_partials,
        } => {
            let workstream = runtime.workstreams().find(&workstream_id).await.wire()?;
            let StreamStartOpts {
                harness,
                model,
                effort,
                permission_mode,
                prompt_name,
                client_message_id,
                parent_session_id,
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
                    client_message_id,
                    parent_session_id,
                })
                .await
                .wire()?;
            let session = prepared.session.clone();
            let prompt = prepared.prompt.clone();
            let session_id = session.id.clone();
            let attachment = runtime
                .sessions()
                .attach(&session_id, echo_user_messages)
                .await
                .wire()?;
            if let Some(launch) = stash_pending_launch(server, session_id.clone(), prepared.launch)
            {
                tracing::warn!("session {session_id}: its client left while it was prepared");
                runtime.sessions().fail_unlaunched(launch, ABANDONED).await;
            }
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
                    echo_user_messages,
                    with_meta,
                    partials: Partials::for_stream(server, with_meta, all_partials),
                    decoder: server.decoder(),
                    rev: attachment.rev,
                }),
            ))
        }
        StreamRequest::ResumeSession {
            session_id,
            model,
            effort,
            permission_mode,
            cursor,
            echo_user_messages,
            with_meta,
            all_partials,
        } => {
            let session = load_session(&runtime, &session_id)
                .await
                .map_err(wire_error)?;
            let sessions = runtime.sessions();
            // Two clients resuming an exited session at once would each spawn a
            // harness on the same conversation; the second must see the first's.
            let op = sessions.op_lock(&session_id).await;
            let _resuming = op.lock().await;

            if sessions.is_live(&session_id).await {
                // Another client may be mid-turn; changing modes is restart_session's job.
                let (replay, live, rev) = match cursor {
                    Some(c) => {
                        let a = sessions
                            .attach_from(&session_id, c, echo_user_messages)
                            .await
                            .wire()?;
                        (a.replay, a.live, a.rev)
                    }
                    None => {
                        let a = sessions
                            .attach(&session_id, echo_user_messages)
                            .await
                            .wire()?;
                        (Vec::new(), a.live, a.rev)
                    }
                };
                let running = sessions.permission_mode(&session_id).await;
                let stream_id = server.streams().next_id();
                return Ok((
                    StreamResponse::Session {
                        session,
                        stream_id,
                        permission_mode: running,
                    },
                    Some(StreamSource::Session {
                        session_id,
                        replay,
                        live,
                        echo_user_messages,
                        with_meta,
                        partials: Partials::for_stream(server, with_meta, all_partials),
                        decoder: server.decoder(),
                        rev,
                    }),
                ));
            }

            let workstream = runtime
                .workstreams()
                .find(&session.workstream_id)
                .await
                .wire()?;
            // The relaunch replays too, for uniformity: a session that exited in
            // the gap between the client's log read and this resume ends with
            // `Exited` on the stream rather than silently vanishing.
            let (replay, live, rev) = match cursor {
                Some(c) => {
                    let a = sessions
                        .attach_from(&session_id, c, echo_user_messages)
                        .await
                        .wire()?;
                    (a.replay, a.live, a.rev)
                }
                None => {
                    let a = sessions
                        .attach(&session_id, echo_user_messages)
                        .await
                        .wire()?;
                    (Vec::new(), a.live, a.rev)
                }
            };
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
                    max_budget_usd: None,
                    extra_allowed_tools: Vec::new(),
                    extra_disallowed_tools: Vec::new(),
                    report_only: false,
                })
                .await
                .wire()?;
            let running = sessions.permission_mode(&session_id).await;
            Ok((
                StreamResponse::Session {
                    session,
                    stream_id,
                    permission_mode: running,
                },
                Some(StreamSource::Session {
                    session_id,
                    replay,
                    live,
                    echo_user_messages,
                    with_meta,
                    partials: Partials::for_stream(server, with_meta, all_partials),
                    decoder: server.decoder(),
                    rev,
                }),
            ))
        }
        StreamRequest::RestartSession {
            session_id,
            model,
            effort,
            permission_mode,
            echo_user_messages,
            with_meta,
            all_partials,
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
            let attachment = sessions
                .attach(&session_id, echo_user_messages)
                .await
                .wire()?;
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
                    max_budget_usd: None,
                    extra_allowed_tools: Vec::new(),
                    extra_disallowed_tools: Vec::new(),
                    report_only: false,
                })
                .await
                .wire()?;
            let running = sessions.permission_mode(&session_id).await;
            Ok((
                StreamResponse::Session {
                    session,
                    stream_id,
                    permission_mode: running,
                },
                Some(StreamSource::Session {
                    session_id,
                    replay: Vec::new(),
                    live: attachment.live,
                    echo_user_messages,
                    with_meta,
                    partials: Partials::for_stream(server, with_meta, all_partials),
                    decoder: server.decoder(),
                    rev: attachment.rev,
                }),
            ))
        }
        StreamRequest::RewindSession {
            session_id,
            user_turn,
            permission_mode,
            echo_user_messages,
            with_meta,
            all_partials,
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
            let attachment = sessions
                .attach(&session_id, echo_user_messages)
                .await
                .wire()?;
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
                    echo_user_messages,
                    with_meta,
                    partials: Partials::for_stream(server, with_meta, all_partials),
                    decoder: server.decoder(),
                    rev: attachment.rev,
                }),
            ))
        }
        StreamRequest::AttachSession {
            session_id,
            echo_user_messages,
            with_meta,
            all_partials,
        } => {
            let attachment = runtime
                .sessions()
                .attach(&session_id, echo_user_messages)
                .await
                .wire()?;
            let stream_id = server.streams().next_id();
            Ok((
                StreamResponse::AttachSession { stream_id },
                Some(StreamSource::Session {
                    session_id,
                    replay: attachment.replay,
                    live: attachment.live,
                    echo_user_messages,
                    with_meta,
                    partials: Partials::for_stream(server, with_meta, all_partials),
                    decoder: server.decoder(),
                    rev: attachment.rev,
                }),
            ))
        }
    }
}

/// Starts forwarding a prepared source and registers its abort handle. Must
/// only be called after the response naming `stream_id` is written
/// (see `prepare_stream`'s doc comment). `None` when a cancel got there first.
/// `rows` is held until the stream ends, however it ends.
pub(crate) fn spawn_stream(
    runtime: Arc<HyprlayerRuntime>,
    stream_id: u64,
    source: StreamSource,
    outbound: Outbound,
    registry: StreamRegistry,
    rows: Option<StreamedRows>,
) -> Option<AbortHandle> {
    // A client without credit gets the old unbounded window; its slot still bounds the server.
    let window = if outbound.flow_control() {
        STREAM_WINDOW as i64
    } else {
        i64::MAX
    };
    let credit = Arc::new(Credit::new(window));
    let tx = StreamTx {
        outbound,
        stream_id,
        queue: Arc::new(Credit::new(STREAM_QUEUE_BYTES)),
        credit: credit.clone(),
    };
    let for_task = registry.clone();
    // One lock across the check and the insert, so a cancel lands on one side of it.
    let mut open = registry.lock();
    if let Some(Entry::Reserved { cancelled: true }) = open.get(&stream_id) {
        open.remove(&stream_id);
        return None;
    }
    let task = tokio::spawn(async move {
        let _rows = rows;
        run_stream(runtime, stream_id, source, tx, for_task).await
    });
    open.insert(stream_id, Entry::Open(task.abort_handle(), credit));
    Some(task.abort_handle())
}

async fn run_stream(
    runtime: Arc<HyprlayerRuntime>,
    stream_id: u64,
    source: StreamSource,
    tx: StreamTx,
    registry: StreamRegistry,
) {
    let ended = match source {
        StreamSource::Terminal(live) => forward_terminal(stream_id, live, &tx).await,
        StreamSource::Session {
            session_id,
            replay,
            live,
            echo_user_messages,
            with_meta,
            partials,
            decoder,
            rev,
        } => {
            if send_replay(&tx, stream_id, replay, rev, with_meta, &decoder).await {
                let sink = SessionSink::new(&tx, stream_id, with_meta, partials, decoder);
                forward_session(&runtime, &session_id, live, echo_user_messages, sink).await
            } else {
                "the client stopped reading during the replay"
            }
        }
    };
    tracing::debug!("stream {stream_id} ended: {ended}");
    registry.remove(stream_id);
}

/// Returns why the stream ended. Waits for room before reading, so output piles
/// up in the terminal's ring, which coalesces it and drops the oldest.
async fn forward_terminal(stream_id: u64, mut live: RingReader, tx: &StreamTx) -> &'static str {
    let raw = tx.outbound.raw_terminal();
    loop {
        let Some(ready) = tx.ready().await else {
            return "the client stopped reading";
        };
        let Some(chunk) = live.recv_at_most(TERMINAL_READ_BYTES).await else {
            return "the terminal closed";
        };
        let sent = match chunk {
            TerminalChunk::Output { offset, bytes } if raw => ready.push(
                frame::encode_terminal_output_frame(stream_id, offset, &bytes),
            ),
            TerminalChunk::Output { bytes, .. } => ready.send(
                stream_id,
                &JsonTerminalEvent::Output {
                    data: B64.encode(bytes),
                },
            ),
            TerminalChunk::Exited { code } => {
                ready.send(stream_id, &JsonTerminalEvent::Exited { code })
            }
        };
        if !sent {
            return "the client stopped reading";
        }
    }
}

/// Returns why the stream ended.
async fn forward_session(
    runtime: &HyprlayerRuntime,
    session_id: &str,
    mut live: LiveReceiver,
    echo_user_messages: bool,
    mut sink: SessionSink<'_>,
) -> &'static str {
    let (tx, stream_id, with_meta) = (sink.tx, sink.stream_id, sink.with_meta);
    let decoder = sink.decoder.clone();
    loop {
        let next = match sink.flush_at() {
            Some(at) => tokio::select! {
                next = live.recv_item() => next,
                () = tokio::time::sleep_until(at) => {
                    if !sink.flush().await {
                        return "the client stopped reading";
                    }
                    continue;
                }
            },
            None => live.recv_item().await,
        };
        match next {
            Some(Ok(item)) => {
                let exited = matches!(item.event, SessionEvent::Exited { .. });
                if !sink.send(item).await {
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
                // The re-attach seeds nothing, so held text not sent now is lost.
                if !sink.flush().await {
                    return "the client stopped reading";
                }
                match runtime
                    .sessions()
                    .attach_after(session_id, epoch, last_persisted_seq, echo_user_messages)
                    .await
                {
                    Ok(resumed) => {
                        tracing::warn!(
                            "stream {stream_id}: session {session_id} lagged after seq \
                             {last_persisted_seq:?}; replaying {} events",
                            resumed.replay.len()
                        );
                        live = resumed.live;
                        let (replay, rev) = (resumed.replay, resumed.rev);
                        if !send_replay(tx, stream_id, replay, rev, with_meta, &decoder).await {
                            return "the client stopped reading during a lag replay";
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
            None => {
                sink.flush().await;
                return "the session's event source closed";
            }
        }
    }
}

// Under the ~100 ms a delay starts to read as lag, and a harness that sends a delta per
// token (codex, opencode) is held to 20 frames a second.
const PARTIAL_TICK: Duration = Duration::from_millis(50);

/// A session stream's outbound side. Partial text its client renders is coalesced: the
/// first delta goes out at once, later ones at most one frame per `PARTIAL_TICK`.
struct SessionSink<'a> {
    tx: &'a StreamTx,
    stream_id: u64,
    with_meta: bool,
    partials: Partials,
    decoder: Decoder,
    held: Option<HeldPartial>,
    open_at: Instant,
}

struct HeldPartial {
    rev: u64,
    author: Option<ClientRef>,
    text: String,
    parent_id: Option<String>,
}

/// `SessionEvent` keeps its null fields for specta; a partial is sent often enough to drop them.
#[derive(Serialize)]
#[serde(tag = "kind", rename = "partialText", rename_all = "camelCase")]
struct PartialEvent<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
}

#[derive(Serialize)]
struct PartialItem<'a> {
    rev: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<&'a ClientRef>,
    event: PartialEvent<'a>,
}

impl<'a> SessionSink<'a> {
    fn new(
        tx: &'a StreamTx,
        stream_id: u64,
        with_meta: bool,
        partials: Partials,
        decoder: Decoder,
    ) -> Self {
        Self {
            tx,
            stream_id,
            with_meta,
            partials,
            decoder,
            held: None,
            open_at: Instant::now(),
        }
    }

    fn flush_at(&self) -> Option<Instant> {
        self.held.as_ref().map(|_| self.open_at)
    }

    async fn send(&mut self, item: SessionStreamItem) -> bool {
        if self.decoder.withholds(item.event.kind()) {
            return true;
        }
        let SessionEvent::PartialText { text, parent_id } = item.event else {
            return self.flush().await && self.write(&item).await;
        };
        if !self
            .partials
            .forwards(item.author.as_ref(), parent_id.as_deref())
        {
            return true;
        }
        if let Some(held) = &mut self.held {
            if held.parent_id == parent_id && held.author == item.author {
                held.text.push_str(&text);
                held.rev = item.rev;
                return true;
            }
            if !self.flush().await {
                return false;
            }
        }
        let partial = HeldPartial {
            rev: item.rev,
            author: item.author,
            text,
            parent_id,
        };
        if Instant::now() >= self.open_at {
            return self.emit(partial).await;
        }
        self.held = Some(partial);
        true
    }

    async fn flush(&mut self) -> bool {
        match self.held.take() {
            Some(partial) => self.emit(partial).await,
            None => true,
        }
    }

    async fn emit(&mut self, partial: HeldPartial) -> bool {
        self.open_at = Instant::now() + PARTIAL_TICK;
        let event = PartialEvent {
            text: &partial.text,
            parent_id: partial.parent_id.as_deref(),
        };
        if self.with_meta {
            let item = PartialItem {
                rev: partial.rev,
                author: partial.author.as_ref(),
                event,
            };
            send_event(self.tx, self.stream_id, &item).await
        } else {
            send_event(self.tx, self.stream_id, &event).await
        }
    }

    async fn write(&self, item: &SessionStreamItem) -> bool {
        if self.with_meta {
            send_event(self.tx, self.stream_id, item).await
        } else {
            send_event(self.tx, self.stream_id, &item.event).await
        }
    }
}

/// Replayed rows are committed: each carries the rev the session has once the replay is applied.
async fn send_replay(
    tx: &StreamTx,
    stream_id: u64,
    replay: Vec<SessionLogEntry>,
    rev: u64,
    with_meta: bool,
    decoder: &Decoder,
) -> bool {
    for entry in replay {
        if decoder.withholds(&entry.kind) {
            continue;
        }
        let sent = if with_meta {
            send_event(
                tx,
                stream_id,
                &SessionStreamItem {
                    seq: Some(entry.seq),
                    created_at: Some(entry.created_at),
                    rev,
                    author: None,
                    event: entry.event,
                },
            )
            .await
        } else {
            send_event(tx, stream_id, &entry.event).await
        };
        if !sent {
            return false;
        }
    }
    true
}

async fn send_event(tx: &StreamTx, stream_id: u64, event: &impl Serialize) -> bool {
    match tx.ready().await {
        Some(ready) => ready.send(stream_id, event),
        None => false,
    }
}

pub struct TerminalAttached {
    pub stream_id: u64,
    /// The ring held the requested offset at open; a trim can still beat the first read, so output offsets decide.
    pub resumed: bool,
    pub events: TerminalStream,
}

fn expect_stream(rx: Option<StreamReceiver>, command: &str) -> Result<StreamReceiver, WireError> {
    rx.ok_or_else(|| WireError::protocol(format!("{command}: server opened no stream")))
}

fn unexpected(command: &str, response: &StreamResponse) -> WireError {
    WireError::protocol(format!("{command} was answered with {response:?}"))
}

/// A session stream's frames; `Bare` where `with_meta` was not asked for or was ignored.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionFrame {
    Item(SessionStreamItem),
    Bare(SessionEvent),
}

pub struct SessionEvents {
    rx: StreamReceiver,
    with_meta: bool,
}

impl SessionEvents {
    /// `None` at a clean end of stream or an undecodable frame, as `EventStream::recv`.
    pub async fn recv(&mut self) -> Option<SessionFrame> {
        let bytes = self.rx.recv().await?;
        if self.with_meta {
            if let Ok(item) = serde_json::from_slice(&bytes) {
                return Some(SessionFrame::Item(item));
            }
        }
        match serde_json::from_slice(&bytes) {
            Ok(event) => Some(SessionFrame::Bare(event)),
            Err(e) => {
                tracing::error!("session stream event did not decode: {e}");
                None
            }
        }
    }
}

/// The streaming commands' client entry points. Hand-written, not
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
    ) -> Result<(String, u64, TerminalStream), WireError> {
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
                TerminalStream::new(expect_stream(rx, "terminal_open")?),
            )),
            other => Err(unexpected("terminal_open", &other)),
        }
    }

    /// From `offset` when the terminal still holds it; `None` when the terminal is gone.
    pub async fn terminal_attach_stream(
        &self,
        terminal_id: String,
        offset: Option<u64>,
    ) -> Result<Option<TerminalAttached>, WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::TerminalAttach {
                terminal_id,
                offset,
            })
            .await?;
        match response {
            StreamResponse::TerminalAttach {
                attached: true,
                stream_id: Some(stream_id),
                resumed,
            } => Ok(Some(TerminalAttached {
                stream_id,
                resumed,
                events: TerminalStream::new(expect_stream(rx, "terminal_attach")?),
            })),
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
    ) -> Result<(String, u64, TerminalStream), WireError> {
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
                TerminalStream::new(expect_stream(rx, "spawn_cli_terminal")?),
            )),
            other => Err(unexpected("spawn_cli_terminal", &other)),
        }
    }

    pub async fn spawn_shell_terminal_stream(
        &self,
        command: String,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, TerminalStream), WireError> {
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
                TerminalStream::new(expect_stream(rx, "spawn_shell_terminal")?),
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
    ) -> Result<(String, u64, TerminalStream), WireError> {
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
                TerminalStream::new(expect_stream(rx, "agent_open")?),
            )),
            other => Err(unexpected("agent_open", &other)),
        }
    }

    pub async fn pty_launch_stream(
        &self,
        workstream_id: String,
        engine_id: String,
        session: PtyLaunchSession,
        cols: u16,
        rows: u16,
    ) -> Result<(PtyLaunched, u64, TerminalStream), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::PtyLaunch {
                workstream_id,
                engine_id,
                session,
                cols,
                rows,
            })
            .await?;
        match response {
            StreamResponse::PtyLaunch {
                launched,
                stream_id,
            } => Ok((
                launched,
                stream_id,
                TerminalStream::new(expect_stream(rx, "pty_launch")?),
            )),
            other => Err(unexpected("pty_launch", &other)),
        }
    }

    pub async fn script_run_stream(
        &self,
        workstream_id: String,
        kind: ScriptKind,
        cols: u16,
        rows: u16,
    ) -> Result<(String, u64, TerminalStream), WireError> {
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
                TerminalStream::new(expect_stream(rx, "script_run")?),
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
                // Only this branch's client sets this; a released 0.8.4 desktop never does.
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
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
        cursor: Option<LogCursor>,
    ) -> Result<
        (
            Session,
            Option<PermissionMode>,
            u64,
            EventStream<SessionEvent>,
        ),
        WireError,
    > {
        let (response, rx) = self
            .call_stream(StreamRequest::ResumeSession {
                session_id,
                model,
                effort,
                permission_mode,
                cursor,
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
            })
            .await?;
        match response {
            StreamResponse::Session {
                session,
                stream_id,
                permission_mode,
            } => Ok((
                session,
                permission_mode,
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
    ) -> Result<
        (
            Session,
            Option<PermissionMode>,
            u64,
            EventStream<SessionEvent>,
        ),
        WireError,
    > {
        let (response, rx) = self
            .call_stream(StreamRequest::RestartSession {
                session_id,
                model,
                effort,
                permission_mode,
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
            })
            .await?;
        match response {
            StreamResponse::Session {
                session,
                stream_id,
                permission_mode,
            } => Ok((
                session,
                permission_mode,
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
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
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

    /// Opens any session stream; `request` carries its own `with_meta`.
    pub async fn open_session_stream(
        &self,
        request: StreamRequest,
    ) -> Result<(StreamResponse, u64, SessionEvents), WireError> {
        let command = request.command();
        let Some(with_meta) = request.with_meta() else {
            return Err(WireError::protocol(format!(
                "{command} is not a session stream"
            )));
        };
        let (response, rx) = self.call_stream(request).await?;
        let Some(stream_id) = response.stream_id() else {
            return Err(unexpected(command, &response));
        };
        let rx = expect_stream(rx, command)?;
        Ok((response, stream_id, SessionEvents { rx, with_meta }))
    }

    pub async fn attach_session_stream(
        &self,
        session_id: String,
    ) -> Result<(u64, EventStream<SessionEvent>), WireError> {
        let (response, rx) = self
            .call_stream(StreamRequest::AttachSession {
                session_id,
                echo_user_messages: true,
                with_meta: false,
                all_partials: false,
            })
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use hyprlayer_proto::Features;

    use super::*;

    #[tokio::test]
    async fn closing_the_connection_wakes_a_stream_waiting_for_credit() {
        let outbound = Outbound::new(Features::ALL, false);
        let tx = StreamTx {
            outbound: outbound.clone(),
            stream_id: 1,
            queue: Arc::new(Credit::new(STREAM_QUEUE_BYTES)),
            credit: Arc::new(Credit::new(0)),
        };
        let closing = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            outbound.close();
        };
        let woke = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(tx.ready(), closing).0.is_none()
        })
        .await;
        assert_eq!(woke, Ok(true), "a closed connection must end the wait");
    }
}
