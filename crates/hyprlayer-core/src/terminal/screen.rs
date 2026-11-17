//! A PTY's screen as a person sees it, for reading a TUI that repaints only the cells that changed.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::PtySession;

pub struct ScreenMirror {
    state: Mutex<Mirror>,
}

struct Mirror {
    parser: vt100::Parser,
    /// Started after the ring dropped output, or the parser panicked: some cells may be stale for good.
    lost: bool,
}

impl ScreenMirror {
    /// Fed on the PTY's own output thread, so it never lags the ring; start it at launch, once per session.
    pub fn follow(session: &PtySession) -> Arc<Self> {
        let (rows, cols) = session.size();
        let mirror = Arc::new(Self {
            state: Mutex::new(Mirror {
                parser: vt100::Parser::new(rows, cols, 0),
                lost: false,
            }),
        });
        let feed = mirror.clone();
        let whole = session.tap_output(move |bytes, size| {
            // A parser panic here would end the output thread every reader of this PTY depends on.
            if catch_unwind(AssertUnwindSafe(|| feed.feed(bytes, size))).is_err() {
                feed.lock().lost = true;
            }
        });
        if !whole {
            mirror.lock().lost = true;
        }
        mirror
    }

    fn feed(&self, bytes: &[u8], (rows, cols): (u16, u16)) {
        let mut state = self.lock();
        if state.parser.screen().size() != (rows, cols) {
            state.parser.screen_mut().set_size(rows, cols);
        }
        state.parser.process(bytes);
    }

    /// Each row's text, top to bottom, trailing blanks trimmed; `None` once the mirror is `lost`.
    pub fn rows(&self) -> Option<Vec<String>> {
        let state = self.lock();
        if state.lost {
            return None;
        }
        let screen = state.parser.screen();
        Some(screen.rows(0, screen.size().1).collect())
    }

    /// `rows`, and the same rows from one read with dim cells blanked: claude's placeholder is dim.
    pub fn rows_and_bright(&self) -> Option<(Vec<String>, Vec<String>)> {
        let state = self.lock();
        if state.lost {
            return None;
        }
        let screen = state.parser.screen();
        let (height, cols) = screen.size();
        let rows = screen.rows(0, cols).collect();
        let bright = (0..height)
            .map(|row| bright_row(screen, row, cols))
            .collect();
        Some((rows, bright))
    }

    fn lock(&self) -> MutexGuard<'_, Mirror> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn bright_row(screen: &vt100::Screen, row: u16, cols: u16) -> String {
    let mut text = String::new();
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        if cell.has_contents() && !cell.dim() {
            text.push_str(cell.contents());
        } else {
            text.push(' ');
        }
    }
    text.truncate(text.trim_end().len());
    text
}
