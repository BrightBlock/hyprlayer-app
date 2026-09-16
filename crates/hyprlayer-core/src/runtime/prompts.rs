use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result};
use crate::settings::{CustomPrompt, PromptSettings};
use serde::{Deserialize, Serialize};

use super::HyprlayerRuntime;

pub const PROMPTS_FILE_VERSION: u32 = 1;
pub const PROMPTS_FILE_EXT: &str = "hlprompts";

const MAX_PROMPTS_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Built-in prompt ids paired with the display names an exported file collides
/// against. The ids key `PromptSettings::builtin_bodies`.
const BUILTIN_PROMPTS: [(&str, &str); 2] = [("commit", "Commit"), ("ship", "Ship")];

const MOD_ORDER: [&str; 5] = ["mod", "meta", "ctrl", "alt", "shift"];

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(CoreError::Io(std::io::Error::other(join_err.to_string()))),
    }
}

fn io_context(label: &str, err: std::io::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(format!("{label}: {err}")))
}

fn invalid(detail: &str) -> CoreError {
    CoreError::PromptsFileInvalid {
        detail: detail.to_string(),
    }
}

/// Ported from `src/lib/keybindings/chord.ts` so an imported chord compares and
/// stores in exactly the spelling the frontend would have produced. Only the
/// canonicalisation is shared; capture (`eventToChord`) and rendering
/// (`formatChord`) stay platform-specific and stay on the frontend.
pub fn normalize_chord(raw: &str) -> String {
    let lowered = raw.to_lowercase();
    let parts: Vec<&str> = lowered
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let mods = MOD_ORDER
        .iter()
        .copied()
        .filter(|m| parts.contains(m))
        .map(str::to_string);
    let key = parts
        .iter()
        .rev()
        .find(|part| !MOD_ORDER.contains(part))
        .map(|part| part.to_string());
    mods.chain(key).collect::<Vec<_>>().join("+")
}

