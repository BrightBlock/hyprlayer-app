//! Installing skills from a source into app-owned stores (ADR-0010).

use std::collections::BTreeMap;
use std::path::Path;

use hyprlayer_models::{SkillScope, SkillSourceKind, SkillTarget, StepStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod fetch;
pub mod github;
pub mod install;
pub mod layout;
pub mod lock;
pub mod skillfish;
pub mod skills_sh;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SkillSourceInput {
    #[serde(rename_all = "camelCase")]
    Url { url: String },
    #[serde(rename_all = "camelCase")]
    SkillsSh { source: String, skill_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillSourceRef {
    pub kind: SkillSourceKind,
    pub repo: String,
    /// `<ref>/<path>` as pasted: `ls-remote` decides where the ref ends.
    pub ref_and_path: Option<String>,
    /// A path with no ref, from `owner/repo/path`.
    pub path: Option<String>,
    /// A skills.sh pick's `skillId`: names the folder when no path is given.
    pub name_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillPreviewFile {
    pub rel_path: String,
    pub bytes: f64,
    pub executable: bool,
    pub script: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillTrustFlags {
    pub scripts: bool,
    pub executables: bool,
    pub bash_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SkillConflictReason {
    Reserved,
    Installed,
    Taken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillConflict {
    pub scope: SkillScope,
    pub reason: SkillConflictReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallPreview {
    pub token: String,
    /// The folder name it installs as.
    pub name: String,
    pub description: String,
    /// SKILL.md after its frontmatter, for the review to render. Defaulted so a
    /// server from before this field doesn't drop the client's connection.
    #[serde(default)]
    pub instructions: String,
    pub source: SkillSourceKind,
    pub repo: String,
    pub path: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub commit: String,
    pub hash: String,
    /// Set only when `HYPRLAYER_SKILLS_GIT_BASE` overrides the base.
    pub fetched_from: Option<String>,
    pub files: Vec<SkillPreviewFile>,
    pub flags: SkillTrustFlags,
    pub conflicts: Vec<SkillConflict>,
    /// Set for update and restore-approval previews that replace an installed skill.
    pub changes: Option<SkillChanges>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SkillInstallStepKind {
    Verify,
    Store,
    Link,
    Lock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallStep {
    pub kind: SkillInstallStepKind,
    pub target: Option<SkillTarget>,
    pub status: StepStatus,
    pub detail: Option<String>,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallReport {
    pub name: String,
    pub scope: SkillScope,
    pub steps: Vec<SkillInstallStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillOrigin {
    pub source: SkillSourceKind,
    pub repo: String,
    pub path: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SkillLockState {
    Ok,
    Missing,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillLockStatus {
    /// The folder name.
    pub name: String,
    pub scope: SkillScope,
    pub state: SkillLockState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdate {
    /// The folder name.
    pub name: String,
    pub scope: SkillScope,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SkillRestoreOutcome {
    Ok,
    Relinked,
    Refetched,
    NeedsApproval,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillRestoreEntry {
    /// The folder name.
    pub name: String,
    pub outcome: SkillRestoreOutcome,
    pub preview: Option<SkillInstallPreview>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillRestoreReport {
    pub scope: SkillScope,
    pub entries: Vec<SkillRestoreEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillfishCandidate {
    pub scope: SkillScope,
    pub spec: String,
    pub installed: bool,
}

/// Regular files only, framed `relPath ‖ 0x00 ‖ u64 BE length ‖ bytes` in byte order of
/// the `/`-joined path. The mode bit is excluded so restore agrees across machines.
pub fn content_hash(dir: &Path) -> std::io::Result<String> {
    let mut files = Vec::new();
    collect_files(dir, "", &mut files)?;
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut hasher = Sha256::new();
    for (rel, path) in files {
        let bytes = std::fs::read(path)?;
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    Ok(format!(
        "sha256:{}",
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ))
}

/// Per-file sha256 keyed by `/`-joined relPath, for update change summaries.
pub(crate) fn file_hashes(dir: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let mut files = Vec::new();
    collect_files(dir, "", &mut files)?;
    files
        .into_iter()
        .map(|(rel, path)| {
            let digest = Sha256::digest(std::fs::read(path)?);
            Ok((rel, format!("{digest:x}")))
        })
        .collect()
}

fn collect_files(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(&entry.path(), &rel, out)?;
        } else if kind.is_file() {
            out.push((rel, entry.path()));
        }
    }
    Ok(())
}
