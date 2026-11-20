use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hyprlayer_core::terminal::TerminalEvent;
use hyprlayer_server::client::TerminalStream;
use tokio::time::Instant;

/// Terminal input is base64 on the wire.
pub fn keystrokes(text: &str) -> String {
    STANDARD.encode(text)
}

/// Decoded output, without escape sequences, until `needle` appears, the
/// terminal exits, or `within` passes. The caller asserts on what came back.
pub async fn read_until(events: &mut TerminalStream, needle: &str, within: Duration) -> String {
    let deadline = Instant::now() + within;
    let mut text = String::new();
    while !text.contains(needle) {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(TerminalEvent::Output { data, .. })) => {
                text.push_str(&String::from_utf8_lossy(
                    &STANDARD.decode(data).unwrap_or_default(),
                ));
            }
            Ok(Some(TerminalEvent::Exited { .. })) | Ok(None) | Err(_) => break,
        }
    }
    plain(&text)
}

/// Drops CSI and OSC sequences: bash's bracketed-paste toggle would otherwise
/// prefix the first line of a command's output.
fn plain(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The exit code once the terminal reports `Exited`; `None` if it never does.
pub async fn wait_for_exit(events: &mut TerminalStream, within: Duration) -> Option<Option<i32>> {
    let deadline = Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(TerminalEvent::Exited { code })) => return Some(code),
            Ok(Some(TerminalEvent::Output { .. })) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}
