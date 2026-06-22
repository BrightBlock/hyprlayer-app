//! Per-repo hyprlayer settings at `<repo>/.hyprlayer/settings.toml`. App-owned,
//! repo-local, safe to commit (distinct from the global CLI config — ADR-0002).
//! Reads use serde; writes use toml_edit so comments + unrelated sections survive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

pub const HYPRLAYER_DIR: &str = ".hyprlayer";
pub const SETTINGS_FILE: &str = "settings.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct RepoSettings {
    pub worktree: WorktreeSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct WorktreeSettings {
    /// Files/globs copied from the repo into each new worktree.
    pub copy: Vec<String>,
}

pub fn settings_path(repo: &Path) -> PathBuf {
    repo.join(HYPRLAYER_DIR).join(SETTINGS_FILE)
}

/// Read repo settings. Absent file → default (empty). Parse error → `RepoSettingsParse`.
pub fn read(repo: &Path) -> Result<RepoSettings> {
    let path = settings_path(repo);
    match std::fs::read_to_string(&path) {
        Ok(raw) => toml::from_str(&raw).map_err(|e| CoreError::RepoSettingsParse {
            path,
            message: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RepoSettings::default()),
        Err(e) => Err(CoreError::Io(e)),
    }
}

/// Overwrite `[worktree].copy`, preserving comments/order/other sections via toml_edit.
pub fn set_worktree_copy(repo: &Path, patterns: &[String]) -> Result<()> {
    let path = settings_path(repo);
    let mut doc = match std::fs::read_to_string(&path) {
        Ok(raw) => {
            raw.parse::<toml_edit::DocumentMut>()
                .map_err(|e| CoreError::RepoSettingsParse {
                    path: path.clone(),
                    message: e.to_string(),
                })?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(CoreError::Io(e)),
    };

    let mut arr = toml_edit::Array::new();
    for p in patterns {
        arr.push(p.as_str());
    }
    doc["worktree"]["copy"] = toml_edit::value(arr);

    // Atomic tmp+rename (mirrors settings.rs:206 discipline).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_repo() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn absent_file_returns_default() {
        let repo = make_repo();
        let settings = read(repo.path()).unwrap();
        assert!(settings.worktree.copy.is_empty());
    }

    #[test]
    fn commented_file_parses() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            b"# this is a comment\n[worktree]\ncopy = [\".env\", \"certs/\"]\n",
        )
        .unwrap();
        let settings = read(repo.path()).unwrap();
        assert_eq!(settings.worktree.copy, vec![".env", "certs/"]);
    }

    #[test]
    fn set_worktree_copy_preserves_comment_and_other_section() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        // Write an initial file with a comment and an unrelated section.
        std::fs::write(
            dir.join("settings.toml"),
            "# repo-level hyprlayer settings\n[other]\nfoo = \"bar\"\n\n[worktree]\ncopy = []\n",
        )
        .unwrap();

        set_worktree_copy(repo.path(), &[".env".to_string(), "certs/".to_string()]).unwrap();

        let raw = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        // Comment must survive.
        assert!(
            raw.contains("# repo-level hyprlayer settings"),
            "comment missing: {raw}"
        );
        // Unrelated section must survive.
        assert!(raw.contains("[other]"), "[other] missing: {raw}");
        assert!(raw.contains("foo = \"bar\""), "foo missing: {raw}");
        // New copy array must be present.
        assert!(raw.contains(".env"), ".env missing: {raw}");
        assert!(raw.contains("certs/"), "certs/ missing: {raw}");

        // Roundtrip via read() as well.
        let settings = read(repo.path()).unwrap();
        assert_eq!(settings.worktree.copy, vec![".env", "certs/"]);
    }

    #[test]
    fn set_worktree_copy_creates_file_when_absent() {
        let repo = make_repo();
        set_worktree_copy(repo.path(), &[".env".to_string()]).unwrap();
        let settings = read(repo.path()).unwrap();
        assert_eq!(settings.worktree.copy, vec![".env"]);
    }

    #[test]
    fn bad_toml_returns_parse_error() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.toml"), b"this is not valid toml {{{{").unwrap();
        let err = read(repo.path()).unwrap_err();
        assert_eq!(err.code(), "repo_settings_parse");
    }
}
