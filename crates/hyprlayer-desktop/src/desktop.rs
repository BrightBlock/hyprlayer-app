//! The root view: plain lists of projects, workstreams and sessions, the recovery screen while
//! there is no server, and the state dump the drive bridge and the E2E tier read.

use gpui::{
    div, px, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};
use gpui_component::TitleBar;
use hyprlayer_core::runtime::projects::ProjectView;
use hyprlayer_core::runtime::sessions::ActiveSession;
use hyprlayer_server::rpc::WorkstreamView;
use hyprlayer_ui::rows::{row, Tier};
use hyprlayer_ui::theme::{self, tokens, Mode};
use hyprlayer_ui::track::{tracked, Tracker};
use hyprlayer_ui::type_scale::{Type, Typed as _};
use serde_json::{json, Value};

use crate::config::Config;
use crate::connection::{Connection, Status};
use crate::events::Topic;
use crate::io::Io;
use crate::query::{fetch, Fetch, Query};
use crate::recovery::Recovery;

pub struct Desktop {
    pub connection: Entity<Connection>,
    pub projects: Entity<Query<(), Vec<ProjectView>>>,
    pub workstreams: Entity<Query<(), Vec<WorkstreamView>>>,
    pub sessions: Entity<Query<(), Vec<ActiveSession>>>,
    _subscriptions: Vec<Subscription>,
}

/// One list's rows as (tracker key, label).
type Rows = Vec<(String, String)>;

fn list<T: Send + 'static>(
    topics: &'static [Topic],
    read: Fetch<(), T>,
    connection: &Entity<Connection>,
    io: &Io,
    cx: &mut Context<Desktop>,
) -> Entity<Query<(), T>> {
    let (connection, io) = (connection.clone(), io.clone());
    cx.new(|cx| Query::new((), topics, read, connection, io, cx))
}