#[derive(Debug, Clone)]
pub struct PromptFileEntry {
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PromptsFile {
    pub version: u32,
    pub source: Option<String>,
    pub prompts: Vec<PromptFileEntry>,
}

pub fn parse_prompts_file(text: &str) -> Result<PromptsFile> {
    let raw: serde_json::Value = serde_json::from_str(text).map_err(|_| {
        invalid("This file isn't valid JSON. It may be corrupted or not a prompt export.")
    })?;
    let obj = raw
        .as_object()
        .ok_or_else(|| invalid("This doesn't look like a Hyprlayer prompt file."))?;
    let items = obj
        .get("prompts")
        .and_then(|v| v.as_array())
        .ok_or_else(|| invalid("This doesn't look like a Hyprlayer prompt file."))?;

    let declared = obj.get("hlprompts").and_then(|v| v.as_f64());
    if declared.is_some_and(|v| v > f64::from(PROMPTS_FILE_VERSION)) {
        return Err(invalid(
            "This file was exported by a newer version of Hyprlayer. Update to import it.",
        ));
    }

    let mut prompts = Vec::new();
    for item in items {
        let Some(entry) = item.as_object() else {
            continue;
        };
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let chord = entry
            .get("chord")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        let overrides = entry
            .get("overrides")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(project, body)| {
                        body.as_str().map(|b| (project.clone(), b.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        prompts.push(PromptFileEntry {
            name,
            body: entry
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            chord,
            overrides,
        });
    }

    if prompts.is_empty() {
        return Err(invalid("This prompt file has no prompts to import."));
    }

    Ok(PromptsFile {
        version: declared.map_or(PROMPTS_FILE_VERSION, |v| v as u32),
        source: obj
            .get("source")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        prompts,
    })
}

#[derive(Debug, Clone)]
pub struct PromptImportRequest {
    pub path: String,
    /// Chords already claimed across the app. Built-in prompts resolve theirs
    /// through the frontend keybinding catalog, which core does not own, so the
    /// caller supplies the set rather than core deriving it from settings.
    pub taken_chords: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptConflictKind {
    Builtin,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImportCandidate {
    pub key: String,
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub chord_taken: bool,
    pub unknown_projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImportConflict {
    pub key: String,
    pub kind: PromptConflictKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImportPreview {
    pub source_sha256: String,
    pub source_version: u32,
    pub source: Option<String>,
    pub prompts: Vec<PromptImportCandidate>,
    pub conflicts: Vec<PromptImportConflict>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptImportMode {
    Merge,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptConflictAction {
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImportDecision {
    pub path: String,
    pub expected_source_sha256: String,
    pub mode: PromptImportMode,
    pub conflict_actions: BTreeMap<String, PromptConflictAction>,
    pub taken_chords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptImportResult {
    pub settings: PromptSettings,
    pub added: u32,
    pub replaced: u32,
    pub renamed: u32,
    pub skipped: u32,
    pub first_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptExportEntry {
    pub name: String,
    pub body: String,
    pub chord: Option<String>,
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptExportSelection {
    pub entries: Vec<PromptExportEntry>,
    pub include_chords: bool,
    pub include_overrides: bool,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptExportRequest {
    pub path: String,
    pub selection: PromptExportSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptExportResult {
    pub path: String,
    pub prompt_count: u32,
}

#[derive(serde::Serialize)]
struct WireEntry {
    name: String,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    chord: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overrides: Option<BTreeMap<String, String>>,
}

#[derive(serde::Serialize)]
struct WireFile {
    hlprompts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    prompts: Vec<WireEntry>,
}

fn read_source(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path).map_err(|e| CoreError::PromptsFileUnreadable {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    if meta.len() > MAX_PROMPTS_FILE_BYTES {
        return Err(CoreError::PromptsFileTooLarge);
    }
    std::fs::read_to_string(path).map_err(|e| CoreError::PromptsFileUnreadable {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn builtin_id_for(name: &str) -> Option<&'static str> {
    BUILTIN_PROMPTS
        .iter()
        .find(|(_, builtin)| *builtin == name)
        .map(|(id, _)| *id)
}

fn conflict_for(name: &str, custom: &[CustomPrompt]) -> Option<PromptConflictKind> {
    if builtin_id_for(name).is_some() {
        return Some(PromptConflictKind::Builtin);
    }
    custom
        .iter()
        .any(|c| c.name.trim() == name)
        .then_some(PromptConflictKind::Custom)
}

fn sanitize(mut prompts: PromptSettings) -> PromptSettings {
    prompts
        .builtin_bodies
        .retain(|_, body| !body.trim().is_empty());
    prompts
}

pub struct PromptService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn prompts(&self) -> PromptService<'_> {
        PromptService { runtime: self }
    }
}

impl PromptService<'_> {
    async fn project_path_by_name(&self) -> BTreeMap<String, String> {
        let db = self.runtime.db.clone();
        match blocking(move || db.list_projects()).await {
            Ok(records) => records
                .into_iter()
                .map(|record| (record.name, record.path))
                .collect(),
            Err(e) => {
                tracing::warn!("could not load project registry for prompts: {e}");
                BTreeMap::new()
            }
        }
    }

    pub async fn list(&self) -> Result<PromptSettings> {
        ensure_running(self.runtime)?;
        Ok(self.runtime.settings.get().await.prompts)
    }

    pub async fn preview_import(&self, req: PromptImportRequest) -> Result<PromptImportPreview> {
        ensure_running(self.runtime)?;
        let path = PathBuf::from(&req.path);
        let text = blocking(move || read_source(&path)).await?;
        let source_sha256 = sha256_hex(&text);
        let file = parse_prompts_file(&text)?;

        let custom = self.runtime.settings.get().await.prompts.custom;
        let taken: BTreeSet<String> = req
            .taken_chords
            .iter()
            .map(|chord| normalize_chord(chord))
            .collect();
        let projects = self.project_path_by_name().await;

        let mut prompts = Vec::with_capacity(file.prompts.len());
        let mut conflicts = Vec::new();
        for (index, entry) in file.prompts.iter().enumerate() {
            let key = index.to_string();
            if let Some(kind) = conflict_for(&entry.name, &custom) {
                conflicts.push(PromptImportConflict {
                    key: key.clone(),
                    kind,
                });
            }
            let chord = entry.chord.as_deref().map(normalize_chord);
            prompts.push(PromptImportCandidate {
                chord_taken: chord.as_ref().is_some_and(|c| taken.contains(c)),
                unknown_projects: entry
                    .overrides
                    .keys()
                    .filter(|name| !projects.contains_key(*name))
                    .cloned()
                    .collect(),
                key,
                name: entry.name.clone(),
                body: entry.body.clone(),
                chord,
            });
        }

        Ok(PromptImportPreview {
            source_sha256,
            source_version: file.version,
            source: file.source,
            prompts,
            conflicts,
        })
    }

    pub async fn apply_import(&self, decision: PromptImportDecision) -> Result<PromptImportResult> {
        ensure_running(self.runtime)?;
        let path = PathBuf::from(&decision.path);
        let text = blocking(move || read_source(&path)).await?;
        if sha256_hex(&text) != decision.expected_source_sha256 {
            return Err(CoreError::PromptsSourceChanged);
        }
        let file = parse_prompts_file(&text)?;
        let projects = self.project_path_by_name().await;

        let base = self.runtime.settings.get().await.prompts;
        // Classified against the settings as they were before this import, so a
        // file carrying two entries of one new name adds both rather than
        // letting the first make the second look like a collision.
        let conflicts: Vec<Option<PromptConflictKind>> = file
            .prompts
            .iter()
            .map(|entry| conflict_for(&entry.name, &base.custom))
            .collect();
        let mut builtin_bodies = base.builtin_bodies.clone();
        let mut custom = base.custom.clone();
        let mut taken: BTreeSet<String> = decision
            .taken_chords
            .iter()
            .map(|chord| normalize_chord(chord))
            .collect();

        let default_action = match decision.mode {
            PromptImportMode::Merge => PromptConflictAction::KeepBoth,
            PromptImportMode::Replace => PromptConflictAction::Replace,
        };

        let mut added = 0;
        let mut replaced = 0;
        let mut renamed = 0;
        let mut skipped = 0;
        let mut first_id: Option<String> = None;

        for (index, entry) in file.prompts.iter().enumerate() {
            let key = index.to_string();
            let requested = decision
                .conflict_actions
                .get(&key)
                .copied()
                .unwrap_or(default_action);
            if requested == PromptConflictAction::Skip {
                skipped += 1;
                continue;
            }
            let conflict = conflicts[index];
            let action = if conflict.is_some() {
                requested
            } else {
                PromptConflictAction::KeepBoth
            };

            let overrides: BTreeMap<String, String> = entry
                .overrides
                .iter()
                .filter_map(|(name, body)| {
                    projects.get(name).map(|path| (path.clone(), body.clone()))
                })
                .collect();

            // A chord already spoken for is dropped rather than stolen, and the
            // claim lands as this loop walks the file, so two entries asking for
            // one free chord give it to the earlier of them.
            let mut claim = |chord: Option<&str>, own: Option<&str>| -> Option<String> {
                let canon = normalize_chord(chord?);
                if !taken.contains(&canon) || Some(canon.as_str()) == own {
                    taken.insert(canon.clone());
                    return Some(canon);
                }
                own.map(str::to_string)
            };

            if action == PromptConflictAction::Replace {
                match conflict {
                    Some(PromptConflictKind::Builtin) => {
                        if let Some(id) = builtin_id_for(&entry.name) {
                            builtin_bodies.insert(id.to_string(), entry.body.clone());
                            first_id.get_or_insert_with(|| id.to_string());
                            replaced += 1;
                        }
                    }
                    Some(PromptConflictKind::Custom) => {
                        if let Some(existing) =
                            custom.iter_mut().find(|c| c.name.trim() == entry.name)
                        {
                            let own = existing.chord.as_deref().map(normalize_chord);
                            existing.body = entry.body.clone();
                            existing.overrides.extend(overrides);
                            existing.chord = claim(entry.chord.as_deref(), own.as_deref());
                            first_id.get_or_insert_with(|| existing.id.clone());
                            replaced += 1;
                        }
                    }
                    None => unreachable!("replace is only reachable behind a conflict"),
                }
                continue;
            }

            let id = uuid::Uuid::new_v4().to_string();
            custom.push(CustomPrompt {
                name: if conflict.is_some() {
                    format!("{} (imported)", entry.name)
                } else {
                    entry.name.clone()
                },
                body: entry.body.clone(),
                overrides: overrides.into_iter().collect(),
                chord: claim(entry.chord.as_deref(), None),
                pinned: false,
                id: id.clone(),
            });
            first_id.get_or_insert(id);
            if conflict.is_some() {
                renamed += 1;
            } else {
                added += 1;
            }
        }

        let next = sanitize(PromptSettings {
            builtin_bodies,
            custom,
        });
        let saved = next.clone();
        self.runtime
            .settings
            .update(|settings| settings.prompts = saved)
            .await?;

        Ok(PromptImportResult {
            settings: next,
            added,
            replaced,
            renamed,
            skipped,
            first_id,
        })
    }

    pub async fn export(&self, req: PromptExportRequest) -> Result<PromptExportResult> {
        ensure_running(self.runtime)?;
        let projects = self.project_path_by_name().await;
        let name_by_path: BTreeMap<&str, &str> = projects
            .iter()
            .map(|(name, path)| (path.as_str(), name.as_str()))
            .collect();

        let selection = req.selection;
        let prompts: Vec<WireEntry> = selection
            .entries
            .iter()
            .map(|entry| WireEntry {
                name: entry.name.clone(),
                body: entry.body.clone(),
                chord: selection
                    .include_chords
                    .then(|| entry.chord.clone())
                    .flatten(),
                overrides: selection.include_overrides.then(|| {
                    entry
                        .overrides
                        .iter()
                        .map(|(path, body)| {
                            let name = name_by_path.get(path.as_str()).copied().unwrap_or(path);
                            (name.to_string(), body.clone())
                        })
                        .collect::<BTreeMap<_, _>>()
                }),
            })
            .map(|mut entry| {
                if entry.overrides.as_ref().is_some_and(BTreeMap::is_empty) {
                    entry.overrides = None;
                }
                entry
            })
            .collect();

        let prompt_count = prompts.len() as u32;
        let doc = WireFile {
            hlprompts: PROMPTS_FILE_VERSION,
            source: selection.source.filter(|s| !s.is_empty()),
            prompts,
        };
        let mut content = serde_json::to_string_pretty(&doc)
            .map_err(|e| io_context("serialize prompts", std::io::Error::other(e)))?;
        content.push('\n');

        let path = if req.path.ends_with(&format!(".{PROMPTS_FILE_EXT}")) {
            req.path
        } else {
            format!("{}.{PROMPTS_FILE_EXT}", req.path)
        };
        let target = path.clone();
        blocking(move || {
            std::fs::write(&target, content.as_bytes())
                .map_err(|e| io_context(&format!("write {target}"), e))
        })
        .await?;

        Ok(PromptExportResult { path, prompt_count })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_chord_lowercases_and_orders_modifiers() {
        assert_eq!(normalize_chord("Shift+Mod+K"), "mod+shift+k");
        assert_eq!(normalize_chord("ALT+MOD+COMMA"), "mod+alt+comma");
    }

    #[test]
    fn normalize_chord_is_idempotent() {
        let once = normalize_chord("shift+mod+k");
        assert_eq!(normalize_chord(&once), once);
    }

    #[test]
    fn normalize_chord_tolerates_whitespace_and_stray_separators() {
        assert_eq!(normalize_chord(" mod + shift + k "), "mod+shift+k");
        assert_eq!(normalize_chord("mod++k"), "mod+k");
    }

    #[test]
    fn normalize_chord_keeps_the_last_non_modifier_token() {
        assert_eq!(normalize_chord("mod+a+b"), "mod+b");
        assert_eq!(normalize_chord("mod+shift"), "mod+shift");
    }
}
