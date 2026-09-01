use std::path::{Path, PathBuf};

use hyprlayer_models::RuntimeEvent;

use crate::branch_prefix;
use crate::config::EffectiveConfig;
use crate::error::{CoreError, Result};
use crate::hyprcli::HyprCli;
use crate::pipeline::Stage;
use crate::pr::{self, PrStatus};
use crate::process::binaries::BinaryKind;
use crate::repo_settings;
use crate::runtime::script_registry::RunPhase;
use crate::runtime::scripts::RunLifecycleRequest;
use crate::scripts::ScriptKind;
use crate::store::{NewWorkstream, Workstream, WorkstreamDoc, WorkstreamStatus, WorktreeMode};
use crate::worktree::seed::SeedReport;
use crate::worktree::thoughts_links::{self, LinkStatus};
use crate::worktree::{self, archive_dir_for, CreateMode, CreateSpec, RemovePolicy};

use super::HyprlayerRuntime;

pub type WorkstreamGitStatus = worktree::WorktreeStatus;
pub type WorkstreamPrStatus = Option<PrStatus>;

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

pub struct ListWorkstreamsRequest {
    pub project_path: String,
    pub include_archived: bool,
}

pub struct CreateWorkstreamRequest {
    pub project_path: String,
    pub title: String,
    pub base: Option<String>,
    pub mode: WorktreeMode,
    pub branch: Option<String>,
    pub branch_prefix: Option<String>,
}

pub struct CreateWorkstreamResult {
    pub item: Workstream,
    pub link: Option<LinkStatus>,
    pub seed: Option<SeedReport>,
}

pub struct RenameWorkstreamRequest {
    pub id: String,
    pub title: String,
}

pub struct UpdateWorkstreamRequest {
    pub id: String,
    pub stage: Option<Stage>,
}

pub struct ArchiveWorkstreamRequest {
    pub id: String,
    pub skip_scripts: bool,
}

pub struct RestoreWorkstreamRequest {
    pub id: String,
}

pub struct RestoreWorkstreamResult {
    pub item: Workstream,
    pub link: Option<LinkStatus>,
}

pub struct PurgeWorkstreamRequest {
    pub id: String,
}

pub struct CleanupArchivedRequest {
    pub before_ms: Option<f64>,
}

pub struct ArchiveCleanupReport {
    pub deleted: u32,
    pub failed: u32,
}

pub struct ReprovisionWorkstreamRequest {
    pub id: String,
}

pub struct ReprovisionWorkstreamResult {
    pub item: Workstream,
    pub link: LinkStatus,
}

pub struct WorkstreamGitStatusRequest {
    pub id: String,
}

pub struct WorkstreamPrStatusRequest {
    pub id: String,
}

pub struct WorkstreamDocumentsRequest {
    pub id: String,
}

pub struct HeadBranchObservation {
    pub item: Workstream,
    pub head_branch: Option<String>,
}

pub struct BranchAdoption {
    pub workstream_id: String,
    pub branch: String,
}

pub struct WorkstreamService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn workstreams(&self) -> WorkstreamService<'_> {
        WorkstreamService { runtime: self }
    }
}

