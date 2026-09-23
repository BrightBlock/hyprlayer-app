//! End-to-end harness: the real `hyprlayer-server` binary in a sandboxed
//! profile, driven over its socket with the desktop's own `RpcClient`.

pub mod containers;
pub mod flows;
mod journey;
mod sandbox;
mod server;
mod terminal;

pub use hyprlayer_core::RuntimeEvent;
pub use hyprlayer_server::rpc::Request;
pub use journey::Journey;
pub use sandbox::Sandbox;
pub use serde_json::{json, Value};
pub use server::{Server, Stopped};
pub use terminal::{keystrokes, read_until, wait_for_exit};

/// Polls `check` until it holds or `within` passes.
pub async fn eventually(within: std::time::Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if check() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Collects runtime events until `stop` matches one or `within` passes, and
/// returns everything seen. The server pushes these; nothing else in the
/// harness observes them, so a scenario that wants one has to wait for it.
pub async fn events_until(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RuntimeEvent>,
    within: std::time::Duration,
    stop: impl Fn(&RuntimeEvent) -> bool,
) -> Vec<RuntimeEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(within, async {
        while let Some(event) = rx.recv().await {
            let done = stop(&event);
            seen.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    seen
}

/// The string at a JSON pointer, or `""` when absent.
pub fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
