//! Server-launched PTY agents: hook signals folded into state, broadcast as `PtyAgentChanged`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hyprlayer_models::{
    AgentSignal, AgentStatusReport, PtyAgent, PtyAgentState, RuntimeEvent, UiRequest,
};
use tokio::sync::broadcast;

use super::codex_remote::CodexRemote;
use super::links::AgentLinks;
use super::pty_controls::{footer_mode, ready};
use super::rollout::RolloutTail;
use super::TokenGuard;
use crate::runtime::terminal_registry::TerminalRegistry;
use crate::terminal::screen::Screen;
use crate::terminal::{PtySession, RingReader};

pub(crate) const CLAUDE_ENGINE: &str = "claude-code";
const ACTIVITY_THROTTLE: Duration = Duration::from_millis(500);
const SETTLE_QUIET: Duration = Duration::from_secs(2);
const SETTLE_GIVE_UP: Duration = Duration::from_secs(30);
/// Title model calls per terminal, so a broken one isn't called on every prompt.
const TITLE_TRIES: u8 = 3;
/// claude runs its statusLine just before it paints the footer, so a report follows the screen this long.
const FOOTER_FOLLOW: Duration = Duration::from_millis(1500);
/// One screen read per burst: claude streams a repaint over many writes.
const FOOTER_COALESCE: Duration = Duration::from_millis(100);

pub(crate) struct PtyLaunchEntry {
    pub agent: PtyAgent,
    pub session: Arc<PtySession>,
    pub token: Option<TokenGuard>,
    pub remote: Option<Arc<CodexRemote>>,
}

/// What a launch's codex supports.
#[derive(Clone, Copy)]
pub(crate) struct CodexGate {
    pub hooks: bool,
    /// Its TUI can run over an app-server hyprlayer starts (`--remote`, `app-server --listen`).
    // Only a unix launch reads it.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub remote: bool,
}

struct Entry {
    agent: PtyAgent,
    session: Arc<PtySession>,
    /// Every signal bumps it, so a settle can tell one arrived after the input that armed it.
    signals: u64,
    /// The tools whose asks are open, so only one of their own `PostToolUse`s ends an ask.
    asks: Vec<ToolKey>,
    /// `Some` while a settle task runs; each input re-arms it rather than spawning another.
    settle: Option<SettleArm>,
    emitted_at: Option<Instant>,
    trailing: bool,
    /// An agent's paste the hooks haven't acknowledged yet; until then the agent reads as working.
    prompt_owed_until: Option<Instant>,
    /// claude is at a prompt it shows before its session starts, as its folder-trust one.
    startup_prompt: bool,
    /// While a title is being generated, so a later prompt doesn't start a second.
    title_busy: bool,
    title_tries: u8,
    /// Shared with the blocking reader, which holds it for the whole read so reads stay in order.
    rollout: Arc<Mutex<RolloutTail>>,
    /// Hooked claude only: what the change driver reads.
    screen: Option<Arc<Screen>>,
    /// Held by an agent's message or a change from the bar, so their keystrokes never interleave.
    input: Arc<tokio::sync::Mutex<()>>,
    /// Until when a footer follower reads the screen; each report extends it rather than spawning another.
    footer_watch: Option<Instant>,
    /// While set, `PreModelSwitch` lets claude switch without its "Switch model?" dialog.
    switch_pending: bool,
    /// The model a `PostModelSwitch` named while a switch was pending; plan mode's own switch
    /// follows it at once, so the status alone can't confirm it.
    switched_to: Option<String>,
    /// The app-server a PTY codex runs over; its API, not the hooks or rollout, reports the settings.
    remote: Option<Arc<CodexRemote>>,
    /// Held only to revoke on drop, when the entry goes.
    _token: Option<TokenGuard>,
}

/// A tool call as its hooks name it: `PermissionRequest` carries no tool-use id to match on.
type ToolKey = (String, Option<String>);

#[derive(Clone, Copy)]
struct SettleArm {
    signals: u64,
    at: Instant,
}

pub struct PtyAgents {
    entries: Mutex<HashMap<String, Entry>>,
    /// Whether a codex runs over an app-server, by command and version.
    codex_remotes: Mutex<HashMap<(String, [u64; 3]), bool>>,
    events: broadcast::Sender<RuntimeEvent>,
    terminals: Arc<TerminalRegistry>,
    links: Arc<AgentLinks>,
    launch_root: PathBuf,
}

