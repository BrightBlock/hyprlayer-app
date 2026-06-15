use std::path::{Path, PathBuf};

use hyprlayer_core::gitops::FileDiff;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::store::{NewWorkstream, Workstream, WorkstreamDoc};
use hyprlayer_core::worktree::thoughts_links::{self, LinkStatus};
use hyprlayer_core::worktree::{self, CreateSpec, RemovePolicy, WorktreeStatus};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;

use super::{blocking, db_task, effective_for};
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamView {
    pub item: Workstream,
    pub git: WorktreeStatus,
}

fn item_not_found(id: &str) -> AppError {
    AppError {
        code: "workstream_not_found".into(),
        message: format!("No workstream with id {id}"),
        remedy: None,
    }
}

async fn load_item(state: &AppState, id: &str) -> CmdResult<Workstream> {
    let id_owned = id.to_string();
    db_task(state.db.clone(), move |db| db.get_workstream(&id_owned))
        .await?
        .ok_or_else(|| item_not_found(id))
}

/// Live git + thoughts-link status for one item (blocking work).
fn item_view(item: Workstream, link: Option<LinkStatus>) -> WorkstreamView {
    let git = worktree::status(Path::new(&item.worktree_path), &item.base_branch, link);
    WorkstreamView { item, git }
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstreams(
    state: State<'_, AppState>,
    project_path: String,
    include_archived: bool,
) -> CmdResult<Vec<WorkstreamView>> {
    let items = db_task(state.db.clone(), move |db| {
        db.list_workstreams(&project_path, include_archived)
    })
    .await?;

    // Status polling is filesystem-bound; do the whole batch off-thread.
    let effective = if let Some(first) = items.first() {
        Some(effective_for(&state, &first.project_path).await?)
    } else {
        None
    };
    blocking("status", move || {
        items
            .into_iter()
            .map(|item| {
                let link = effective
                    .as_ref()
                    .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff));
                item_view(item, link)
            })
            .collect::<Vec<_>>()
    })
    .await
}

