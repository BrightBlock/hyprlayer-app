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
    pub scripts: ScriptSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct WorktreeSettings {
    /// Files/globs copied from the repo into each new worktree.
    pub copy: Vec<String>,
}

/// Per-project lifecycle scripts, inherited by every workstream. Each value is a
/// free-form multi-line block of shell commands run in the workstream's worktree
/// root; an empty string means "unconfigured". Run sequentially — a non-zero
/// exit halts the block (see `crate::scripts::build_block`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct ScriptSettings {
    /// Runs once when a new worktree is created.
    pub setup: String,
    /// Runs each time the user presses Run; the prior Run process is stopped first.
    pub run: String,
    /// Runs before a workstream is archived.
    pub archive: String,
}

pub fn settings_path(repo: &Path) -> PathBuf {
    repo.join(HYPRLAYER_DIR).join(SETTINGS_FILE)
}

/// Resolve the settings path, refusing a symlinked `.hyprlayer` directory. The
/// repo is untrusted input: if it ships `.hyprlayer` as a symlink we refuse to
/// follow it (it could redirect a read — or our atomic write — to an out-of-tree
/// location). A real directory, or none yet, is fine.
fn checked_settings_path(repo: &Path) -> Result<PathBuf> {
    let dir = repo.join(HYPRLAYER_DIR);
    if std::fs::symlink_metadata(&dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(CoreError::InvalidPath {
            path: dir.display().to_string(),
        });
    }
    Ok(settings_path(repo))
}

/// Read repo settings. Absent file → default (empty). Parse error → `RepoSettingsParse`.
pub fn read(repo: &Path) -> Result<RepoSettings> {
    let path = checked_settings_path(repo)?;
    match std::fs::read_to_string(&path) {
        Ok(raw) => toml::from_str(&raw).map_err(|e| CoreError::RepoSettingsParse {
            path,
            message: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RepoSettings::default()),
        Err(e) => Err(CoreError::Io(e)),
    }
}

/// Load `<repo>/.hyprlayer/settings.toml` as an editable document (empty if
/// absent), apply `edit` (which receives the document + its path, for error
/// messages), then write it back atomically (tmp + rename) — preserving
/// comments, ordering, and unrelated sections. Shared scaffolding behind the
/// `[worktree]` and `[scripts]` setters.
fn edit_settings_doc(
    repo: &Path,
    edit: impl FnOnce(&mut toml_edit::DocumentMut, &Path) -> Result<()>,
) -> Result<()> {
    let path = checked_settings_path(repo)?;
    let mut doc = match std::fs::read_to_string(&path) {
        Ok(raw) => raw
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| CoreError::RepoSettingsParse {
                path: path.clone(),
                message: e.to_string(),
            })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(CoreError::Io(e)),
    };

    edit(&mut doc, &path)?;

    // Atomic tmp+rename (mirrors settings.rs:206 discipline).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Overwrite `[worktree].copy`, preserving comments/order/other sections via toml_edit.
pub fn set_worktree_copy(repo: &Path, patterns: &[String]) -> Result<()> {
    edit_settings_doc(repo, |doc, _path| {
        let mut arr = toml_edit::Array::new();
        for p in patterns {
            arr.push(p.as_str());
        }
        doc["worktree"]["copy"] = toml_edit::value(arr);
        Ok(())
    })
}

/// Overwrite `[scripts]` (setup/run/archive), preserving comments/order/other
/// sections via toml_edit. Mirrors [`set_worktree_copy`]. Multi-line blocks
/// round-trip (toml_edit renders them as multi-line basic strings).
pub fn set_scripts(repo: &Path, scripts: &ScriptSettings) -> Result<()> {
    edit_settings_doc(repo, |doc, path| {
        // Force a real (non-inline) `[scripts]` table so it renders as a section
        // header — index-assigning a brand-new key on an already-parsed document
        // yields an inline `scripts = { … }` table otherwise.
        let table = doc
            .entry("scripts")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| CoreError::RepoSettingsParse {
                path: path.to_path_buf(),
                message: "[scripts] is not a table".to_string(),
            })?;
        table.set_implicit(false);
        table["setup"] = toml_edit::value(scripts.setup.as_str());
        table["run"] = toml_edit::value(scripts.run.as_str());
        table["archive"] = toml_edit::value(scripts.archive.as_str());
        Ok(())
    })
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

    #[test]
    fn absent_scripts_section_defaults_empty() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.toml"), b"[worktree]\ncopy = [\".env\"]\n").unwrap();
        let settings = read(repo.path()).unwrap();
        assert_eq!(settings.scripts.setup, "");
        assert_eq!(settings.scripts.run, "");
        assert_eq!(settings.scripts.archive, "");
    }

    #[test]
    fn scripts_round_trip_multiline() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            b"[scripts]\nsetup = \"npm install\\ncp .env.example .env.local\"\nrun = \"npm run dev -- --port $HL_PORT\"\narchive = \"docker compose down\"\n",
        )
        .unwrap();
        let s = read(repo.path()).unwrap();
        assert_eq!(s.scripts.setup, "npm install\ncp .env.example .env.local");
        assert_eq!(s.scripts.run, "npm run dev -- --port $HL_PORT");
        assert_eq!(s.scripts.archive, "docker compose down");
    }

    #[test]
    fn set_scripts_preserves_comment_and_worktree() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            "# repo-level hyprlayer settings\n[worktree]\ncopy = [\".env\"]\n",
        )
        .unwrap();

        let scripts = ScriptSettings {
            setup: "npm ci\nnpx playwright install".to_string(),
            run: "npm run dev -- --port $HL_PORT".to_string(),
            archive: String::new(),
        };
        set_scripts(repo.path(), &scripts).unwrap();

        let raw = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(raw.contains("# repo-level hyprlayer settings"), "comment missing: {raw}");
        assert!(raw.contains("[worktree]"), "[worktree] missing: {raw}");
        assert!(raw.contains(".env"), "worktree copy missing: {raw}");
        assert!(raw.contains("[scripts]"), "[scripts] missing: {raw}");

        // Round-trips, including the multi-line setup block.
        let s = read(repo.path()).unwrap();
        assert_eq!(s.scripts.setup, "npm ci\nnpx playwright install");
        assert_eq!(s.scripts.run, "npm run dev -- --port $HL_PORT");
        assert_eq!(s.scripts.archive, "");
        assert_eq!(s.worktree.copy, vec![".env"]);
    }

    #[test]
    fn set_scripts_creates_file_when_absent() {
        let repo = make_repo();
        let scripts = ScriptSettings {
            setup: "echo hi".to_string(),
            ..Default::default()
        };
        set_scripts(repo.path(), &scripts).unwrap();
        let s = read(repo.path()).unwrap();
        assert_eq!(s.scripts.setup, "echo hi");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_hyprlayer_dir_is_refused() {
        let repo = make_repo();
        // A second dir holding a real settings.toml that `.hyprlayer` links to.
        let outside = make_repo();
        std::fs::write(
            outside.path().join("settings.toml"),
            b"[scripts]\nsetup = \"echo pwned\"\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join(".hyprlayer")).unwrap();

        // Reads must refuse the symlinked dir rather than following it out of tree.
        let err = read(repo.path()).unwrap_err();
        assert_eq!(err.code(), "invalid_path");
        // Writes too.
        let err = set_scripts(repo.path(), &ScriptSettings::default()).unwrap_err();
        assert_eq!(err.code(), "invalid_path");
    }
}
