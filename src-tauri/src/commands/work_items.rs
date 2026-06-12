use std::path::{Path, PathBuf};

use hyprlayer_core::gitops::FileDiff;
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::{NewWorkItem, WorkItem, WorkItemDoc};
use hyprlayer_core::worktree::thoughts_links::{self, LinkStatus};
use hyprlayer_core::worktree::{self, CreateSpec, RemovePolicy, WorktreeStatus};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemView {
    pub item: WorkItem,
    pub git: WorktreeStatus,
}

fn item_not_found(id: &str) -> AppError {
    AppError {
        code: "work_item_not_found".into(),
        message: format!("No work item with id {id}"),
        remedy: None,
    }
}

async fn load_item(state: &AppState, id: &str) -> CmdResult<WorkItem> {
    let db = state.db.clone();
    let id_owned = id.to_string();
    tauri::async_runtime::spawn_blocking(move || db.get_work_item(&id_owned))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)?
        .ok_or_else(|| item_not_found(id))
}

/// Live git + thoughts-link status for one item (blocking work).
fn item_view(state: &AppState, item: WorkItem, link: Option<LinkStatus>) -> WorkItemView {
    let _ = state;
    let git = worktree::status(Path::new(&item.worktree_path), &item.base_branch, link);
    WorkItemView { item, git }
}

async fn effective_for(
    state: &AppState,
    project_path: &str,
) -> CmdResult<hyprlayer_core::config::EffectiveConfig> {
    let loaded = state.config.load().await.map_err(AppError::from)?;
    let thoughts = loaded.config.thoughts.as_ref().ok_or_else(|| AppError {
        code: "config_missing".into(),
        message: "Thoughts are not configured yet.".into(),
        remedy: Some("Add the repository as a project first.".into()),
    })?;
    Ok(thoughts.effective_config_for(project_path))
}

#[tauri::command]
#[specta::specta]
pub async fn list_work_items(
    state: State<'_, AppState>,
    project_path: String,
    include_archived: bool,
) -> CmdResult<Vec<WorkItemView>> {
    let db = state.db.clone();
    let items = tauri::async_runtime::spawn_blocking(move || {
        db.list_work_items(&project_path, include_archived)
    })
    .await
    .map_err(|e| AppError::internal(format!("db task: {e}")))?
    .map_err(AppError::from)?;

    // Status polling is filesystem-bound; do the whole batch off-thread.
    let state_ref: &AppState = &state;
    let effective = if let Some(first) = items.first() {
        Some(effective_for(state_ref, &first.project_path).await?)
    } else {
        None
    };
    let views = tauri::async_runtime::spawn_blocking(move || {
        items
            .into_iter()
            .map(|item| {
                let link = effective
                    .as_ref()
                    .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff));
                let git = worktree::status(Path::new(&item.worktree_path), &item.base_branch, link);
                WorkItemView { item, git }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::internal(format!("status task: {e}")))?;
    Ok(views)
}

#[tauri::command]
#[specta::specta]
pub async fn create_work_item(
    state: State<'_, AppState>,
    project_path: String,
    title: String,
) -> CmdResult<WorkItemView> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(AppError {
            code: "invalid_title".into(),
            message: "Work items need a title.".into(),
            remedy: None,
        });
    }

    let effective = effective_for(&state, &project_path).await?;
    let mapped = effective.mapped_name.clone().ok_or_else(|| AppError {
        code: "project_not_found".into(),
        message: format!("No hyprlayer project is mapped at {project_path}"),
        remedy: Some("Add the repository as a project first.".into()),
    })?;

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let created = state
        .worktrees
        .create(
            &git,
            &CreateSpec {
                project_repo: PathBuf::from(&project_path),
                mapped_name: mapped.clone(),
                title: title.clone(),
                base_override: None,
            },
        )
        .await
        .map_err(AppError::from)?;

    // Provision hyprlayer context inside the worktree. Failures here roll the
    // worktree back so we never leave half-provisioned state behind.
    let worktree_path = PathBuf::from(&created.path);
    let provision = (|| -> Result<LinkStatus, CoreError> {
        let link = thoughts_links::provision_thoughts(&worktree_path, &effective)?;
        thoughts_links::add_worktree_mapping(
            state.config.path(),
            &worktree_path,
            &mapped,
            effective.profile_name.as_deref(),
        )?;
        Ok(link)
    })();
    let link = match provision {
        Ok(link) => link,
        Err(e) => {
            let _ = state
                .worktrees
                .remove(
                    &git,
                    Path::new(&project_path),
                    &worktree_path,
                    &created.branch,
                    &created.base_branch,
                    RemovePolicy::ForceDiscard,
                )
                .await;
            return Err(AppError::from(e));
        }
    };
    state.config.invalidate().await;

    let db = state.db.clone();
    let new_item = NewWorkItem {
        project_path,
        title,
        slug: created.slug.clone(),
        branch: created.branch.clone(),
        worktree_path: created.path.clone(),
        base_branch: created.base_branch.clone(),
    };
    let item = tauri::async_runtime::spawn_blocking(move || db.insert_work_item(new_item))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)?;

    Ok(item_view(&state, item, Some(link)))
}

