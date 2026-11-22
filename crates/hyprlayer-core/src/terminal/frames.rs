use std::io::Write as _;

use ghostty_vt::{ActiveScreen, CellColor, CellRead, CellStyle, CellWide, Error, RowAt, Terminal};

const ATTACH_ROWS: u64 = 1000;
/// The desktop's own scrollback: more catch-up rows than it keeps would only cost bytes.
const CATCH_UP_ROWS: u64 = 5000;
/// History text one frame carries; the rest goes in the frames after it. A flood then still
/// crosses a 16 KiB/s link a frame every 3 s or so.
const FRAME_HISTORY_BYTES: usize = 48 * 1024;

/// Modes xterm encodes keys, mouse, focus and paste by, and reverse video. All are off by default.
const MODES: [u16; 12] = [
    1, 5, 9, 1000, 1002, 1003, 1004, 1005, 1006, 1015, 1016, 2004,
];
const KEYPAD: u16 = 66;

pub struct Frame {
    pub bytes: Vec<u8>,
    /// `(cols, rows)` the client takes before it writes `bytes`, when the size changed.
    pub resized: Option<(u16, u16)>,
    /// The history count a reattach resumes from once `bytes` are written.
    pub history: u64,
}

/// One stream's baseline. A rendered frame becomes the baseline only once `commit` says it was sent.
pub struct Frames {
    client: Client,
    next: Option<Client>,
}

impl Frames {
    /// `resume` is a history count a client already holds; `None` starts a fresh client.
    pub fn new(t: &Terminal, resume: Option<u64>) -> (Self, bool) {
        let pushed = t.history_pushed().unwrap_or(0);
        let alt = t.active_screen().ok() == Some(ActiveScreen::Alternate);
        let held = held(t, alt);
        let resumed = resume.is_some_and(|h| h <= pushed && (alt || pushed - h <= held));
        let client = Client {
            fresh: !resumed,
            history: resume.filter(|_| resumed).unwrap_or(pushed),
            cleared: t.history_cleared().unwrap_or(0),
            ..Client::default()
        };
        (Self { client, next: None }, resumed)
    }

    pub fn commit(&mut self) {
        if let Some(next) = self.next.take() {
            self.client = next;
        }
    }

    /// After the latest frame, history is still owed and the main screen is up to take it.
    pub fn catching_up(&self) -> bool {
        let client = self.next.as_ref().unwrap_or(&self.client);
        client.backlog > 0 && client.alt == Some(false)
    }

    pub fn render(&mut self, t: &Terminal) -> Option<Frame> {
        let model = Model::read(t).ok()?;
        let mut out = Out {
            bytes: Vec::new(),
            client: self.client.clone(),
            cols: model.cols,
            height: model.rows,
            changed: false,
        };
        let resized = out.client.size != Some((model.rows, model.cols));
        out.render(t, &model);
        let history = out.client.history.saturating_sub(out.client.backlog);
        self.next = Some(out.client);
        out.changed.then(|| Frame {
            bytes: out.bytes,
            resized: resized.then_some((model.cols, model.rows)),
            history,
        })
    }
}

fn held(t: &Terminal, alt: bool) -> u64 {
    if alt {
        return 0;
    }
    t.scrollback_rows().unwrap_or(0) as u64
}

#[derive(Clone, Default)]
struct Client {
    /// `(rows, cols)`.
    size: Option<(u16, u16)>,
    /// `None` for a row whose content is unknown.
    rows: Vec<Option<Row>>,
    history: u64,
    /// History rows held before `history` that the client still has to be sent.
    backlog: u64,
    cleared: u64,
    alt: Option<bool>,
    modes: Option<u16>,
    keypad: Option<bool>,
    shape: Option<u8>,
    cursor: Option<(u16, u16, bool)>,
    pen: Option<Style>,
    link: Option<String>,
    autowrap: Option<bool>,
    /// Starts with RIS: a client that couldn't resume shows nothing worth keeping.
    fresh: bool,
}

struct Model {
    rows: u16,
    cols: u16,
    screen: Vec<Row>,
    cursor: (u16, u16, bool),
    shape: u8,
    modes: u16,
    keypad: bool,
    alt: bool,
    pushed: u64,
    cleared: u64,
    held: u64,
}

