//! The screen shown while there is no server: what is wrong, what to do, and a retry.

use gpui::{
    div, px, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use hyprlayer_ui::icon::{icon, Key, Size};
use hyprlayer_ui::theme::tokens;
use hyprlayer_ui::track::tracked;
use hyprlayer_ui::type_scale::{Type, Typed as _};

use crate::connection::Status;

pub struct Recovery {
    title: SharedString,
    message: SharedString,
    remedy: Option<SharedString>,
}

impl Recovery {
    /// `None` while connected, and during the launch attempt, which has nothing to report yet.
    pub fn for_status(status: &Status, target: &str) -> Option<Self> {
        match status {
            Status::Connected { .. } | Status::Connecting => None,
            Status::Lost { error } => Some(Self {
                title: "Lost the connection".into(),
                message: match error {
                    Some(error) => error.message.clone().into(),
                    None => format!("The Hyprlayer server on {target} stopped answering.").into(),
                },
                remedy: error
                    .as_ref()
                    .and_then(|e| e.remedy.clone())
                    .map(Into::into),
            }),
            Status::Blocked { title, error } => Some(Self {
                title: title.clone().into(),
                message: error.message.clone().into(),
                remedy: error.remedy.clone().map(Into::into),
            }),
        }
    }

    pub fn render(
        self,
        attempting: bool,
        retry: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> impl IntoElement {
        let t = tokens(cx);
        let text = |id: &'static str, token: Type, body: SharedString| {
            tracked(
                id,
                div().relative().typed(token).max_w(px(520.0)).child(body),
            )
        };
        let mut body = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .bg(t.surface)
            .text_color(t.ink)
            .child(icon(Key::Warning, Size::Lg).text_color(t.warn))
            .child(text("recovery-title", Type::H3, self.title))
            .child(text("recovery-message", Type::Body, self.message).text_color(t.ink_dim));
        if let Some(remedy) = self.remedy {
            body = body.child(text("recovery-remedy", Type::Ui, remedy));
        }
        let label = if attempting {
            "Reconnecting…"
        } else {
            "Retry now"
        };
        let button = div()
            .id("recovery-retry")
            .relative()
            .typed(Type::Ui)
            .px(px(12.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(3.0))
            .bg(t.accent)
            .text_color(t.accent_ink)
            .cursor_pointer()
            .on_click(move |_, window, cx| retry(window, cx))
            .child(icon(Key::Retry, Size::Sm))
            .child(label);
        tracked("recovery", body.child(tracked("recovery-retry", button)))
    }
}
