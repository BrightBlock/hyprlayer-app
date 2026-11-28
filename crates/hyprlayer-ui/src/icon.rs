//! Semantic icon keys and size tiers (ADR-0006 "Icons"); keys and glyphs follow `Icon.tsx`.
//! Only gpui-component's bundled Lucide subset is embedded, so a new key must name one of those.

use gpui::{px, Pixels, Styled};
use gpui_component::{Icon, IconName};

pub use gpui_kit_assets::Assets;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Add,
    Agents,
    Check,
    ChevronDown,
    ChevronRight,
    Close,
    Console,
    Copy,
    Folder,
    Info,
    Loading,
    Retry,
    Search,
    Settings,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Xs,
    Sm,
    Md,
    Rail,
    Lg,
}

impl Size {
    pub fn px(self) -> Pixels {
        px(match self {
            Self::Xs => 12.0,
            Self::Sm => 14.0,
            Self::Md => 16.0,
            Self::Rail => 18.0,
            Self::Lg => 24.0,
        })
    }
}

impl Key {
    fn glyph(self) -> IconName {
        match self {
            Self::Add => IconName::Plus,
            Self::Agents => IconName::Bot,
            Self::Check => IconName::Check,
            Self::ChevronDown => IconName::ChevronDown,
            Self::ChevronRight => IconName::ChevronRight,
            Self::Close => IconName::Close,
            Self::Console => IconName::SquareTerminal,
            Self::Copy => IconName::Copy,
            Self::Folder => IconName::Folder,
            Self::Info => IconName::Info,
            Self::Loading => IconName::LoaderCircle,
            Self::Retry => IconName::RotateCw,
            Self::Search => IconName::Search,
            Self::Settings => IconName::Settings,
            Self::Warning => IconName::TriangleAlert,
        }
    }
}

/// Coloured by the caller's `text_color`, as `currentColor` is in the webview.
pub fn icon(key: Key, size: Size) -> Icon {
    Icon::new(key.glyph()).size(size.px())
}
