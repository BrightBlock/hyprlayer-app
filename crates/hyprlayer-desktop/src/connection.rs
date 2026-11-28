//! One server connection as an entity: connect, announce, relay runtime events, pace reconnects.
//! Sockets and timers run on the io thread and report over one channel drained on the foreground.

use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, EventEmitter, Task};
use hyprlayer_client::client::RpcClient;
use hyprlayer_client::hosts::Target;
use hyprlayer_client::supervisor::{self, Outcome};
use hyprlayer_core::models::RuntimeEvent;
use hyprlayer_server::client::GapPacer;
use hyprlayer_server::events::{REVISIONS_IN_STREAM, TARGETED_EVENTS, TRANSCRIPT_SYNC};
use hyprlayer_ui::theme::Mode;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::JoinHandle;

use crate::config::{colorfgbg, env_overlay, Config};
use crate::error::UiError;
use crate::io::Io;
use crate::terminal_theme;

/// Quick at first, then settled; never gives up, as `runtime_events.rs` paces the Tauri window.
const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];
const RECONNECT_STEADY: Duration = Duration::from_secs(30);

/// Failures only the user can clear; retrying every 30 s would not help (`reconnect.ts`).
const HALTS_RETRIES: [&str; 2] = ["protocol_version_mismatch", "remote_sign_in_cancelled"];

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// The launch attempt, before anything has failed.
    Connecting,
    Connected {
        outdated: bool,
    },
    /// The link dropped after it was up; `error` is the latest reconnect's failure.
    Lost {
        error: Option<UiError>,
    },
    /// It never came up.
    Blocked {
        title: String,
        error: UiError,
    },
}

#[derive(Clone, Debug)]
pub enum Notice {
    Connected,
    Lost,
    Runtime(Arc<RuntimeEvent>),
    EventsMissed,
}

impl EventEmitter<Notice> for Connection {}

enum Msg {
    Attached {
        generation: u64,
        client: Arc<RpcClient>,
        outdated: bool,
        transcript_sync: bool,
    },
    Failed {
        generation: u64,
        title: String,
        error: UiError,
    },
    Event(u64, RuntimeEvent),
    Missed(u64),
    Ended(u64),
    ReconnectDue,
}

pub struct Connection {
    config: Arc<Config>,
    io: Io,
    tx: UnboundedSender<Msg>,
    status: Status,
    client: Option<Arc<RpcClient>>,
    /// Bumped per attempt; messages from an older attempt's client are dropped.
    generation: u64,
    attempting: bool,
    pacer: Option<JoinHandle<()>>,
    mode: Mode,
    transcript_sync: bool,
    connects: u64,
    gaps: u64,
    _drain: Task<()>,
}

