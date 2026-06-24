use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprlayer_core::gitops::{DirEntry, FileSides, WorktreeGitStatus};
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::pr::{self, MergeMethod, PrStatus, PrSummary};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::skills::SlashCommand;
use hyprlayer_core::store::{NewWorkstream, Workstream, WorkstreamDoc};
use hyprlayer_core::worktree::thoughts_links::{self, LinkStatus};
use hyprlayer_core::worktree::{self, archive_dir_for, CreateSpec, RemovePolicy, WorktreeStatus};
use hyprlayer_core::CoreError;
use serde::Serialize;
use tauri::State;
use tauri_specta::Event;

use super::sessions::kill_and_await_exit;
use super::{blocking, db_task, effective_for};
use crate::error::{AppError, CmdResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamView {
    pub item: Workstream,
    pub git: WorktreeStatus,
    /// Populated only by `create_workstream`: result of copying configured
    /// untracked files into the new worktree. `null` on every other path.
    pub seed: Option<hyprlayer_core::worktree::seed::SeedReport>,
}

/// Base-branch choices for the "new workstream" dialog: every local + remote
/// branch, plus the auto-detected default the picker pre-selects.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamBases {
    pub branches: Vec<String>,
    pub default_base: String,
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

/// The directory to run `gh` PR commands in for an item: its worktree when on
/// disk, else the main repo. Both share the same remote, so PR number/branch
/// resolution is identical; the fallback keeps PR ops working for a workstream
/// whose worktree was removed (e.g. just archived).
fn pr_dir(item: &Workstream) -> &Path {
    let worktree = Path::new(&item.worktree_path);
    if worktree.is_dir() {
        worktree
    } else {
        Path::new(&item.project_path)
    }
}

/// Live git + thoughts-link status for one item (blocking work).
fn item_view(item: Workstream, link: Option<LinkStatus>) -> WorkstreamView {
    let git = worktree::status(Path::new(&item.worktree_path), &item.base_branch, link);
    WorkstreamView {
        item,
        git,
        seed: None,
    }
}

/// Compute each workstream's git+link status concurrently on the blocking pool
/// (one task per item), preserving input order. Status is filesystem-bound
/// (two git2 tree walks per repo), so a serial sweep made sidebar latency scale
/// with the *number* of active workstreams; fanning out makes it the slowest
/// single repo instead. `link_for` runs inside each task (it's also blocking fs
/// work), so a shared piece of context (the effective config) is passed by Arc.
async fn status_batch<F>(items: Vec<Workstream>, link_for: F) -> CmdResult<Vec<WorkstreamView>>
where
    F: Fn(&Workstream) -> Option<LinkStatus> + Send + Sync + 'static,
{
    let link_for = Arc::new(link_for);
    let handles: Vec<_> = items
        .into_iter()
        .map(|item| {
            let link_for = link_for.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let link = link_for(&item);
                item_view(item, link)
            })
        })
        .collect();
    let mut views = Vec::with_capacity(handles.len());
    for handle in handles {
        views.push(
            handle
                .await
                .map_err(|e| AppError::internal(format!("status task: {e}")))?,
        );
    }
    Ok(views)
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

    // All items in one project share a thoughts config; resolve it once, then
    // each task checks its own worktree's links against it.
    let effective = if let Some(first) = items.first() {
        Some(effective_for(&state, &first.project_path).await?)
    } else {
        None
    };
    status_batch(items, move |item| {
        effective
            .as_ref()
            .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), eff))
    })
    .await
}

/// Active workstreams across every project — feeds the sidebar tree.
#[tauri::command]
#[specta::specta]
pub async fn list_all_workstreams(state: State<'_, AppState>) -> CmdResult<Vec<WorkstreamView>> {
    let items = db_task(state.db.clone(), |db| db.list_active_workstreams()).await?;
    status_batch(items, |_| None).await
}

