use std::collections::HashMap;
use std::path::PathBuf;

use hyprlayer_core::agents::AgentOverlay;
use hyprlayer_core::branch_prefix::BranchPrefix;
use hyprlayer_core::process::binaries::BinaryKind;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::{AppError, CmdResult};

const UNSUPPORTED_OPENCODE_PROVIDER_IDS: &[&str] = &["github-copilot"];

pub(crate) fn opencode_provider_is_supported(provider: &str) -> bool {
    let provider = provider.trim().trim_end_matches('/');
    !UNSUPPORTED_OPENCODE_PROVIDER_IDS
        .iter()
        .any(|unsupported| provider.eq_ignore_ascii_case(unsupported))
}

pub(crate) fn opencode_model_is_supported(model: &str) -> bool {
    let model = model
        .strip_prefix("opencode:")
        .or_else(|| {
            model
                .get(.."opencode:".len())
                .filter(|prefix| prefix.eq_ignore_ascii_case("opencode:"))
                .map(|_| &model["opencode:".len()..])
        })
        .unwrap_or(model);
    let provider = model.split('/').next().unwrap_or(model);
    opencode_provider_is_supported(provider)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct KeybindingSettings {
    pub overrides: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptSettings {
    pub builtin_bodies: HashMap<String, String>,
    pub custom: Vec<CustomPrompt>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomPrompt {
    pub id: String,
    pub name: String,
    pub body: String,
    pub overrides: HashMap<String, String>,
    pub chord: Option<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
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
    pub multiplexer: MultiplexerSettings,
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

        if self.tab_titles.model.as_deref().is_some_and(|model| {
            model
                .get(.."opencode:".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("opencode:"))
                && !opencode_model_is_supported(model)
        }) {
            self.tab_titles.model = None;
            changed = true;
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct QuitSettings {
    pub confirm: bool,
}

impl Default for QuitSettings {
    fn default() -> Self {
        Self { confirm: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub default_harness: Option<String>,
    pub claude: ClaudeSettings,
    pub opencode: OpenCodeSettings,
    pub ollama: OllamaSettings,
    pub codex: CodexSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenCodeSettings {
    pub default_model: Option<String>,
    pub commit_model: Option<String>,
    pub selected_models: Vec<String>,
}

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CodexSettings {
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub commit_model: Option<String>,
    pub commit_effort: Option<String>,
    pub selected_models: Vec<String>,
    pub personality: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct MultiplexerSettings {
    pub enabled: bool,
    pub default_engine_id: String,
    pub notify_on_blocked: bool,
    pub persist_across_restarts: bool,
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

pub const DEFAULT_ENGINE_ID: &str = "claude-code";

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
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

#[cfg(not(test))]
fn scrub_unsupported_opencode_defaults() {
    for provider in UNSUPPORTED_OPENCODE_PROVIDER_IDS {
        match hyprlayer_core::opencode::config::clear_default_model_for_provider(provider) {
            Ok(true) => log::info!("removed the retired OpenCode default model for {provider}"),
            Ok(false) => {}
            Err(error) => {
                log::warn!("could not remove unsupported OpenCode default model: {error}")
            }
        }
    }
}

impl SettingsStore {
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("settings.json");
        let mut settings: AppSettings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if settings.normalize() {
            if let Err(error) = persist_sync(&path, &settings) {
                log::warn!("could not persist normalized settings: {error}");
            }
        }
        #[cfg(not(test))]
        std::thread::spawn(scrub_unsupported_opencode_defaults);
        Self {
            path,
            cache: RwLock::new(settings),
        }
    }

    pub async fn get(&self) -> AppSettings {
        self.cache.read().await.clone()
    }

    pub async fn update<F>(&self, mutate: F) -> CmdResult<AppSettings>
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

    pub async fn set(&self, settings: AppSettings) -> CmdResult<()> {
        self.update(|s| *s = settings).await.map(|_| ())
    }
}

fn persist_sync(path: &std::path::Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("create settings dir: {error}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("serialize settings: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|error| format!("write settings: {error}"))?;
    std::fs::rename(&tmp, path).map_err(|error| format!("persist settings: {error}"))
}

async fn persist(path: &std::path::Path, settings: &AppSettings) -> CmdResult<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::internal(format!("create settings dir: {e}")))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| AppError::internal(format!("serialize settings: {e}")))?;

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
    async fn switching_the_default_harness_keeps_a_title_model_the_user_really_picked() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().to_path_buf());

        store
            .update(|settings| {
                settings.providers.default_harness = Some("codex".into());
                settings.tab_titles.model = Some("claude:sonnet".into());
            })
            .await
            .unwrap();

        let back = store
            .update(|settings| settings.providers.default_harness = Some("claude".into()))
            .await
            .unwrap();

        assert_eq!(back.tab_titles.model.as_deref(), Some("claude:sonnet"));
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
    fn notifications_round_trip() {
        let raw = r#"{"notifications":{"enabled":false,"sound":false}}"#;
        let s: AppSettings = serde_json::from_str(raw).unwrap();
        assert!(!s.notifications.enabled);
        assert!(!s.notifications.sound);
    }

    #[test]
    fn quit_confirmation_defaults_on() {
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
}
