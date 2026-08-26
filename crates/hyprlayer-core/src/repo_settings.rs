use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::branch_prefix::BranchPrefix;
use crate::error::{CoreError, Result};

pub const HYPRLAYER_DIR: &str = ".hyprlayer";
pub const SETTINGS_FILE: &str = "settings.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct RepoSettings {
    pub worktree: WorktreeSettings,
    pub scripts: ScriptSettings,
    pub branches: BranchSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct WorktreeSettings {
    pub copy: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct ScriptSettings {
    pub setup: String,
    pub run: String,
    pub archive: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "kebab-case", default)]
pub struct BranchSettings {
    pub prefixes: Vec<BranchPrefix>,
}

pub fn settings_path(repo: &Path) -> PathBuf {
    repo.join(HYPRLAYER_DIR).join(SETTINGS_FILE)
}

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

fn edit_settings_doc(
    repo: &Path,
    edit: impl FnOnce(&mut toml_edit::DocumentMut, &Path) -> Result<()>,
) -> Result<()> {
    let path = checked_settings_path(repo)?;
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

    edit(&mut doc, &path)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

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

pub fn set_scripts(repo: &Path, scripts: &ScriptSettings) -> Result<()> {
    edit_settings_doc(repo, |doc, path| {
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

pub fn set_branch_prefixes(repo: &Path, prefixes: &[BranchPrefix]) -> Result<()> {
    edit_settings_doc(repo, |doc, path| {
        let table = doc
            .entry("branches")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| CoreError::RepoSettingsParse {
                path: path.to_path_buf(),
                message: "[branches] is not a table".to_string(),
            })?;
        table.set_implicit(false);
        let mut aot = toml_edit::ArrayOfTables::new();
        for p in prefixes {
            let mut t = toml_edit::Table::new();
            t["label"] = toml_edit::value(p.label.as_str());
            t["prefix"] = toml_edit::value(p.prefix.as_str());
            let mut kw = toml_edit::Array::new();
            for k in &p.keywords {
                kw.push(k.as_str());
            }
            t["keywords"] = toml_edit::value(kw);
            aot.push(t);
        }
        table["prefixes"] = toml_edit::Item::ArrayOfTables(aot);
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
        std::fs::write(
            dir.join("settings.toml"),
            "# repo-level hyprlayer settings\n[other]\nfoo = \"bar\"\n\n[worktree]\ncopy = []\n",
        )
        .unwrap();

        set_worktree_copy(repo.path(), &[".env".to_string(), "certs/".to_string()]).unwrap();

        let raw = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(
            raw.contains("# repo-level hyprlayer settings"),
            "comment missing: {raw}"
        );
        assert!(raw.contains("[other]"), "[other] missing: {raw}");
        assert!(raw.contains("foo = \"bar\""), "foo missing: {raw}");
        assert!(raw.contains(".env"), ".env missing: {raw}");
        assert!(raw.contains("certs/"), "certs/ missing: {raw}");

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
        std::fs::write(
            dir.join("settings.toml"),
            b"[worktree]\ncopy = [\".env\"]\n",
        )
        .unwrap();
        let settings = read(repo.path()).unwrap();
        assert_eq!(settings.scripts.setup, "");
        assert_eq!(settings.scripts.run, "");
        assert_eq!(settings.scripts.archive, "");
    }

    #[test]
    fn absent_branches_section_defaults_empty() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            b"[worktree]\ncopy = [\".env\"]\n",
        )
        .unwrap();
        let settings = read(repo.path()).unwrap();
        assert!(settings.branches.prefixes.is_empty());
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
    fn branch_prefixes_round_trip() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            "[[branches.prefixes]]\nlabel = \"Bug fix\"\nprefix = \"fix/\"\nkeywords = [\"fix\", \"bug\"]\n\n[[branches.prefixes]]\nlabel = \"Hotfix\"\nprefix = \"hotfix-\"\nkeywords = []\n",
        )
        .unwrap();
        let settings = read(repo.path()).unwrap();
        assert_eq!(
            settings.branches.prefixes,
            vec![
                BranchPrefix {
                    label: "Bug fix".to_string(),
                    prefix: "fix/".to_string(),
                    keywords: vec!["fix".to_string(), "bug".to_string()],
                },
                BranchPrefix {
                    label: "Hotfix".to_string(),
                    prefix: "hotfix-".to_string(),
                    keywords: vec![],
                },
            ]
        );
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
        assert!(
            raw.contains("# repo-level hyprlayer settings"),
            "comment missing: {raw}"
        );
        assert!(raw.contains("[worktree]"), "[worktree] missing: {raw}");
        assert!(raw.contains(".env"), "worktree copy missing: {raw}");
        assert!(raw.contains("[scripts]"), "[scripts] missing: {raw}");

        let s = read(repo.path()).unwrap();
        assert_eq!(s.scripts.setup, "npm ci\nnpx playwright install");
        assert_eq!(s.scripts.run, "npm run dev -- --port $HL_PORT");
        assert_eq!(s.scripts.archive, "");
        assert_eq!(s.worktree.copy, vec![".env"]);
    }

    #[test]
    fn set_branch_prefixes_preserves_comment_and_other_sections() {
        let repo = make_repo();
        let dir = repo.path().join(".hyprlayer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            "# repo-level hyprlayer settings\n[worktree]\ncopy = [\".env\"]\n\n[scripts]\nsetup = \"npm ci\"\n",
        )
        .unwrap();

        let prefixes = vec![BranchPrefix {
            label: "Bug fix".to_string(),
            prefix: "fix/".to_string(),
            keywords: vec!["fix".to_string(), "bug".to_string()],
        }];
        set_branch_prefixes(repo.path(), &prefixes).unwrap();

        let raw = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(
            raw.contains("# repo-level hyprlayer settings"),
            "comment missing: {raw}"
        );
        assert!(raw.contains("[worktree]"), "[worktree] missing: {raw}");
        assert!(raw.contains(".env"), "worktree copy missing: {raw}");
        assert!(raw.contains("[scripts]"), "[scripts] missing: {raw}");
        assert!(
            raw.contains("[[branches.prefixes]]"),
            "[[branches.prefixes]] missing: {raw}"
        );

        let s = read(repo.path()).unwrap();
        assert_eq!(s.branches.prefixes, prefixes);
        assert_eq!(s.scripts.setup, "npm ci");
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

    #[test]
    fn set_branch_prefixes_creates_file_when_absent() {
        let repo = make_repo();
        let prefixes = vec![BranchPrefix {
            label: "Feature".to_string(),
            prefix: "feat/".to_string(),
            keywords: vec![],
        }];
        set_branch_prefixes(repo.path(), &prefixes).unwrap();
        let s = read(repo.path()).unwrap();
        assert_eq!(s.branches.prefixes, prefixes);
    }

    #[test]
    fn set_branch_prefixes_with_empty_list_reads_back_empty() {
        let repo = make_repo();
        let prefixes = vec![BranchPrefix {
            label: "Feature".to_string(),
            prefix: "feat/".to_string(),
            keywords: vec![],
        }];
        set_branch_prefixes(repo.path(), &prefixes).unwrap();
        set_branch_prefixes(repo.path(), &[]).unwrap();
        let s = read(repo.path()).unwrap();
        assert!(s.branches.prefixes.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_hyprlayer_dir_is_refused() {
        let repo = make_repo();
        let outside = make_repo();
        std::fs::write(
            outside.path().join("settings.toml"),
            b"[scripts]\nsetup = \"echo pwned\"\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join(".hyprlayer")).unwrap();

        let err = read(repo.path()).unwrap_err();
        assert_eq!(err.code(), "invalid_path");
        let err = set_scripts(repo.path(), &ScriptSettings::default()).unwrap_err();
        assert_eq!(err.code(), "invalid_path");
        let err = set_branch_prefixes(repo.path(), &[]).unwrap_err();
        assert_eq!(err.code(), "invalid_path");
    }
}
