//! Folds a recorded session log the way the webview does and snapshots every derived view, for the differential test.

use std::collections::HashSet;

use hyprlayer_core::{SessionEvent, SessionLogEntry, SessionState};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::sync::{closed_turn_in, merge_entries, LastTurn};
use super::transcript::{
    cancel_pending_prompts, reduce_event, resolve_permission, resolve_question, restored_view,
    settle_unwaited_asks, ItemKind, PermissionDecision, SessionView,
};

const MERGE_PAGE: usize = 25;

/// A view reduced to comparable text: its status fields, and one hash per item.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub status: String,
    pub item_count: usize,
    pub items: Vec<String>,
}

#[derive(Serialize)]
pub struct Checkpoint {
    pub index: usize,
    pub view: Snapshot,
}

/// Number formatting and key order match the TS runner's `canon`; nulls are dropped.
pub fn canon(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => f.to_string(),
            _ => n.to_string(),
        },
        Value::String(s) => serde_json::to_string(s).expect("encode string"),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canon).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, _)| k)
                .collect();
            keys.sort();
            let body: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).expect("encode key"),
                        canon(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", body.join(","))
        }
    }
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn item_canons(view: &SessionView) -> Vec<String> {
    view.items
        .iter()
        .map(|it| canon(&serde_json::to_value(it).expect("encode item")))
        .collect()
}

pub fn snapshot(view: &SessionView) -> Snapshot {
    let mut value = serde_json::to_value(view).expect("encode view");
    let map = value.as_object_mut().expect("view is an object");
    map.remove("items");
    Snapshot {
        status: canon(&value),
        item_count: view.items.len(),
        items: item_canons(view).iter().map(|c| hash(c)).collect(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePage {
    pub revisions: u32,
    pub first_new_seq: Option<i64>,
    pub applied_seq: i64,
    pub reset: bool,
    pub closed_turn: Option<LastTurn>,
}

#[derive(Serialize)]
pub struct MergeReplay {
    pub view: Snapshot,
    pub pages: Vec<MergePage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Replay {
    pub events: usize,
    pub checkpoints: Vec<Checkpoint>,
    pub fold: Snapshot,
    /// The final fold's items as canonical text, to show what differs.
    pub fold_items: Vec<String>,
    pub restored: Snapshot,
    pub restored_live: Snapshot,
    pub settled_none: Snapshot,
    pub cancelled: Snapshot,
    pub resolved_permission: Option<Snapshot>,
    pub resolved_question: Option<Snapshot>,
    pub merge: MergeReplay,
    pub closed_turn: Option<LastTurn>,
}

/// Every event up to 500, else about 32 evenly spaced; the TS runner uses the same rule.
pub fn checkpoint_stride(events: usize) -> usize {
    if events <= 500 {
        1
    } else {
        events.div_ceil(32)
    }
}

/// The webview's `foldLog`: a fresh `starting` view, each row reduced, the row's time stamped on the last user item.
pub fn fold_log(
    entries: &[SessionLogEntry],
    mut on_step: impl FnMut(usize, &SessionView),
) -> SessionView {
    let mut view = SessionView::new(SessionState::Starting);
    for (i, entry) in entries.iter().enumerate() {
        reduce_event(&mut view, &entry.event, Some(entry.seq));
        if matches!(entry.event, SessionEvent::UserMessage { .. }) {
            if let Some(ItemKind::User {
                created_at: at @ None,
                ..
            }) = view.items.last_mut().map(|it| &mut it.kind)
            {
                *at = Some(entry.created_at);
            }
        }
        on_step(i, &view);
    }
    view
}

pub fn replay(entries: &[SessionLogEntry]) -> Replay {
    let stride = checkpoint_stride(entries.len());
    let mut checkpoints = Vec::new();
    let fold = fold_log(entries, |i, view| {
        if (i + 1) % stride == 0 || i + 1 == entries.len() {
            checkpoints.push(Checkpoint {
                index: i,
                view: snapshot(view),
            });
        }
    });

    let open: HashSet<String> = fold
        .pending_permission
        .iter()
        .map(|p| p.request_id.clone())
        .chain(
            fold.pending_question
                .iter()
                .chain(&fold.queued_questions)
                .map(|q| q.request_id.clone()),
        )
        .collect();
    let mut live = fold.clone();
    settle_unwaited_asks(&mut live, &open);
    let mut none = fold.clone();
    settle_unwaited_asks(&mut none, &HashSet::new());
    let mut cancelled = fold.clone();
    cancel_pending_prompts(&mut cancelled);
    let resolved_permission = fold.pending_permission.as_ref().map(|p| {
        let mut v = fold.clone();
        resolve_permission(&mut v, &p.request_id, PermissionDecision::Allow);
        snapshot(&v)
    });
    let resolved_question = fold.pending_question.as_ref().map(|q| {
        let mut v = fold.clone();
        resolve_question(&mut v, &q.request_id);
        snapshot(&v)
    });

    let mut merged = SessionView::new(SessionState::Starting);
    let mut applied = -1;
    let mut pages = Vec::new();
    for page in entries.chunks(MERGE_PAGE) {
        let m = merge_entries(merged, applied, page);
        merged = m.view;
        applied = m.applied_seq;
        pages.push(MergePage {
            revisions: m.revisions,
            first_new_seq: m.first_new_seq,
            applied_seq: m.applied_seq,
            reset: m.reset,
            closed_turn: m.closed_turn,
        });
        if m.reset {
            break;
        }
    }

    Replay {
        events: entries.len(),
        checkpoints,
        restored: snapshot(&restored_view(fold.clone(), false)),
        restored_live: snapshot(&restored_view(live, true)),
        settled_none: snapshot(&none),
        cancelled: snapshot(&cancelled),
        resolved_permission,
        resolved_question,
        closed_turn: closed_turn_in(entries),
        merge: MergeReplay {
            view: snapshot(&merged),
            pages,
        },
        fold_items: item_canons(&fold),
        fold: snapshot(&fold),
    }
}
