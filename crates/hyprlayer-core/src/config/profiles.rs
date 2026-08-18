//! Thoughts profile listing for the Add-project dialog.
//!
//! Listing resolves each destination's existing repo folders so the dialog
//! can offer them. Creation goes through the CLI (`hyprlayer thoughts profile
//! create`, see `HyprCli::thoughts_profile_create`) per ADR-0002's
//! single-writer rule, this module only supplies the app-side name rules the
//! CLI doesn't enforce, and the read-back of what the CLI wrote.

use serde::Serialize;

use crate::error::{CoreError, Result};

use super::{BackendConfig, GitConfig, ThoughtsConfig, DEFAULT_THOUGHTS_REPO};

/// One selectable thoughts destination: the default backend (`name: None`) or
/// a named profile.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ThoughtsProfile {
    /// Profile name; `None` is the default backend (mapped without `--profile`).
    pub name: Option<String>,
    pub backend_kind: String,
    /// Thoughts root as written in config (tilde kept, e.g. `~/thoughts`), for
    /// display; None for backends without a filesystem tree (notion, anytype).
    pub thoughts_repo: Option<String>,
    pub repos_dir: Option<String>,
    pub global_dir: Option<String>,
    /// Existing folder names under `<root>/<reposDir>`, the "existing folder"
    /// choices for the dialog's folder select.
    pub folders: Vec<String>,
    /// Why `folders` couldn't be read (e.g. permission denied), when the tree
    /// exists but is inaccessible. An unreadable tree must not masquerade as
    /// an empty one, the dialog surfaces this next to the folder select.
    pub folders_error: Option<String>,
}

/// All destinations a project can map into: the default backend first, then
/// profiles sorted by name. Walks the filesystem for each entry's folders.
pub fn list_profiles(thoughts: &ThoughtsConfig) -> Vec<ThoughtsProfile> {
    let mut out = vec![profile_entry(None, &thoughts.backend)];
    let mut names: Vec<&String> = thoughts.profiles.keys().collect();
    names.sort();
    out.extend(
        names
            .into_iter()
            .map(|n| profile_entry(Some(n.clone()), &thoughts.profiles[n].backend)),
    );
    out
}

/// The entry a true first run will get once bootstrap writes the minimal
/// config: git backend at `~/thoughts`. Folders are listed when the tree
/// already exists (e.g. a CLI user opening the app for the first time).
pub fn bootstrap_default_profile() -> ThoughtsProfile {
    profile_entry(
        None,
        &BackendConfig::Git(GitConfig::standard(DEFAULT_THOUGHTS_REPO)),
    )
}

fn profile_entry(name: Option<String>, backend: &BackendConfig) -> ThoughtsProfile {
    let dirs = backend.tree_dirs();
    let (folders, folders_error) = existing_folders(backend);
    ThoughtsProfile {
        name,
        backend_kind: backend.kind().as_str().to_string(),
        thoughts_repo: display_root(backend),
        repos_dir: dirs.map(|(repos, _)| repos.to_string()),
        global_dir: dirs.map(|(_, global)| global.to_string()),
        folders,
        folders_error,
    }
}

/// The root as the user wrote it (tilde kept), preview text, not a real path.
fn display_root(backend: &BackendConfig) -> Option<String> {
    match backend {
        BackendConfig::Git(g) if !g.thoughts_repo.is_empty() => Some(g.thoughts_repo.clone()),
        BackendConfig::Obsidian(o) if !o.vault_path.is_empty() => {
            Some(match o.vault_subpath.as_deref().filter(|s| !s.is_empty()) {
                Some(sub) => format!("{}/{sub}", o.vault_path.trim_end_matches('/')),
                None => o.vault_path.clone(),
            })
        }
        _ => None,
    }
}

/// Existing project folders under `<thoughts root>/<reposDir>`, sorted
/// case-insensitively. An absent tree is legitimately empty; any other read
/// failure (permission denied, I/O) is returned as a message instead of
/// being silently flattened to "no folders".
fn existing_folders(backend: &BackendConfig) -> (Vec<String>, Option<String>) {
    let (Some(root), Some((repos_dir, _))) = (backend.thoughts_root(), backend.tree_dirs()) else {
        return (Vec::new(), None);
    };
    if repos_dir.is_empty() {
        return (Vec::new(), None);
    }
    let dir = root.join(repos_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(e) => return (Vec::new(), Some(format!("{}: {e}", dir.display()))),
    };
    let mut folders: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    folders.sort_by_key(|n| n.to_lowercase());
    (folders, None)
}

/// The entry for one named profile, the read-back after the CLI creates it.
pub fn get_profile(thoughts: &ThoughtsConfig, name: &str) -> Option<ThoughtsProfile> {
    thoughts
        .profiles
        .get(name)
        .map(|p| profile_entry(Some(name.to_string()), &p.backend))
}

