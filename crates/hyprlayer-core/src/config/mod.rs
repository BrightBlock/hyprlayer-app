//! Loading and caching of the hyprlayer CLI config file.
//!
//! Ownership rule: the CLI writes the config, the app reads it. The service
//! re-reads on mtime change (pull-based; callers re-query after any CLI
//! invocation and on window focus).

mod types;

pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::RwLock;

use crate::error::{CoreError, Result};

/// `dirs::config_dir()/hyprlayer/config.json` — matches the CLI's
/// `get_default_config_path` (`~/Library/Application Support` on macOS,
/// `%APPDATA%` on Windows, `~/.config` on Linux).
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("hyprlayer").join("config.json"))
}

/// Parse config JSON, failing loudly when the schema version isn't the one we
/// mirror and tolerating everything else. Pre-v4 files exist in the wild (the
/// CLI migrates lazily on its next run); reading them through the v4 structs
/// would silently misreport a configured user as unconfigured (v1) or fail
/// with a bare serde error (v2/v3), so both directions are dedicated errors
/// with remedies. The CLI's `load()` treats an absent `version` as v0.
pub fn parse_config(raw: &str, path: &Path) -> Result<HyprlayerConfig> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|source| CoreError::ConfigParse {
            path: path.to_path_buf(),
            source,
        })?;

    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version > MAX_SUPPORTED_CONFIG_VERSION {
        return Err(CoreError::ConfigVersionTooNew {
            found: version,
            max: MAX_SUPPORTED_CONFIG_VERSION,
        });
    }
    if version < MAX_SUPPORTED_CONFIG_VERSION {
        return Err(CoreError::ConfigNeedsMigration {
            found: version,
            required: MAX_SUPPORTED_CONFIG_VERSION,
        });
    }

    serde_json::from_value(value).map_err(|source| CoreError::ConfigParse {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Arc<HyprlayerConfig>,
    pub path: PathBuf,
    /// Cache key: (mtime, len). mtime alone misses same-tick rewrites on
    /// coarse-granularity filesystems; length catches most of those.
    pub mtime: Option<SystemTime>,
    pub len: u64,
}

/// TEMPORARY exception to the single-writer rule (ADR 0002): a true first run
/// has no config at all, and `hyprlayer thoughts init --yes` refuses to run
/// without one. Write the minimal v4 config (git backend, `~/thoughts`) and hand
/// ownership straight back to the CLI. Thin wrapper around
/// [`ensure_thoughts_config`] for the existing `AddProjectDialog` first-run path.
/// Delete once upstream `thoughts init` gains a `--user` flag.
pub fn write_bootstrap_config(path: &Path, user: &str) -> Result<()> {
    ensure_thoughts_config(path, user, "~/thoughts")
}

/// Merge-tolerant thoughts-config writer used by the onboarding wizard.
///
/// Three cases:
/// - **Missing** → create a fresh v4 config with the `thoughts` section.
/// - **Exists without `thoughts`** → parse as `serde_json::Value`, inject the
///   `thoughts` object preserving every other key (`ai`, `telemetry`, …), then
///   atomic tmp+rename.
/// - **Exists with `thoughts` already** → error `thoughts_already_configured`
///   (the user should use Settings / the CLI to change it).
pub fn ensure_thoughts_config(path: &Path, user: &str, thoughts_repo: &str) -> Result<()> {
    let user = user.trim();
    if user.is_empty() {
        return Err(CoreError::Io(std::io::Error::other(
            "a username is required to write the hyprlayer config",
        )));
    }

    let thoughts_section = serde_json::json!({
        "user": user,
        "backend": {
            "kind": "git",
            "thoughtsRepo": thoughts_repo,
            "reposDir": "repos",
            "globalDir": "global"
        },
        "repoMappings": {},
        "profiles": {}
    });

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let value = if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        let mut v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| CoreError::ConfigParse {
                path: path.to_path_buf(),
                source: e,
            })?;
        if v.get("thoughts").is_some() {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "thoughts is already configured; use Settings or the CLI to change it",
            )));
        }
        v["thoughts"] = thoughts_section;
        // Ensure version is set to 4 if missing (e.g. an ai-only config written
        // by `ai configure` before version was established).
        if v.get("version").is_none() || v["version"].is_null() {
            v["version"] = serde_json::json!(4);
        }
        v
    } else {
        serde_json::json!({
            "version": 4,
            "thoughts": thoughts_section
        })
    };

    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(&value).expect("static json"),
    )?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Cached config reader. `load()` is cheap to call repeatedly: it re-reads
