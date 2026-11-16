//! Changes a hooked claude PTY's model, effort and mode from the status bar, session-only, by
//! driving its TUI as a person would (recipe observed on claude 2.1.322). A digit or Enter in a
//! picker, and a typed `/model <x>` or `/effort <x>`, each save a default (the model one on the
//! user's claude account), so none is ever sent. A codex over its own app-server goes through the
//! API instead.

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
use crate::terminal::screen::ScreenMirror;
use crate::terminal::PtySession;

const TYPE_GAP: Duration = Duration::from_millis(20);
const BEFORE_ENTER: Duration = Duration::from_millis(400);
const PICKER_OPEN: Duration = Duration::from_secs(3);
/// A TUI busy running a statusLine reads its input late; the Enter waits for the echo.
const ECHO_WAIT: Duration = Duration::from_secs(2);
const KEY_SETTLE: Duration = Duration::from_millis(400);
const CONFIRM: Duration = Duration::from_secs(5);
const DIALOG_CONFIRM: Duration = Duration::from_secs(3);
const MODE_SETTLE: Duration = Duration::from_secs(1);
const PLAN_CONFIRM: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);
const MAX_PRESSES: usize = 6;

const MODEL_FOOTER: &str = "Enter to set as default · s to use this session only · Esc to cancel";
/// The `/model` picker's footer while "More models" is highlighted.
const MORE_FOOTER: &str = "Enter to expand · Esc to cancel";
const EFFORT_FOOTER: &str =
    "←/→ to adjust · Enter to confirm · s for this session only · Esc to cancel";
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
    Regex::new(r"^\s*(❯\s+)?(\d)\.\s{2}(\S.*?)(\s+✔)?\s{2,}").expect("model row regex")
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
            drive.key_when(
                |rows| highlighted(rows).as_deref() == Some(label.as_str()),
                SESSION_ONLY,
            )?;
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
            drive.key_when(
                |rows| effort_slider(rows).and_then(|s| s.selected()).as_deref() == Some(effort),
                SESSION_ONLY,
            )?;
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
            .settled(footer_mode)
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
        if let Some(now) = drive.screen.rows().as_deref().and_then(footer_mode) {
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
        if !drive.settled(ready).await? {
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
    screen: Arc<ScreenMirror>,
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

    /// `bytes` only while `on` holds for the screen and the agent isn't asking: a prompt can open
    /// between any two keys.
    fn key_when(&self, on: impl Fn(&[String]) -> bool, bytes: &[u8]) -> Result<()> {
        if self.asking() {
            return Err(failed(
                "the agent started asking you something, so the change stopped",
            ));
        }
        if !on(&self.rows()?) {
            return Err(failed(
                "claude's screen changed under the change, so it stopped",
            ));
        }
        self.key(bytes)
    }

    /// `key_when` once `on` holds, within `KEY_SETTLE`: a TUI repainting between keys reads blank for a moment.
    async fn key_soon(&self, on: impl Fn(&[String]) -> bool, bytes: &[u8]) -> Result<()> {
        let deadline = Instant::now() + KEY_SETTLE;
        while !self.asking() && !on(&self.rows()?) && Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
        }
        self.key_when(on, bytes)
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

    /// `read` once two polls agree: claude repaints a row over more than one write.
    async fn settled<T: PartialEq>(&self, read: impl Fn(&[String]) -> T) -> Result<T> {
        let deadline = Instant::now() + KEY_SETTLE;
        let mut last = read(&self.rows()?);
        loop {
            tokio::time::sleep(POLL).await;
            let now = read(&self.rows()?);
            if now == last || Instant::now() >= deadline {
                return Ok(now);
            }
            last = now;
        }
    }

    /// One key while `on` holds, then what `read` settles on, or the old value after `within`.
    async fn press<T: PartialEq>(
        &self,
        key: &[u8],
        within: Duration,
        on: impl Fn(&[String]) -> bool,
        read: impl Fn(&[String]) -> T,
    ) -> Result<T> {
        let before = self.settled(&read).await?;
        self.key_when(on, key)?;
        let deadline = Instant::now() + within;
        loop {
            tokio::time::sleep(POLL).await;
            if read(&self.rows()?) != before {
                return self.settled(&read).await;
            }
            if Instant::now() >= deadline {
                return Ok(before);
            }
        }
    }

    /// Types a bare slash command onto the empty input line and submits it only once the line
    /// reads exactly that.
    async fn command(&self, command: &str) -> Result<()> {
        for ch in command.chars() {
            self.key_soon(ready, ch.to_string().as_bytes()).await?;
            tokio::time::sleep(TYPE_GAP).await;
        }
        tokio::time::sleep(BEFORE_ENTER).await;
        let shows =
            |rows: &[String]| ready(rows) && input_line(rows).map(input_text) == Some(command);
        let typed = self.until(ECHO_WAIT, shows).await? && self.settled(shows).await?;
        if typed {
            return self.key_when(shows, ENTER);
        }
        // An Enter on anything else could submit a draft or pick a row. The line started empty, so
        // once it shows exactly the command, that many backspaces remove only what was typed.
        let ours = self.until(ECHO_WAIT, shows).await?;
        if ours {
            for _ in command.chars() {
                self.key_soon(ready, BACKSPACE).await?;
            }
        }
        Err(failed(if ours {
            format!("the agent's input didn't take {command}, so nothing was sent")
        } else {
            format!("the agent's input didn't take {command}, so nothing was sent; it may still hold part of it")
        }))
    }

    /// Arrows wrap through every row and "More models", one press per re-read. The route never
    /// lands on "More models": Enter there would expand it, and claude's footer changes under it.
    async fn highlight(&self, label: &str) -> Result<()> {
        for pressed in 0..=MAX_PRESSES {
            let picker = self
                .settled(model_picker)
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
                KEY_SETTLE,
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
                self.key_when(|rows| switch_dialog_for(rows, label), ENTER)?;
                answered = true;
                deadline = deadline.max(Instant::now() + DIALOG_CONFIRM);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// ←/→ stop at the ends, so counting from the left end lands without reading every step.
    async fn slide_to(&self, effort: &str) -> Result<()> {
        let slider = self
            .settled(effort_slider)
            .await?
            .ok_or_else(|| failed("claude's /effort picker closed"))?;
        let target = slider
            .levels
            .iter()
            .position(|(level, _)| level == effort)
            .ok_or_else(|| failed(format!("this model has no {effort} effort")))?;
        let selected = |rows: &[String]| effort_slider(rows).and_then(|s| s.selected());
        let on = |rows: &[String]| effort_slider(rows).is_some();
        for _ in 1..slider.levels.len() {
            self.press(LEFT, KEY_SETTLE, on, selected).await?;
        }
        for _ in 0..target {
            self.press(RIGHT, KEY_SETTLE, on, selected).await?;
        }
        match self.settled(selected).await? {
            Some(level) if level == effort => Ok(()),
            _ => Err(failed(format!(
                "claude's /effort slider didn't reach {effort}"
            ))),
        }
    }

    /// shift+tab one press at a time; back at the start means the ring doesn't hold `want`.
    async fn cycle_to(&self, start: &str, want: &str) -> Result<bool> {
        for _ in 0..MAX_PRESSES {
            let on = |rows: &[String]| ready(rows) && footer_mode(rows).is_some();
            match self.press(SHIFT_TAB, MODE_SETTLE, on, footer_mode).await? {
                Some(now) if now == want => return Ok(true),
                Some(now) if now != start => {}
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// One Esc per picker or dialog seen on screen, the next only once that one has gone: an Esc
    /// anywhere else interrupts the turn. A dialog's Esc returns to the picker, so two at most.
    async fn dismiss(&self) {
        let dialog = |rows: &[String]| rows.iter().any(|r| r.contains(SWITCH_DIALOG));
        for _ in 0..2 {
            let Some(was_dialog) = self
                .screen
                .rows()
                .filter(|rows| showing_picker(rows))
                .map(|rows| dialog(&rows))
            else {
                return;
            };
            if self.key(ESC).is_err() {
                return;
            }
            let gone = self
                .until(DIALOG_CONFIRM, |rows| {
                    !showing_picker(rows) || dialog(rows) != was_dialog
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
    let footer = rows
        .iter()
        .rposition(|r| r.contains(MODEL_FOOTER) || r.contains(MORE_FOOTER))?;
    let title = rows[..footer]
        .iter()
        .rposition(|r| r.trim() == "Select model");
    // "Enter to expand" isn't the picker's alone; its title or a lit "More models" row makes it this picker.
    let more_lit = || {
        rows[..footer]
            .iter()
            .any(|r| r.trim_start().starts_with('❯') && r.contains("More models"))
    };
    if title.is_none() && !rows[footer].contains(MODEL_FOOTER) && !more_lit() {
        return None;
    }
    let top = title.map_or(0, |i| i + 1);
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
            picker.labels.push(caps[3].to_string());
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

#[derive(PartialEq)]
struct EffortSlider {
    /// Each level with its column on the labels row.
    levels: Vec<(String, usize)>,
    /// The `▲` column on the row above.
    marker: Option<usize>,
}

impl EffortSlider {
    fn selected(&self) -> Option<String> {
        let marker = self.marker?;
        self.levels
            .iter()
            .min_by_key(|(level, col)| (col + level.len() / 2).abs_diff(marker))
            .map(|(level, _)| level.clone())
    }
}

fn effort_slider(rows: &[String]) -> Option<EffortSlider> {
    if input_line(rows).is_some() {
        return None;
    }
    let footer = rows.iter().rposition(|r| r.contains(EFFORT_FOOTER))?;
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
pub(super) fn paste_refusal(screen: &ScreenMirror) -> Option<&'static str> {
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

fn covered(rows: &[String]) -> bool {
    rows.iter()
        .any(|r| r.contains("Enter to confirm") || r.contains("Esc to cancel"))
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

/// Text on the input line in `bright`, the same screen without its dim cells.
fn has_draft(rows: &[String], bright: &[String]) -> bool {
    input_row(rows)
        .and_then(|at| bright.get(at))
        .is_some_and(|line| !input_text(line).is_empty())
}

/// The model row the `/model` picker highlights; `None` on "More models".
fn highlighted(rows: &[String]) -> Option<String> {
    let picker = model_picker(rows)?;
    picker.labels.get(picker.highlighted?).cloned()
}

/// The prompt's own `❯` and the no-break space after it are not input.
fn input_text(line: &str) -> &str {
    line.trim_start_matches('❯').trim()
}

/// A picker or dialog replaces the input box, so the same words in the conversation don't count.
fn showing_picker(rows: &[String]) -> bool {
    input_line(rows).is_none()
        && (model_picker(rows).is_some()
            || rows.iter().any(|r| {
                r.contains(MODEL_FOOTER) || r.contains(EFFORT_FOOTER) || r.contains(SWITCH_DIALOG)
            }))
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
        && !rows.iter().any(|r| r.contains("Enter to set as default"))
}

/// The newest footer among the last four lines.
fn footer_mode(rows: &[String]) -> Option<&'static str> {
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
