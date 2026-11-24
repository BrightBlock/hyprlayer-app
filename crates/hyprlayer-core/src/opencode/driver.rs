//! One hyprlayer session on the shared OpenCode 2 server.
//!
//! A message goes to OpenCode's inbox under this driver's own `msg_` id. OpenCode reports when
//! its run reads it (`session.inbox.delivered`) and when the run ends
//! (`session.execution.succeeded|failed|interrupted`), so a turn ends on the end of the run that
//! read one of its messages. A message sent while a turn runs is steered into that run; its echo
//! waits for the delivery, and a stop that withdraws it first hands it back.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch, Mutex};

use crate::agent::{declined_message, AgentDriver, KillFlag, UsageSoFar};
use crate::claude::{ExitOutcome, SessionEvent};
use crate::error::Result as CoreResult;
use crate::models::session::QuestionAnswers;
use crate::opencode::runtime::OpenCodeServer;
use crate::opencode::translate::{
    failure_text, form_answer, form_ask, form_fields, permission_ask, OcEvent, Outcome, Translator,
};

const EVENT_CAP: usize = 1024;
const RESYNC_MESSAGES: u32 = 50;
const RESYNC_WAIT: Duration = Duration::from_secs(5);
/// Under the desktop's three seconds before a stop escalates to a kill.
const STOP_WAIT: Duration = Duration::from_secs(2);

enum AskKind {
    Permission,
    Form(Vec<Value>),
}

/// A request OpenCode waits on, and the session that owns it: a subagent's ask is answered
/// under the subagent's session.
struct Ask {
    owner: String,
    kind: AskKind,
}

/// What the driver knows of its turn and of the run OpenCode has going.
#[derive(Default)]
struct Turn {
    /// The turn's first echo went out and its TurnDone has not.
    open: bool,
    /// A run read one of the open turn's messages.
    delivered: bool,
    /// A run this driver saw start has not ended.
    executing: bool,
    /// A stop was asked and no run's end, nor the stop finding none, has settled it yet.
    stopping: bool,
    /// Held messages being withdrawn; the turn's end waits for each to come back, so its
    /// hand-back reaches the relay ahead of the TurnDone it resends at.
    stop_cancels: HashSet<String>,
    /// That end's status, while it waits.
    done_owed: Option<String>,
    /// Messages steered again once after a run ended without reading them.
    woken: HashSet<String>,
    /// Messages whose prompt OpenCode has answered: one it has not is missing from its inbox
    /// too, which a read-back must not take for a withdrawal.
    admitted: HashSet<String>,
    /// A command's inbox id, which OpenCode names itself, by the driver's own.
    renamed: HashMap<String, String>,
    /// The messages a stop is withdrawing: those undelivered when it was pressed.
    stop_targets: HashSet<String>,
    /// The running run started with no turn open: a subagent's result woke the session.
    foreign: bool,
    /// Messages OpenCode holds undelivered, in send order; `Some` while the echo waits.
    undelivered: Vec<(String, Option<SessionEvent>)>,
    /// Commands posted whose inbox item OpenCode names itself.
    adopting: Vec<String>,
    /// Held messages the composer is taking back, and those OpenCode cancelled for it.
    withdrawing: HashSet<String>,
    withdrawn: HashSet<String>,
}

/// One POST, sent in order with every other.
enum Post {
    Prompt {
        id: String,
        body: Value,
    },
    Command {
        id: String,
        body: Value,
        fallback: Value,
    },
    Interrupt {
        session: String,
    },
    CancelInbox {
        id: String,
    },
    Wake {
        id: String,
    },
    Model {
        model: Value,
    },
    PermissionReply {
        owner: String,
        id: String,
        decision: &'static str,
    },
    FormReply {
        owner: String,
        id: String,
        answer: Value,
    },
    FormCancel {
        owner: String,
        id: String,
    },
}

struct Core {
    root: String,
    family: HashSet<String>,
    turn: Turn,
    asks: HashMap<String, Ask>,
    xlate: Translator,
    tx: broadcast::Sender<SessionEvent>,
    posts: mpsc::UnboundedSender<Post>,
    /// Unix ms at spawn: a read-back replays no stored message older.
    since: u64,
}

type Shared = Arc<StdMutex<Core>>;

