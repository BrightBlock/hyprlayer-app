//! Changes a hooked claude PTY's model, effort and mode from the status bar, session-only, by
//! driving its TUI as a person would (recipe observed on claude 2.1.322 to 2.1.330). A digit or
//! Enter in a picker, and a typed `/model <x>` or `/effort <x>`, each save a default (the model one
//! on the user's claude account), so none is ever sent. A codex over its own app-server goes
//! through the API instead.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use hyprlayer_models::{PtyAgentState, PtyAgentStatus, PtyControlOutcome};
use regex::Regex;
use tokio::sync::OwnedMutexGuard;
use tokio::time::Instant;

use super::codex_remote::CodexRemote;
use super::pty_agents::{asking, PtyAgents};
use crate::error::{CoreError, Result};
use crate::runtime::HyprlayerRuntime;
use crate::terminal::screen::Screen;
use crate::terminal::PtySession;

const TYPE_GAP: Duration = Duration::from_millis(20);
const BEFORE_ENTER: Duration = Duration::from_millis(400);
const PICKER_OPEN: Duration = Duration::from_secs(3);
/// A TUI busy running a statusLine reads its input late; the Enter waits for the echo.
const ECHO_WAIT: Duration = Duration::from_secs(2);
/// How long a read waits for a screen it knows, or a key for what it moved: a loaded machine
/// paints late, and a read mid-repaint sees no picker at all.
const SCREEN_WAIT: Duration = Duration::from_secs(5);
const CONFIRM: Duration = Duration::from_secs(5);
const DIALOG_CONFIRM: Duration = Duration::from_secs(3);
const PLAN_CONFIRM: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);
const MAX_PRESSES: usize = 6;

/// A picker footer's hint for `s`, the only part the driver relies on: 2.1.330 reorders the rest
/// and renames Enter's action to "save". The second is `/effort`'s before 2.1.330. The leading
/// space keeps a note like `Set model to opus for this session only` from reading as one.
const SESSION_HINTS: [&str; 2] = [" s to use this session only", " s for this session only"];
/// The `/model` picker's footer while "More models" is highlighted.
const MORE_FOOTER: &str = "Enter to expand · Esc to cancel";
const MODEL_TITLE: &str = "Select model";
const SWITCH_DIALOG: &str = "Switch model?";
const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

const UP: &[u8] = b"\x1b[A";
const DOWN: &[u8] = b"\x1b[B";
const LEFT: &[u8] = b"\x1b[D";
const RIGHT: &[u8] = b"\x1b[C";
const SHIFT_TAB: &[u8] = b"\x1b[Z";
const ESC: &[u8] = b"\x1b";
const ENTER: &[u8] = b"\r";
const BACKSPACE: &[u8] = b"\x7f";
const SESSION_ONLY: &[u8] = b"s";

static MODEL_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:❯\s+)?\d\.\s+(?P<label>\S.*?)(?:\s+✔)?\s{2,}(?P<hint>\S.*)$")
        .expect("model row regex")
});
static FOOTER_MODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:⏸|⏵⏵) (manual mode|accept edits|plan mode|auto mode|bypass permissions) on")
        .expect("footer mode regex")
});
/// Each footer phrase and the mode key claude's hooks use for it.
const FOOTER_MODES: [(&str, &str); 5] = [
    ("manual mode", "default"),
    ("accept edits", "acceptEdits"),
    ("plan mode", "plan"),
    ("auto mode", "auto"),
    ("bypass permissions", "bypassPermissions"),
];
static DATE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-\d{8}(-v\d+)?$").expect("date suffix regex"));

pub struct PtyControlService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn pty_controls(&self) -> PtyControlService<'_> {
        PtyControlService { runtime: self }
    }
}

