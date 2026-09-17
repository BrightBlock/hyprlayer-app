use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::State;

use hyprlayer_core::runtime::prompts as core_prompts;
use hyprlayer_core::settings::PromptSettings;
use hyprlayer_core::CoreError;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportCandidate {
    pub key: String,
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub chord_taken: bool,
    pub unknown_projects: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportConflict {
    pub key: String,
    pub kind: PromptConflictKind,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PromptImportMode {
    Merge,
    Replace,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PromptConflictAction {
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptImportDecision {
    pub source: String,
    pub expected_source_sha256: String,
    pub mode: PromptImportMode,
    pub conflict_actions: BTreeMap<String, PromptConflictAction>,
    pub taken_chords: Vec<String>,
}

impl From<PromptImportDecision> for core_prompts::PromptImportDecision {
    fn from(decision: PromptImportDecision) -> Self {
        Self {
            source: decision.source,
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

#[derive(Debug, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptExportEntry {
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptExportRequest {
    pub entries: Vec<PromptExportEntry>,
    pub include_chords: bool,
    pub include_overrides: bool,
    pub source: Option<String>,
}

impl From<PromptExportRequest> for core_prompts::PromptExportRequest {
    fn from(req: PromptExportRequest) -> Self {
        Self {
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

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptExportResult {
    pub content: String,
    pub prompt_count: u32,
}

impl From<core_prompts::PromptExportResult> for PromptExportResult {
    fn from(result: core_prompts::PromptExportResult) -> Self {
        Self {
            content: result.content,
            prompt_count: result.prompt_count,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn preview_prompt_import(
    state: State<'_, AppState>,
    source: String,
    taken_chords: Vec<String>,
) -> CmdResult<PromptImportPreview> {
    state
        .client()?
        .preview_prompt_import(source, taken_chords)
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
        .client()?
        .apply_prompt_import(decision.into())
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
        .client()?
        .export_prompts(request.into())
        .await
        .map(PromptExportResult::from)
        .map_err(AppError::from)
}

/// A client-side file write, in the same class as `reveal_doc`
/// (`commands/library.rs`) — it acts on the machine the user picked the path
/// on, which is always the client, so it needs no remote guard.
#[tauri::command]
#[specta::specta]
pub async fn write_prompt_export(path: String, contents: String) -> CmdResult<String> {
    if contents.len() as u64 > core_prompts::MAX_PROMPTS_FILE_BYTES {
        return Err(AppError::from(CoreError::PromptsFileTooLarge));
    }
    let ext = format!(".{}", core_prompts::PROMPTS_FILE_EXT);
    let final_path = if path.ends_with(&ext) {
        path
    } else if Path::new(&path).extension().is_some() {
        return Err(AppError::from(CoreError::PromptsFileInvalid {
            detail: format!(
                "Prompt exports must use the .{} extension.",
                core_prompts::PROMPTS_FILE_EXT
            ),
        }));
    } else {
        format!("{path}{ext}")
    };
    tokio::fs::write(&final_path, contents.as_bytes())
        .await
        .map_err(|e| AppError::internal(format!("write {final_path}: {e}")))?;
    Ok(final_path)
}

/// The read half of `write_prompt_export`: also client-side, also unguarded.
#[tauri::command]
#[specta::specta]
pub async fn read_prompt_import(path: String) -> CmdResult<String> {
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|e| AppError::internal(format!("read {path}: {e}")))?;
    if meta.len() > core_prompts::MAX_PROMPTS_FILE_BYTES {
        return Err(AppError::from(CoreError::PromptsFileTooLarge));
    }
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| AppError::internal(format!("read {path}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_prompt_export_appends_the_extension_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("share");
        let path = write_prompt_export(target.display().to_string(), "{}".into())
            .await
            .unwrap();

        assert!(path.ends_with(".hlprompts"), "{path}");
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "{}");
    }

    #[tokio::test]
    async fn write_prompt_export_refuses_a_different_extension() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("share.txt");
        let err = write_prompt_export(target.display().to_string(), "{}".into())
            .await
            .unwrap_err();

        assert_eq!(err.code, "prompts_file_invalid");
        assert!(!target.exists());
    }
}
