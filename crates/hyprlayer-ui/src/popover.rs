//! Floating panels are a `Popover` (ADR-0006 "Floating panels"): one anchor and one offset.

use gpui::{px, Anchor, ElementId};
use gpui_component::popover::Popover;

pub fn popover(id: impl Into<ElementId>) -> Popover {
    Popover::new(id).anchor(Anchor::TopLeft).offset(px(4.0))
}