impl PtyControlService<'_> {
    /// `/model`, the row, then `s`; `Relaunch` when claude never confirmed it and sits idle.
    pub async fn set_model(&self, terminal_id: &str, model: &str) -> Result<PtyControlOutcome> {
        if let Some(remote) = self.remote(terminal_id) {
            remote.set_model(model).await?;
            return Ok(PtyControlOutcome::Applied);
        }
        let label = display_name(model)
            .ok_or_else(|| failed(format!("claude's /model list has no name for {model}")))?;
        let drive = self.begin(terminal_id).await?;
        if drive.reported(|s| s.model.as_deref() == Some(model)) {
            return Ok(PtyControlOutcome::Applied);
        }
        // Confirmation reads the name, so it must differ from the model's now (`[1m]` shares it).
        if drive.reported(|s| on_model(s, &label)) {
            return Err(failed(format!(
                "{model} shows as {label} in claude's /model list, like the model it runs now"
            )));
        }
        drive.command("/model").await?;
        let picked = async {
            if !drive
                .until(PICKER_OPEN, |rows| model_picker(rows).is_some())
                .await?
            {
                return Err(failed("claude's /model picker didn't open"));
            }
            drive.highlight(&label).await?;
            // An earlier switch's note for this model would confirm this one before claude acts.
            let screen_confirms = !switch_noted(&drive.rows()?, &label);
            let _pending = PendingSwitch::arm(&drive.agents, terminal_id);
            drive
                .key_soon(
                    |rows| highlighted(rows).as_deref() == Some(label.as_str()),
                    SESSION_ONLY,
                )
                .await?;
            drive.confirm_model(&label, screen_confirms).await
        }
        .await;
        match picked {
            Ok(true) => return Ok(PtyControlOutcome::Applied),
            Ok(false) => drive.dismiss().await,
            Err(e) => {
                drive.dismiss().await;
                return Err(e);
            }
        }
        match self.runtime.pty_agents.snapshot(terminal_id) {
            Some(now)
                if now.agent.state == PtyAgentState::Idle
                    && now.agent.provider_session_id.is_some() =>
            {
                Ok(PtyControlOutcome::Relaunch)
            }
            _ => Err(failed(format!(
                "claude didn't confirm the switch to {label}"
            ))),
        }
    }

    /// `/effort`, the slider, then `s`; confirmed by the statusLine's `effort.level`.
    pub async fn set_effort(&self, terminal_id: &str, effort: &str) -> Result<()> {
        if let Some(remote) = self.remote(terminal_id) {
            return remote.set_effort(effort).await;
        }
        if !EFFORT_LEVELS.contains(&effort) {
            return Err(failed(format!("claude's /effort has no {effort} level")));
        }
        let drive = self.begin(terminal_id).await?;
        if drive.reported(|s| s.effort.as_deref() == Some(effort)) {
            return Ok(());
        }
        drive.command("/effort").await?;
        let set = async {
            if !drive
                .until(PICKER_OPEN, |rows| effort_slider(rows).is_some())
                .await?
            {
                return Err(failed("claude's /effort picker didn't open"));
            }
            drive.slide_to(effort).await?;
            drive
                .key_soon(
                    |rows| {
                        effort_slider(rows).and_then(|s| s.selected()).as_deref() == Some(effort)
                    },
                    SESSION_ONLY,
                )
                .await?;
            Ok(drive
                .until_reported(CONFIRM, |s| s.effort.as_deref() == Some(effort))
                .await)
        }
        .await;
        match set {
            Ok(true) => Ok(()),
            Ok(false) => {
                drive.dismiss().await;
                Err(failed(format!("claude didn't confirm effort {effort}")))
            }
            Err(e) => {
                drive.dismiss().await;
                Err(e)
            }
        }
    }

    /// `mode` is claude's own key, `auto` included; `/plan` for plan, else shift+tab round its ring.
    pub async fn set_permission_mode(&self, terminal_id: &str, mode: &str) -> Result<()> {
        if let Some(remote) = self.remote(terminal_id) {
            return remote.set_permission_mode(mode).await;
        }
        let want = FOOTER_MODES
            .iter()
            .map(|(_, key)| *key)
            .find(|key| *key == mode)
            .ok_or_else(|| failed(format!("claude has no {mode} permission mode")))?;
        let drive = self.begin(terminal_id).await?;
        let start = drive
            .settled_some(footer_mode)
            .await?
            .ok_or_else(|| failed("claude's footer shows no permission mode"))?;
        let reached = if start == want {
            true
        } else if want == "plan" {
            drive.command("/plan").await?;
            drive
                .until(PLAN_CONFIRM, |rows| footer_mode(rows) == Some(want))
                .await?
        } else {
            drive.cycle_to(start, want).await?
        };
        // Every mode shift+tab passed through took effect, so the bar shows where it stopped.
        if let Ok(Some(now)) = drive.settled_some(footer_mode).await {
            self.runtime
                .pty_agents
                .set_permission_mode(terminal_id, now);
        }
        if reached {
            Ok(())
        } else {
            Err(failed(format!(
                "claude's shift+tab cycle never reached {want} in this session"
            )))
        }
    }

    /// No keystroke reaches a remote codex's TUI, so neither its input lock nor an ask stops a change.
    fn remote(&self, terminal_id: &str) -> Option<Arc<CodexRemote>> {
        self.runtime
            .pty_agents
            .snapshot(terminal_id)
            .and_then(|s| s.remote)
    }

    /// Holds the terminal's input lock, then refuses an asking agent or a screen that isn't its input box.
    async fn begin(&self, terminal_id: &str) -> Result<Drive> {
        let agents = self.runtime.pty_agents.clone();
        let gone = || failed("that agent is no longer running");
        let target = agents.snapshot(terminal_id).ok_or_else(gone)?;
        let screen = target
            .screen
            .ok_or_else(|| failed("only a hooked claude terminal can be changed from the bar"))?;
        let input = target
            .input
            .try_lock_owned()
            .map_err(|_| failed("that agent is taking another change or a message; try again"))?;
        // Read under the lock: a message that held it may have moved the agent on.
        let state = agents.snapshot(terminal_id).ok_or_else(gone)?.agent.state;
        if asking(state) {
            return Err(failed(
                "that agent is asking you something; answer it first",
            ));
        }
        let drive = Drive {
            agents,
            pty: target.session,
            screen,
            terminal_id: terminal_id.to_string(),
            _input: input,
        };
        drive
            .until(SCREEN_WAIT, |rows| ready(rows) || covered(rows))
            .await?;
        if drive.settled_some(|rows| painted(rows, ready)).await? != Some(true) {
            return Err(failed("the agent is showing a prompt"));
        }
        // `command` types onto the input line, so a person's draft would be sent or wiped.
        let (rows, bright) = drive.rows_and_bright()?;
        if has_draft(&rows, &bright) {
            return Err(failed(
                "the agent's input holds a draft; clear your draft first",
            ));
        }
        Ok(drive)
    }
}