/// One launch as the agent-control verbs see it, unhooked ones included.
pub(crate) struct PtySnapshot {
    pub agent: PtyAgent,
    /// Bumps on every hook signal, so a wait can tell a quiet agent from one still reporting.
    pub signals: u64,
    pub prompt_owed: bool,
    pub session: Arc<PtySession>,
    pub screen: Option<Arc<Screen>>,
    pub input: Arc<tokio::sync::Mutex<()>>,
    pub remote: Option<Arc<CodexRemote>>,
}

impl PtyAgents {
    /// Prunes the launch dirs, and the app-servers and sockets, a crashed server left; call only
    /// while holding the profile lock.
    pub(crate) fn new(
        app_data_dir: &Path,
        runtime_dir: Option<&Path>,
        events: broadcast::Sender<RuntimeEvent>,
        terminals: Arc<TerminalRegistry>,
        links: Arc<AgentLinks>,
    ) -> Self {
        let launch_root = app_data_dir.join("agent-control").join("launch");
        if let Err(e) = std::fs::remove_dir_all(&launch_root) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("could not prune {}: {e}", launch_root.display());
            }
        }
        if let Some(dir) = runtime_dir {
            prune_app_servers(dir);
        }
        Self {
            entries: Mutex::new(HashMap::new()),
            codex_remotes: Mutex::new(HashMap::new()),
            events,
            terminals,
            links,
            launch_root,
        }
    }

    /// Writes `<launch dir>/<name>` readable by this user only; the exit watcher deletes the dir.
    pub(crate) fn write_launch_file(
        &self,
        terminal_id: &str,
        name: &str,
        contents: &[u8],
    ) -> std::io::Result<PathBuf> {
        let dir = self.launch_root.join(terminal_id);
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            builder.mode(0o700);
            options.mode(0o600);
        }
        builder.create(&dir)?;
        let path = dir.join(name);
        std::io::Write::write_all(&mut options.open(&path)?, contents)?;
        Ok(path)
    }

    pub(crate) fn remove_launch_dir(&self, terminal_id: &str) {
        let dir = self.launch_root.join(terminal_id);
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("could not remove {}: {e}", dir.display());
            }
        }
    }

    pub(crate) fn codex_remote(&self, command: &str, version: [u64; 3]) -> Option<bool> {
        lock(&self.codex_remotes)
            .get(&(command.to_string(), version))
            .copied()
    }

    pub(crate) fn set_codex_remote(&self, command: &str, version: [u64; 3], remote: bool) {
        lock(&self.codex_remotes).insert((command.to_string(), version), remote);
    }

    /// Tracks the launch until its PTY exits, then revokes its token and drops the terminal.
    pub(crate) fn register(self: &Arc<Self>, launch: PtyLaunchEntry) {
        let PtyLaunchEntry {
            mut agent,
            session,
            token,
            remote,
        } = launch;
        let screen = (agent.hooked && agent.engine_id == CLAUDE_ENGINE).then(|| session.screen());
        agent.status.controllable = screen.is_some() || remote.is_some();
        let terminal_id = agent.terminal_id.clone();
        let mut reader = session.subscribe();
        {
            let mut entries = lock(&self.entries);
            let entry = entries.entry(terminal_id.clone()).or_insert(Entry {
                agent,
                session,
                signals: 0,
                asks: Vec::new(),
                settle: None,
                emitted_at: None,
                trailing: false,
                prompt_owed_until: None,
                startup_prompt: false,
                title_busy: false,
                title_tries: 0,
                rollout: Arc::default(),
                screen,
                input: Arc::default(),
                footer_watch: None,
                switch_pending: false,
                switched_to: None,
                remote,
                _token: token,
            });
            self.emit(entry);
        }
        let agents = self.clone();
        tokio::spawn(async move {
            while reader.skip().await {}
            agents.release(&terminal_id);
            agents.terminals.remove(&terminal_id).await;
        });
    }

    /// Marks the agent exited and frees its app-server, lineage, token and launch dir; idempotent.
    pub(crate) fn release(&self, terminal_id: &str) {
        // Only a registered id reaches the filesystem: `close` passes the client's string.
        let Some(mut entry) = lock(&self.entries).remove(terminal_id) else {
            return;
        };
        if let Some(remote) = &entry.remote {
            remote.stop();
        }
        if entry.agent.state != PtyAgentState::Exited {
            entry.agent.state = PtyAgentState::Exited;
            self.emit(&mut entry);
        }
        self.links.unlink(terminal_id);
        self.remove_launch_dir(terminal_id);
    }

    /// Hooked agents, and unhooked ones an agent spawned so a desktop can still adopt them.
    pub fn list(&self, workstream_id: Option<&str>) -> Vec<PtyAgent> {
        lock(&self.entries)
            .values()
            .filter(|e| e.agent.hooked || e.agent.spawned_by.is_some())
            .filter(|e| workstream_id.is_none_or(|ws| e.agent.workstream_id == ws))
            .map(|e| e.agent.clone())
            .collect()
    }

    pub(crate) fn snapshot(&self, terminal_id: &str) -> Option<PtySnapshot> {
        lock(&self.entries).get(terminal_id).map(|e| PtySnapshot {
            agent: e.agent.clone(),
            signals: e.signals,
            prompt_owed: e
                .prompt_owed_until
                .is_some_and(|until| Instant::now() < until),
            session: e.session.clone(),
            screen: e.screen.clone(),
            input: e.input.clone(),
            remote: e.remote.clone(),
        })
    }

    /// A codex app-server's report, folded like a hook's.
    #[cfg(unix)]
    pub(crate) fn fold_remote_status(
        self: &Arc<Self>,
        terminal_id: &str,
        change: impl FnOnce(&mut hyprlayer_models::PtyAgentStatus),
    ) {
        let mut entries = lock(&self.entries);
        if let Some(entry) = entries.get_mut(terminal_id) {
            let before = entry.agent.clone();
            change(&mut entry.agent.status);
            self.publish(entry, terminal_id, &before);
        }
    }

    /// Kills the PTY; its exit watcher then marks the agent exited and cleans up.
    #[cfg(unix)]
    pub(crate) fn end_terminal(&self, terminal_id: &str) {
        let session = lock(&self.entries)
            .get(terminal_id)
            .map(|e| e.session.clone());
        if let Some(session) = session {
            session.kill();
        }
    }

    /// For shutdown, which doesn't wait for the exit watchers.
    pub(crate) fn stop_app_servers(&self) {
        for entry in lock(&self.entries).values() {
            if let Some(remote) = &entry.remote {
                remote.stop();
            }
        }
    }

    pub(crate) fn set_switch_pending(&self, terminal_id: &str, pending: bool) {
        if let Some(entry) = lock(&self.entries).get_mut(terminal_id) {
            entry.switch_pending = pending;
            if pending {
                entry.switched_to = None;
            }
        }
    }

    pub(crate) fn switched_to(&self, terminal_id: &str) -> Option<String> {
        lock(&self.entries)
            .get(terminal_id)
            .and_then(|e| e.switched_to.clone())
    }

    pub(crate) fn switch_pending(&self, terminal_id: &str) -> bool {
        lock(&self.entries)
            .get(terminal_id)
            .is_some_and(|e| e.switch_pending)
    }

    /// A mode the change driver read off claude's footer; no hook reports a shift+tab.
    pub(crate) fn set_permission_mode(self: &Arc<Self>, terminal_id: &str, mode: &str) {
        let mut entries = lock(&self.entries);
        if let Some(entry) = entries.get_mut(terminal_id) {
            let before = entry.agent.clone();
            entry.agent.status.permission_mode = Some(mode.to_string());
            self.publish(entry, terminal_id, &before);
        }
    }

    /// Marks a prompt as on its way for at most `within`; any hook signal but `SessionStart` settles it.
    pub(crate) fn owe_prompt(&self, terminal_id: &str, within: Duration) {
        if let Some(entry) = lock(&self.entries).get_mut(terminal_id) {
            entry.prompt_owed_until = Some(Instant::now() + within);
        }
    }

    pub(crate) fn clear_prompt_owed(&self, terminal_id: &str) {
        if let Some(entry) = lock(&self.entries).get_mut(terminal_id) {
            entry.prompt_owed_until = None;
        }
    }

    /// A prompt before the session starts reads as a permission ask; it needs a person to answer it.
    pub(crate) fn startup_prompt(self: &Arc<Self>, terminal_id: &str, showing: bool) {
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get_mut(terminal_id) else {
            return;
        };
        let before = entry.agent.clone();
        if showing && entry.agent.state == PtyAgentState::Starting {
            entry.startup_prompt = true;
            entry.agent.state = PtyAgentState::AwaitingPermission;
        } else if !showing && std::mem::take(&mut entry.startup_prompt) {
            unprompt(&mut entry.agent);
        }
        self.publish(entry, terminal_id, &before);
    }

    pub(crate) fn apply(self: &Arc<Self>, terminal_id: &str, signal: &AgentSignal) {
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get_mut(terminal_id) else {
            tracing::debug!("a {} hook signal for a gone terminal", signal.event);
            return;
        };
        entry.signals += 1;
        // `SessionStart` comes before a spawn's first prompt is even pasted.
        if signal.event != "SessionStart" {
            entry.prompt_owed_until = None;
        }
        // `auto` is claude's own switch on entering or leaving plan mode, not the picker's.
        if entry.switch_pending
            && signal.event == "PostModelSwitch"
            && signal.source.as_deref() != Some("auto")
        {
            entry.switched_to = signal.to_model.clone();
        }
        let before = entry.agent.clone();
        if std::mem::take(&mut entry.startup_prompt) {
            unprompt(&mut entry.agent);
        }
        let remote = entry.remote.clone();
        fold(&mut entry.agent, &mut entry.asks, signal);
        let hook_mode = remote.as_ref().is_none_or(|r| !r.mode_reported());
        fold_status(&mut entry.agent, signal, hook_mode);
        let rollout = signal
            .transcript_path
            .as_ref()
            .filter(|_| entry.agent.engine_id == "codex" && remote.is_none())
            .map(|path| (entry.rollout.clone(), PathBuf::from(path)));
        self.publish(entry, terminal_id, &before);
        drop(entries);
        if let Some((tail, path)) = rollout {
            self.read_rollout(terminal_id, tail, path);
        }
        let started = signal
            .session_id
            .as_deref()
            .filter(|_| signal.event == "SessionStart");
        if let (Some(remote), Some(thread)) = (remote, started) {
            remote.adopt_thread(thread);
        }
    }

    /// Whether this terminal still wants a title: a hooked agent with none, none in progress, and
    /// tries left. `release_title` ends a claim that stored none.
    pub(crate) fn claim_title(&self, terminal_id: &str) -> bool {
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get_mut(terminal_id) else {
            return false;
        };
        if !entry.agent.hooked
            || entry.agent.title.is_some()
            || entry.title_busy
            || entry.title_tries >= TITLE_TRIES
        {
            return false;
        }
        entry.title_busy = true;
        true
    }

    /// Spends one of the terminal's title model calls; false once they are gone.
    pub(crate) fn take_title_try(&self, terminal_id: &str) -> bool {
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get_mut(terminal_id) else {
            return false;
        };
        if entry.title_tries >= TITLE_TRIES {
            return false;
        }
        entry.title_tries += 1;
        true
    }

    pub(crate) fn release_title(&self, terminal_id: &str) {
        if let Some(entry) = lock(&self.entries).get_mut(terminal_id) {
            entry.title_busy = false;
        }
    }

    pub(crate) fn set_title(self: &Arc<Self>, terminal_id: &str, title: String) {
        let mut entries = lock(&self.entries);
        if let Some(entry) = entries.get_mut(terminal_id) {
            let before = entry.agent.clone();
            entry.agent.title = Some(title);
            entry.title_busy = false;
            self.publish(entry, terminal_id, &before);
        }
    }

    /// A statusLine run: model, effort and context as claude reported them, then the mode its footer shows.
    pub(crate) fn report_status(self: &Arc<Self>, terminal_id: &str, report: AgentStatusReport) {
        let mode = self.footer(terminal_id);
        let reader = {
            let mut entries = lock(&self.entries);
            let Some(entry) = entries.get_mut(terminal_id) else {
                tracing::debug!("a status report for a gone terminal");
                return;
            };
            let before = entry.agent.clone();
            let status = &mut entry.agent.status;
            status.model = report.model;
            status.effort = report.effort;
            status.context = report.context;
            if let Some(mode) = mode {
                status.permission_mode = Some(mode.to_string());
            }
            self.publish(entry, terminal_id, &before);
            let live = entry.footer_watch.is_some();
            entry.footer_watch = Some(Instant::now() + FOOTER_FOLLOW);
            if live || entry.screen.is_none() {
                return;
            }
            entry.session.subscribe()
        };
        let agents = self.clone();
        let id = terminal_id.to_string();
        tokio::spawn(async move { agents.follow_footer(reader, &id).await });
    }

    /// Rereads the footer on each output until the watch lapses.
    async fn follow_footer(self: Arc<Self>, mut reader: RingReader, terminal_id: &str) {
        loop {
            let left = match lock(&self.entries)
                .get(terminal_id)
                .and_then(|e| e.footer_watch)
            {
                Some(until) => until.saturating_duration_since(Instant::now()),
                None => return,
            };
            let output = tokio::time::timeout(left, reader.skip()).await;
            if matches!(output, Ok(true)) {
                tokio::time::sleep(FOOTER_COALESCE).await;
                if let Some(mode) = self.footer(terminal_id) {
                    self.set_permission_mode(terminal_id, mode);
                }
            }
            let mut entries = lock(&self.entries);
            let Some(entry) = entries.get_mut(terminal_id) else {
                return;
            };
            let lapsed = entry
                .footer_watch
                .is_none_or(|until| Instant::now() >= until);
            if matches!(output, Ok(false)) || lapsed {
                entry.footer_watch = None;
                return;
            }
        }
    }

    /// The footer is the only sign of a typed shift+tab; `None` while a picker hides it. Read off the lock.
    fn footer(&self, terminal_id: &str) -> Option<&'static str> {
        let screen = lock(&self.entries).get(terminal_id)?.screen.clone()?;
        screen.rows().as_deref().and_then(footer_mode)
    }

    /// Off the async runtime: a first read walks the whole rollout.
    fn read_rollout(
        self: &Arc<Self>,
        terminal_id: &str,
        tail: Arc<Mutex<RolloutTail>>,
        path: PathBuf,
    ) {
        let agents = self.clone();
        let id = terminal_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut tail = lock(&tail);
            let delta = match tail.read(&path) {
                Ok(delta) => delta,
                Err(e) => {
                    tracing::debug!("codex rollout {} unread: {e}", path.display());
                    return;
                }
            };
            let mut entries = lock(&agents.entries);
            if let Some(entry) = entries.get_mut(&id) {
                let before = entry.agent.clone();
                delta.apply(&mut entry.agent.status);
                agents.publish(entry, &id, &before);
            }
        });
    }

    /// Emits a change at once, except one only in activity or status, which goes out at most every 500 ms.
    fn publish(self: &Arc<Self>, entry: &mut Entry, terminal_id: &str, before: &PtyAgent) {
        if entry.agent == *before {
            return;
        }
        if asking(entry.agent.state) && !asking(before.state) {
            self.notify_asking(&entry.agent);
        }
        let quiet_only = PtyAgent {
            activity: before.activity.clone(),
            status: before.status.clone(),
            ..entry.agent.clone()
        } == *before;
        let throttled = entry
            .emitted_at
            .map(|at| at.elapsed())
            .filter(|since| *since < ACTIVITY_THROTTLE);
        match throttled {
            Some(since) if quiet_only => {
                if !entry.trailing {
                    entry.trailing = true;
                    let agents = self.clone();
                    let id = terminal_id.to_string();
                    tokio::spawn(async move {
                        tokio::time::sleep(ACTIVITY_THROTTLE - since).await;
                        let mut entries = lock(&agents.entries);
                        if let Some(entry) = entries.get_mut(&id).filter(|e| e.trailing) {
                            agents.emit(entry);
                        }
                    });
                }
            }
            _ => self.emit(entry),
        }
    }

    /// Arms the post-deny settle: claude fires no `Stop` after a deny or Esc.
    pub(crate) fn note_input(self: &Arc<Self>, terminal_id: &str) {
        let reader = {
            let mut entries = lock(&self.entries);
            let Some(entry) = entries.get_mut(terminal_id) else {
                return;
            };
            if !entry.agent.hooked || !settles(entry.agent.state, entry.screen.is_some()) {
                return;
            }
            let live = entry.settle.is_some();
            entry.settle = Some(SettleArm {
                signals: entry.signals,
                at: Instant::now(),
            });
            if live {
                return;
            }
            entry.session.subscribe()
        };
        let agents = self.clone();
        let id = terminal_id.to_string();
        tokio::spawn(async move { agents.settle(reader, &id).await });
    }

    /// 2 s of no output and no signal since the last input → idle, once claude's screen shows its
    /// input box; gives up after 30 s of output.
    async fn settle(&self, mut reader: RingReader, terminal_id: &str) {
        loop {
            let quiet = tokio::time::timeout(SETTLE_QUIET, reader.skip()).await;
            let mut entries = lock(&self.entries);
            let Some(entry) = entries.get_mut(terminal_id) else {
                return;
            };
            let Some(arm) = entry.settle else {
                return;
            };
            let stale =
                entry.signals != arm.signals || !settles(entry.agent.state, entry.screen.is_some());
            // A person moving a prompt's highlight is input too; the prompt is still open.
            let boxed = entry
                .screen
                .as_ref()
                .is_none_or(|screen| screen.rows().is_some_and(|rows| ready(&rows)));
            match quiet {
                Ok(true) if !stale && arm.at.elapsed() < SETTLE_GIVE_UP => continue,
                Err(_) if !stale && boxed => {
                    entry.agent.state = PtyAgentState::Idle;
                    entry.asks.clear();
                    self.emit(entry);
                }
                _ => {}
            }
            entry.settle = None;
            return;
        }
    }

    /// An agent-spawned PTY may sit in a tab no desktop has shown, so no runtime there notifies (F20's remedy).
    fn notify_asking(&self, agent: &PtyAgent) {
        if agent.spawned_by.is_none() {
            return;
        }
        let body = match agent.state {
            PtyAgentState::AwaitingQuestion => "It is asking a question.",
            _ => "It is waiting on a permission prompt.",
        };
        let _ = self.events.send(RuntimeEvent::UiRequested {
            request: UiRequest::Notify {
                workstream_id: agent.workstream_id.clone(),
                title: format!("{} agent needs you", agent.engine_id),
                body: body.into(),
                session_id: None,
                terminal_id: Some(agent.terminal_id.clone()),
                from_lead: false,
            },
        });
    }

    fn emit(&self, entry: &mut Entry) {
        entry.emitted_at = Some(Instant::now());
        entry.trailing = false;
        if !entry.agent.hooked {
            return;
        }
        let a = &entry.agent;
        let _ = self.events.send(RuntimeEvent::PtyAgentChanged {
            terminal_id: a.terminal_id.clone(),
            workstream_id: a.workstream_id.clone(),
            engine_id: a.engine_id.clone(),
            state: a.state,
            provider_session_id: a.provider_session_id.clone(),
            activity: a.activity.clone(),
            last_reply: a.last_reply.clone(),
            title: a.title.clone(),
            status: a.status.clone(),
        });
    }
}