/// Point the live worktree watcher at this workstream's worktree, so external
/// edits push a `WorktreeChanged` event and the Changes panel + git status
/// refresh without waiting for window focus (the way the agent and terminal
/// panes already stream live). Single-slot: switching workstreams replaces the
/// previous watcher; calling it again for the same id is a no-op. The frontend
/// calls this whenever the viewed workstream changes.
#[tauri::command]
#[specta::specta]
pub async fn watch_worktree(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workstream_id: String,
) -> CmdResult<()> {
    let mut guard = state.worktree_watch.lock().await;
    if guard.as_ref().is_some_and(|(id, _)| id == &workstream_id) {
        return Ok(()); // already watching this worktree
    }
    // Drop the previous watcher before resolving the new one (so a missing
    // worktree, e.g. just-archived, simply leaves nothing watched).
    *guard = None;

    let ws = load_item(&state, &workstream_id).await?;
    let path = PathBuf::from(&ws.worktree_path);
    if !path.is_dir() {
        return Ok(());
    }

    let app = app.clone();
    let wsid = workstream_id.clone();
    let handle = blocking("watch_worktree", move || {
        hyprlayer_core::index::watcher::watch_path(
            &path,
            Box::new(move || {
                let _ = crate::events::WorktreeChanged {
                    workstream_id: wsid.clone(),
                }
                .emit(&app);
            }),
        )
    })
    .await?
    .map_err(AppError::from)?;

    *guard = Some((workstream_id, handle));
    Ok(())
}

