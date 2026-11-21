use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hyprlayer_core::models::runtime::ThoughtsIndexUpdateKind;
use hyprlayer_core::models::RuntimeEvent;
use hyprlayer_server::client::GapPacer;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::client::RpcClient;
use crate::commands::hosts::ConnectionState;
use crate::events;
use crate::state::AppState;

/// One bridge per client, retiring with it: `take_events()` is one-shot per
/// client, so a reconnect's fresh client simply gets a fresh bridge.
pub fn spawn(handle: AppHandle, client: Arc<RpcClient>) {
    let mut events = client
        .take_events()
        .expect("a fresh client still holds its event stream");
    let mut gaps = GapPacer::new(client.events_missed());
    let paced = handle.clone();
    tauri::async_runtime::spawn(async move {
        while gaps.next().await {
            log::warn!("runtime events were missed; the window re-reads");
            let _ = events::EventsMissed.emit(&paced);
        }
    });
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            map(&handle, event);
        }
        // The stream ends only with the connection. Mark the level state
        // lost before emitting the edge, so a listener that queries
        // `connection_status` on the event already sees it (plan Phase 1 §6) —
        // and stay quiet if a reconnect has already moved on (Phase 8 §2).
        if handle.state::<AppState>().mark_lost(&client) {
            log::warn!("the server connection is lost; the window will try to reconnect");
            let _ = events::ServerDisconnected.emit(&handle);
            pace_reconnect(handle, true);
        }
    });
}

/// Quick at first, then settled; never gives up, since relaunching is what reconnecting spares.
const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];
const RECONNECT_STEADY: Duration = Duration::from_secs(30);

static PACER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// Starts the schedule from its first step, or stops it; at most one runs. The window still makes each attempt.
pub fn pace_reconnect(handle: AppHandle, on: bool) {
    let mut pacer = PACER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(old) = pacer.take() {
        old.abort();
    }
    if !on {
        return;
    }
    *pacer = Some(tauri::async_runtime::spawn(async move {
        for attempt in 0.. {
            let delay = RECONNECT_DELAYS.get(attempt).copied();
            tokio::time::sleep(delay.unwrap_or(RECONNECT_STEADY)).await;
            if handle.state::<AppState>().status().state == ConnectionState::Connected {
                return;
            }
            let _ = events::ReconnectDue.emit(&handle);
        }
    }));
}

// A reply that lands once a hidden page has suspended waits for its next wake; the pacer stops at Connected.
pub fn wake_after_reply(handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = events::ReconnectDue.emit(&handle);
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
        RuntimeEvent::AccountChanged => {
            let _ = events::AccountChanged.emit(handle);
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
        RuntimeEvent::DocStateChanged { ids, change } => {
            let _ = events::DocStateChanged { ids, change }.emit(handle);
        }
        RuntimeEvent::ProjectsChanged => {}
        RuntimeEvent::GoalChanged {
            goal_id,
            session_id,
            workstream_id,
            status,
            step,
            pause_reason,
            end_reason,
            reason,
            iteration,
            max_iterations,
        } => {
            let _ = events::GoalChanged {
                goal_id,
                session_id,
                workstream_id,
                status,
                step,
                pause_reason,
                end_reason,
                reason,
                iteration,
                max_iterations,
            }
            .emit(handle);
        }
        RuntimeEvent::PrStatusChanged {
            workstream_id,
            observed_pr_changed,
        } => {
            let _ = events::PrStatusChanged {
                workstream_id,
                observed_pr_changed,
            }
            .emit(handle);
        }
        RuntimeEvent::WorktreeChanged { workstream_id } => {
            let _ = events::WorktreeChanged { workstream_id }.emit(handle);
        }
        RuntimeEvent::ReviewChanged { workstream_id } => {
            let _ = events::ReviewChanged { workstream_id }.emit(handle);
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
        RuntimeEvent::SessionRevised {
            session_id,
            workstream_id,
            rev,
            turns,
            author,
            working,
            at,
        } => {
            let _ = events::SessionRevised {
                session_id,
                workstream_id,
                rev: rev as f64,
                turns: turns
                    .into_iter()
                    .map(|t| events::RevisedTurnDto {
                        seq: t.seq as f64,
                        kind: t.kind,
                    })
                    .collect(),
                author,
                working,
                at,
            }
            .emit(handle);
        }
        RuntimeEvent::SessionReset {
            session_id,
            workstream_id,
            rev,
        } => {
            let _ = events::SessionReset {
                session_id,
                workstream_id,
                rev: rev as f64,
            }
            .emit(handle);
        }
        RuntimeEvent::SessionViewers {
            session_id,
            viewers,
        } => {
            let _ = events::SessionViewers {
                session_id,
                viewers,
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
        RuntimeEvent::PtyAgentChanged {
            terminal_id,
            workstream_id,
            engine_id,
            state,
            provider_session_id,
            activity,
            last_reply,
            title,
            status,
        } => {
            let _ = events::PtyAgentChanged {
                terminal_id,
                workstream_id,
                engine_id,
                state,
                provider_session_id,
                activity,
                last_reply,
                title,
                status,
            }
            .emit(handle);
        }
        RuntimeEvent::UiRequested { request } => {
            let _ = events::UiRequested { request }.emit(handle);
        }
    }
}
