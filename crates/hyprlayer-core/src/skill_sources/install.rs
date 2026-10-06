use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use hyprlayer_models::{SkillScope, SkillSourceKind, SkillTarget, StepStatus};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

use super::fetch::Fetched;
use super::layout::{self, remove_any, remove_link, Layout, LinkMade, LinkState};
use super::lock::{self, LockEntry};
use super::{
    content_hash, file_hashes, SkillChanges, SkillInstallPreview, SkillInstallReport,
    SkillInstallStep, SkillInstallStepKind, SkillPreviewFile, SkillTrustFlags,
};

const MAX_STAGING_AGE: Duration = Duration::from_secs(60 * 60);
const SCRIPT_EXTENSIONS: [&str; 12] = [
    "sh", "bash", "zsh", "py", "js", "mjs", "ts", "rb", "pl", "ps1", "bat", "cmd",
];

pub struct StagedSource<'a> {
    pub kind: SkillSourceKind,
    pub token: &'a str,
    pub repo: &'a str,
    pub git_ref: &'a str,
    pub commit: &'a str,
    pub fetched_from: Option<String>,
    /// An update or restore keeps the installed folder name, whatever the frontmatter says now.
    pub folder: Option<&'a str>,
}

/// The server's own record of a preview; `install` trusts nothing from the client but the token.
#[derive(Serialize, Deserialize)]
struct Staged {
    #[serde(flatten)]
    preview: SkillInstallPreview,
    #[serde(default)]
    replaces: Option<SkillScope>,
}

pub fn write_record(
    token_dir: &Path,
    preview: &SkillInstallPreview,
    replaces: Option<SkillScope>,
) -> Result<()> {
    let record = Staged {
        preview: preview.clone(),
        replaces,
    };
    std::fs::write(
        token_dir.join("preview.json"),
        serde_json::to_vec(&record).map_err(std::io::Error::other)?,
    )?;
    Ok(())
}