pub(super) fn asking(state: PtyAgentState) -> bool {
    matches!(
        state,
        PtyAgentState::AwaitingPermission | PtyAgentState::AwaitingQuestion
    )
}

/// A settle may have idled it since.
fn unprompt(agent: &mut PtyAgent) {
    if agent.state == PtyAgentState::AwaitingPermission {
        agent.state = PtyAgentState::Starting;
    }
}

/// Without a screen to read (codex), a quiet ask can't be told from a denied one.
fn settles(state: PtyAgentState, mirrored: bool) -> bool {
    match state {
        PtyAgentState::Working => true,
        PtyAgentState::AwaitingPermission => mirrored,
        _ => false,
    }
}

fn fold(agent: &mut PtyAgent, asks: &mut Vec<ToolKey>, signal: &AgentSignal) {
    let was_asking = asking(agent.state);
    let tool = || {
        signal
            .tool_name
            .clone()
            .map(|name| (name, signal.tool_target.clone()))
    };
    let tool_activity = || {
        signal
            .tool_name
            .as_ref()
            .map(|tool| match &signal.tool_target {
                Some(target) => format!("{tool} {target}"),
                None => tool.clone(),
            })
    };
    match signal.event.as_str() {
        "SessionStart" => {
            // Exited too: `/clear` ends one session and starts the next in the same process.
            if matches!(agent.state, PtyAgentState::Starting | PtyAgentState::Exited) {
                agent.state = PtyAgentState::Idle;
            }
            if signal.session_id.is_some() {
                agent.provider_session_id = signal.session_id.clone();
            }
        }
        "UserPromptSubmit" => {
            agent.state = PtyAgentState::Working;
            agent.activity = None;
        }
        "PreToolUse" if signal.tool_name.as_deref() == Some("AskUserQuestion") => {
            agent.state = PtyAgentState::AwaitingQuestion;
            asks.extend(tool());
        }
        // A parallel or subagent tool's hooks don't answer the ask on screen; its own result does.
        "PreToolUse" if was_asking => {}
        "PostToolUse" if was_asking => {
            if let Some(at) = tool().and_then(|key| asks.iter().position(|a| *a == key)) {
                asks.remove(at);
                if asks.is_empty() {
                    agent.state = PtyAgentState::Working;
                    agent.activity = tool_activity();
                }
            }
        }
        "PreToolUse" | "PostToolUse" => {
            agent.state = PtyAgentState::Working;
            agent.activity = tool_activity();
        }
        "PermissionRequest" => {
            agent.state = PtyAgentState::AwaitingPermission;
            asks.extend(tool());
        }
        "Notification" if was_asking => {}
        "Notification" => match signal.notification_type.as_deref() {
            Some("permission_prompt") => agent.state = PtyAgentState::AwaitingPermission,
            Some("idle_prompt") => agent.state = PtyAgentState::Idle,
            _ => {}
        },
        "Stop" => {
            agent.state = PtyAgentState::Idle;
            if signal.last_assistant_message.is_some() {
                agent.last_reply = signal.last_assistant_message.clone();
            }
        }
        "Interrupt" => agent.state = PtyAgentState::Idle,
        "StopFailure" => {
            agent.state = PtyAgentState::Idle;
            agent.activity = signal.reason.clone();
        }
        // The token stays live until the PTY exits: the process can outlive its session.
        "SessionEnd" => {
            agent.state = PtyAgentState::Exited;
            agent.activity = signal.reason.clone();
        }
        _ => {}
    }
    if !asking(agent.state) {
        asks.clear();
    }
}

