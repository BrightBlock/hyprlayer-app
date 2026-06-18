use serde::{Deserialize, Serialize};

/// The hyprlayer CLI config file changed on disk (the CLI rewrote it, or the
/// user edited it). Frontend should refetch bootstrap/project queries.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ConfigChanged;

/// Binary statuses were re-probed (override set, redetect requested).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct BinariesChanged;

/// The thoughts index changed: docs were (re)indexed, removed, or the whole
/// tree was rescanned. `ids` are absolute-path doc ids (empty on rescans).
/// Open editors use this to detect external changes to their document.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsIndexUpdated {
    pub kind: String,
    pub ids: Vec<String>,
}

/// A workstream's PR status changed server-side — it merged, its checks
/// settled, or a PR appeared/disappeared for the branch. The backend PR watcher
/// (`crate::pr_watch`) diffs each poll and emits this only on a real change; the
/// frontend refetches that workstream's PR query so the merge/archive bar stays
/// in sync without the UI polling `gh` on a timer.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusChanged {
    pub workstream_id: String,
}

/// Coarse session state change, for board/sidebar badges (the fine-grained
/// transcript flows over the per-session Channel, not this global event).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionLifecycle {
    pub session_id: String,
    pub workstream_id: String,
    /// Matches the `sessions.state` column.
    pub state: String,
}