impl Model {
    fn read(t: &Terminal) -> Result<Self, Error> {
        let (rows, cols) = (t.rows()?, t.cols()?);
        let alt = t.active_screen()? == ActiveScreen::Alternate;
        let mut modes = 0;
        for (bit, &mode) in MODES.iter().enumerate() {
            if t.mode_get(mode)? {
                modes |= 1 << bit;
            }
        }
        Ok(Self {
            rows,
            cols,
            screen: (0..rows)
                .map(|y| read_row(t, RowAt::Viewport(y.into())))
                .collect::<Result<_, _>>()?,
            cursor: (t.cursor_y()?, t.cursor_x()?, t.cursor_visible()?),
            shape: t.cursor_shape()?,
            modes,
            keypad: t.mode_get(KEYPAD)?,
            alt,
            pushed: t.history_pushed()?,
            cleared: t.history_cleared()?,
            held: held(t, alt),
        })
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Row {
    text: String,
    cells: Vec<Cell>,
    links: Vec<String>,
    wraps: bool,
    continues: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    /// Where this cell's text ends in the row's string; it starts where the previous one ends.
    end: u32,
    /// 0 for the second half of a wide cell.
    width: u8,
    /// 1-based into the row's links; 0 for none.
    link: u8,
    style: Style,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    fg: Color,
    bg: Color,
    underline_color: Color,
    flags: u8,
    underline: u8,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb(u8, u8, u8),
}

const BOLD: u8 = 1;
const FAINT: u8 = 2;
const ITALIC: u8 = 4;
const BLINK: u8 = 8;
const INVERSE: u8 = 16;
const INVISIBLE: u8 = 32;
const STRIKE: u8 = 64;
const OVERLINE: u8 = 128;

impl Style {
    fn bg_only(bg: Color) -> Self {
        Self {
            bg,
            ..Self::default()
        }
    }
}

impl From<&CellStyle> for Style {
    fn from(s: &CellStyle) -> Self {
        let color = |c: Option<CellColor>| match c {
            None => Color::Default,
            Some(CellColor::Palette(i)) => Color::Palette(i),
            Some(CellColor::Rgb(c)) => Color::Rgb(c.r, c.g, c.b),
        };
        let flags = [
            (s.bold, BOLD),
            (s.faint, FAINT),
            (s.italic, ITALIC),
            (s.blink, BLINK),
            (s.inverse, INVERSE),
            (s.invisible, INVISIBLE),
            (s.strikethrough, STRIKE),
            (s.overline, OVERLINE),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |flags, (_, bit)| flags | bit);
        Self {
            fg: color(s.fg_color),
            bg: color(s.bg_color),
            underline_color: color(s.underline_color),
            flags,
            underline: if s.underlined { s.underline.max(1) } else { 0 },
        }
    }
}

fn read_row(t: &Terminal, at: RowAt) -> Result<Row, Error> {
    let mut row = Row::default();
    let (wraps, continues) = t.for_each_row_cell(at, |cell: CellRead<'_>| {
        row.text
            .extend(cell.graphemes.iter().filter_map(|&c| char::from_u32(c)));
        let link = match cell.hyperlink {
            Some(uri) => match row.links.iter().position(|l| l == uri) {
                Some(i) => i + 1,
                None if row.links.len() < usize::from(u8::MAX) => {
                    row.links.push(uri.to_string());
                    row.links.len()
                }
                None => 0,
            },
            None => 0,
        };
        row.cells.push(Cell {
            end: row.text.len() as u32,
            width: match cell.wide {
                CellWide::SpacerTail => 0,
                CellWide::Wide => 2,
                CellWide::Narrow | CellWide::SpacerHead => 1,
            },
            link: link as u8,
            style: Style::from(&cell.style),
        });
    })?;
    row.wraps = wraps;
    row.continues = continues;
    Ok(row)
}

impl Row {
    fn text_of(&self, x: usize) -> &str {
        let start = x.checked_sub(1).map_or(0, |prev| self.cells[prev].end);
        &self.text[start as usize..self.cells[x].end as usize]
    }