#[tauri::command]
#[specta::specta]
pub async fn get_work_item(state: State<'_, AppState>, id: String) -> CmdResult<WorkItemView> {
    let item = load_item(&state, &id).await?;
    let effective = effective_for(&state, &item.project_path).await?;
    let link = thoughts_links::check_thoughts(Path::new(&item.worktree_path), &effective);
    Ok(item_view(&state, item, Some(link)))
}

#[tauri::command]
#[specta::specta]
pub async fn rename_work_item(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> CmdResult<()> {
    load_item(&state, &id).await?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.update_work_item_title(&id, title.trim()))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_work_item_stage(
    state: State<'_, AppState>,
    id: String,
    stage: String,
) -> CmdResult<()> {
    const STAGES: [&str; 6] = [
        "general",
        "research",
        "plan",
        "implement",
        "validate",
        "ship",
    ];
    if !STAGES.contains(&stage.as_str()) {
        return Err(AppError {
            code: "invalid_stage".into(),
            message: format!("Unknown stage: {stage}"),
            remedy: None,
        });
    }
    load_item(&state, &id).await?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.update_work_item_stage(&id, &stage))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn attach_work_item_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
    role: String,
) -> CmdResult<Vec<WorkItemDoc>> {
    load_item(&state, &id).await?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.attach_work_item_doc(&id, &doc_path, &role)?;
        db.work_item_docs(&id)
    })
    .await
    .map_err(|e| AppError::internal(format!("db task: {e}")))?
    .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn detach_work_item_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
) -> CmdResult<Vec<WorkItemDoc>> {
    load_item(&state, &id).await?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.detach_work_item_doc(&id, &doc_path)?;
        db.work_item_docs(&id)
    })
    .await
    .map_err(|e| AppError::internal(format!("db task: {e}")))?
    .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_work_item_docs(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<WorkItemDoc>> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.work_item_docs(&id))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
}

/// Archive an item. `discard: false` is the safe path — it fails with
/// `worktree_dirty` / `worktree_unpushed` when work would be lost, and the
/// UI re-asks with `discard: true`.
#[tauri::command]
#[specta::specta]
pub async fn archive_work_item(
    state: State<'_, AppState>,
    id: String,
    discard: bool,
) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let policy = if discard {
        RemovePolicy::ForceDiscard
    } else {
        RemovePolicy::Safe
    };

    state
        .worktrees
        .remove(
            &git,
            Path::new(&item.project_path),
            Path::new(&item.worktree_path),
            &item.branch,
            &item.base_branch,
            policy,
        )
        .await
        .map_err(AppError::from)?;

    // Unmap after the worktree is gone; tolerate a missing entry.
    if let Err(e) =
        thoughts_links::remove_worktree_mapping(state.config.path(), Path::new(&item.worktree_path))
    {
        log::warn!("could not remove worktree mapping: {e}");
    }
    state.config.invalidate().await;

    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.archive_work_item(&id))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
}

/// Re-create broken thoughts links (status shows `broken` after e.g. a moved
/// thoughts repo).
#[tauri::command]
#[specta::specta]
pub async fn reprovision_work_item(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorkItemView> {
    let item = load_item(&state, &id).await?;
    let effective = effective_for(&state, &item.project_path).await?;
    let link = thoughts_links::provision_thoughts(Path::new(&item.worktree_path), &effective)
        .map_err(AppError::from)?;
    Ok(item_view(&state, item, Some(link)))
}

/// Working-tree diff vs HEAD (staged + unstaged), thoughts/ excluded.
#[tauri::command]
#[specta::specta]
pub async fn worktree_diff(state: State<'_, AppState>, id: String) -> CmdResult<Vec<FileDiff>> {
    let item = load_item(&state, &id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        hyprlayer_core::gitops::worktree_diff(Path::new(&item.worktree_path))
    })
    .await
    .map_err(|e| AppError::internal(format!("diff task: {e}")))?
    .map_err(|e| AppError {
        code: "worktree_diff".into(),
        message: e.to_string(),
        remedy: Some("The worktree may have been deleted; archive this work item.".into()),
    })
}