/// A staging token is a hyphenated UUID; anything else can't name a directory.
fn token_ok(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Copy out, hash and flag a fetched skill into `<token>/skill/`, then drop the checkout.
pub fn stage_preview(
    token_dir: &Path,
    fetched: &Fetched,
    source: &StagedSource<'_>,
) -> Result<SkillInstallPreview> {
    let root = fetched.repo_dir.join(&fetched.path);
    let skill_md = root.join("SKILL.md");
    let missing = |detail: &str| CoreError::SkillNotFoundInSource {
        detail: detail.to_string(),
    };
    let size = std::fs::metadata(&skill_md)?.len();
    if size > crate::skills::MAX_SKILL_BYTES {
        return Err(missing("SKILL.md is larger than 1 MiB."));
    }
    let text = std::fs::read_to_string(&skill_md)
        .map_err(|_| missing("SKILL.md isn't valid UTF-8 text."))?;
    let parsed = crate::docs::frontmatter::parse(&text);
    let pick = |key: &str| {
        parsed
            .fm
            .extra
            .get(key)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let folder = if fetched.path.is_empty() {
        source.repo.rsplit('/').next().unwrap_or(source.repo)
    } else {
        fetched.path.rsplit('/').next().unwrap_or(&fetched.path)
    };
    let name = match source.folder {
        Some(kept) => kept.to_string(),
        None => {
            let name = pick("name").unwrap_or_else(|| folder.to_string());
            if !crate::skills::is_valid_skill_name(&name) {
                return Err(CoreError::SkillNameInvalid { name });
            }
            name
        }
    };
    let description = pick("description")
        .or_else(|| crate::skills::first_prose_line(&text[parsed.body_offset..]))
        .unwrap_or_default();
    let bash_allowed = allowed_tools(pick("allowed-tools").as_deref().unwrap_or_default())
        .iter()
        .any(|tool| tool.starts_with("Bash"));

    let dest = token_dir.join("skill");
    let mut files = Vec::with_capacity(fetched.entries.len());
    for entry in &fetched.entries {
        let to = dest.join(&entry.rel_path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(root.join(&entry.rel_path), &to)?;
        files.push(SkillPreviewFile {
            rel_path: entry.rel_path.clone(),
            bytes: entry.bytes as f64,
            executable: entry.executable,
            script: is_script(&entry.rel_path),
        });
    }
    let hash = content_hash(&dest)?;
    let _ = std::fs::remove_dir_all(&fetched.repo_dir);

    let flags = SkillTrustFlags {
        scripts: files.iter().any(|f| f.script),
        executables: files.iter().any(|f| f.executable),
        bash_allowed,
    };
    Ok(SkillInstallPreview {
        token: source.token.to_string(),
        name,
        description,
        instructions: text[parsed.body_offset..].to_string(),
        source: source.kind,
        repo: source.repo.to_string(),
        path: fetched.path.clone(),
        git_ref: source.git_ref.to_string(),
        commit: source.commit.to_string(),
        hash,
        fetched_from: source.fetched_from.clone(),
        files,
        flags,
        conflicts: Vec::new(),
        changes: None,
    })
}

/// What an update would do to the installed store copy.
pub fn diff_changes(staged: &Path, installed: &Path) -> Result<SkillChanges> {
    let (new, old) = (file_hashes(staged)?, file_hashes(installed)?);
    let mut changes = SkillChanges {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
    };
    for (rel, hash) in &new {
        match old.get(rel) {
            None => changes.added.push(rel.clone()),
            Some(prev) if prev != hash => changes.changed.push(rel.clone()),
            Some(_) => {}
        }
    }
    changes
        .removed
        .extend(old.keys().filter(|rel| !new.contains_key(*rel)).cloned());
    Ok(changes)
}

fn is_script(rel_path: &str) -> bool {
    rel_path.starts_with("scripts/")
        || rel_path
            .rsplit_once('.')
            .is_some_and(|(_, ext)| SCRIPT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// A YAML list arrives as JSON, a scalar as "a, b" (core's display value).
fn allowed_tools(raw: &str) -> Vec<String> {
    if raw.starts_with('[') {
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
            return list
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string())
                })
                .filter(|t| !t.is_empty())
                .collect();
        }
    }
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Removes the claimed token dir however the install ends.
struct Claimed(PathBuf);

impl Drop for Claimed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Undo {
    links: Vec<PathBuf>,
    store: Option<PathBuf>,
    tmp: Option<PathBuf>,
}

impl Undo {
    fn run(self) {
        for link in &self.links {
            let _ = remove_link(link);
        }
        for dir in self.store.iter().chain(&self.tmp) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn taken(name: &str, installed: bool) -> CoreError {
    CoreError::SkillNameTaken {
        name: name.to_string(),
        installed,
    }
}

fn collision(e: std::io::Error, name: &str) -> CoreError {
    match e.kind() {
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::DirectoryNotEmpty => {
            taken(name, false)
        }
        _ => e.into(),
    }
}

pub fn commit(
    staging_root: &Path,
    token: &str,
    layout: &Layout,
    targets: &[SkillTarget],
) -> Result<SkillInstallReport> {
    if !token_ok(token) {
        return Err(CoreError::SkillPreviewExpired);
    }
    let claimed = staging_root.join(format!(".claimed-{token}"));
    match std::fs::rename(staging_root.join(token), &claimed) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CoreError::SkillPreviewExpired)
        }
        Err(e) => return Err(e.into()),
    }
    let claimed = Claimed(claimed);

    let Staged { preview, replaces } = std::fs::read(claimed.0.join("preview.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or(CoreError::SkillPreviewExpired)?;
    let staged = claimed.0.join("skill");
    let mut steps = Vec::new();

    let started = Instant::now();
    if content_hash(&staged)? != preview.hash {
        return Err(CoreError::SkillContentChanged);
    }
    record(
        &mut steps,
        SkillInstallStepKind::Verify,
        None,
        None,
        started,
    );

    if let Some(scope) = replaces {
        if scope != layout.scope {
            return Err(CoreError::SkillPreviewExpired);
        }
        replace(layout, token, &staged, &preview, &mut steps)?;
        return Ok(SkillInstallReport {
            name: preview.name,
            scope: layout.scope,
            steps,
        });
    }
    if targets.is_empty() {
        return Err(CoreError::SkillTargetsEmpty);
    }
    let folder = preview.name.as_str();
    let mut lock = lock::read(&layout.lock)?;
    if layout::reserved(folder) {
        return Err(CoreError::SkillNameReserved {
            name: folder.to_string(),
        });
    }
    for &target in targets {
        let link = layout.link_path(target, folder);
        if std::fs::symlink_metadata(&link).is_ok() {
            return Err(taken(
                folder,
                layout::owned(&link, &layout.store, folder, &lock),
            ));
        }
    }
    let any_owned = [SkillTarget::Claude, SkillTarget::Agents]
        .iter()
        .any(|&t| layout::owned(&layout.link_path(t, folder), &layout.store, folder, &lock));
    if any_owned || lock.skills.contains_key(folder) {
        return Err(taken(folder, true));
    }

    let mut undo = Undo::default();
    let outcome = (|| -> Result<()> {
        let store_dir = layout.store_dir(folder);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir)?;
        }
        let started = Instant::now();
        let tmp = layout
            .store
            .join(format!(".tmp-{token}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp)?;
        undo.tmp = Some(tmp.clone());
        layout::copy_tree(&staged, &tmp)?;
        std::fs::rename(&tmp, &store_dir).map_err(|e| collision(e, folder))?;
        undo.tmp = None;
        undo.store = Some(store_dir);
        record(&mut steps, SkillInstallStepKind::Store, None, None, started);

        for (carried, link) in layout.link_paths(targets, folder)? {
            let started = Instant::now();
            let made =
                layout::link_skill(&layout.link_target(carried[0], folder), &link, layout.scope)
                    .map_err(|e| collision(e, folder))?;
            undo.links.push(link);
            let detail = match made {
                LinkMade::Symlink => None,
                LinkMade::Junction => Some("junction".to_string()),
                LinkMade::Copy => Some("copied".to_string()),
            };
            for target in carried {
                record(
                    &mut steps,
                    SkillInstallStepKind::Link,
                    Some(target),
                    detail.clone(),
                    started,
                );
            }
        }

        let started = Instant::now();
        let mut recorded: Vec<String> = targets.iter().map(|t| t.as_str().to_string()).collect();
        recorded.sort();
        recorded.dedup();
        lock.skills.insert(
            folder.to_string(),
            LockEntry {
                commit: preview.commit.clone(),
                hash: preview.hash.clone(),
                installed_at: (layout.scope == SkillScope::Global).then(now_ms),
                path: preview.path.clone(),
                git_ref: preview.git_ref.clone(),
                repo: preview.repo.clone(),
                source: preview.source.as_str().to_string(),
                targets: recorded,
            },
        );
        lock::write(&layout.lock, &lock)?;
        record(&mut steps, SkillInstallStepKind::Lock, None, None, started);
        Ok(())
    })();
    if let Err(e) = outcome {
        undo.run();
        return Err(e);
    }
    Ok(SkillInstallReport {
        name: preview.name,
        scope: layout.scope,
        steps,
    })
}

fn record(
    steps: &mut Vec<SkillInstallStep>,
    kind: SkillInstallStepKind,
    target: Option<SkillTarget>,
    detail: Option<String>,
    started: Instant,
) {
    steps.push(SkillInstallStep {
        kind,
        target,
        status: StepStatus::Done,
        detail,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    });
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

pub fn remove(layout: &Layout, folder: &str) -> Result<()> {
    let mut lock = lock::read(&layout.lock)?;
    let store_dir = layout.store_dir(folder);
    let links = [SkillTarget::Claude, SkillTarget::Agents].map(|t| layout.link_path(t, folder));
    let owned: Vec<&PathBuf> = links
        .iter()
        .filter(|l| layout::owned(l, &layout.store, folder, &lock))
        .collect();
    if owned.is_empty() && !lock.skills.contains_key(folder) && !store_dir.exists() {
        return Err(CoreError::SkillNotInstalled {
            name: folder.to_string(),
        });
    }
    for link in owned {
        remove_link(link)?;
    }
    if store_dir.exists() {
        std::fs::remove_dir_all(&store_dir)?;
    }
    if lock.skills.remove(folder).is_some() {
        lock::write(&layout.lock, &lock)?;
    }
    let _ = std::fs::remove_dir(&layout.store);
    if let Some(parent) = &layout.store_parent {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

#[derive(Default)]
struct ReplaceUndo {
    links: Vec<PathBuf>,
    /// `(link, set-aside original)`, in the order they were swapped.
    swapped: Vec<(PathBuf, PathBuf)>,
    tmps: Vec<PathBuf>,
    new_store: bool,
    old_store: Option<PathBuf>,
}

impl ReplaceUndo {
    fn run(self, store_dir: &Path) {
        for link in &self.links {
            let _ = remove_link(link);
        }
        for (link, old) in self.swapped.iter().rev() {
            let _ = remove_link(link);
            let _ = std::fs::rename(old, link);
        }
        for tmp in &self.tmps {
            let _ = std::fs::remove_dir_all(tmp);
        }
        if self.new_store {
            let _ = std::fs::remove_dir_all(store_dir);
        }
        if let Some(old) = &self.old_store {
            let _ = std::fs::rename(old, store_dir);
        }
    }
}

fn beside(link: &Path, folder: &str, kind: &str) -> PathBuf {
    link.with_file_name(format!(".{folder}.{kind}-{}", uuid::Uuid::new_v4()))
}

/// Swaps a copy-fallback link for a fresh copy of the new store, keeping the old until the end.
fn refresh_copy(link: &Path, store_dir: &Path, folder: &str, undo: &mut ReplaceUndo) -> Result<()> {
    let tmp = beside(link, folder, "tmp");
    std::fs::create_dir(&tmp)?;
    undo.tmps.push(tmp.clone());
    layout::copy_tree(store_dir, &tmp)?;
    set_aside(link, folder, undo)?;
    std::fs::rename(&tmp, link)?;
    Ok(())
}

fn set_aside(link: &Path, folder: &str, undo: &mut ReplaceUndo) -> Result<()> {
    let old = beside(link, folder, "old");
    std::fs::rename(link, &old)?;
    undo.swapped.push((link.to_path_buf(), old));
    Ok(())
}

/// Install over an existing lock entry (an update, or an approved restore): the new content
/// replaces the store, copy-fallback links are refreshed, and missing links come back.
fn replace(
    layout: &Layout,
    token: &str,
    staged: &Path,
    preview: &SkillInstallPreview,
    steps: &mut Vec<SkillInstallStep>,
) -> Result<()> {
    let folder = preview.name.as_str();
    let mut lock = lock::read(&layout.lock)?;
    let Some(entry) = lock.skills.get(folder).cloned() else {
        return Err(CoreError::SkillNotInstalled {
            name: folder.to_string(),
        });
    };
    layout.sweep_leftovers(folder);
    let links = layout.link_paths(&layout::lock_targets(&entry), folder)?;
    // Classified by the old lock hash, before the store moves.
    let states: Vec<LinkState> = links
        .iter()
        .map(|(_, link)| layout::link_state(link, &layout.store, folder, &lock))
        .collect();
    let store_dir = layout.store_dir(folder);

    let mut undo = ReplaceUndo::default();
    let outcome = (|| -> Result<()> {
        let started = Instant::now();
        std::fs::create_dir_all(&layout.store)?;
        if store_dir.exists() {
            let old = layout
                .store
                .join(format!(".old-{token}-{}", uuid::Uuid::new_v4()));
            std::fs::rename(&store_dir, &old)?;
            undo.old_store = Some(old);
        }
        let tmp = layout
            .store
            .join(format!(".tmp-{token}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp)?;
        undo.tmps.push(tmp.clone());
        layout::copy_tree(staged, &tmp)?;
        std::fs::rename(&tmp, &store_dir)?;
        undo.new_store = true;
        record(steps, SkillInstallStepKind::Store, None, None, started);

        for ((carried, link), state) in links.iter().zip(&states) {
            let started = Instant::now();
            let is_copy = std::fs::symlink_metadata(link).is_ok_and(|m| m.is_dir());
            let detail = match state {
                LinkState::Healthy if is_copy => {
                    refresh_copy(link, &store_dir, folder, &mut undo)?;
                    Some("copied".to_string())
                }
                LinkState::Absent | LinkState::TextFile => {
                    if *state == LinkState::TextFile {
                        set_aside(link, folder, &mut undo)?;
                    }
                    let made = layout::link_skill(
                        &layout.link_target(carried[0], folder),
                        link,
                        layout.scope,
                    )?;
                    if *state == LinkState::Absent {
                        undo.links.push(link.clone());
                    }
                    match made {
                        LinkMade::Symlink => None,
                        LinkMade::Junction => Some("junction".to_string()),
                        LinkMade::Copy => Some("copied".to_string()),
                    }
                }
                _ => None,
            };
            for &target in carried {
                record(
                    steps,
                    SkillInstallStepKind::Link,
                    Some(target),
                    detail.clone(),
                    started,
                );
            }
        }

        let started = Instant::now();
        if let Some(entry) = lock.skills.get_mut(folder) {
            entry.commit = preview.commit.clone();
            entry.hash = preview.hash.clone();
            if layout.scope == SkillScope::Global {
                entry.installed_at = Some(now_ms());
            }
        }
        lock::write(&layout.lock, &lock)?;
        record(steps, SkillInstallStepKind::Lock, None, None, started);
        Ok(())
    })();
    match outcome {
        Err(e) => {
            undo.run(&store_dir);
            Err(e)
        }
        Ok(()) => {
            for old in undo
                .old_store
                .iter()
                .chain(undo.swapped.iter().map(|(_, old)| old))
            {
                remove_any(old);
            }
            Ok(())
        }
    }
}

/// Moves a staged skill into a missing store dir (a restore whose bytes match the lock).
pub fn adopt_into_store(layout: &Layout, staged: &Path, folder: &str, token: &str) -> Result<()> {
    std::fs::create_dir_all(&layout.store)?;
    let tmp = layout
        .store
        .join(format!(".tmp-{token}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp)?;
    let moved = layout::copy_tree(staged, &tmp)
        .and_then(|()| std::fs::rename(&tmp, layout.store_dir(folder)));
    if moved.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    Ok(moved?)
}

/// Token and claimed dirs older than an hour are abandoned previews or crashed installs.
pub fn sweep_staging(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > MAX_STAGING_AGE);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