    fn link_of(&self, cell: &Cell) -> Option<&str> {
        let i = usize::from(cell.link).checked_sub(1)?;
        self.links.get(i).map(String::as_str)
    }

    /// Where the trailing cells an erase can draw begin, and the background it erases with.
    fn trailing(&self) -> (usize, Color) {
        let bg = self.cells.last().map_or(Color::Default, |c| c.style.bg);
        let erasable = |x: usize| {
            let cell = &self.cells[x];
            cell.width == 1
                && cell.link == 0
                && cell.style == Style::bg_only(bg)
                && self.text_of(x).is_empty()
        };
        let mut end = self.cells.len();
        while end > 0 && erasable(end - 1) {
            end -= 1;
        }
        (end, bg)
    }

    fn blank(cols: u16) -> Self {
        Self::marker_or_blank(None, cols)
    }

    fn marker(skipped: u64, cols: u16) -> Self {
        Self::marker_or_blank(Some(skipped), cols)
    }

    fn marker_or_blank(skipped: Option<u64>, cols: u16) -> Self {
        let mut row = Row::default();
        let faint = Style {
            flags: FAINT,
            ..Style::default()
        };
        let text = skipped.map(|n| format!("[… {n} lines skipped]"));
        for ch in text.iter().flat_map(|text| text.chars()) {
            if row.cells.len() == usize::from(cols) {
                break;
            }
            row.text.push(ch);
            row.cells.push(Cell {
                end: row.text.len() as u32,
                width: 1,
                link: 0,
                style: faint,
            });
        }
        while row.cells.len() < usize::from(cols) {
            row.cells.push(Cell {
                end: row.text.len() as u32,
                width: 1,
                link: 0,
                style: Style::default(),
            });
        }
        row
    }
}

/// Whether xterm 5.5, which measures with Unicode 6 and joins no clusters, draws `text` `width` wide.
fn xterm_agrees(text: &str, width: u8) -> bool {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return text.is_empty() && width == 1;
    };
    let c = u32::from(c);
    if c < 0x7f {
        return c >= 0x20 && width == 1;
    }
    let combining =
        matches!(c, 0x0300..=0x036f | 0x200b..=0x200f | 0x20d0..=0x20ff | 0xfe00..=0xfe0f);
    let wide = matches!(c,
        0x1100..=0x115f | 0x2329 | 0x232a | 0x2e80..=0x303e | 0x3040..=0xa4cf | 0xac00..=0xd7a3
        | 0xf900..=0xfaff | 0xfe10..=0xfe19 | 0xfe30..=0xfe6f | 0xff00..=0xff60 | 0xffe0..=0xffe6
        | 0x20000..=0x2fffd | 0x30000..=0x3fffd);
    !combining && wide == (width == 2)
}

struct Out {
    bytes: Vec<u8>,
    client: Client,
    cols: u16,
    height: u16,
    changed: bool,
}

