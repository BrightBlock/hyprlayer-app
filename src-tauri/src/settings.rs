//! App-local settings, persisted as JSON in the app config dir.
//!
//! This holds only what belongs to the app itself (binary overrides, UI
//! flags). Anything about projects/backends lives in the hyprlayer CLI
//! config, which the CLI owns.

use std::collections::HashMap;
use std::path::PathBuf;

use hyprlayer_core::agents::AgentOverlay;
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

/// The prompt library (Settings → Prompts).
///
/// Built-in prompts (Commit, Ship) are global; `builtin_bodies` holds a body
/// override keyed by prompt id (`"commit"` | `"ship"`), and an absent key means
/// the stock text (owned by `hyprlayer_core::pipeline`). Custom prompts are
/// user-defined and may carry per-project body overrides. Bodies may contain
/// `{{variable}}` tokens resolved from the worktree at send time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptSettings {
    pub builtin_bodies: HashMap<String, String>,
    pub custom: Vec<CustomPrompt>,
}

/// One user-defined prompt in the library.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomPrompt {
    /// Stable id (uuid) — referenced by the palette, chips, and shortcuts.
    pub id: String,
    pub name: String,
    /// The global body.
    pub body: String,
    /// Per-project body overrides, keyed by project path.
    pub overrides: HashMap<String, String>,
    /// Keyboard chord in the frontend's canonical form (e.g. `"mod+shift+t"`);
    /// `None` = unbound. Dispatched by the frontend, stored here so it rides
    /// with the prompt.
    pub chord: Option<String>,
    /// Pinned prompts render as one-click chips above the session composer.
    pub pinned: bool,
}

/// The Agents view's app-side state. The agent definitions themselves are the
/// `.claude/agents/*.md` files — this holds only what those files have no key
/// for, keyed by the agent's absolute path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentLibrarySettings {
    /// List-only folders, including ones no agent has been moved into yet.
    pub folders: Vec<String>,
    /// Launch settings per agent path (effort, harness, permissions, rules).
    pub overlays: HashMap<String, AgentOverlay>,
    /// Path of the agent the composer opens on. Only one can hold it.
    pub default_agent: Option<String>,
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
    /// The prompt library: built-in prompt overrides + custom prompts.
    pub prompts: PromptSettings,
    /// The agent library's app-side state — folders and the launch settings
    /// claude's own agent frontmatter has no key for.
    pub agents: AgentLibrarySettings,
    /// Per-provider defaults (default model, etc.). Surfaced in Settings → Models.
    pub providers: ProviderSettings,
    /// OS-toast behaviour when a session transitions to "needs you".
    pub notifications: NotificationSettings,
    /// Whether quitting asks first. Settings → General.
    pub quit: QuitSettings,
    /// Auto-generated agent tab titles (enable + which model). Settings → General.
    pub tab_titles: TabTitleSettings,
    /// Multiplexer mode — run agents as native CLIs in persistent PTYs. Off by
    /// default; with it off the app behaves exactly as before. Settings → Advanced.
    pub multiplexer: MultiplexerSettings,
    /// Developer tools (web inspector, PTY byte log, IPC trace). A Settings →
    /// Advanced sibling row; reserved/off by default — nothing consumes it yet
    /// (a stub like [`CodexSettings`], so the row can render and gain behaviour
    /// later without a settings migration).
    pub dev_tools: bool,
    /// The user dismissed the Full Disk Access explainer. macOS can't prompt for
    /// that grant, so the notice is the only way to surface it — but once waved
    /// off it stays off.
    pub full_disk_access_notice_dismissed: bool,
}

/// Auto-generated agent-tab titles. When enabled, a new agent tab summarizes the
/// user's first prompt into a short title via a small model. Surfaced in
/// Settings → General. The model can be a Claude or Codex model, or — so
/// opencode-only users are covered — one of their configured OpenCode models.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct TabTitleSettings {
    /// Master switch. `true` (default) titles each new agent tab from its first
    /// prompt; `false` leaves the default `Agent N` name in place.
    pub enabled: bool,
    /// Which model generates titles, as a tagged id:
    ///   `"claude:<flag>"`        → `claude -p --model <flag>` (empty flag = default)
    ///   `"opencode:<prov>/<id>"` → a one-shot OpenCode turn with that model
    ///   `"codex:<id>"`           → a one-shot ephemeral Codex turn
    /// `None`/absent follows the default harness — see
    /// [`TabTitleSettings::effective_model`].
    pub model: Option<String>,
}

/// Built-in default title model: cheap, fast Claude Haiku.
pub const DEFAULT_TAB_TITLE_MODEL: &str = "claude:haiku";