impl WorkstreamService<'_> {
    pub async fn find(&self, id: &str) -> Result<Workstream> {
        let owned = id.to_string();
        let db = self.runtime.db.clone();
        blocking(move || db.get_workstream(&owned))
            .await?
            .ok_or_else(|| CoreError::WorkstreamNotFound { id: id.to_string() })
    }

    async fn effective_for(&self, project_path: &str) -> Result<EffectiveConfig> {
        let loaded = self.runtime.config.load().await?;
        let thoughts = loaded
            .config
            .thoughts
            .as_ref()
            .ok_or(CoreError::ThoughtsNotConfigured)?;
        Ok(thoughts.effective_config_for(project_path))
    }

    async fn run_lifecycle(&self, item: &Workstream, kind: ScriptKind) -> Option<RunPhase> {
        if let Some(runner) = &self.runtime.lifecycle_runner {
            return runner.run(item, kind).await;
        }
        self.runtime
            .scripts()
            .run_lifecycle(RunLifecycleRequest {
                workstream: item.clone(),
                kind,
            })
            .await
            .unwrap_or(None)
    }

    pub async fn list(&self, req: ListWorkstreamsRequest) -> Result<Vec<Workstream>> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        blocking(move || db.list_workstreams(&req.project_path, req.include_archived)).await
    }

    pub async fn list_all(&self) -> Result<Vec<Workstream>> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        blocking(move || db.list_active_workstreams()).await
    }

    pub async fn list_archived(&self) -> Result<Vec<Workstream>> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        blocking(move || db.list_archived_workstreams()).await
    }

    pub async fn documents(&self, req: WorkstreamDocumentsRequest) -> Result<Vec<WorkstreamDoc>> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        blocking(move || db.workstream_docs(&req.id)).await
    }

    pub async fn attach_doc(
        &self,
        id: &str,
        doc_path: &str,
        role: &str,
    ) -> Result<Vec<WorkstreamDoc>> {
        ensure_running(self.runtime)?;
        self.find(id).await?;
        let db = self.runtime.db.clone();
        let (wid, doc_path, role) = (id.to_string(), doc_path.to_string(), role.to_string());
        blocking(move || db.attach_workstream_doc(&wid, &doc_path, &role)).await?;
        self.documents(WorkstreamDocumentsRequest { id: id.to_string() })
            .await
    }

    pub async fn detach_doc(&self, id: &str, doc_path: &str) -> Result<Vec<WorkstreamDoc>> {
        ensure_running(self.runtime)?;
        self.find(id).await?;
        let db = self.runtime.db.clone();
        let (wid, doc_path) = (id.to_string(), doc_path.to_string());
        blocking(move || db.detach_workstream_doc(&wid, &doc_path)).await?;
        self.documents(WorkstreamDocumentsRequest { id: id.to_string() })
            .await
    }

    pub async fn set_base_branch(&self, id: &str, base: &str) -> Result<()> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        let (id, base) = (id.to_string(), base.to_string());
        blocking(move || db.update_workstream_base_branch(&id, &base)).await
    }

    pub async fn set_pr(&self, id: &str, pr_number: Option<u32>) -> Result<()> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        let id = id.to_string();
        blocking(move || db.set_workstream_pr(&id, pr_number)).await
    }

    /// Records the branch a follow-head workstream's worktree has moved onto,
    /// and announces each adoption on the runtime event stream. The caller
    /// supplies the observed head because it has just computed git status for
    /// the whole list; re-deriving it here would run that work twice.
    pub async fn adopt_head_branches(
        &self,
        observed: Vec<HeadBranchObservation>,
    ) -> Result<Vec<BranchAdoption>> {
        ensure_running(self.runtime)?;
        let adopted: Vec<(BranchAdoption, String)> = observed
            .iter()
            .filter_map(|o| {
                let next = o.item.branch_to_adopt(o.head_branch.as_deref())?;
                Some((
                    BranchAdoption {
                        workstream_id: o.item.id.clone(),
                        branch: next.to_string(),
                    },
                    o.item.branch.clone(),
                ))
            })
            .collect();
        if adopted.is_empty() {
            return Ok(Vec::new());
        }

        let writes: Vec<(String, String)> = adopted
            .iter()
            .map(|(a, _)| (a.workstream_id.clone(), a.branch.clone()))
            .collect();
        let db = self.runtime.db.clone();
        blocking(move || {
            for (id, branch) in &writes {
                db.update_workstream_branch(id, branch)?;
            }
            Ok(())
        })
        .await?;

        let mut adoptions = Vec::with_capacity(adopted.len());
        for (adoption, previous) in adopted {
            tracing::info!(
                "workstream {} adopted branch {} (was {previous})",
                adoption.workstream_id,
                adoption.branch
            );
            let _ = self
                .runtime
                .events
                .send(RuntimeEvent::WorkstreamBranchChanged {
                    workstream_id: adoption.workstream_id.clone(),
                    branch: adoption.branch.clone(),
                    previous,
                });
            adoptions.push(adoption);
        }
        Ok(adoptions)
    }

    /// Arms the single worktree watcher on one workstream, or disarms it when
    /// `workstream_id` is `None`. The handle is shared with the PR watcher,
    /// which polls the watched workstream more eagerly than the rest.
    pub async fn watch(&self, workstream_id: Option<String>) -> Result<()> {
        ensure_running(self.runtime)?;
        let mut guard = self.runtime.watchers.worktree_watch.lock().await;
        let Some(workstream_id) = workstream_id else {
            *guard = None;
            return Ok(());
        };
        if guard.as_ref().is_some_and(|(id, _)| id == &workstream_id) {
            return Ok(());
        }
        *guard = None;

        let item = self.find(&workstream_id).await?;
        let path = PathBuf::from(&item.worktree_path);
        if !path.is_dir() {
            return Ok(());
        }

        let events = self.runtime.events.clone();
        let watched = workstream_id.clone();
        let handle = blocking(move || {
            let git_dir = crate::gitops::git_dir(&path).unwrap_or_else(|| path.join(".git"));
            crate::index::watcher::watch_worktree(
                &path,
                &git_dir,
                Box::new(move || {
                    let _ = events.send(RuntimeEvent::WorktreeChanged {
                        workstream_id: watched.clone(),
                    });
                }),
            )
        })
        .await?;
        *guard = Some((workstream_id, handle));
        Ok(())
    }

    pub async fn git_status(&self, req: WorkstreamGitStatusRequest) -> Result<WorkstreamGitStatus> {
        ensure_running(self.runtime)?;
        let item = self.find(&req.id).await?;
        let link = if item.is_folder() {
            None
        } else {
            let effective = self.effective_for(&item.project_path).await.ok();
            effective
                .map(|eff| thoughts_links::check_thoughts(Path::new(&item.worktree_path), &eff))
        };
        let dir = PathBuf::from(&item.worktree_path);
        blocking(move || {
            Ok(if item.is_folder() {
                worktree::folder_status(&dir)
            } else {
                worktree::status(&dir, &item.base_branch, link)
            })
        })
        .await
    }

    pub async fn pr_status(&self, req: WorkstreamPrStatusRequest) -> Result<WorkstreamPrStatus> {
        ensure_running(self.runtime)?;
        let item = self.find(&req.id).await?;
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return Ok(None);
        };
        let dir = pr_dir(&item);
        match pr::pr_status(&gh, dir, &item.pr_ref()).await {
            Ok(status) => Ok(status),
            Err(e) => {
                tracing::debug!("gh pr status unavailable for {}: {e}", item.pr_ref());
                Ok(None)
            }
        }
    }

    pub async fn rename(&self, req: RenameWorkstreamRequest) -> Result<Workstream> {
        ensure_running(self.runtime)?;
        self.find(&req.id).await?;
        let db = self.runtime.db.clone();
        let title = req.title.trim().to_string();
        let id = req.id.clone();
        blocking(move || db.update_workstream_title(&id, &title)).await?;
        self.find(&req.id).await
    }

    pub async fn update(&self, req: UpdateWorkstreamRequest) -> Result<Workstream> {
        ensure_running(self.runtime)?;
        self.find(&req.id).await?;
        if let Some(stage) = req.stage {
            let db = self.runtime.db.clone();
            let id = req.id.clone();
            blocking(move || db.update_workstream_stage(&id, stage)).await?;
        }
        self.find(&req.id).await
    }

    async fn create_folder(
        &self,
        project_path: String,
        title: String,
    ) -> Result<CreateWorkstreamResult> {
        let dir = PathBuf::from(&project_path);
        if !dir.is_dir() {
            return Err(CoreError::WorkstreamFolderMissing { path: project_path });
        }
        let new_item = NewWorkstream {
            slug: worktree::unique_slug(&title),
            title,
            branch: String::new(),
            base_branch: String::new(),
            worktree_path: project_path.clone(),
            project_path,
            worktree_mode: WorktreeMode::Folder,
        };
        let db = self.runtime.db.clone();
        let item = blocking(move || db.insert_workstream(new_item)).await?;
        Ok(CreateWorkstreamResult {
            item,
            link: None,
            seed: None,
        })
    }

    async fn adopt_branch(&self, project_path: &str, branch: String) -> Result<(String, bool)> {
        let repo = PathBuf::from(project_path);
        let probe = repo.clone();
        let (locals, remotes) = blocking(move || {
            let locals = crate::gitops::list_local_branches(&probe)?;
            let remotes = crate::gitops::list_remote_branches(&probe)?;
            Ok::<_, CoreError>((locals, remotes))
        })
        .await?;

        if locals.contains(&branch) {
            return Ok((branch, false));
        }

        if remotes.contains(&branch) {
            let local = crate::gitops::local_name_for_remote(&branch)
                .unwrap_or(branch.as_str())
                .to_string();
            if locals.contains(&local) {
                return Ok((local, false));
            }
            let git = self.runtime.binaries.require(BinaryKind::Git).await?;
            let _ = worktree::fetch_remote(&git, &repo).await;
            worktree::create_tracking_branch(&git, &repo, &local, &branch).await?;
            return Ok((local, true));
        }

        Err(CoreError::WorkstreamBranchInvalid { branch })
    }

    pub async fn create(&self, req: CreateWorkstreamRequest) -> Result<CreateWorkstreamResult> {
        ensure_running(self.runtime)?;
        let CreateWorkstreamRequest {
            project_path,
            title,
            base,
            mode,
            branch,
            branch_prefix: branch_prefix_req,
        } = req;
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(CoreError::WorkstreamTitleRequired);
        }

        if !self
            .runtime
            .projects()
            .source_of(&project_path)
            .await
            .is_git()
        {
            return self.create_folder(project_path, title).await;
        }

        let picked = branch
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty());
        let (branch_choice, branch_is_ours) = match picked {
            Some(b) => {
                let (branch, created) = self.adopt_branch(&project_path, b).await?;
                (Some(branch), created)
            }
            None => (None, false),
        };

        let needs_branch = |m: &'static str| {
            branch_choice
                .clone()
                .ok_or(CoreError::WorkstreamNeedsBranch { mode: m })
        };
        let create_mode = match mode {
            WorktreeMode::Isolated => CreateMode::Isolated,
            WorktreeMode::Existing => CreateMode::Existing {
                branch: needs_branch("existing-branch")?,
            },
            WorktreeMode::Inplace => CreateMode::InPlace {
                branch: needs_branch("in-place")?,
            },
            WorktreeMode::Folder => return Err(CoreError::WorkstreamModeNeedsBranch),
        };
        let inplace = mode == WorktreeMode::Inplace;

        let _guard = self
            .runtime
            .workstream_locks
            .lock_project(&project_path)
            .await;

        if inplace {
            let pp = project_path.clone();
            let db = self.runtime.db.clone();
            let active = blocking(move || db.list_workstreams(&pp, false)).await?;
            if active.iter().any(|w| w.is_inplace()) {
                return Err(CoreError::InplaceWorkstreamExists);
            }
        }

        let effective = self.effective_for(&project_path).await?;
        let mapped = effective
            .mapped_name
            .clone()
            .ok_or_else(|| CoreError::ProjectNotFound {
                path: project_path.clone(),
            })?;

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        let picked = branch_prefix_req
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        let branch_prefix = match picked {
            Some(p) => p,
            None => self
                .runtime
                .settings
                .get()
                .await
                .git
                .effective_branch_prefix(),
        };
        if matches!(create_mode, CreateMode::Isolated) {
            branch_prefix::validate_prefix(&branch_prefix)?;
        }
        let base_override = base.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        let created = self
            .runtime
            .worktrees
            .create(
                &git,
                &CreateSpec {
                    project_repo: PathBuf::from(&project_path),
                    mapped_name: mapped.clone(),
                    title: title.clone(),
                    base_override,
                    branch_prefix,
                    mode: create_mode,
                },
            )
            .await?;

        let worktree_path = PathBuf::from(&created.path);

        let (link, seed) = if inplace {
            (
                thoughts_links::check_thoughts(&worktree_path, &effective),
                None,
            )
        } else {
            let cli_init = HyprCli::new(&self.runtime.binaries)
                .thoughts_init(&worktree_path, &mapped, effective.profile_name.as_deref())
                .await;
            let provision = match cli_init {
                Ok(_) => Ok(thoughts_links::check_thoughts(&worktree_path, &effective)),
                Err(init_err) => {
                    tracing::warn!(
                        "thoughts init in worktree failed ({init_err}); provisioning manually"
                    );
                    (|| -> Result<LinkStatus> {
                        let link = thoughts_links::provision_thoughts(&worktree_path, &effective)?;
                        thoughts_links::add_worktree_mapping(
                            self.runtime.config.path(),
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
                    let _ = self
                        .runtime
                        .worktrees
                        .remove(
                            &git,
                            Path::new(&project_path),
                            &worktree_path,
                            &created.branch,
                            &created.base_branch,
                            RemovePolicy::ForceDiscard,
                            mode == WorktreeMode::Isolated || branch_is_ours,
                        )
                        .await;
                    return Err(e);
                }
            };
            self.runtime.config.invalidate().await;

            let repo = PathBuf::from(&project_path);
            let wt = worktree_path.clone();
            let seed = blocking(move || {
                let patterns = repo_settings::read(&repo)
                    .map(|s| s.worktree.copy)
                    .unwrap_or_else(|e| {
                        tracing::warn!("read .hyprlayer/settings.toml: {e}");
                        Vec::new()
                    });
                Ok(worktree::seed::seed_worktree(&repo, &wt, &patterns))
            })
            .await
            .unwrap_or_default();
            if !seed.errors.is_empty() || !seed.missing.is_empty() {
                tracing::warn!(
                    "worktree seed: copied {}, missing {:?}, errors {:?}",
                    seed.copied.len(),
                    seed.missing,
                    seed.errors
                );
            } else if !seed.copied.is_empty() {
                tracing::info!("worktree seed: copied {} file(s)", seed.copied.len());
            }
            (link, Some(seed))
        };

        let new_item = NewWorkstream {
            project_path,
            title,
            slug: created.slug.clone(),
            branch: created.branch.clone(),
            worktree_path: created.path.clone(),
            base_branch: created.base_branch.clone(),
            worktree_mode: mode,
        };
        let db = self.runtime.db.clone();
        let item = blocking(move || db.insert_workstream(new_item)).await?;

        Ok(CreateWorkstreamResult {
            item,
            link: Some(link),
            seed,
        })
    }

    pub async fn archive(&self, req: ArchiveWorkstreamRequest) -> Result<Workstream> {
        ensure_running(self.runtime)?;
        let ArchiveWorkstreamRequest { id, skip_scripts } = req;
        let peek = self.find(&id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, &id)
            .await;
        let item = self.find(&id).await?;

        let sessions = {
            let db = self.runtime.db.clone();
            let sid = id.clone();
            blocking(move || db.list_sessions(&sid))
                .await
                .unwrap_or_default()
        };
        for session in &sessions {
            self.runtime
                .sessions()
                .kill_and_await_exit(&session.id)
                .await;
        }
        self.runtime.terminals.remove_for_workstream(&id).await;
        self.runtime.scripts.clear_workstream(&id).await;

        if item.shares_project_dir() {
            let db = self.runtime.db.clone();
            let aid = id.clone();
            blocking(move || db.archive_workstream(&aid)).await?;
            return self.find(&id).await;
        }

        if !skip_scripts
            && self.run_lifecycle(&item, ScriptKind::Archive).await == Some(RunPhase::Failed)
        {
            return Err(CoreError::ArchiveScriptFailed);
        }

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        let project_path = Path::new(&item.project_path);
        let worktree_path = Path::new(&item.worktree_path);
        let archive_path = archive_dir_for(project_path, &item.slug);
        let live = worktree::is_live_worktree(worktree_path);

        if self
            .runtime
            .settings
            .get()
            .await
            .git
            .delete_branch_on_archive
            && item.app_owns_branch()
        {
            if let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await {
                if let Ok(Some(status)) = pr::pr_status(&gh, pr_dir(&item), &item.pr_ref()).await {
                    if status.state == pr::PrState::Merged {
                        if let Err(e) =
                            worktree::delete_remote_branch(&git, project_path, &item.branch).await
                        {
                            tracing::warn!(
                                "delete remote branch {} on archive failed: {e}",
                                item.branch
                            );
                        }
                    }
                }
            }
        }

        let mut unmapped = false;
        if live {
            match HyprCli::new(&self.runtime.binaries)
                .thoughts_uninit(worktree_path)
                .await
            {
                Ok(_) => unmapped = true,
                Err(e) => tracing::warn!("thoughts uninit in worktree failed: {e}"),
            }
        }

        if live
            && self
                .runtime
                .settings
                .get()
                .await
                .git
                .prune_ignored_on_archive
        {
            let keep = repo_settings::read(project_path)
                .map(|s| s.worktree.copy)
                .unwrap_or_default();
            if let Err(e) = self
                .runtime
                .worktrees
                .clean_ignored(&git, worktree_path, &keep)
                .await
            {
                tracing::warn!("prune ignored files before archive failed: {e}");
            }
        }

        let moved = self
            .runtime
            .worktrees
            .archive(&git, project_path, worktree_path, &archive_path)
            .await?;
        if moved {
            if let Err(e) = worktree::ensure_archive_excluded(project_path) {
                tracing::warn!("could not write .git/info/exclude for archive: {e}");
            }
        }

        if !unmapped {
            if let Err(e) =
                thoughts_links::remove_worktree_mapping(self.runtime.config.path(), worktree_path)
            {
                tracing::warn!("could not remove worktree mapping: {e}");
            }
        }
        self.runtime.config.invalidate().await;

        let db = self.runtime.db.clone();
        let aid = id.clone();
        blocking(move || db.archive_workstream(&aid)).await?;
        self.find(&id).await
    }

    pub async fn restore(&self, req: RestoreWorkstreamRequest) -> Result<RestoreWorkstreamResult> {
        ensure_running(self.runtime)?;
        let RestoreWorkstreamRequest { id } = req;
        let peek = self.find(&id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, &id)
            .await;
        let item = self.find(&id).await?;
        if item.status != WorkstreamStatus::Archived {
            return Err(CoreError::WorkstreamNotArchived {
                title: item.title,
                actionable: false,
            });
        }

        if item.shares_project_dir() {
            let link = if item.is_folder() {
                None
            } else {
                let effective = self.effective_for(&item.project_path).await?;
                Some(thoughts_links::check_thoughts(
                    Path::new(&item.worktree_path),
                    &effective,
                ))
            };
            let db = self.runtime.db.clone();
            let rid = id.clone();
            blocking(move || db.unarchive_workstream(&rid)).await?;
            let item = self.find(&id).await?;
            return Ok(RestoreWorkstreamResult { item, link });
        }

        let project_path = Path::new(&item.project_path);
        let worktree_path = Path::new(&item.worktree_path);
        let archive_path = archive_dir_for(project_path, &item.slug);

        if !archive_path.exists() {
            return Err(CoreError::WorkstreamArchiveMissing {
                path: archive_path.display().to_string(),
            });
        }
        if worktree_path.exists() {
            return Err(CoreError::WorkstreamRestoreTargetOccupied {
                path: worktree_path.display().to_string(),
            });
        }

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        self.runtime
            .worktrees
            .restore(&git, project_path, &archive_path, worktree_path)
            .await?;

        let effective = self.effective_for(&item.project_path).await?;
        let link = match thoughts_links::provision_thoughts(worktree_path, &effective) {
            Ok(link) => Some(link),
            Err(e) => {
                tracing::warn!("thoughts re-provision after restore failed: {e}");
                None
            }
        };
        if let Some(mapped) = effective.mapped_name.as_deref() {
            if let Err(e) = thoughts_links::add_worktree_mapping(
                self.runtime.config.path(),
                worktree_path,
                mapped,
                effective.profile_name.as_deref(),
            ) {
                tracing::warn!("could not re-add worktree mapping after restore: {e}");
            }
        }
        self.runtime.config.invalidate().await;

        let db = self.runtime.db.clone();
        let rid = id.clone();
        blocking(move || db.unarchive_workstream(&rid)).await?;

        let item = self.find(&id).await?;
        Ok(RestoreWorkstreamResult { item, link })
    }

    pub async fn purge(&self, req: PurgeWorkstreamRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        let PurgeWorkstreamRequest { id } = req;
        let peek = self.find(&id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, &id)
            .await;
        let item = self.find(&id).await?;
        self.purge_locked(&item).await
    }

    async fn purge_locked(&self, item: &Workstream) -> Result<()> {
        if item.status != WorkstreamStatus::Archived {
            return Err(CoreError::WorkstreamNotArchived {
                title: item.title.clone(),
                actionable: true,
            });
        }
        if item.is_folder() {
            let db = self.runtime.db.clone();
            let id = item.id.clone();
            return blocking(move || db.delete_workstream(&id)).await;
        }

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        let project_path = Path::new(&item.project_path);
        let archive_path = archive_dir_for(project_path, &item.slug);
        let delete_branch = self
            .runtime
            .settings
            .get()
            .await
            .git
            .delete_branch_on_archive
            && item.app_owns_branch();

        self.runtime
            .worktrees
            .purge_archive(
                &git,
                project_path,
                &archive_path,
                &item.branch,
                delete_branch,
            )
            .await?;

        let db = self.runtime.db.clone();
        let id = item.id.clone();
        blocking(move || db.delete_workstream(&id)).await
    }

    pub async fn cleanup_archived(
        &self,
        req: CleanupArchivedRequest,
    ) -> Result<ArchiveCleanupReport> {
        ensure_running(self.runtime)?;
        let items = self.list_archived().await?;
        let items: Vec<_> = items
            .into_iter()
            .filter(|w| {
                req.before_ms
                    .is_none_or(|cut| w.archived_at.is_some_and(|a| a < cut))
            })
            .collect();

        let mut deleted = 0u32;
        let mut failed = 0u32;
        for item in items {
            let _guard = self
                .runtime
                .workstream_locks
                .lock_pair(&item.project_path, &item.id)
                .await;
            let Ok(item) = self.find(&item.id).await else {
                continue;
            };
            match self.purge_locked(&item).await {
                Ok(()) => deleted += 1,
                Err(e) => {
                    tracing::warn!(
                        "cleanup_archived_workstreams: purge {} failed: {e}",
                        item.id
                    );
                    failed += 1;
                }
            }
        }

        Ok(ArchiveCleanupReport { deleted, failed })
    }

    pub async fn reprovision(
        &self,
        req: ReprovisionWorkstreamRequest,
    ) -> Result<ReprovisionWorkstreamResult> {
        ensure_running(self.runtime)?;
        let ReprovisionWorkstreamRequest { id } = req;
        let peek = self.find(&id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, &id)
            .await;
        let item = self.find(&id).await?;
        if item.is_folder() {
            return Err(CoreError::WorkstreamFolderNoThoughts);
        }
        let worktree = PathBuf::from(&item.worktree_path);
        // provision_thoughts create_dir_all's its way to the link directory, so on a
        // deleted worktree it would resurrect the path as a bare symlink farm with no
        // .git — reporting success while leaving a squatter that blocks a later restore.
        if !worktree.is_dir() {
            return Err(CoreError::WorkstreamWorktreeMissing {
                path: item.worktree_path.clone(),
            });
        }
        let effective = self.effective_for(&item.project_path).await?;
        let link = thoughts_links::provision_thoughts(&worktree, &effective)?;
        Ok(ReprovisionWorkstreamResult { item, link })
    }
}

fn pr_dir(item: &Workstream) -> &Path {
    let worktree = Path::new(&item.worktree_path);
    if worktree.is_dir() {
        worktree
    } else {
        Path::new(&item.project_path)
    }
}
