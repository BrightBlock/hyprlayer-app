use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{BackendConfig, EffectiveConfig};
use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    Ok,
    Unsupported,
    Broken,
}

fn expected_targets(effective: &EffectiveConfig) -> Option<Vec<(String, PathBuf)>> {
    let (repos_dir, global_dir) = match &effective.backend {
        BackendConfig::Git(g) => (g.repos_dir.as_str(), g.global_dir.as_str()),
        BackendConfig::Obsidian(o) => (o.repos_dir.as_str(), o.global_dir.as_str()),
        _ => return None,
    };
    let root = effective.backend.thoughts_root()?;
    let mapped = effective.mapped_name.as_deref()?;

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

    let mut targets = Vec::with_capacity(3);
    if !effective.user.is_empty() {
        targets.push((effective.user.clone(), repo_tree.join(&effective.user)));
    }
    targets.push(("shared".to_string(), repo_tree.join("shared")));
    targets.push(("global".to_string(), global_tree));
    Some(targets)
}

pub fn provision_thoughts(worktree: &Path, effective: &EffectiveConfig) -> Result<LinkStatus> {
    if !effective.backend.kind().uses_filesystem() || effective.backend.thoughts_root().is_none() {
        return Ok(LinkStatus::Unsupported);
    }
    if effective.mapped_name.is_none() {
        return Err(CoreError::OutputParse {
            program: "provision_thoughts".into(),
            detail: "main repository is not mapped in the hyprlayer config".into(),
        });
    }
    if effective.user.is_empty() {
        return Err(CoreError::OutputParse {
            program: "provision_thoughts".into(),
            detail: "hyprlayer config has no user".into(),
        });
    }
    let Some(targets) = expected_targets(effective) else {
        return Ok(LinkStatus::Unsupported);
    };

    let thoughts_dir = worktree.join(super::THOUGHTS_DIR);
    std::fs::create_dir_all(&thoughts_dir)?;
    for (name, target) in &targets {
        std::fs::create_dir_all(target)?;
        link_dir(target, &thoughts_dir.join(name))?;
    }

    Ok(check_thoughts(worktree, effective))
}

pub fn broken_thoughts_links(repo: &Path, effective: &EffectiveConfig) -> Vec<String> {
    let thoughts_dir = repo.join(super::THOUGHTS_DIR);
    match expected_targets(effective) {
        Some(targets) => targets
            .into_iter()
            .filter(|(name, target)| !links_to(&thoughts_dir.join(name), target))
            .map(|(name, _)| name)
            .collect(),
        None => {
            let mut expected = vec!["shared".to_string(), "global".to_string()];
            if !effective.user.is_empty() {
                expected.insert(0, effective.user.clone());
            }
            expected
                .into_iter()
                .filter(|name| !thoughts_dir.join(name).exists())
                .collect()
        }
    }
}

fn links_to(link: &Path, target: &Path) -> bool {
    match (std::fs::canonicalize(link), std::fs::canonicalize(target)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub fn check_thoughts(worktree: &Path, effective: &EffectiveConfig) -> LinkStatus {
    if !effective.backend.kind().uses_filesystem() {
        return LinkStatus::Unsupported;
    }
    if broken_thoughts_links(worktree, effective).is_empty() {
        LinkStatus::Ok
    } else {
        LinkStatus::Broken
    }
}

fn link_dir(target: &Path, link: &Path) -> Result<()> {
    match std::fs::symlink_metadata(link) {
        Ok(meta) => remove_existing_link(link, &meta)?,
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
            Err(e) if e.raw_os_error() == Some(1314) => {
                junction::create(target, link)?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(unix)]
fn remove_existing_link(link: &Path, meta: &std::fs::Metadata) -> Result<()> {
    if meta.is_dir() {
        std::fs::remove_dir_all(link)?;
    } else {
        std::fs::remove_file(link)?;
    }
    Ok(())
}

#[cfg(windows)]
fn remove_existing_link(link: &Path, meta: &std::fs::Metadata) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let attrs = meta.file_attributes();
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            std::fs::remove_dir(link)?;
        } else {
            std::fs::remove_file(link)?;
        }
    } else if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        std::fs::remove_dir_all(link)?;
    } else {
        std::fs::remove_file(link)?;
    }
    Ok(())
}

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

