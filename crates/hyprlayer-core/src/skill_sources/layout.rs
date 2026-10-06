use std::path::{Path, PathBuf};

use hyprlayer_models::{SkillScope, SkillTarget, Stage};

use crate::skills::SkillHome;

use super::lock::{LockEntry, SkillLock};
use super::{content_hash, SkillConflict, SkillConflictReason, SkillLockState};

const MAX_LINK_TEXT: u64 = 1024;

/// Where one scope keeps its real copies, links and lock (ADR-0010).
#[derive(Debug, Clone)]
pub struct Layout {
    pub scope: SkillScope,
    pub store: PathBuf,
    pub lock: PathBuf,
    /// Dropped with the store when both are empty (`.agents/hyprlayer` for a project).
    pub store_parent: Option<PathBuf>,
    claude_skills: PathBuf,
    agents_skills: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMade {
    Symlink,
    Junction,
    Copy,
}

pub fn global_lock_path(global_store: &Path) -> PathBuf {
    global_store.with_file_name("skills.lock")
}

pub fn project_store(checkout: &Path) -> PathBuf {
    checkout.join(".agents/hyprlayer/skills")
}

pub fn reserved(name: &str) -> bool {
    use Stage::*;
    [
        Research, Plan, Implement, Validate, Commit, Ship, Revise, General,
    ]
    .into_iter()
    .filter_map(Stage::command)
    .any(|command| command.trim_start_matches('/') == name)
}

/// A link is ours when it points into this scope's store, is a copy the lock vouches for,
/// or is a text file standing in for a committed link (a Windows checkout).
pub fn owned(link: &Path, store: &Path, folder: &str, lock: &SkillLock) -> bool {
    !matches!(
        link_state(link, store, folder, lock),
        LinkState::Absent | LinkState::Foreign
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Absent,
    /// An owned symlink or copy that resolves.
    Healthy,
    /// An owned symlink whose target is gone.
    Dangling,
    /// An owned text file standing in for a link.
    TextFile,
    Foreign,
}

pub fn link_state(link: &Path, store: &Path, folder: &str, lock: &SkillLock) -> LinkState {
    let Ok(meta) = std::fs::symlink_metadata(link) else {
        return LinkState::Absent;
    };
    if meta.file_type().is_symlink() {
        return match (
            crate::agents::link_targets_store(link, store),
            link.exists(),
        ) {
            (false, _) => LinkState::Foreign,
            (true, true) => LinkState::Healthy,
            (true, false) => LinkState::Dangling,
        };
    }
    if meta.is_dir() {
        let vouched = lock
            .skills
            .get(folder)
            .is_some_and(|entry| content_hash(link).is_ok_and(|hash| hash == entry.hash));
        return if vouched {
            LinkState::Healthy
        } else {
            LinkState::Foreign
        };
    }
    if meta.is_file()
        && meta.len() <= MAX_LINK_TEXT
        && text_link_points_at(link, &store.join(folder))
    {
        return LinkState::TextFile;
    }
    LinkState::Foreign
}

fn text_link_points_at(link: &Path, store_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(link) else {
        return false;
    };
    let target = Path::new(text.trim());
    let resolved = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link.parent().unwrap_or(Path::new("")).join(target)
    };
    lexical(&resolved) == lexical(store_dir)
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl Layout {
    pub fn global(global_store: &Path, home: &SkillHome) -> Option<Self> {
        Some(Self {
            scope: SkillScope::Global,
            store: global_store.to_path_buf(),
            lock: global_lock_path(global_store),
            store_parent: None,
            claude_skills: home.claude.as_ref()?.join("skills"),
            agents_skills: home.home.as_ref()?.join(".agents/skills"),
        })
    }

    pub fn project(checkout: &Path) -> Self {
        Self {
            scope: SkillScope::Project,
            store: project_store(checkout),
            lock: checkout.join(".agents/hyprlayer/skills.lock"),
            store_parent: Some(checkout.join(".agents/hyprlayer")),
            claude_skills: checkout.join(".claude/skills"),
            agents_skills: checkout.join(".agents/skills"),
        }
    }

    pub fn store_dir(&self, folder: &str) -> PathBuf {
        self.store.join(folder)
    }

    fn parent(&self, target: SkillTarget) -> &Path {
        match target {
            SkillTarget::Claude => &self.claude_skills,
            SkillTarget::Agents => &self.agents_skills,
        }
    }

    pub fn link_path(&self, target: SkillTarget, folder: &str) -> PathBuf {
        self.parent(target).join(folder)
    }

    /// Project links are relative so a committed link survives a clone; global ones are absolute.
    pub fn link_target(&self, target: SkillTarget, folder: &str) -> PathBuf {
        match (self.scope, target) {
            (SkillScope::Global, _) => self.store_dir(folder),
            (SkillScope::Project, SkillTarget::Claude) => {
                Path::new("../../.agents/hyprlayer/skills").join(folder)
            }
            (SkillScope::Project, SkillTarget::Agents) => {
                Path::new("../hyprlayer/skills").join(folder)
            }
        }
    }

    /// The link paths to create for `targets`. Two targets that share one physical parent
    /// (`~/.agents/skills` symlinked to the Claude root) get one link carrying both.
    pub fn link_paths(
        &self,
        targets: &[SkillTarget],
        folder: &str,
    ) -> std::io::Result<Vec<(Vec<SkillTarget>, PathBuf)>> {
        let mut out: Vec<(Vec<SkillTarget>, PathBuf, PathBuf)> = Vec::new();
        for &target in targets {
            let parent = self.parent(target);
            std::fs::create_dir_all(parent)?;
            let physical = std::fs::canonicalize(parent)?;
            match out.iter_mut().find(|(_, _, seen)| *seen == physical) {
                Some((carried, _, _)) => carried.push(target),
                None => out.push((vec![target], self.link_path(target, folder), physical)),
            }
        }
        Ok(out.into_iter().map(|(t, link, _)| (t, link)).collect())
    }

    /// Crashed copy refreshes leave `.<folder>.tmp-*` / `.old-*` beside the links.
    pub fn sweep_leftovers(&self, folder: &str) {
        let prefixes = [format!(".{folder}.tmp-"), format!(".{folder}.old-")];
        for parent in [&self.claude_skills, &self.agents_skills] {
            let Ok(entries) = std::fs::read_dir(parent) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if prefixes.iter().any(|p| name.starts_with(p)) {
                    remove_any(&entry.path());
                }
            }
        }
    }

    /// Local, no network: what a restore would have to repair.
    pub fn lock_state(&self, folder: &str, entry: &LockEntry, lock: &SkillLock) -> SkillLockState {
        if !self.store_dir(folder).exists() {
            return SkillLockState::Missing;
        }
        let broken = lock_targets(entry).into_iter().any(|target| {
            let link = self.link_path(target, folder);
            matches!(
                link_state(&link, &self.store, folder, lock),
                LinkState::Absent | LinkState::Dangling | LinkState::TextFile
            )
        });
        if broken {
            SkillLockState::Broken
        } else {
            SkillLockState::Ok
        }
    }

    /// Re-creates every lock-target link that isn't owned and healthy; a foreign entry stays.
    pub fn relink(
        &self,
        folder: &str,
        entry: &LockEntry,
        lock: &SkillLock,
    ) -> std::io::Result<bool> {
        let mut changed = false;
        for (carried, link) in self.link_paths(&lock_targets(entry), folder)? {
            match link_state(&link, &self.store, folder, lock) {
                LinkState::Healthy | LinkState::Foreign => continue,
                LinkState::Absent => {}
                LinkState::Dangling | LinkState::TextFile => {
                    remove_link(&link).map_err(std::io::Error::other)?;
                }
            }
            link_skill(&self.link_target(carried[0], folder), &link, self.scope)?;
            changed = true;
        }
        Ok(changed)
    }

    /// Why `folder` can't be installed in this scope, if it can't.
    pub fn conflict(&self, folder: &str, lock: &SkillLock) -> Option<SkillConflictReason> {
        let links = [SkillTarget::Claude, SkillTarget::Agents].map(|t| self.link_path(t, folder));
        let present = |p: &PathBuf| std::fs::symlink_metadata(p).is_ok();
        if lock.skills.contains_key(folder)
            || links.iter().any(|l| owned(l, &self.store, folder, lock))
        {
            return Some(SkillConflictReason::Installed);
        }
        links
            .iter()
            .any(present)
            .then_some(SkillConflictReason::Taken)
    }
}

/// The lock's targets, unknown ones dropped.
pub fn lock_targets(entry: &LockEntry) -> Vec<SkillTarget> {
    let mut targets: Vec<SkillTarget> = entry
        .targets
        .iter()
        .filter_map(|t| SkillTarget::parse(t))
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// Reserved names conflict in both scopes; otherwise each scope reports its own.
pub fn conflicts(folder: &str, scopes: &[(&Layout, &SkillLock)]) -> Vec<SkillConflict> {
    if reserved(folder) {
        return [SkillScope::Project, SkillScope::Global]
            .into_iter()
            .map(|scope| SkillConflict {
                scope,
                reason: SkillConflictReason::Reserved,
            })
            .collect();
    }
    scopes
        .iter()
        .filter_map(|(layout, lock)| {
            layout.conflict(folder, lock).map(|reason| SkillConflict {
                scope: layout.scope,
                reason,
            })
        })
        .collect()
}

pub fn remove_any(path: &Path) {
    let _ = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) => Err(e),
    };
}

