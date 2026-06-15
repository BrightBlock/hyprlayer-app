//! Re-declared hyprlayer CLI config types (schema v4).
//!
//! The CLI is a binary-only crate, so these mirror
//! `hyprlayer-cli/src/config.rs` field-for-field. The app treats the config
//! as read-only (the CLI owns mutations); deserialization is tolerant —
//! unknown fields are ignored and everything the app does not strictly need
//! is defaulted. Fixture round-trip tests guard against upstream drift.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Maximum config schema version this app understands.
pub const MAX_SUPPORTED_CONFIG_VERSION: u64 = 4;

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

    /// Filesystem backends create `<repo>/thoughts/` symlinks and store
    /// markdown on disk; flat-list backends delegate I/O to MCP tools.
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
    /// Root directory holding the thoughts tree inside the vault.
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

    /// Filesystem root of the thoughts tree, for backends that have one.
    pub fn thoughts_root(&self) -> Option<PathBuf> {
        match self {
            BackendConfig::Git(g) if !g.thoughts_repo.is_empty() => {
                Some(expand_path(&g.thoughts_repo))
            }
            BackendConfig::Obsidian(o) => o.content_root(),
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
    #[serde(default)]
    pub repo_mappings: HashMap<String, RepoMapping>,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

/// Effective configuration resolved for a specific repository path.
/// Port of `ThoughtsConfig::effective_config_for` in the CLI.
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

    /// Repo mapping keys are the repo's absolute path as a display string —
    /// the CLI uses `current_dir().display().to_string()`, no canonicalization.
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

/// Top-level config. Only the fields the app reads are declared; the rest
/// (telemetry, update bookkeeping) are intentionally ignored on read and
/// never written back.
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

/// Tilde-expansion identical to the CLI's `expand_path`.
pub fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}
