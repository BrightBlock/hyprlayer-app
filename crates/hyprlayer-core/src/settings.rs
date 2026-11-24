use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use hyprlayer_models::{AgentControlLevel, Harness, PermissionMode};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::RwLock;

use crate::agents::AgentOverlay;
use crate::branch_prefix::BranchPrefix;
use crate::error::{CoreError, Result};
use crate::opencode::support::opencode_model_is_supported;
use crate::process::binaries::BinaryKind;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct KeybindingSettings {
    pub overrides: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct PromptSettings {
    pub builtin_bodies: HashMap<String, String>,
    pub custom: Vec<CustomPrompt>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct CustomPrompt {
    pub id: String,
    pub name: String,
    pub body: String,
    pub overrides: HashMap<String, String>,
    pub chord: Option<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentLibrarySettings {
    pub folders: Vec<String>,
    pub overlays: HashMap<String, AgentOverlay>,
    pub default_agent: Option<String>,
    pub group_uncategorized: bool,
    pub closed_folders: Vec<String>,
}

impl Default for AgentLibrarySettings {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            overlays: HashMap::new(),
            default_agent: None,
            group_uncategorized: true,
            closed_folders: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub binary_overrides: HashMap<BinaryKind, String>,
    pub worktree_root: Option<String>,
    pub repos_root: Option<String>,
    pub git: GitSettings,
    pub keybindings: KeybindingSettings,
    pub prompts: PromptSettings,
    pub agents: AgentLibrarySettings,
    pub providers: ProviderSettings,
    pub notifications: NotificationSettings,
    pub quit: QuitSettings,
    pub tab_titles: TabTitleSettings,
    pub recaps: RecapSettings,
    pub multiplexer: MultiplexerSettings,
    pub goals: GoalSettings,
    pub agent_control: AgentControlSettings,
    pub dev_tools: bool,
    pub full_disk_access_notice_dismissed: bool,
}

impl AppSettings {
    fn normalize(&mut self) -> bool {
        let opencode = &mut self.providers.opencode;
        let selected_len = opencode.selected_models.len();
        opencode
            .selected_models
            .retain(|model| opencode_model_is_supported(model));
        let mut changed = selected_len != opencode.selected_models.len();

        for model in [&mut opencode.default_model, &mut opencode.commit_model] {
            if model
                .as_deref()
                .is_some_and(|model| !opencode_model_is_supported(model))
            {
                *model = None;
                changed = true;
            }
        }

        match &mut self.agent_control.lead {
            None => {
                self.agent_control.lead = Some(LeadSettings::default());
                changed = true;
            }
            Some(lead) => changed |= crate::lead::refresh_retired_rules(&mut lead.rules),
        }

        for model in [&mut self.tab_titles.model, &mut self.goals.evaluator_model] {
            if model.as_deref().is_some_and(|model| {
                model
                    .get(.."opencode:".len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("opencode:"))
                    && !opencode_model_is_supported(model)
            }) {
                *model = None;
                changed = true;
            }
        }

        changed
    }

    // A title model that only restates the outgoing harness's own default was never a
    // deliberate pin: the picker shows the resolved default as its value, so selecting it
    // is indistinguishable from leaving it unset. Drop it so the new harness's default wins.
    fn refollow_title_model(&mut self, previous_harness: Option<&str>) {
        if self.providers.default_harness.as_deref() == previous_harness {
            return;
        }
        if self.tab_titles.model.as_deref()
            == Some(TabTitleSettings::default_model_for(previous_harness))
        {
            self.tab_titles.model = None;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct TabTitleSettings {
    pub enabled: bool,
    pub model: Option<String>,
}

pub const DEFAULT_TAB_TITLE_MODEL: &str = "claude:haiku";

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
    pub fn default_model_for(default_harness: Option<&str>) -> &'static str {
        match default_harness {
            Some("codex") => DEFAULT_CODEX_TAB_TITLE_MODEL,
            _ => DEFAULT_TAB_TITLE_MODEL,
        }
    }

    pub fn effective_model<'a>(&'a self, providers: &ProviderSettings) -> &'a str {
        if let Some(explicit) = self.model.as_deref().filter(|m| !m.is_empty()) {
            return explicit;
        }
        Self::default_model_for(providers.default_harness.as_deref())
    }
}

/// A short recap of a turn that finished while the user was away, on the tab-title model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct RecapSettings {
    pub enabled: bool,
}

impl Default for RecapSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// A new goal's limits, named as `GoalLimits` names them; plain numbers, since a goal is never unlimited.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct GoalSettings {
    pub max_iterations: u32,
    pub max_cost_usd: f64,
    pub max_tokens: u32,
    pub max_active_secs: u32,
    pub evaluator_model: Option<String>,
}