/// Active workstreams across every project — feeds the sidebar tree.
#[tauri::command]
#[specta::specta]
pub async fn list_all_workstreams(state: State<'_, AppState>) -> CmdResult<Vec<WorkstreamView>> {
    let items = db_task(state.db.clone(), |db| db.list_active_workstreams()).await?;
    blocking("status", move || {
        items
            .into_iter()
            .map(|item| item_view(item, None))
            .collect::<Vec<_>>()
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn create_workstream(
    state: State<'_, AppState>,
    project_path: String,
    title: String,
) -> CmdResult<WorkstreamView> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(AppError {
            code: "invalid_title".into(),
            message: "Workstreams need a title.".into(),
            remedy: None,
        });
    }

    let effective = effective_for(&state, &project_path).await?;
    let mapped = effective
        .mapped_name
        .clone()
        .ok_or_else(|| AppError::project_not_found(&project_path))?;

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

    // Provision hyprlayer context inside the worktree, CLI-first: `thoughts
    // init --yes --directory <mapped>` creates the symlinks and maps the
    // worktree to the SAME repo name, with the CLI owning its config write.
    // Manual provisioning (which has the Windows junction fallback) is the
    // fallback. Failures roll the worktree back so we never leave
    // half-provisioned state behind.
    let worktree_path = PathBuf::from(&created.path);
    let cli_init = HyprCli::new(&state.binaries)
        .thoughts_init(&worktree_path, &mapped, effective.profile_name.as_deref())
        .await;
    let provision = match cli_init {
        Ok(_) => Ok(thoughts_links::check_thoughts(&worktree_path, &effective)),
        Err(init_err) => {
            log::warn!("thoughts init in worktree failed ({init_err}); provisioning manually");
            (|| -> Result<LinkStatus, CoreError> {
                let link = thoughts_links::provision_thoughts(&worktree_path, &effective)?;
                thoughts_links::add_worktree_mapping(
                    state.config.path(),
                    &worktree_path,
                    &mapped,
                    effective.profile_name.as_deref(),
                )?;
                Ok(link)
            })()
        }
    };
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

    let new_item = NewWorkstream {
        project_path,
        title,
        slug: created.slug.clone(),
        branch: created.branch.clone(),
        worktree_path: created.path.clone(),
        base_branch: created.base_branch.clone(),
    };
    let item = db_task(state.db.clone(), move |db| db.insert_workstream(new_item)).await?;

    Ok(item_view(item, Some(link)))
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workstream(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> CmdResult<()> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.update_workstream_title(&id, title.trim())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_workstream_stage(
    state: State<'_, AppState>,
    id: String,
    stage: String,
) -> CmdResult<()> {
    if hyprlayer_core::pipeline::Stage::try_parse(&stage).is_none() {
        return Err(AppError {
            code: "invalid_stage".into(),
            message: format!("Unknown stage: {stage}"),
            remedy: None,
        });
    }
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.update_workstream_stage(&id, &stage)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn attach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
    role: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.attach_workstream_doc(&id, &doc_path, &role)?;
        db.workstream_docs(&id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| {
        db.detach_workstream_doc(&id, &doc_path)?;
        db.workstream_docs(&id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_docs(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    db_task(state.db.clone(), move |db| db.workstream_docs(&id)).await
}

/// Archive an item. `discard: false` is the safe path — it fails with
/// `worktree_dirty` / `worktree_unpushed` when work would be lost, and the
/// UI re-asks with `discard: true`.
#[tauri::command]
#[specta::specta]
pub async fn archive_workstream(
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
    let worktree_path = Path::new(&item.worktree_path);

    // Safe gate FIRST — `thoughts uninit` tears down the worktree's links,
    // so it must not run before we know the removal will be allowed.
    if policy == RemovePolicy::Safe && worktree_path.exists() {
        worktree::ensure_removable(worktree_path, &item.base_branch, &item.branch)
            .map_err(AppError::from)?;
    }

    // CLI-first teardown: uninit removes the thoughts/ links and the
    // worktree's repoMappings entry. Direct mapping edit is the fallback.
    let mut unmapped = false;
    if worktree_path.exists() {
        match HyprCli::new(&state.binaries)
            .thoughts_uninit(worktree_path)
            .await
        {
            Ok(_) => unmapped = true,
            Err(e) => log::warn!("thoughts uninit in worktree failed: {e}"),
        }
    }

    state
        .worktrees
        .remove(
            &git,
            Path::new(&item.project_path),
            worktree_path,
            &item.branch,
            &item.base_branch,
            policy,
        )
        .await
        .map_err(AppError::from)?;

    if !unmapped {
        if let Err(e) = thoughts_links::remove_worktree_mapping(state.config.path(), worktree_path)
        {
            log::warn!("could not remove worktree mapping: {e}");
        }
    }
    state.config.invalidate().await;

    db_task(state.db.clone(), move |db| db.archive_workstream(&id)).await
}

/// Re-create broken thoughts links (status shows `broken` after e.g. a moved
/// thoughts repo).
#[tauri::command]
#[specta::specta]
pub async fn reprovision_workstream(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorkstreamView> {
    let item = load_item(&state, &id).await?;
    let effective = effective_for(&state, &item.project_path).await?;
    let link = thoughts_links::provision_thoughts(Path::new(&item.worktree_path), &effective)
        .map_err(AppError::from)?;
    Ok(item_view(item, Some(link)))
}

/// Working-tree diff vs HEAD (staged + unstaged), thoughts/ excluded.
#[tauri::command]
#[specta::specta]
pub async fn worktree_diff(state: State<'_, AppState>, id: String) -> CmdResult<Vec<FileDiff>> {
    let item = load_item(&state, &id).await?;
    blocking("diff", move || {
        hyprlayer_core::gitops::worktree_diff(Path::new(&item.worktree_path))
    })
    .await?
    .map_err(|e| AppError {
        code: "worktree_diff".into(),
        message: e.to_string(),
        remedy: Some("The worktree may have been deleted; archive this workstream.".into()),
    })
}