/// the file only when its mtime changes.
pub struct ConfigService {
    path: PathBuf,
    cache: RwLock<Option<LoadedConfig>>,
}

impl ConfigService {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            cache: RwLock::new(None),
        }
    }

    pub fn with_default_path() -> Result<Self> {
        let path = default_config_path().ok_or_else(|| {
            CoreError::Io(std::io::Error::other(
                "could not determine the OS config directory",
            ))
        })?;
        Ok(Self::new(path))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub async fn load(&self) -> Result<LoadedConfig> {
        let meta = tokio::fs::metadata(&self.path).await.ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);

        if let Some(cached) = self.cache.read().await.as_ref() {
            if cached.mtime.is_some() && cached.mtime == mtime && cached.len == len {
                return Ok(cached.clone());
            }
        }

        let raw = tokio::fs::read_to_string(&self.path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::ConfigMissing {
                    path: self.path.clone(),
                }
            } else {
                CoreError::Io(e)
            }
        })?;

        let config = parse_config(&raw, &self.path)?;
        let loaded = LoadedConfig {
            config: Arc::new(config),
            path: self.path.clone(),
            mtime,
            len,
        };
        *self.cache.write().await = Some(loaded.clone());
        Ok(loaded)
    }

    /// Drop the cache so the next `load()` re-reads from disk (used after CLI
    /// invocations that may have rewritten the file).
    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v4_git_config() {
        let raw = include_str!("../../tests/fixtures/config_v4_git.json");
        let cfg = parse_config(raw, Path::new("test.json")).unwrap();
        assert_eq!(cfg.version, Some(4));
        let thoughts = cfg.thoughts.as_ref().unwrap();
        assert_eq!(thoughts.user, "alice");
        assert!(thoughts.is_configured());
        match &thoughts.backend {
            BackendConfig::Git(g) => {
                assert_eq!(g.thoughts_repo, "~/thoughts");
                assert_eq!(g.repos_dir, "repos");
                assert_eq!(g.global_dir, "global");
            }
            other => panic!("expected git backend, got {other:?}"),
        }
        let eff = thoughts.effective_config_for("/Users/alice/code/myproj");
        assert_eq!(eff.mapped_name.as_deref(), Some("myproj"));
        assert_eq!(eff.profile_name, None);
    }

    #[test]
    fn parses_profile_mappings_and_obsidian() {
        let raw = include_str!("../../tests/fixtures/config_v4_profiles.json");
        let cfg = parse_config(raw, Path::new("test.json")).unwrap();
        let thoughts = cfg.thoughts.as_ref().unwrap();

        // String mapping → default (git) backend.
        let eff = thoughts.effective_config_for("/work/repo-a");
        assert_eq!(eff.mapped_name.as_deref(), Some("repo-a"));
        assert_eq!(eff.backend.kind(), BackendKind::Git);

        // Object mapping with profile → obsidian backend from the profile.
        let eff = thoughts.effective_config_for("/work/repo-b");
        assert_eq!(eff.mapped_name.as_deref(), Some("repo-b"));
        assert_eq!(eff.profile_name.as_deref(), Some("personal"));
        match &eff.backend {
            BackendConfig::Obsidian(o) => {
                assert_eq!(o.vault_path, "~/Obsidian/Vault");
                assert_eq!(o.vault_subpath.as_deref(), Some("hyprlayer"));
                // Path::ends_with matches whole components, so it's correct on
                // Windows (backslashes) without a forward-slash string compare.
                assert!(o
                    .content_root()
                    .unwrap()
                    .ends_with(Path::new("Obsidian/Vault/hyprlayer")));
            }
            other => panic!("expected obsidian backend, got {other:?}"),
        }

        // Unmapped repo → default backend, no mapping.
        let eff = thoughts.effective_config_for("/elsewhere");
        assert_eq!(eff.mapped_name, None);
    }

    #[test]
    fn rejects_future_schema_versions() {
        let raw = r#"{"version": 5}"#;
        match parse_config(raw, Path::new("test.json")) {
            Err(CoreError::ConfigVersionTooNew { found: 5, max: 4 }) => {}
            other => panic!("expected ConfigVersionTooNew, got {other:?}"),
        }
    }

    #[test]
    fn pre_v4_configs_require_migration_instead_of_misparsing() {
        // v1 shape: no version field, flat thoughts keys, no backend object.
        // Reading it through v4 structs would "succeed" with an unconfigured
        // backend — a configured user shown as unconfigured.
        let v1 = r#"{"thoughts": {"user": "alice", "thoughtsRepo": "~/thoughts",
                     "reposDir": "repos", "globalDir": "global"}}"#;
        match parse_config(v1, Path::new("test.json")) {
            Err(CoreError::ConfigNeedsMigration {
                found: 0,
                required: 4,
            }) => {}
            other => panic!("expected ConfigNeedsMigration for v1, got {other:?}"),
        }

        // v2 shape: backend is a bare string, which the tagged enum rejects
        // with an unhelpful serde error if it reaches deserialization.
        let v2 = r#"{"version": 2, "thoughts": {"user": "alice", "backend": "git"}}"#;
        match parse_config(v2, Path::new("test.json")) {
            Err(CoreError::ConfigNeedsMigration {
                found: 2,
                required: 4,
            }) => {}
            other => panic!("expected ConfigNeedsMigration for v2, got {other:?}"),
        }

        // Both carry an actionable remedy.
        let err = parse_config(v2, Path::new("test.json")).unwrap_err();
        assert_eq!(err.code(), "config_needs_migration");
        assert!(err.remedy().is_some());
    }

    #[test]
    fn ensure_thoughts_config_creates_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        ensure_thoughts_config(&path, "alice", "~/thoughts").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["version"], 4);
        assert_eq!(v["thoughts"]["user"], "alice");
        assert_eq!(v["thoughts"]["backend"]["thoughtsRepo"], "~/thoughts");
    }

    #[test]
    fn ensure_thoughts_config_merges_into_ai_only_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Write an ai-only config first (as `ai configure` might create)
        std::fs::write(&path, r#"{"version":4,"ai":{"agentTool":"claude"}}"#).unwrap();
        ensure_thoughts_config(&path, "bob", "~/my-thoughts").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // `ai` section preserved
        assert_eq!(v["ai"]["agentTool"], "claude");
        // `thoughts` section injected
        assert_eq!(v["thoughts"]["user"], "bob");
        assert_eq!(v["thoughts"]["backend"]["thoughtsRepo"], "~/my-thoughts");
    }

    #[test]
    fn ensure_thoughts_config_errors_when_thoughts_already_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        ensure_thoughts_config(&path, "alice", "~/thoughts").unwrap();
        let err = ensure_thoughts_config(&path, "alice", "~/thoughts").unwrap_err();
        assert_eq!(err.code(), "io");
        assert!(err.to_string().contains("already configured"));
    }

    #[test]
    fn tolerates_unknown_fields_and_missing_sections() {
        let raw = r#"{"version": 4, "telemetry": {"mode": "anonymous"}, "futureField": [1,2,3]}"#;
        let cfg = parse_config(raw, Path::new("test.json")).unwrap();
        assert!(cfg.thoughts.is_none());
        assert!(cfg.ai.is_none());
    }
}