impl Default for GoalSettings {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_cost_usd: 10.0,
            max_tokens: 2_000_000,
            max_active_secs: 3_600,
            evaluator_model: None,
        }
    }
}

impl GoalSettings {
    pub fn effective_evaluator_model<'a>(&'a self, providers: &ProviderSettings) -> &'a str {
        if let Some(explicit) = self.evaluator_model.as_deref().filter(|m| !m.is_empty()) {
            return explicit;
        }
        TabTitleSettings::default_model_for(providers.default_harness.as_deref())
    }
}

/// What an agent may ask of the runtime, and the fan-out limits on agents that spawn agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentControlSettings {
    pub level: AgentControlLevel,
    pub max_depth: u32,
    pub max_live_children: u32,
    pub max_agent_hops: u32,
    // `None` only on a save from a desktop that predates the Lead; the store keeps the stored one.
    #[serde(default)]
    pub lead: Option<LeadSettings>,
}

impl AgentControlSettings {
    pub fn lead(&self) -> LeadSettings {
        self.lead.clone().unwrap_or_default()
    }
}

impl Default for AgentControlSettings {
    fn default() -> Self {
        Self {
            level: AgentControlLevel::Read,
            max_depth: 2,
            max_live_children: 4,
            max_agent_hops: 6,
            lead: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct LeadSettings {
    pub enabled: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub max_live_children: u32,
    pub rules: Vec<GuardrailRule>,
    pub projects: BTreeMap<String, LeadRuleScope>,
    // `None` reads as Auto, and on a save from a desktop that predates the field keeps the stored one.
    pub permission_mode: LeadPermissionMode,
    pub remote_control: BTreeSet<String>,
}

impl LeadSettings {
    pub fn permission_mode(&self) -> PermissionMode {
        self.permission_mode.0.unwrap_or(PermissionMode::Auto)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct LeadPermissionMode(pub Option<PermissionMode>);

// Tolerant, so one unknown mode cannot fail the whole settings file and reset it.
impl<'de> Deserialize<'de> for LeadPermissionMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        let mode = raw.as_str().and_then(PermissionMode::try_parse);
        if mode.is_none() && !raw.is_null() {
            tracing::warn!("unknown Lead permission mode {raw}; using Auto");
        }
        Ok(Self(mode))
    }
}

impl Default for LeadSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            effort: None,
            max_live_children: 6,
            rules: crate::lead::default_rules(),
            projects: BTreeMap::new(),
            permission_mode: LeadPermissionMode(None),
            remote_control: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct GuardrailRule {
    pub id: String,
    pub title: String,
    pub text: String,
    pub enabled: bool,
}

impl Default for GuardrailRule {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            text: String::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum LeadRuleScope {
    InheritAndFile,
    #[default]
    InheritOnly,
}

// Tolerant, so one unknown scope cannot fail the whole settings file and reset it.
impl<'de> Deserialize<'de> for LeadRuleScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            Some("inherit_and_file") => Self::InheritAndFile,
            Some("inherit_only") => Self::InheritOnly,
            _ => {
                tracing::warn!("unknown Lead rule scope {raw}; using Inherit only");
                Self::default()
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub enabled: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct QuitSettings {
    pub confirm: bool,
}

impl Default for QuitSettings {
    fn default() -> Self {
        Self { confirm: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub default_harness: Option<String>,
    pub claude: ClaudeSettings,
    pub opencode: OpenCodeSettings,
    pub ollama: OllamaSettings,
    pub codex: CodexSettings,
}

impl ProviderSettings {
    /// The model a new chat on `harness` launches with when none is asked for. `None` launches
    /// the harness's own default: opencode reads the one Settings writes into its config.
    pub fn default_model(&self, harness: Harness) -> Option<String> {
        match harness {
            Harness::Claude => self
                .claude
                .default_model
                .as_deref()
                .and_then(claude_model_flag),
            Harness::Codex => self.codex.default_model.clone().filter(|m| !m.is_empty()),
            Harness::Opencode => None,
        }
    }
}

/// Settings store a Claude model as the desktop's catalog key (`src/lib/domain/models.ts`);
/// claude takes its flag. `opus` is the catalog's Default, the model claude picks with no flag.
fn claude_model_flag(key: &str) -> Option<String> {
    let flag = match key {
        "" | "opus" => return None,
        "sonnet-1m" => "sonnet[1m]",
        "sonnet-4-6" => "claude-sonnet-4-6",
        "sonnet-4-6-1m" => "claude-sonnet-4-6[1m]",
        "opus-4-8" => "claude-opus-4-8",
        "opus-4-8-1m" => "claude-opus-4-8[1m]",
        "opus-4-7" => "claude-opus-4-7",
        "opus-4-7-1m" => "claude-opus-4-7[1m]",
        "opus-4-6" => "claude-opus-4-6",
        "opus-4-6-1m" => "claude-opus-4-6[1m]",
        "opus-4-1" => "claude-opus-4-1",
        other => other,
    };
    Some(flag.to_string())
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct ClaudeSettings {
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub commit_model: Option<String>,
    pub commit_effort: Option<String>,
    pub default_plan_mode: bool,
    pub visible_models: Option<Vec<String>>,
    pub seen_model_keys: Option<Vec<String>>,

    pub auth_mode: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub use_bedrock: bool,
    pub use_vertex: bool,
    pub chrome_enabled: bool,

    pub review_model: Option<String>,
    pub review_effort: Option<String>,
    pub default_fast_mode: bool,
}

// Hand-written so `{:?}` never prints the API key; the exhaustive destructure
// makes a new field a compile error until it is listed here.
impl std::fmt::Debug for ClaudeSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            default_model,
            default_effort,
            commit_model,
            commit_effort,
            default_plan_mode,
            visible_models,
            seen_model_keys,
            auth_mode,
            api_key,
            base_url,
            use_bedrock,
            use_vertex,
            chrome_enabled,
            review_model,
            review_effort,
            default_fast_mode,
        } = self;
        f.debug_struct("ClaudeSettings")
            .field("default_model", default_model)
            .field("default_effort", default_effort)
            .field("commit_model", commit_model)
            .field("commit_effort", commit_effort)
            .field("default_plan_mode", default_plan_mode)
            .field("visible_models", visible_models)
            .field("seen_model_keys", seen_model_keys)
            .field("auth_mode", auth_mode)
            .field("api_key", &api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", base_url)
            .field("use_bedrock", use_bedrock)
            .field("use_vertex", use_vertex)
            .field("chrome_enabled", chrome_enabled)
            .field("review_model", review_model)
            .field("review_effort", review_effort)
            .field("default_fast_mode", default_fast_mode)
            .finish()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct OpenCodeSettings {
    pub default_model: Option<String>,
    pub commit_model: Option<String>,
    pub selected_models: Vec<String>,
}

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct OllamaSettings {
    pub endpoint: Option<String>,
    pub selected_models: Vec<String>,
    pub default_model: Option<String>,
    pub commit_model: Option<String>,
}

impl OllamaSettings {
    pub fn effective_endpoint(&self) -> String {
        self.endpoint
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or(DEFAULT_OLLAMA_ENDPOINT)
            .trim_end_matches('/')
            .to_string()
    }

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct CodexSettings {
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub commit_model: Option<String>,
    pub commit_effort: Option<String>,
    pub selected_models: Vec<String>,
    pub personality: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct GitSettings {
    pub branch_prefix: Option<String>,
    pub prefixes: Option<Vec<BranchPrefix>>,
    pub delete_branch_on_archive: bool,
    pub archive_on_merge: bool,
    pub automerge: bool,
    pub prune_ignored_on_archive: bool,
}

pub const DEFAULT_BRANCH_PREFIX: &str = "hyprlayer/";

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            branch_prefix: None,
            prefixes: None,
            delete_branch_on_archive: true,
            archive_on_merge: false,
            automerge: false,
            prune_ignored_on_archive: true,
        }
    }
}

impl GitSettings {
    pub fn effective_branch_prefix(&self) -> String {
        self.branch_prefix
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or(DEFAULT_BRANCH_PREFIX)
            .to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct MultiplexerSettings {
    pub enabled: bool,
    pub default_engine_id: String,
    pub notify_on_blocked: bool,
    pub persist_across_restarts: bool,
    /// App actions (Commit, Ship, plan builds, prompts) open the default engine instead of a chat.
    pub actions_in_cli: bool,
    pub engines: Vec<Engine>,
}

impl Default for MultiplexerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            default_engine_id: DEFAULT_ENGINE_ID.to_string(),
            notify_on_blocked: true,
            persist_across_restarts: true,
            actions_in_cli: true,
            engines: Engine::builtins(),
        }
    }
}

impl MultiplexerSettings {
    /// The engine app actions open in, as the desktop's `resolveMultiplexer` + `defaultEngine` pick it; `None` opens a chat.
    pub fn action_engine(&self) -> Option<String> {
        if !self.enabled || !self.actions_in_cli {
            return None;
        }
        let builtins = Engine::builtins();
        let mut engines: Vec<&Engine> = self.engines.iter().filter(|e| !e.id.is_empty()).collect();
        engines.extend(
            builtins
                .iter()
                .filter(|b| !self.engines.iter().any(|e| e.id == b.id)),
        );
        engines
            .iter()
            .find(|e| e.id == self.default_engine_id && e.enabled)
            .or_else(|| engines.iter().find(|e| e.enabled))
            .or_else(|| engines.first())
            .map(|e| e.id.clone())
    }
}

pub const DEFAULT_ENGINE_ID: &str = "claude-code";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct Engine {
    pub id: String,
    pub label: String,
    pub glyph: String,
    pub accent_token: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
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
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("settings.json");
        let mut settings = read_or_move_aside(&path);
        if settings.normalize() {
            if let Err(error) = persist_sync(&path, &settings) {
                tracing::warn!("could not persist normalized settings: {error}");
            }
        }
        Self {
            path,
            cache: RwLock::new(settings),
        }
    }

    pub async fn get(&self) -> AppSettings {
        self.cache.read().await.clone()
    }

    pub async fn update<F>(&self, mutate: F) -> Result<AppSettings>
    where
        F: FnOnce(&mut AppSettings),
    {
        let mut guard = self.cache.write().await;
        let mut next = guard.clone();
        let previous_harness = next.providers.default_harness.clone();
        mutate(&mut next);
        next.refollow_title_model(previous_harness.as_deref());
        next.normalize();
        persist(&self.path, &next).await?;
        *guard = next.clone();
        Ok(next)
    }

    pub async fn set(&self, mut settings: AppSettings) -> Result<()> {
        self.update(|s| {
            let stored = s.agent_control.lead.take();
            // Only `set_lead_remote_control` writes it, so a desktop's stale copy never turns one off.
            if let (Some(lead), Some(stored)) = (&mut settings.agent_control.lead, &stored) {
                lead.remote_control = stored.remote_control.clone();
            }
            match &mut settings.agent_control.lead {
                None => settings.agent_control.lead = stored,
                Some(lead) if lead.permission_mode.0.is_none() => {
                    lead.permission_mode = stored.map(|l| l.permission_mode).unwrap_or_default();
                }
                Some(_) => {}
            }
            *s = settings;
        })
        .await
        .map(|_| ())
    }
}

/// An unreadable file is renamed aside rather than ignored: the next `update()`
/// would otherwise persist defaults over the user's keys and overrides.
fn read_or_move_aside(path: &std::path::Path) -> AppSettings {
    let failure = match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(settings) => return settings,
            Err(e) => e.to_string(),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AppSettings::default(),
        Err(e) => e.to_string(),
    };
    let aside = path.with_extension(format!("json.unreadable-{}", crate::store::now_ms() as u64));
    match std::fs::rename(path, &aside) {
        Ok(()) => tracing::error!(
            "{} is unreadable ({failure}); moved it to {} and started from defaults",
            path.display(),
            aside.display()
        ),
        Err(e) => tracing::error!(
            "{} is unreadable ({failure}) and could not be moved aside ({e}); \
             starting from defaults",
            path.display()
        ),
    }
    AppSettings::default()
}

fn persist_sync(path: &std::path::Path, settings: &AppSettings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| std::io::Error::other(format!("serialize settings: {error}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

async fn persist(path: &std::path::Path, settings: &AppSettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| CoreError::Io(std::io::Error::other(format!("serialize settings: {e}"))))?;

    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, json).await?;
    tokio::fs::rename(&tmp, path).await?;
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

    #[test]
    fn title_model_default_follows_the_default_harness() {
        let tab = TabTitleSettings::default();
        assert_eq!(
            tab.effective_model(&providers_with_default_harness(Some("codex"))),
            DEFAULT_CODEX_TAB_TITLE_MODEL
        );
        for harness in [None, Some("claude"), Some("opencode"), Some("ollama")] {
            assert_eq!(
                tab.effective_model(&providers_with_default_harness(harness)),
                DEFAULT_TAB_TITLE_MODEL,
                "harness {harness:?} should keep the Claude default"
            );
        }
    }

    #[tokio::test]
    async fn an_unparseable_settings_file_is_moved_aside_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let original = r#"{"providers": {"claude": {"apiKey": "sk-keep"}}, oops"#;
        std::fs::write(dir.path().join("settings.json"), original).unwrap();

        let store = SettingsStore::load(dir.path().to_path_buf());
        store.update(|_| {}).await.unwrap();

        let aside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("settings.json.unreadable-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read_to_string(aside[0].path()).unwrap(), original);
    }

    #[tokio::test]
    async fn a_save_without_lead_keeps_the_stored_lead() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().to_path_buf());
        assert!(store.get().await.agent_control.lead().enabled);
        store
            .update(|s| {
                s.agent_control.lead = Some(LeadSettings {
                    enabled: false,
                    ..LeadSettings::default()
                })
            })
            .await
            .unwrap();

        let from_an_older_desktop: AppSettings = serde_json::from_str(
            r#"{"agentControl": {"level": "full", "maxDepth": 2, "maxLiveChildren": 4, "maxAgentHops": 6}}"#,
        )
        .unwrap();
        store.set(from_an_older_desktop).await.unwrap();

        let saved = store.get().await.agent_control;
        assert_eq!(saved.level, AgentControlLevel::Full);
        assert!(!saved.lead().enabled);
    }

    #[test]
    fn an_unknown_lead_scope_loads_as_inherit_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"agentControl": {"lead": {"enabled": false, "projects": {"/a": "inherit_and_file", "/b": "file_only", "/c": 3}}}}"#,
        )
        .unwrap();

        let lead = SettingsStore::load(dir.path().to_path_buf())
            .cache
            .into_inner()
            .agent_control
            .lead();

        assert!(!lead.enabled, "the rest of the file still loads");
        assert_eq!(lead.projects["/a"], LeadRuleScope::InheritAndFile);
        assert_eq!(lead.projects["/b"], LeadRuleScope::InheritOnly);
        assert_eq!(lead.projects["/c"], LeadRuleScope::InheritOnly);
    }

