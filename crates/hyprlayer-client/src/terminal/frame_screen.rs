use ghostty_vt::{CellStyle, CellWide, Error, RowAt, Terminal};

/// The server's scrollback cap: the same cap keeps history equal to the server's.
pub const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCell {
    /// Empty for a blank cell.
    pub text: String,
    pub wide: CellWide,
    pub style: CellStyle,
    pub linked: bool,
}

/// A PTY's screen as the server's frames draw it.
pub struct FrameScreen {
    terminal: Terminal,
}

impl FrameScreen {
    pub fn new(cols: u16, rows: u16) -> Result<Self, Error> {
        Ok(Self {
            terminal: Terminal::new(cols.max(1), rows.max(1), SCROLLBACK_BYTES)?,
        })
    }

    /// Applies a `resized` event, which arrives ahead of the frame drawn at the new size.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        self.terminal.resize(cols.max(1), rows.max(1), 0, 0)
    }

    /// Writes a frame's decoded bytes.
    pub fn write(&mut self, frame: &[u8]) {
        self.terminal.write(frame);
        // Nothing reads them: drained so they don't pile up.
        self.terminal.take_clipboard_writes();
        self.terminal.take_pwd_changes();
    }

    pub fn size(&self) -> Result<(u16, u16), Error> {
        Ok((self.terminal.cols()?, self.terminal.rows()?))
    }

    /// The viewport row's cells, and whether it continues the row above.
    pub fn row(&self, y: u16) -> Result<(Vec<ScreenCell>, bool), Error> {
        self.cells(RowAt::Viewport(y.into()))
    }

    pub fn rows(&self) -> Result<Vec<(Vec<ScreenCell>, bool)>, Error> {
        (0..self.terminal.rows()?).map(|y| self.row(y)).collect()
    }

    pub fn history_len(&self) -> Result<usize, Error> {
        self.terminal.scrollback_rows()
    }

    /// Each history row's text, oldest first, spacer cells dropped.
    pub fn history(&self) -> Result<Vec<String>, Error> {
        (0..self.history_len()? as u32)
            .map(|y| {
                let (cells, _) = self.cells(RowAt::History(y))?;
                Ok(cells
                    .iter()
                    .filter(|cell| cell.wide != CellWide::SpacerTail)
                    .map(|cell| cell.text.as_str())
                    .collect())
            })
            .collect()
    }

    fn cells(&self, at: RowAt) -> Result<(Vec<ScreenCell>, bool), Error> {
        let mut cells = Vec::new();
        let (_, continues) = self.terminal.for_each_row_cell(at, |cell| {
            cells.push(ScreenCell {
                text: cell
                    .graphemes
                    .iter()
                    .filter_map(|&c| char::from_u32(c))
                    .collect(),
                wide: cell.wide,
                style: cell.style,
                linked: cell.hyperlink.is_some(),
            });
        })?;
        Ok((cells, continues))
    }
}
