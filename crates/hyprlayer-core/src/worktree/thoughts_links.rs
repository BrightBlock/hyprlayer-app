//! Per-worktree hyprlayer provisioning.
//!
//! A fresh worktree has no `thoughts/` directory (the links are untracked in
//! the main checkout), and the hyprlayer CLI resolves a repo's project by
//! exact cwd path — so each worktree needs (1) the three symlinks re-created
//! and (2) a `repoMappings` entry. The mapping write is the documented
//! ADR-0002 exception #2: a Value-preserving, atomic read-modify-rename edit
//! of the CLI's config that never touches keys the app doesn't own.

use std::path::Path;

use serde::Serialize;

use crate::config::{BackendConfig, EffectiveConfig};
use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    /// Links exist and resolve.
    Ok,
    /// Backend has no filesystem tree (notion/anytype) — nothing to link.
    Unsupported,
    /// Links missing or broken.
    Broken,
}

/// Create `<worktree>/thoughts/{<user>,shared,global}` mirroring the CLI's
/// `setup_symlinks_into` layout. On Windows, falls back to junctions when
/// real symlinks need privileges. Idempotent.
pub fn provision_thoughts(worktree: &Path, effective: &EffectiveConfig) -> Result<LinkStatus> {
    let (repos_dir, global_dir) = match &effective.backend {
        BackendConfig::Git(g) => (g.repos_dir.as_str(), g.global_dir.as_str()),
        BackendConfig::Obsidian(o) => (o.repos_dir.as_str(), o.global_dir.as_str()),
        _ => return Ok(LinkStatus::Unsupported),
    };
    let Some(root) = effective.backend.thoughts_root() else {
        return Ok(LinkStatus::Unsupported);
    };
    let Some(mapped) = effective.mapped_name.as_deref() else {
        return Err(CoreError::OutputParse {
            program: "provision_thoughts".into(),
            detail: "main repository is not mapped in the hyprlayer config".into(),
        });
    };
    if effective.user.is_empty() {
        return Err(CoreError::OutputParse {
            program: "provision_thoughts".into(),
            detail: "hyprlayer config has no user".into(),
        });
    }

    let repo_tree = root
        .join(if repos_dir.is_empty() {
            "repos"
        } else {
            repos_dir
        })
        .join(mapped);
    let global_tree = root.join(if global_dir.is_empty() {
        "global"
    } else {
        global_dir
    });

    let thoughts_dir = worktree.join("thoughts");
    std::fs::create_dir_all(&thoughts_dir)?;

    link_dir(
        &repo_tree.join(&effective.user),
        &thoughts_dir.join(&effective.user),
    )?;
    link_dir(&repo_tree.join("shared"), &thoughts_dir.join("shared"))?;
    link_dir(&global_tree, &thoughts_dir.join("global"))?;

    Ok(check_thoughts(worktree, effective))
}

/// Current health of the links (used by status polling).
pub fn check_thoughts(worktree: &Path, effective: &EffectiveConfig) -> LinkStatus {
    if !effective.backend.kind().uses_filesystem() {
        return LinkStatus::Unsupported;
    }
    let thoughts_dir = worktree.join("thoughts");
    let mut expected = vec!["shared".to_string(), "global".to_string()];
    if !effective.user.is_empty() {
        expected.push(effective.user.clone());
    }
    // exists() follows links, so a dangling symlink reports Broken.
    if expected.iter().all(|name| thoughts_dir.join(name).exists()) {
        LinkStatus::Ok
    } else {
        LinkStatus::Broken
    }
}