/// App-side name rules, checked before shelling out to `thoughts profile
/// create`. The CLI is more permissive than the app can represent: it accepts
/// `default` (collides with the built-in default backend) and `__`-prefixed
/// names (the dialog's select sentinels), and silently sanitizes other
/// characters (`bad name` → `bad_name`), rejecting those up front keeps the
/// created name identical to what the user typed.
pub fn validate_profile_name(name: &str) -> Result<()> {
    let invalid = |detail: String| CoreError::ProfileInvalid { detail };
    if name.is_empty() {
        return Err(invalid("a profile name is required".into()));
    }
    if name == "default" {
        return Err(invalid(
            "\"default\" is the built-in default backend. Pick another name".into(),
        ));
    }
    if name.starts_with("__") {
        return Err(invalid(
            "profile names starting with \"__\" are reserved".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(invalid(format!(
            "profile name {name:?} may only use letters, digits, '.', '-' and '_'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config;
    use std::path::Path;

    fn thoughts_from(json: &str) -> ThoughtsConfig {
        let cfg = parse_config(json, Path::new("test.json")).unwrap();
        cfg.thoughts.unwrap()
    }

    #[test]
    fn lists_default_first_then_profiles_sorted() {
        let thoughts = thoughts_from(
            r#"{"version":4,"thoughts":{
                "user":"alice",
                "backend":{"kind":"git","thoughtsRepo":"~/thoughts","reposDir":"repos","globalDir":"global"},
                "repoMappings":{},
                "profiles":{
                    "work":{"backend":{"kind":"git","thoughtsRepo":"~/work-thoughts","reposDir":"repos","globalDir":"global"}},
                    "notes":{"backend":{"kind":"notion","parentPageId":"abc"}}
                }}}"#,
        );
        let profiles = list_profiles(&thoughts);
        let names: Vec<Option<&str>> = profiles.iter().map(|p| p.name.as_deref()).collect();
        assert_eq!(names, vec![None, Some("notes"), Some("work")]);
        assert_eq!(profiles[0].thoughts_repo.as_deref(), Some("~/thoughts"));
        assert_eq!(profiles[0].repos_dir.as_deref(), Some("repos"));
        assert_eq!(profiles[0].folders_error, None);
        // Notion has no filesystem tree: no root, no dirs, no folders.
        assert_eq!(profiles[1].backend_kind, "notion");
        assert_eq!(profiles[1].thoughts_repo, None);
        assert_eq!(profiles[1].repos_dir, None);
        assert!(profiles[1].folders.is_empty());
        assert_eq!(
            profiles[2].thoughts_repo.as_deref(),
            Some("~/work-thoughts")
        );
    }

    #[test]
    fn lists_existing_folders_sorted_dirs_only() {
        let dir = tempfile::tempdir().unwrap();
        let repos = dir.path().join("repos");
        std::fs::create_dir_all(repos.join("beta")).unwrap();
        std::fs::create_dir_all(repos.join("Alpha")).unwrap();
        std::fs::create_dir_all(repos.join(".hidden")).unwrap();
        std::fs::write(repos.join("stray.md"), "not a folder").unwrap();

        let backend = BackendConfig::Git(GitConfig::standard(dir.path().display().to_string()));
        let entry = profile_entry(None, &backend);
        assert_eq!(entry.folders, vec!["Alpha".to_string(), "beta".to_string()]);
        assert_eq!(entry.folders_error, None);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_repos_dir_surfaces_error_instead_of_masquerading_as_empty() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let repos = dir.path().join("repos");
        std::fs::create_dir_all(repos.join("hidden-by-permissions")).unwrap();
        std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o000)).unwrap();

        let backend = BackendConfig::Git(GitConfig::standard(dir.path().display().to_string()));
        let entry = profile_entry(None, &backend);

        // Restore so tempdir cleanup can delete the tree.
        std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(entry.folders.is_empty());
        let msg = entry.folders_error.expect("permission failure surfaces");
        assert!(msg.contains("repos"), "message names the path: {msg}");
    }

    #[test]
    fn obsidian_display_root_joins_vault_subpath() {
        let backend = BackendConfig::Obsidian(super::super::ObsidianConfig {
            vault_path: "~/Vault/".into(),
            vault_subpath: Some("hyprlayer".into()),
            repos_dir: "repos".into(),
            global_dir: "global".into(),
        });
        assert_eq!(display_root(&backend).as_deref(), Some("~/Vault/hyprlayer"));
    }

    #[test]
    fn validates_app_side_name_rules_with_branchable_code() {
        // The CLI accepts these (or silently sanitizes them); the app rejects
        // them up front so the created name always equals what was typed and
        // never collides with the built-in default or the select sentinels.
        for bad in ["", "default", "__auto", "bad name!"] {
            let err = validate_profile_name(bad).unwrap_err();
            assert_eq!(err.code(), "invalid_profile_name", "for {bad:?}");
        }
        assert!(validate_profile_name("work.2-a_b").is_ok());
    }

    #[test]
    fn get_profile_reads_back_one_entry() {
        let thoughts = thoughts_from(
            r#"{"version":4,"thoughts":{
                "user":"alice",
                "backend":{"kind":"git","thoughtsRepo":"~/thoughts","reposDir":"repos","globalDir":"global"},
                "repoMappings":{},
                "profiles":{"work":{"backend":{"kind":"git","thoughtsRepo":"~/work-thoughts","reposDir":"repos","globalDir":"global"}}}}}"#,
        );
        let entry = get_profile(&thoughts, "work").expect("exists");
        assert_eq!(entry.name.as_deref(), Some("work"));
        assert_eq!(entry.thoughts_repo.as_deref(), Some("~/work-thoughts"));
        assert!(get_profile(&thoughts, "nope").is_none());
    }
}
