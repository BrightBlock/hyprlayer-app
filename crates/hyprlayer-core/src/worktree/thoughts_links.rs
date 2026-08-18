//! Per-worktree hyprlayer provisioning.
//!
//! A fresh worktree has no `thoughts/` directory (the links are untracked in
//! the main checkout), and the hyprlayer CLI resolves a repo's project by
//! exact cwd path, so each worktree needs (1) the three symlinks re-created
//! and (2) a `repoMappings` entry. The mapping write is the documented
//! ADR-0002 exception #2: a Value-preserving, atomic read-modify-rename edit
//! of the CLI's config that never touches keys the app doesn't own.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{BackendConfig, EffectiveConfig};
use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    /// Links exist and resolve.
    Ok,
    /// Backend has no filesystem tree (notion/anytype), nothing to link.
    Unsupported,
    /// Links missing or broken.
    Broken,
}

/// Expected `<repo>/thoughts/` entries and their targets in the thoughts
/// tree: `<root>/<reposDir>/<mapped>/{<user>,shared}` and `<root>/<globalDir>`.
/// The single source of the link layout, provisioning creates exactly these
/// and the health check verifies against them. None when the backend has no
/// filesystem tree or the repo is unmapped.
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

/// Create `<worktree>/thoughts/{<user>,shared,global}` mirroring the CLI's
/// `setup_symlinks_into` layout, creating the target directories in the
/// thoughts tree when absent (`thoughts init` skips mapped checkouts wholesale,
/// so after a repoint nobody else recreates them). On Windows, falls back to
/// junctions when real symlinks need privileges. Idempotent, replaces
/// whatever occupies the link slots, including stale links into a previous
/// thoughts root.
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
        return Ok(LinkStatus::Unsupported); // unreachable given the guards
    };

    let thoughts_dir = worktree.join(super::THOUGHTS_DIR);
    std::fs::create_dir_all(&thoughts_dir)?;
    for (name, target) in &targets {
        std::fs::create_dir_all(target)?;
        link_dir(target, &thoughts_dir.join(name))?;
    }

    Ok(check_thoughts(worktree, effective))
}

/// Expected entries under `<repo>/thoughts/` that are missing, dangling, or
/// pointing outside the current thoughts tree (stale, e.g. left behind after
/// the config was repointed at a different thoughts repo). The single
/// definition of link health, status polling and the backend health checks
/// both build on it.
pub fn broken_thoughts_links(repo: &Path, effective: &EffectiveConfig) -> Vec<String> {
    let thoughts_dir = repo.join(super::THOUGHTS_DIR);
    match expected_targets(effective) {
        Some(targets) => targets
            .into_iter()
            .filter(|(name, target)| !links_to(&thoughts_dir.join(name), target))
            .map(|(name, _)| name)
            .collect(),
        // No computable targets (e.g. unmapped repo): existence-only checks.
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

/// True when `link` resolves to `target`. Both sides canonicalized so
/// symlinked parents (`/tmp` → `/private/tmp` on macOS) and Windows verbatim
/// `\\?\` junction paths compare equal; a dangling link fails canonicalize
/// and reports broken, same as a missing one.
fn links_to(link: &Path, target: &Path) -> bool {
    match (std::fs::canonicalize(link), std::fs::canonicalize(target)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Current health of the links (used by status polling).
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
        // Replace whatever is there (broken link, stale dir), the link
        // location is app-owned inside an app-created worktree.
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

/// Remove whatever occupies a link slot, without ever recursing into a link's
/// target. Split by platform because a directory symlink/junction must be
/// removed with `remove_dir` on Windows, `remove_file` there fails with
/// "Access is denied", which broke idempotent re-provisioning.
#[cfg(unix)]
fn remove_existing_link(link: &Path, meta: &std::fs::Metadata) -> Result<()> {
    // symlink_metadata: a symlink (even to a dir) is not is_dir(), so only a
    // real directory takes the recursive branch.
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
        // A symlink or junction: drop only the link. Directory reparse points
        // need remove_dir; file symlinks need remove_file.
        if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            std::fs::remove_dir(link)?;
        } else {
            std::fs::remove_file(link)?;
        }
    } else if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        std::fs::remove_dir_all(link)?; // a real directory
    } else {
        std::fs::remove_file(link)?;
    }
    Ok(())
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

        // The links still resolve (the old root exists) but point at the wrong
        // tree, an exists()-style check would call this healthy.
        assert_eq!(
            check_thoughts(worktree.path(), &new_eff),
            LinkStatus::Broken
        );
        assert_eq!(broken_thoughts_links(worktree.path(), &new_eff).len(), 3);

        // Re-provisioning against a completely empty new root creates the
        // target tree, replaces the links in place, and writes land in it.
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

    #[test]
    fn mapping_edits_never_rewrite_foreign_schema_versions() {
        // Shared `edit_config` guard: a v5 (or pre-v4) file that happens to
        // carry repoMappings must error, not be rewritten.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let raw = r#"{"version":5,"thoughts":{"repoMappings":{}}}"#;
        std::fs::write(&config_path, raw).unwrap();

        let err = add_worktree_mapping(&config_path, Path::new("/wt/x"), "x", None).unwrap_err();
        assert_eq!(err.code(), "config_version_too_new");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), raw);
    }
}