struct Drive {
    agents: Arc<PtyAgents>,
    pty: Arc<PtySession>,
    screen: Arc<Screen>,
    terminal_id: String,
    _input: OwnedMutexGuard<()>,
}

impl Drive {
    fn rows(&self) -> Result<Vec<String>> {
        self.screen
            .rows()
            .ok_or_else(|| failed("the agent's screen can't be read"))
    }

    fn rows_and_bright(&self) -> Result<(Vec<String>, Vec<String>)> {
        self.screen
            .rows_and_bright()
            .ok_or_else(|| failed("the agent's screen can't be read"))
    }

    /// Straight to the PTY: no hop reset and no settle arming, since no person typed it.
    fn key(&self, bytes: &[u8]) -> Result<()> {
        self.pty.write_input(bytes)?;
        Ok(())
    }

    /// `bytes` once `on` holds for the screen, within `SCREEN_WAIT`, while the agent isn't asking: a
    /// prompt can open between any two keys.
    async fn key_soon(&self, on: impl Fn(&[String]) -> bool, bytes: &[u8]) -> Result<()> {
        when_shown(|| self.read_unless_asking(), on).await?;
        self.key(bytes)
    }

    /// The screen, or `None` while the agent asks.
    fn read_unless_asking(&self) -> Result<Option<Vec<String>>> {
        if self.asking() {
            return Ok(None);
        }
        self.rows().map(Some)
    }

    fn asking(&self) -> bool {
        self.agents
            .snapshot(&self.terminal_id)
            .is_none_or(|s| asking(s.agent.state))
    }

    fn reported(&self, check: impl Fn(&PtyAgentStatus) -> bool) -> bool {
        self.agents
            .snapshot(&self.terminal_id)
            .is_some_and(|s| check(&s.agent.status))
    }

    async fn until(&self, within: Duration, check: impl Fn(&[String]) -> bool) -> Result<bool> {
        let deadline = Instant::now() + within;
        loop {
            if check(&self.rows()?) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(POLL).await;
        }
    }