impl Out {
    fn render(&mut self, t: &Terminal, model: &Model) {
        let size = (model.rows, model.cols);
        self.bytes.extend_from_slice(b"\x1b[?2026h\x1b[?25l");
        if self.client.fresh {
            self.client.fresh = false;
            self.bytes.extend_from_slice(b"\x1bc");
            self.client.alt = Some(false);
            self.client.modes = Some(0);
            self.client.keypad = Some(false);
            self.client.shape = Some(0);
            self.client.pen = Some(Style::default());
            self.client.link = None;
            self.client.autowrap = Some(true);
            self.client.rows = vec![Some(Row::blank(model.cols)); usize::from(model.rows)];
            self.client.size = Some(size);
            // A resize moves rows into history too, uncounted, so a fresh client's backlog is
            // what is held, counted here: a resize since the attach has reflowed it.
            self.client.history = model.pushed;
            self.client.backlog = model.held.min(ATTACH_ROWS);
            self.changed = true;
        }
        // A reflow renumbers the model's history, so history still owed can't be found by count.
        let rewind = self.client.backlog > 0 && self.client.size != Some(size);
        if self.client.size != Some(size) {
            self.client.size = Some(size);
            self.client.rows = vec![None; usize::from(model.rows)];
            self.client.cursor = None;
            self.changed = true;
        }
        if self.client.autowrap != Some(false) {
            self.bytes.extend_from_slice(b"\x1b[?7l");
            self.client.autowrap = Some(false);
        }
        if self.client.pen.is_none() || self.client.link.is_some() {
            self.link(None);
            self.pen(Style::default());
        }

        // History is the primary screen's: it goes into the client's normal buffer, and
        // waits while the model shows the alternate screen.
        if !model.alt {
            self.alt(false);
            if rewind {
                self.pen(Style::default());
                self.bytes.extend_from_slice(b"\x1b[3J");
                self.client.history = model.pushed;
                self.client.backlog = model.held.min(ATTACH_ROWS);
            }
            let mut pending =
                model.pushed.saturating_sub(self.client.history) + self.client.backlog;
            self.client.backlog = 0;
            let mut skipped = 0;
            if model.cleared != self.client.cleared {
                self.client.cleared = model.cleared;
                self.pen(Style::default());
                self.bytes.extend_from_slice(b"\x1b[3J");
                pending = pending.min(model.held);
                self.changed = true;
            } else if pending > model.held.min(CATCH_UP_ROWS) {
                let sent = model.held.min(CATCH_UP_ROWS);
                skipped = pending - sent;
                pending = sent;
            }
            let mut list = Vec::new();
            if skipped > 0 {
                list.push(Row::marker(skipped, model.cols));
            }
            let (mut sent, mut text) = (0, 0);
            while sent < pending && text < FRAME_HISTORY_BYTES {
                let at = model.held - pending + sent;
                let Ok(row) = read_row(t, RowAt::History(at as u32)) else {
                    sent = pending;
                    break;
                };
                text += row.text.len();
                list.push(row);
                sent += 1;
            }
            self.push(&list);
            self.client.backlog = pending - sent;
            self.client.history = model.pushed;
        }
        if model.alt {
            self.alt(true);
        }

        self.modes(model);
        self.screen(&model.screen);

        let (y, x, visible) = model.cursor;
        if self.client.shape != Some(model.shape) {
            let _ = write!(self.bytes, "\x1b[{} q", model.shape);
            self.client.shape = Some(model.shape);
            self.changed = true;
        }
        if self.client.cursor != Some(model.cursor) {
            self.changed = true;
        }
        self.cup(y, x);
        if visible {
            self.bytes.extend_from_slice(b"\x1b[?25h");
        }
        self.client.cursor = Some(model.cursor);
        self.bytes.extend_from_slice(b"\x1b[?2026l");
    }

    fn alt(&mut self, on: bool) {
        if self.client.alt == Some(on) {
            return;
        }
        self.link(None);
        self.pen(Style::default());
        let _ = write!(self.bytes, "\x1b[?1049{}", if on { 'h' } else { 'l' });
        self.client.alt = Some(on);
        let blank = on.then(|| Row::blank(self.cols));
        self.client.rows = vec![blank; usize::from(self.height)];
        self.client.cursor = None;
        self.changed = true;
    }

    fn modes(&mut self, model: &Model) {
        let have = self.client.modes;
        // Resets first: xterm turns off whichever mouse tracking mode is on at any reset of one.
        for set in [false, true] {
            for (bit, mode) in MODES.iter().enumerate() {
                let want = model.modes & (1 << bit) != 0;
                if want == set && have.is_none_or(|have| (have & (1 << bit) != 0) != want) {
                    let _ = write!(self.bytes, "\x1b[?{mode}{}", if want { 'h' } else { 'l' });
                    self.changed = true;
                }
            }
        }
        self.client.modes = Some(model.modes);
        if self.client.keypad != Some(model.keypad) {
            self.bytes
                .extend_from_slice(if model.keypad { b"\x1b=" } else { b"\x1b>" });
            self.client.keypad = Some(model.keypad);
            self.changed = true;
        }
    }

