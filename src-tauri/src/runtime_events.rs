use std::sync::Arc;

use hyprlayer_core::models::runtime::ThoughtsIndexUpdateKind;
use hyprlayer_core::models::RuntimeEvent;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::client::RpcClient;
use crate::events;
use crate::state::AppState;

/// One bridge per client, retiring with it: `take_events()` is one-shot per
/// client, so a reconnect's fresh client simply gets a fresh bridge.
pub fn spawn(handle: AppHandle, client: Arc<RpcClient>) {
    let mut events = client
        .take_events()
        .expect("a fresh client still holds its event stream");
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            map(&handle, event);
        }
        // The stream ends only with the connection. Mark the level state
        // lost before emitting the edge, so a listener that queries
        // `connection_status` on the event already sees it (plan Phase 1 §6) —
        // and stay quiet if a reconnect has already moved on (Phase 8 §2).
        if handle.state::<AppState>().mark_lost(&client) {
            let _ = events::ServerDisconnected.emit(&handle);
        }
    });
}

fn map(handle: &AppHandle, event: RuntimeEvent) {
    match event {
        RuntimeEvent::ConfigChanged => {
            let _ = events::ConfigChanged.emit(handle);
        }
        RuntimeEvent::BinariesChanged => {
            let _ = events::BinariesChanged.emit(handle);
        }
        RuntimeEvent::ThoughtsIndexUpdated { update, ids } => {
            let kind = match update {
                ThoughtsIndexUpdateKind::Updated => "updated",
                ThoughtsIndexUpdateKind::Removed => "removed",
                ThoughtsIndexUpdateKind::Rescanned => "rescanned",
            };
            let _ = events::ThoughtsIndexUpdated {
                kind: kind.to_string(),
                ids,
            }
            .emit(handle);
        }
        RuntimeEvent::ProjectsChanged => {}
        RuntimeEvent::PrStatusChanged { workstream_id } => {
            let _ = events::PrStatusChanged { workstream_id }.emit(handle);
        }
        RuntimeEvent::WorktreeChanged { workstream_id } => {
            let _ = events::WorktreeChanged { workstream_id }.emit(handle);
        }
        RuntimeEvent::WorkstreamBranchChanged {
            workstream_id,
            branch,
            previous,
        } => {
            let _ = events::WorkstreamBranchChanged {
                workstream_id,
                branch,
                previous,
            }
            .emit(handle);
        }
        RuntimeEvent::SessionLifecycle {
            session_id,
            workstream_id,
            state,
        } => {
            let _ = events::SessionLifecycle {
                session_id,
                workstream_id,
                state,
            }
            .emit(handle);
        }
        RuntimeEvent::ScriptRunChanged {
            workstream_id,
            kind,
            phase,
            exit_code,
            port,
        } => {
            let _ = events::ScriptRunChanged {
                workstream_id,
                kind,
                phase,
                exit_code,
                port,
            }
            .emit(handle);
        }
        RuntimeEvent::ProjectCreateProgress {
            op_id,
            step,
            status,
            detail,
        } => {
            let _ = events::ProjectCreateProgress {
                op_id,
                step,
                status,
                detail,
            }
            .emit(handle);
        }
    }
}