/// What the hooks say about the agent's settings; claude's `effort` rides tool and Stop hooks, codex's `model` most.
/// A remote codex's hooks know only default or bypass, so its app-server's mode stands once reported.
fn fold_status(agent: &mut PtyAgent, signal: &AgentSignal, hook_mode: bool) {
    let status = &mut agent.status;
    if let Some(mode) = signal.permission_mode.as_ref().filter(|_| hook_mode) {
        status.permission_mode = Some(mode.clone());
    }
    if let Some(effort) = &signal.effort {
        status.effort = Some(effort.clone());
    }
    let model = match signal.event.as_str() {
        "PostModelSwitch" => &signal.to_model,
        _ => &signal.model,
    };
    if let Some(model) = model {
        status.model = Some(model.clone());
    }
}

/// Ends the app-servers a crashed server left, by their pid files, then drops their sockets.
fn prune_app_servers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    let has = |path: &Path, ext: &str| path.extension().is_some_and(|e| e == ext);
    for pid_file in paths.iter().filter(|p| has(p, "pid")) {
        crate::process::orphan::reap(pid_file);
    }
    for socket in paths.iter().filter(|p| has(p, "sock")) {
        if let Err(e) = std::fs::remove_file(socket) {
            tracing::warn!("could not prune {}: {e}", socket.display());
        }
    }
}

