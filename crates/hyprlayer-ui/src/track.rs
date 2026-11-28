//! Each tracked element's visible window bounds by id, re-recorded every frame, so the drive
//! bridge and tests can find, click and list what is painted.

use std::collections::BTreeMap;

use gpui::{canvas, App, Bounds, Global, ParentElement, Pixels, SharedString, Styled};

#[derive(Default)]
pub struct Tracker(BTreeMap<SharedString, Bounds<Pixels>>);

impl Global for Tracker {}

impl Tracker {
    /// Called by the root view at the top of its render, before this frame's elements record.
    pub fn clear(cx: &mut App) {
        cx.default_global::<Self>().0.clear();
    }

    pub fn find(cx: &App, id: &str) -> Option<Bounds<Pixels>> {
        cx.try_global::<Self>()?.0.get(id).copied()
    }

    pub fn ids(cx: &App) -> Vec<(SharedString, Bounds<Pixels>)> {
        cx.try_global::<Self>()
            .map(|t| t.0.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default()
    }
}

/// `el` must be positioned (`relative` or `absolute`): the probe fills it. Clipped-out parts
/// are not recorded, so a row scrolled out of its list is absent.
pub fn tracked<E: ParentElement + Styled>(id: impl Into<SharedString>, el: E) -> E {
    let id = id.into();
    el.child(
        canvas(
            move |bounds, window, cx| {
                let visible = bounds.intersect(&window.content_mask().bounds);
                if !visible.is_empty() {
                    cx.default_global::<Tracker>().0.insert(id, visible);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full(),
    )
}
