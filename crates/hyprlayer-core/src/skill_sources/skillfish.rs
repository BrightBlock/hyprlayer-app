//! Read-only `skillfish.json` import: skillfish copies into `.claude/skills`, so we never write it.

use std::path::Path;

use hyprlayer_models::SkillScope;
use serde::Deserialize;

use super::lock::SkillLock;
use super::{github, SkillSourceRef, SkillfishCandidate};

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    skills: Vec<String>,
}

/// Specs that parse as skill links; a missing or malformed file yields none.
pub fn candidates(scope: SkillScope, manifest: &Path, lock: &SkillLock) -> Vec<SkillfishCandidate> {
    let Some(parsed) = std::fs::read_to_string(manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<Manifest>(&text).ok())
    else {
        return Vec::new();
    };
    parsed
        .skills
        .into_iter()
        .filter_map(|spec| {
            let source = github::parse(&spec).ok()?;
            Some(SkillfishCandidate {
                scope,
                installed: is_installed(&source, lock),
                spec,
            })
        })
        .collect()
}

/// Same repo and path as a lock entry; the ref only counts when the spec names one.
fn is_installed(source: &SkillSourceRef, lock: &SkillLock) -> bool {
    lock.skills.values().any(|entry| {
        entry.repo == source.repo
            && match (&source.ref_and_path, &source.path) {
                (Some(rest), _) => {
                    (*rest == entry.git_ref && entry.path.is_empty())
                        || *rest == format!("{}/{}", entry.git_ref, entry.path)
                }
                (None, Some(path)) => *path == entry.path,
                (None, None) => entry.path.is_empty(),
            }
    })
}