pub fn remove_worktree_mapping(config_path: &Path, worktree: &Path) -> Result<()> {
    edit_mappings(config_path, |mappings| {
        mappings.remove(&worktree.display().to_string());
    })
}

fn edit_mappings(
    config_path: &Path,
    edit: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> Result<()> {
    crate::config::edit_config(config_path, |root| {
        let mappings = root
            .get_mut("thoughts")
            .and_then(|t| t.get_mut("repoMappings"))
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| CoreError::OutputParse {
                program: "config edit".into(),
                detail: "config has no thoughts.repoMappings object".into(),
            })?;
        edit(mappings);
        Ok(())
    })
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

    // Why WorkstreamService::reprovision refuses a missing worktree: this call
    // would otherwise recreate the path, leaving a bare symlink farm with no
    // .git that blocks a later restore.
    #[test]
    fn provisioning_a_missing_worktree_recreates_the_directory() {
        let thoughts = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        for dir in ["repos/myproj/jt", "repos/myproj/shared", "global"] {
            std::fs::create_dir_all(thoughts.path().join(dir)).unwrap();
        }
        let eff = effective(thoughts.path(), "jt", Some("myproj"));

        let gone = parent.path().join("deleted-worktree");
        assert!(!gone.exists());

        assert_eq!(provision_thoughts(&gone, &eff).unwrap(), LinkStatus::Ok);
        assert!(gone.join("thoughts/jt").exists());
        assert!(!gone.join(".git").exists());
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

        let user_link = worktree.path().join("thoughts/jt");
        assert!(user_link.join("..").exists());
        std::fs::write(user_link.join("note.md"), "x").unwrap();
        assert!(thoughts.path().join("repos/myproj/jt/note.md").exists());

        assert_eq!(
            provision_thoughts(worktree.path(), &eff).unwrap(),
            LinkStatus::Ok
        );

        std::fs::remove_dir_all(thoughts.path().join("repos/myproj/shared")).unwrap();
        assert_eq!(check_thoughts(worktree.path(), &eff), LinkStatus::Broken);
    }

    #[test]
    fn repointed_root_marks_resolving_links_stale_until_reprovisioned() {
        let old_root = tempfile::tempdir().unwrap();
        let new_root = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        for dir in ["repos/myproj/jt", "repos/myproj/shared", "global"] {
            std::fs::create_dir_all(old_root.path().join(dir)).unwrap();
        }
        let old_eff = effective(old_root.path(), "jt", Some("myproj"));
        let new_eff = effective(new_root.path(), "jt", Some("myproj"));

        assert_eq!(
            provision_thoughts(worktree.path(), &old_eff).unwrap(),
            LinkStatus::Ok
        );

        assert_eq!(
            check_thoughts(worktree.path(), &new_eff),
            LinkStatus::Broken
        );
        assert_eq!(broken_thoughts_links(worktree.path(), &new_eff).len(), 3);

        assert_eq!(
            provision_thoughts(worktree.path(), &new_eff).unwrap(),
            LinkStatus::Ok
        );
        assert_eq!(
            check_thoughts(worktree.path(), &old_eff),
            LinkStatus::Broken
        );
        std::fs::write(worktree.path().join("thoughts/shared/note.md"), "x").unwrap();
        assert!(new_root.path().join("repos/myproj/shared/note.md").exists());
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
        assert_eq!(value["thoughts"]["repoMappings"]["/wt/auth-ab12"], "myproj");
        assert_eq!(
            value["thoughts"]["repoMappings"]["/wt/pay-cd34"]["profile"],
            "work"
        );
        assert_eq!(value["telemetry"]["installationId"], "abc");
        assert_eq!(value["futureKey"][2], 3);
        assert_eq!(value["thoughts"]["repoMappings"]["/main/repo"], "myproj");

        remove_worktree_mapping(&config_path, Path::new("/wt/auth-ab12")).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(value["thoughts"]["repoMappings"]["/wt/auth-ab12"].is_null());
        remove_worktree_mapping(&config_path, Path::new("/never/was")).unwrap();
    }

    #[test]
    fn mapping_edits_never_rewrite_foreign_schema_versions() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let raw = r#"{"version":5,"thoughts":{"repoMappings":{}}}"#;
        std::fs::write(&config_path, raw).unwrap();

        let err = add_worktree_mapping(&config_path, Path::new("/wt/x"), "x", None).unwrap_err();
        assert_eq!(err.code(), "config_version_too_new");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), raw);
    }
}
