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
