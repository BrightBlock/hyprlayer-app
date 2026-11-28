//! `Dialog` takes a named size, never a width (ADR-0006 "The Dialog API").

use gpui::{px, Pixels};
use gpui_component::dialog::Dialog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Sm,
    Md,
    Lg,
    Xl,
    X2,
    X3,
    X4,
}

impl Size {
    /// Tailwind's `max-w-sm`…`max-w-4xl` at a 16px root.
    pub fn width(self) -> Pixels {
        px(match self {
            Self::Sm => 384.0,
            Self::Md => 448.0,
            Self::Lg => 512.0,
            Self::Xl => 576.0,
            Self::X2 => 672.0,
            Self::X3 => 768.0,
            Self::X4 => 896.0,
        })
    }
}

pub fn sized(dialog: Dialog, size: Size) -> Dialog {
    dialog.w(size.width())
}