/// Branches the user can base a new workstream on, with the auto-detected
/// default. Best-effort: the default falls back to empty (the dialog then
/// pre-selects nothing and the backend re-detects on create) so a repo with no
/// resolvable main/master still lists its branches.
#[tauri::command]
#[specta::specta]
pub async fn list_workstream_bases(
    state: State<'_, AppState>,
    project_path: String,
) -> CmdResult<WorkstreamBases> {
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let repo = PathBuf::from(&project_path);
    let default_base = worktree::detect_base_branch(&git, &repo)
        .await
        .unwrap_or_default();
    let branches = blocking("list_branches", move || {
        hyprlayer_core::gitops::list_branches(&repo)
    })
    .await?
    .map_err(AppError::from)?;
    Ok(WorkstreamBases {
        branches,
        default_base,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn create_workstream(
    state: State<'_, AppState>,
    project_path: String,
    title: String,
    base: Option<String>,
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
    let branch_prefix = state.settings.get().await.git.effective_branch_prefix();
    // An empty/blank selection means "auto-detect" — same as omitting it.
    let base_override = base.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
    let created = state
        .worktrees
        .create(
            &git,
            &CreateSpec {
                project_repo: PathBuf::from(&project_path),
                mapped_name: mapped.clone(),
                title: title.clone(),
                base_override,
                branch_prefix,
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
                    true,
                )
                .await;
            return Err(AppError::from(e));
        }
    };
    state.config.invalidate().await;

    // Best-effort: copy configured untracked files (e.g. .env) into the new
    // worktree. NEVER fatal — a missing file must not undo a provisioned worktree.
    let seed = {
        let repo = PathBuf::from(&project_path);
        let wt = worktree_path.clone();
        blocking("seed_worktree", move || {
            let patterns = hyprlayer_core::repo_settings::read(&repo)
                .map(|s| s.worktree.copy)
                .unwrap_or_else(|e| {
                    log::warn!("read .hyprlayer/settings.toml: {e}");
                    Vec::new()
                });
            hyprlayer_core::worktree::seed::seed_worktree(&repo, &wt, &patterns)
        })
        .await
        .unwrap_or_default()
    };
    if !seed.errors.is_empty() || !seed.missing.is_empty() {
        log::warn!(
            "worktree seed: copied {}, missing {:?}, errors {:?}",
            seed.copied.len(),
            seed.missing,
            seed.errors
        );
    } else if !seed.copied.is_empty() {
        log::info!("worktree seed: copied {} file(s)", seed.copied.len());
    }

    let new_item = NewWorkstream {
        project_path,
        title,
        slug: created.slug.clone(),
        branch: created.branch.clone(),
        worktree_path: created.path.clone(),
        base_branch: created.base_branch.clone(),
    };
    let item = db_task(state.db.clone(), move |db| db.insert_workstream(new_item)).await?;

    let mut view = item_view(item, Some(link));
    view.seed = Some(seed);
    Ok(view)
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

/// Archive a workstream non-destructively: move the worktree to
/// `<project>/archived-contexts/<slug>/` (Conductor-style) and flip the
/// DB row to `archived`. Branch and every uncommitted file are preserved
/// for [`restore_workstream`]. Husks (worktree dir without `.git`) can't be
/// moved; they're pruned + cleaned, the row still flips to archived, and
/// restore for that row will fail with `archive_missing`.
#[tauri::command]
#[specta::specta]
pub async fn archive_workstream(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;

    // Tear down any live agent sessions for this workstream BEFORE moving the
    // worktree. Otherwise the claude process keeps running against a stale
    // cwd and its driver + forwarder task leak (they only exit on the
    // process's own `Exited` event, which archive never triggers). Kill each
    // and wait — bounded — for the forwarder to drop it from the live
    // registry.
    let sessions = {
        let id = id.clone();
        db_task(state.db.clone(), move |db| db.list_sessions(&id))
            .await
            .unwrap_or_default()
    };
    for session in &sessions {
        kill_and_await_exit(&state, &session.id).await;
    }

    // Same for any live terminals in this workstream: kill their shells so we
    // don't leak orphaned PTY processes against the about-to-move worktree.
    state.terminals.remove_for_workstream(&id).await;

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let project_path = Path::new(&item.project_path);
    let worktree_path = Path::new(&item.worktree_path);
    let archive_path = archive_dir_for(project_path, &item.slug);
    let live = worktree::is_live_worktree(worktree_path);

    // Uninit so the worktree's absolute-path symlinks + repoMappings don't
    // accumulate stale entries pointing at the archive location. Restore
    // re-provisions from scratch. Direct mapping removal is the fallback.
    let mut unmapped = false;
    if live {
        match HyprCli::new(&state.binaries)
            .thoughts_uninit(worktree_path)
            .await
        {
            Ok(_) => unmapped = true,
            Err(e) => log::warn!("thoughts uninit in worktree failed: {e}"),
        }
    }

    if live && state.settings.get().await.git.prune_ignored_on_archive {
        // worktree.copy patterns are preserved; read mirrors create_workstream.
        let keep = hyprlayer_core::repo_settings::read(project_path)
            .map(|s| s.worktree.copy)
            .unwrap_or_default();
        if let Err(e) = state.worktrees.clean_ignored(&git, worktree_path, &keep).await {
            log::warn!("prune ignored files before archive failed: {e}");
        }
    }

    let moved = state
        .worktrees
        .archive(&git, project_path, worktree_path, &archive_path)
        .await
        .map_err(AppError::from)?;
    if moved {
        // Per-clone exclude — written once per project, idempotent.
        if let Err(e) = worktree::ensure_archive_excluded(project_path) {
            log::warn!("could not write .git/info/exclude for archive: {e}");
        }
    }

    if !unmapped {
        if let Err(e) = thoughts_links::remove_worktree_mapping(state.config.path(), worktree_path)
        {
            log::warn!("could not remove worktree mapping: {e}");
        }
    }
    state.config.invalidate().await;

    db_task(state.db.clone(), move |db| db.archive_workstream(&id)).await
}

/// View shape for the archive history view. `archive_exists` flags rows
/// whose backup directory was deleted out-of-band (restore would fail; UI
/// surfaces "delete permanently" as the only action).
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedWorkstreamView {
    pub item: Workstream,
    /// `<project>/archived-contexts/<slug>` — the on-disk backup path.
    pub archive_path: String,
    pub archive_exists: bool,
}

/// All archived workstreams across every project, newest-first.
#[tauri::command]
#[specta::specta]
pub async fn list_archived_workstreams(
    state: State<'_, AppState>,
) -> CmdResult<Vec<ArchivedWorkstreamView>> {
    let items = db_task(state.db.clone(), |db| db.list_archived_workstreams()).await?;
    blocking("list_archived", move || {
        items
            .into_iter()
            .map(|item| {
                let archive_path = archive_dir_for(Path::new(&item.project_path), &item.slug);
                let archive_exists = archive_path.exists();
                ArchivedWorkstreamView {
                    archive_path: archive_path.display().to_string(),
                    archive_exists,
                    item,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
}

/// Move an archived workstream back to its original path and flip the row
/// to `active`. Fails with `restore_target_occupied` when something has
/// taken the original `worktree_path` since archive (e.g. a new workstream
/// reused the slug), and `archive_missing` if the backup directory was
/// deleted out-of-band.
#[tauri::command]
#[specta::specta]
pub async fn restore_workstream(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorkstreamView> {
    let item = load_item(&state, &id).await?;
    if item.status != "archived" {
        return Err(AppError {
            code: "not_archived".into(),
            message: format!("{} is not archived.", item.title),
            remedy: None,
        });
    }
    let project_path = Path::new(&item.project_path);
    let worktree_path = Path::new(&item.worktree_path);
    let archive_path = archive_dir_for(project_path, &item.slug);

    if !archive_path.exists() {
        return Err(AppError {
            code: "archive_missing".into(),
            message: format!(
                "The backup directory for this workstream is gone ({}).",
                archive_path.display()
            ),
            remedy: Some("Delete the archived row to remove it from the list.".into()),
        });
    }
    if worktree_path.exists() {
        return Err(AppError {
            code: "restore_target_occupied".into(),
            message: format!(
                "Another workstream took the original path ({}).",
                worktree_path.display()
            ),
            remedy: Some("Move or archive the workstream at that path, then try again.".into()),
        });
    }

    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    state
        .worktrees
        .restore(&git, project_path, &archive_path, worktree_path)
        .await
        .map_err(AppError::from)?;

    // Re-provision thoughts: links + repoMappings. Best-effort — restore is
    // still useful with broken links (the user sees the broken-link badge
    // and can hit Reprovision), but the common path should produce a fully
    // working worktree.
    let effective = effective_for(&state, &item.project_path).await?;
    let link = match thoughts_links::provision_thoughts(worktree_path, &effective) {
        Ok(link) => Some(link),
        Err(e) => {
            log::warn!("thoughts re-provision after restore failed: {e}");
            None
        }
    };
    if let Some(mapped) = effective.mapped_name.as_deref() {
        if let Err(e) = thoughts_links::add_worktree_mapping(
            state.config.path(),
            worktree_path,
            mapped,
            effective.profile_name.as_deref(),
        ) {
            log::warn!("could not re-add worktree mapping after restore: {e}");
        }
    }
    state.config.invalidate().await;

    let id_for_db = id.clone();
    db_task(state.db.clone(), move |db| {
        db.unarchive_workstream(&id_for_db)
    })
    .await?;

    let item = load_item(&state, &id).await?;
    Ok(item_view(item, link))
}

/// Permanent delete: remove the archive directory, prune git's worktree
/// registry, optionally drop the branch (per `git.delete_branch_on_archive`,
/// which now governs the *permanent delete* side of the archive lifecycle),
/// and DELETE the workstream row.
#[tauri::command]
#[specta::specta]
pub async fn delete_archived_workstream(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    if item.status != "archived" {
        return Err(AppError {
            code: "not_archived".into(),
            message: format!("{} is not archived.", item.title),
            remedy: Some("Archive it first.".into()),
        });
    }
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let project_path = Path::new(&item.project_path);
    let archive_path = archive_dir_for(project_path, &item.slug);
    let delete_branch = state.settings.get().await.git.delete_branch_on_archive;

    state
        .worktrees
        .purge_archive(
            &git,
            project_path,
            &archive_path,
            &item.branch,
            delete_branch,
        )
        .await
        .map_err(AppError::from)?;

    db_task(state.db.clone(), move |db| db.delete_workstream(&id)).await
}

/// Report returned by [`cleanup_archived_workstreams`]: rows successfully
/// deleted vs. rows that failed (logged individually as warnings).
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupReport {
    pub deleted: u32,
    pub failed: u32,
}

/// Bulk permanent-delete of archived workstreams. `before_ms = None` purges
/// ALL archived rows; `Some(cutoff)` purges those with `archived_at < cutoff`.
/// Per-row reuse of purge_archive + delete_workstream; best-effort (failures
/// counted, not fatal), mirroring delete_archived_workstream.
#[tauri::command]
#[specta::specta]
pub async fn cleanup_archived_workstreams(
    state: State<'_, AppState>,
    before_ms: Option<f64>,
) -> CmdResult<ArchiveCleanupReport> {
    let git = state
        .binaries
        .require(BinaryKind::Git)
        .await
        .map_err(AppError::from)?;
    let delete_branch = state.settings.get().await.git.delete_branch_on_archive;

    let items = db_task(state.db.clone(), |db| db.list_archived_workstreams()).await?;

    let mut deleted = 0u32;
    let mut failed = 0u32;

    for w in items {
        // Filter: None → all; Some(cut) → only rows archived before the cutoff.
        if let Some(cut) = before_ms {
            if !w.archived_at.map_or(false, |a| a < cut) {
                continue;
            }
        }

        let project_path = PathBuf::from(&w.project_path);
        let archive_path = archive_dir_for(&project_path, &w.slug);
        let id = w.id.clone();
        let branch = w.branch.clone();

        if let Err(e) = state
            .worktrees
            .purge_archive(&git, &project_path, &archive_path, &branch, delete_branch)
            .await
        {
            log::warn!("cleanup_archived_workstreams: purge {id} failed: {e}");
            failed += 1;
            continue;
        }

        match db_task(state.db.clone(), move |db| db.delete_workstream(&id)).await {
            Ok(_) => deleted += 1,
            Err(e) => {
                log::warn!("cleanup_archived_workstreams: db delete failed: {e:?}");
                failed += 1;
            }
        }
    }

    Ok(ArchiveCleanupReport { deleted, failed })
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

/// Run a blocking git operation against a workstream's worktree: load the item,
/// hand its worktree path to `f` on the blocking pool, and map any core error to
/// an `AppError` tagged with `op` (the stable code the frontend branches on).
/// Every command below is a one-line call to this.
async fn with_worktree<T, F>(
    state: &State<'_, AppState>,
    id: &str,
    op: &'static str,
    f: F,
) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T, CoreError> + Send + 'static,
{
    let item = load_item(state, id).await?;
    blocking(op, move || f(Path::new(&item.worktree_path)))
        .await?
        .map_err(|e| AppError {
            code: op.to_string(),
            message: e.to_string(),
            remedy: Some("The worktree may have been deleted; archive this workstream.".into()),
        })
}

/// Split staged/unstaged status for the source-control panel (thoughts/ and
/// .hyprlayer/ excluded).
#[tauri::command]
#[specta::specta]
pub async fn worktree_git_status(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<WorktreeGitStatus> {
    with_worktree(&state, &id, "worktree_git_status", |p| {
        hyprlayer_core::gitops::worktree_git_status(p)
    })
    .await
}

/// Stage the given worktree-relative paths (HEAD→index).
#[tauri::command]
#[specta::specta]
pub async fn worktree_stage(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    with_worktree(&state, &id, "worktree_stage", move |p| {
        hyprlayer_core::gitops::stage_files(p, &paths)
    })
    .await
}

/// Unstage the given paths (reset index entries back to HEAD).
#[tauri::command]
#[specta::specta]
pub async fn worktree_unstage(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    with_worktree(&state, &id, "worktree_unstage", move |p| {
        hyprlayer_core::gitops::unstage_files(p, &paths)
    })
    .await
}

/// Discard working-tree changes for the given paths (restore tracked to HEAD,
/// delete untracked). Destructive — the UI confirms first.
#[tauri::command]
#[specta::specta]
pub async fn worktree_discard(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    with_worktree(&state, &id, "worktree_discard", move |p| {
        hyprlayer_core::gitops::discard_files(p, &paths)
    })
    .await
}

/// The two file texts the diff viewer compares (HEAD/index/working-tree depending
/// on `staged`).
#[tauri::command]
#[specta::specta]
pub async fn worktree_file_sides(
    state: State<'_, AppState>,
    id: String,
    path: String,
    staged: bool,
) -> CmdResult<FileSides> {
    with_worktree(&state, &id, "worktree_file_sides", move |p| {
        hyprlayer_core::gitops::worktree_file_sides(p, &path, staged)
    })
    .await
}

/// Immediate children of a directory in the worktree (lazy "All Files" tree).
#[tauri::command]
#[specta::specta]
pub async fn worktree_list_dir(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<Vec<DirEntry>> {
    with_worktree(&state, &id, "worktree_list_dir", move |p| {
        hyprlayer_core::gitops::list_dir(p, &path)
    })
    .await
}

/// Read a worktree file as text for the read-only viewer.
#[tauri::command]
#[specta::specta]
pub async fn worktree_read_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<String> {
    with_worktree(&state, &id, "worktree_read_file", move |p| {
        hyprlayer_core::gitops::read_text_file(p, &path)
    })
    .await
}

/// Slash commands available in this worktree (project + user `.claude`), for the
/// composer's `/skill` autocomplete and highlight.
#[tauri::command]
#[specta::specta]
pub async fn list_slash_commands(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<SlashCommand>> {
    with_worktree(&state, &id, "list_slash_commands", |p| {
        Ok(hyprlayer_core::skills::list_slash_commands(p))
    })
    .await
}

/// Worktree-relative file paths matching `query`, for the composer's `@file`
/// autocomplete. Returns at most `limit` results.
#[tauri::command]
#[specta::specta]
pub async fn worktree_search_files(
    state: State<'_, AppState>,
    id: String,
    query: String,
    limit: u32,
) -> CmdResult<Vec<String>> {
    with_worktree(&state, &id, "worktree_search_files", move |p| {
        hyprlayer_core::gitops::search_files(p, &query, limit as usize)
    })
    .await
}

/// The pull request for a workstream — the manually attached PR (by number) when
/// one is set, otherwise the branch's own PR. Derived live from `gh`. Degrades to
/// `None` (never an error) when gh is missing/unauthenticated or the call fails —
/// the PR bar simply stays hidden, matching `list_recent_repos`.
#[tauri::command]
#[specta::specta]
pub async fn workstream_pr_status(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Option<PrStatus>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(None);
    };
    let pr_ref = item.pr_ref();
    match pr::pr_status(&gh, Path::new(&item.worktree_path), &pr_ref).await {
        Ok(status) => Ok(status),
        Err(e) => {
            log::debug!("gh pr status unavailable for {pr_ref}: {e}");
            Ok(None)
        }
    }
}

/// Open PRs in the workstream's repo for the "Attach PR" picker, filtered by
/// `query` (free-text GitHub search when non-empty). Degrades to an empty list
/// (never an error) when gh is missing/unauthenticated or the repo has none, so
/// the picker shows its "No open PRs found" state rather than failing.
#[tauri::command]
#[specta::specta]
pub async fn list_workstream_open_prs(
    state: State<'_, AppState>,
    id: String,
    query: String,
) -> CmdResult<Vec<PrSummary>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(Vec::new());
    };
    // List from the worktree when it's on disk, else the main repo — both
    // resolve to the same remote, and a just-archived worktree shouldn't error.
    let dir = pr_dir(&item);
    match pr::list_open_prs(&gh, dir, &query, 50).await {
        Ok(prs) => Ok(prs),
        Err(e) => {
            log::debug!("gh pr list unavailable for {}: {e}", item.project_path);
            Ok(Vec::new())
        }
    }
}

/// A live preview of the PR a user typed into the attach picker (by number), or
/// `None` when no such PR exists. Degrades to `None` (never an error) on any gh
/// failure, so the picker shows its "not found" hint rather than failing.
#[tauri::command]
#[specta::specta]
pub async fn preview_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    pr_number: u32,
) -> CmdResult<Option<PrSummary>> {
    let item = load_item(&state, &id).await?;
    let Ok(gh) = state.binaries.require(BinaryKind::Gh).await else {
        return Ok(None);
    };
    match pr::pr_summary(&gh, pr_dir(&item), &pr_number.to_string()).await {
        Ok(summary) => Ok(summary),
        Err(e) => {
            log::debug!("gh pr preview unavailable for #{pr_number}: {e}");
            Ok(None)
        }
    }
}

/// Attach PR `pr_number` to a workstream, overriding branch-based detection.
/// Validates the number resolves to a real PR in the repo before persisting, so
/// a typo'd number surfaces an error in the dialog instead of silently hiding
/// the PR bar. Returns the freshly fetched status (what the bar then renders).
#[tauri::command]
#[specta::specta]
pub async fn attach_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    pr_number: u32,
) -> CmdResult<Option<PrStatus>> {
    let item = load_item(&state, &id).await?;
    let gh = state
        .binaries
        .require(BinaryKind::Gh)
        .await
        .map_err(AppError::from)?;
    let status = match pr::pr_status(&gh, pr_dir(&item), &pr_number.to_string()).await {
        Ok(Some(status)) => status,
        Ok(None) => return Err(attach_pr_error(pr_number, None)),
        // A bad number doesn't hit the "no pull requests found" path — gh fails
        // with GitHub's own message ("Could not resolve to a PullRequest…").
        // Surface that text rather than the framed `gh` exec error.
        Err(e) => return Err(attach_pr_error(pr_number, Some(e))),
    };
    db_task(state.db.clone(), move |db| {
        db.set_workstream_pr(&id, Some(pr_number))
    })
    .await?;
    Ok(Some(status))
}

/// Build a dialog-facing error for a failed PR attach. Surfaces GitHub's own
/// message (from gh's stderr) without the underlying `gh …` invocation framing;
/// falls back to a plain "not found" when there's no usable detail.
fn attach_pr_error(pr_number: u32, err: Option<CoreError>) -> AppError {
    let detail = match err {
        Some(CoreError::Exec { stderr_tail, .. }) => clean_gh_stderr(&stderr_tail),
        _ => String::new(),
    };
    let message = if detail.is_empty() {
        format!("No pull request #{pr_number} found in this repository")
    } else {
        detail
    };
    AppError {
        code: "pr_not_found".into(),
        message,
        remedy: None,
    }
}

/// Strip gh/GraphQL decoration from a stderr line so the user sees just the
/// message: drops a leading `GraphQL: ` tag and a trailing space-free
/// `(path.hint)` GraphQL field path (e.g. `(repository.pullRequest)`).
fn clean_gh_stderr(stderr: &str) -> String {
    let mut s = stderr.trim();
    s = s.strip_prefix("GraphQL: ").unwrap_or(s);
    if let Some(open) = s.rfind(" (") {
        if s.ends_with(')') && !s[open + 2..s.len() - 1].contains(' ') {
            s = s[..open].trim_end();
        }
    }
    s.to_string()
}

/// Detach any manually attached PR, reverting to branch-based detection.
#[tauri::command]
#[specta::specta]
pub async fn detach_workstream_pr(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    load_item(&state, &id).await?;
    db_task(state.db.clone(), move |db| db.set_workstream_pr(&id, None)).await
}

/// Merge a workstream's PR with the chosen method (squash/merge/rebase).
#[tauri::command]
#[specta::specta]
pub async fn merge_workstream_pr(
    state: State<'_, AppState>,
    id: String,
    method: MergeMethod,
) -> CmdResult<()> {
    let item = load_item(&state, &id).await?;
    let gh = state
        .binaries
        .require(BinaryKind::Gh)
        .await
        .map_err(AppError::from)?;
    let auto = state.settings.get().await.git.automerge;
    pr::merge_pr(
        &gh,
        Path::new(&item.worktree_path),
        &item.pr_ref(),
        method,
        auto,
    )
    .await
    .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_gh_stderr_strips_graphql_decoration() {
        // The real "unknown PR number" message: GraphQL tag + trailing field path.
        assert_eq!(
            clean_gh_stderr(
                "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)"
            ),
            "Could not resolve to a PullRequest with the number of 999."
        );
        // A plain message is passed through untouched (trimmed only).
        assert_eq!(clean_gh_stderr("  not found  "), "not found");
        // A trailing parenthetical that isn't a field path (has spaces) is kept.
        assert_eq!(clean_gh_stderr("failed (try again later)"), "failed (try again later)");
    }

    #[test]
    fn attach_pr_error_uses_gh_message_then_falls_back() {
        let err = CoreError::Exec {
            program: "gh pr view".into(),
            exit: Some(1),
            stderr_tail: "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)".into(),
        };
        // The exec error's stderr becomes the message — no `gh` framing leaks.
        let app = attach_pr_error(999, Some(err));
        assert_eq!(app.message, "Could not resolve to a PullRequest with the number of 999.");
        assert!(!app.message.contains("gh"));
        // No usable detail → a plain not-found fallback.
        assert_eq!(
            attach_pr_error(7, None).message,
            "No pull request #7 found in this repository"
        );
    }
}