/// The Codex analogue of Haiku — its smallest, cheapest model, used when Codex is
/// the default harness and the user hasn't picked a title model. Deliberately a
/// fixed id rather than the user's configured Codex models: titling must work
/// before anything is configured, and `selectedModels` only gates the composer's
/// picker (the app-server accepts any valid id). If OpenAI retires it,
/// `generate_with_codex` still falls back to Codex's own default model.
pub const DEFAULT_CODEX_TAB_TITLE_MODEL: &str = "codex:gpt-5.4-mini";

impl Default for TabTitleSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
        }
    }
}

impl TabTitleSettings {
    /// The effective model tag. An explicit pick always wins; otherwise the
    /// default follows the default harness, so a Codex-only user gets titles
    /// without a `claude` binary (and without being silently billed on Anthropic).
    /// OpenCode has no equivalent fallback — which model is even available depends
    /// on which providers the user keyed — so it stays an explicit choice.
    pub fn effective_model<'a>(&'a self, providers: &ProviderSettings) -> &'a str {
        if let Some(explicit) = self.model.as_deref().filter(|m| !m.is_empty()) {
            return explicit;
        }
        match providers.default_harness.as_deref() {
            Some("codex") => DEFAULT_CODEX_TAB_TITLE_MODEL,
            _ => DEFAULT_TAB_TITLE_MODEL,
        }
    }
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

/// Quit confirmation (Settings → General). Quitting takes every running agent,
/// PTY, and lifecycle script down with it, so ⌘Q and the window's close button
/// ask first. The dialog's "Don't ask again" clears `confirm`; the Settings
/// toggle puts it back.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct QuitSettings {
    /// `true` (default) prompts before quitting; `false` exits immediately.
    pub confirm: bool,
}

impl Default for QuitSettings {
    fn default() -> Self {
        Self { confirm: true }
    }
}

/// Defaults for each agent provider/harness. Claude, OpenCode and Codex are all
/// real harnesses; Ollama runs local models *through* the OpenCode harness.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    /// The harness new sessions default to, and the one the Commit/Ship actions
    /// run on: `"claude"`, `"opencode"` or `"codex"` (Ollama runs *through*
    /// OpenCode). `None`/absent ⇒ `"claude"`. Read by the composer and the
    /// commit/ship launch path; the picker still overrides it per tab.
    pub default_harness: Option<String>,
    /// Anthropic Claude (the `claude` CLI).
    pub claude: ClaudeSettings,
    /// OpenCode (the `opencode` CLI) — multi-provider harness.
    pub opencode: OpenCodeSettings,
    /// Ollama — local/remote model server, surfaced through the OpenCode harness.
    pub ollama: OllamaSettings,
    /// OpenAI Codex (the `codex` CLI, driven via its app-server).
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
    /// Model key for the Commit/Ship/Revise action agents when the default
    /// harness is Claude — cheaper than the chat default since these stages are
    /// mechanical git/PR plumbing. `None` ⇒ `"sonnet"`.
    pub commit_model: Option<String>,
    /// Reasoning-effort key for the Commit/Ship/Revise action agents (Claude).
    /// `None` ⇒ `"medium"`.
    pub commit_effort: Option<String>,
    /// Start new sessions in plan permission mode (`--permission-mode plan`).
    pub default_plan_mode: bool,
    /// Which model keys appear in the composer's picker. `None` shows the
    /// whole catalog (the CLI's live list when it resolves, else the built-in
    /// presets, minus org-disabled rows); `Some(list)` shows exactly those keys
    /// (presets plus any custom model ids the user added).
    pub visible_models: Option<Vec<String>>,
    /// Model keys the picker had already offered the last time it was open —
    /// the watermark that lets a new model be added to a CURATED
    /// [`Self::visible_models`] without resurrecting one the user removed on
    /// purpose. `None` on an install that predates the check; the frontend then
    /// seeds it from a frozen snapshot of the pre-catalog preset list.
    pub seen_model_keys: Option<Vec<String>>,

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
    /// `"provider/model"` id for the Commit/Ship/Revise action agents when the
    /// default harness is OpenCode. `None` ⇒ falls back to `default_model` (or
    /// the harness's own default). May be an `"ollama/<id>"` id.
    pub commit_model: Option<String>,
    /// The `"provider/model"` ids the user picked to expose in the composer's
    /// model picker when the OpenCode harness is selected. Empty = none chosen
    /// yet (the OpenCode tab prompts the user to add some).
    pub selected_models: Vec<String>,
}

