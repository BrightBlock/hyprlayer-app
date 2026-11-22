use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use hyprlayer_models::RuntimeEvent;
use serde::{Deserialize, Serialize};

use crate::agent::Harness;
use crate::branch_prefix;
use crate::claude::memory;
use crate::config::EffectiveConfig;
use crate::error::{CoreError, Result};
use crate::hyprcli::HyprCli;
use crate::index::watcher::WatchHandle;
use crate::pipeline::Stage;
use crate::pr::{self, MergeMethod, PrLookup, PrLookupFailure, PrState, PrStatus, PrSummary};
use crate::process::binaries::BinaryKind;
use crate::repo_settings;
use crate::runtime::script_registry::RunPhase;
use crate::runtime::scripts::RunLifecycleRequest;
use crate::scripts::ScriptKind;
use crate::store::{
    now_ms, ArchiveRecord, ClaudeWorktree, Db, NewClaudeWorktree, NewWorkstream, ObservedPr,
    Workstream, WorkstreamDoc, WorkstreamStatus, WorktreeMode,
};
use crate::worktree::seed::SeedReport;
use crate::worktree::thoughts_links::{self, LinkStatus};
use crate::worktree::{self, archive_dir_for, BranchCleanup, CreateMode, CreateSpec, RemovePolicy};

use super::agent_control::AgentIdentity;
use super::doc_status;
use super::pr_watcher::{
    cached_lookup, cached_lookup_any_ref, edit_cached_pr, failure_cause, store_lookup, PrBudget,
};
use super::HyprlayerRuntime;

pub type WorkstreamGitStatus = worktree::WorktreeStatus;
pub type WorkstreamPrStatus = PrLookup;
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
    pub parent_workstream_id: Option<String>,
}

struct ClaudeName {
    name: String,
    harness_session_id: Option<String>,
}

fn same_repo(a: &Path, b: &Path) -> bool {
    let common = |path: &Path| {
        let repo = git2::Repository::discover(path).ok()?;
        dunce::canonicalize(crate::gitops::common_dir(&repo)).ok()
    };
    matches!((common(a), common(b)), (Some(a), Some(b)) if a == b)
}

/// Archive, restore and delete find an archive by its slug, so no two rows of a project share one.
fn free_slug(db: &Db, project_path: &str, stem: &str) -> Result<String> {
    for n in 1u32.. {
        let slug = match n {
            1 => stem.to_string(),
            n => format!("{stem}-{n}"),
        };
        if !db.workstream_slug_taken(project_path, &slug)? {
            return Ok(slug);
        }
    }
    unreachable!("some stem-N is free")
}

fn untouched(claude: &ClaudeWorktree) -> bool {
    let path = Path::new(&claude.workstream.worktree_path);
    let status = worktree::status(path, &claude.workstream.base_branch, None, None);
    let head = head_of(path);
    status.exists
        && status.unreadable.is_none()
        && status.dirty + status.untracked == 0
        && head.is_some()
        && head == claude.created_head
}

fn head_of(worktree: &Path) -> Option<String> {
    let repo = git2::Repository::open(worktree).ok()?;
    let head = repo.head().ok()?.target()?;
    Some(head.to_string())
}

pub struct CreateWorkstreamResult {
    pub item: Workstream,
    pub link: Option<LinkStatus>,
    pub seed: Option<SeedReport>,
    pub reused: bool,
}

pub struct RenameWorkstreamRequest {
    pub id: String,
    pub title: String,
}