impl Desktop {
    pub fn new(config: Config, io: Io, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mode = Mode::from_appearance(window.appearance());
        theme::install(mode, cx);
        let connection = cx.new(|cx| Connection::new(config, io.clone(), mode, cx));
        let projects = list(
            &[Topic::Bootstrap],
            fetch(|c, ()| async move { c.get_bootstrap().await.map(|b| b.projects) }),
            &connection,
            &io,
            cx,
        );
        let workstreams = list(
            &[Topic::Workstreams],
            fetch(|c, ()| async move { c.list_all_workstreams().await }),
            &connection,
            &io,
            cx,
        );
        let sessions = list(
            &[Topic::ActiveSessions],
            fetch(|c, ()| async move { c.list_active_sessions().await }),
            &connection,
            &io,
            cx,
        );
        let subscriptions = vec![
            cx.observe(&connection, |_, _, cx| cx.notify()),
            cx.observe(&projects, |_, _, cx| cx.notify()),
            cx.observe(&workstreams, |_, _, cx| cx.notify()),
            cx.observe(&sessions, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |this, window, cx| {
                let mode = Mode::from_appearance(window.appearance());
                if mode != theme::mode(cx) {
                    theme::install(mode, cx);
                    this.connection.update(cx, |c, cx| c.set_mode(mode, cx));
                }
            }),
            // The user coming back is the strongest signal a retry is worth making (`reconnect.ts`).
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                this.connection.update(cx, |c, cx| {
                    c.set_foreground(active);
                    if active {
                        c.connect(cx);
                    }
                });
            }),
        ];
        connection.update(cx, |c, cx| c.connect(cx));
        Self {
            connection,
            projects,
            workstreams,
            sessions,
            _subscriptions: subscriptions,
        }
    }

    fn project_rows(&self, cx: &App) -> Option<Rows> {
        let projects = self.projects.read(cx).current(cx)?;
        Some(
            projects
                .iter()
                .map(|p| (p.path.clone(), p.mapped_name.clone()))
                .collect(),
        )
    }

    fn workstream_rows(&self, cx: &App) -> Option<Rows> {
        let workstreams = self.workstreams.read(cx).current(cx)?;
        Some(
            workstreams
                .iter()
                .map(|w| (w.item.id.clone(), w.item.title.clone()))
                .collect(),
        )
    }

    fn session_rows(&self, cx: &App) -> Option<Rows> {
        let sessions = self.sessions.read(cx).current(cx)?;
        Some(
            sessions
                .iter()
                .map(|a| {
                    let label = format!("{} · {}", a.workstream.title, a.session.stage.as_str());
                    (a.session.id.clone(), label)
                })
                .collect(),
        )
    }

    /// What the drive bridge's `state` op returns. A list is `null` unless it was read on the
    /// live connection, so a lost link never reports stale rows as current.
    pub fn dump(&self, cx: &App) -> Value {
        let conn = self.connection.read(cx);
        let connection = match conn.status() {
            Status::Connecting => json!({ "state": "connecting" }),
            Status::Connected { outdated } => json!({ "state": "connected", "outdated": outdated }),
            Status::Lost { error } => json!({
                "state": "lost",
                "code": error.as_ref().map(|e| &e.code),
                "message": error.as_ref().map(|e| &e.message),
            }),
            Status::Blocked { title, error } => {
                json!({ "state": "blocked", "title": title, "code": error.code, "message": error.message })
            }
        };
        let rows = |rows: Option<Rows>| {
            rows.map(|rows| {
                rows.into_iter()
                    .map(|(key, label)| json!({ "key": key, "label": label }))
                    .collect::<Vec<_>>()
            })
        };
        let painted: Vec<SharedString> = Tracker::ids(cx).into_iter().map(|(id, _)| id).collect();
        json!({
            "target": conn.target().name(),
            "connection": connection,
            "attempting": conn.attempting(),
            "connects": conn.connects(),
            "gaps": conn.gaps(),
            "transcriptSync": conn.transcript_sync(),
            "lists": {
                "projects": rows(self.project_rows(cx)),
                "workstreams": rows(self.workstream_rows(cx)),
                "sessions": rows(self.session_rows(cx)),
            },
            "fetches": {
                "projects": self.projects.read(cx).fetches(),
                "workstreams": self.workstreams.read(cx).fetches(),
                "sessions": self.sessions.read(cx).fetches(),
            },
            "painted": painted,
        })
    }

    fn section(
        &self,
        id: &'static str,
        title: &str,
        rows: Option<Rows>,
        cx: &App,
    ) -> impl IntoElement {
        let t = tokens(cx);
        let heading = match &rows {
            Some(rows) => format!("{} · {}", title.to_uppercase(), rows.len()),
            None => format!("{} · …", title.to_uppercase()),
        };
        let list = rows.unwrap_or_default().into_iter().map(|(key, label)| {
            let key = format!("{id}:{key}");
            tracked(
                key.clone(),
                row(SharedString::from(key), Tier::Standard, cx)
                    .typed(Type::Ui)
                    .child(label),
            )
        });
        tracked(
            id,
            div()
                .id(id)
                .relative()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .border_r_1()
                .border_color(t.edge)
                .child(
                    div()
                        .h(px(28.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .typed(Type::Label)
                        .text_color(t.ink_dim)
                        .child(heading),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-rows")))
                        .flex_1()
                        .overflow_y_scroll()
                        .px(px(4.0))
                        .children(list),
                ),
        )
    }
}

impl Render for Desktop {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Tracker::clear(cx);
        let t = tokens(cx);
        let (status, target, attempting) = {
            let c = self.connection.read(cx);
            (c.status().clone(), c.target().name(), c.attempting())
        };
        let body = match Recovery::for_status(&status, &target) {
            Some(recovery) => {
                let connection = self.connection.clone();
                let retry = move |_: &mut Window, cx: &mut App| {
                    connection.update(cx, |c, cx| c.connect(cx))
                };
                recovery.render(attempting, retry, cx).into_any_element()
            }
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(self.section("projects", "Projects", self.project_rows(cx), cx))
                .child(self.section("workstreams", "Workstreams", self.workstream_rows(cx), cx))
                .child(self.section("sessions", "Sessions", self.session_rows(cx), cx))
                .into_any_element(),
        };
        let label = match status {
            Status::Connected { .. } => format!("Connected to {target}"),
            Status::Connecting => format!("Connecting to {target}"),
            Status::Lost { .. } | Status::Blocked { .. } => format!("Not connected to {target}"),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.surface)
            .text_color(t.ink)
            .typed(Type::Body)
            .child(
                TitleBar::new().on_close_window(|_, _, cx| cx.quit()).child(
                    div()
                        .typed(Type::Ui)
                        .text_color(t.ink_dim)
                        .child("Hyprlayer"),
                ),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .child(tracked(
                "status",
                div()
                    .relative()
                    .h(px(22.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(t.edge)
                    .typed(Type::Meta)
                    .text_color(t.ink_dim)
                    .child(label),
            ))
    }
}
