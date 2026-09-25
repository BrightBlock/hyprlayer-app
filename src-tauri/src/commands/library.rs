use hyprlayer_core::docs::frontmatter::{FmPatch, FmValue};
use hyprlayer_core::gitops::ThoughtsRepoStatus;
use hyprlayer_core::index::related::RelatedLink;
use hyprlayer_core::index::{DocMeta, QueryResult, ThoughtQuery};
use hyprlayer_core::runtime::library::{CreateDocumentRequest, Document, FrontmatterEdit};
use serde::Deserialize;
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DocContent {
    pub meta: DocMeta,
    pub content: String,
    pub mtime_ms: f64,
    pub body: String,
    pub frontmatter: Option<String>,
    pub fm_error: Option<String>,
    pub fm_keys: Vec<String>,
}

impl From<Document> for DocContent {
    fn from(doc: Document) -> Self {
        DocContent {
            meta: doc.meta,
            content: doc.content,
            mtime_ms: doc.mtime_ms,
            body: doc.body,
            frontmatter: doc.frontmatter,
            fm_error: doc.fm_error,
            fm_keys: doc.fm_keys,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FmSetEntry {
    pub key: String,
    pub value: FmValueDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum FrontmatterEditDto {
    Patch { patch: FmPatchDto },
    Raw { text: String },
}

impl From<FrontmatterEditDto> for FrontmatterEdit {
    fn from(dto: FrontmatterEditDto) -> Self {
        match dto {
            FrontmatterEditDto::Patch { patch } => FrontmatterEdit::Patch(patch.into()),
            FrontmatterEditDto::Raw { text } => FrontmatterEdit::Raw(text),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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
        .client()?
        .query_docs(query)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_doc(state: State<'_, AppState>, id: String) -> CmdResult<DocContent> {
    state
        .client()?
        .get_doc(id)
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
        .client()?
        .save_doc(id, content, base_mtime_ms)
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
        .client()?
        .patch_doc_frontmatter(id, patch.into(), base_mtime_ms)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn save_doc_parts(
    state: State<'_, AppState>,
    id: String,
    body: String,
    frontmatter: FrontmatterEditDto,
    base_mtime_ms: f64,
) -> CmdResult<DocMeta> {
    state
        .client()?
        .save_doc_parts(id, body, frontmatter.into(), base_mtime_ms)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn resolve_related_docs(
    state: State<'_, AppState>,
    id: String,
    values: Vec<String>,
) -> CmdResult<Vec<RelatedLink>> {
    state
        .client()?
        .resolve_related_docs(id, values)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn create_doc(state: State<'_, AppState>, input: CreateDocInput) -> CmdResult<DocMeta> {
    state
        .client()?
        .create_doc(input.into())
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn reveal_doc(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if let Some(err) = state.remote_unsupported("Reveal in file manager") {
        return Err(err);
    }
    let path = state
        .client()?
        .resolve_doc_path(id)
        .await
        .map_err(AppError::from)?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|e| AppError::internal(format!("reveal: {e}")))
}

#[tauri::command]
#[specta::specta]
pub async fn doc_web_url(state: State<'_, AppState>, id: String) -> CmdResult<Option<String>> {
    state
        .client()?
        .doc_web_url(id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn get_thoughts_git_status(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ThoughtsRepoStatus> {
    state
        .client()?
        .get_thoughts_git_status(project_path)
        .await
        .map_err(AppError::from)
}