    /// Rows into the client's history, oldest first: each takes the top row, which is
    /// repainted first when it isn't that row already, and rows past the screen's height
    /// are written at the bottom as they scroll, so a soft-wrapped row stays joined.
    fn push(&mut self, list: &[Row]) {
        if list.is_empty() {
            return;
        }
        self.changed = true;
        let height = usize::from(self.height);
        let bottom = self.height - 1;
        for (y, row) in list.iter().take(height).enumerate() {
            if self.client.rows[y].as_ref() != Some(row) {
                self.paint(y as u16, row);
            }
        }
        self.link(None);
        self.pen(Style::default());
        self.cup(bottom, 0);
        for k in 0..list.len() {
            let above = &list[(k + height - 1).min(list.len() - 1)];
            match list.get(k + height) {
                Some(row) if row.continues && above.wraps => {
                    self.pending_wrap(bottom, above);
                    self.pen(Style::default());
                    self.bytes.extend_from_slice(b"\x1b[?7h \x1b[?7l");
                    self.shift();
                    self.paint_cells(bottom, row, false, false);
                    self.client.rows[height - 1] = Some(row.clone());
                }
                next => {
                    self.pen(Style::default());
                    self.bytes.push(b'\n');
                    self.shift();
                    if let Some(row) = next {
                        self.bytes.push(b'\r');
                        self.paint_cells(bottom, row, true, false);
                        self.client.rows[height - 1] = Some(row.clone());
                    }
                }
            }
        }
    }

    fn shift(&mut self) {
        self.client.rows.remove(0);
        self.client.rows.push(Some(Row::blank(self.cols)));
        self.client.cursor = None;
    }

    /// The rows that changed, or a cleared screen and every row when that is shorter.
    fn screen(&mut self, rows: &[Row]) {
        let stale = rows
            .iter()
            .zip(&self.client.rows)
            .filter(|(row, have)| have.as_ref() != Some(*row))
            .count();
        if stale == 0 {
            return;
        }
        self.changed = true;
        let unknown = self.client.rows.iter().any(Option::is_none);
        let mut out = self.fork();
        if !unknown {
            out.paint_rows(rows);
        }
        if unknown || stale * 2 > rows.len() {
            let mut full = self.fork();
            full.link(None);
            full.pen(Style::default());
            full.bytes.extend_from_slice(b"\x1b[H\x1b[2J");
            full.client.rows = vec![Some(Row::blank(self.cols)); rows.len()];
            full.paint_rows(rows);
            if unknown || full.bytes.len() < out.bytes.len() {
                out = full;
            }
        }
        self.bytes.extend_from_slice(&out.bytes);
        self.client = out.client;
    }

    fn fork(&self) -> Out {
        Out {
            bytes: Vec::new(),
            client: self.client.clone(),
            cols: self.cols,
            height: self.height,
            changed: true,
        }
    }

    fn paint_rows(&mut self, rows: &[Row]) {
        for (y, row) in rows.iter().enumerate() {
            if self.client.rows[y].as_ref() != Some(row) {
                self.paint(y as u16, row);
            }
        }
    }

    /// Row `y` as the model has it. A continued row is reached by wrapping from the row
    /// above, so xterm marks it wrapped and joins the two for links, selection and reflow.
    fn paint(&mut self, y: u16, row: &Row) {
        let above = (y > 0)
            .then(|| self.client.rows[usize::from(y) - 1].clone())
            .flatten();
        match above.filter(|above| row.continues && above.wraps) {
            Some(above) => {
                self.pending_wrap(y - 1, &above);
                self.pen(Style::default());
                self.bytes.extend_from_slice(b"\x1b[?7h \x1b[?7l");
                self.paint_cells(y, row, false, true);
            }
            None if row.continues => {
                self.cup(y, 0);
                self.paint_cells(y, row, true, true);
            }
            None => {
                self.link(None);
                self.pen(Style::default());
                self.cup(y, 0);
                self.bytes.extend_from_slice(b"\x1b[2K");
                self.paint_cells(y, row, true, false);
            }
        }
        self.client.rows[usize::from(y)] = Some(row.clone());
    }

    /// Prints the last cell of row `y` again, leaving xterm's cursor waiting to wrap.
    fn pending_wrap(&mut self, y: u16, row: &Row) {
        let last = row.cells.len() - 1;
        let (x, cell) = match row.cells[last] {
            tail if tail.width == 0 && last > 0 => (last - 1, row.cells[last - 1]),
            cell => (last, cell),
        };
        let text = row.text_of(x);
        self.link(row.link_of(&cell));
        self.pen(cell.style);
        if xterm_agrees(text, cell.width) && !text.is_empty() {
            self.cup(y, x as u16);
            self.bytes.extend_from_slice(text.as_bytes());
        } else {
            self.cup(y, self.cols - 1);
            self.bytes.push(b' ');
        }
        self.link(None);
    }