/// Default Ollama base URL when none is configured. A local install isn't
/// required — the endpoint may point at a remote host (e.g. `http://nexus:11434`).
pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";

/// Ollama harness defaults (Settings → Harnesses → Ollama). Local models are run
/// *through* the OpenCode harness: selecting one sets `harness=opencode` with an
/// `"ollama/<model>"` model id. The provider definition (base URL + model list)
/// is mirrored into OpenCode's `opencode.json` as a `provider.ollama` block —
/// see `hyprlayer_core::opencode::config::set_ollama_provider`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct OllamaSettings {
    /// Base URL of the Ollama server. `None`/empty ⇒ [`DEFAULT_OLLAMA_ENDPOINT`].
    /// May be a remote host; a local install is not required.
    pub endpoint: Option<String>,
    /// Bare Ollama model ids the user chose to expose (e.g. `"qwen2.5-coder:7b"`).
    /// Each surfaces as an `"ollama/<id>"` row in the composer's model picker.
    pub selected_models: Vec<String>,
    /// Default local model for new sessions, as an `"ollama/<id>"` id. `None`
    /// keeps no Ollama default.
    pub default_model: Option<String>,
    /// `"ollama/<id>"` model for the Commit/Ship/Revise action agents when Ollama
    /// is the default harness. `None` ⇒ falls back to `default_model`, then the
    /// first selected model. Runs through OpenCode like every Ollama session.
    pub commit_model: Option<String>,
}

impl OllamaSettings {
    /// The effective base URL, falling back to the default and trimming any
    /// trailing slash so callers can append `/v1` or `/api/tags` cleanly.
    pub fn effective_endpoint(&self) -> String {
        self.endpoint
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or(DEFAULT_OLLAMA_ENDPOINT)
            .trim_end_matches('/')
            .to_string()
    }

    /// Clear `default_model`/`commit_model` if the underlying id is no longer in
    /// `selected_models`. Call after mutating `selected_models` so a default
    /// (stored as `"ollama/<id>"`) can never dangle past the model it names.
    pub fn reconcile_default(&mut self) {
        let in_selected = |v: &Option<String>| {
            v.as_deref().is_some_and(|def| {
                def.strip_prefix("ollama/")
                    .is_some_and(|id| self.selected_models.iter().any(|m| m == id))
            })
        };
        if !in_selected(&self.default_model) {
            self.default_model = None;
        }
        if !in_selected(&self.commit_model) {
            self.commit_model = None;
        }
    }
}

/// Codex harness defaults (Settings → Harnesses → Codex). Auth is NOT stored
/// here — `codex login` owns it in `~/.codex`, and the app only reads the status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CodexSettings {
    /// Default model id for new Codex sessions (e.g. `"gpt-5.4-codex"`), as
    /// returned by `model/list`. `None` keeps Codex's own configured default.
    pub default_model: Option<String>,
    /// Default reasoning effort for new sessions (`minimal`..`xhigh`). `None`
    /// keeps the model's own default effort.
    pub default_effort: Option<String>,
    /// Model for the Commit/Ship/Revise action agents when the default harness is
    /// Codex — these stages are mechanical git/PR plumbing, so a cheaper model
    /// than the chat default is usually right. `None` ⇒ falls back to
    /// `default_model`, then the first selected model.
    pub commit_model: Option<String>,
    /// Reasoning effort for the Commit/Ship/Revise action agents (Codex).
    pub commit_effort: Option<String>,
    /// The model ids the user picked to expose in the composer's model picker.
    /// Empty = none chosen yet (the Codex tab prompts the user to add some).
    pub selected_models: Vec<String>,
    /// Personality/style for new Codex chats. Reserved — `thread/start` takes a
    /// `personality`, but nothing sets it yet.
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

/// Multiplexer mode (Settings → Advanced). When enabled, a new agent tab
/// launches a real CLI in a persistent PTY instead of the native chat, and the
/// `+` on the agent tab strip lets the user pick which engine. The frontend is
/// the only consumer — the backend just stores the config and (separately)
/// spawns the chosen command via `agent_open`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct MultiplexerSettings {
    /// Master switch. `false` (default) keeps every agent tab the native chat;
    /// nothing about the shell changes until this is on.
    pub enabled: bool,
    /// Engine a new tab launches with (also bound to ⌘T). Defaults to
    /// `"claude-code"`. If it names a disabled/removed engine the frontend falls
    /// back to the first enabled one.
    pub default_engine_id: String,
    /// Badge the tab + fire a system notification when an agent is blocked.
    pub notify_on_blocked: bool,
    /// Reattach running PTYs when the app relaunches. A child process can't
    /// outlive the app it was spawned from, so in practice this relaunches the
    /// engine on next launch rather than resurrecting a dead process.
    pub persist_across_restarts: bool,
    /// The engines offered in the `+` menu. Seeded with the three built-ins;
    /// users can disable any or add custom commands ("Add CLI…").
    pub engines: Vec<Engine>,
}

