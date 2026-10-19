use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprlayer_models::RuntimeEvent;
use serde::{Deserialize, Serialize};

use crate::agent::Harness;
use crate::branch_prefix;
use crate::claude::memory;
use crate::config::EffectiveConfig;
use crate::error::{CoreError, Result};
use crate::hyprcli::HyprCli;
use crate::pipeline::Stage;
use crate::pr::{self, MergeMethod, PrStatus, PrSummary};
use crate::process::binaries::BinaryKind;
use crate::repo_settings;
use crate::runtime::script_registry::RunPhase;
use crate::runtime::scripts::RunLifecycleRequest;
use crate::scripts::ScriptKind;
use crate::store::{
    now_ms, ArchiveRecord, NewWorkstream, Workstream, WorkstreamDoc, WorkstreamStatus, WorktreeMode,
};
use crate::worktree::seed::SeedReport;
use crate::worktree::thoughts_links::{self, LinkStatus};
use crate::worktree::{self, archive_dir_for, BranchCleanup, CreateMode, CreateSpec, RemovePolicy};

use super::doc_status;
use super::HyprlayerRuntime;

pub type WorkstreamGitStatus = worktree::WorktreeStatus;
pub type WorkstreamPrStatus = Option<PrStatus>;
pub use crate::claude::memory::{MemoryFile, MemoryKeyCheck};

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
    /// Elsewhere than the original path; an existing folder receives the
    /// worktree under its slug.
    pub target: Option<PathBuf>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamBases {
    pub branches: Vec<String>,
    pub local_branches: Vec<String>,
    pub remote_branches: Vec<String>,
    pub default_base: String,
    pub current_branch: Option<String>,
    #[serde(default)]
    pub worktrees: Vec<BranchWorktree>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BranchWorktree {
    pub branch: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemory {
    pub dir: String,
    pub key_check: MemoryKeyCheck,
    pub files: Vec<MemoryFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ArchivedPr {
    Unknown,
    #[serde(rename = "none")]
    NoPr,
    Open {
        number: u32,
    },
    Merged {
        number: u32,
    },
    Closed {
        number: u32,
    },
}

impl ArchivedPr {
    fn from_record(record: &ArchiveRecord) -> Self {
        match (record.pr_state.as_deref(), record.pr_number) {
            (Some("none"), _) => Self::NoPr,
            (Some("open"), Some(number)) => Self::Open { number },
            (Some("merged"), Some(number)) => Self::Merged { number },
            (Some("closed"), Some(number)) => Self::Closed { number },
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ArchiveInfo {
    /// By the server's clock, so the countdown and the sweep agree.
    pub purge_in_ms: f64,
    pub kept: bool,
    pub size_bytes: Option<f64>,
    pub uncommitted: Option<u32>,
    pub unpushed: Option<u32>,
    /// Whether deleting it also deletes its branch.
    pub deletes_branch: bool,
    pub pr: ArchivedPr,
}

#[derive(Debug, Clone)]
pub struct ArchivedWorkstream {
    pub item: Workstream,
    pub archive_path: String,
    pub archive_exists: bool,
    pub info: ArchiveInfo,
}

const PR_RECHECK_MS: f64 = 15.0 * 60_000.0;
const PR_LOOKUPS_AT_ONCE: usize = 4;

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

    pub async fn list_archived_details(&self) -> Result<Vec<ArchivedWorkstream>> {
        ensure_running(self.runtime)?;
        let delete_branch = self
            .runtime
            .settings
            .get()
            .await
            .git
            .delete_branch_on_archive;
        let db = self.runtime.db.clone();
        blocking(move || {
            let now = now_ms();
            db.ensure_archive_records(now)?;
            Ok(db
                .archived_with_records()?
                .into_iter()
                .map(|(item, record)| archived_view(item, &record, now, delete_branch))
                .collect())
        })
        .await
    }

    /// Measures archives not yet scanned and asks `gh` about PRs whose state
    /// could still change. Returns how many rows changed.
    pub async fn refresh_archived(&self) -> Result<u32> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        let rows = blocking(move || {
            db.ensure_archive_records(now_ms())?;
            db.archived_with_records()
        })
        .await?;
        let mut changed = 0u32;

        for (item, _) in rows
            .iter()
            .filter(|(item, record)| record.scanned_at.is_none() && item.app_owns_worktree())
        {
            let dir = archive_dir_for(Path::new(&item.project_path), &item.slug);
            let base = item.base_branch.clone();
            let id = item.id.clone();
            let db = self.runtime.db.clone();
            blocking(move || {
                db.cache_archive_scan(&id, &worktree::scan_archive(&dir, &base), now_ms())
            })
            .await?;
            changed += 1;
        }

        let now = now_ms();
        let due: Vec<_> = rows
            .into_iter()
            .filter(|(item, record)| pr_lookup_due(item, record, now))
            .collect();
        if due.is_empty() {
            return Ok(changed);
        }
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return Ok(changed);
        };
        let overlay = self.runtime.child_env().await;
        let permits = Arc::new(tokio::sync::Semaphore::new(PR_LOOKUPS_AT_ONCE));
        let mut lookups = tokio::task::JoinSet::new();
        for (item, record) in due {
            let (gh, overlay, permits) = (gh.clone(), overlay.clone(), permits.clone());
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let found = pr::pr_status(&gh, pr_dir(&item), &item.pr_ref(), &overlay).await;
                (item, record, found)
            });
        }
        while let Some(joined) = lookups.join_next().await {
            let Ok((item, record, found)) = joined else {
                continue;
            };
            let id = item.id;
            if let Ok(Some(status)) = &found {
                doc_status::settle_workstream(
                    self.runtime.db.clone(),
                    &self.runtime.config,
                    &id,
                    &item.project_path,
                    status.number,
                    status.state,
                )
                .await;
            }
            let cached = match found {
                Ok(Some(status)) => Some((Some(status.number), pr_state_name(&status.state))),
                Ok(None) => Some((None, "none")),
                Err(e) => {
                    tracing::debug!("PR lookup for archived workstream {id} failed: {e}");
                    None
                }
            };
            if cached.is_some_and(|(number, state)| {
                record.pr_number != number || record.pr_state.as_deref() != Some(state)
            }) {
                changed += 1;
            }
            let db = self.runtime.db.clone();
            blocking(move || db.cache_archive_pr(&id, cached, now_ms())).await?;
        }
        Ok(changed)
    }

    /// Exempts an archive from the sweep until it is deleted.
    pub async fn keep_archived(&self, id: &str) -> Result<()> {
        ensure_running(self.runtime)?;
        let peek = self.find(id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, id)
            .await;
        let item = self.find(id).await?;
        if item.status != WorkstreamStatus::Archived {
            return Err(CoreError::WorkstreamNotArchived {
                title: item.title,
                actionable: false,
            });
        }
        let db = self.runtime.db.clone();
        let owned = id.to_string();
        blocking(move || {
            let now = now_ms();
            db.ensure_archive_records(now)?;
            db.keep_archive(&owned, now).map(|_| ())
        })
        .await
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
        let item = self.find(id).await?;
        let base = base.trim().to_string();
        if base.is_empty() {
            return Err(CoreError::WorkstreamBaseBranchEmpty);
        }
        if let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await {
            let dir = pr_dir(&item);
            if let Ok(Some(status)) =
                pr::pr_status(&gh, dir, &item.pr_ref(), &self.runtime.child_env().await).await
            {
                if status.state == pr::PrState::Open {
                    pr::edit_pr_base(
                        &gh,
                        dir,
                        &item.pr_ref(),
                        &base,
                        &self.runtime.child_env().await,
                    )
                    .await?;
                }
            }
        }
        let db = self.runtime.db.clone();
        let id = id.to_string();
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

    pub async fn project_memory(&self, id: &str) -> Result<ProjectMemory> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let managed = self.runtime.paths.claude_managed_settings_files.clone();
        let config_dir = self.runtime.paths.claude_config_dir.clone();
        let home = self.runtime.environment.home_dir.clone();
        let db = self.runtime.db.clone();
        blocking(move || {
            let latest = db
                .list_sessions(&item.id)?
                .into_iter()
                .rev()
                .find(|s| matches!(s.harness, Harness::Claude));
            let (dir, key_check) = memory::resolve(
                &managed,
                &config_dir,
                &home,
                Path::new(&item.project_path),
                latest.as_ref().map(|s| (s.id.as_str(), Path::new(&s.cwd))),
            );
            let files = match key_check {
                MemoryKeyCheck::Mismatch => Vec::new(),
                _ => memory::list_files(&dir),
            };
            Ok(ProjectMemory {
                dir: dir.to_string_lossy().into_owned(),
                key_check,
                files,
            })
        })
        .await
    }

    pub async fn save_project_memory_file(
        &self,
        id: &str,
        name: &str,
        content: String,
        base_mtime_ms: f64,
    ) -> Result<()> {
        ensure_running(self.runtime)?;
        if !memory::is_memory_file_name(name) {
            return Err(CoreError::InvalidPath {
                path: name.to_string(),
            });
        }
        let item = self.find(id).await?;
        let managed = self.runtime.paths.claude_managed_settings_files.clone();
        let config_dir = self.runtime.paths.claude_config_dir.clone();
        let home = self.runtime.environment.home_dir.clone();
        let name = name.to_string();
        blocking(move || {
            let (dir, _) = memory::resolve(
                &managed,
                &config_dir,
                &home,
                Path::new(&item.project_path),
                None,
            );
            let target = dir.join(&name);
            match std::fs::symlink_metadata(&target) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(CoreError::InvalidPath { path: name })
                }
                Ok(meta) if meta.is_file() => {}
                _ => {
                    return Err(CoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("{name} is no longer in project memory"),
                    )))
                }
            }
            super::library::ensure_not_stale(&target, base_mtime_ms)?;
            let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
            let written = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .and_then(|mut file| file.write_all(content.as_bytes()))
                .and_then(|()| std::fs::rename(&tmp, &target));
            if written.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            written.map_err(CoreError::Io)
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
        match pr::pr_status(&gh, dir, &item.pr_ref(), &self.runtime.child_env().await).await {
            Ok(status) => Ok(status),
            // An error, not "no PR": the client keeps the last known PR and
            // backs off, where `None` made a spent GraphQL quota erase it.
            Err(e) => {
                let text = e.to_string().to_lowercase();
                let cause = if text.contains("rate limit") {
                    "the GitHub API quota is spent"
                } else if crate::repos::is_github_auth_failure(&text) {
                    "gh is not signed in"
                } else {
                    "gh failed"
                };
                tracing::debug!("PR status for {} unavailable: {cause}: {e}", item.pr_ref());
                Err(e)
            }
        }
    }

    pub async fn list_bases(&self, project_path: &str) -> Result<WorkstreamBases> {
        ensure_running(self.runtime)?;
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        let repo = PathBuf::from(project_path);
        let overlay = self.runtime.child_env().await;
        let default_base = worktree::detect_base_branch(&git, &repo, &overlay)
            .await
            .unwrap_or_default();
        let (branches, local_branches, remote_branches, current_branch, worktrees) =
            blocking(move || {
                let branches = crate::gitops::list_branches(&repo)?;
                let local_branches = crate::gitops::list_local_branches(&repo)?;
                let remote_branches = crate::gitops::list_remote_branches(&repo)?
                    .into_iter()
                    .filter(|r| match crate::gitops::local_name_for_remote(r) {
                        Some(local) => !local_branches.iter().any(|l| l == local),
                        None => false,
                    })
                    .collect();
                let current_branch = crate::gitops::current_branch(&repo);
                let worktrees = attachable_worktrees(&repo)
                    .into_iter()
                    .map(|(branch, path)| BranchWorktree {
                        branch,
                        path: path.display().to_string(),
                    })
                    .collect();
                Ok((
                    branches,
                    local_branches,
                    remote_branches,
                    current_branch,
                    worktrees,
                ))
            })
            .await?;
        Ok(WorkstreamBases {
            branches,
            local_branches,
            remote_branches,
            default_base,
            current_branch,
            worktrees,
        })
    }

    pub async fn fetch_project_remote(&self, project_path: &str) -> Result<()> {
        ensure_running(self.runtime)?;
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        worktree::fetch_remote(
            &git,
            Path::new(project_path),
            &self.runtime.child_env().await,
        )
        .await
    }

    pub async fn update_from_base(&self, id: &str) -> Result<worktree::PullOutcome> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        worktree::pull_base(
            &git,
            Path::new(&item.worktree_path),
            &item.base_branch,
            &self.runtime.child_env().await,
        )
        .await
    }

    pub async fn fetch_remote(&self, id: &str) -> Result<()> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        worktree::fetch_remote(
            &git,
            Path::new(&item.worktree_path),
            &self.runtime.child_env().await,
        )
        .await
    }

    pub async fn list_open_prs(&self, id: &str, query: &str) -> Result<Vec<PrSummary>> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return Ok(Vec::new());
        };
        match pr::list_open_prs(
            &gh,
            pr_dir(&item),
            query,
            50,
            &self.runtime.child_env().await,
        )
        .await
        {
            Ok(prs) => Ok(prs),
            Err(e) => {
                tracing::debug!("gh pr list unavailable for {}: {e}", item.project_path);
                Ok(Vec::new())
            }
        }
    }

    pub async fn preview_pr(&self, id: &str, pr_number: u32) -> Result<Option<PrSummary>> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await else {
            return Ok(None);
        };
        match pr::pr_summary(
            &gh,
            pr_dir(&item),
            &pr_number.to_string(),
            &self.runtime.child_env().await,
        )
        .await
        {
            Ok(summary) => Ok(summary),
            Err(e) => {
                tracing::debug!("gh pr preview unavailable for #{pr_number}: {e}");
                Ok(None)
            }
        }
    }

    pub async fn attach_pr(&self, id: &str, pr_number: u32) -> Result<Option<PrStatus>> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        if item.is_folder() {
            return Err(CoreError::WorkstreamFolderNoGit);
        }
        let gh = self.runtime.binaries.require(BinaryKind::Gh).await?;
        let status = match pr::pr_status(
            &gh,
            pr_dir(&item),
            &pr_number.to_string(),
            &self.runtime.child_env().await,
        )
        .await
        {
            Ok(Some(status)) => status,
            Ok(None) => return Err(pr_not_found(pr_number, None)),
            Err(e) => return Err(pr_not_found(pr_number, Some(e))),
        };
        self.set_pr(id, Some(pr_number)).await?;
        Ok(Some(status))
    }

    pub async fn merge_pr(&self, id: &str, method: MergeMethod) -> Result<()> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let gh = self.runtime.binaries.require(BinaryKind::Gh).await?;
        let auto = self.runtime.settings.get().await.git.automerge;
        pr::merge_pr(
            &gh,
            Path::new(&item.worktree_path),
            &item.pr_ref(),
            method,
            auto,
            &self.runtime.child_env().await,
        )
        .await
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
            let overlay = self.runtime.child_env().await;
            if let Err(e) = worktree::fetch_remote(&git, &repo, &overlay).await {
                tracing::warn!("fetch before tracking {branch} failed; using the local ref: {e}");
            }
            worktree::create_tracking_branch(&git, &repo, &local, &branch, &overlay).await?;
            return Ok((local, true));
        }

        Err(CoreError::WorkstreamBranchInvalid { branch })
    }

    /// Callers hold the project lock, so no other workstream can claim `path` meanwhile.
    async fn ensure_unattached(&self, project_path: &str, path: &Path) -> Result<()> {
        let pp = project_path.to_string();
        let db = self.runtime.db.clone();
        let active = blocking(move || db.list_workstreams(&pp, false)).await?;
        match active
            .iter()
            .find(|w| canonical_prefix(Path::new(&w.worktree_path)) == path)
        {
            Some(owner) => Err(CoreError::WorktreeAlreadyAttached {
                path: path.display().to_string(),
                title: owner.title.clone(),
            }),
            None => Ok(()),
        }
    }

    pub async fn create(&self, req: CreateWorkstreamRequest) -> Result<CreateWorkstreamResult> {
        let started = std::time::Instant::now();
        let what = format!(
            "create {:?} in {} ({:?})",
            req.title, req.project_path, req.mode
        );
        let result = self.create_inner(req).await;
        match &result {
            Ok(created) => tracing::info!(
                "{what}: workstream {} on {} at {} in {:?}",
                created.item.id,
                created.item.branch,
                created.item.worktree_path,
                started.elapsed()
            ),
            Err(e) => tracing::warn!("{what} failed after {:?}: {e}", started.elapsed()),
        }
        result
    }

    async fn create_inner(&self, req: CreateWorkstreamRequest) -> Result<CreateWorkstreamResult> {
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
            WorktreeMode::Attached => {
                let branch = needs_branch("attached")?;
                let path = attachable_worktree(&project_path, &branch).await?;
                CreateMode::Attached { branch, path }
            }
            WorktreeMode::Folder => return Err(CoreError::WorkstreamModeNeedsBranch),
        };
        let inplace = mode == WorktreeMode::Inplace;
        let owns_folder = matches!(mode, WorktreeMode::Isolated | WorktreeMode::Existing);

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
        if let CreateMode::Attached { path, .. } = &create_mode {
            self.ensure_unattached(&project_path, path).await?;
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

        let (link, seed) = if !owns_folder {
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
                    tracing::warn!("rolling back {} after: {e}", worktree_path.display());
                    if let Err(rollback) = self
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
                        .await
                    {
                        tracing::warn!(
                            "rollback left {} in place: {rollback}",
                            worktree_path.display()
                        );
                    }
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
        let worktree_left = new_item.worktree_path.clone();
        let item = blocking(move || db.insert_workstream(new_item))
            .await
            .inspect_err(|e| {
                tracing::error!("not recorded, so {worktree_left} is left on disk untracked: {e}")
            })?;

        Ok(CreateWorkstreamResult {
            item,
            link: Some(link),
            seed,
        })
    }

    pub async fn archive(&self, req: ArchiveWorkstreamRequest) -> Result<Workstream> {
        let what = format!("archive workstream {}", req.id);
        logged(&what, self.archive_inner(req)).await
    }

    async fn archive_inner(&self, req: ArchiveWorkstreamRequest) -> Result<Workstream> {
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
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        "archive of {id} cannot see its sessions, so none are stopped: {e}"
                    );
                    Vec::new()
                })
        };
        self.runtime.goals().stop_for_workstream(&id).await;
        for session in &sessions {
            self.runtime
                .sessions()
                .kill_and_await_exit(&session.id)
                .await;
        }
        self.runtime.terminals.remove_for_workstream(&id).await;
        self.runtime.scripts.clear_workstream(&id).await;

        // After the kills: no agent of this workstream can write a doc past this point.
        let pr = self.archive_pr_status(&item).await;
        if let Some(status) = &pr {
            doc_status::settle_workstream(
                self.runtime.db.clone(),
                &self.runtime.config,
                &id,
                &item.project_path,
                status.number,
                status.state,
            )
            .await;
        }

        if !item.app_owns_worktree() {
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
            && pr.is_some_and(|s| s.state == pr::PrState::Merged)
        {
            if let Err(e) = worktree::delete_remote_branch(
                &git,
                project_path,
                &item.branch,
                &self.runtime.child_env().await,
            )
            .await
            {
                tracing::warn!(
                    "delete remote branch {} on archive failed: {e}",
                    item.branch
                );
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
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        "archive prunes every ignored file: .hyprlayer/settings.toml unreadable: {e}"
                    );
                    Vec::new()
                });
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

        let scan_dir = moved.then(|| archive_path.clone());
        let base = item.base_branch.clone();
        let db = self.runtime.db.clone();
        let aid = id.clone();
        blocking(move || {
            // Scanned once: nothing writes to an archive after this.
            let scan = scan_dir.map(|dir| worktree::scan_archive(&dir, &base));
            db.archive_workstream(&aid)?;
            match scan {
                Some(scan) => db.cache_archive_scan(&aid, &scan, now_ms()),
                None => Ok(()),
            }
        })
        .await?;
        self.find(&id).await
    }

    async fn archive_pr_status(&self, item: &Workstream) -> Option<pr::PrStatus> {
        if !has_own_pr(item) {
            return None;
        }
        let gh = self.runtime.binaries.require(BinaryKind::Gh).await.ok()?;
        pr::pr_status(
            &gh,
            pr_dir(item),
            &item.pr_ref(),
            &self.runtime.child_env().await,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::debug!("archive of {}: PR lookup failed: {e}", item.id);
            None
        })
    }

    pub async fn restore(&self, req: RestoreWorkstreamRequest) -> Result<RestoreWorkstreamResult> {
        let what = format!("restore workstream {}", req.id);
        logged(&what, self.restore_inner(req)).await
    }

    async fn restore_inner(
        &self,
        req: RestoreWorkstreamRequest,
    ) -> Result<RestoreWorkstreamResult> {
        ensure_running(self.runtime)?;
        let RestoreWorkstreamRequest { id, target } = req;
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

        if !item.app_owns_worktree() {
            if let Some(target) = target {
                return Err(CoreError::WorkstreamRestoreTargetInvalid {
                    path: target.display().to_string(),
                    reason: "it has no worktree of its own to move".into(),
                });
            }
            if item.worktree_mode == WorktreeMode::Attached {
                let path = canonical_prefix(Path::new(&item.worktree_path));
                if !worktree::is_live_worktree(&path) {
                    return Err(CoreError::WorkstreamWorktreeMissing {
                        path: item.worktree_path.clone(),
                    });
                }
                self.ensure_unattached(&item.project_path, &path).await?;
            }
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
        let original = PathBuf::from(&item.worktree_path);
        let archive_path = archive_dir_for(project_path, &item.slug);

        if !archive_path.exists() {
            return Err(CoreError::WorkstreamArchiveMissing {
                path: archive_path.display().to_string(),
            });
        }
        let destination = match &target {
            Some(raw) => self.restore_destination(&item, raw).await?,
            None => original.clone(),
        };
        if destination.exists() {
            return Err(CoreError::WorkstreamRestoreTargetOccupied {
                path: destination.display().to_string(),
            });
        }

        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        self.runtime
            .worktrees
            .restore(&git, project_path, &archive_path, &destination)
            .await?;

        // Committed before the best-effort steps below: a failure there must not
        // leave a live worktree on a row the sweep still treats as archived.
        let relocate = (destination != original).then(|| {
            (
                original.display().to_string(),
                destination.display().to_string(),
            )
        });
        let db = self.runtime.db.clone();
        let rid = id.clone();
        let moved = relocate.clone();
        blocking(move || {
            db.unarchive_workstream_at(
                &rid,
                moved
                    .as_ref()
                    .map(|(from, to)| (from.as_str(), to.as_str())),
            )
        })
        .await?;

        let link = match self.effective_for(&item.project_path).await {
            Ok(effective) => {
                let link = match thoughts_links::provision_thoughts(&destination, &effective) {
                    Ok(link) => Some(link),
                    Err(e) => {
                        tracing::warn!("thoughts re-provision after restore failed: {e}");
                        None
                    }
                };
                if let Some(mapped) = effective.mapped_name.as_deref() {
                    if let Err(e) = thoughts_links::add_worktree_mapping(
                        self.runtime.config.path(),
                        &destination,
                        mapped,
                        effective.profile_name.as_deref(),
                    ) {
                        tracing::warn!("could not re-add worktree mapping after restore: {e}");
                    }
                }
                link
            }
            Err(e) => {
                tracing::warn!("thoughts re-link after restore skipped: {e}");
                None
            }
        };
        self.runtime.config.invalidate().await;
        if relocate.is_some() {
            self.relocate_claude_transcripts(&id, &destination).await;
        }

        let item = self.find(&id).await?;
        Ok(RestoreWorkstreamResult { item, link })
    }

    async fn restore_destination(&self, item: &Workstream, raw: &Path) -> Result<PathBuf> {
        let invalid = |reason: &str| CoreError::WorkstreamRestoreTargetInvalid {
            path: raw.display().to_string(),
            reason: reason.to_string(),
        };
        let expanded = crate::repos::expand_user_path(
            &raw.to_string_lossy(),
            Some(&self.runtime.environment.home_dir),
        );
        // Relative would resolve against the server's working directory.
        if !expanded.is_absolute() {
            return Err(invalid("the path is not absolute"));
        }
        let resolved = canonical_prefix(&expanded);
        let destination = if resolved.is_dir() {
            resolved.join(&item.slug)
        } else {
            resolved
        };
        if destination.starts_with(canonical_prefix(Path::new(&item.project_path))) {
            return Err(invalid("it is inside the project"));
        }
        let own = PathBuf::from(&item.worktree_path);
        let db = self.runtime.db.clone();
        let others = blocking(move || db.list_worktree_paths()).await?;
        if others
            .iter()
            .map(PathBuf::from)
            .any(|other| other != own && destination.starts_with(&other))
        {
            return Err(invalid("it is inside another workstream's worktree"));
        }
        Ok(destination)
    }

    /// Copies each claude session's transcript to the new cwd's project key,
    /// where `--resume` looks. Never fatal: the worktree is already restored.
    async fn relocate_claude_transcripts(&self, id: &str, cwd: &Path) {
        let db = self.runtime.db.clone();
        let owned = id.to_string();
        let config_dir = self.runtime.paths.claude_config_dir.clone();
        let cwd = cwd.to_path_buf();
        let copied = blocking(move || {
            for session in db.list_sessions(&owned)? {
                if session.harness != Harness::Claude {
                    continue;
                }
                let mut ids = vec![session.id.clone()];
                ids.extend(session.provider_session_id.filter(|p| *p != session.id));
                for sid in ids {
                    if let Err(e) =
                        crate::claude::driver::relocate_transcript(&config_dir, &sid, &cwd)
                    {
                        tracing::warn!("claude transcript {sid} not copied: {e}");
                    }
                }
            }
            Ok(())
        })
        .await;
        if let Err(e) = copied {
            tracing::warn!("claude transcripts for {id} not copied: {e}");
        }
    }

    pub async fn purge(&self, req: PurgeWorkstreamRequest) -> Result<()> {
        let what = format!("purge workstream {}", req.id);
        logged(&what, self.purge_inner(req)).await
    }

    async fn purge_inner(&self, req: PurgeWorkstreamRequest) -> Result<()> {
        ensure_running(self.runtime)?;
        let PurgeWorkstreamRequest { id } = req;
        let peek = self.find(&id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, &id)
            .await;
        let item = self.find(&id).await?;
        let cleanup = self.branch_cleanup(&item, BranchCleanup::Force).await;
        self.purge_locked(&item, cleanup).await
    }

    /// `wanted` applies only when the app created the branch and the settings
    /// ask for it to go with the archive.
    async fn branch_cleanup(&self, item: &Workstream, wanted: BranchCleanup) -> BranchCleanup {
        let enabled = self
            .runtime
            .settings
            .get()
            .await
            .git
            .delete_branch_on_archive;
        if enabled && item.app_owns_branch() {
            wanted
        } else {
            BranchCleanup::Keep
        }
    }

    async fn purge_locked(&self, item: &Workstream, cleanup: BranchCleanup) -> Result<()> {
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
        self.runtime
            .worktrees
            .purge_archive(&git, project_path, &archive_path, &item.branch, cleanup)
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
            let cleanup = self.branch_cleanup(&item, BranchCleanup::Force).await;
            match self.purge_locked(&item, cleanup).await {
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

    pub async fn purge_archived(&self, ids: Vec<String>) -> Result<ArchiveCleanupReport> {
        ensure_running(self.runtime)?;
        let mut report = ArchiveCleanupReport {
            deleted: 0,
            failed: 0,
        };
        for id in ids {
            match self.purge(PurgeWorkstreamRequest { id }).await {
                Ok(()) => report.deleted += 1,
                Err(_) => report.failed += 1,
            }
        }
        Ok(report)
    }

    /// Purges archives past their deadline. Unlike a manual delete it never
    /// touches the remote, and removes the local branch only with `-d`.
    pub async fn sweep_expired_archives(&self) -> Result<ArchiveCleanupReport> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        let due = blocking(move || {
            let now = now_ms();
            db.ensure_archive_records(now)?;
            db.due_archive_ids(now)
        })
        .await?;
        let mut report = ArchiveCleanupReport {
            deleted: 0,
            failed: 0,
        };
        for id in due {
            let what = format!("sweep expired archive {id}");
            match logged(&what, self.sweep_one(&id)).await {
                Ok(true) => report.deleted += 1,
                Ok(false) => {}
                Err(_) => report.failed += 1,
            }
        }
        Ok(report)
    }

    async fn sweep_one(&self, id: &str) -> Result<bool> {
        let peek = self.find(id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, id)
            .await;
        let item = self.find(id).await?;
        let db = self.runtime.db.clone();
        let owned = id.to_string();
        let record = blocking(move || db.archive_record(&owned)).await?;
        let due = item.status == WorkstreamStatus::Archived
            && record.is_some_and(|r| r.kept_at.is_none() && r.purge_after <= now_ms());
        if !due || half_restored(&item) {
            return Ok(false);
        }
        let cleanup = self.branch_cleanup(&item, BranchCleanup::SafeLocal).await;
        self.purge_locked(&item, cleanup).await?;
        Ok(true)
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

fn archived_view(
    item: Workstream,
    record: &ArchiveRecord,
    now: f64,
    delete_branch: bool,
) -> ArchivedWorkstream {
    let archive_path = archive_dir_for(Path::new(&item.project_path), &item.slug);
    let archive_exists = !item.app_owns_worktree() || archive_path.exists();
    let info = ArchiveInfo {
        purge_in_ms: record.purge_after - now,
        kept: record.kept_at.is_some(),
        size_bytes: record.size_bytes,
        uncommitted: record.uncommitted,
        unpushed: record.unpushed,
        deletes_branch: delete_branch && item.app_owns_branch(),
        pr: ArchivedPr::from_record(record),
    };
    ArchivedWorkstream {
        archive_path: archive_path.display().to_string(),
        archive_exists,
        info,
        item,
    }
}

fn own_branch(item: &Workstream) -> bool {
    !item.branch.is_empty()
        && item.branch != item.base_branch
        && item.base_branch.strip_prefix("origin/") != Some(item.branch.as_str())
}

/// Whether a PR `pr_ref` finds is this workstream's own, not one whose head is its base branch.
pub(crate) fn has_own_pr(item: &Workstream) -> bool {
    !item.is_folder() && (item.pr_number.is_some() || own_branch(item))
}

/// Unknown or open, and not asked recently: a lookup could still change it.
fn pr_lookup_due(item: &Workstream, record: &ArchiveRecord, now: f64) -> bool {
    let own_branch = own_branch(item);
    let settled = matches!(
        record.pr_state.as_deref(),
        Some("none" | "merged" | "closed")
    );
    let recent = record
        .pr_checked_at
        .is_some_and(|at| now - at < PR_RECHECK_MS);
    own_branch && !settled && !recent
}

fn pr_state_name(state: &pr::PrState) -> &'static str {
    match state {
        pr::PrState::Open => "open",
        pr::PrState::Merged => "merged",
        pr::PrState::Closed => "closed",
    }
}

/// The worktree went back but the row never flipped, so its files are live.
fn half_restored(item: &Workstream) -> bool {
    item.app_owns_worktree()
        && !archive_dir_for(Path::new(&item.project_path), &item.slug).exists()
        && worktree::is_live_worktree(Path::new(&item.worktree_path))
}

/// Linked worktrees a workstream may attach to; the app's own archives are excluded.
fn attachable_worktrees(repo: &Path) -> Vec<(String, PathBuf)> {
    let archives = canonical_prefix(&repo.join(worktree::ARCHIVE_SUBDIR));
    match crate::gitops::branch_worktrees(repo) {
        Ok(all) => all
            .into_iter()
            .filter(|(_, path)| !path.starts_with(&archives))
            .collect(),
        Err(e) => {
            tracing::warn!("listing worktrees of {} failed: {e}", repo.display());
            Vec::new()
        }
    }
}

async fn attachable_worktree(project_path: &str, branch: &str) -> Result<PathBuf> {
    let repo = PathBuf::from(project_path);
    let worktrees = blocking(move || Ok(attachable_worktrees(&repo))).await?;
    worktrees
        .into_iter()
        .find_map(|(b, path)| (b == branch).then_some(path))
        .ok_or_else(|| CoreError::WorktreeNotFound {
            branch: branch.to_string(),
        })
}

/// Canonicalizes the longest existing ancestor and re-appends the rest.
fn canonical_prefix(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = dunce::canonicalize(existing) {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn pr_not_found(pr_number: u32, err: Option<CoreError>) -> CoreError {
    let detail = match err {
        Some(CoreError::Exec { stderr_tail, .. }) => clean_gh_stderr(&stderr_tail),
        _ => String::new(),
    };
    CoreError::PrNotFound { pr_number, detail }
}

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

/// Destructive transactions leave a line with their workstream id either way.
async fn logged<T>(what: &str, work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    let started = std::time::Instant::now();
    let result = work.await;
    match &result {
        Ok(_) => tracing::info!("{what} in {:?}", started.elapsed()),
        Err(e) => tracing::warn!("{what} failed after {:?}: {e}", started.elapsed()),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_gh_stderr_strips_graphql_decoration() {
        assert_eq!(
            clean_gh_stderr(
                "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)"
            ),
            "Could not resolve to a PullRequest with the number of 999."
        );
        assert_eq!(clean_gh_stderr("  not found  "), "not found");
        assert_eq!(
            clean_gh_stderr("failed (try again later)"),
            "failed (try again later)"
        );
    }

    #[test]
    fn pr_not_found_uses_gh_message_then_falls_back() {
        let err = CoreError::Exec {
            program: "gh pr view".into(),
            exit: Some(1),
            stderr_tail: "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)".into(),
        };
        let not_found = pr_not_found(999, Some(err));
        assert_eq!(
            not_found.to_string(),
            "Could not resolve to a PullRequest with the number of 999."
        );
        assert!(!not_found.to_string().contains("gh"));
        assert_eq!(not_found.code(), "pr_not_found");
        assert_eq!(
            pr_not_found(7, None).to_string(),
            "No pull request #7 found in this repository"
        );
    }
}
