use std::path::{Path, PathBuf};

use hyprlayer_core::docs::frontmatter::{FmPatch, FmValue};
use hyprlayer_core::docs::templates::{new_doc, DocTemplate};
use hyprlayer_core::gitops::ThoughtsRepoStatus;
use hyprlayer_core::index::{thoughts_roots, DocMeta, QueryResult, ThoughtQuery, ThoughtsRoot};
use serde::Deserialize;
use serde::Serialize;
use tauri::State;
use tauri_specta::Event;

use super::{blocking, effective_for};
use crate::error::{AppError, CmdResult};
use crate::events::ThoughtsIndexUpdated;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DocContent {
    pub meta: DocMeta,
    pub content: String,
    pub mtime_ms: f64,
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
    /// Repo path of the project the doc belongs to (None for global scope).
    pub project_repo_path: Option<String>,
    pub scope: String,
    pub doc_type: String,
    pub title: String,
    /// Today's date from the user's locale, YYYY-MM-DD.
    pub date: String,
    pub ticket: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn doc_conflict() -> AppError {
    AppError {
        code: "doc_conflict".into(),
        message: "The document changed on disk since you opened it.".into(),
        remedy: Some(
            "Reload the document to pick up the external changes, then re-apply yours.".into(),
        ),
    }
}

fn not_indexed(id: &str) -> AppError {
    AppError {
        code: "doc_not_found".into(),
        message: format!("Document is not in the thoughts index: {id}"),
        remedy: Some("It may have been deleted or moved; refresh the library.".into()),
    }
}

fn file_mtime_ms(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Optimistic-concurrency guard for doc writes: fail if the file changed on disk
/// since the caller read it (mtime drifted more than rounding noise). Shared by
/// every mutating doc command so the conflict policy lives in one place.
fn ensure_not_stale(abs: &Path, base_mtime_ms: f64) -> CmdResult<()> {
    if (file_mtime_ms(abs) - base_mtime_ms).abs() > 0.5 {
        return Err(doc_conflict());
    }
    Ok(())
}

/// Atomic write + immediate re-index + change event (no waiting on the
/// filesystem watcher debounce).
async fn write_and_reindex(
    app: &tauri::AppHandle,
    state: &AppState,
    abs: PathBuf,
    root: ThoughtsRoot,
    content: String,
) -> CmdResult<DocMeta> {
    let index = state.index.clone();
    let id =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, std::io::Error> {
            let tmp = abs.with_extension("md.tmp");
            std::fs::write(&tmp, &content)?;
            std::fs::rename(&tmp, &abs)?;
            Ok(index.upsert_path(&root, &abs))
        })
        .await
        .map_err(|e| AppError::internal(format!("write task: {e}")))?
        .map_err(|e| AppError::internal(format!("write doc: {e}")))?;

    let Some(id) = id else {
        return Err(AppError::internal("document was written but not indexable"));
    };
    let meta = state.index.get(&id).ok_or_else(|| not_indexed(&id))?;
    let _ = ThoughtsIndexUpdated {
        kind: "updated".into(),
        ids: vec![id],
    }
    .emit(app);
    Ok(meta)
}

/// The root a doc id (absolute path) belongs to, per current config.
async fn root_for_doc(state: &AppState, abs: &Path) -> CmdResult<ThoughtsRoot> {
    let loaded = state.config.load().await.map_err(AppError::from)?;
    thoughts_roots(&loaded.config)
        .into_iter()
        .find(|r| abs.starts_with(&r.root))
        .ok_or_else(|| AppError {
            code: "doc_outside_roots".into(),
            message: format!(
                "{} is not inside any configured thoughts root",
                abs.display()
            ),
            remedy: None,
        })
}

#[tauri::command]
#[specta::specta]
pub async fn query_docs(state: State<'_, AppState>, query: ThoughtQuery) -> CmdResult<QueryResult> {
    let index = state.index.clone();
    blocking("query", move || index.query(&query)).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_doc(state: State<'_, AppState>, id: String) -> CmdResult<DocContent> {
    // Indexed library docs: rich metadata straight from the in-memory index.
    if let Some(resolved) = state.index.resolve_id(&id) {
        if let Some(meta) = state.index.get(&resolved) {
            let path = PathBuf::from(&resolved);
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| AppError::internal(format!("read doc: {e}")))?;
            return Ok(DocContent {
                meta,
                content,
                mtime_ms: file_mtime_ms(&path),
            });
        }
    }

    // Fallback: an agent-created doc that lives outside every configured
    // thoughts root — e.g. a plan Claude wrote under `~/.claude/plans/`. These
    // surface in a workstream's Changes panel but are never indexed, so render
    // them read-only straight from disk with a best-effort meta instead of
    // failing. Gated to `.md` so this can't be turned into an arbitrary-file
    // reader; a missing file falls through to the "deleted or moved" message.
    let abs = PathBuf::from(&id);
    if !abs
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        return Err(not_indexed(&id));
    }
    let content = tokio::fs::read_to_string(&abs)
        .await
        .map_err(|_| not_indexed(&id))?;
    Ok(DocContent {
        meta: hyprlayer_core::index::unindexed_meta(&abs, &content),
        content,
        mtime_ms: file_mtime_ms(&abs),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn save_doc(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    content: String,
    base_mtime_ms: f64,
) -> CmdResult<DocMeta> {
    let id = state
        .index
        .resolve_id(&id)
        .ok_or_else(|| not_indexed(&id))?;
    let abs = PathBuf::from(&id);
    ensure_not_stale(&abs, base_mtime_ms)?;
    let root = root_for_doc(&state, &abs).await?;
    write_and_reindex(&app, &state, abs, root, content).await
}

#[tauri::command]
#[specta::specta]
pub async fn patch_doc_frontmatter(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    patch: FmPatchDto,
    base_mtime_ms: f64,
) -> CmdResult<DocMeta> {
    let id = state
        .index
        .resolve_id(&id)
        .ok_or_else(|| not_indexed(&id))?;
    let abs = PathBuf::from(&id);
    ensure_not_stale(&abs, base_mtime_ms)?;
    let original = tokio::fs::read_to_string(&abs)
        .await
        .map_err(|e| AppError::internal(format!("read doc: {e}")))?;

    let patched = hyprlayer_core::docs::frontmatter::patch_frontmatter(&original, &patch.into())
        .map_err(|e| AppError {
            code: "frontmatter_patch".into(),
            message: format!("could not patch frontmatter: {e}"),
            remedy: Some("Edit the frontmatter directly in the editor instead.".into()),
        })?;

    let root = root_for_doc(&state, &abs).await?;
    write_and_reindex(&app, &state, abs, root, patched).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_doc(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CreateDocInput,
) -> CmdResult<DocMeta> {
    let loaded = state.config.load().await.map_err(AppError::from)?;
    let thoughts = loaded
        .config
        .thoughts
        .as_ref()
        .ok_or_else(AppError::thoughts_not_configured)?;

    let (effective, mapped_name) = match (&input.project_repo_path, input.scope.as_str()) {
        (_, "global") | (None, _) => (thoughts.effective_config_for(""), None),
        (Some(repo), _) => {
            let eff = thoughts.effective_config_for(repo);
            let mapped = eff.mapped_name.clone();
            if mapped.is_none() {
                return Err(AppError::project_not_found(repo));
            }
            (eff, mapped)
        }
    };

    let root = effective.backend.thoughts_root().ok_or_else(|| AppError {
        code: "backend_not_filesystem".into(),
        message: format!(
            "The {} backend has no local document tree to create files in.",
            effective.backend.kind().as_str()
        ),
        remedy: Some("Create the document in the backend's own app instead.".into()),
    })?;

    // Layout mirrors the CLI: repos/<mapped>/<user|shared>/<typedir>/ or
    // global/<user|shared>/<typedir>/.
    let (repos_dir, global_dir) = match &effective.backend {
        hyprlayer_core::config::BackendConfig::Git(g) => {
            (g.repos_dir.clone(), g.global_dir.clone())
        }
        hyprlayer_core::config::BackendConfig::Obsidian(o) => {
            (o.repos_dir.clone(), o.global_dir.clone())
        }
        _ => unreachable!("thoughts_root() returned Some for a flat-list backend"),
    };
    let area_dir = match input.scope.as_str() {
        "shared" | "global" => "shared".to_string(),
        _ => effective.user.clone(),
    };
    let type_dir = match input.doc_type.as_str() {
        "plan" => "plans",
        "research" => "research",
        "handoff" => "handoffs",
        "pr" => "prs",
        _ => "notes",
    };
    let dir = if input.scope == "global" {
        root.join(&global_dir).join(&area_dir).join(type_dir)
    } else {
        let mapped = mapped_name.as_deref().unwrap_or_default();
        root.join(&repos_dir)
            .join(mapped)
            .join(&area_dir)
            .join(type_dir)
    };

    let template = DocTemplate {
        title: input.title,
        doc_type: input.doc_type,
        date: input.date,
        ticket: input.ticket,
        project: mapped_name,
        scope: input.scope,
        author: effective.user,
        tags: input.tags,
    };
    let doc = new_doc(&template);

    let abs = dir.join(&doc.file_name);
    if abs.exists() {
        return Err(AppError {
            code: "doc_exists".into(),
            message: format!("{} already exists", abs.display()),
            remedy: Some("Pick a different title.".into()),
        });
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::internal(format!("create doc dir: {e}")))?;

    let root_info = root_for_doc(&state, &abs).await?;
    write_and_reindex(&app, &state, abs, root_info, doc.content).await
}

#[tauri::command]
#[specta::specta]
pub async fn reveal_doc(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    // Only reveal paths the index knows — the frontend can't point this at
    // arbitrary filesystem locations. resolve_id also maps a worktree-symlink
    // path back to the canonical indexed id before the index check.
    let id = state
        .index
        .resolve_id(&id)
        .ok_or_else(|| not_indexed(&id))?;
    let path = state.index.doc_path(&id).ok_or_else(|| not_indexed(&id))?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|e| AppError::internal(format!("reveal: {e}")))
}

#[tauri::command]
#[specta::specta]
pub async fn get_thoughts_git_status(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<ThoughtsRepoStatus> {
    let effective = effective_for(&state, &project_path).await?;

    match &effective.backend {
        hyprlayer_core::config::BackendConfig::Git(_) => {}
        other => {
            return Err(AppError {
                code: "backend_not_git".into(),
                message: format!(
                    "The {} backend does not sync through a git repository.",
                    other.kind().as_str()
                ),
                remedy: None,
            })
        }
    }
    let root = effective.backend.thoughts_root().ok_or_else(|| AppError {
        code: "backend_not_filesystem".into(),
        message: "No thoughts repository is configured.".into(),
        remedy: None,
    })?;

    blocking("status", move || {
        hyprlayer_core::gitops::thoughts_repo_status(&root)
    })
    .await?
    .map_err(AppError::from)
}
