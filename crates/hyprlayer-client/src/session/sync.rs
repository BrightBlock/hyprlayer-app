//! Port of `mergeEntries` and `closedTurnIn` from `src/features/session/transcriptSync.ts`.

use hyprlayer_core::{SessionEvent, SessionLogEntry};
use serde::Serialize;

use super::transcript::{reduce_event, ItemKind, SessionView};

/// The last turn the rows both opened and ended, timed by them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastTurn {
    pub ms: f64,
    pub ended_at: f64,
}

pub struct Merged {
    pub view: SessionView,
    pub applied_seq: i64,
    pub first_new_seq: Option<i64>,
    pub revisions: u32,
    pub reset: bool,
    pub closed_turn: Option<LastTurn>,
}

fn is_commit(event: &SessionEvent) -> bool {
    matches!(
        event,
        SessionEvent::UserMessage { .. }
            | SessionEvent::AssistantText { .. }
            | SessionEvent::ToolResult { .. }
    )
}

pub fn merge_entries(
    mut view: SessionView,
    applied_seq: i64,
    entries: &[SessionLogEntry],
) -> Merged {
    let unchanged = |view, reset| Merged {
        view,
        applied_seq,
        first_new_seq: None,
        revisions: 0,
        reset,
        closed_turn: None,
    };
    // A rewind reuses seqs, so its boundary is found whatever its seq.
    if entries
        .iter()
        .any(|e| matches!(e.event, SessionEvent::Rewound {}))
    {
        return unchanged(view, true);
    }
    let mut fresh: Vec<&SessionLogEntry> = entries.iter().filter(|e| e.seq > applied_seq).collect();
    fresh.sort_by_key(|e| e.seq);
    if fresh.is_empty() {
        return unchanged(view, false);
    }

    let mut revisions = 0;
    for entry in &fresh {
        if !reduce_event(&mut view, &entry.event, Some(entry.seq)) {
            continue;
        }
        if is_commit(&entry.event) {
            revisions += 1;
        }
        if matches!(entry.event, SessionEvent::UserMessage { .. }) {
            if let Some(last) = view.items.last_mut() {
                if let ItemKind::User {
                    created_at: at @ None,
                    ..
                } = &mut last.kind
                {
                    if last.seq == Some(entry.seq) {
                        *at = Some(entry.created_at);
                    }
                }
            }
        }
    }
    Merged {
        view,
        applied_seq: fresh[fresh.len() - 1].seq,
        first_new_seq: Some(fresh[0].seq),
        revisions,
        reset: false,
        closed_turn: closed_turn_in(fresh.iter().copied()),
    }
}

pub fn closed_turn_in<'a>(
    entries: impl IntoIterator<Item = &'a SessionLogEntry>,
) -> Option<LastTurn> {
    let mut opened_at = None;
    let mut closed = None;
    for entry in entries {
        match entry.event {
            SessionEvent::UserMessage { .. } => {
                opened_at.get_or_insert(entry.created_at);
            }
            SessionEvent::TurnDone { .. } => {
                if let Some(opened) = opened_at.take() {
                    closed = Some(LastTurn {
                        ms: entry.created_at - opened,
                        ended_at: entry.created_at,
                    });
                }
            }
            _ => {}
        }
    }
    closed
}
