mod profiles;
mod types;

pub use profiles::*;
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::RwLock;

use crate::error::{CoreError, Result};

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
    pub mtime: Option<SystemTime>,
    pub len: u64,
}

pub fn write_bootstrap_config(path: &Path, user: &str) -> Result<()> {
    ensure_thoughts_config(path, user, DEFAULT_THOUGHTS_REPO)
}

static CONFIG_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn ensure_thoughts_config(path: &Path, user: &str, thoughts_repo: &str) -> Result<()> {
    let _write = CONFIG_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let user = user.trim();
    if user.is_empty() {
        return Err(CoreError::Io(std::io::Error::other(
            "a username is required to write the hyprlayer config",
        )));
    }

    let git_backend = serde_json::json!({
        "kind": "git",
        "thoughtsRepo": thoughts_repo,
        "reposDir": DEFAULT_REPOS_DIR,
        "globalDir": DEFAULT_GLOBAL_DIR
    });

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let value = if path.exists() {
        let raw = std::fs::read_to_string(path)?;
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&raw).map_err(|e| CoreError::ConfigParse {
                path: path.to_path_buf(),
                source: e,
            })?;
        let mut v = serde_json::Value::Object(map);

        let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(0);
        if version > MAX_SUPPORTED_CONFIG_VERSION {
            return Err(CoreError::ConfigVersionTooNew {
                found: version,
                max: MAX_SUPPORTED_CONFIG_VERSION,
            });
        }
        let has_thoughts = v.get("thoughts").is_some_and(|t| t.is_object());
        if has_thoughts && version < MAX_SUPPORTED_CONFIG_VERSION {
            return Err(CoreError::ConfigNeedsMigration {
                found: version,
                required: MAX_SUPPORTED_CONFIG_VERSION,
            });
        }

        if has_thoughts {
            let t = &mut v["thoughts"];
            t["user"] = user.into();
            if t["backend"]["kind"] == "git" {
                let b = &mut t["backend"];
                b["thoughtsRepo"] = thoughts_repo.into();
                for (key, default) in [
                    ("reposDir", DEFAULT_REPOS_DIR),
                    ("globalDir", DEFAULT_GLOBAL_DIR),
                ] {
                    if b[key].as_str().is_none_or(str::is_empty) {
                        b[key] = default.into();
                    }
                }
            } else {
                t["backend"] = git_backend;
            }
            for key in ["repoMappings", "profiles"] {
                if !t[key].is_object() {
                    t[key] = serde_json::json!({});
                }
            }
        } else {
            v["thoughts"] = serde_json::json!({
                "user": user,
                "backend": git_backend,
                "repoMappings": {},
                "profiles": {}
            });
        }
        if version < MAX_SUPPORTED_CONFIG_VERSION {
            v["version"] = serde_json::json!(MAX_SUPPORTED_CONFIG_VERSION);
        }
        v
    } else {
        serde_json::json!({
            "version": MAX_SUPPORTED_CONFIG_VERSION,
            "thoughts": {
                "user": user,
                "backend": git_backend,
                "repoMappings": {},
                "profiles": {}
            }
        })
    };

    write_config_atomic(path, &value)
}

