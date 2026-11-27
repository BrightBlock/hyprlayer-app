//! A grid element that paints a `ghostty-vt` screen: one shaped line per style run, one quad per background run.
use ghostty_vt::{CellColor, CellRead, CellWide, RgbColor, RowAt, Terminal};
use gpui::{
    canvas, div, fill, font, point, px, size, Bounds, Context, Hsla, IntoElement, ParentElement,
    Pixels, Render, Rgba, SharedString, Styled, TextAlign, TextRun, Window,
};

const FONT_SIZE: f32 = 13.0;
const DEFAULT_FG: RgbColor = RgbColor { r: 0xd0, g: 0xd4, b: 0xda };
const DEFAULT_BG: RgbColor = RgbColor { r: 0x10, g: 0x14, b: 0x18 };

#[cfg(target_os = "macos")]
pub const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
pub const MONO: &str = "Consolas";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub const MONO: &str = "DejaVu Sans Mono";

#[derive(Clone)]
struct Run {
    text: String,
    col: usize,
    width: usize,
    fg: RgbColor,
    bg: Option<RgbColor>,
    bold: bool,
}

pub struct TermGrid {
    term: Terminal,
    palette: [RgbColor; 256],
    pub frames: u64,
    pub bytes: usize,
    pub status: String,
}

impl TermGrid {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            term: Terminal::new(cols, rows, 5000).expect("ghostty-vt terminal"),
            palette: ghostty_vt::default_palette(),
            frames: 0,
            bytes: 0,
            status: String::new(),
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.frames += 1;
        self.bytes += bytes.len();
        self.term.write(bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.term.resize(cols, rows, 8, 16);
    }

    pub fn text(&self) -> String {
        let (c, r) = (self.term.cols().unwrap_or(1), self.term.rows().unwrap_or(1));
        self.term.read_text_viewport((0, 0), (c - 1, u32::from(r) - 1), false).unwrap_or_default()
    }

    fn color(&self, c: CellColor) -> RgbColor {
        match c {
            CellColor::Palette(i) => self.palette[i as usize],
            CellColor::Rgb(rgb) => rgb,
        }
    }

    fn snapshot(&self) -> (Vec<Vec<Run>>, Option<(usize, usize)>) {
        let rows = self.term.rows().unwrap_or(0) as u32;
        let mut out = Vec::with_capacity(rows as usize);
        for y in 0..rows {
            let mut runs: Vec<Run> = Vec::new();
            let mut col = 0usize;
            let _ = self.term.for_each_row_cell(RowAt::Viewport(y), |cell: CellRead<'_>| {
                if cell.wide == CellWide::SpacerTail {
                    col += 1;
                    return;
                }
                let s = cell.style;
                let (mut fg, mut bg) = (
                    s.fg_color.map(|c| self.color(c)).unwrap_or(DEFAULT_FG),
                    s.bg_color.map(|c| self.color(c)),
                );
                if s.inverse {
                    let old_bg = bg.unwrap_or(DEFAULT_BG);
                    bg = Some(fg);
                    fg = old_bg;
                }
                let ch: String = if cell.graphemes.is_empty() || s.invisible {
                    " ".into()
                } else {
                    cell.graphemes.iter().filter_map(|&c| char::from_u32(c)).collect()
                };
                let width = if cell.wide == CellWide::Wide { 2 } else { 1 };
                match runs.last_mut() {
                    Some(last) if last.fg == fg && last.bg == bg && last.bold == s.bold => {
                        last.text.push_str(&ch);
                        last.width += width;
                    }
                    _ => runs.push(Run { text: ch, col, width, fg, bg, bold: s.bold }),
                }
                col += width;
            });
            out.push(runs);
        }
        let cursor = match (self.term.cursor_visible(), self.term.cursor_x(), self.term.cursor_y()) {
            (Ok(true), Ok(x), Ok(y)) => Some((x as usize, y as usize)),
            _ => None,
        };
        (out, cursor)
    }
}

fn hsla(c: RgbColor) -> Hsla {
    Rgba { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: 1.0 }.into()
}

impl Render for TermGrid {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (rows, cursor) = self.snapshot();
        div().size_full().bg(hsla(DEFAULT_BG)).child(
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<Pixels>, _, window, cx| {
                    let font_size = px(FONT_SIZE);
                    let base = font(MONO);
                    let font_id = window.text_system().resolve_font(&base);
                    let cell_w = window
                        .text_system()
                        .advance(font_id, font_size, 'm')
                        .map(|s| s.width)
                        .unwrap_or(px(8.0));
                    let line_h = font_size * 1.3;
                    for (y, runs) in rows.iter().enumerate() {
                        let top = bounds.origin.y + line_h * y as f32;
                        for run in runs {
                            let left = bounds.origin.x + cell_w * run.col as f32;
                            if let Some(bg) = run.bg {
                                window.paint_quad(fill(
                                    Bounds::new(point(left, top), size(cell_w * run.width as f32, line_h)),
                                    hsla(bg),
                                ));
                            }
                            let mut f = base.clone();
                            if run.bold {
                                f.weight = gpui::FontWeight::BOLD;
                            }
                            let text: SharedString = run.text.clone().into();
                            let text_run = TextRun {
                                len: text.len(),
                                font: f,
                                color: hsla(run.fg),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(text, font_size, &[text_run], Some(cell_w));
                            let _ = line.paint(point(left, top), line_h, TextAlign::Left, None, window, cx);
                        }
                    }
                    if let Some((x, y)) = cursor {
                        let mut c: Hsla = hsla(DEFAULT_FG);
                        c.a = 0.55;
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.origin.x + cell_w * x as f32, bounds.origin.y + line_h * y as f32),
                                size(cell_w, line_h),
                            ),
                            c,
                        ));
                    }
                },
            )
            .size_full(),
        )
    }
}
