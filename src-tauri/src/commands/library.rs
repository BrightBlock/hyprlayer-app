use hyprlayer_core::docs::frontmatter::{FmPatch, FmValue};
use hyprlayer_core::gitops::ThoughtsRepoStatus;
use hyprlayer_core::index::{DocMeta, QueryResult, ThoughtQuery};
use hyprlayer_core::runtime::library::{CreateDocumentRequest, Document};
use serde::Deserialize;
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DocContent {
    pub meta: DocMeta,
    pub content: String,
    pub mtime_ms: f64,
}

impl From<Document> for DocContent {
    fn from(doc: Document) -> Self {
        DocContent {
            meta: doc.meta,
            content: doc.content,
            mtime_ms: doc.mtime_ms,
        }
    }
}

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum FmValueDto {
    Str { value: String },
    List { values: Vec<String> },
    Null,
}

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FmSetEntry {
    pub key: String,
    pub value: FmValueDto,
}

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FmPatchDto {
    #[serde(default)]
    pub set: Vec<FmSetEntry>,
    #[serde(default)]
    pub remove: Vec<String>,
}

impl From<FmPatchDto> for FmPatch {
    fn from(dto: FmPatchDto) -> Self {
        FmPatch {
            set: dto
                .set
                .into_iter()
                .map(|e| {
                    let value = match e.value {
                        FmValueDto::Str { value } => FmValue::Str(value),
                        FmValueDto::List { values } => FmValue::List(values),
                        FmValueDto::Null => FmValue::Null,
                    };
                    (e.key, value)
                })
                .collect(),
            remove: dto.remove,
        }
    }
}

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocInput {
    pub project_repo_path: Option<String>,
    pub scope: String,
    pub doc_type: String,
    pub title: String,
    pub date: String,
    pub ticket: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl From<CreateDocInput> for CreateDocumentRequest {
    fn from(input: CreateDocInput) -> Self {
        CreateDocumentRequest {
            project_repo_path: input.project_repo_path,
            scope: input.scope,
            doc_type: input.doc_type,
            title: input.title,
            date: input.date,
            ticket: input.ticket,
            tags: input.tags,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn query_docs(state: State<'_, AppState>, query: ThoughtQuery) -> CmdResult<QueryResult> {
    state
        .runtime
        .library()
        .query(query)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_doc(state: State<'_, AppState>, id: String) -> CmdResult<DocContent> {
    state
        .runtime
        .library()
        .get(&id)
        .await
        .map(DocContent::from)
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn save_doc(
    state: State<'_, AppState>,
    id: String,
    content: String,
    base_mtime_ms: f64,
) -> CmdResult<DocMeta> {
    state
        .runtime
        .library()
        .save(&id, content, base_mtime_ms)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn patch_doc_frontmatter(
    state: State<'_, AppState>,
    id: String,
    patch: FmPatchDto,
    base_mtime_ms: f64,
) -> CmdResult<DocMeta> {
    state
        .runtime
        .library()
        .patch_frontmatter(&id, patch.into(), base_mtime_ms)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn create_doc(state: State<'_, AppState>, input: CreateDocInput) -> CmdResult<DocMeta> {
    state
        .runtime
        .library()
        .create(input.into())
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn reveal_doc(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let path = state.runtime.library().resolve_path(&id)?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|e| AppError::internal(format!("reveal: {e}")))
}

#[tauri::command]
#[specta::specta]
pub async fn doc_web_url(state: State<'_, AppState>, id: String) -> CmdResult<Option<String>> {
    let Ok(abs) = state.runtime.library().resolve_path(&id) else {
        return Ok(None);
    };
    let url =
        tauri::async_runtime::spawn_blocking(move || hyprlayer_core::gitops::file_web_url(&abs))
            .await
            .ok()
            .flatten();
    Ok(url)
}

#[tauri::command]
#[specta::specta]
pub async fn get_thoughts_git_status(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ThoughtsRepoStatus> {
    state
        .runtime
        .library()
        .thoughts_git_status(&project_path)
        .await
        .map_err(AppError::from)
}