impl Default for MultiplexerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            default_engine_id: DEFAULT_ENGINE_ID.to_string(),
            notify_on_blocked: true,
            persist_across_restarts: true,
            engines: Engine::builtins(),
        }
    }
}

/// The seeded default engine id (Claude Code).
pub const DEFAULT_ENGINE_ID: &str = "claude-code";

/// One CLI engine the multiplexer can launch in a PTY.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct Engine {
    /// Stable id: `"claude-code" | "codex" | "gemini"` for built-ins, a uuid for
    /// custom engines.
    pub id: String,
    /// Display name, e.g. "Claude Code".
    pub label: String,
    /// Single-glyph mark shown on the tab + in the menu (e.g. "✻", "◆", "✦").
    pub glyph: String,
    /// Theme custom-property *name* for the engine accent (e.g. `"--color-accent"`).
    /// Never a raw hex — the frontend applies it as `var(<token>)` so it stays
    /// theme-aware.
    pub accent_token: String,
    /// Executable to run, resolved on the user's login PATH (e.g. `"claude"`).
    pub command: String,
    /// Extra arguments appended to the command.
    pub args: Vec<String>,
    /// Whether this engine appears in the `+` menu.
    pub enabled: bool,
    /// True for the three seeded engines; false for user-added customs.
    pub builtin: bool,
}

