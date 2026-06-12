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
    let db = state.db.clone();
    let id_owned = id.to_string();
    tauri::async_runtime::spawn_blocking(move || db.get_workstream(&id_owned))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)?
        .ok_or_else(|| item_not_found(id))
}

/// Live git + thoughts-link status for one item (blocking work).
fn item_view(state: &AppState, item: Workstream, link: Option<LinkStatus>) -> WorkstreamView {
    let _ = state;
    let git = worktree::status(Path::new(&item.worktree_path), &item.base_branch, link);
    WorkstreamView { item, git }
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
pub async fn list_workstreams(
    state: State<'_, AppState>,
    project_path: String,
    include_archived: bool,
) -> CmdResult<Vec<WorkstreamView>> {
    let db = state.db.clone();
    let items = tauri::async_runtime::spawn_blocking(move || {
        db.list_workstreams(&project_path, include_archived)
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
                WorkstreamView { item, git }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::internal(format!("status task: {e}")))?;
    Ok(views)
}

/// Active workstreams across every project — feeds the sidebar tree.
#[tauri::command]
#[specta::specta]
pub async fn list_all_workstreams(state: State<'_, AppState>) -> CmdResult<Vec<WorkstreamView>> {
    let db = state.db.clone();
    let items = tauri::async_runtime::spawn_blocking(move || db.list_active_workstreams())
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)?;

    let views = tauri::async_runtime::spawn_blocking(move || {
        items
            .into_iter()
            .map(|item| {
                let git = worktree::status(Path::new(&item.worktree_path), &item.base_branch, None);
                WorkstreamView { item, git }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::internal(format!("status task: {e}")))?;
    Ok(views)
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

    let db = state.db.clone();
    let new_item = NewWorkstream {
        project_path,
        title,
        slug: created.slug.clone(),
        branch: created.branch.clone(),
        worktree_path: created.path.clone(),
        base_branch: created.base_branch.clone(),
    };
    let item = tauri::async_runtime::spawn_blocking(move || db.insert_workstream(new_item))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)?;

    Ok(item_view(&state, item, Some(link)))
}

#[tauri::command]
#[specta::specta]
pub async fn get_workstream(state: State<'_, AppState>, id: String) -> CmdResult<WorkstreamView> {
    let item = load_item(&state, &id).await?;
    let effective = effective_for(&state, &item.project_path).await?;
    let link = thoughts_links::check_thoughts(Path::new(&item.worktree_path), &effective);
    Ok(item_view(&state, item, Some(link)))
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workstream(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> CmdResult<()> {
    load_item(&state, &id).await?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.update_workstream_title(&id, title.trim()))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn set_workstream_stage(
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
    tauri::async_runtime::spawn_blocking(move || db.update_workstream_stage(&id, &stage))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
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
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.attach_workstream_doc(&id, &doc_path, &role)?;
        db.workstream_docs(&id)
    })
    .await
    .map_err(|e| AppError::internal(format!("db task: {e}")))?
    .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_doc(
    state: State<'_, AppState>,
    id: String,
    doc_path: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    load_item(&state, &id).await?;
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.detach_workstream_doc(&id, &doc_path)?;
        db.workstream_docs(&id)
    })
    .await
    .map_err(|e| AppError::internal(format!("db task: {e}")))?
    .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workstream_docs(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<WorkstreamDoc>> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.workstream_docs(&id))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
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
        let status = worktree::status(worktree_path, &item.base_branch, None);
        if status.dirty + status.untracked > 0 {
            return Err(AppError::from(CoreError::WorktreeDirty(
                status.dirty + status.untracked,
            )));
        }
        if status.ahead > 0 {
            return Err(AppError::from(CoreError::WorktreeUnpushed {
                branch: item.branch.clone(),
                count: status.ahead,
            }));
        }
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

    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || db.archive_workstream(&id))
        .await
        .map_err(|e| AppError::internal(format!("db task: {e}")))?
        .map_err(AppError::from)
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
        remedy: Some("The worktree may have been deleted; archive this workstream.".into()),
    })
}
