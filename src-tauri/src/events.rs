use serde::{Deserialize, Serialize};

/// The hyprlayer CLI config file changed on disk (the CLI rewrote it, or the
/// user edited it). Frontend should refetch bootstrap/project queries.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ConfigChanged;

/// Binary statuses were re-probed (override set, redetect requested).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct BinariesChanged;
