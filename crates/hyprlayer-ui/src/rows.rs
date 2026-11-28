//! Two row tiers, never three (ADR-0006 "Row tiers"), with the list-row hover.

use gpui::{div, px, App, Div, InteractiveElement, Pixels, Stateful, Styled};

use crate::theme::tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Compact,
    Standard,
}

impl Tier {
    pub fn height(self) -> Pixels {
        match self {
            Self::Compact => px(28.0),
            Self::Standard => px(36.0),
        }
    }

    fn radius(self) -> Pixels {
        match self {
            Self::Compact => px(6.0),
            Self::Standard => px(8.0),
        }
    }
}

/// A positioned list row at `tier`, hovering `edge/50` (`rowClass`'s `ROW_HOVER`).
pub fn row(id: impl Into<gpui::ElementId>, tier: Tier, cx: &App) -> Stateful<Div> {
    let edge = tokens(cx).edge;
    div()
        .id(id)
        .relative()
        .h(tier.height())
        .rounded(tier.radius())
        .flex()
        .flex_row()
        .items_center()
        .px(px(10.0))
        .cursor_pointer()
        .hover(move |s| s.bg(edge.opacity(0.5)))
}
