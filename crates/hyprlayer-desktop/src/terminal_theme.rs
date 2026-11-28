//! The colours the server needs before a PTY paints: `xtermThemeFromCss` over the v4 tokens.

use gpui::Hsla;
use hyprlayer_models::{Rgb, TerminalTheme};
use hyprlayer_ui::theme::{Mode, Tokens};

fn hex(hex: u32) -> Rgb {
    Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn token(c: Hsla) -> Rgb {
    let c = c.to_rgb();
    let byte = |v: f32| (v * 255.0).round() as u8;
    Rgb(byte(c.r), byte(c.g), byte(c.b))
}

pub fn for_mode(mode: Mode) -> TerminalTheme {
    let t = Tokens::for_mode(mode);
    // The base CSS aliases magenta to accent, which both v4 modes override.
    let magenta = match mode {
        Mode::Dark => 0xd97757,
        Mode::Light => 0xc25e3d,
    };
    TerminalTheme {
        foreground: token(t.ink),
        background: token(t.surface),
        cursor: token(t.accent),
        ansi: [
            hex(0x2e3436),
            token(t.bad),
            token(t.ok),
            token(t.warn),
            token(t.brand),
            hex(magenta),
            token(t.brand_soft),
            token(t.ink),
            token(t.ink_dim),
            hex(0xef2929),
            hex(0x8ae234),
            hex(0xfce94f),
            hex(0x729fcf),
            hex(0xad7fa8),
            hex(0x34e2e2),
            hex(0xeeeeec),
        ],
        dark: mode == Mode::Dark,
    }
}