fn lock(core: &Shared) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Core {
    fn emit(&self, event: SessionEvent) {
        let _ = self.tx.send(event);
    }

    fn post(&self, post: Post) {
        let _ = self.posts.send(post);
    }

    /// Mid-turn, a message from someone who hears back (`hold`) echoes once OpenCode reads it.
    fn send(&mut self, id: String, echo: SessionEvent, hold: bool) {
        if self.turn.open && hold {
            self.turn.undelivered.push((id, Some(echo)));
            return;
        }
        self.emit(echo);
        if !self.turn.open {
            // A stop pressed with no turn open is not this new turn's.
            self.turn.stopping = false;
            self.open_turn();
        }
        self.turn.undelivered.push((id, None));
    }

    fn open_turn(&mut self) {
        self.turn.open = true;
        self.turn.delivered = false;
    }

    /// OpenCode answered the prompt that sent it.
    fn admitted(&mut self, id: &str) {
        let id = self
            .turn
            .renamed
            .remove(id)
            .unwrap_or_else(|| id.to_string());
        if self.turn.undelivered.iter().any(|(sent, _)| *sent == id) {
            self.turn.admitted.insert(id);
        }
    }

    fn take_undelivered(&mut self, id: &str) -> Option<Option<SessionEvent>> {
        self.turn.admitted.remove(id);
        self.turn.stop_targets.remove(id);
        let at = self
            .turn
            .undelivered
            .iter()
            .position(|(sent, _)| sent == id)?;
        Some(self.turn.undelivered.remove(at).1)
    }

    fn echoed_undelivered(&self) -> bool {
        self.turn.undelivered.iter().any(|(_, held)| held.is_none())
    }

    fn enqueued(&mut self, ev: &OcEvent) {
        let id = ev.str("inboxID");
        let user = ev.data.pointer("/item/type").and_then(Value::as_str) == Some("user");
        let known = self.turn.undelivered.iter().any(|(sent, _)| sent == id);
        if user && !known && !self.turn.adopting.is_empty() {
            let ours = self.turn.adopting.remove(0);
            if let Some(entry) = self
                .turn
                .undelivered
                .iter_mut()
                .find(|(sent, _)| *sent == ours)
            {
                entry.0 = id.to_string();
            }
            self.turn.renamed.insert(ours, id.to_string());
        }
    }

    fn delivered(&mut self, id: &str) {
        let Some(held) = self.take_undelivered(id) else {
            return;
        };
        self.turn.withdrawing.remove(id);
        self.turn.stop_cancels.remove(id);
        if let Some(echo) = held {
            self.emit(echo);
        }
        if self.turn.done_owed.is_none() {
            if !self.turn.open {
                self.open_turn();
            }
            self.turn.delivered = true;
        }
        self.settle_owed();
    }

    fn cancelled(&mut self, id: &str) {
        if self.turn.withdrawing.remove(id) {
            self.take_undelivered(id);
            self.turn.withdrawn.insert(id.to_string());
            return;
        }
        self.turn.stop_cancels.remove(id);
        match self.take_undelivered(id) {
            Some(Some(echo)) => self.emit_all(declined_message(&echo)),
            // Nothing of the turn went to a run, and nothing of it is left to: it ends, a run going
            // or not.
            Some(None) if self.turn.open && !self.turn.delivered && !self.echoed_undelivered() => {
                let status = if self.turn.stopping {
                    "interrupted"
                } else {
                    "failed"
                };
                self.finish(status);
            }
            _ => {}
        }
        self.settle_owed();
    }

    /// A withdrawal OpenCode could not be asked for will not come back.
    fn cancel_failed(&mut self, id: &str) {
        self.turn.stop_cancels.remove(id);
        self.settle_owed();
    }

    fn withdrawals_pending(&self) -> bool {
        self.turn
            .undelivered
            .iter()
            .any(|(id, _)| self.turn.stop_cancels.contains(id))
    }

    fn settle_owed(&mut self) {
        if self.turn.done_owed.is_some() && !self.withdrawals_pending() {
            if let Some(status) = self.turn.done_owed.take() {
                self.emit(self.xlate.ended_as(&status));
            }
        }
    }

    fn emit_all(&self, events: impl IntoIterator<Item = SessionEvent>) {
        for event in events {
            self.emit(event);
        }
    }

    fn started(&mut self) {
        self.turn.executing = true;
        if !self.turn.open {
            self.turn.foreign = true;
        }
        // A stop asked as this run started, while an earlier one was ending.
        if self.turn.stopping {
            self.post(Post::Interrupt {
                session: self.root.clone(),
            });
        }
    }

    fn execution_end(&mut self, outcome: Outcome, error: Option<&Value>) {
        self.turn.executing = false;
        self.decline_owned(&self.root.clone());
        self.wake_parked();
        let ours = (self.turn.open && self.turn.delivered) || self.turn.stopping;
        if ours || (!self.turn.open && self.turn.foreign) {
            let status = if self.turn.stopping || outcome == Outcome::Interrupted {
                "interrupted"
            } else if outcome == Outcome::Failed {
                self.emit(failure_text(error));
                "failed"
            } else {
                "completed"
            };
            self.finish(status);
        }
        self.turn.foreign = false;
    }

    /// A run that never read them, or one that failed first, leaves them parked: steering them
    /// again starts a run, once. One a second run also left unread is withdrawn and handed
    /// back, so a run that keeps failing is not woken forever. Those a stop is withdrawing are
    /// left to it.
    fn wake_parked(&mut self) {
        let parked: Vec<(String, bool)> = self
            .turn
            .undelivered
            .iter()
            .filter(|(id, _)| {
                self.turn.admitted.contains(id) && !self.turn.stop_targets.contains(id)
            })
            .map(|(id, echo)| (id.clone(), echo.is_some()))
            .collect();
        for (id, held) in parked {
            if self.turn.woken.insert(id.clone()) {
                self.post(Post::Wake { id });
            } else {
                if held {
                    self.turn.stop_cancels.insert(id.clone());
                }
                self.post(Post::CancelInbox { id });
            }
        }
        // With no turn open, a hand-back still needs a TurnDone after it: the relay resends a
        // handed-back agent message at one.
        if !self.turn.open && self.turn.done_owed.is_none() && self.withdrawals_pending() {
            self.turn.done_owed = Some("failed".into());
        }
    }

    /// The turn's end; a stopped one waits for the stop's withdrawals of held messages.
    fn finish(&mut self, status: &str) {
        if self.withdrawals_pending() {
            self.turn.done_owed = Some(status.to_string());
        } else {
            self.emit(self.xlate.ended_as(status));
        }
        self.turn.open = false;
        self.turn.delivered = false;
        self.turn.stopping = false;
        // A run still going goes on outside a turn, and its end closes it.
        self.turn.foreign = self.turn.executing;
    }

    /// A stop found no run, and none was seen start: OpenCode starts a run as it admits a
    /// prompt, so nothing is left to end the turn.
    fn stop_missed(&mut self) {
        if self.turn.stopping && !self.turn.executing {
            self.finish("interrupted");
        }
    }

    /// Every held ask closes as declined; the stop, a stop for each subagent waiting on the user,
    /// and a withdrawal of each message OpenCode has not read go out behind what is queued.
    fn stop(&mut self) {
        let mut owners: Vec<String> = self.asks.values().map(|ask| ask.owner.clone()).collect();
        owners.sort();
        owners.dedup();
        let mut asks: Vec<String> = self.asks.keys().cloned().collect();
        asks.sort();
        for id in asks {
            self.take_ask(&id, false);
        }
        let undelivered: Vec<(String, bool)> = self
            .turn
            .undelivered
            .iter()
            .map(|(id, echo)| (id.clone(), echo.is_some()))
            .collect();
        if self.turn.open || self.turn.executing || !undelivered.is_empty() {
            self.turn.stopping = true;
        }
        // Each message OpenCode has not read is withdrawn first: a stop is not to run it later,
        // and a run a subagent's result wakes must find none of them parked.
        for (id, held) in undelivered {
            if held {
                self.turn.stop_cancels.insert(id.clone());
            }
            self.turn.stop_targets.insert(id.clone());
            self.post(Post::CancelInbox { id });
        }
        let root = self.root.clone();
        self.post(Post::Interrupt {
            session: root.clone(),
        });
        // A subagent waiting on the user stops with the turn, so no ask outlives the stop.
        for owner in owners.into_iter().filter(|owner| *owner != root) {
            self.post(Post::Interrupt { session: owner });
        }
    }

    fn post_failed(&mut self, id: &str, error: &str) {
        tracing::warn!("opencode prompt failed: {error}");
        match self.take_undelivered(id) {
            Some(Some(echo)) => self.emit_all(declined_message(&echo)),
            Some(None) => {
                self.emit(SessionEvent::AssistantText {
                    text: format!("⚠ OpenCode request failed: {error}"),
                    parent_id: None,
                    injected: false,
                    auth_required: false,
                });
                if self.turn.open && !self.turn.delivered && !self.echoed_undelivered() {
                    self.finish("failed");
                }
            }
            None => {}
        }
    }

    fn asked(&mut self, id: &str, owner: &str, kind: AskKind, ask: SessionEvent) {
        if id.is_empty() || self.asks.contains_key(id) {
            return;
        }
        let owner = owner.to_string();
        self.asks.insert(id.to_string(), Ask { owner, kind });
        self.emit(ask);
    }

    fn take_ask(&mut self, id: &str, allow: bool) -> Option<Ask> {
        let ask = self.asks.remove(id)?;
        self.emit(closing(id, &ask, allow));
        Some(ask)
    }

    /// An ended run drops its asks with no reply event.
    fn decline_owned(&mut self, owner: &str) {
        let mut gone: Vec<String> = self
            .asks
            .iter()
            .filter(|(_, ask)| ask.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        gone.sort();
        for id in gone {
            self.take_ask(&id, false);
        }
    }

    /// `older`: queued before the last read-back, which already covers what it says of asks
    /// and of whether a run is going.
    fn take(&mut self, ev: &OcEvent, older: bool) {
        let from = ev.session().unwrap_or_default().to_string();
        if ev.kind == "session.created" {
            let parent = ev.str("parentID");
            if !parent.is_empty() && self.family.contains(parent) {
                self.family.insert(from.clone());
            }
        }
        if !self.family.contains(&from) {
            return;
        }
        let outcome = Outcome::of(&ev.kind);
        if older && (ev.kind == "permission.asked" || ev.kind == "form.created") {
            return;
        }
        // The read-back said whether a run is going.
        let lifecycle = outcome.is_some() || ev.kind == "session.execution.started";
        if older && lifecycle && from == self.root {
            return;
        }
        match ev.kind.as_str() {
            "permission.asked" => {
                let ask = permission_ask(&ev.data);
                self.asked(ev.str("id"), &from, AskKind::Permission, ask);
                return;
            }
            "permission.replied" => {
                self.take_ask(ev.str("requestID"), ev.str("reply") != "reject");
                return;
            }
            "form.created" => {
                let form = ev.data.get("form").unwrap_or(&Value::Null);
                let id = form.get("id").and_then(Value::as_str).unwrap_or_default();
                let kind = AskKind::Form(form_fields(form));
                self.asked(id, &from, kind, form_ask(form));
                return;
            }
            "form.replied" | "form.cancelled" => {
                self.take_ask(ev.str("id"), true);
                return;
            }
            _ => {}
        }
        if from != self.root {
            if outcome.is_some() {
                self.decline_owned(&from);
            }
            return;
        }
        match ev.kind.as_str() {
            "session.inbox.enqueued" => self.enqueued(ev),
            "session.inbox.delivered" => self.delivered(ev.str("inboxID")),
            "session.inbox.cancelled" => self.cancelled(ev.str("inboxID")),
            "session.execution.started" => self.started(),
            _ => {}
        }
        let content = self.xlate.content(ev);
        self.emit_all(content);
        if let Some(outcome) = outcome {
            self.execution_end(outcome, ev.data.get("error"));
        }
    }

    /// What OpenCode holds after a gap in the stream, folded in as the events it stands for.
    fn resync(&mut self, seen: ReadBack) {
        let ReadBack {
            inbox,
            busy,
            messages,
            asks,
        } = seen;
        if let Some(messages) = &messages {
            let mut stored: Vec<&Value> = messages
                .iter()
                .filter(|m| {
                    let created = m.pointer("/time/created").and_then(Value::as_u64);
                    created.is_some_and(|at| at >= self.since)
                })
                .collect();
            stored.reverse();
            let users: HashSet<&str> = messages
                .iter()
                .filter(|m| m.get("type").and_then(Value::as_str) == Some("user"))
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .collect();
            if let Some(inbox) = &inbox {
                let left: Vec<String> = self
                    .turn
                    .undelivered
                    .iter()
                    .map(|(id, _)| id.clone())
                    .filter(|id| self.turn.admitted.contains(id) && !inbox.contains(id))
                    .collect();
                for id in left {
                    if users.contains(id.as_str()) {
                        self.delivered(&id);
                    } else {
                        self.cancelled(&id);
                    }
                }
            }
            for message in stored {
                if message.get("type").and_then(Value::as_str) == Some("assistant") {
                    let replayed = self.xlate.replay(message);
                    self.emit_all(replayed);
                }
            }
            if busy == Some(false) {
                let last = messages
                    .iter()
                    .find(|m| m.get("type").and_then(Value::as_str) == Some("idle"));
                let outcome = last
                    .and_then(|m| m.get("outcome"))
                    .and_then(Value::as_str)
                    .map_or(Outcome::Succeeded, Outcome::of_idle);
                if self.turn.executing || self.turn.open || self.turn.stopping {
                    self.execution_end(outcome, None);
                } else {
                    // Whatever ended in the gap, what OpenCode holds of this driver's is parked.
                    self.wake_parked();
                }
            }
        }
        if busy == Some(true) {
            self.turn.executing = true;
            // A run going with no turn open, its start lost in the gap: a subagent's result.
            if !self.turn.open {
                self.turn.foreign = true;
            }
        }
        if let Some(listed) = asks {
            let gone: Vec<String> = self
                .asks
                .keys()
                .filter(|id| !listed.iter().any(|(listed, _, _)| listed == *id))
                .cloned()
                .collect();
            for id in gone {
                self.take_ask(&id, false);
            }
            for (id, owner, ask) in listed {
                match ask {
                    Listed::Permission(data) => {
                        let ask = permission_ask(&data);
                        self.asked(&id, &owner, AskKind::Permission, ask);
                    }
                    Listed::Form(form) => {
                        let kind = AskKind::Form(form_fields(&form));
                        self.asked(&id, &owner, kind, form_ask(&form));
                    }
                }
            }
        }
    }
}

enum Listed {
    Permission(Value),
    Form(Value),
}

/// One read of OpenCode's state; `None` where a read failed, which then changes nothing.
struct ReadBack {
    inbox: Option<Vec<String>>,
    busy: Option<bool>,
    /// Newest first.
    messages: Option<Vec<Value>>,
    asks: Option<Vec<(String, String, Listed)>>,
}

/// The row that closes an ask; a form's carries no decision.
fn closing(id: &str, ask: &Ask, allow: bool) -> SessionEvent {
    match ask.kind {
        AskKind::Permission => SessionEvent::PermissionResolved {
            request_id: id.to_string(),
            allow,
            withdrawn: false,
        },
        AskKind::Form(_) => SessionEvent::QuestionAnswered {
            request_id: id.to_string(),
        },
    }
}

/// An id in OpenCode's ascending format: the low 48 bits of `ms * 0x1000 + counter`.
fn message_id() -> String {
    static LAST: StdMutex<u64> = StdMutex::new(0);
    let now = crate::opencode::runtime::now_ms().wrapping_mul(0x1000);
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    *last = now.max(*last + 1);
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("msg_{:012x}{}", *last & 0xffff_ffff_ffff, &suffix[..14])
}

pub(crate) fn model_ref(model: &(String, String)) -> Value {
    json!({ "id": model.1, "providerID": model.0 })
}

fn parse_model(id: &str) -> Option<(String, String)> {
    id.split_once('/')
        .map(|(p, m)| (p.to_string(), m.to_string()))
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

pub struct OpenCodeDriver {
    server: Arc<OpenCodeServer>,
    session_id: String,
    skills: HashSet<String>,
    /// What OpenCode can run as `/name`: its commands, and its skills.
    commands: HashSet<String>,
    invocable_skills: HashSet<String>,
    model: StdMutex<Option<(String, String)>>,
    events: broadcast::Sender<SessionEvent>,
    kill_flag: Arc<KillFlag>,
    filter_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    core: Shared,
}

impl OpenCodeDriver {
    pub async fn spawn(
        server: Arc<OpenCodeServer>,
        cwd: &str,
        model: Option<String>,
        resume: Option<&str>,
    ) -> Result<(Arc<Self>, broadcast::Receiver<SessionEvent>), String> {
        let model = model.as_deref().and_then(parse_model);
        let (session_id, lost) = match resume {
            Some(id) => match server.missing_session(id).await? {
                None => (id.to_string(), None),
                Some(status) => (
                    server.create_session(cwd, model.as_ref()).await?,
                    Some((id, status)),
                ),
            },
            None => (server.create_session(cwd, model.as_ref()).await?, None),
        };
        let resumed = lost.is_none() && resume.is_some();

        // OpenCode's prompt route does not parse slash commands: a typed `/name` runs through
        // its command route, or as a prompt carrying the skill. The local skill index only
        // decides which invocations carry the cross-harness routing preamble.
        let skill_scan = tokio::task::spawn_blocking({
            let cwd = cwd.to_string();
            let home = server.home().to_path_buf();
            move || {
                crate::skills::list_skills(
                    Some(std::path::Path::new(&cwd)),
                    &crate::skills::SkillHome::under(&home),
                )
                .into_iter()
                .map(|skill| skill.name)
                .collect::<HashSet<String>>()
            }
        });
        let (skills, commands, invocable_skills) = tokio::join!(
            skill_scan,
            server.names("/command", "name", cwd),
            server.names("/skill", "id", cwd),
        );
        let skills = skills.unwrap_or_default();
        let discovered = |found: Result<HashSet<String>, String>, what: &str| {
            found.unwrap_or_else(|error| {
                tracing::warn!("opencode {what} discovery failed; `/name` falls back to a plain prompt: {error}");
                HashSet::new()
            })
        };
        let commands = discovered(commands, "command");
        let invocable_skills = discovered(invocable_skills, "skill");

        let (events, _) = broadcast::channel(EVENT_CAP);
        let primary_rx = events.subscribe();
        let _ = events.send(SessionEvent::Init {
            provider_session_id: session_id.clone(),
            model: model.as_ref().map(|(p, m)| format!("{p}/{m}")),
            mcp_servers: None,
        });
        if let Some((lost, status)) = lost {
            tracing::warn!(
                "opencode no longer has session {lost} ({status}); continuing in {session_id} without its earlier turns"
            );
            let _ = events.send(SessionEvent::ProtocolError {
                detail: "OpenCode no longer has this conversation; it continues without the earlier turns".into(),
                raw: format!("OpenCode answered {status} for session {lost}; started {session_id}"),
            });
        }
        if let Some(tokens) = model.as_ref().and_then(|(p, m)| {
            let id = format!("{p}/{m}");
            crate::opencode::config::opencode_model_context_window(&id)
                .or_else(|| server.model_context(&id))
        }) {
            let _ = events.send(SessionEvent::ContextWindow { tokens });
        }

        let (posts, queued) = mpsc::unbounded_channel();
        let core = Arc::new(StdMutex::new(Core {
            root: session_id.clone(),
            family: HashSet::from([session_id.clone()]),
            turn: Turn::default(),
            asks: HashMap::new(),
            xlate: Translator::default(),
            tx: events.clone(),
            posts,
            since: crate::opencode::runtime::now_ms(),
        }));
        if let (true, Some(model)) = (resumed, &model) {
            lock(&core).post(Post::Model {
                model: model_ref(model),
            });
        }
        spawn_posts(&server, &session_id, &core, queued);

        let task = spawn_filter(&server, core.clone(), events.clone());
        let driver = Arc::new(Self {
            server,
            session_id,
            skills,
            commands,
            invocable_skills,
            model: StdMutex::new(model),
            events,
            kill_flag: Arc::new(KillFlag::default()),
            filter_task: Mutex::new(Some(task)),
            core,
        });
        Ok((driver, primary_rx))
    }

    /// Stops the run and withdraws the messages it has not read before a relaunch reuses the
    /// session; the next driver on it waits for this.
    fn end(&self, outcome: ExitOutcome) {
        let held: Vec<String> = {
            let core = lock(&self.core);
            core.turn
                .undelivered
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        };
        let session = self.session_id.clone();
        let server = self.server.clone();
        let (done, stopped) = watch::channel(false);
        self.server.stopping(&session, stopped);
        self.server.spawn(async move {
            for id in held {
                let path = format!("/session/{session}/inbox/{id}");
                let _ = server
                    .request(reqwest::Method::DELETE, &path)
                    .timeout(STOP_WAIT)
                    .send()
                    .await;
            }
            let interrupt = server
                .request(
                    reqwest::Method::POST,
                    &format!("/session/{session}/interrupt"),
                )
                .query(&[("resume", "false")]);
            if let Err(e) = interrupt.timeout(STOP_WAIT).send().await {
                tracing::warn!("opencode interrupt of {session} failed: {e}");
            }
            let _ = done.send(true);
        });
        if let Ok(mut guard) = self.filter_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        let _ = self.events.send(SessionEvent::Exited {
            outcome,
            code: None,
            stderr_tail: String::new(),
        });
    }

    pub async fn delete_session(&self) {
        let path = format!("/session/{}", self.session_id);
        if let Err(e) = self
            .server
            .request(reqwest::Method::DELETE, &path)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            tracing::warn!("opencode session delete failed: {e}");
        }
    }

    fn invocation_body(&self, id: &str, text: &str, prompt_name: Option<&str>) -> Post {
        let model = self.model.lock().ok().and_then(|model| model.clone());
        let prompt = json!({ "id": id, "text": text, "delivery": "steer" });
        let Some(invocation) = leading_invocation(text, prompt_name) else {
            return Post::Prompt {
                id: id.to_string(),
                body: prompt,
            };
        };
        let arguments = if self.skills.contains(invocation.name) {
            routed_skill_arguments(skill_base_for_model(model.as_ref()), invocation.arguments)
        } else {
            invocation.arguments.to_string()
        };
        if self.invocable_skills.contains(invocation.name) {
            let body = json!({
                "id": id, "text": arguments, "delivery": "steer",
                "skills": [{ "id": invocation.name }],
            });
            return Post::Prompt {
                id: id.to_string(),
                body,
            };
        }
        if self.commands.contains(invocation.name) {
            let body = json!({ "name": invocation.name, "text": arguments, "delivery": "steer" });
            return Post::Command {
                id: id.to_string(),
                body,
                fallback: prompt,
            };
        }
        Post::Prompt {
            id: id.to_string(),
            body: prompt,
        }
    }
}

/// Every POST in the order the driver made it, each once the one before is answered. The first
/// waits for a stop an earlier driver of this session still has in flight.
fn spawn_posts(
    server: &Arc<OpenCodeServer>,
    session: &str,
    core: &Shared,
    mut queued: mpsc::UnboundedReceiver<Post>,
) {
    let server = server.clone();
    let session = session.to_string();
    let tasks = server.clone();
    // Weak: the core holds this queue's sender, so a strong hold would keep both, and the
    // server, alive after the driver is gone.
    let core = Arc::downgrade(core);
    tasks.spawn(async move {
        if let Some(mut stopped) = server.stop_in_flight(&session) {
            let _ = tokio::time::timeout(STOP_WAIT * 2, stopped.wait_for(|done| *done)).await;
        }
        while let Some(post) = queued.recv().await {
            send_post(&server, &session, &core, post).await;
        }
    });
}

async fn send_post(
    server: &OpenCodeServer,
    session: &str,
    core: &Weak<StdMutex<Core>>,
    post: Post,
) {
    let with_core = |change: &dyn Fn(&mut Core)| {
        if let Some(core) = core.upgrade() {
            change(&mut lock(&core));
        }
    };
    let post_json =
        |path: String, body: Value| server.request(reqwest::Method::POST, &path).json(&body);
    let sent = |request: reqwest::RequestBuilder| async move {
        request
            .timeout(RESYNC_WAIT)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
    };
    match post {
        Post::Prompt { id, body } => {
            match sent(post_json(format!("/session/{session}/prompt"), body)).await {
                Ok(_) => with_core(&|c| c.admitted(&id)),
                Err(e) => with_core(&|c| c.post_failed(&id, &e.to_string())),
            }
        }
        Post::Command { id, body, fallback } => {
            with_core(&|c| c.turn.adopting.push(id.clone()));
            let ran = sent(post_json(format!("/session/{session}/command"), body)).await;
            if ran.is_ok() {
                with_core(&|c| c.admitted(&id));
            }
            if let Err(e) = ran {
                tracing::warn!("opencode command failed; sending it as a plain prompt: {e}");
                with_core(&|c| c.turn.adopting.retain(|adopting| *adopting != id));
                match sent(post_json(format!("/session/{session}/prompt"), fallback)).await {
                    Ok(_) => with_core(&|c| c.admitted(&id)),
                    Err(e) => with_core(&|c| c.post_failed(&id, &e.to_string())),
                }
            }
        }
        Post::Interrupt { session: target } => {
            let request = server
                .request(
                    reqwest::Method::POST,
                    &format!("/session/{target}/interrupt"),
                )
                .query(&[("resume", "false")]);
            match sent(request).await {
                Ok(resp) => {
                    let body: Value = resp.json().await.unwrap_or(Value::Null);
                    let interrupted = body
                        .get("interrupted")
                        .or_else(|| body.pointer("/data/interrupted"))
                        .and_then(Value::as_bool);
                    if target == session && interrupted == Some(false) {
                        with_core(&|c| c.stop_missed());
                    }
                }
                Err(e) => tracing::warn!("opencode interrupt of {target} failed: {e}"),
            }
        }
        Post::CancelInbox { id } => {
            let path = format!("/session/{session}/inbox/{id}");
            if let Err(e) = sent(server.request(reqwest::Method::DELETE, &path)).await {
                tracing::warn!("opencode inbox cancel of {id} failed: {e}");
                with_core(&|c| c.cancel_failed(&id));
            }
        }
        Post::Wake { id } => {
            let path = format!("/session/{session}/inbox/{id}");
            let wake = server
                .request(reqwest::Method::PATCH, &path)
                .json(&json!({ "delivery": "steer" }));
            if let Err(e) = sent(wake).await {
                tracing::warn!("opencode inbox wake of {id} failed: {e}");
            }
        }
        Post::Model { model } => {
            let body = json!({ "model": model });
            if let Err(e) = sent(post_json(format!("/session/{session}/model"), body)).await {
                tracing::warn!("opencode model switch failed: {e}");
            }
        }
        Post::PermissionReply {
            owner,
            id,
            decision,
        } => {
            let path = format!("/session/{owner}/permission/{id}/reply");
            if let Err(e) = sent(post_json(path, json!({ "decision": decision }))).await {
                tracing::warn!("opencode permission reply {id} failed: {e}");
            }
        }
        Post::FormReply { owner, id, answer } => {
            let path = format!("/session/{owner}/form/{id}/reply");
            if let Err(e) = sent(post_json(path, json!({ "answer": answer }))).await {
                tracing::warn!("opencode form reply {id} failed: {e}");
            }
        }
        Post::FormCancel { owner, id } => {
            let path = format!("/session/{owner}/form/{id}");
            if let Err(e) = sent(server.request(reqwest::Method::DELETE, &path)).await {
                tracing::warn!("opencode form cancel {id} failed: {e}");
            }
        }
    }
}

/// Reads the server's event stream for this session until the driver ends or the server exits.
fn spawn_filter(
    server: &Arc<OpenCodeServer>,
    core: Shared,
    events: broadcast::Sender<SessionEvent>,
) -> tokio::task::JoinHandle<()> {
    let mut sse = server.subscribe();
    let mut exit = server.exit_watch();
    let reader = server.clone();
    let translate = async move {
        let mut older: usize = 0;
        loop {
            match sse.recv().await {
                Ok(ev) if ev.kind != "server.connected" => {
                    lock(&core).take(&ev, older > 0);
                    older = older.saturating_sub(1);
                }
                // OpenCode starts every event stream with it, so one here follows a gap.
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    let queued = sse.len();
                    let (root, family) = {
                        let core = lock(&core);
                        (core.root.clone(), core.family.clone())
                    };
                    let seen = read_back(&reader, &root, &family).await;
                    lock(&core).resync(seen);
                    older = queued;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    let exited = async move {
        let exited = exit.wait_for(Option::is_some).await.ok()?;
        exited.clone()
    };
    server.spawn(async move {
        tokio::select! {
            biased;
            () = translate => {}
            Some(exited) = exited => {
                let _ = events.send(exited);
            }
        }
    })
}

async fn read_back(server: &OpenCodeServer, root: &str, family: &HashSet<String>) -> ReadBack {
    let get = |path: String| async move {
        let response = server
            .request(reqwest::Method::GET, &path)
            .timeout(RESYNC_WAIT)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        response.json::<Value>().await.ok()
    };
    let data = |v: Option<Value>| -> Option<Vec<Value>> {
        v?.get("data").and_then(Value::as_array).cloned()
    };
    let messages = format!("/session/{root}/message?limit={RESYNC_MESSAGES}&order=desc");
    let (inbox, active, messages) = tokio::join!(
        get(format!("/session/{root}/inbox")),
        get("/session/active".to_string()),
        get(messages),
    );
    let inbox = data(inbox).map(|items| {
        items
            .iter()
            .filter_map(|i| i.get("id").and_then(Value::as_str).map(str::to_string))
            .collect()
    });
    let busy = active
        .as_ref()
        .and_then(|a| a.get("data"))
        .map(|all| all.get(root).is_some());
    let mut asks = Some(Vec::new());
    for member in family {
        let (permissions, forms) = tokio::join!(
            get(format!("/session/{member}/permission")),
            get(format!("/session/{member}/form")),
        );
        match (data(permissions), data(forms), asks.as_mut()) {
            (Some(permissions), Some(forms), Some(listed)) => {
                let id = |v: &Value| {
                    v.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                for p in permissions {
                    listed.push((id(&p), member.clone(), Listed::Permission(p)));
                }
                for f in forms {
                    listed.push((id(&f), member.clone(), Listed::Form(f)));
                }
            }
            _ => asks = None,
        }
    }
    ReadBack {
        inbox,
        busy,
        messages: data(messages),
        asks,
    }
}

impl AgentDriver for OpenCodeDriver {
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    fn send_user_turn(
        &self,
        text: &str,
        stage: Option<&str>,
        prompt_name: Option<&str>,
        client_message_id: Option<&str>,
    ) -> CoreResult<()> {
        let echo = SessionEvent::UserMessage {
            text: text.to_string(),
            stage: stage.map(str::to_string),
            prompt_name: prompt_name.map(str::to_string),
            client_message_id: client_message_id.map(str::to_string),
            author: None,
        };
        let id = message_id();
        let post = self.invocation_body(&id, text, prompt_name);
        let mut core = lock(&self.core);
        core.send(id, echo, client_message_id.is_some());
        core.post(post);
        Ok(())
    }

    fn cancel_queued_message<'a>(
        &'a self,
        client_message_id: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CoreResult<bool>> + Send + 'a>> {
        Box::pin(async move {
            let held = {
                let core = lock(&self.core);
                core.turn
                    .undelivered
                    .iter()
                    .find_map(|(id, echo)| match echo {
                        Some(SessionEvent::UserMessage {
                            client_message_id: Some(cmid),
                            ..
                        }) if cmid == client_message_id => Some(id.clone()),
                        _ => None,
                    })
            };
            let Some(id) = held else {
                return Ok(false);
            };
            lock(&self.core).turn.withdrawing.insert(id.clone());
            let path = format!("/session/{}/inbox/{id}", self.session_id);
            let deleted = self
                .server
                .request(reqwest::Method::DELETE, &path)
                .timeout(RESYNC_WAIT)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            // OpenCode answers the delete of a message it already read as a no-op, so only its
            // `inbox.cancelled` says the withdrawal won.
            let deadline = tokio::time::Instant::now() + STOP_WAIT;
            while deleted.is_ok() && tokio::time::Instant::now() < deadline {
                if !lock(&self.core)
                    .turn
                    .undelivered
                    .iter()
                    .any(|(sent, _)| *sent == id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let mut core = lock(&self.core);
            core.turn.withdrawing.remove(&id);
            Ok(core.turn.withdrawn.remove(&id))
        })
    }

    fn set_model(&self, model: Option<String>) {
        let parsed = model.as_deref().and_then(parse_model);
        if let Ok(mut m) = self.model.lock() {
            *m = parsed.clone();
        }
        if let Some(model) = parsed {
            lock(&self.core).post(Post::Model {
                model: model_ref(&model),
            });
        }
    }

    fn respond_permission(
        &self,
        request_id: &str,
        allow: bool,
        _message: Option<&str>,
    ) -> CoreResult<()> {
        let mut core = lock(&self.core);
        let owner = match core.take_ask(request_id, allow) {
            Some(ask) => ask.owner,
            None => core.root.clone(),
        };
        let decision = if allow { "once" } else { "reject" };
        core.post(Post::PermissionReply {
            owner,
            id: request_id.to_string(),
            decision,
        });
        Ok(())
    }

    fn answer_question(&self, request_id: &str, answers: QuestionAnswers) -> CoreResult<()> {
        let mut core = lock(&self.core);
        let (owner, fields) = match core.take_ask(request_id, true) {
            Some(Ask {
                owner,
                kind: AskKind::Form(fields),
            }) => (owner, fields),
            Some(Ask { owner, .. }) => (owner, Vec::new()),
            None => (core.root.clone(), Vec::new()),
        };
        let id = request_id.to_string();
        if answers.is_empty() {
            core.post(Post::FormCancel { owner, id });
        } else {
            let answer = form_answer(&fields, &answers);
            core.post(Post::FormReply { owner, id, answer });
        }
        Ok(())
    }

    fn pending_requests(&self) -> Option<Vec<String>> {
        Some(lock(&self.core).asks.keys().cloned().collect())
    }

    fn interrupt(&self) -> CoreResult<()> {
        lock(&self.core).stop();
        Ok(())
    }

    fn usage_so_far(&self) -> UsageSoFar {
        UsageSoFar {
            cost_usd: Some(lock(&self.core).xlate.cost_total),
            thread_tokens: None,
        }
    }

    fn kill(&self) {
        if self.kill_flag.mark_killed() {
            self.end(ExitOutcome::Killed);
        }
    }

    fn kill_detached(&self) {
        if self.kill_flag.mark_detached() {
            self.end(ExitOutcome::Detached);
        }
    }

    fn pid(&self) -> Option<u32> {
        None
    }

    fn provider_settings_outdated(&self) -> bool {
        self.server.retired()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillBase {
    Claude,
    Codex,
}

fn skill_base_for_model(model: Option<&(String, String)>) -> SkillBase {
    let Some((provider, model)) = model else {
        return SkillBase::Codex;
    };
    let id = format!("{provider}/{model}").to_ascii_lowercase();
    if provider.eq_ignore_ascii_case("anthropic") || id.contains("claude") {
        SkillBase::Claude
    } else {
        SkillBase::Codex
    }
}

struct Invocation<'a> {
    name: &'a str,
    arguments: &'a str,
}

fn leading_invocation<'a>(text: &'a str, prompt_name: Option<&str>) -> Option<Invocation<'a>> {
    // Custom prompts have already been expanded by the app. Their bodies are
    // ordinary user text even if the first line happens to resemble a skill.
    if prompt_name.is_some() {
        return None;
    }
    let text = text.trim_start();
    let rest = text.strip_prefix('/').or_else(|| text.strip_prefix('$'))?;
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }
    Some(Invocation {
        name,
        arguments: rest[name_end..].trim_start(),
    })
}

fn routed_skill_arguments(base: SkillBase, arguments: &str) -> String {
    let (platform, agent_source) = match base {
        SkillBase::Claude => ("Claude", "the matching .claude/agents/<name>.md persona"),
        SkillBase::Codex => (
            "Codex",
            "the matching .codex/agents/<name>.toml persona, or ~/.claude/agents/<name>.md when no TOML exists",
        ),
    };
    let routing = format!(
        "<hyprlayer-skill-routing>\n\
         Base platform: {platform}. Where this skill distinguishes harness behavior, follow its \
         {platform} path. OpenCode is only the execution harness: if it cannot load a named \
         {platform} agent directly, read {agent_source} and carry those instructions into a \
         generic subagent.\n\
         </hyprlayer-skill-routing>"
    );
    if arguments.is_empty() {
        routing
    } else {
        format!("{routing}\n\n{arguments}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> (
        Core,
        broadcast::Receiver<SessionEvent>,
        mpsc::UnboundedReceiver<Post>,
    ) {
        let (tx, rx) = broadcast::channel(256);
        let (posts, queued) = mpsc::unbounded_channel();
        let core = Core {
            root: "ses_root".into(),
            family: HashSet::from(["ses_root".to_string()]),
            turn: Turn::default(),
            asks: HashMap::new(),
            xlate: Translator::default(),
            tx,
            posts,
            since: 0,
        };
        (core, rx, queued)
    }

    fn ev(kind: &str, data: Value) -> OcEvent {
        let mut data = data;
        if data.get("sessionID").is_none() && data.get("form").is_none() {
            data["sessionID"] = json!("ses_root");
        }
        OcEvent::new(kind, data)
    }

    fn drain(rx: &mut broadcast::Receiver<SessionEvent>) -> Vec<SessionEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn names(events: &[SessionEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                SessionEvent::UserMessage { text, .. } => format!("echo:{text}"),
                SessionEvent::TurnDone { turn_status, .. } => {
                    format!("done:{}", turn_status.clone().unwrap_or_default())
                }
                SessionEvent::CommandLifecycle { state, .. } => format!("lifecycle:{state}"),
                SessionEvent::AssistantText { text, .. } => format!("text:{text}"),
                SessionEvent::PermissionAsk { request_id, .. } => format!("ask:{request_id}"),
                SessionEvent::QuestionAsk { request_id, .. } => format!("form:{request_id}"),
                SessionEvent::PermissionResolved {
                    request_id, allow, ..
                } => {
                    format!("resolved:{request_id}:{allow}")
                }
                SessionEvent::QuestionAnswered { request_id } => format!("answered:{request_id}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    fn echo(text: &str, cmid: Option<&str>) -> SessionEvent {
        SessionEvent::UserMessage {
            text: text.into(),
            stage: None,
            prompt_name: None,
            client_message_id: cmid.map(str::to_string),
            author: None,
        }
    }

    fn posts(queued: &mut mpsc::UnboundedReceiver<Post>) -> Vec<String> {
        std::iter::from_fn(|| queued.try_recv().ok())
            .map(|p| match p {
                Post::Interrupt { session } => format!("interrupt:{session}"),
                Post::CancelInbox { id } => format!("cancel:{id}"),
                Post::Wake { id } => format!("wake:{id}"),
                Post::PermissionReply { id, decision, .. } => format!("reply:{id}:{decision}"),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn a_turn_ends_once_on_the_run_that_read_it() {
        let (mut c, mut rx, _q) = core();
        // A run from before this driver sent ends nothing.
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(&ev("session.execution.succeeded", json!({})), false);
        assert_eq!(
            names(&drain(&mut rx)),
            ["done:completed"],
            "a background wake closes itself"
        );

        c.send("msg_1".into(), echo("one", None), false);
        c.take(
            &ev("session.execution.interrupted", json!({ "reason": "user" })),
            false,
        );
        assert_eq!(
            names(&drain(&mut rx)),
            ["echo:one"],
            "an earlier run's end is not this turn's"
        );
        for (kind, data) in [
            (
                "session.inbox.enqueued",
                json!({ "inboxID": "msg_1", "item": { "type": "user" } }),
            ),
            ("session.execution.started", json!({})),
            ("session.inbox.delivered", json!({ "inboxID": "msg_1" })),
            (
                "session.text.ended",
                json!({ "assistantMessageID": "msg_a", "ordinal": 0, "text": "hi" }),
            ),
            ("session.execution.succeeded", json!({})),
            ("session.execution.succeeded", json!({})),
        ] {
            c.take(&ev(kind, data), false);
        }
        assert_eq!(names(&drain(&mut rx)), ["text:hi", "done:completed"]);

        c.send("msg_2".into(), echo("two", None), false);
        for (kind, data) in [
            ("session.execution.started", json!({})),
            ("session.inbox.delivered", json!({ "inboxID": "msg_2" })),
            (
                "session.execution.failed",
                json!({ "error": { "type": "x", "message": "boom" } }),
            ),
        ] {
            c.take(&ev(kind, data), false);
        }
        assert_eq!(
            names(&drain(&mut rx)),
            ["echo:two", "text:⚠ OpenCode error: boom", "done:failed"]
        );
    }

    #[test]
    fn a_steer_echoes_when_read_and_a_stop_hands_back_what_it_withdrew() {
        let (mut c, mut rx, mut q) = core();
        c.send("msg_1".into(), echo("one", Some("c1")), true);
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_1" })),
            false,
        );
        c.send("msg_2".into(), echo("two", Some("c2")), true);
        c.send("msg_3".into(), echo("three", Some("c3")), true);
        assert_eq!(
            names(&drain(&mut rx)),
            ["echo:one"],
            "a steer's echo waits for its delivery"
        );
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_2" })),
            false,
        );
        assert_eq!(names(&drain(&mut rx)), ["echo:two"]);

        // The stop withdraws the one still held; it is handed back once OpenCode cancels it.
        c.stop();
        assert_eq!(posts(&mut q), ["cancel:msg_3", "interrupt:ses_root"]);
        c.take(
            &ev("session.execution.interrupted", json!({ "reason": "user" })),
            false,
        );
        assert!(
            drain(&mut rx).is_empty(),
            "the end waits for the withdrawal"
        );
        c.take(
            &ev("session.inbox.cancelled", json!({ "inboxID": "msg_3" })),
            false,
        );
        assert_eq!(
            names(&drain(&mut rx)),
            ["lifecycle:cancelled", "done:interrupted"],
            "the hand-back comes ahead of the TurnDone the relay resends at"
        );
        assert!(c.turn.undelivered.is_empty());

        // A steer that wins the race with the stop's withdrawal echoes, once, and is not handed back.
        c.send("msg_4".into(), echo("four", Some("c4")), true);
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_4" })),
            false,
        );
        c.send("msg_5".into(), echo("five", Some("c5")), true);
        c.stop();
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_5" })),
            false,
        );
        c.take(
            &ev("session.inbox.cancelled", json!({ "inboxID": "msg_5" })),
            false,
        );
        c.take(
            &ev("session.execution.interrupted", json!({ "reason": "user" })),
            false,
        );
        assert_eq!(
            names(&drain(&mut rx)),
            ["echo:four", "echo:five", "done:interrupted"]
        );
    }

    #[test]
    fn a_steer_sent_as_the_run_ends_opens_the_next_turn_when_read() {
        let (mut c, mut rx, _q) = core();
        c.send("msg_1".into(), echo("one", Some("c1")), true);
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_1" })),
            false,
        );
        c.send("msg_2".into(), echo("two", Some("c2")), true);
        c.take(&ev("session.execution.succeeded", json!({})), false);
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_2" })),
            false,
        );
        c.take(&ev("session.execution.succeeded", json!({})), false);
        assert_eq!(
            names(&drain(&mut rx)),
            ["echo:one", "done:completed", "echo:two", "done:completed"]
        );
    }

    #[test]
    fn a_stop_that_finds_no_run_ends_the_turn_and_one_asked_mid_start_is_sent_again() {
        let (mut c, mut rx, mut q) = core();
        c.send("msg_1".into(), echo("one", None), false);
        c.stop();
        assert_eq!(posts(&mut q), ["cancel:msg_1", "interrupt:ses_root"]);
        c.stop_missed();
        assert_eq!(names(&drain(&mut rx)), ["echo:one", "done:interrupted"]);
        c.take(
            &ev("session.inbox.cancelled", json!({ "inboxID": "msg_1" })),
            false,
        );
        c.take(&ev("session.execution.started", json!({})), false);
        assert!(posts(&mut q).is_empty(), "a later run is not this stop's");
        c.take(&ev("session.execution.succeeded", json!({})), false);
        drain(&mut rx);
        // A stop asked as the run starts is sent again once the driver sees it start.
        c.send("msg_2".into(), echo("two", None), false);
        c.turn.stopping = true;
        c.take(&ev("session.execution.started", json!({})), false);
        assert_eq!(posts(&mut q), ["interrupt:ses_root"]);
    }

    #[test]
    fn a_run_that_never_read_the_turn_wakes_opencode_for_it_once() {
        let (mut c, mut rx, mut q) = core();
        c.take(&ev("session.execution.started", json!({})), false);
        c.send("msg_1".into(), echo("one", None), false);
        c.take(
            &ev(
                "session.execution.failed",
                json!({ "error": { "message": "x" } }),
            ),
            false,
        );
        assert!(
            posts(&mut q).is_empty(),
            "a prompt OpenCode has not answered is not woken"
        );
        c.admitted("msg_1");
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev(
                "session.execution.failed",
                json!({ "error": { "message": "x" } }),
            ),
            false,
        );
        assert_eq!(names(&drain(&mut rx)), ["echo:one"]);
        assert_eq!(posts(&mut q), ["wake:msg_1"]);

        // A failed run that read the turn leaves a steer it never read parked: it is steered again.
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev("session.inbox.delivered", json!({ "inboxID": "msg_1" })),
            false,
        );
        c.send("msg_2".into(), echo("two", Some("c2")), true);
        c.admitted("msg_2");
        c.take(
            &ev(
                "session.execution.failed",
                json!({ "error": { "message": "x" } }),
            ),
            false,
        );
        assert_eq!(posts(&mut q), ["wake:msg_2"]);
        assert_eq!(
            names(&drain(&mut rx)),
            ["text:⚠ OpenCode error: x", "done:failed"]
        );

        // The run the wake started fails too: the steer is withdrawn and handed back, ahead of
        // that run's end.
        c.take(&ev("session.execution.started", json!({})), false);
        c.take(
            &ev(
                "session.execution.failed",
                json!({ "error": { "message": "x" } }),
            ),
            false,
        );
        assert_eq!(posts(&mut q), ["cancel:msg_2"]);
        assert_eq!(names(&drain(&mut rx)), ["text:⚠ OpenCode error: x"]);
        c.take(
            &ev("session.inbox.cancelled", json!({ "inboxID": "msg_2" })),
            false,
        );
        assert_eq!(
            names(&drain(&mut rx)),
            ["lifecycle:cancelled", "done:failed"]
        );
    }

    #[test]
    fn asks_close_once_and_an_ended_run_drops_its_own() {
        let (mut c, mut rx, mut q) = core();
        c.send("msg_1".into(), echo("one", None), false);
        c.admitted("msg_1");
        c.take(
            &ev(
                "session.created",
                json!({ "sessionID": "ses_kid", "parentID": "ses_root" }),
            ),
            false,
        );
        let perm = |id: &str, owner: &str| {
            ev(
                "permission.asked",
                json!({ "id": id, "sessionID": owner, "action": "read", "resources": [] }),
            )
        };
        c.take(&perm("per_1", "ses_root"), false);
        c.take(&perm("per_1", "ses_root"), false);
        c.take(&perm("per_2", "ses_kid"), false);
        c.take(&perm("per_3", "ses_stranger"), false);
        c.take(
            &ev(
                "form.created",
                json!({ "form": { "id": "frm_1", "sessionID": "ses_root", "fields": [] } }),
            ),
            false,
        );
        assert_eq!(
            names(&drain(&mut rx)),
            ["echo:one", "ask:per_1", "ask:per_2", "form:frm_1"]
        );

        c.take(
            &ev(
                "permission.replied",
                json!({ "requestID": "per_1", "reply": "reject" }),
            ),
            false,
        );
        c.take(
            &ev(
                "permission.replied",
                json!({ "requestID": "per_1", "reply": "reject" }),
            ),
            false,
        );
        assert_eq!(names(&drain(&mut rx)), ["resolved:per_1:false"]);

        // The root's run ending drops its form, not the subagent's ask, which waits on.
        c.take(
            &ev("session.execution.interrupted", json!({ "reason": "user" })),
            false,
        );
        assert_eq!(names(&drain(&mut rx)), ["answered:frm_1"]);
        c.take(
            &ev(
                "session.execution.succeeded",
                json!({ "sessionID": "ses_kid" }),
            ),
            false,
        );
        assert_eq!(names(&drain(&mut rx)), ["resolved:per_2:false"]);
        assert_eq!(
            posts(&mut q),
            ["wake:msg_1"],
            "the root's run never read the turn"
        );
    }

    #[test]
    fn a_read_back_settles_what_the_gap_hid() {
        let (mut c, mut rx, _q) = core();
        c.send("msg_1".into(), echo("one", None), false);
        c.take(&ev("session.execution.started", json!({})), false);
        c.send("msg_2".into(), echo("two", Some("c2")), true);
        c.send("msg_3".into(), echo("three", Some("c3")), true);
        c.asks.insert(
            "per_gone".into(),
            Ask {
                owner: "ses_root".into(),
                kind: AskKind::Permission,
            },
        );
        for id in ["msg_1", "msg_2", "msg_3"] {
            c.admitted(id);
        }
        c.send("msg_4".into(), echo("four", Some("c4")), true);
        drain(&mut rx);
        let messages = vec![
            json!({ "id": "msg_i", "type": "idle", "outcome": "succeeded", "time": { "created": 9 } }),
            json!({ "id": "msg_b", "type": "assistant", "time": { "created": 8 }, "content": [{ "type": "text", "text": "done" }] }),
            json!({ "id": "msg_2", "type": "user", "time": { "created": 7 } }),
            json!({ "id": "msg_1", "type": "user", "time": { "created": 6 } }),
        ];
        c.resync(ReadBack {
            inbox: Some(vec![]),
            busy: Some(false),
            messages: Some(messages),
            asks: Some(vec![(
                "per_new".into(),
                "ses_root".into(),
                Listed::Permission(json!({ "id": "per_new", "action": "read" })),
            )]),
        });
        assert_eq!(
            names(&drain(&mut rx)),
            [
                "echo:two",
                "lifecycle:cancelled",
                "text:done",
                "resolved:per_gone:false",
                "done:completed",
                "ask:per_new"
            ]
        );
        assert!(
            c.turn.undelivered.iter().any(|(id, _)| id == "msg_4"),
            "a message OpenCode has not answered the prompt of is not taken for withdrawn"
        );
    }

    #[test]
    fn events_queued_before_a_read_back_do_not_undo_it() {
        let (mut c, mut rx, _q) = core();
        c.send("msg_1".into(), echo("one", None), false);
        c.admitted("msg_1");
        drain(&mut rx);
        c.resync(ReadBack {
            inbox: Some(vec![]),
            busy: Some(true),
            messages: Some(vec![
                json!({ "id": "msg_1", "type": "user", "time": { "created": 1 } }),
            ]),
            asks: Some(vec![]),
        });
        // Queued before the read: an ask it found gone, and the end of a run before the one going.
        c.take(
            &ev(
                "permission.asked",
                json!({ "id": "per_old", "action": "read" }),
            ),
            true,
        );
        c.take(&ev("session.execution.succeeded", json!({})), true);
        assert!(drain(&mut rx).is_empty());
        assert!(c.asks.is_empty() && c.turn.open && c.turn.executing);
        c.take(&ev("session.execution.succeeded", json!({})), false);
        assert_eq!(names(&drain(&mut rx)), ["done:completed"]);
    }
}
