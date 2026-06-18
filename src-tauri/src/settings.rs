//! App-local settings, persisted as JSON in the app config dir.
//!
//! This holds only what belongs to the app itself (binary overrides, UI
//! flags). Anything about projects/backends lives in the hyprlayer CLI
//! config, which the CLI owns.

use std::collections::HashMap;
use std::path::PathBuf;

use hyprlayer_core::process::binaries::BinaryKind;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::{AppError, CmdResult};

/// User overrides for keyboard shortcuts. Surfaced in Settings → Keyboard.
///
/// Only *overrides* live here — the per-OS defaults are owned by the frontend
/// catalog so they can change without a settings migration. Keys are stable
/// command ids (e.g. "nav.dashboard"); a `Some(chord)` is a custom binding,
/// `None` means the command is explicitly unbound, and an absent key means
/// "use the catalog default".
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct KeybindingSettings {
    pub overrides: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub binary_overrides: HashMap<BinaryKind, String>,
    /// Where work-item worktrees live; default ~/hyprlayer/worktrees.
    pub worktree_root: Option<String>,
    /// Where app-owned project clones live; default ~/hyprlayer/repos.
    pub repos_root: Option<String>,
    /// Git behaviour for worktree creation, archive, and merge.
    pub git: GitSettings,
    /// Keyboard-shortcut overrides for app operations.
    pub keybindings: KeybindingSettings,
}

/// User-tunable git behaviour. Surfaced in Settings → Git.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct GitSettings {
    /// Branch-name prefix for new worktrees. `None` keeps the built-in
    /// `hyprlayer/` default; a value (e.g. `"feature/"`) is used verbatim.
    pub branch_prefix: Option<String>,
    /// Delete the local branch when archiving a workspace. Default `true`,
    /// matching the historical behaviour.
    pub delete_branch_on_archive: bool,
    /// Auto-archive a workspace once its PR is observed merged.
    pub archive_on_merge: bool,
    /// Queue a GitHub auto-merge (`gh pr merge --auto`) so the PR lands once
    /// required checks pass. Requires auto-merge enabled on the repo.
    pub automerge: bool,
}

/// The default branch prefix when none is configured.
pub const DEFAULT_BRANCH_PREFIX: &str = "hyprlayer/";

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            branch_prefix: None,
            delete_branch_on_archive: true,
            archive_on_merge: false,
            automerge: false,
        }
    }
}

impl GitSettings {
    /// The effective branch prefix, falling back to the built-in default.
    pub fn effective_branch_prefix(&self) -> String {
        self.branch_prefix
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or(DEFAULT_BRANCH_PREFIX)
            .to_string()
    }
}

pub struct SettingsStore {
    path: PathBuf,
    cache: RwLock<AppSettings>,
}

impl SettingsStore {
    /// Load (or default) settings from `<dir>/settings.json`.
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("settings.json");
        let settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            cache: RwLock::new(settings),
        }
    }

    pub async fn get(&self) -> AppSettings {
        self.cache.read().await.clone()
    }

    /// Atomic read-modify-write: the lock is held across the mutation AND the
    /// file write, so concurrent updates can't lose each other's edits, and
    /// the in-memory cache only advances when the write stuck.
    pub async fn update<F>(&self, mutate: F) -> CmdResult<AppSettings>
    where
        F: FnOnce(&mut AppSettings),
    {
        let mut guard = self.cache.write().await;
        let mut next = guard.clone();
        mutate(&mut next);
        persist(&self.path, &next).await?;
        *guard = next.clone();
        Ok(next)
    }

    pub async fn set(&self, settings: AppSettings) -> CmdResult<()> {
        self.update(|s| *s = settings).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_defaults_preserve_branch_deletion() {
        let git = GitSettings::default();
        assert!(
            git.delete_branch_on_archive,
            "must keep deleting by default"
        );
        assert!(!git.archive_on_merge);
        assert!(!git.automerge);
        assert_eq!(git.branch_prefix, None);
    }

    #[test]
    fn effective_prefix_falls_back_and_overrides() {
        assert_eq!(
            GitSettings::default().effective_branch_prefix(),
            DEFAULT_BRANCH_PREFIX
        );
        let custom = GitSettings {
            branch_prefix: Some("feature/".into()),
            ..GitSettings::default()
        };
        assert_eq!(custom.effective_branch_prefix(), "feature/");
        // An empty string is treated as "unset" so the branch is never bare.
        let empty = GitSettings {
            branch_prefix: Some(String::new()),
            ..GitSettings::default()
        };
        assert_eq!(empty.effective_branch_prefix(), DEFAULT_BRANCH_PREFIX);
    }

    #[test]
    fn settings_default_when_git_section_absent() {
        // Old settings.json (no `git` key) must deserialize with git defaults.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.git.delete_branch_on_archive);
    }

    #[test]
    fn settings_default_when_keybindings_absent() {
        // Old settings.json (no `keybindings` key) must deserialize with an
        // empty overrides map — no migration needed.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.keybindings.overrides.is_empty());
    }
}

async fn persist(path: &std::path::Path, settings: &AppSettings) -> CmdResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::internal(format!("create settings dir: {e}")))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::internal(format!("serialize settings: {e}")))?;

    // tmp + rename for atomicity, mirroring the CLI's discipline.
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, json)
        .await
        .map_err(|e| AppError::internal(format!("write settings: {e}")))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| AppError::internal(format!("persist settings: {e}")))?;
    Ok(())
}
