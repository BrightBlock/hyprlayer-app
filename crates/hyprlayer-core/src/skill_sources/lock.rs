use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

const VERSION: u32 = 1;

/// Fields are declared in key order so the output is sorted without `preserve_order`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillLock {
    pub skills: BTreeMap<String, LockEntry>,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockEntry {
    pub commit: String,
    pub hash: String,
    /// Global lock only.
    #[serde(
        rename = "installedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub installed_at: Option<f64>,
    pub path: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub repo: String,
    /// `SkillSourceKind::parse` on read; an unknown value is listed, never updated or restored.
    pub source: String,
    /// `SkillTarget::parse` on read; sorted.
    pub targets: Vec<String>,
}

impl Default for SkillLock {
    fn default() -> Self {
        Self {
            skills: BTreeMap::new(),
            version: VERSION,
        }
    }
}

fn invalid(path: &Path, detail: impl std::fmt::Display) -> CoreError {
    CoreError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{} {detail}", path.display()),
    ))
}

/// A missing file is an empty lock.
pub fn read(path: &Path) -> Result<SkillLock> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SkillLock::default()),
        Err(e) => return Err(e.into()),
    };
    let lock: SkillLock = serde_json::from_str(&text)
        .map_err(|e| invalid(path, format!("is not a valid lock: {e}")))?;
    if lock.version > VERSION {
        return Err(invalid(
            path,
            format!("is version {}, newer than this app supports", lock.version),
        ));
    }
    Ok(lock)
}

/// Atomic, and always called under the scope lock. An empty lock deletes the file.
pub fn write(path: &Path, lock: &SkillLock) -> Result<()> {
    if lock.skills.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("skills.lock"),
        uuid::Uuid::new_v4()
    ));
    let text = serde_json::to_string_pretty(lock).map_err(|e| invalid(path, e))? + "\n";
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}