/// Only xterm's own answers to the TUI's queries: they are not a human at the keyboard.
pub(crate) fn is_terminal_reply(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    while !rest.is_empty() {
        match reply_len(rest) {
            Some(n) => rest = &rest[n..],
            None => return false,
        }
    }
    !bytes.is_empty()
}

fn reply_len(b: &[u8]) -> Option<usize> {
    const ESC: u8 = 0x1b;
    match b {
        [ESC, b'[', body @ ..] => {
            let end = body.iter().position(|c| (0x40..=0x7e).contains(c))?;
            let (params, last) = (&body[..end], body[end]);
            let reply = match last {
                // Cursor-position report.
                b'R' => !params.is_empty(),
                b'c' => matches!(params.first(), Some(b'?' | b'>')),
                // Status reports, the colour-scheme one (`? 997 ; N`) included; no key ends in `n`.
                b'n' => true,
                b'I' | b'O' => params.is_empty(),
                // DECRQM, private or ANSI.
                b'y' => params.last() == Some(&b'$'),
                _ => false,
            };
            reply.then_some(end + 3)
        }
        [ESC, b']' | b'P', body @ ..] => {
            let end = body.iter().enumerate().find_map(|(i, c)| match c {
                0x07 => Some(i + 1),
                &ESC if body.get(i + 1) == Some(&b'\\') => Some(i + 2),
                _ => None,
            })?;
            Some(end + 2)
        }
        _ => None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
