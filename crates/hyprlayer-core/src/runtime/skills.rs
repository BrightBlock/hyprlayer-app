use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_models::{SkillScope, SkillSourceKind, SkillTarget};

use crate::error::{CoreError, Result};
use crate::process::binaries::{parse_semver, BinaryKind};
use crate::skill_sources::fetch::{self, Git, Resolved};
use crate::skill_sources::install::{self, StagedSource};
use crate::skill_sources::layout::{self, Layout};
use crate::skill_sources::lock::{self, LockEntry, SkillLock};
use crate::skill_sources::skills_sh::{
    self, SkillLeaderboardPage, SkillSearchResult, SkillsShView,
};
use crate::skill_sources::{
    github, skillfish, SkillInstallPreview, SkillInstallReport, SkillLockStatus, SkillRestoreEntry,
    SkillRestoreOutcome, SkillRestoreReport, SkillSourceInput, SkillSourceRef, SkillUpdate,
    SkillfishCandidate,
};
use crate::skills::{SkillFile, SkillHome};

use super::ports::ProviderHttpRequest;
use super::HyprlayerRuntime;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
const SEARCH_MAX_BYTES: usize = 512 * 1024;
const SEARCH_LIMIT: u32 = 20;
const SEARCH_MIN_CHARS: usize = 2;

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

pub struct SkillService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn skills(&self) -> SkillService<'_> {
        SkillService { runtime: self }
    }
}