fn write_config_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(value).expect("json value"),
    )?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn edit_config(
    path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<()>,
) -> Result<()> {
    let _write = CONFIG_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let raw = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CoreError::ConfigMissing {
                path: path.to_path_buf(),
            }
        } else {
            CoreError::Io(e)
        }
    })?;
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|source| CoreError::ConfigParse {
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

    let root = value
        .as_object_mut()
        .ok_or_else(|| CoreError::OutputParse {
            program: "config edit".into(),
            detail: "config root is not an object".into(),
        })?;
    edit(root)?;

    write_config_atomic(path, &value)
}

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

        let eff = thoughts.effective_config_for("/work/repo-a");
        assert_eq!(eff.mapped_name.as_deref(), Some("repo-a"));
        assert_eq!(eff.backend.kind(), BackendKind::Git);

        let eff = thoughts.effective_config_for("/work/repo-b");
        assert_eq!(eff.mapped_name.as_deref(), Some("repo-b"));
        assert_eq!(eff.profile_name.as_deref(), Some("personal"));
        match &eff.backend {
            BackendConfig::Obsidian(o) => {
                assert_eq!(o.vault_path, "~/Obsidian/Vault");
                assert_eq!(o.vault_subpath.as_deref(), Some("hyprlayer"));
                assert!(o
                    .content_root()
                    .unwrap()
                    .ends_with(Path::new("Obsidian/Vault/hyprlayer")));
            }
            other => panic!("expected obsidian backend, got {other:?}"),
        }

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
        let v1 = r#"{"thoughts": {"user": "alice", "thoughtsRepo": "~/thoughts",
                     "reposDir": "repos", "globalDir": "global"}}"#;
        match parse_config(v1, Path::new("test.json")) {
            Err(CoreError::ConfigNeedsMigration {
                found: 0,
                required: 4,
            }) => {}
            other => panic!("expected ConfigNeedsMigration for v1, got {other:?}"),
        }

        let v2 = r#"{"version": 2, "thoughts": {"user": "alice", "backend": "git"}}"#;
        match parse_config(v2, Path::new("test.json")) {
            Err(CoreError::ConfigNeedsMigration {
                found: 2,
                required: 4,
            }) => {}
            other => panic!("expected ConfigNeedsMigration for v2, got {other:?}"),
        }

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
        std::fs::write(&path, r#"{"version":4,"ai":{"agentTool":"claude"}}"#).unwrap();
        ensure_thoughts_config(&path, "bob", "~/my-thoughts").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["ai"]["agentTool"], "claude");
        assert_eq!(v["thoughts"]["user"], "bob");
        assert_eq!(v["thoughts"]["backend"]["thoughtsRepo"], "~/my-thoughts");
    }

    #[test]
    fn ensure_thoughts_config_repoints_existing_thoughts_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"version":4,"ai":{"agentTool":"claude"},"thoughts":{
                "user":"alice",
                "backend":{"kind":"git","thoughtsRepo":"~/old-thoughts","reposDir":"custom","globalDir":"shared"},
                "repoMappings":{"/w/a":"a"},
                "profiles":{"personal":{}}}}"#,
        )
        .unwrap();
        ensure_thoughts_config(&path, "bob", "/data/thoughts").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["thoughts"]["user"], "bob");
        assert_eq!(v["thoughts"]["backend"]["thoughtsRepo"], "/data/thoughts");
        assert_eq!(v["thoughts"]["backend"]["reposDir"], "custom");
        assert_eq!(v["thoughts"]["backend"]["globalDir"], "shared");
        assert_eq!(v["thoughts"]["repoMappings"]["/w/a"], "a");
        assert!(v["thoughts"]["profiles"]["personal"].is_object());
        assert_eq!(v["ai"]["agentTool"], "claude");
    }

    #[test]
    fn ensure_thoughts_config_completes_half_written_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"version":4,"thoughts":{"user":"","backend":{"kind":"git","thoughtsRepo":"","reposDir":"","globalDir":""}}}"#,
        )
        .unwrap();
        ensure_thoughts_config(&path, "alice", "~/thoughts").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let cfg = parse_config(&raw, &path).unwrap();
        assert!(cfg.thoughts.as_ref().unwrap().is_configured());
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["thoughts"]["backend"]["reposDir"], "repos");
        assert_eq!(v["thoughts"]["repoMappings"], serde_json::json!({}));
    }

    #[test]
    fn ensure_thoughts_config_replaces_non_git_backend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"version":4,"thoughts":{
                "user":"alice",
                "backend":{"kind":"obsidian","vaultPath":"~/Vault","reposDir":"repos","globalDir":"global"},
                "repoMappings":{"/w/a":"a"},"profiles":{}}}"#,
        )
        .unwrap();
        ensure_thoughts_config(&path, "alice", "~/thoughts").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["thoughts"]["backend"]["kind"], "git");
        assert_eq!(v["thoughts"]["backend"]["thoughtsRepo"], "~/thoughts");
        assert_eq!(v["thoughts"]["repoMappings"]["/w/a"], "a");
    }

    #[test]
    fn ensure_thoughts_config_never_rewrites_foreign_versions() {
        let dir = tempfile::tempdir().unwrap();

        let old = dir.path().join("old.json");
        std::fs::write(
            &old,
            r#"{"version":2,"thoughts":{"user":"alice","backend":"git"}}"#,
        )
        .unwrap();
        let err = ensure_thoughts_config(&old, "alice", "~/thoughts").unwrap_err();
        assert_eq!(err.code(), "config_needs_migration");

        let new = dir.path().join("new.json");
        let raw = r#"{"version":5,"thoughts":{"user":"alice"}}"#;
        std::fs::write(&new, raw).unwrap();
        let err = ensure_thoughts_config(&new, "alice", "~/thoughts").unwrap_err();
        assert_eq!(err.code(), "config_version_too_new");
        assert_eq!(std::fs::read_to_string(&new).unwrap(), raw);
    }

    #[test]
    fn tolerates_unknown_fields_and_missing_sections() {
        let raw = r#"{"version": 4, "telemetry": {"mode": "anonymous"}, "futureField": [1,2,3]}"#;
        let cfg = parse_config(raw, Path::new("test.json")).unwrap();
        assert!(cfg.thoughts.is_none());
    }

    #[test]
    fn tolerates_cli_emitted_nulls_for_empty_maps() {
        let raw = r#"{"version": 4, "thoughts": {
            "user": "jt",
            "backend": {"kind": "git", "thoughtsRepo": "~/thoughts", "reposDir": "repos", "globalDir": "global"},
            "repoMappings": null,
            "profiles": null}}"#;
        let cfg = parse_config(raw, Path::new("test.json")).unwrap();
        let thoughts = cfg.thoughts.unwrap();
        assert!(thoughts.profiles.is_empty());
        assert!(thoughts.repo_mappings.is_empty());
        assert!(thoughts.is_configured());
    }
}