/// The branch a title rename would move an app-made branch to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct BranchRename {
    pub branch: String,
    pub blocked: Option<BranchRenameBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum BranchRenameBlock {
    Pushed,
    HasPr,
    Moved,
    Taken,
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
const FETCHES_AT_ONCE: usize = 2;
const FETCH_BACKOFF: Duration = Duration::from_secs(30 * 60);
const REFOCUS_FETCH_GAP: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct FetchHistory {
    attempted: HashMap<String, Instant>,
    failing_until: HashMap<String, Instant>,
}

impl FetchHistory {
    fn due(&self, project: &str, now: Instant, regained: bool) -> bool {
        if regained {
            self.attempted
                .get(project)
                .is_none_or(|at| now.duration_since(*at) >= REFOCUS_FETCH_GAP)
        } else {
            self.failing_until
                .get(project)
                .is_none_or(|until| *until <= now)
        }
    }

    fn record(&mut self, project: String, at: Instant, fetched: bool) {
        if fetched {
            self.failing_until.remove(&project);
        } else {
            self.failing_until
                .insert(project.clone(), at + FETCH_BACKOFF);
        }
        self.attempted.insert(project, at);
    }
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
        blocking(move || {
            let items = db.list_workstreams(&req.project_path, req.include_archived)?;
            Ok(items.into_iter().map(without_moved_observation).collect())
        })
        .await
    }

    pub async fn list_all(&self) -> Result<Vec<Workstream>> {
        ensure_running(self.runtime)?;
        let db = self.runtime.db.clone();
        blocking(move || {
            let items = db.list_active_workstreams()?;
            Ok(items.into_iter().map(without_moved_observation).collect())
        })
        .await
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
            let merged = item.merged_pr_head().map(String::from);
            let id = item.id.clone();
            let db = self.runtime.db.clone();
            blocking(move || {
                let scan = worktree::scan_archive(&dir, &base, merged.as_deref());
                db.cache_archive_scan(&id, &scan, now_ms())
            })
            .await?;
            changed += 1;
        }

        let now = now_ms();
        let due: Vec<_> = rows
            .into_iter()
            .filter(|(item, record)| pr_lookup_due(item, record, now))
            .filter_map(|(item, record)| Some((current_pr_ref(&item)?, item, record)))
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
        for (pr_ref, item, record) in due {
            let (gh, overlay, permits) = (gh.clone(), overlay.clone(), permits.clone());
            let budget = self.runtime.watchers.pr_budget.clone();
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let found = workstream_pr(&gh, &item, &pr_ref, &overlay, &budget, true).await;
                (item, record, found)
            });
        }
        while let Some(joined) = lookups.join_next().await {
            let Ok((item, record, found)) = joined else {
                continue;
            };
            let id = item.id;
            if let PrLookup::Found { pr: status } = &found {
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
            let cached = match &found {
                PrLookup::Found { pr: status } => {
                    Some((Some(status.number), status.state.as_str()))
                }
                PrLookup::NoPr | PrLookup::NotGitHub => Some((None, "none")),
                PrLookup::Failed { .. } | PrLookup::Unknown => None,
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

    /// Attached docs, then those its agents wrote last (`written`), then those whose frontmatter names
    /// its branch (`branch`): a doc written through a shell redirect fires no write tool.
    pub async fn documents(&self, req: WorkstreamDocumentsRequest) -> Result<Vec<WorkstreamDoc>> {
        ensure_running(self.runtime)?;
        let item = match self.find(&req.id).await {
            Ok(item) => item,
            Err(CoreError::WorkstreamNotFound { .. }) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let project = match self.effective_for(&item.project_path).await {
            Ok(effective) => effective.mapped_name,
            Err(_) => None,
        };
        let db = self.runtime.db.clone();
        let index = self.runtime.index.clone();
        blocking(move || {
            let mut docs = db.workstream_docs(&item.id)?;
            // A doc another workstream wrote since is that one's.
            let written = db
                .docs_last_written_by(&item.id)?
                .into_iter()
                .filter(|p| index.doc_path(p).is_some())
                .map(|p| (p, "written"));
            // A branch another workstream works on or builds from leaves a doc's owner unknown.
            let shared = item.branch == item.base_branch
                || db.list_active_workstreams()?.iter().any(|w| {
                    w.id != item.id
                        && w.project_path == item.project_path
                        && (w.branch == item.branch || w.base_branch == item.branch)
                });
            let on_branch = project
                .filter(|_| !shared)
                .map(|p| index.on_branch(&p, &item.branch, &created_day(item.created_at)))
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p, "branch"));
            for (doc_path, role) in written.chain(on_branch) {
                if !docs.iter().any(|d| d.doc_path == doc_path) {
                    docs.push(WorkstreamDoc {
                        workstream_id: item.id.clone(),
                        doc_path,
                        role: role.to_string(),
                    });
                }
            }
            Ok(docs)
        })
        .await
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

    pub async fn set_base_branch(&self, id: &str, base: &str) -> Result<()> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let base = base.trim().to_string();
        if base.is_empty() {
            return Err(CoreError::WorkstreamBaseBranchEmpty);
        }
        if let Ok(gh) = self.runtime.binaries.require(BinaryKind::Gh).await {
            let pr_ref = current_pr_ref(&item);
            let cache = &self.runtime.watchers.pr_cache;
            let cached = match &pr_ref {
                Some(pr_ref) => cached_lookup(cache, &item.id, pr_ref),
                None => cached_lookup_any_ref(cache, &item.id).map(|(_, lookup)| lookup),
            }
            .and_then(|lookup| lookup.known().cloned());
            let known = match (cached, &pr_ref) {
                (Some(pr), _) => Some(pr),
                (None, Some(pr_ref)) => self.lookup_and_store(&item, pr_ref).await.known().cloned(),
                (None, None) => None,
            };
            let short = base.strip_prefix("origin/").unwrap_or(&base);
            if let Some(pr) =
                known.filter(|pr| pr.state == PrState::Open && pr.base_ref_name != short)
            {
                let overlay = self.runtime.child_env().await;
                let edited =
                    pr::edit_pr_base(&gh, pr_dir(&item), &pr.number.to_string(), &base, &overlay)
                        .await;
                let still_open = match &pr_ref {
                    Some(pr_ref) => self
                        .lookup_and_store(&item, pr_ref)
                        .await
                        .known()
                        .is_some_and(|pr| pr.state == PrState::Open),
                    None => {
                        if edited.is_ok() {
                            edit_cached_pr(cache, &item.id, |pr| {
                                short.clone_into(&mut pr.base_ref_name)
                            });
                            let _ = self.runtime.events.send(RuntimeEvent::PrStatusChanged {
                                workstream_id: item.id.clone(),
                                observed_pr_changed: false,
                            });
                        }
                        true
                    }
                };
                if let Err(e) = edited {
                    if still_open {
                        return Err(e);
                    }
                }
            }
        }
        let db = self.runtime.db.clone();
        let id = id.to_string();
        blocking(move || {
            if !item.is_folder() && item.base_branch != base {
                let worktree = Path::new(&item.worktree_path);
                if let Err(e) =
                    crate::gitops::untrack_base(worktree, &item.branch, &item.base_branch)
                {
                    tracing::warn!("{} still tracks {}: {e}", item.branch, item.base_branch);
                }
            }
            db.update_workstream_base_branch(&id, &base)
        })
        .await
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

    /// Runs one worktree watcher per workstream in `workstream_ids` and drops
    /// the rest. The PR watcher polls these more eagerly than the others.
    pub async fn watch(&self, workstream_ids: HashSet<String>) -> Result<()> {
        ensure_running(self.runtime)?;
        let mut watched = self.runtime.watchers.worktree_watch.lock().await;
        watched.retain(|id, handle| {
            let keep = workstream_ids.contains(id);
            if !keep && handle.is_some() {
                tracing::debug!("worktree watch on {id}: released");
            }
            keep
        });
        for id in workstream_ids {
            if watched.contains_key(&id) {
                continue;
            }
            // Not fatal: one client's unwatchable workstream must not fail another's watch.
            let handle = self.open_worktree_watch(&id).await.unwrap_or_else(|e| {
                tracing::warn!("worktree watch on {id} stays off: {e}");
                None
            });
            watched.insert(id, handle);
        }
        Ok(())
    }

    async fn open_worktree_watch(&self, workstream_id: &str) -> Result<Option<WatchHandle>> {
        let item = self.find(workstream_id).await?;
        let path = PathBuf::from(&item.worktree_path);
        if !path.is_dir() {
            return Ok(None);
        }

        let events = self.runtime.events.clone();
        let kick = self.runtime.watchers.pr_kick.clone();
        let watched = workstream_id.to_string();
        blocking(move || {
            let git_dir = crate::gitops::git_dir(&path).unwrap_or_else(|| path.join(".git"));
            let head = std::sync::Mutex::new(crate::gitops::linked_head_branch(&path));
            let worktree = path.clone();
            crate::index::watcher::watch_worktree(
                &path,
                &git_dir,
                Box::new(move || {
                    let now = crate::gitops::linked_head_branch(&worktree);
                    let mut seen = head.lock().unwrap_or_else(PoisonError::into_inner);
                    if now.is_some() && *seen != now {
                        kick.kick(&watched);
                    }
                    *seen = now;
                    let _ = events.send(RuntimeEvent::WorktreeChanged {
                        workstream_id: watched.clone(),
                    });
                }),
            )
        })
        .await
        .map(Some)
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
                worktree::status(&dir, &item.base_branch, item.merged_pr_head(), link)
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
        let cache = &self.runtime.watchers.pr_cache;
        let Some(pr_ref) = current_pr_ref(&item) else {
            return Ok(cached_lookup_any_ref(cache, &item.id)
                .map_or(PrLookup::Unknown, |(_, lookup)| lookup));
        };
        match cached_lookup(cache, &item.id, &pr_ref) {
            Some(PrLookup::Found { pr }) if !moved_past(&item, &pr_ref, &pr) => {
                Ok(PrLookup::Found { pr })
            }
            _ => Ok(self.lookup_and_store(&item, &pr_ref).await),
        }
    }

    async fn lookup_and_store(&self, item: &Workstream, pr_ref: &str) -> PrLookup {
        let started = std::time::Instant::now();
        let watchers = &self.runtime.watchers;
        let next = match self.runtime.binaries.require(BinaryKind::Gh).await {
            _ if not_github(item) => PrLookup::NotGitHub,
            Ok(gh) => {
                let overlay = self.runtime.child_env().await;
                workstream_pr(&gh, item, pr_ref, &overlay, &watchers.pr_budget, false).await
            }
            Err(_) => PrLookup::Failed {
                cause: PrLookupFailure::GhMissing,
                last: last_from_row(item, pr_ref),
            },
        };
        let Some((stored, changed)) = store_lookup(
            &watchers.pr_cache,
            &item.id,
            pr_ref,
            next.clone(),
            true,
            started,
        ) else {
            return cached_lookup(&watchers.pr_cache, &item.id, pr_ref).unwrap_or(next);
        };
        let recorded = match &stored {
            PrLookup::Found { pr }
                if pr.state == PrState::Open && item.observed_ref.as_deref() != Some(pr_ref) =>
            {
                self.record_open_pr(item, pr_ref, pr).await
            }
            _ => false,
        };
        if changed || recorded {
            let _ = self.runtime.events.send(RuntimeEvent::PrStatusChanged {
                workstream_id: item.id.clone(),
                observed_pr_changed: recorded,
            });
        }
        stored
    }

    async fn record_open_pr(&self, item: &Workstream, pr_ref: &str, pr: &PrStatus) -> bool {
        let db = self.runtime.db.clone();
        let (id, pr_ref, head) = (item.id.clone(), pr_ref.to_string(), pr.head_ref_oid.clone());
        let observed = ObservedPr {
            number: pr.number,
            state: pr.state,
        };
        blocking(move || db.record_pr_observation(&id, Some(observed), &pr_ref, Some(&head), None))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("recording {}'s PR failed: {e}", item.id);
                false
            })
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
            None,
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

    pub async fn pull_branch(&self, id: &str) -> Result<worktree::PullOutcome> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        worktree::pull_upstream(
            &git,
            Path::new(&item.worktree_path),
            &item.base_branch,
            &self.runtime.child_env().await,
        )
        .await
    }

    pub(crate) async fn fetch_active_remotes(&self, history: &mut FetchHistory, regained: bool) {
        let Ok(git) = self.runtime.binaries.require(BinaryKind::Git).await else {
            return;
        };
        let db = self.runtime.db.clone();
        let Ok(mut targets) = blocking(move || {
            let mut targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for item in db.list_active_workstreams()? {
                if item.is_folder() || item.status != WorkstreamStatus::Active {
                    continue;
                }
                let project = Path::new(&item.project_path);
                if let Some(remote) =
                    crate::gitops::fetch_remote_for(project, Some(&item.base_branch))
                {
                    targets.entry(item.project_path).or_default().insert(remote);
                }
            }
            Ok(targets)
        })
        .await
        else {
            return;
        };
        let now = Instant::now();
        targets.retain(|project, _| history.due(project, now, regained));
        let overlay = self.runtime.child_env().await;
        let permits = Arc::new(tokio::sync::Semaphore::new(FETCHES_AT_ONCE));
        let mut fetches = tokio::task::JoinSet::new();
        for (project, remotes) in targets {
            let (git, overlay, permits) = (git.clone(), overlay.clone(), permits.clone());
            fetches.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let remotes: Vec<String> = remotes.into_iter().collect();
                let fetched =
                    worktree::fetch_unattended(&git, Path::new(&project), &remotes, &overlay).await;
                (project, fetched)
            });
        }
        while let Some(joined) = fetches.join_next().await {
            let Ok((project, fetched)) = joined else {
                continue;
            };
            if let Err(e) = &fetched {
                tracing::debug!("background fetch in {project} failed: {e}");
            }
            history.record(project, Instant::now(), fetched.is_ok());
        }
    }

    pub async fn fetch_remote(&self, id: &str) -> Result<()> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        self.runtime.watchers.pr_kick.kick(id);
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        worktree::fetch_remote(
            &git,
            Path::new(&item.worktree_path),
            Some(&item.base_branch),
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
        let started = std::time::Instant::now();
        let item = self.find(id).await?;
        if item.is_folder() {
            return Err(CoreError::WorkstreamFolderNoGit);
        }
        let gh = self.runtime.binaries.require(BinaryKind::Gh).await?;
        let budget = &self.runtime.watchers.pr_budget;
        if let Some(resumes) = budget.held(now_ms(), false) {
            let minutes = ((resumes - now_ms()) / 60_000.0).ceil().max(1.0);
            return Err(pr_not_found(
                pr_number,
                Some(CoreError::Exec {
                    program: "gh".into(),
                    exit: None,
                    stderr_tail: format!(
                        "GitHub API quota is low; lookups resume in {minutes} min"
                    ),
                }),
            ));
        }
        let pr_ref = pr_number.to_string();
        let status =
            match pr::pr_status(&gh, pr_dir(&item), &pr_ref, &self.runtime.child_env().await).await
            {
                Ok(answer) => {
                    if let Some(quota) = answer.quota {
                        budget.record(quota);
                    }
                    answer.pr.ok_or_else(|| pr_not_found(pr_number, None))?
                }
                Err(e) => {
                    failure_cause(&e, budget);
                    return Err(pr_not_found(pr_number, Some(e)));
                }
            };
        self.set_pr(id, Some(pr_number)).await?;
        self.record_and_settle(&item, &pr_ref, &status).await?;
        store_lookup(
            &self.runtime.watchers.pr_cache,
            id,
            &pr_ref,
            PrLookup::Found { pr: status.clone() },
            true,
            started,
        );
        let _ = self.runtime.events.send(RuntimeEvent::PrStatusChanged {
            workstream_id: id.to_string(),
            observed_pr_changed: true,
        });
        Ok(Some(status))
    }

    async fn record_and_settle(
        &self,
        item: &Workstream,
        pr_ref: &str,
        status: &PrStatus,
    ) -> Result<()> {
        let (db, wid, reference) = (self.runtime.db.clone(), item.id.clone(), pr_ref.to_string());
        let observed = ObservedPr {
            number: status.number,
            state: status.state,
        };
        let head = status.head_ref_oid.clone();
        blocking(move || {
            db.record_pr_observation(&wid, Some(observed), &reference, Some(&head), None)
        })
        .await?;
        if status.state != PrState::Open && has_own_pr(item) {
            doc_status::settle_workstream(
                self.runtime.db.clone(),
                &self.runtime.config,
                &item.id,
                &item.project_path,
                status.number,
                status.state,
            )
            .await;
        }
        Ok(())
    }

    pub async fn merge_pr(&self, id: &str, method: MergeMethod) -> Result<()> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let gh = self.runtime.binaries.require(BinaryKind::Gh).await?;
        let auto = self.runtime.settings.get().await.git.automerge;
        let pr_ref = current_pr_ref(&item);
        let cache = &self.runtime.watchers.pr_cache;
        let lookup = match &pr_ref {
            Some(pr_ref) => Some(match cached_lookup(cache, &item.id, pr_ref) {
                Some(lookup) => lookup,
                None => self.lookup_and_store(&item, pr_ref).await,
            }),
            None => cached_lookup_any_ref(cache, &item.id).map(|(_, lookup)| lookup),
        };
        let target = match lookup.as_ref().and_then(PrLookup::known) {
            Some(pr) if !pr.url.is_empty() => pr.url.clone(),
            Some(pr) => pr.number.to_string(),
            None => pr_ref.clone().ok_or(CoreError::PrUnknownWhileDetached)?,
        };
        pr::merge_pr(
            &gh,
            Path::new(&item.worktree_path),
            &target,
            method,
            auto,
            &self.runtime.child_env().await,
        )
        .await?;
        if let Some(pr_ref) = pr_ref {
            self.lookup_and_store(&item, &pr_ref).await;
        }
        Ok(())
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

    /// `None` when the workstream has no app-made branch or `title` wouldn't change it.
    pub async fn preview_branch_rename(
        &self,
        id: &str,
        title: &str,
    ) -> Result<Option<BranchRename>> {
        ensure_running(self.runtime)?;
        let item = self.find(id).await?;
        let title = title.trim().to_string();
        blocking(move || Ok(branch_rename(&item, &title))).await
    }

    /// Renames a local, PR-less branch to follow `title`; the slug and folders keep their names.
    pub async fn rename_branch(&self, id: &str, title: &str) -> Result<Workstream> {
        ensure_running(self.runtime)?;
        let peek = self.find(id).await?;
        let _guard = self
            .runtime
            .workstream_locks
            .lock_pair(&peek.project_path, id)
            .await;
        let item = self.find(id).await?;
        let (probe, title) = (item.clone(), title.trim().to_string());
        let Some(rename) = blocking(move || Ok(branch_rename(&probe, &title))).await? else {
            return Ok(item);
        };
        if rename.blocked.is_some() {
            return Err(CoreError::BranchRenameBlocked {
                branch: item.branch,
            });
        }
        let git = self.runtime.binaries.require(BinaryKind::Git).await?;
        worktree::rename_branch(
            &git,
            Path::new(&item.worktree_path),
            &item.branch,
            &rename.branch,
            &self.runtime.child_env().await,
        )
        .await?;
        let db = self.runtime.db.clone();
        let (wid, branch) = (item.id.clone(), rename.branch.clone());
        blocking(move || db.rename_workstream_branch(&wid, &branch)).await?;
        let _ = self
            .runtime
            .events
            .send(RuntimeEvent::WorkstreamBranchChanged {
                workstream_id: item.id.clone(),
                branch: rename.branch,
                previous: item.branch,
            });
        self.find(id).await
    }

    /// The stored stage only moves forward, whether a session started or a tab's mode changed.
    pub async fn advance_stage(&self, id: &str, stage: Stage) -> Result<()> {
        let item = self.find(id).await?;
        let db = self.runtime.db.clone();
        blocking(move || {
            let current = (!item.is_folder()).then(|| current_pr_ref(&item)).flatten();
            db.advance_workstream_stage(&item.id, stage, current.as_deref())
        })
        .await
    }

    pub async fn update(&self, req: UpdateWorkstreamRequest) -> Result<Workstream> {
        ensure_running(self.runtime)?;
        self.find(&req.id).await?;
        if let Some(stage) = req.stage {
            self.advance_stage(&req.id, stage).await?;
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
            reused: false,
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
            if let Err(e) = worktree::fetch_remote(&git, &repo, Some(&branch), &overlay).await {
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
        self.create_logged(req, None).await
    }

    pub async fn create_for_claude(
        &self,
        identity: &AgentIdentity,
        name: String,
        cwd: &str,
        session_id: Option<String>,
    ) -> Result<CreateWorkstreamResult> {
        self.runtime.agent_control().require_full(identity).await?;
        let project = identity.project_path.clone();
        let (from, to) = (PathBuf::from(cwd), PathBuf::from(&project));
        if !blocking(move || Ok(same_repo(&from, &to))).await? {
            return Err(CoreError::AgentControlDenied {
                detail: format!("{cwd} is not a checkout of {project}"),
            });
        }
        let req = CreateWorkstreamRequest {
            project_path: project,
            title: name.clone(),
            base: None,
            mode: WorktreeMode::Isolated,
            branch: None,
            branch_prefix: None,
            parent_workstream_id: Some(identity.workstream_id.clone()),
        };
        let claude = ClaudeName {
            name,
            harness_session_id: session_id,
        };
        self.create_logged(req, Some(claude)).await
    }

    pub async fn claude_removed(
        &self,
        identity: &AgentIdentity,
        worktree_path: String,
    ) -> Result<Option<Workstream>> {
        self.runtime.agent_control().require_full(identity).await?;
        let (db, project) = (self.runtime.db.clone(), identity.project_path.clone());
        let found =
            blocking(move || db.active_claude_worktree_at(&project, &worktree_path)).await?;
        Ok(found.map(|claude| claude.workstream))
    }

    /// A stopped subagent's workstream, only if it left the worktree exactly as it was made.
    pub async fn claude_subagent_stopped(
        &self,
        identity: &AgentIdentity,
        agent_id: String,
    ) -> Result<Option<Workstream>> {
        self.runtime.agent_control().require_full(identity).await?;
        let (db, project) = (self.runtime.db.clone(), identity.project_path.clone());
        blocking(move || {
            let found = db.active_claude_worktree(&project, &format!("agent-{agent_id}"))?;
            Ok(found.filter(untouched).map(|claude| claude.workstream))
        })
        .await
    }

    async fn create_logged(
        &self,
        req: CreateWorkstreamRequest,
        claude: Option<ClaudeName>,
    ) -> Result<CreateWorkstreamResult> {
        let started = std::time::Instant::now();
        let what = format!(
            "create {:?} in {} ({:?})",
            req.title, req.project_path, req.mode
        );
        let result = self.create_inner(req, claude).await;
        match &result {
            Ok(created) if created.reused => tracing::info!(
                "{what}: claude's worktree {} already exists",
                created.item.worktree_path
            ),
            Ok(created) => {
                tracing::info!(
                    "{what}: workstream {} on {} at {} in {:?}",
                    created.item.id,
                    created.item.branch,
                    created.item.worktree_path,
                    started.elapsed()
                );
                self.announce_list_change(&created.item.project_path);
            }
            Err(e) => tracing::warn!("{what} failed after {:?}: {e}", started.elapsed()),
        }
        result
    }

    fn announce_list_change(&self, project_path: &str) {
        let _ = self.runtime.events.send(RuntimeEvent::WorkstreamsChanged {
            project_path: project_path.to_string(),
        });
    }

    async fn create_inner(
        &self,
        req: CreateWorkstreamRequest,
        claude: Option<ClaudeName>,
    ) -> Result<CreateWorkstreamResult> {
        ensure_running(self.runtime)?;
        let CreateWorkstreamRequest {
            project_path,
            title,
            base,
            mode,
            branch,
            branch_prefix: branch_prefix_req,
            parent_workstream_id,
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
            if claude.is_some() {
                return Err(CoreError::AgentControlDenied {
                    detail: format!("{project_path} is a folder project, with no worktrees"),
                });
            }
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
        if let Some(claude) = &claude {
            let (pp, name, db) = (
                project_path.clone(),
                claude.name.clone(),
                self.runtime.db.clone(),
            );
            if let Some(found) = blocking(move || db.active_claude_worktree(&pp, &name)).await? {
                return Ok(CreateWorkstreamResult {
                    item: found.workstream,
                    link: None,
                    seed: None,
                    reused: true,
                });
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
        let mut branch_prefix = match picked {
            Some(p) => p,
            None => self
                .runtime
                .settings
                .get()
                .await
                .git
                .effective_branch_prefix(),
        };
        let claude_slug = match &claude {
            Some(claude) => {
                let (db, pp) = (self.runtime.db.clone(), project_path.clone());
                let stem = worktree::slug_stem(&claude.name);
                branch_prefix.push_str("claude/");
                Some(blocking(move || free_slug(&db, &pp, &stem)).await?)
            }
            None => None,
        };
        if matches!(create_mode, CreateMode::Isolated) {
            branch_prefix::validate_prefix(&branch_prefix)?;
        }
        let base_override = base.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        let created = self
            .runtime
            .worktrees
            .create_named(
                &git,
                &CreateSpec {
                    project_repo: PathBuf::from(&project_path),
                    mapped_name: mapped.clone(),
                    title: title.clone(),
                    base_override,
                    branch_prefix,
                    mode: create_mode,
                },
                claude_slug.as_deref(),
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
        let item = blocking(move || {
            let item = db.insert_child_workstream(new_item, parent_workstream_id)?;
            if let Some(claude) = claude {
                db.insert_claude_worktree(NewClaudeWorktree {
                    workstream_id: item.id.clone(),
                    project_path: item.project_path.clone(),
                    agent_id: claude.name.strip_prefix("agent-").map(str::to_string),
                    name: claude.name,
                    harness_session_id: claude.harness_session_id,
                    created_head: head_of(Path::new(&item.worktree_path)),
                })?;
            }
            Ok(item)
        })
        .await
        .inspect_err(|e| {
            tracing::error!("not recorded, so {worktree_left} is left on disk untracked: {e}")
        })?;

        Ok(CreateWorkstreamResult {
            item,
            link: Some(link),
            seed,
            reused: false,
        })
    }

    pub async fn archive(&self, req: ArchiveWorkstreamRequest) -> Result<Workstream> {
        let what = format!("archive workstream {}", req.id);
        let archived = logged(&what, self.archive_inner(req)).await?;
        self.announce_list_change(&archived.project_path);
        Ok(archived)
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
                .end_quietly_and_await_exit(&session.id)
                .await;
        }
        self.runtime.terminals.remove_for_workstream(&id).await;
        self.runtime.scripts.clear_workstream(&id).await;

        // After the kills: no agent of this workstream can write a doc past this point.
        let (pr_ref, pr) = self
            .archive_pr_status(&item, moved_head(&item).unwrap_or_else(|| item.pr_ref()))
            .await
            .unzip();
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
            let found = pr.map(|s| (s.number, s.state));
            blocking(move || {
                db.archive_workstream(&aid)?;
                cache_found_pr(&db, &aid, found)
            })
            .await?;
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
            // A PR found on a renamed or switched branch says nothing about the app's branch.
            && pr_ref.is_some_and(|r| r == item.pr_ref())
            && pr.as_ref().is_some_and(|s| s.state == pr::PrState::Merged)
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

        // Windows refuses to move a directory a watcher holds open. Holding the lock through
        // the move stops `watch` re-arming; the UI itself only re-arms on navigation.
        let mut watch = self.runtime.watchers.worktree_watch.lock().await;
        let disarmed = watch.get_mut(&id).and_then(Option::take).is_some();
        let moved = match self
            .runtime
            .worktrees
            .archive(&git, project_path, worktree_path, &archive_path)
            .await
        {
            Ok(moved) => moved,
            Err(e) => {
                if disarmed {
                    match self.open_worktree_watch(&id).await {
                        Ok(handle) => {
                            watch.insert(id.clone(), handle);
                        }
                        Err(rearm) => tracing::warn!("worktree watch on {id} stays off: {rearm}"),
                    }
                }
                return Err(e);
            }
        };
        drop(watch);
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
        let merged = pr
            .as_ref()
            .filter(|s| s.state == pr::PrState::Merged)
            .map(|s| s.head_ref_oid.clone())
            .or_else(|| item.merged_pr_head().map(String::from));
        let db = self.runtime.db.clone();
        let aid = id.clone();
        let found = pr.map(|s| (s.number, s.state));
        blocking(move || {
            // Scanned once: nothing writes to an archive after this.
            let scan = scan_dir.map(|dir| worktree::scan_archive(&dir, &base, merged.as_deref()));
            db.archive_workstream(&aid)?;
            cache_found_pr(&db, &aid, found)?;
            match scan {
                Some(scan) => db.cache_archive_scan(&aid, &scan, now_ms()),
                None => Ok(()),
            }
        })
        .await?;
        self.find(&id).await
    }

    async fn archive_pr_status(
        &self,
        item: &Workstream,
        pr_ref: String,
    ) -> Option<(String, pr::PrStatus)> {
        if !has_own_pr(item) {
            return None;
        }
        let cache = &self.runtime.watchers.pr_cache;
        if current_pr_ref(item).is_none() {
            let (cached_ref, lookup) = cached_lookup_any_ref(cache, &item.id)?;
            return Some((cached_ref, lookup.known()?.clone()));
        }
        let gh = self.runtime.binaries.require(BinaryKind::Gh).await.ok()?;
        let overlay = self.runtime.child_env().await;
        let budget = &self.runtime.watchers.pr_budget;
        let pr = match workstream_pr(&gh, item, &pr_ref, &overlay, budget, false).await {
            PrLookup::Failed { .. } => {
                cached_lookup(cache, &item.id, &pr_ref).and_then(|lookup| lookup.known().cloned())
            }
            lookup => lookup.known().cloned(),
        };
        pr.map(|pr| (pr_ref, pr))
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

    /// A removed project's archives, purged as the expiry sweep does: never the remote, local branches only with `-d`.
    pub(crate) async fn purge_project_archives(&self, project_path: &str) -> Result<()> {
        let items = self.list_archived().await?;
        for item in items.into_iter().filter(|w| w.project_path == project_path) {
            let _guard = self
                .runtime
                .workstream_locks
                .lock_pair(&item.project_path, &item.id)
                .await;
            let Ok(item) = self.find(&item.id).await else {
                continue;
            };
            if item.status != WorkstreamStatus::Archived {
                continue;
            }
            let cleanup = self.branch_cleanup(&item, BranchCleanup::SafeLocal).await;
            self.purge_locked(&item, cleanup).await?;
        }
        Ok(())
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

pub(crate) fn pr_dir(item: &Workstream) -> &Path {
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

/// The branch is `<prefix><slug>`, or `<prefix><title stem>-<suffix>` once renamed with its title;
/// a branch matching neither (named at create time) is never offered.
fn branch_rename(item: &Workstream, title: &str) -> Option<BranchRename> {
    if !item.app_owns_branch() || item.status != WorkstreamStatus::Active {
        return None;
    }
    let (_, suffix) = item.slug.rsplit_once('-')?;
    let titled = format!("{}-{suffix}", worktree::slug_stem(&item.title));
    let prefix = item
        .branch
        .strip_suffix(item.slug.as_str())
        .or_else(|| item.branch.strip_suffix(titled.as_str()))?;
    let branch = format!("{prefix}{}-{suffix}", worktree::slug_stem(title));
    if branch == item.branch {
        return None;
    }
    let worktree = Path::new(&item.worktree_path);
    let pushed = crate::gitops::list_remote_branches(worktree)
        .unwrap_or_default()
        .iter()
        .any(|r| crate::gitops::local_name_for_remote(r) == Some(item.branch.as_str()));
    let blocked = if item.pr_number.is_some() {
        Some(BranchRenameBlock::HasPr)
    } else if crate::gitops::linked_head_branch(worktree).as_deref() != Some(item.branch.as_str()) {
        Some(BranchRenameBlock::Moved)
    } else if pushed {
        Some(BranchRenameBlock::Pushed)
    } else if crate::gitops::list_local_branches(worktree)
        .unwrap_or_default()
        .contains(&branch)
    {
        Some(BranchRenameBlock::Taken)
    } else {
        None
    };
    Some(BranchRename { branch, blocked })
}

/// What `gh` should look up: the attached number, else the branch the worktree is really on.
pub(crate) fn pr_lookup_ref(item: &Workstream) -> String {
    moved_head(item).unwrap_or_else(|| item.pr_ref())
}

pub(crate) async fn workstream_pr(
    gh: &Path,
    item: &Workstream,
    pr_ref: &str,
    overlay: &[(String, String)],
    budget: &PrBudget,
    background: bool,
) -> PrLookup {
    if not_github(item) {
        return PrLookup::NotGitHub;
    }
    if let Some(resumes) = budget.held(now_ms(), background) {
        return PrLookup::Failed {
            cause: PrLookupFailure::RateLimited {
                resumes_at: Some(resumes),
            },
            last: last_from_row(item, pr_ref),
        };
    }
    match pr::pr_status(gh, pr_dir(item), pr_ref, overlay).await {
        Ok(answer) => {
            if let Some(quota) = answer.quota {
                budget.record(quota);
            }
            match answer.pr {
                Some(pr) if !moved_past(item, pr_ref, &pr) && owns(item, pr_ref, &pr) => {
                    PrLookup::Found { pr }
                }
                _ => PrLookup::NoPr,
            }
        }
        Err(e) if pr::gh_found_no_github_remote(&e.to_string()) => PrLookup::NotGitHub,
        Err(e) => {
            tracing::debug!("PR lookup for {pr_ref} failed: {e}");
            PrLookup::Failed {
                cause: failure_cause(&e, budget),
                last: last_from_row(item, pr_ref),
            }
        }
    }
}

fn not_github(item: &Workstream) -> bool {
    !crate::gitops::has_github_remote(pr_dir(item))
}

fn owns(item: &Workstream, pr_ref: &str, pr: &PrStatus) -> bool {
    item.pr_number.is_some()
        || pr.state == PrState::Open
        || observed_on_row(item, pr_ref, pr.number).is_some()
        || crate::gitops::pr_head_is_ours(pr_dir(item), pr_ref, &item.base_branch, &pr.head_ref_oid)
            .unwrap_or(true)
}

fn observed_on_row(item: &Workstream, pr_ref: &str, number: u32) -> Option<ObservedPr> {
    item.observed_pr
        .filter(|o| o.number == number && item.observed_ref.as_deref() == Some(pr_ref))
}

fn moved_past(item: &Workstream, pr_ref: &str, pr: &PrStatus) -> bool {
    item.pr_number.is_none()
        && pr.state != PrState::Open
        && observed_on_row(item, pr_ref, pr.number).is_some_and(|o| o.state != PrState::Open)
        && crate::gitops::has_work_past(pr_dir(item), pr_ref, &pr.head_ref_oid, None)
}

fn last_from_row(item: &Workstream, pr_ref: &str) -> Option<PrStatus> {
    let observed = item
        .observed_pr
        .filter(|o| o.state != PrState::Open && item.observed_ref.as_deref() == Some(pr_ref))?;
    let url = crate::gitops::base_remote_web_url(pr_dir(item))
        .map(|web| format!("{web}/pull/{}", observed.number))
        .unwrap_or_default();
    Some(PrStatus::recorded(
        observed.number,
        observed.state,
        url,
        item.observed_pr_head.clone().unwrap_or_default(),
    ))
}

/// The ref to look up, or `None` while HEAD has left its branch (a rebase, or the base branch):
/// `pr_lookup_ref` would fall back to the app's branch, which would read as the worktree moving.
pub(crate) fn current_pr_ref(item: &Workstream) -> Option<String> {
    let looked_up = pr_lookup_ref(item);
    let elsewhere = item
        .observed_ref
        .as_deref()
        .is_some_and(|seen| seen != looked_up);
    (!(elsewhere && head_off_branch(item))).then_some(looked_up)
}

/// A worktree that moved since its last observation has no PR of its own to show yet.
fn without_moved_observation(mut item: Workstream) -> Workstream {
    if item.status == WorkstreamStatus::Active
        && item.observed_ref.is_some()
        && current_pr_ref(&item).is_some_and(|now| item.observed_ref.as_deref() != Some(&now))
    {
        item.observed_pr = None;
    }
    item
}

fn head_off_branch(item: &Workstream) -> bool {
    if item.pr_number.is_some() || !item.app_owns_branch() {
        return false;
    }
    let dir = head_dir(item);
    crate::gitops::linked_head_detached(&dir)
        || crate::gitops::linked_head_branch(&dir).is_some_and(|head| on_base(item, &head))
}

fn head_dir(item: &Workstream) -> PathBuf {
    if item.status == WorkstreamStatus::Archived {
        archive_dir_for(Path::new(&item.project_path), &item.slug)
    } else {
        PathBuf::from(&item.worktree_path)
    }
}

fn on_base(item: &Workstream, head: &str) -> bool {
    head == item.base_branch || item.base_branch.strip_prefix("origin/") == Some(head)
}

/// An isolated worktree's HEAD once renamed or switched off the app's branch. The row keeps
/// the app's branch because archive may delete it; only PR lookups follow HEAD.
fn moved_head(item: &Workstream) -> Option<String> {
    if item.pr_number.is_some() || !item.app_owns_branch() {
        return None;
    }
    let head = crate::gitops::linked_head_branch(&head_dir(item))?;
    (head != item.branch && !on_base(item, &head)).then_some(head)
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

/// Archiving resets the record, so what it just learned goes back in; a miss waits for the sweep.
fn cache_found_pr(db: &Db, id: &str, found: Option<(u32, pr::PrState)>) -> Result<()> {
    match found {
        Some((number, state)) => {
            db.cache_archive_pr(id, Some((Some(number), state.as_str())), now_ms())
        }
        None => Ok(()),
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
    s = s
        .strip_prefix("GraphQL: ")
        .or_else(|| s.strip_prefix("gh: "))
        .unwrap_or(s);
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

/// The day before a workstream's creation, `YYYY-MM-DD` in UTC: a doc's date is its author's local day.
fn created_day(created_at_ms: f64) -> String {
    crate::store::utc_date(((created_at_ms / 1000.0) as u64).saturating_sub(86_400))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_worktree_is_not_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        let tree = repo
            .find_tree(repo.index().unwrap().write_tree().unwrap())
            .unwrap();
        let head = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let workstream = Db::open_in_memory()
            .unwrap()
            .insert_workstream(NewWorkstream {
                project_path: "/repo".into(),
                title: "t".into(),
                slug: "t".into(),
                branch: "t".into(),
                worktree_path: dir.path().display().to_string(),
                base_branch: "HEAD".into(),
                worktree_mode: WorktreeMode::Isolated,
            })
            .unwrap();
        let claude = ClaudeWorktree {
            workstream,
            created_head: Some(head.to_string()),
        };
        assert!(untouched(&claude));
        std::fs::write(dir.path().join(".git/index"), "not an index").unwrap();
        assert!(!untouched(&claude));
    }

    #[test]
    fn a_refocus_fetches_past_a_failure_but_at_most_every_30_seconds() {
        let mut history = FetchHistory::default();
        let start = Instant::now();
        history.record("/p".into(), start, false);
        assert!(!history.due("/p", start + Duration::from_secs(60), false));
        assert!(!history.due("/p", start + Duration::from_secs(29), true));
        assert!(history.due("/p", start + REFOCUS_FETCH_GAP, true));
        assert!(history.due("/p", start + FETCH_BACKOFF, false));
        assert!(history.due("/other", start, true) && history.due("/other", start, false));
    }

    #[test]
    fn clean_gh_stderr_strips_graphql_decoration() {
        assert_eq!(
            clean_gh_stderr(
                "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)"
            ),
            "Could not resolve to a PullRequest with the number of 999."
        );
        assert_eq!(
            clean_gh_stderr("gh: Could not resolve to a PullRequest with the number of 999."),
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
