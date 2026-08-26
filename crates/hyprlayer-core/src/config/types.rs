use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const MAX_SUPPORTED_CONFIG_VERSION: u64 = 4;

pub const DEFAULT_THOUGHTS_REPO: &str = "~/thoughts";
pub const DEFAULT_REPOS_DIR: &str = "repos";
pub const DEFAULT_GLOBAL_DIR: &str = "global";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Git,
    Obsidian,
    Notion,
    Anytype,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Git => "git",
            BackendKind::Obsidian => "obsidian",
            BackendKind::Notion => "notion",
            BackendKind::Anytype => "anytype",
        }
    }

    pub fn uses_filesystem(self) -> bool {
        matches!(self, BackendKind::Git | BackendKind::Obsidian)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitConfig {
    #[serde(default)]
    pub thoughts_repo: String,
    #[serde(default)]
    pub repos_dir: String,
    #[serde(default)]
    pub global_dir: String,
}

impl GitConfig {
    pub fn standard(thoughts_repo: impl Into<String>) -> Self {
        Self {
            thoughts_repo: thoughts_repo.into(),
            repos_dir: DEFAULT_REPOS_DIR.into(),
            global_dir: DEFAULT_GLOBAL_DIR.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianConfig {
    #[serde(default)]
    pub vault_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_subpath: Option<String>,
    #[serde(default)]
    pub repos_dir: String,
    #[serde(default)]
    pub global_dir: String,
}

impl ObsidianConfig {
    pub fn content_root(&self) -> Option<PathBuf> {
        if self.vault_path.is_empty() {
            return None;
        }
        let vault = expand_path(&self.vault_path);
        Some(
            match self.vault_subpath.as_deref().filter(|s| !s.is_empty()) {
                Some(sub) => vault.join(sub),
                None => vault,
            },
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotionConfig {
    #[serde(default)]
    pub parent_page_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnytypeConfig {
    #[serde(default)]
    pub space_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_token_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BackendConfig {
    Git(GitConfig),
    Obsidian(ObsidianConfig),
    Notion(NotionConfig),
    Anytype(AnytypeConfig),
}

impl Default for BackendConfig {
    fn default() -> Self {
        BackendConfig::Git(GitConfig::default())
    }
}

impl BackendConfig {
    pub fn kind(&self) -> BackendKind {
        match self {
            BackendConfig::Git(_) => BackendKind::Git,
            BackendConfig::Obsidian(_) => BackendKind::Obsidian,
            BackendConfig::Notion(_) => BackendKind::Notion,
            BackendConfig::Anytype(_) => BackendKind::Anytype,
        }
    }

    pub fn thoughts_root(&self) -> Option<PathBuf> {
        match self {
            BackendConfig::Git(g) if !g.thoughts_repo.is_empty() => {
                Some(expand_path(&g.thoughts_repo))
            }
            BackendConfig::Obsidian(o) => o.content_root(),
            _ => None,
        }
    }

    pub fn tree_dirs(&self) -> Option<(&str, &str)> {
        match self {
            BackendConfig::Git(g) => Some((&g.repos_dir, &g.global_dir)),
            BackendConfig::Obsidian(o) => Some((&o.repos_dir, &o.global_dir)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileConfig {
    #[serde(default)]
    pub backend: BackendConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RepoMapping {
    String(String),
    Object {
        repo: String,
        profile: Option<String>,
    },
}

impl RepoMapping {
    pub fn repo(&self) -> &str {
        match self {
            RepoMapping::String(s) => s,
            RepoMapping::Object { repo, .. } => repo,
        }
    }

    pub fn profile(&self) -> Option<&str> {
        match self {
            RepoMapping::String(_) => None,
            RepoMapping::Object { profile, .. } => profile.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsConfig {
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub backend: BackendConfig,
    #[serde(default, deserialize_with = "null_as_default")]
    pub repo_mappings: HashMap<String, RepoMapping>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub profiles: HashMap<String, ProfileConfig>,
}

fn null_as_default<'de, D, T>(de: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(de)?.unwrap_or_default())
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub user: String,
    pub backend: BackendConfig,
    pub profile_name: Option<String>,
    pub mapped_name: Option<String>,
}

impl ThoughtsConfig {
    pub fn is_configured(&self) -> bool {
        if self.user.is_empty() {
            return false;
        }
        match &self.backend {
            BackendConfig::Git(g) => {
                !g.thoughts_repo.is_empty() && !g.repos_dir.is_empty() && !g.global_dir.is_empty()
            }
            BackendConfig::Obsidian(o) => {
                !o.vault_path.is_empty() && !o.repos_dir.is_empty() && !o.global_dir.is_empty()
            }
            BackendConfig::Notion(n) => !n.parent_page_id.is_empty(),
            BackendConfig::Anytype(a) => !a.space_id.is_empty(),
        }
    }

    pub fn effective_config_for(&self, repo_path: &str) -> EffectiveConfig {
        let mapping = self.repo_mappings.get(repo_path);

        let profile_name = mapping
            .and_then(|m| m.profile())
            .filter(|name| self.profiles.contains_key(*name))
            .map(|s| s.to_string());

        let backend = profile_name
            .as_ref()
            .and_then(|n| self.profiles.get(n))
            .map(|p| p.backend.clone())
            .unwrap_or_else(|| self.backend.clone());

        EffectiveConfig {
            user: self.user.clone(),
            backend,
            profile_name,
            mapped_name: mapping.map(|m| m.repo().to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    Claude,
    Copilot,
    OpenCode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    #[serde(default)]
    pub agent_tool: Option<AgentTool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HyprlayerConfig {
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub thoughts: Option<ThoughtsConfig>,
    #[serde(default)]
    pub ai: Option<AiConfig>,
}

pub fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}