    /// An erase from the first column would drop xterm's wrap mark, so an empty continued row
    /// erases from where its wrap left the cursor.
    fn paint_cells(&mut self, y: u16, row: &Row, at_start: bool, mut stale: bool) {
        let (end, bg) = row.trailing();
        if end > 0 && !at_start {
            self.cup(y, 0);
        }
        for x in 0..end {
            let cell = row.cells[x];
            if cell.width == 0 {
                continue;
            }
            self.link(row.link_of(&cell));
            self.pen(cell.style);
            let text = row.text_of(x);
            if text.is_empty() {
                self.bytes.push(b' ');
            } else if xterm_agrees(text, cell.width) {
                self.bytes.extend_from_slice(text.as_bytes());
            } else {
                // Its cells painted first, then the cluster, then the next cell placed: xterm's own width can't shift the row.
                // What it draws past its cells is painted over by the cells after it, or erased.
                stale = true;
                for _ in 0..cell.width {
                    self.bytes.push(b' ');
                }
                let _ = write!(self.bytes, "\x1b[{}G", x + 1);
                self.bytes.extend_from_slice(text.as_bytes());
                let _ = write!(self.bytes, "\x1b[{}G", x + usize::from(cell.width) + 1);
            }
        }
        self.link(None);
        if end < row.cells.len() && (stale || bg != Color::Default) {
            self.pen(Style::bg_only(bg));
            self.bytes.extend_from_slice(b"\x1b[K");
        }
    }

    fn cup(&mut self, y: u16, x: u16) {
        match x {
            0 => write!(self.bytes, "\x1b[{}H", y + 1),
            _ => write!(self.bytes, "\x1b[{};{}H", y + 1, x + 1),
        }
        .expect("writes to a Vec");
    }

    fn link(&mut self, uri: Option<&str>) {
        if self.client.link.as_deref() == uri {
            return;
        }
        let _ = write!(self.bytes, "\x1b]8;;{}\x07", uri.unwrap_or(""));
        self.client.link = uri.map(str::to_string);
    }

    fn pen(&mut self, style: Style) {
        if self.client.pen == Some(style) {
            return;
        }
        self.bytes.extend_from_slice(b"\x1b[0");
        for (bit, code) in [
            (BOLD, "1"),
            (FAINT, "2"),
            (ITALIC, "3"),
            (BLINK, "5"),
            (INVERSE, "7"),
            (INVISIBLE, "8"),
            (STRIKE, "9"),
            (OVERLINE, "53"),
        ] {
            if style.flags & bit != 0 {
                let _ = write!(self.bytes, ";{code}");
            }
        }
        match style.underline {
            0 => {}
            1 => self.bytes.extend_from_slice(b";4"),
            n => {
                let _ = write!(self.bytes, ";4:{n}");
            }
        }
        self.color(style.fg, 30, 90, 38);
        self.color(style.bg, 40, 100, 48);
        match style.underline_color {
            Color::Default => {}
            Color::Palette(i) => {
                let _ = write!(self.bytes, ";58;5;{i}");
            }
            Color::Rgb(r, g, b) => {
                let _ = write!(self.bytes, ";58;2;{r};{g};{b}");
            }
        }
        self.bytes.push(b'm');
        self.client.pen = Some(style);
    }

    /// Palette colours stay indices, so each desktop paints them with its own theme.
    fn color(&mut self, color: Color, base: u8, bright: u8, extended: u8) {
        let _ = match color {
            Color::Default => Ok(()),
            Color::Palette(i @ 0..=7) => write!(self.bytes, ";{}", base + i),
            Color::Palette(i @ 8..=15) => write!(self.bytes, ";{}", bright + i - 8),
            Color::Palette(i) => write!(self.bytes, ";{extended};5;{i}"),
            Color::Rgb(r, g, b) => write!(self.bytes, ";{extended};2;{r};{g};{b}"),
        };
    }
}
