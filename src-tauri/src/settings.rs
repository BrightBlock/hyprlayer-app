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
    /// Per-provider defaults (default model, etc.). Surfaced in Settings → Models.
    pub providers: ProviderSettings,
    /// OS-toast behaviour when a session transitions to "needs you".
    pub notifications: NotificationSettings,
}

/// Controls native OS notifications fired when a session moves into the
/// needs-you state (awaiting permission/question) while the window is
/// unfocused. The frontend reads this on every transition; toggling the
/// setting takes effect on the next event without a restart.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    /// Master switch. `true` (default) fires a toast when an agent needs you;
    /// `false` suppresses all notifications regardless of `sound`.
    pub enabled: bool,
    /// Whether the toast plays the OS default notification sound.
    pub sound: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sound: true,
        }
    }
}

/// Defaults for each agent provider/harness. Claude and OpenCode are wired up;
/// Codex is stubbed so the section can render now and gain real fields later
/// without a settings migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    /// Anthropic Claude (the `claude` CLI).
    pub claude: ClaudeSettings,
    /// OpenCode (the `opencode` CLI) — multi-provider harness.
    pub opencode: OpenCodeSettings,
    /// OpenAI Codex — not wired up yet; reserved so the providers section is
    /// stable as we build it out.
    pub codex: CodexSettings,
}

/// Claude provider defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaudeSettings {
    /// Default model key for new sessions — one of the composer's model keys
    /// (e.g. `"opus"`, `"sonnet"`) or a custom `--model` value. `None` keeps
    /// the built-in default (Opus).
    pub default_model: Option<String>,
    /// Default reasoning-effort key for new sessions (`low`..`max`). `None`
    /// keeps the built-in default (High).
    pub default_effort: Option<String>,
    /// Start new sessions in plan permission mode (`--permission-mode plan`).
    pub default_plan_mode: bool,
    /// Which model keys appear in the composer's picker. `None` shows every
    /// built-in model; `Some(list)` shows exactly those keys (presets plus any
    /// custom model ids the user added).
    pub visible_models: Option<Vec<String>>,

    // --- Authentication (Settings → Harnesses → Claude Code). ---
    /// How `claude` authenticates: `"cli"` (subscription/login, the default and
    /// historical behaviour — the spawned process inherits the user's env and
    /// `~/.claude` state) or `"apiKey"` (enterprise — the app injects an API key
    /// into the spawn env). `None`/absent is treated as `"cli"`.
    pub auth_mode: Option<String>,
    /// Anthropic API key, injected as `ANTHROPIC_API_KEY` at spawn when
    /// `auth_mode == "apiKey"`. Stored in plaintext settings.json (see module
    /// note); empty/`None` means "no key set".
    pub api_key: Option<String>,
    /// Override the API endpoint (`ANTHROPIC_BASE_URL`) for a proxy/gateway.
    pub base_url: Option<String>,
    /// Route through Amazon Bedrock (`CLAUDE_CODE_USE_BEDROCK=1`).
    pub use_bedrock: bool,
    /// Route through Google Vertex (`CLAUDE_CODE_USE_VERTEX=1`).
    pub use_vertex: bool,
    /// Enable the "Claude in Chrome" browser-control feature — adds `--chrome`
    /// at spawn so sessions can drive the connected Chrome extension.
    pub chrome_enabled: bool,

    // --- Not wired up yet (UI stubs; nothing consumes these). ---
    /// Default model for in-app code reviews. Reserved — no review feature yet.
    pub review_model: Option<String>,
    /// Default effort for in-app code reviews. Reserved.
    pub review_effort: Option<String>,
    /// Start new sessions in fast mode. Reserved — not a `claude` spawn flag yet.
    pub default_fast_mode: bool,
}

/// OpenCode harness defaults (Settings → Harnesses → OpenCode). Provider API
/// keys themselves are NOT stored here — they live in OpenCode's own
/// `auth.json` so the `opencode` CLI uses them natively (see `opencode::config`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenCodeSettings {
    /// Default model for new OpenCode sessions, as a `"provider/model"` id
    /// (e.g. `"anthropic/claude-sonnet-4-5"`). Mirrored into `opencode.json`'s
    /// `model` field so the CLI agrees. `None` keeps OpenCode's own default.
    pub default_model: Option<String>,
    /// The `"provider/model"` ids the user picked to expose in the composer's
    /// model picker when the OpenCode harness is selected. Empty = none chosen
    /// yet (the OpenCode tab prompts the user to add some).
    pub selected_models: Vec<String>,
}

/// Codex provider defaults. Stub — Codex isn't supported yet; this exists so
/// the providers section round-trips and can grow fields without a migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CodexSettings {
    /// Default model for new Codex sessions. Unused until Codex lands.
    pub default_model: Option<String>,
    /// Personality/style for new Codex chats. Reserved — Codex isn't wired up.
    pub personality: Option<String>,
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
    /// Strip git-ignored files (build output, deps) from a worktree before
    /// archiving so archives don't balloon. Files matched by the repo's
    /// `worktree.copy` patterns are preserved. Default `true`.
    pub prune_ignored_on_archive: bool,
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
            prune_ignored_on_archive: true,
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
        assert!(git.prune_ignored_on_archive, "prune must default to true");
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

    #[test]
    fn settings_default_when_providers_section_absent() {
        // Old settings.json (no `providers` key) must deserialize with no
        // configured default model rather than failing.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.providers.claude.default_model, None);
        assert_eq!(s.providers.codex.default_model, None);
    }

    #[test]
    fn providers_default_model_round_trips() {
        let raw = r#"{"providers":{"claude":{"defaultModel":"sonnet"}}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(s.providers.claude.default_model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn claude_auth_defaults_to_cli() {
        // Old settings.json without auth fields: cli mode, no key, no chrome.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.providers.claude.auth_mode, None);
        assert_eq!(s.providers.claude.api_key, None);
        assert!(!s.providers.claude.use_bedrock);
        assert!(!s.providers.claude.chrome_enabled);
    }

    #[test]
    fn claude_auth_fields_round_trip() {
        let raw = r#"{"providers":{"claude":{"authMode":"apiKey","apiKey":"sk-x","useBedrock":true,"chromeEnabled":true}}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        let c = &s.providers.claude;
        assert_eq!(c.auth_mode.as_deref(), Some("apiKey"));
        assert_eq!(c.api_key.as_deref(), Some("sk-x"));
        assert!(c.use_bedrock);
        assert!(c.chrome_enabled);
    }

    #[test]
    fn opencode_settings_default_and_round_trip() {
        // Absent section -> empty defaults (no migration needed).
        let empty: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.providers.opencode.default_model, None);
        assert!(empty.providers.opencode.selected_models.is_empty());

        let raw = r#"{"providers":{"opencode":{"defaultModel":"anthropic/claude-sonnet-4-5","selectedModels":["anthropic/claude-sonnet-4-5","openai/gpt-4o"]}}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(
            s.providers.opencode.default_model.as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        assert_eq!(s.providers.opencode.selected_models.len(), 2);
    }

    #[test]
    fn notifications_default_on_with_sound() {
        // Old settings.json without the `notifications` key must default to
        // enabled — the feature is opt-out, not opt-in.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.notifications.enabled);
        assert!(s.notifications.sound);
    }

    #[test]
    fn notifications_round_trip() {
        let raw = r#"{"notifications":{"enabled":false,"sound":false}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert!(!s.notifications.enabled);
        assert!(!s.notifications.sound);
    }
}