impl Engine {
    fn builtin(id: &str, label: &str, glyph: &str, accent_token: &str, command: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            glyph: glyph.to_string(),
            accent_token: accent_token.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            enabled: true,
            builtin: true,
        }
    }

    /// The engines seeded when multiplexer mode is first enabled. Accent tokens
    /// map onto existing theme colors: terracotta / violet / blue / teal.
    pub fn builtins() -> Vec<Engine> {
        vec![
            Engine::builtin(
                DEFAULT_ENGINE_ID,
                "Claude Code",
                "✻",
                "--color-accent",
                "claude",
            ),
            Engine::builtin("codex", "Codex", "◆", "--color-validate", "codex"),
            Engine::builtin("gemini", "Gemini", "✦", "--color-brand", "gemini"),
            Engine::builtin("antigravity", "Antigravity", "◉", "--color-commit", "agy"),
        ]
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

    fn providers_with_default_harness(harness: Option<&str>) -> ProviderSettings {
        ProviderSettings {
            default_harness: harness.map(str::to_string),
            ..Default::default()
        }
    }

    /// A Codex-only user must get Codex titles: falling back to Claude Haiku
    /// would need a `claude` binary they may not have, and would bill Anthropic
    /// for someone who chose OpenAI.
    #[test]
    fn title_model_default_follows_the_default_harness() {
        let tab = TabTitleSettings::default();
        assert_eq!(
            tab.effective_model(&providers_with_default_harness(Some("codex"))),
            DEFAULT_CODEX_TAB_TITLE_MODEL
        );
        // Every other harness keeps Haiku: Claude is the default, and OpenCode has
        // no universally-available cheap model to fall back to.
        for harness in [None, Some("claude"), Some("opencode"), Some("ollama")] {
            assert_eq!(
                tab.effective_model(&providers_with_default_harness(harness)),
                DEFAULT_TAB_TITLE_MODEL,
                "harness {harness:?} should keep the Claude default"
            );
        }
    }

    /// An explicit pick is never overridden by the harness default — switching the
    /// default harness must not silently retarget a model the user chose.
    #[test]
    fn explicit_title_model_wins_over_the_harness_default() {
        let tab = TabTitleSettings {
            enabled: true,
            model: Some("opencode:anthropic/claude-haiku-4-5".into()),
        };
        assert_eq!(
            tab.effective_model(&providers_with_default_harness(Some("codex"))),
            "opencode:anthropic/claude-haiku-4-5"
        );
        // A blank string reads as unset (a cleared select), not as "no model".
        let blank = TabTitleSettings {
            enabled: true,
            model: Some(String::new()),
        };
        assert_eq!(
            blank.effective_model(&providers_with_default_harness(Some("codex"))),
            DEFAULT_CODEX_TAB_TITLE_MODEL
        );
    }

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
    fn ollama_settings_default_and_round_trip() {
        // Absent section -> empty defaults, endpoint resolves to the local default.
        let empty: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.providers.ollama.endpoint, None);
        assert!(empty.providers.ollama.selected_models.is_empty());
        assert_eq!(
            empty.providers.ollama.effective_endpoint(),
            DEFAULT_OLLAMA_ENDPOINT
        );

        // A remote endpoint round-trips; effective_endpoint trims the trailing slash.
        let raw = r#"{"providers":{"ollama":{"endpoint":"http://nexus:11434/","selectedModels":["qwen2.5-coder:7b"],"defaultModel":"ollama/qwen2.5-coder:7b"}}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(
            s.providers.ollama.effective_endpoint(),
            "http://nexus:11434"
        );
        assert_eq!(s.providers.ollama.selected_models, vec!["qwen2.5-coder:7b"]);
        assert_eq!(
            s.providers.ollama.default_model.as_deref(),
            Some("ollama/qwen2.5-coder:7b")
        );
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

    #[test]
    fn quit_confirmation_defaults_on() {
        // Old settings.json without the `quit` key must default to asking —
        // the prompt is opt-out, and a silent quit kills running agents.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.quit.confirm);
    }

    #[test]
    fn quit_confirmation_round_trips() {
        let raw = r#"{"quit":{"confirm":false}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert!(!s.quit.confirm);
    }

    #[test]
    fn multiplexer_defaults_off_with_builtins() {
        // Old settings.json without the `multiplexer` key must default OFF with
        // the built-in engines seeded — the feature is strictly opt-in.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!s.multiplexer.enabled);
        assert_eq!(s.multiplexer.default_engine_id, "claude-code");
        assert!(s.multiplexer.notify_on_blocked);
        assert!(s.multiplexer.persist_across_restarts);
        let ids: Vec<&str> = s
            .multiplexer
            .engines
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, ["claude-code", "codex", "gemini", "antigravity"]);
        assert!(s.multiplexer.engines.iter().all(|e| e.builtin && e.enabled));
    }

    #[test]
    fn multiplexer_round_trips_custom_engine() {
        let raw = r#"{"multiplexer":{"enabled":true,"defaultEngineId":"codex","engines":[{"id":"x","label":"My CLI","glyph":"›","accentToken":"--color-accent","command":"mycli","args":["--foo"],"enabled":true,"builtin":false}]}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert!(s.multiplexer.enabled);
        assert_eq!(s.multiplexer.default_engine_id, "codex");
        assert_eq!(s.multiplexer.engines.len(), 1);
        let e = &s.multiplexer.engines[0];
        assert_eq!(e.command, "mycli");
        assert_eq!(e.args, vec!["--foo"]);
        assert!(!e.builtin);
        // A field absent from the provided object still falls back to its default.
        assert!(s.multiplexer.notify_on_blocked);
    }

    #[test]
    fn prompts_default_empty_when_section_absent() {
        // Old settings.json (no `prompts` key) must deserialize with no
        // overrides and no custom prompts — no migration needed.
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.prompts.builtin_bodies.is_empty());
        assert!(s.prompts.custom.is_empty());
    }

    #[test]
    fn prompts_round_trip() {
        let raw = r#"{"prompts":{
            "builtinBodies":{"commit":"/commit\n\nNever use chore:."},
            "custom":[{
                "id":"p1","name":"Backfill tests",
                "body":"Cover {{changed_files}} with tests.",
                "overrides":{"/repos/app":"Use vitest for {{changed_files}}."},
                "chord":"mod+shift+t","pinned":true
            }]
        }}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(
            s.prompts.builtin_bodies.get("commit").map(String::as_str),
            Some("/commit\n\nNever use chore:.")
        );
        let p = &s.prompts.custom[0];
        assert_eq!(p.name, "Backfill tests");
        assert_eq!(p.chord.as_deref(), Some("mod+shift+t"));
        assert!(p.pinned);
        assert_eq!(
            p.overrides.get("/repos/app").map(String::as_str),
            Some("Use vitest for {{changed_files}}.")
        );
        // A custom prompt missing optional fields still deserializes (defaults).
        let sparse: AppSettings =
            serde_json::from_str(r#"{"prompts":{"custom":[{"id":"x","name":"N","body":""}]}}"#)
                .unwrap();
        assert_eq!(sparse.prompts.custom[0].chord, None);
        assert!(!sparse.prompts.custom[0].pinned);
        assert!(sparse.prompts.custom[0].overrides.is_empty());
    }

    #[test]
    fn dev_tools_defaults_off() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!s.dev_tools);
        let on: AppSettings = serde_json::from_str(r#"{"devTools":true}"#).unwrap();
        assert!(on.dev_tools);
    }
}