    #[tokio::test]
    async fn switching_the_default_harness_retargets_a_title_model_left_on_its_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().to_path_buf());

        store
            .update(|settings| {
                settings.providers.default_harness = Some("codex".into());
                settings.tab_titles.model = Some(DEFAULT_CODEX_TAB_TITLE_MODEL.into());
            })
            .await
            .unwrap();

        let back = store
            .update(|settings| settings.providers.default_harness = Some("claude".into()))
            .await
            .unwrap();

        assert_eq!(back.tab_titles.model, None);
        assert_eq!(
            back.tab_titles.effective_model(&back.providers),
            DEFAULT_TAB_TITLE_MODEL
        );
    }

    #[tokio::test]
    async fn a_title_model_survives_an_update_that_leaves_the_harness_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().to_path_buf());

        store
            .update(|settings| {
                settings.providers.default_harness = Some("codex".into());
                settings.tab_titles.model = Some(DEFAULT_CODEX_TAB_TITLE_MODEL.into());
            })
            .await
            .unwrap();

        let same = store
            .update(|settings| settings.tab_titles.enabled = false)
            .await
            .unwrap();

        assert_eq!(
            same.tab_titles.model.as_deref(),
            Some(DEFAULT_CODEX_TAB_TITLE_MODEL)
        );
    }

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
        assert_eq!(git.prefixes, None);
        assert!(git.prune_ignored_on_archive, "prune must default to true");
    }

    #[test]
    fn settings_default_when_prefixes_absent() {
        let raw = r#"{"git":{"branchPrefix":"jt/"}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(s.git.prefixes, None);
        assert_eq!(s.git.branch_prefix.as_deref(), Some("jt/"));
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
        let empty = GitSettings {
            branch_prefix: Some(String::new()),
            ..GitSettings::default()
        };
        assert_eq!(empty.effective_branch_prefix(), DEFAULT_BRANCH_PREFIX);
    }

    #[test]
    fn settings_default_when_git_section_absent() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.git.delete_branch_on_archive);
    }

    #[test]
    fn settings_default_when_keybindings_absent() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.keybindings.overrides.is_empty());
    }

    #[test]
    fn settings_default_when_providers_section_absent() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.providers.claude.default_model, None);
        assert_eq!(s.providers.codex.default_model, None);
    }

    #[test]
    fn a_new_chat_takes_the_configured_model_as_its_harness_flag() {
        let mut p = ProviderSettings::default();
        assert_eq!(p.default_model(Harness::Claude), None);
        for (key, flag) in [
            ("haiku", Some("haiku")),
            ("sonnet-1m", Some("sonnet[1m]")),
            ("opus-4-8-1m", Some("claude-opus-4-8[1m]")),
            ("claude-opus-5-5", Some("claude-opus-5-5")),
            ("opus", None),
        ] {
            p.claude.default_model = Some(key.into());
            assert_eq!(p.default_model(Harness::Claude).as_deref(), flag, "{key}");
        }
        p.codex.default_model = Some("gpt-5.5".into());
        assert_eq!(p.default_model(Harness::Codex).as_deref(), Some("gpt-5.5"));
        p.opencode.default_model = Some("anthropic/claude-sonnet-4-5".into());
        assert_eq!(p.default_model(Harness::Opencode), None);
    }

    #[test]
    fn providers_default_model_round_trips() {
        let raw = r#"{"providers":{"claude":{"defaultModel":"sonnet"}}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert_eq!(s.providers.claude.default_model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn claude_auth_defaults_to_cli() {
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

    #[tokio::test]
    async fn settings_store_migrates_legacy_copilot_models_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let raw = r#"{
            "providers": {
                "claude": { "defaultModel": "github-copilot/not-an-opencode-model" },
                "opencode": {
                    "defaultModel": "GITHUB-COPILOT/gpt-5",
                    "commitModel": "github-copilot/claude-sonnet",
                    "selectedModels": [
                        "anthropic/claude-sonnet-4-5",
                        "GitHub-Copilot/gpt-5",
                        "openai/github-copilot"
                    ]
                }
            },
            "tabTitles": {
                "enabled": true,
                "model": "OpenCode:github-COPILOT/gpt-5"
            }
        }"#;
        std::fs::write(&path, raw).unwrap();

        let store = SettingsStore::load(dir.path().to_path_buf());
        let settings = store.get().await;
        assert_eq!(
            settings.providers.opencode.selected_models,
            ["anthropic/claude-sonnet-4-5", "openai/github-copilot"]
        );
        assert_eq!(settings.providers.opencode.default_model, None);
        assert_eq!(settings.providers.opencode.commit_model, None);
        assert_eq!(settings.tab_titles.model, None);
        assert_eq!(
            settings.providers.claude.default_model.as_deref(),
            Some("github-copilot/not-an-opencode-model")
        );

        let persisted: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            persisted.providers.opencode.selected_models,
            settings.providers.opencode.selected_models
        );
        assert_eq!(persisted.providers.opencode.default_model, None);
        assert_eq!(persisted.providers.opencode.commit_model, None);
        assert_eq!(persisted.tab_titles.model, None);
    }

    #[tokio::test]
    async fn settings_store_normalizes_copilot_models_before_every_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let store = SettingsStore::load(dir.path().to_path_buf());

        let updated = store
            .update(|settings| {
                let opencode = &mut settings.providers.opencode;
                opencode.selected_models =
                    vec!["github-copilot/gpt-5".into(), "openai/gpt-5".into()];
                opencode.default_model = Some("GitHub-Copilot/gpt-5".into());
                opencode.commit_model = Some("GITHUB-COPILOT/gpt-5".into());
                settings.tab_titles.model = Some("opencode:github-copilot/gpt-5".into());
            })
            .await
            .unwrap();

        assert_eq!(updated.providers.opencode.selected_models, ["openai/gpt-5"]);
        assert_eq!(updated.providers.opencode.default_model, None);
        assert_eq!(updated.providers.opencode.commit_model, None);
        assert_eq!(updated.tab_titles.model, None);

        let persisted = std::fs::read_to_string(path).unwrap();
        assert!(!persisted.to_ascii_lowercase().contains("github-copilot"));
    }

    #[test]
    fn ollama_settings_default_and_round_trip() {
        let empty: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.providers.ollama.endpoint, None);
        assert!(empty.providers.ollama.selected_models.is_empty());
        assert_eq!(
            empty.providers.ollama.effective_endpoint(),
            DEFAULT_OLLAMA_ENDPOINT
        );

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
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.notifications.enabled);
        assert!(s.notifications.sound);
    }

    #[test]
    fn quit_confirmation_defaults_on() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.quit.confirm);
    }

    #[test]
    fn multiplexer_defaults_off_with_builtins() {
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
        assert!(s.multiplexer.notify_on_blocked);
    }

    #[test]
    fn agent_library_group_uncategorized_defaults_on() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(s.agents.group_uncategorized);
        assert!(s.agents.closed_folders.is_empty());

        let s: AppSettings = serde_json::from_str(r#"{"agents":{}}"#).unwrap();
        assert!(s.agents.group_uncategorized);
    }

    #[test]
    fn agent_library_group_uncategorized_round_trips_false() {
        let raw = r#"{"agents":{"groupUncategorized":false}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert!(!s.agents.group_uncategorized);
    }

    #[test]
    fn prompts_default_empty_when_section_absent() {
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

    const V0_7_SETTINGS_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/compat/v0_7/settings.json"
    ));

    #[test]
    fn the_v0_7_settings_fixture_parses_with_todays_shape() {
        let s: AppSettings = serde_json::from_str(V0_7_SETTINGS_FIXTURE).unwrap();

        assert_eq!(
            s.binary_overrides
                .get(&BinaryKind::Claude)
                .map(String::as_str),
            Some("/opt/homebrew/bin/claude")
        );
        assert_eq!(
            s.worktree_root.as_deref(),
            Some("/Users/jt/hyprlayer/worktrees")
        );
        assert_eq!(s.git.branch_prefix.as_deref(), Some("jt/"));
        assert_eq!(s.git.prefixes.as_ref().map(Vec::len), Some(2));
        assert_eq!(s.prompts.custom.len(), 1);
        assert_eq!(s.agents.overlays.len(), 1);
        assert_eq!(
            s.providers.claude.default_model.as_deref(),
            Some("claude-opus-4-8")
        );
        assert_eq!(
            s.providers.opencode.selected_models,
            ["anthropic/claude-sonnet-4-5", "openai/gpt-4o"]
        );
        assert_eq!(s.multiplexer.engines.len(), 4);
        assert!(s.full_disk_access_notice_dismissed);
    }

    #[test]
    fn the_v0_7_settings_fixture_round_trips_losslessly() {
        let settings: AppSettings = serde_json::from_str(V0_7_SETTINGS_FIXTURE).unwrap();
        let once = serde_json::to_value(&settings).unwrap();
        let reparsed: AppSettings = serde_json::from_value(once.clone()).unwrap();
        let twice = serde_json::to_value(&reparsed).unwrap();
        assert_eq!(once, twice);
    }
}
