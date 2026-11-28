//! The v4 palettes from `src/index.css` (`html.dark`, `html.light`) as semantic tokens.
//! Light values the CSS derives with `color-mix()` are resolved here (ADR-0006's v4 table).

use gpui::{rgb, App, Global, Hsla, WindowAppearance};
use gpui_component::{Theme, ThemeMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    pub fn from_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub surface: Hsla,
    pub panel: Hsla,
    pub panel_2: Hsla,
    pub edge: Hsla,
    pub edge_2: Hsla,
    pub line_strong: Hsla,
    pub ink: Hsla,
    pub ink_2: Hsla,
    pub ink_dim: Hsla,
    pub ink_4: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_ink: Hsla,
    pub accent_soft: Hsla,
    pub info: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub warn_ink: Hsla,
    pub bad: Hsla,
    pub bad_ink: Hsla,
    pub brand: Hsla,
    pub brand_soft: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Tokens {
    pub fn dark() -> Self {
        Self {
            surface: c(0x111113),
            panel: c(0x18181b),
            panel_2: c(0x222226),
            edge: c(0x2c2c31),
            edge_2: c(0x232327),
            line_strong: c(0x3a3a40),
            ink: c(0xececee),
            ink_2: c(0xc6c6cb),
            ink_dim: c(0x9a9aa3),
            ink_4: c(0x6f6f78),
            accent: c(0xececee),
            accent_hover: c(0xffffff),
            accent_ink: c(0x111113),
            accent_soft: c(0x27272c),
            info: c(0x5b9dff),
            ok: c(0x57c23a),
            warn: c(0xf2b92f),
            warn_ink: c(0xf2b92f),
            bad: c(0xff4d68),
            bad_ink: c(0xff7389),
            brand: c(0x60a5fa),
            brand_soft: c(0x7dd3fc),
        }
    }

    pub fn light() -> Self {
        Self {
            surface: c(0xf7f7f7),
            panel: c(0xffffff),
            panel_2: c(0xf5f5f5),
            edge: c(0xe9e9e9),
            edge_2: c(0xf0f0f0),
            line_strong: c(0xe9e9e9),
            ink: c(0x111111),
            ink_2: c(0x303030),
            ink_dim: c(0x5e5e5e),
            ink_4: c(0x8a8a8a),
            accent: c(0x111111),
            accent_hover: c(0x000000),
            accent_ink: c(0xffffff),
            accent_soft: c(0xf0f0f0),
            info: c(0x206eec),
            ok: c(0x368813),
            warn: c(0xe0a400),
            warn_ink: c(0x7a5a00),
            bad: c(0xef0028),
            bad_ink: c(0xb80020),
            brand: c(0x1d4ed8),
            brand_soft: c(0x7c3aed),
        }
    }

    pub fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self::dark(),
            Mode::Light => Self::light(),
        }
    }
}

struct Current(Mode, Tokens);
impl Global for Current {}

/// Installs `mode` and points gpui-component's theme at the same tokens, so its chrome matches.
pub fn install(mode: Mode, cx: &mut App) {
    let t = Tokens::for_mode(mode);
    cx.set_global(Current(mode, t));
    Theme::change(
        match mode {
            Mode::Dark => ThemeMode::Dark,
            Mode::Light => ThemeMode::Light,
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        let colors = &mut theme.colors;
        colors.background = t.surface;
        colors.foreground = t.ink;
        colors.border = t.edge;
        colors.title_bar = t.surface;
        colors.title_bar_border = t.edge;
        colors.window_border = t.edge;
        colors.popover = t.panel;
        colors.popover_foreground = t.ink;
        colors.muted = t.panel_2;
        colors.muted_foreground = t.ink_dim;
    });
}

pub fn mode(cx: &App) -> Mode {
    cx.try_global::<Current>().map_or(Mode::Dark, |c| c.0)
}

/// The installed mode's tokens; dark before `install`.
pub fn tokens(cx: &App) -> Tokens {
    cx.try_global::<Current>()
        .map_or_else(Tokens::dark, |c| c.1)
}
