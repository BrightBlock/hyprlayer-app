//! One generic cached read. Its fetch runs on the io thread; a reconnect, an `EventsMissed`, or
//! a runtime event whose topic it lists (`events::topics_for`) re-reads it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gpui::{App, Context, Entity, Subscription};
use hyprlayer_client::client::RpcClient;
use hyprlayer_proto::WireError;

use crate::connection::{Connection, Notice};
use crate::error::UiError;
use crate::events::{topics_for, Topic};
use crate::io::Io;

pub type Fetched<T> = Pin<Box<dyn Future<Output = Result<T, UiError>> + Send>>;
pub type Fetch<K, T> = Arc<dyn Fn(Arc<RpcClient>, K) -> Fetched<T> + Send + Sync>;

pub fn fetch<K, T, Fut>(
    call: impl Fn(Arc<RpcClient>, K) -> Fut + Send + Sync + 'static,
) -> Fetch<K, T>
where
    Fut: Future<Output = Result<T, WireError>> + Send + 'static,
{
    Arc::new(move |client, key| {
        let read = call(client, key);
        Box::pin(async move { read.await.map_err(UiError::from) })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    Fresh,
    /// Wanted; waits for a live client.
    Stale,
    /// `again`: invalidated while in flight, so it reads once more when this one lands.
    InFlight {
        again: bool,
    },
}

pub struct Query<K, T> {
    key: K,
    topics: &'static [Topic],
    fetch: Fetch<K, T>,
    connection: Entity<Connection>,
    io: Io,
    /// With the connection generation it was read on.
    value: Option<(u64, T)>,
    error: Option<UiError>,
    read: Read,
    fetches: u64,
    _notices: Subscription,
}

impl<K: Clone + Send + 'static, T: Send + 'static> Query<K, T> {
    pub fn new(
        key: K,
        topics: &'static [Topic],
        fetch: Fetch<K, T>,
        connection: Entity<Connection>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        let notices = cx.subscribe(&connection, |this, _, notice: &Notice, cx| {
            let wanted = match notice {
                Notice::Connected | Notice::EventsMissed => true,
                Notice::Runtime(event) => topics_for(event).iter().any(|t| this.topics.contains(t)),
                Notice::Lost => false,
            };
            if wanted {
                this.invalidate(cx);
            }
        });
        let mut this = Self {
            key,
            topics,
            fetch,
            connection,
            io,
            value: None,
            error: None,
            read: Read::Stale,
            fetches: 0,
            _notices: notices,
        };
        this.start(cx);
        this
    }

    /// The value, only if it was read on the live connection: a lost link's lists are not current.
    pub fn current(&self, cx: &App) -> Option<&T> {
        let live = self.connection.read(cx).live_generation()?;
        self.value
            .as_ref()
            .filter(|(read_on, _)| *read_on == live)
            .map(|(_, value)| value)
    }

    pub fn error(&self) -> Option<&UiError> {
        self.error.as_ref()
    }

    /// Reads started, so a check can tell a re-read from a value that merely survived.
    pub fn fetches(&self) -> u64 {
        self.fetches
    }

    pub fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.read = match self.read {
            Read::InFlight { .. } => Read::InFlight { again: true },
            Read::Fresh | Read::Stale => Read::Stale,
        };
        self.start(cx);
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.read != Read::Stale {
            return;
        }
        let connection = self.connection.read(cx);
        let (Some(client), Some(generation)) = (connection.client(), connection.live_generation())
        else {
            return;
        };
        self.read = Read::InFlight { again: false };
        self.fetches += 1;
        let landed = self.io.spawn((self.fetch)(client, self.key.clone()));
        cx.spawn(async move |this, cx| {
            let result = landed.await;
            let _ = this.update(cx, |this, cx| {
                let again = matches!(this.read, Read::InFlight { again: true });
                this.read = if again { Read::Stale } else { Read::Fresh };
                match result {
                    Ok(Ok(value)) => {
                        this.value = Some((generation, value));
                        this.error = None;
                    }
                    Ok(Err(error)) => this.error = Some(error),
                    Err(_) => this.read = Read::Stale,
                }
                cx.notify();
                this.start(cx);
            });
        })
        .detach();
    }
}
