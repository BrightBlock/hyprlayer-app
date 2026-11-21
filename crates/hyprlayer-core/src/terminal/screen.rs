//! A PTY's screen as a person sees it, for reading a TUI that repaints only the cells that changed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ghostty_vt::{CellWide, ColorScheme, RgbColor, Terminal, TerminalCompressionResult};
use hyprlayer_models::{Rgb, TerminalTheme};

use super::Size;

const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;

pub struct Screen {
    state: Mutex<State>,
}

struct State {
    terminal: Terminal,
    /// Read by the reply callback.
    answers: Arc<AtomicBool>,
    replies: Sender<Vec<u8>>,
    theme: TerminalTheme,
    /// The compression activity when idle compression last finished.
    compressed: Option<u64>,
}

impl Screen {
    pub(crate) fn new(
        (rows, cols): Size,
        theme: TerminalTheme,
        replies: Sender<Vec<u8>>,
    ) -> Result<Self, ghostty_vt::Error> {
        let mut terminal = Terminal::new(cols.max(1), rows.max(1), SCROLLBACK_BYTES)?;
        let answers = Arc::new(AtomicBool::new(true));
        let (open, sink) = (answers.clone(), replies.clone());
        terminal.set_write_pty_callback(move |bytes| {
            if open.load(Ordering::Relaxed) {
                let _ = sink.send(bytes.to_vec());
            }
        })?;
        let mut state = State {
            terminal,
            answers,
            replies,
            theme,
            compressed: None,
        };
        state.apply(theme);
        Ok(Self {
            state: Mutex::new(state),
        })
    }

    pub(crate) fn feed(&self, bytes: &[u8]) {
        let mut state = self.lock();
        state.terminal.write(bytes);
        // Taken so they don't pile up: nothing reads them yet.
        state.terminal.take_pwd_changes();
        state.terminal.take_clipboard_writes();
    }

    pub(crate) fn resize(&self, (rows, cols): Size) {
        if let Err(e) = self.lock().terminal.resize(cols.max(1), rows.max(1), 0, 0) {
            tracing::debug!("screen resize to {cols}x{rows} failed: {e}");
        }
    }

    /// A theme of `None` keeps the current one.
    pub(crate) fn follow(&self, answers: bool, owner: Option<TerminalTheme>) {
        let mut state = self.lock();
        state.answers.store(answers, Ordering::Relaxed);
        let Some(theme) = owner.filter(|theme| *theme != state.theme) else {
            return;
        };
        let flipped = theme.dark != state.theme.dark;
        state.apply(theme);
        let reports = state
            .terminal
            .mode_get(ghostty_vt::MODE_COLOR_SCHEME_REPORT)
            .unwrap_or(false);
        if flipped && answers && reports {
            let scheme = if theme.dark { 1 } else { 2 };
            let _ = state
                .replies
                .send(format!("\x1b[?997;{scheme}n").into_bytes());
        }
    }

    pub(crate) fn compress(&self) {
        loop {
            let mut state = self.lock();
            let activity = state.terminal.compression_activity().ok();
            if activity.is_none() || state.compressed == activity {
                return;
            }
            if !matches!(
                state.terminal.compress_incremental(),
                Ok(TerminalCompressionResult::Pending)
            ) {
                state.compressed = state.terminal.compression_activity().ok();
                return;
            }
        }
    }

    /// Each row's text, top to bottom, trailing blanks trimmed; `None` when it can't be read.
    pub fn rows(&self) -> Option<Vec<String>> {
        self.read().map(|(rows, _)| rows)
    }

    /// `rows`, and the same rows from one read with dim cells blanked: claude's placeholder is dim.
    pub fn rows_and_bright(&self) -> Option<(Vec<String>, Vec<String>)> {
        self.read()
    }

    fn read(&self) -> Option<(Vec<String>, Vec<String>)> {
        let state = self.lock();
        let height = state.terminal.rows().ok()?;
        let mut rows = Vec::with_capacity(height.into());
        let mut bright = Vec::with_capacity(height.into());
        for y in 0..height {
            let (mut text, mut lit) = (String::new(), String::new());
            state
                .terminal
                .for_each_viewport_row_cell(y.into(), |wide, graphemes, faint| {
                    if wide == CellWide::SpacerTail {
                        return;
                    }
                    let cell = graphemes.iter().filter_map(|&c| char::from_u32(c));
                    if graphemes.is_empty() {
                        text.push(' ');
                    }
                    text.extend(cell.clone());
                    if graphemes.is_empty() || faint {
                        lit.push(' ');
                    } else {
                        lit.extend(cell);
                    }
                })
                .ok()?;
            text.truncate(text.trim_end().len());
            lit.truncate(lit.trim_end().len());
            rows.push(text);
            bright.push(lit);
        }
        Some((rows, bright))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    fn apply(&mut self, theme: TerminalTheme) {
        let rgb = |Rgb(r, g, b): Rgb| RgbColor { r, g, b };
        let mut palette = ghostty_vt::default_palette();
        for (slot, color) in palette.iter_mut().zip(theme.ansi) {
            *slot = rgb(color);
        }
        let applied = self
            .terminal
            .set_default_colors(
                rgb(theme.foreground),
                rgb(theme.background),
                rgb(theme.cursor),
            )
            .and_then(|()| self.terminal.set_default_palette(&palette));
        if let Err(e) = applied {
            tracing::debug!("screen theme not applied: {e}");
        }
        self.terminal.set_color_scheme(Some(if theme.dark {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        }));
        self.theme = theme;
    }
}

pub(crate) fn spawn_theme(env: &[(String, String)]) -> TerminalTheme {
    let light = env
        .iter()
        .rev()
        .find(|(key, _)| key == "COLORFGBG")
        .and_then(|(_, value)| value.rsplit(';').next()?.parse::<u8>().ok())
        .is_some_and(|background| matches!(background, 7 | 15));
    let palette = ghostty_vt::default_palette();
    let ansi = std::array::from_fn(|i| Rgb(palette[i].r, palette[i].g, palette[i].b));
    // The desktop's own fallbacks (`terminalTheme.ts`, `index.css`).
    let (foreground, background) = if light {
        (Rgb(0x11, 0x11, 0x11), Rgb(0xf7, 0xf7, 0xf7))
    } else {
        (Rgb(0xe8, 0xe6, 0xe2), Rgb(0x0c, 0x0c, 0x0c))
    };
    TerminalTheme {
        foreground,
        background,
        cursor: Rgb(0xd9, 0x77, 0x57),
        ansi,
        dark: !light,
    }
}