impl Connection {
    pub fn new(config: Config, io: Io, mode: Mode, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = unbounded_channel();
        let drain = cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.recv().await {
                if this.update(cx, |this, cx| this.handle(msg, cx)).is_err() {
                    return;
                }
            }
        });
        Self {
            config: Arc::new(config),
            io,
            tx,
            status: Status::Connecting,
            client: None,
            generation: 0,
            attempting: false,
            pacer: None,
            mode,
            transcript_sync: false,
            connects: 0,
            gaps: 0,
            _drain: drain,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn client(&self) -> Option<Arc<RpcClient>> {
        self.client.clone()
    }

    /// The generation reads must match to count as current; `None` unless connected.
    pub fn live_generation(&self) -> Option<u64> {
        self.client.as_ref().map(|_| self.generation)
    }

    pub fn target(&self) -> &Target {
        &self.config.target
    }

    pub fn attempting(&self) -> bool {
        self.attempting
    }

    pub fn transcript_sync(&self) -> bool {
        self.transcript_sync
    }

    /// Successful attaches.
    pub fn connects(&self) -> u64 {
        self.connects
    }

    /// `EventsMissed` notices relayed.
    pub fn gaps(&self) -> u64 {
        self.gaps
    }

    pub fn connect(&mut self, cx: &mut Context<Self>) {
        if self.attempting || self.client.is_some() {
            return;
        }
        self.attempting = true;
        self.generation += 1;
        cx.notify();
        let (generation, tx, config, mode) = (
            self.generation,
            self.tx.clone(),
            self.config.clone(),
            self.mode,
        );
        self.io.handle().spawn(async move {
            let paths = &config.paths;
            let outcome = supervisor::connect(
                &config.target,
                &paths.config,
                &paths.data,
                &paths.log,
                &config.ssh,
            )
            .await;
            let failed = match outcome {
                Outcome::Connected { client, outdated } => {
                    match attach(&client, &config, mode).await {
                        Ok(transcript_sync) => {
                            // Before the relay starts, so its `Ended` can't overtake the attach.
                            let _ = tx.send(Msg::Attached {
                                generation,
                                client: client.clone(),
                                outdated,
                                transcript_sync,
                            });
                            relay(&tx, generation, &client);
                            return;
                        }
                        Err(error) => Msg::Failed {
                            generation,
                            title: "Can't use this server".into(),
                            error,
                        },
                    }
                }
                Outcome::Blocked { title, error } => Msg::Failed {
                    generation,
                    title,
                    error: error.into(),
                },
            };
            let _ = tx.send(failed);
        });
    }

    /// Theme changes reach the server as `set_terminal_theme`/`set_terminal_color_scheme` do.
    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        let Some(client) = self.client.clone() else {
            return;
        };
        self.io.handle().spawn(async move {
            if let Err(e) = client.set_env_overlay([colorfgbg(mode)].into()).await {
                log::warn!("could not send the colour scheme: {}", e.message);
            }
            send_theme(&client, mode).await;
        });
        cx.notify();
    }

    /// Feeds the server's PR watcher, as the Tauri window's focus event does.
    pub fn set_foreground(&self, foreground: bool) {
        if let Some(client) = self.client.clone() {
            self.io.handle().spawn(async move {
                let _ = client.set_foreground(foreground).await;
            });
        }
    }

    fn handle(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Attached {
                generation,
                client,
                outdated,
                transcript_sync,
            } if generation == self.generation => {
                log::info!("attached to {}", self.config.target.name());
                self.attempting = false;
                self.stop_pacer();
                self.client = Some(client);
                self.transcript_sync = transcript_sync;
                self.connects += 1;
                self.status = Status::Connected { outdated };
                cx.emit(Notice::Connected);
                cx.notify();
            }
            Msg::Failed {
                generation,
                title,
                error,
            } if generation == self.generation => {
                self.attempting = false;
                let halts = HALTS_RETRIES.contains(&error.code.as_str());
                self.status = match &self.status {
                    Status::Lost { .. } => Status::Lost { error: Some(error) },
                    _ => Status::Blocked { title, error },
                };
                if halts {
                    log::error!("stopped retrying: only the user can clear this failure");
                    self.stop_pacer();
                } else if self.pacer.is_none() {
                    self.start_pacer();
                }
                cx.notify();
            }
            Msg::Event(generation, event) if self.is_live(generation) => {
                cx.emit(Notice::Runtime(Arc::new(event)));
            }
            Msg::Missed(generation) if self.is_live(generation) => {
                log::warn!("runtime events were missed; re-reading");
                self.gaps += 1;
                cx.emit(Notice::EventsMissed);
            }
            Msg::Ended(generation) if self.is_live(generation) => {
                log::warn!("the server connection is lost; reconnecting");
                self.client = None;
                self.status = Status::Lost { error: None };
                self.start_pacer();
                cx.emit(Notice::Lost);
                cx.notify();
            }
            Msg::ReconnectDue => self.connect(cx),
            _ => {}
        }
    }

    fn is_live(&self, generation: u64) -> bool {
        generation == self.generation && self.client.is_some()
    }

    fn stop_pacer(&mut self) {
        if let Some(pacer) = self.pacer.take() {
            pacer.abort();
        }
    }

    /// Restarts the schedule from its first step; at most one runs.
    fn start_pacer(&mut self) {
        self.stop_pacer();
        let tx = self.tx.clone();
        self.pacer = Some(self.io.handle().spawn(async move {
            for attempt in 0.. {
                let delay = RECONNECT_DELAYS.get(attempt).copied();
                tokio::time::sleep(delay.unwrap_or(RECONNECT_STEADY)).await;
                if tx.send(Msg::ReconnectDue).is_err() {
                    return;
                }
            }
        }));
    }
}

/// One relay per client, retiring with it: `take_events` is one-shot per connection.
fn relay(tx: &UnboundedSender<Msg>, generation: u64, client: &Arc<RpcClient>) {
    let mut events = client
        .take_events()
        .expect("a fresh client still holds its event stream");
    let mut gaps = GapPacer::new(client.events_missed());
    let missed = tx.clone();
    tokio::spawn(async move {
        while gaps.next().await {
            if missed.send(Msg::Missed(generation)).is_err() {
                return;
            }
        }
    });
    let tx = tx.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if tx.send(Msg::Event(generation, event)).is_err() {
                return;
            }
        }
        let _ = tx.send(Msg::Ended(generation));
    });
}

/// What `attach_client` does in the Tauri shell, on launch and on every reconnect.
async fn attach(client: &RpcClient, config: &Config, mode: Mode) -> Result<bool, UiError> {
    // The handshake passed but the first request did not: a build this app cannot talk to.
    if let Err(e) = client.worktree_root().await {
        if e.code == "server_disconnected" {
            return Err(supervisor::incompatible_server_error().into());
        }
        log::warn!("could not fetch the worktree root: {}", e.message);
    }
    let overlay = env_overlay(&config.target, mode);
    log::info!(
        "sending the server {}",
        overlay.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    if let Err(e) = client.set_env_overlay(overlay).await {
        log::warn!("could not send the environment overlay: {}", e.message);
    }
    send_theme(client, mode).await;
    let identity = &config.identity;
    let announced = client
        .announce_client(
            identity.client_id.clone(),
            identity.device.clone(),
            TARGETED_EVENTS
                .into_iter()
                .chain([REVISIONS_IN_STREAM])
                .map(str::to_string)
                .collect(),
        )
        .await;
    Ok(match announced {
        Ok(announced) => {
            let on = announced.features.iter().any(|f| f == TRANSCRIPT_SYNC);
            log::info!(
                "announced this client as {}; transcript sync {}",
                identity.device,
                if on { "on" } else { "off" }
            );
            on
        }
        Err(e) if e.code == "unsupported_command" => false,
        Err(e) => {
            log::warn!("announcing this client failed: {}", e.message);
            false
        }
    })
}

async fn send_theme(client: &RpcClient, mode: Mode) {
    if !client.server_features().screen_frames {
        return;
    }
    let theme = terminal_theme::for_mode(mode);
    // server-compat: gated on screen_frames
    if let Err(e) = client.terminal_theme(theme).await {
        log::warn!("could not send the terminal theme: {}", e.message);
    }
}