impl SkillService<'_> {
    pub fn skill_home(&self) -> SkillHome {
        SkillHome {
            claude: Some(self.runtime.paths.claude_config_dir.clone()),
            home: Some(self.runtime.environment.home_dir.clone()),
        }
    }

    /// Beside the CLI's config file, as `AgentService::store_root` derives it (ADR-0002).
    fn config_dir(&self) -> Option<&Path> {
        self.runtime.paths.hyprlayer_config_file.parent()
    }

    fn bundle_store(&self) -> Option<PathBuf> {
        self.config_dir().map(|dir| dir.join("agents"))
    }

    fn global_store(&self) -> Option<PathBuf> {
        self.config_dir().map(|dir| dir.join("skills"))
    }

    pub async fn list(&self, project_path: Option<String>) -> Result<Vec<SkillFile>> {
        ensure_running(self.runtime)?;
        let home = self.skill_home();
        let (bundle_store, global_store) = (self.bundle_store(), self.global_store());
        blocking(move || {
            Ok(crate::skills::list_skill_files(
                project_path.as_deref().map(Path::new),
                &home,
                bundle_store.as_deref(),
                global_store.as_deref(),
            ))
        })
        .await
    }

    pub async fn read_file(&self, project_path: Option<String>, path: String) -> Result<String> {
        ensure_running(self.runtime)?;
        let home = self.skill_home();
        blocking(move || {
            crate::skills::read_skill_file(
                project_path.as_deref().map(Path::new),
                &home,
                Path::new(&path),
            )
        })
        .await
    }

    pub async fn read_asset(
        &self,
        project_path: Option<String>,
        skill_path: String,
        rel_path: String,
    ) -> Result<crate::skills::SkillAssetText> {
        ensure_running(self.runtime)?;
        let home = self.skill_home();
        blocking(move || {
            crate::skills::read_skill_asset(
                project_path.as_deref().map(Path::new),
                &home,
                Path::new(&skill_path),
                &rel_path,
            )
        })
        .await
    }

    fn staging_root(&self) -> PathBuf {
        self.runtime.paths.app_data_dir.join("skill-staging")
    }

    fn layout(&self, scope: SkillScope, project: Option<&Path>) -> Result<Layout> {
        match (scope, project) {
            (SkillScope::Project, Some(checkout)) => Ok(Layout::project(checkout)),
            (SkillScope::Project, None) => Err(CoreError::InvalidPath {
                path: String::new(),
            }),
            (SkillScope::Global, _) => self
                .global_store()
                .and_then(|store| Layout::global(&store, &self.skill_home()))
                .ok_or_else(|| CoreError::InvalidPath {
                    path: String::new(),
                }),
        }
    }

    /// Canonical, so `/var/...` and `/private/var/...` share one mutex.
    fn scope_lock_key(&self, scope: SkillScope, project: Option<&Path>) -> Result<PathBuf> {
        let canonical =
            |dir: &Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        match (scope, project) {
            (SkillScope::Project, Some(checkout)) => {
                Ok(canonical(checkout).join(".agents/hyprlayer"))
            }
            (SkillScope::Global, _) => {
                let config = self.config_dir().ok_or_else(|| CoreError::InvalidPath {
                    path: String::new(),
                })?;
                Ok(canonical(config).join("skills"))
            }
            (SkillScope::Project, None) => Err(CoreError::InvalidPath {
                path: String::new(),
            }),
        }
    }

    /// Creates and sweeps the staging root; returns it.
    async fn prepare_staging(&self) -> Result<PathBuf> {
        let staging_root = self.staging_root();
        let sweep = staging_root.clone();
        blocking(move || {
            std::fs::create_dir_all(&sweep)?;
            install::sweep_staging(&sweep);
            Ok(())
        })
        .await?;
        Ok(staging_root)
    }

    pub async fn preview_install(
        &self,
        project_path: Option<String>,
        source: SkillSourceInput,
    ) -> Result<SkillInstallPreview> {
        ensure_running(self.runtime)?;
        let staging_root = self.prepare_staging().await?;
        let checkout = project_checkout(project_path)?;
        let parsed = match source {
            SkillSourceInput::Url { url } => github::parse(&url)?,
            SkillSourceInput::SkillsSh { source, skill_id } => {
                let skill_id = skill_id.trim();
                let invalid = || CoreError::SkillSourceInvalid {
                    input: source.clone(),
                };
                if skill_id.is_empty() {
                    return Err(invalid());
                }
                SkillSourceRef {
                    kind: SkillSourceKind::SkillsSh,
                    repo: github::parse_owner_repo(&source).ok_or_else(invalid)?,
                    ref_and_path: None,
                    path: None,
                    name_hint: Some(skill_id.to_string()),
                }
            }
        };
        self.runtime.binaries.require(BinaryKind::Git).await?;

        let token = uuid::Uuid::new_v4().to_string();
        let token_dir = staging_root.join(&token);
        std::fs::create_dir_all(&token_dir)?;
        let preview = self
            .stage(&token, &token_dir, &parsed, checkout.as_deref())
            .await;
        if preview.is_err() {
            let _ = std::fs::remove_dir_all(&token_dir);
        }
        preview
    }

    async fn stage(
        &self,
        token: &str,
        token_dir: &Path,
        parsed: &SkillSourceRef,
        checkout: Option<&Path>,
    ) -> Result<SkillInstallPreview> {
        let mut preview = self
            .fetch_stage(
                token,
                token_dir,
                &parsed.repo,
                parsed.kind,
                Pick::Link(parsed),
                None,
            )
            .await?;
        let project = checkout.map(Layout::project);
        let global = self.layout(SkillScope::Global, None).ok();
        let dir = token_dir.to_path_buf();
        blocking(move || {
            let scopes: Vec<(&Layout, SkillLock)> = project
                .iter()
                .chain(global.iter())
                .map(|layout| (layout, lock::read(&layout.lock).unwrap_or_default()))
                .collect();
            let scopes: Vec<(&Layout, &SkillLock)> = scopes.iter().map(|(l, k)| (*l, k)).collect();
            preview.conflicts = layout::conflicts(&preview.name, &scopes);
            install::write_record(&dir, &preview, None)?;
            Ok(preview)
        })
        .await
    }

    /// Fetches, copies out and hashes into `token_dir`; the caller records the preview.
    async fn fetch_stage(
        &self,
        token: &str,
        token_dir: &Path,
        repo: &str,
        kind: SkillSourceKind,
        pick: Pick<'_>,
        folder: Option<&str>,
    ) -> Result<SkillInstallPreview> {
        let status = self.runtime.binaries.status(BinaryKind::Git).await;
        let git_bin = self.runtime.binaries.require(BinaryKind::Git).await?;
        let filter = status
            .version
            .as_deref()
            .and_then(parse_semver)
            .is_some_and(|v| v >= [2, 25, 0]);
        let overlay = self.runtime.environment.overlay_env();
        let (clone_url, fetched_from) = github::git_url(&self.runtime.environment, repo);
        let git = Git {
            bin: &git_bin,
            overlay: &overlay,
            staging: token_dir,
        };
        let (resolved, pinned_path) = match pick {
            Pick::Link(parsed) => (fetch::resolve(&git, &clone_url, parsed).await?, None),
            Pick::Pinned {
                git_ref,
                commit,
                path,
            } => (
                Resolved {
                    git_ref,
                    commit,
                    path: (!path.is_empty()).then(|| path.clone()),
                    name_hint: None,
                },
                Some(path),
            ),
        };
        let fetched = fetch::fetch_into(&git, &clone_url, &resolved, token_dir, filter).await?;
        if pinned_path.is_some_and(|path| path != fetched.path) {
            return Err(CoreError::SkillNotFoundInSource {
                detail: "The skill moved in its source.".into(),
            });
        }

        let repo = repo.to_string();
        let token_owned = token.to_string();
        let dir = token_dir.to_path_buf();
        let folder = folder.map(str::to_string);
        blocking(move || {
            install::stage_preview(
                &dir,
                &fetched,
                &StagedSource {
                    kind,
                    token: &token_owned,
                    repo: &repo,
                    git_ref: &resolved.git_ref,
                    commit: &resolved.commit,
                    fetched_from,
                    folder: folder.as_deref(),
                },
            )
        })
        .await
    }

    /// The remote's refs, read through a git that never prompts.
    async fn remote(&self, repo: &str) -> Result<fetch::Remote> {
        let staging_root = self.staging_root();
        std::fs::create_dir_all(&staging_root)?;
        let git_bin = self.runtime.binaries.require(BinaryKind::Git).await?;
        let overlay = self.runtime.environment.overlay_env();
        let (clone_url, _) = github::git_url(&self.runtime.environment, repo);
        let git = Git {
            bin: &git_bin,
            overlay: &overlay,
            staging: &staging_root,
        };
        fetch::ls_remote(&git, &clone_url).await
    }

    /// Project first (when a checkout is given), then global.
    fn layouts(&self, checkout: Option<&Path>) -> Vec<Layout> {
        checkout
            .map(Layout::project)
            .into_iter()
            .chain(self.layout(SkillScope::Global, None).ok())
            .collect()
    }

    pub async fn lock_status(&self, project_path: Option<String>) -> Result<Vec<SkillLockStatus>> {
        ensure_running(self.runtime)?;
        let layouts = self.layouts(project_checkout(project_path)?.as_deref());
        blocking(move || {
            let mut out = Vec::new();
            for layout in &layouts {
                let lock = lock::read(&layout.lock).unwrap_or_default();
                for (folder, entry) in &lock.skills {
                    if SkillSourceKind::parse(&entry.source).is_none() {
                        continue;
                    }
                    out.push(SkillLockStatus {
                        name: folder.clone(),
                        scope: layout.scope,
                        state: layout.lock_state(folder, entry, &lock),
                    });
                }
            }
            Ok(out)
        })
        .await
    }

    pub async fn check_updates(&self, project_path: Option<String>) -> Result<Vec<SkillUpdate>> {
        ensure_running(self.runtime)?;
        let layouts = self.layouts(project_checkout(project_path)?.as_deref());
        let installed = blocking(move || {
            let mut out = Vec::new();
            for layout in &layouts {
                let lock = lock::read(&layout.lock).unwrap_or_default();
                for (folder, entry) in lock.skills {
                    if SkillSourceKind::parse(&entry.source).is_some() {
                        out.push((layout.scope, folder, entry));
                    }
                }
            }
            Ok(out)
        })
        .await?;

        if installed.is_empty() {
            return Ok(Vec::new());
        }
        self.runtime.binaries.require(BinaryKind::Git).await?;
        let mut remotes: HashMap<String, Option<fetch::Remote>> = HashMap::new();
        for (_, _, entry) in &installed {
            if !remotes.contains_key(&entry.repo) {
                let remote = self.remote(&entry.repo).await.ok();
                remotes.insert(entry.repo.clone(), remote);
            }
        }
        Ok(installed
            .into_iter()
            .map(|(scope, name, entry)| {
                let remote = remotes.get(&entry.repo).and_then(Option::as_ref);
                let latest = remote.and_then(|r| r.commit_of(&entry.git_ref));
                let floats = remote.is_some_and(|r| r.floats(&entry.git_ref));
                // A tag or SHA is pinned: it only moves by installing a different ref.
                let (latest, update_available) = match latest {
                    Some(tip) if floats => {
                        let moved = tip != entry.commit;
                        (Some(tip), moved)
                    }
                    Some(_) => (Some(entry.commit.clone()), false),
                    None => (None, false),
                };
                SkillUpdate {
                    name,
                    scope,
                    git_ref: entry.git_ref,
                    current: entry.commit,
                    latest,
                    update_available,
                }
            })
            .collect())
    }

    pub async fn preview_update(
        &self,
        project_path: Option<String>,
        scope: SkillScope,
        name: String,
    ) -> Result<SkillInstallPreview> {
        ensure_running(self.runtime)?;
        let checkout = project_checkout(project_path)?;
        let layout = self.layout(scope, checkout.as_deref())?;
        let staging_root = self.prepare_staging().await?;
        let (lock_path, folder) = (layout.lock.clone(), name.clone());
        let (entry, kind) = blocking(move || {
            let lock = lock::read(&lock_path)?;
            let not_installed = || CoreError::SkillNotInstalled {
                name: folder.clone(),
            };
            let entry = lock
                .skills
                .get(&folder)
                .cloned()
                .ok_or_else(not_installed)?;
            let kind = SkillSourceKind::parse(&entry.source).ok_or_else(not_installed)?;
            Ok((entry, kind))
        })
        .await?;

        let tip = self
            .remote(&entry.repo)
            .await?
            .commit_of(&entry.git_ref)
            .ok_or_else(|| CoreError::SkillFetchFailed {
                detail: format!("{} no longer exists in {}.", entry.git_ref, entry.repo),
            })?;
        let token = uuid::Uuid::new_v4().to_string();
        let token_dir = staging_root.join(&token);
        std::fs::create_dir_all(&token_dir)?;
        let pick = Pick::Pinned {
            git_ref: entry.git_ref.clone(),
            commit: tip,
            path: entry.path.clone(),
        };
        let target = Replacing {
            layout: &layout,
            folder: &name,
            kind,
            repo: &entry.repo,
        };
        let preview = self
            .replacing_preview(&token, &token_dir, &target, pick, true)
            .await;
        if preview.is_err() {
            let _ = std::fs::remove_dir_all(&token_dir);
        }
        preview
    }

    /// A preview that replaces an installed skill; `diff` adds the change summary.
    async fn replacing_preview(
        &self,
        token: &str,
        token_dir: &Path,
        target: &Replacing<'_>,
        pick: Pick<'_>,
        diff: bool,
    ) -> Result<SkillInstallPreview> {
        let mut preview = self
            .fetch_stage(
                token,
                token_dir,
                target.repo,
                target.kind,
                pick,
                Some(target.folder),
            )
            .await?;
        let (dir, scope) = (token_dir.to_path_buf(), target.layout.scope);
        let installed = target.layout.store_dir(target.folder);
        blocking(move || {
            if diff && installed.exists() {
                preview.changes = Some(install::diff_changes(&dir.join("skill"), &installed)?);
            }
            install::write_record(&dir, &preview, Some(scope))?;
            Ok(preview)
        })
        .await
    }

    pub async fn restore(
        &self,
        project_path: Option<String>,
        scope: SkillScope,
    ) -> Result<SkillRestoreReport> {
        ensure_running(self.runtime)?;
        let checkout = project_checkout(project_path)?;
        let layout = self.layout(scope, checkout.as_deref())?;
        let key = self.scope_lock_key(scope, checkout.as_deref())?;
        let staging_root = self.prepare_staging().await?;
        let _guard = self.runtime.skill_locks.lock(&key).await;
        let lock_path = layout.lock.clone();
        let lock = blocking(move || lock::read(&lock_path)).await?;
        let mut entries = Vec::new();
        for (folder, entry) in &lock.skills {
            entries.push(
                self.restore_entry(&layout, &lock, folder, entry, &staging_root)
                    .await,
            );
        }
        Ok(SkillRestoreReport { scope, entries })
    }

    async fn restore_entry(
        &self,
        layout: &Layout,
        lock: &SkillLock,
        folder: &str,
        entry: &LockEntry,
        staging_root: &Path,
    ) -> SkillRestoreEntry {
        let report = |outcome, preview, detail: Option<String>| SkillRestoreEntry {
            name: folder.to_string(),
            outcome,
            preview,
            detail,
        };
        let Some(kind) = SkillSourceKind::parse(&entry.source) else {
            return report(SkillRestoreOutcome::Skipped, None, None);
        };
        let mut outcome = SkillRestoreOutcome::Ok;
        if !layout.store_dir(folder).exists() {
            let token = uuid::Uuid::new_v4().to_string();
            let token_dir = staging_root.join(&token);
            let staged = match std::fs::create_dir_all(&token_dir) {
                Ok(()) => {
                    let pick = Pick::Pinned {
                        git_ref: entry.git_ref.clone(),
                        commit: entry.commit.clone(),
                        path: entry.path.clone(),
                    };
                    let target = Replacing {
                        layout,
                        folder,
                        kind,
                        repo: &entry.repo,
                    };
                    self.replacing_preview(&token, &token_dir, &target, pick, false)
                        .await
                }
                Err(e) => Err(e.into()),
            };
            let preview = match staged {
                Ok(preview) => preview,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&token_dir);
                    return report(SkillRestoreOutcome::Failed, None, Some(e.to_string()));
                }
            };
            if preview.hash != entry.hash {
                return report(SkillRestoreOutcome::NeedsApproval, Some(preview), None);
            }
            let (layout, folder_owned) = (layout.clone(), folder.to_string());
            let dir = token_dir.clone();
            let adopted = blocking(move || {
                install::adopt_into_store(&layout, &dir.join("skill"), &folder_owned, &token)
            })
            .await;
            let _ = std::fs::remove_dir_all(&token_dir);
            if let Err(e) = adopted {
                return report(SkillRestoreOutcome::Failed, None, Some(e.to_string()));
            }
            outcome = SkillRestoreOutcome::Refetched;
        }
        let (layout, folder_owned) = (layout.clone(), folder.to_string());
        let (entry, lock) = (entry.clone(), lock.clone());
        let relinked = blocking(move || Ok(layout.relink(&folder_owned, &entry, &lock)?)).await;
        match relinked {
            Ok(true) if outcome == SkillRestoreOutcome::Ok => {
                outcome = SkillRestoreOutcome::Relinked;
            }
            Ok(_) => {}
            Err(e) => return report(SkillRestoreOutcome::Failed, None, Some(e.to_string())),
        }
        report(outcome, None, None)
    }

    pub async fn list_skillfish_candidates(
        &self,
        project_path: Option<String>,
    ) -> Result<Vec<SkillfishCandidate>> {
        ensure_running(self.runtime)?;
        let checkout = project_checkout(project_path)?;
        let home = self.skill_home().home;
        let global = self.layout(SkillScope::Global, None).ok();
        blocking(move || {
            let mut out = Vec::new();
            if let Some(checkout) = &checkout {
                let lock = lock::read(&Layout::project(checkout).lock).unwrap_or_default();
                out.extend(skillfish::candidates(
                    SkillScope::Project,
                    &checkout.join("skillfish.json"),
                    &lock,
                ));
            }
            if let (Some(home), Some(global)) = (&home, &global) {
                let lock = lock::read(&global.lock).unwrap_or_default();
                out.extend(skillfish::candidates(
                    SkillScope::Global,
                    &home.join("skillfish.json"),
                    &lock,
                ));
            }
            Ok(out)
        })
        .await
    }

    pub async fn search(&self, query: String) -> Result<Vec<SkillSearchResult>> {
        ensure_running(self.runtime)?;
        let query = query.trim();
        if query.chars().count() < SEARCH_MIN_CHARS {
            return Ok(Vec::new());
        }
        let base = skills_sh::api_base(&self.runtime.environment);
        let request = ProviderHttpRequest::new(skills_sh::search_url(&base, query, SEARCH_LIMIT)?)
            .timeout(SEARCH_TIMEOUT)
            .max_bytes(SEARCH_MAX_BYTES);
        let response = self.runtime.provider_http.get(request).await.map_err(|e| {
            CoreError::SkillFetchFailed {
                detail: e.to_string(),
            }
        })?;
        if !(200..300).contains(&response.status) {
            return Err(CoreError::SkillFetchFailed {
                detail: format!("skills.sh answered {}", response.status),
            });
        }
        skills_sh::parse_search(&response.body)
    }

    pub async fn leaderboard(&self, view: SkillsShView, page: u32) -> Result<SkillLeaderboardPage> {
        ensure_running(self.runtime)?;
        let base = skills_sh::api_base(&self.runtime.environment);
        let request = ProviderHttpRequest::new(skills_sh::leaderboard_url(&base, view, page))
            .timeout(SEARCH_TIMEOUT)
            .max_bytes(SEARCH_MAX_BYTES);
        let response = self.runtime.provider_http.get(request).await.map_err(|e| {
            CoreError::SkillFetchFailed {
                detail: e.to_string(),
            }
        })?;
        if !(200..300).contains(&response.status) {
            return Err(CoreError::SkillFetchFailed {
                detail: format!("skills.sh answered {}", response.status),
            });
        }
        skills_sh::parse_leaderboard(&response.body)
    }

    pub async fn install(
        &self,
        project_path: Option<String>,
        token: String,
        scope: SkillScope,
        targets: Vec<SkillTarget>,
    ) -> Result<SkillInstallReport> {
        ensure_running(self.runtime)?;
        let checkout = project_checkout(project_path)?;
        let layout = self.layout(scope, checkout.as_deref())?;
        let key = self.scope_lock_key(scope, checkout.as_deref())?;
        let staging_root = self.staging_root();
        let _guard = self.runtime.skill_locks.lock(&key).await;
        blocking(move || install::commit(&staging_root, &token, &layout, &targets)).await
    }

    pub async fn remove(
        &self,
        project_path: Option<String>,
        scope: SkillScope,
        name: String,
    ) -> Result<()> {
        ensure_running(self.runtime)?;
        let checkout = project_checkout(project_path)?;
        let layout = self.layout(scope, checkout.as_deref())?;
        let key = self.scope_lock_key(scope, checkout.as_deref())?;
        let _guard = self.runtime.skill_locks.lock(&key).await;
        blocking(move || install::remove(&layout, &name)).await
    }
}

/// The installed skill a preview replaces.
struct Replacing<'a> {
    layout: &'a Layout,
    folder: &'a str,
    kind: SkillSourceKind,
    repo: &'a str,
}

enum Pick<'a> {
    Link(&'a SkillSourceRef),
    /// A lock entry's commit, at its recorded path (`""` is the repo root).
    Pinned {
        git_ref: String,
        commit: String,
        path: String,
    },
}

/// An absolute, existing directory: the project checkout the Workshop passes.
fn project_checkout(project_path: Option<String>) -> Result<Option<PathBuf>> {
    let Some(path) = project_path else {
        return Ok(None);
    };
    let dir = PathBuf::from(&path);
    if dir.is_absolute() && dir.is_dir() {
        Ok(Some(dir))
    } else {
        Err(CoreError::InvalidPath { path })
    }
}