    async fn until_reported(
        &self,
        within: Duration,
        check: impl Fn(&PtyAgentStatus) -> bool,
    ) -> bool {
        let deadline = Instant::now() + within;
        loop {
            if self.reported(&check) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// `read` once it shows something and two polls agree, or `None` after `SCREEN_WAIT`: a
    /// picker read mid-repaint is gone for a moment, not closed.
    async fn settled_some<T: PartialEq>(
        &self,
        read: impl Fn(&[String]) -> Option<T>,
    ) -> Result<Option<T>> {
        let deadline = Instant::now() + SCREEN_WAIT;
        let mut last = None;
        loop {
            let now = read(&self.rows()?);
            if now.is_some() && now == last {
                return Ok(now);
            }
            if Instant::now() >= deadline {
                return Ok(now.or(last));
            }
            last = now;
            tokio::time::sleep(POLL).await;
        }
    }

    /// One key once `on` holds, then the new value `read` settles on, or the old one when nothing
    /// moved within `SCREEN_WAIT`.
    async fn press<T: PartialEq>(
        &self,
        key: &[u8],
        on: impl Fn(&[String]) -> bool,
        read: impl Fn(&[String]) -> Option<T>,
    ) -> Result<Option<T>> {
        let before = self.settled_some(&read).await?;
        self.key_soon(on, key).await?;
        let deadline = Instant::now() + SCREEN_WAIT;
        loop {
            tokio::time::sleep(POLL).await;
            let now = read(&self.rows()?);
            if now.is_some() && now != before {
                return self.settled_some(&read).await;
            }
            if Instant::now() >= deadline {
                return Ok(before);
            }
        }
    }

    /// Types a bare slash command onto the empty input line and submits it only once the line
    /// reads exactly that.
    async fn command(&self, command: &str) -> Result<()> {
        for (at, ch) in command.char_indices() {
            if let Err(e) = self.key_soon(ready, ch.to_string().as_bytes()).await {
                self.erase(&command[..at]).await;
                return Err(e);
            }
            tokio::time::sleep(TYPE_GAP).await;
        }
        tokio::time::sleep(BEFORE_ENTER).await;
        let shows = |rows: &[String]| shows_line(rows, command);
        let typed = self.until(ECHO_WAIT, shows).await?
            && self.settled_some(|rows| painted(rows, shows)).await? == Some(true);
        if typed {
            return self.key_soon(shows, ENTER).await;
        }
        Err(failed(if self.erase(command).await {
            format!("the agent's input didn't take {command}, so nothing was sent")
        } else {
            format!("the agent's input didn't take {command}, so nothing was sent; it may still hold part of it")
        }))
    }

    /// An Enter on anything else could submit a draft or pick a row. The line started empty, so
    /// once it shows exactly `typed`, that many backspaces remove only what was typed.
    async fn erase(&self, typed: &str) -> bool {
        if typed.is_empty() {
            return true;
        }
        if !matches!(
            self.until(ECHO_WAIT, |rows| shows_line(rows, typed)).await,
            Ok(true)
        ) {
            return false;
        }
        for _ in typed.chars() {
            if self.key_soon(ready, BACKSPACE).await.is_err() {
                return false;
            }
        }
        true
    }

    /// Arrows wrap through every row and "More models", one press per re-read. The route never
    /// lands on "More models": Enter there would expand it, and claude's footer changes under it.
    async fn highlight(&self, label: &str) -> Result<()> {
        for pressed in 0..=MAX_PRESSES {
            let picker = self
                .settled_some(|rows| model_picker(rows).filter(|p| p.highlighted.is_some()))
                .await?
                .ok_or_else(|| failed("claude's /model picker closed"))?;
            let target = picker
                .labels
                .iter()
                .position(|l| l == label)
                .ok_or_else(|| failed(format!("{label} isn't in claude's /model list")))?;
            let at = picker
                .highlighted
                .ok_or_else(|| failed("claude's /model picker shows no highlighted row"))?;
            if at == target {
                return Ok(());
            }
            let more = picker.more.then_some(picker.labels.len());
            if Some(at) == more {
                return Err(failed("claude's /model picker landed on More models"));
            }
            if pressed == MAX_PRESSES {
                break;
            }
            let ring = picker.labels.len() + usize::from(picker.more);
            let down = (target + ring - at) % ring;
            // Exactly one way round passes "More models"; with none, the shorter way.
            let key = match more {
                Some(m) if (m + ring - at) % ring < down => UP,
                Some(_) => DOWN,
                None if down <= ring - down => DOWN,
                None => UP,
            };
            self.press(
                key,
                |rows| model_picker(rows).is_some(),
                |rows| model_picker(rows).and_then(|p| p.highlighted),
            )
            .await?;
        }
        Err(failed(format!(
            "claude's /model picker didn't reach {label}"
        )))
    }

    /// Waits for claude's `PostModelSwitch`, its statusLine or (with `screen_confirms`) its note to
    /// name the model; answers a "Switch model?" dialog the hook didn't skip.
    async fn confirm_model(&self, label: &str, screen_confirms: bool) -> Result<bool> {
        let mut deadline = Instant::now() + CONFIRM;
        let mut answered = false;
        loop {
            let switched = self
                .agents
                .switched_to(&self.terminal_id)
                .and_then(|id| display_name(&id));
            let rows = self.rows()?;
            if switched.as_deref() == Some(label)
                || self.reported(|s| on_model(s, label))
                || (screen_confirms && switch_noted(&rows, label))
            {
                return Ok(true);
            }
            if !answered && switch_dialog_for(&rows, label) {
                self.key_soon(|rows| switch_dialog_for(rows, label), ENTER)
                    .await?;
                answered = true;
                deadline = deadline.max(Instant::now() + DIALOG_CONFIRM);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// ←/→ one level a press, each read back before the next.
    async fn slide_to(&self, effort: &str) -> Result<()> {
        let at = |rows: &[String]| effort_slider(rows).and_then(|s| s.selected_index());
        let on = |rows: &[String]| effort_slider(rows).is_some();
        let closed = || failed("claude's /effort picker closed");
        let slider = self.settled_some(effort_slider).await?.ok_or_else(closed)?;
        let target = slider
            .levels
            .iter()
            .position(|(level, _)| level == effort)
            .ok_or_else(|| failed(format!("this model has no {effort} effort")))?;
        let mut now = self.settled_some(at).await?.ok_or_else(closed)?;
        for _ in 0..slider.levels.len() {
            if now == target {
                return Ok(());
            }
            let key = if now > target { LEFT } else { RIGHT };
            match self.press(key, on, at).await? {
                Some(moved) if moved != now => now = moved,
                _ => break,
            }
        }
        if now == target {
            return Ok(());
        }
        Err(failed(format!(
            "claude's /effort slider didn't reach {effort}"
        )))
    }

    /// shift+tab one press at a time; back at the start means the ring doesn't hold `want`.
    async fn cycle_to(&self, start: &str, want: &str) -> Result<bool> {
        for _ in 0..MAX_PRESSES {
            let on = |rows: &[String]| ready(rows) && footer_mode(rows).is_some();
            match self.press(SHIFT_TAB, on, footer_mode).await? {
                Some(now) if now == want => return Ok(true),
                Some(now) if now != start => {}
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// One Esc per picker or dialog seen on screen, the next only once that one has gone: an Esc
    /// anywhere else interrupts the turn. A dialog's Esc returns to the picker, so two at most.
    /// A screen mid-repaint shows neither the input box nor a picker, so each read waits for one.
    async fn dismiss(&self) {
        for _ in 0..2 {
            let Ok(Some(Shown::Picker { dialog })) = self.settled_some(shown).await else {
                return;
            };
            if self.key(ESC).is_err() {
                return;
            }
            let gone = self
                .until(DIALOG_CONFIRM, |rows| match shown(rows) {
                    Some(Shown::Input) => true,
                    Some(Shown::Picker { dialog: now }) => now != dialog,
                    None => false,
                })
                .await;
            if !matches!(gone, Ok(true)) {
                break;
            }
        }
        if self.screen.rows().is_some_and(|rows| showing_picker(&rows)) {
            tracing::warn!(
                "claude's picker on terminal {} is still open after Esc",
                self.terminal_id
            );
        }
    }
}

/// Polls `read` until `on` holds, within `SCREEN_WAIT`; `read` gives `None` while the agent asks.
/// The key goes on the read that passed: a second could land mid-repaint and see no screen at all.
async fn when_shown(
    mut read: impl FnMut() -> Result<Option<Vec<String>>>,
    on: impl Fn(&[String]) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + SCREEN_WAIT;
    loop {
        let Some(rows) = read()? else {
            return Err(failed(
                "the agent started asking you something, so the change stopped",
            ));
        };
        if on(&rows) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(failed(
                "claude's screen never showed what the change waits for, so it stopped",
            ));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// While armed, the `PreModelSwitch` hook tells claude to skip its "Switch model?" dialog.
struct PendingSwitch<'a> {
    agents: &'a PtyAgents,
    terminal_id: &'a str,
}

impl<'a> PendingSwitch<'a> {
    fn arm(agents: &'a PtyAgents, terminal_id: &'a str) -> Self {
        agents.set_switch_pending(terminal_id, true);
        Self {
            agents,
            terminal_id,
        }
    }
}

impl Drop for PendingSwitch<'_> {
    fn drop(&mut self) {
        self.agents.set_switch_pending(self.terminal_id, false);
    }
}

#[derive(PartialEq)]
struct ModelPicker {
    /// The model each row picks, as `display_name` names it.
    labels: Vec<String>,
    /// A "More models" row follows the numbered ones in the arrow cycle.
    more: bool,
    /// In cycle order: `labels.len()` is the "More models" row.
    highlighted: Option<usize>,
}

fn model_picker(rows: &[String]) -> Option<ModelPicker> {
    if input_line(rows).is_some() {
        return None;
    }
    let footer = session_footer(rows).max(rfind_wrapped(rows, MORE_FOOTER))?;
    // Neither footer is this picker's alone: 2.1.330's `/effort` offers the same `s`.
    let top = rows[..footer]
        .iter()
        .rposition(|r| r.trim() == MODEL_TITLE)?
        + 1;
    let mut picker = ModelPicker {
        labels: Vec::new(),
        more: false,
        highlighted: None,
    };
    let mut more_lit = false;
    for row in &rows[top..footer] {
        let lit = row.trim_start().starts_with('❯');
        if let Some(caps) = MODEL_ROW.captures(row) {
            if lit {
                picker.highlighted = Some(picker.labels.len());
            }
            picker.labels.push(row_model(&caps["label"], &caps["hint"]));
        } else if row.contains("More models") {
            picker.more = true;
            more_lit = lit;
        }
    }
    if more_lit {
        picker.highlighted = Some(picker.labels.len());
    }
    (!picker.labels.is_empty()).then_some(picker)
}

/// A row's label, or its hint's leading name where the label is only that name's family: logged-out
/// 2.1.326 and 2.1.330 show `Opus   Opus 5.5 · …`.
fn row_model(label: &str, hint: &str) -> String {
    let lead = hint.split(" · ").next().unwrap_or(hint);
    match lead.strip_prefix(label) {
        Some(version) if version.starts_with(' ') => lead.to_string(),
        _ => label.to_string(),
    }
}

#[derive(PartialEq)]
struct EffortSlider {
    /// Each level with its column on the labels row.
    levels: Vec<(String, usize)>,
    /// The `▲` column on the row above.
    marker: Option<usize>,
}

impl EffortSlider {
    fn selected_index(&self) -> Option<usize> {
        let marker = self.marker?;
        (0..self.levels.len()).min_by_key(|&i| {
            let (level, col) = &self.levels[i];
            (col + level.len() / 2).abs_diff(marker)
        })
    }

    fn selected(&self) -> Option<String> {
        Some(self.levels[self.selected_index()?].0.clone())
    }
}

fn effort_slider(rows: &[String]) -> Option<EffortSlider> {
    if input_line(rows).is_some() {
        return None;
    }
    let footer = session_footer(rows)?;
    let (at, levels) = rows[..footer]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, row)| level_row(row).map(|levels| (i, levels)))?;
    let marker = at
        .checked_sub(1)
        .and_then(|i| rows[i].chars().position(|c| c == '▲'));
    Some(EffortSlider { levels, marker })
}

/// `low     medium     high     xhigh      max      Tab to toggle`: the levels before "Tab".
fn level_row(row: &str) -> Option<Vec<(String, usize)>> {
    let chars: Vec<char> = row.chars().collect();
    let mut levels = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        if word == "Tab" {
            break;
        }
        if !EFFORT_LEVELS.contains(&word.as_str()) {
            return None;
        }
        levels.push((word, start));
    }
    (levels.len() >= 2).then_some(levels)
}

/// claude's input box: a `❯` line between two rules, with no picker or dialog anywhere.
pub(super) fn ready(rows: &[String]) -> bool {
    !covered(rows) && input_line(rows).is_some()
}

/// `ready` for a box a multi-line paste may have grown: a `❯` line under a rule, nothing over it.
pub(super) fn composing(rows: &[String]) -> bool {
    !covered(rows)
        && rows
            .windows(2)
            .any(|w| is_rule(&w[0]) && w[1].starts_with('❯'))
}

/// Why a paste into claude's input box would land wrong; `None` when it would land as a message.
pub(super) fn paste_refusal(screen: &Screen) -> Option<&'static str> {
    let Some((rows, bright)) = screen.rows_and_bright() else {
        return Some("the agent's screen can't be read");
    };
    if !ready(&rows) {
        Some("the agent is showing a prompt")
    } else if has_draft(&rows, &bright) {
        Some("the agent's input holds a person's draft, which a message would send")
    } else {
        None
    }
}

/// `check` for the input box or a prompt over it; `None` for a screen mid-repaint, which shows neither.
fn painted(rows: &[String], check: impl Fn(&[String]) -> bool) -> Option<bool> {
    (ready(rows) || covered(rows)).then(|| check(rows))
}

pub(super) fn covered(rows: &[String]) -> bool {
    rows.iter().any(|r| r.contains("Enter to confirm"))
        || rfind_wrapped(rows, "Esc to cancel").is_some()
}

fn is_rule(row: &str) -> bool {
    row.starts_with("────")
}

fn input_row(rows: &[String]) -> Option<usize> {
    rows.windows(3)
        .rposition(|w| is_rule(&w[0]) && w[1].starts_with('❯') && is_rule(&w[2]))
        .map(|at| at + 1)
}

fn input_line(rows: &[String]) -> Option<&str> {
    input_row(rows).map(|at| rows[at].as_str())
}

fn shows_line(rows: &[String], text: &str) -> bool {
    ready(rows) && input_line(rows).map(input_text) == Some(text)
}

/// Text on the input line in `bright`, the same screen without its dim cells.
fn has_draft(rows: &[String], bright: &[String]) -> bool {
    input_row(rows)
        .and_then(|at| bright.get(at))
        .is_some_and(|line| !input_text(line).is_empty())
}

/// The model row the `/model` picker highlights while its footer offers `s`; `None` on "More models".
fn highlighted(rows: &[String]) -> Option<String> {
    session_footer(rows)?;
    let picker = model_picker(rows)?;
    picker.labels.get(picker.highlighted?).cloned()
}

/// The prompt's own `❯` and the no-break space after it are not input.
fn input_text(line: &str) -> &str {
    line.trim_start_matches('❯').trim()
}

#[derive(PartialEq)]
enum Shown {
    Input,
    Picker { dialog: bool },
}

/// The input box, a picker or dialog over it, or `None` for a screen mid-repaint.
fn shown(rows: &[String]) -> Option<Shown> {
    if input_line(rows).is_some() {
        Some(Shown::Input)
    } else if showing_picker(rows) {
        Some(Shown::Picker {
            dialog: rows.iter().any(|r| r.contains(SWITCH_DIALOG)),
        })
    } else {
        None
    }
}

/// A picker or dialog replaces the input box, so the same words in the conversation don't count.
fn showing_picker(rows: &[String]) -> bool {
    input_line(rows).is_none()
        && (model_picker(rows).is_some()
            || session_footer(rows).is_some()
            || rfind_wrapped(rows, SWITCH_DIALOG).is_some())
}

/// The newest row a footer offering `s` for this session only starts on.
fn session_footer(rows: &[String]) -> Option<usize> {
    SESSION_HINTS
        .iter()
        .filter_map(|hint| rfind_wrapped(rows, hint))
        .max()
}

/// The newest row `text` starts on; a narrow terminal wraps a footer at words onto up to three rows,
/// indenting the rest.
fn rfind_wrapped(rows: &[String], text: &str) -> Option<usize> {
    (0..rows.len()).rev().find(|&i| {
        let mut joined = rows[i].trim_end().to_string();
        joined.contains(text)
            || rows[i + 1..].iter().take(2).any(|next| {
                joined.push(' ');
                joined.push_str(next.trim());
                joined.contains(text)
            })
    })
}

/// claude's `Set model to <name> for this session only` below the newest `❯ /model`; notes from
/// earlier switches stay on screen above it.
fn switch_noted(rows: &[String], label: &str) -> bool {
    let note = format!("Set model to {label} for this session only");
    rows.iter()
        .rposition(|r| r.starts_with('❯') && input_text(r) == "/model")
        .is_some_and(|at| rows[at + 1..].iter().any(|r| r.contains(&note)))
}

/// Enter is safe only on the dialog's "Yes" for this model, with no picker footer under it.
fn switch_dialog_for(rows: &[String], label: &str) -> bool {
    let yes = format!("❯ 1. Yes, switch to {label}");
    input_line(rows).is_none()
        && rows.iter().any(|r| r.contains(SWITCH_DIALOG))
        && rows
            .iter()
            .any(|r| r.split_whitespace().collect::<Vec<_>>().join(" ") == yes)
        && session_footer(rows).is_none()
}

/// The newest footer among the last four lines.
pub(super) fn footer_mode(rows: &[String]) -> Option<&'static str> {
    rows.iter()
        .rev()
        .filter(|r| !r.trim().is_empty())
        .take(4)
        .find_map(|r| FOOTER_MODE.captures(r))
        .and_then(|caps| {
            FOOTER_MODES
                .iter()
                .find(|(phrase, _)| *phrase == &caps[1])
                .map(|(_, key)| *key)
        })
}

fn on_model(status: &PtyAgentStatus, label: &str) -> bool {
    status.model.as_deref().and_then(display_name).as_deref() == Some(label)
}

/// The label claude's picker shows for a model id, named as the desktop names it: `claude-sonnet-5-5[1m]` → `Sonnet 5.5`.
fn display_name(id: &str) -> Option<String> {
    let bare = id.split('[').next()?;
    let bare = bare.strip_prefix("claude-").unwrap_or(bare);
    let bare = DATE_SUFFIX.replace(bare, "");
    let mut parts = bare.split('-');
    let family = parts.next()?;
    if family.is_empty() || !family.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let version: Vec<&str> = parts.collect();
    if !version
        .iter()
        .all(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let mut name = family[..1].to_ascii_uppercase();
    name.push_str(&family[1..]);
    if !version.is_empty() {
        name.push(' ');
        name.push_str(&version.join("."));
    }
    Some(name)
}

fn failed(detail: impl Into<String>) -> CoreError {
    CoreError::PtyControlFailed {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::screen::spawn_theme;

    /// Logged-out captures from `capture.py`, so their rows are the API-billing layout.
    const CAPTURES: [&str; 4] = [
        "2.1.326-120x40",
        "2.1.326-68x30",
        "2.1.330-120x40",
        "2.1.330-68x30",
    ];

    /// A capture's screen as `Screen::rows` reads it, at the size it was recorded.
    fn screen(name: &str) -> Vec<String> {
        let path = format!(
            "{}/tests/fixtures/claude_tui/{name}.bin",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(&path).expect("read the capture");
        let (cols, rows) = name
            .rsplit_once('-')
            .and_then(|(_, size)| size.split_once('x'))
            .expect("a <cols>x<rows> suffix");
        let (replies, _) = std::sync::mpsc::channel();
        let screen = Screen::new(
            (rows.parse().unwrap(), cols.parse().unwrap()),
            spawn_theme(&[]),
            replies,
        )
        .expect("a screen");
        screen.feed(&bytes);
        screen.rows().expect("the screen reads")
    }

    fn report(failures: Vec<String>) {
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// The input box as a TUI paints it, and the same screen cleared for its next repaint.
    fn input_box() -> Vec<String> {
        let rule = "─".repeat(40);
        vec![rule.clone(), "❯\u{a0}".into(), rule]
    }

    #[tokio::test(start_paused = true)]
    async fn a_repaint_after_the_read_that_passed_doesnt_stop_the_key() {
        let mut reads = vec![input_box(), vec![String::new(); 3]].into_iter();
        let outcome = when_shown(|| Ok(Some(reads.next().unwrap_or_default())), ready).await;
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[test]
    fn a_screen_mid_repaint_reads_as_neither_ready_nor_a_prompt() {
        assert_eq!(painted(&vec![String::new(); 3], ready), None);
        assert_eq!(painted(&input_box(), ready), Some(true));
        let prompt = vec![" Do you want to proceed?".into(), " Esc to cancel".into()];
        assert_eq!(painted(&prompt, ready), Some(false));
    }

    #[tokio::test(start_paused = true)]
    async fn a_screen_that_never_shows_stops_the_key() {
        let outcome = when_shown(|| Ok(Some(vec![String::new(); 3])), ready).await;
        assert!(outcome.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn an_agent_that_asks_stops_the_key() {
        let outcome = when_shown(|| Ok(None), ready).await;
        assert!(outcome.is_err());
    }

    #[test]
    fn each_claudes_model_picker_routes_to_session_only() {
        let mut failures = Vec::new();
        for capture in CAPTURES {
            let rows = screen(&format!("model-{capture}"));
            let mut fail = |what: &str| failures.push(format!("{capture}: {what}"));
            match model_picker(&rows) {
                None => fail("no /model picker read"),
                Some(picker) => {
                    if picker.labels
                        != [
                            "Default (recommended)",
                            "Opus 5.5",
                            "Fable 5.2",
                            "Sonnet 5.5",
                            "Haiku 5.5",
                        ]
                    {
                        fail(&format!("rows read as {:?}", picker.labels));
                    }
                    if picker.more {
                        fail("a More models row read where none shows");
                    }
                }
            }
            if highlighted(&rows).as_deref() != Some("Opus 5.5") {
                fail("`s` would not pick the lit Opus 5.5 row");
            }
            if !showing_picker(&rows) {
                fail("dismiss would leave it open");
            }
            if ready(&rows) || effort_slider(&rows).is_some() {
                fail("read as the input box or the effort slider");
            }
            if switch_dialog_for(&rows, "Opus 5.5") {
                fail("Enter allowed while the picker shows");
            }
        }
        report(failures);
    }

    #[test]
    fn each_claudes_effort_slider_reads_its_level() {
        let mut failures = Vec::new();
        for capture in CAPTURES {
            let rows = screen(&format!("effort-{capture}"));
            let mut fail = |what: &str| failures.push(format!("{capture}: {what}"));
            match effort_slider(&rows) {
                None => fail("no /effort slider read"),
                Some(slider) if slider.selected().as_deref() != Some("medium") => {
                    fail(&format!("selected {:?}, not medium", slider.selected()))
                }
                Some(_) => {}
            }
            if !showing_picker(&rows) {
                fail("dismiss would leave it open");
            }
            if model_picker(&rows).is_some() || ready(&rows) {
                fail("read as the /model picker or the input box");
            }
        }
        report(failures);
    }

    #[test]
    fn claude_2_1_330_notes_a_session_only_switch() {
        let rows = screen("session-2.1.330-68x30");
        assert!(ready(&rows) && !showing_picker(&rows));
        assert!(switch_noted(&rows, "Opus 5.5"));
    }
}