fn link_dir(target: &Path, link: &Path) -> Result<()> {
    match std::fs::symlink_metadata(link) {
        Ok(_) => {
            // Replace whatever is there (broken link, stale dir) — the link
            // location is app-owned inside an app-created worktree.
            if link.is_dir() && link.symlink_metadata()?.file_type().is_dir() {
                std::fs::remove_dir_all(link)?;
            } else {
                std::fs::remove_file(link)?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => Ok(()),
            // ERROR_PRIVILEGE_NOT_HELD: no Developer Mode/admin. Junctions
            // need no privilege and are equivalent for absolute local dirs.
            Err(e) if e.raw_os_error() == Some(1314) => {
                junction::create(target, link)?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// Add `repoMappings[worktree] = {repo, profile?}` (or plain string when no
/// profile) to the CLI config, preserving every other byte of structure via
/// serde_json::Value editing + atomic rename.
pub fn add_worktree_mapping(
    config_path: &Path,
    worktree: &Path,
    mapped_name: &str,
    profile: Option<&str>,
) -> Result<()> {
    edit_mappings(config_path, |mappings| {
        let key = worktree.display().to_string();
        let value = match profile {
            Some(p) => serde_json::json!({ "repo": mapped_name, "profile": p }),
            None => serde_json::Value::String(mapped_name.to_string()),
        };
        mappings.insert(key, value);
    })
}

/// Remove the worktree's mapping. Missing entries are fine (the CLI's own
/// orphan cleanup may have raced us).
pub fn remove_worktree_mapping(config_path: &Path, worktree: &Path) -> Result<()> {
    edit_mappings(config_path, |mappings| {
        mappings.remove(&worktree.display().to_string());
    })
}

fn edit_mappings(
    config_path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> Result<()> {
    let raw = std::fs::read_to_string(config_path)?;
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|source| CoreError::ConfigParse {
            path: config_path.to_path_buf(),
            source,
        })?;

    let mappings = value
        .pointer_mut("/thoughts/repoMappings")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| CoreError::OutputParse {
            program: "config edit".into(),
            detail: "config has no thoughts.repoMappings object".into(),
        })?;
    edit(mappings);

    let tmp = config_path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(&value).expect("json value"),
    )?;
    std::fs::rename(&tmp, config_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GitConfig;

    fn effective(root: &Path, user: &str, mapped: Option<&str>) -> EffectiveConfig {
        EffectiveConfig {
            user: user.into(),
            backend: BackendConfig::Git(GitConfig {
                thoughts_repo: root.display().to_string(),
                repos_dir: "repos".into(),
                global_dir: "global".into(),
            }),
            profile_name: None,
            mapped_name: mapped.map(String::from),
        }
    }

    #[test]
    fn provisions_and_checks_links() {
        let thoughts = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        for dir in ["repos/myproj/jt", "repos/myproj/shared", "global"] {
            std::fs::create_dir_all(thoughts.path().join(dir)).unwrap();
        }
        let eff = effective(thoughts.path(), "jt", Some("myproj"));

        let status = provision_thoughts(worktree.path(), &eff).unwrap();
        assert_eq!(status, LinkStatus::Ok);

        // The links resolve to the right targets.
        let user_link = worktree.path().join("thoughts/jt");
        assert!(user_link.join("..").exists());
        std::fs::write(user_link.join("note.md"), "x").unwrap();
        assert!(thoughts.path().join("repos/myproj/jt/note.md").exists());

        // Idempotent.
        assert_eq!(
            provision_thoughts(worktree.path(), &eff).unwrap(),
            LinkStatus::Ok
        );

        // Breaking a target turns the check Broken.
        std::fs::remove_dir_all(thoughts.path().join("repos/myproj/shared")).unwrap();
        assert_eq!(check_thoughts(worktree.path(), &eff), LinkStatus::Broken);
    }

    #[test]
    fn unmapped_repo_errors_and_flat_backends_unsupported() {
        let thoughts = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let eff = effective(thoughts.path(), "jt", None);
        assert!(provision_thoughts(worktree.path(), &eff).is_err());

        let notion = EffectiveConfig {
            user: "jt".into(),
            backend: BackendConfig::Notion(Default::default()),
            profile_name: None,
            mapped_name: Some("myproj".into()),
        };
        assert_eq!(
            provision_thoughts(worktree.path(), &notion).unwrap(),
            LinkStatus::Unsupported
        );
    }

    #[test]
    fn mapping_edits_preserve_unknown_config_content() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        std::fs::write(
            &config_path,
            r#"{
  "version": 4,
  "telemetry": { "mode": "anonymous", "installationId": "abc" },
  "futureKey": [1, 2, 3],
  "thoughts": {
    "user": "jt",
    "backend": { "kind": "git", "thoughtsRepo": "~/thoughts", "reposDir": "repos", "globalDir": "global" },
    "repoMappings": { "/main/repo": "myproj" },
    "profiles": {}
  }
}"#,
        )
        .unwrap();

        add_worktree_mapping(&config_path, Path::new("/wt/auth-ab12"), "myproj", None).unwrap();
        add_worktree_mapping(
            &config_path,
            Path::new("/wt/pay-cd34"),
            "myproj",
            Some("work"),
        )
        .unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        // App-owned edits landed…
        assert_eq!(value["thoughts"]["repoMappings"]["/wt/auth-ab12"], "myproj");
        assert_eq!(
            value["thoughts"]["repoMappings"]["/wt/pay-cd34"]["profile"],
            "work"
        );
        // …and everything the app doesn't own survived byte-meaningfully.
        assert_eq!(value["telemetry"]["installationId"], "abc");
        assert_eq!(value["futureKey"][2], 3);
        assert_eq!(value["thoughts"]["repoMappings"]["/main/repo"], "myproj");

        remove_worktree_mapping(&config_path, Path::new("/wt/auth-ab12")).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(value["thoughts"]["repoMappings"]["/wt/auth-ab12"].is_null());
        // Removing a non-existent mapping is fine.
        remove_worktree_mapping(&config_path, Path::new("/never/was")).unwrap();
    }
}
