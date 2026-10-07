use std::io::ErrorKind;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::error::{CoreError, Result};

const BIND_ATTEMPTS: u32 = 12;
const BIND_RETRY: Duration = Duration::from_millis(60);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEAD: usize = 16 * 1024;

/// What the browser brought back to `/auth/callback`, whose `state` already matched.
pub struct Callback {
    pub code: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    /// The callback's request, held open so the browser lands only once the exchange is decided.
    browser: Option<TcpStream>,
}

impl Callback {
    pub async fn reply(&mut self, signed_in: bool) {
        let Some(mut stream) = self.browser.take() else {
            return;
        };
        if signed_in {
            reply(&mut stream, "302 Found", SUCCESS_REDIRECT, "").await;
        } else {
            let body = page("Sign-in failed. Return to Hyprlayer to try again.");
            reply(&mut stream, "200 OK", HTML, &body).await;
        }
    }
}

/// The sign-in redirect's listener. It runs where the browser does, because the
/// browser's `localhost` is that machine's.
pub struct Loopback {
    v4: TcpListener,
    v6: Option<TcpListener>,
    port: u16,
}

enum Bound {
    Free(Loopback),
    Busy(u16),
}

impl Loopback {
    /// First candidate free on 127.0.0.1 and ::1 (v4-only only where IPv6 is absent); 0 asks the OS.
    pub async fn bind_first(candidates: &[u16]) -> Result<Self> {
        let mut busy = Vec::new();
        for attempt in 0..BIND_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(BIND_RETRY).await;
            }
            busy.clear();
            for &candidate in candidates {
                match Self::bind(candidate).await? {
                    Bound::Free(loopback) => return Ok(loopback),
                    Bound::Busy(port) => busy.push(port.to_string()),
                }
            }
        }
        Err(CoreError::AccountPortsBusy {
            ports: busy.join(" or "),
        })
    }

    async fn bind(port: u16) -> std::io::Result<Bound> {
        let v4 = match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
            Ok(v4) => v4,
            Err(e) if e.kind() == ErrorKind::AddrInUse => return Ok(Bound::Busy(port)),
            Err(e) => return Err(e),
        };
        let port = v4.local_addr()?.port();
        match TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await {
            Ok(v6) => Ok(Bound::Free(Self {
                v4,
                v6: Some(v6),
                port,
            })),
            // Vite listens on `[::1]`; a browser resolving `localhost` there would never reach us.
            Err(e) if e.kind() == ErrorKind::AddrInUse => Ok(Bound::Busy(port)),
            // Anything else means this host has no IPv6 loopback to resolve to.
            Err(_) => Ok(Bound::Free(Self { v4, v6: None, port })),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Callers cancel with `notify_one`, so a cancel that lands before this call still counts.
    pub async fn wait(
        &self,
        timeout: Duration,
        state: &str,
        authorize_url: &str,
        cancel: &Notify,
    ) -> Result<Callback> {
        tokio::select! {
            outcome = tokio::time::timeout(timeout, self.accept_callback(state, authorize_url)) => {
                outcome.map_err(|_| CoreError::AccountSignInTimedOut)?
            }
            _ = cancel.notified() => Err(CoreError::AccountSignInCancelled),
        }
    }

    async fn accept_callback(&self, state: &str, authorize_url: &str) -> Result<Callback> {
        // A task per connection, so an idle one (a browser's preconnect) never holds up the next.
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = self.accept() => {
                    let stream = accepted?;
                    let state = state.to_string();
                    let authorize_url = authorize_url.to_string();
                    connections.spawn(async move {
                        tokio::time::timeout(READ_TIMEOUT, answer(stream, &state, &authorize_url))
                            .await
                            .ok()
                            .flatten()
                    });
                }
                Some(answered) = connections.join_next() => {
                    if let Ok(Some(callback)) = answered {
                        return Ok(callback);
                    }
                }
            }
        }
    }

    async fn accept(&self) -> std::io::Result<TcpStream> {
        let (stream, _) = match &self.v6 {
            Some(v6) => tokio::select! {
                accepted = self.v4.accept() => accepted,
                accepted = v6.accept() => accepted,
            },
            None => self.v4.accept().await,
        }?;
        Ok(stream)
    }
}

/// Answers one request; `Some` only for this attempt's own callback.
async fn answer(mut stream: TcpStream, state: &str, authorize_url: &str) -> Option<Callback> {
    let target = read_target(&mut stream).await?;
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok()?;
    match url.path() {
        "/auth/loggedout" => {
            let location = format!("Location: {authorize_url}\r\n");
            reply(&mut stream, "302 Found", &location, "").await;
            None
        }
        "/auth/callback" => {
            let mut callback = Callback {
                code: None,
                error: None,
                error_description: None,
                browser: None,
            };
            let mut echoed = None;
            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "code" => callback.code = Some(value.into_owned()),
                    "state" => echoed = Some(value.into_owned()),
                    "error" => callback.error = Some(value.into_owned()),
                    "error_description" => callback.error_description = Some(value.into_owned()),
                    _ => {}
                }
            }
            // Cognito echoes `state` on success and error alike; without it the request is stray or forged.
            if echoed.as_deref() != Some(state) {
                let body = page("Unrecognized sign-in request.");
                reply(&mut stream, "400 Bad Request", HTML, &body).await;
                return None;
            }
            if callback.error.is_some() {
                let body = page("Sign-in was cancelled. You can close this tab.");
                reply(&mut stream, "200 OK", HTML, &body).await;
            } else {
                callback.browser = Some(stream);
            }
            Some(callback)
        }
        _ => {
            reply(&mut stream, "404 Not Found", HTML, &page("Not found.")).await;
            None
        }
    }
}

/// Reads the whole request head before answering: closing on unread bytes resets the
/// connection, and the browser can drop the reply with it.
async fn read_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_HEAD {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    let line_end = head.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&head[..line_end]).ok()?;
    line.split(' ').nth(1).map(str::to_string)
}

/// The sign-in landing page; carries no code or state, and the website is the only UI a sign-in shows.
const SUCCESS_REDIRECT: &str =
    "Location: https://hyprlayer.dev/signin/success\r\nCache-Control: no-store\r\n";

const HTML: &str = "Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n";

async fn reply(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Served to the system browser by this socket, so the webview's CSP never applies.
fn page(message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Hyprlayer</title>\
         <style>html{{color-scheme:light dark}}body{{font:15px system-ui,sans-serif;\
         display:grid;place-items:center;height:100vh;margin:0}}</style></head>\
         <body><p>{message}</p></body></html>"
    )
}