/// Removes a link, a junction or an owned copy; the caller has already proven it's ours.
pub fn remove_link(link: &Path) -> crate::error::Result<()> {
    let meta = std::fs::symlink_metadata(link)?;
    crate::worktree::thoughts_links::remove_existing_link(link, &meta)
}

/// Never deletes: an occupied path is the caller's refusal to make.
pub fn link_skill(target: &Path, link: &Path, scope: SkillScope) -> std::io::Result<LinkMade> {
    #[cfg(unix)]
    {
        let _ = scope;
        std::os::unix::fs::symlink(target, link)?;
        Ok(LinkMade::Symlink)
    }
    #[cfg(windows)]
    {
        let absolute = if target.is_absolute() {
            target.to_path_buf()
        } else {
            link.parent().unwrap_or(Path::new("")).join(target)
        };
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => return Ok(LinkMade::Symlink),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(e),
            Err(e) if e.raw_os_error() == Some(1314) && scope == SkillScope::Global => {
                if junction::create(&absolute, link).is_ok() {
                    return Ok(LinkMade::Junction);
                }
            }
            Err(_) => {}
        }
        copy_dir(&absolute, link)?;
        Ok(LinkMade::Copy)
    }
}

#[cfg(windows)]
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    copy_tree(from, to)
}

pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir(&dest)?;
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}
