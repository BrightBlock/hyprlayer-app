use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;

use hyprlayer_core::runtime::prompts as core_prompts;
use hyprlayer_core::settings::PromptSettings;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PromptConflictKind {
    Builtin,
    Custom,
}

impl From<core_prompts::PromptConflictKind> for PromptConflictKind {
    fn from(kind: core_prompts::PromptConflictKind) -> Self {
        match kind {
            core_prompts::PromptConflictKind::Builtin => Self::Builtin,
            core_prompts::PromptConflictKind::Custom => Self::Custom,
        }
    }
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportCandidate {
    pub key: String,
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub chord_taken: bool,
    pub unknown_projects: Vec<String>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportConflict {
    pub key: String,
    pub kind: PromptConflictKind,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportPreview {
    pub source_sha256: String,
    pub source_version: u32,
    pub source: Option<String>,
    pub prompts: Vec<PromptImportCandidate>,
    pub conflicts: Vec<PromptImportConflict>,
}

impl From<core_prompts::PromptImportPreview> for PromptImportPreview {
    fn from(preview: core_prompts::PromptImportPreview) -> Self {
        Self {
            source_sha256: preview.source_sha256,
            source_version: preview.source_version,
            source: preview.source,
            prompts: preview
                .prompts
                .into_iter()
                .map(|c| PromptImportCandidate {
                    key: c.key,
                    name: c.name,
                    body: c.body,
                    chord: c.chord,
                    chord_taken: c.chord_taken,
                    unknown_projects: c.unknown_projects,
                })
                .collect(),
            conflicts: preview
                .conflicts
                .into_iter()
                .map(|c| PromptImportConflict {
                    key: c.key,
                    kind: c.kind.into(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PromptImportMode {
    Merge,
    Replace,
}

#[derive(Debug, Clone, Copy, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PromptConflictAction {
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportDecision {
    pub path: String,
    pub expected_source_sha256: String,
    pub mode: PromptImportMode,
    pub conflict_actions: BTreeMap<String, PromptConflictAction>,
    pub taken_chords: Vec<String>,
}

impl From<PromptImportDecision> for core_prompts::PromptImportDecision {
    fn from(decision: PromptImportDecision) -> Self {
        Self {
            path: decision.path,
            expected_source_sha256: decision.expected_source_sha256,
            mode: match decision.mode {
                PromptImportMode::Merge => core_prompts::PromptImportMode::Merge,
                PromptImportMode::Replace => core_prompts::PromptImportMode::Replace,
            },
            conflict_actions: decision
                .conflict_actions
                .into_iter()
                .map(|(key, action)| {
                    (
                        key,
                        match action {
                            PromptConflictAction::KeepBoth => {
                                core_prompts::PromptConflictAction::KeepBoth
                            }
                            PromptConflictAction::Replace => {
                                core_prompts::PromptConflictAction::Replace
                            }
                            PromptConflictAction::Skip => core_prompts::PromptConflictAction::Skip,
                        },
                    )
                })
                .collect(),
            taken_chords: decision.taken_chords,
        }
    }
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportResult {
    pub settings: PromptSettings,
    pub added: u32,
    pub replaced: u32,
    pub renamed: u32,
    pub skipped: u32,
    pub first_id: Option<String>,
}

impl From<core_prompts::PromptImportResult> for PromptImportResult {
    fn from(result: core_prompts::PromptImportResult) -> Self {
        Self {
            settings: result.settings,
            added: result.added,
            replaced: result.replaced,
            renamed: result.renamed,
            skipped: result.skipped,
            first_id: result.first_id,
        }
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptExportEntry {
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptExportRequest {
    pub path: String,
    pub entries: Vec<PromptExportEntry>,
    pub include_chords: bool,
    pub include_overrides: bool,
    pub source: Option<String>,
}

impl From<PromptExportRequest> for core_prompts::PromptExportRequest {
    fn from(req: PromptExportRequest) -> Self {
        Self {
            path: req.path,
            selection: core_prompts::PromptExportSelection {
                entries: req
                    .entries
                    .into_iter()
                    .map(|e| core_prompts::PromptExportEntry {
                        name: e.name,
                        body: e.body,
                        chord: e.chord,
                        overrides: e.overrides,
                    })
                    .collect(),
                include_chords: req.include_chords,
                include_overrides: req.include_overrides,
                source: req.source,
            },
        }
    }
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptExportResult {
    pub path: String,
    pub prompt_count: u32,
}

impl From<core_prompts::PromptExportResult> for PromptExportResult {
    fn from(result: core_prompts::PromptExportResult) -> Self {
        Self {
            path: result.path,
            prompt_count: result.prompt_count,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn preview_prompt_import(
    state: State<'_, AppState>,
    path: String,
    taken_chords: Vec<String>,
) -> CmdResult<PromptImportPreview> {
    state
        .runtime
        .prompts()
        .preview_import(core_prompts::PromptImportRequest { path, taken_chords })
        .await
        .map(PromptImportPreview::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn apply_prompt_import(
    state: State<'_, AppState>,
    decision: PromptImportDecision,
) -> CmdResult<PromptImportResult> {
    state
        .runtime
        .prompts()
        .apply_import(decision.into())
        .await
        .map(PromptImportResult::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn export_prompts(
    state: State<'_, AppState>,
    request: PromptExportRequest,
) -> CmdResult<PromptExportResult> {
    state
        .runtime
        .prompts()
        .export(request.into())
        .await
        .map(PromptExportResult::from)
        .map_err(AppError::from)
}
