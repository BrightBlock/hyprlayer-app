use std::path::{Path, PathBuf};

use hyprlayer_models::{SkillSourceKind, SkillTarget};
use serde::{Deserialize, Serialize};

use crate::skill_sources::lock::{self, SkillLock};
use crate::skill_sources::{layout, SkillOrigin};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SlashKind {
    Skill,
    Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub source: String,
    pub kind: SlashKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub name: String,
    pub description: String,
    pub source: String,
    pub plugin: Option<String>,
    pub chars: u32,
    pub user_only: bool,
}

pub(crate) const MAX_SKILL_BYTES: u64 = 1024 * 1024;
const MAX_ASSET_DEPTH: usize = 4;
const MAX_ASSETS: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillAsset {
    pub rel_path: String,
    pub bytes: f64,
}

/// A bundled file for the detail viewer: `text` is `None` for a binary or oversized file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillAssetText {
    pub bytes: f64,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SkillFile {
    pub path: String,
    pub dir: String,
    pub source: String,
    pub plugin: Option<String>,
    pub rank: u32,
    pub managed: bool,
    pub name: String,
    pub slash_name: String,
    pub agent_ref: String,
    pub description: String,
    pub user_only: bool,
    pub chars: u32,
    pub bytes: f64,
    pub frontmatter: Vec<SkillField>,
    pub assets: Vec<SkillAsset>,
    pub targets: Vec<SkillTarget>,
    pub installed: bool,
    pub origin: Option<SkillOrigin>,
}

#[derive(Debug, Clone, Default)]
pub struct SkillHome {
    /// Claude's config dir: `CLAUDE_CONFIG_DIR`, else `<home>/.claude`.
    pub claude: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl SkillHome {
    pub fn under(home: &Path) -> Self {
        Self {
            claude: Some(home.join(".claude")),
            home: Some(home.to_path_buf()),
        }
    }
}

/// 1-64 of `[A-Za-z0-9_-]`, first character alphanumeric.
pub fn is_valid_skill_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `launch_plugin` is the server's orchestration plugin, which claude and codex launches get.
pub fn list_slash_commands(
    worktree: &Path,
    home: &SkillHome,
    launch_plugin: Option<&Path>,
) -> Vec<SlashCommand> {
    let mut found: Vec<SlashCommand> = Vec::new();

    let mut roots: Vec<(PathBuf, &str)> = vec![(worktree.join(".claude"), "project")];
    if let Some(claude) = &home.claude {
        roots.push((claude.clone(), "user"));
    }
    for (root, source) in &roots {
        collect_skills(&root.join("skills"), source, &mut found);
        collect_commands(&root.join("commands"), "", source, &mut found);
    }

    collect_plugins(&mut found, home.claude.as_deref());
    if let Some(dir) = launch_plugin {
        collect_launch_plugin(dir, &mut found);
    }

    found.extend(builtin_commands());

    let mut seen = std::collections::HashSet::new();
    found.retain(|c| seen.insert(c.name.clone()));
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

pub fn list_skills(worktree: Option<&Path>, home: &SkillHome) -> Vec<AgentSkill> {
    let mut out: Vec<AgentSkill> = Vec::new();

    let mut roots: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(wt) = worktree {
        roots.push((wt.join(".claude"), "project"));
    }
    if let Some(claude) = &home.claude {
        roots.push((claude.clone(), "user"));
    }
    for (root, source) in &roots {
        for entry in scan_skills(&root.join("skills")) {
            let name = entry.declared.unwrap_or_else(|| entry.dir.clone());
            if out.iter().any(|s| s.name == name) {
                continue;
            }
            out.push(AgentSkill {
                name,
                description: entry.description.unwrap_or_default(),
                source: source.to_string(),
                plugin: None,
                chars: entry.chars,
                user_only: entry.user_only,
            });
        }
    }

    let enabled = enabled_plugins(worktree, home.claude.as_deref());
    for install in plugin_installs(home.claude.as_deref()) {
        if !enabled.contains(&install.key) {
            continue;
        }
        for entry in scan_skills(&install.path.join("skills")) {
            let name = format!("{}:{}", install.label, entry.dir);
            if out.iter().any(|s| s.name == name) {
                continue;
            }
            out.push(AgentSkill {
                name,
                description: entry.description.unwrap_or_default(),
                source: "plugin".into(),
                plugin: Some(install.label.clone()),
                chars: entry.chars,
                user_only: entry.user_only,
            });
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn skill_roots(worktree: Option<&Path>, home: &SkillHome) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(wt) = worktree {
        roots.push(wt.join(".claude/skills"));
    }
    if let Some(claude) = &home.claude {
        roots.push(claude.join("skills"));
    }
    roots.retain(|r| r.is_dir());
    roots
}

struct PluginInstall {
    key: String,
    label: String,
    path: PathBuf,
}

fn plugin_installs(claude: Option<&Path>) -> Vec<PluginInstall> {
    let mut out = Vec::new();
    let Some(claude) = claude else {
        return out;
    };
    let manifest = claude.join("plugins/installed_plugins.json");
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return out;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };
    let Some(plugins) = json.get("plugins").and_then(|p| p.as_object()) else {
        return out;
    };
    for (key, entries) in plugins {
        let label = key.split('@').next().unwrap_or(key).to_string();
        let Some(installs) = entries.as_array() else {
            continue;
        };
        for install in installs {
            if let Some(path) = install.get("installPath").and_then(|p| p.as_str()) {
                out.push(PluginInstall {
                    key: key.clone(),
                    label: label.clone(),
                    path: PathBuf::from(path),
                });
            }
        }
    }
    out
}

fn settings_chain(worktree: Option<&Path>, claude: Option<&Path>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(claude) = claude {
        files.push(claude.join("settings.json"));
    }
    if let Some(wt) = worktree {
        files.push(wt.join(".claude/settings.json"));
        files.push(wt.join(".claude/settings.local.json"));
    }
    files.push(PathBuf::from(if cfg!(target_os = "windows") {
        r"C:\Program Files\ClaudeCode\managed-settings.json"
    } else if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode/managed-settings.json"
    } else {
        "/etc/claude-code/managed-settings.json"
    }));
    files
}

fn enabled_plugins(
    worktree: Option<&Path>,
    claude: Option<&Path>,
) -> std::collections::HashSet<String> {
    let mut on = std::collections::HashSet::new();
    for file in settings_chain(worktree, claude) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(map) = json.get("enabledPlugins").and_then(|p| p.as_object()) else {
            continue;
        };
        for (key, value) in map {
            if value.as_bool().unwrap_or(false) {
                on.insert(key.clone());
            } else {
                on.remove(key);
            }
        }
    }
    on
}

fn collect_plugins(out: &mut Vec<SlashCommand>, claude: Option<&Path>) {
    for install in plugin_installs(claude) {
        collect_skills(&install.path.join("skills"), "plugin", out);
        collect_commands(&install.path.join("commands"), "", "plugin", out);
    }
}

/// Named as claude names a plugin's skills, `<plugin>:<skill>`; the menu offers them to claude and codex only.
fn collect_launch_plugin(dir: &Path, out: &mut Vec<SlashCommand>) {
    let Some(plugin) = std::fs::read(dir.join(".claude-plugin/plugin.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|manifest| manifest.get("name")?.as_str().map(str::to_string))
    else {
        return;
    };
    for entry in scan_skills(&dir.join("skills")) {
        out.push(SlashCommand {
            name: format!("{plugin}:{}", entry.dir),
            description: entry.description.unwrap_or_default(),
            source: "agent-control".to_string(),
            kind: SlashKind::Skill,
        });
    }
}

fn builtin_commands() -> Vec<SlashCommand> {
    const BUILTINS: &[(&str, &str)] = &[
        (
            "init",
            "Initialize a CLAUDE.md with codebase documentation.",
        ),
        ("review", "Review a pull request."),
        (
            "security-review",
            "Review the pending changes on this branch for security issues.",
        ),
        (
            "pr-comments",
            "Fetch and summarize review comments from a GitHub pull request.",
        ),
    ];
    BUILTINS
        .iter()
        .map(|(name, description)| SlashCommand {
            name: name.to_string(),
            description: description.to_string(),
            source: "builtin".to_string(),
            kind: SlashKind::Command,
        })
        .collect()
}

struct SkillEntry {
    dir: String,
    declared: Option<String>,
    description: Option<String>,
    chars: u32,
    user_only: bool,
}

fn scan_skills(dir: &Path) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let skill_file = entry.path().join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&skill_file) else {
            continue;
        };
        let parsed = crate::docs::frontmatter::parse(&text);
        let pick = |key: &str| {
            parsed
                .fm
                .extra
                .get(key)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let body = &text[parsed.body_offset..];
        out.push(SkillEntry {
            dir: entry.file_name().to_string_lossy().to_string(),
            declared: pick("name"),
            description: pick("description").or_else(|| first_prose_line(body)),
            chars: body.chars().count() as u32,
            user_only: pick("disable-model-invocation").as_deref() == Some("true"),
        });
    }
    out
}

pub(crate) fn skill_count(dir: &Path) -> u32 {
    scan_skills(dir).len() as u32
}

fn collect_skills(dir: &Path, source: &str, out: &mut Vec<SlashCommand>) {
    for entry in scan_skills(dir) {
        let name = entry.declared.unwrap_or(entry.dir);
        out.push(SlashCommand {
            name,
            description: entry.description.unwrap_or_default(),
            source: source.to_string(),
            kind: SlashKind::Skill,
        });
    }
}

fn collect_commands(dir: &Path, prefix: &str, source: &str, out: &mut Vec<SlashCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let nested = if prefix.is_empty() {
                file_name
            } else {
                format!("{prefix}:{file_name}")
            };
            collect_commands(&path, &nested, source, out);
        } else if let Some(stem) = file_name.strip_suffix(".md") {
            let name = if prefix.is_empty() {
                stem.to_string()
            } else {
                format!("{prefix}:{stem}")
            };
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let (_, description) = parse_meta(&text);
            out.push(SlashCommand {
                name,
                description: description.unwrap_or_default(),
                source: source.to_string(),
                kind: SlashKind::Command,
            });
        }
    }
}

fn parse_meta(text: &str) -> (Option<String>, Option<String>) {
    let parsed = crate::docs::frontmatter::parse(text);
    let pick = |key: &str| {
        parsed
            .fm
            .extra
            .get(key)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let name = pick("name");
    let description = pick("description").or_else(|| first_prose_line(&text[parsed.body_offset..]));
    (name, description)
}

pub(crate) fn first_prose_line(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('-') && !l.starts_with('#'))
        .map(|l| l.to_string())
}

pub fn list_skill_files(
    worktree: Option<&Path>,
    home: &SkillHome,
    bundle_store: Option<&Path>,
    global_store: Option<&Path>,
) -> Vec<SkillFile> {
    let project_store = worktree.map(layout::project_store);
    let project_lock = worktree.map(|wt| read_lock(&wt.join(".agents/hyprlayer/skills.lock")));
    let global_lock = global_store.map(|s| read_lock(&layout::global_lock_path(s)));
    let project = |root: PathBuf, target| Scan {
        root,
        source: "project",
        rank: 0,
        plugin: None,
        target,
        installed_in: project_store.as_deref().zip(project_lock.as_ref()),
    };
    let user = |root: PathBuf, target| Scan {
        root,
        source: "user",
        rank: 1,
        plugin: None,
        target,
        installed_in: global_store.zip(global_lock.as_ref()),
    };

    let mut found: Vec<SkillFile> = Vec::new();
    if let Some(wt) = worktree {
        found.extend(scan_skill_files(
            &project(wt.join(".claude/skills"), SkillTarget::Claude),
            bundle_store,
        ));
        found.extend(scan_skill_files(
            &project(wt.join(".agents/skills"), SkillTarget::Agents),
            bundle_store,
        ));
    }
    if let Some(claude) = &home.claude {
        found.extend(scan_skill_files(
            &user(claude.join("skills"), SkillTarget::Claude),
            bundle_store,
        ));
    }
    if let Some(root) = &home.home {
        found.extend(scan_skill_files(
            &user(root.join(".agents/skills"), SkillTarget::Agents),
            bundle_store,
        ));
    }

    let enabled = enabled_plugins(worktree, home.claude.as_deref());
    let mut seen = std::collections::HashSet::new();
    for install in plugin_installs(home.claude.as_deref()) {
        if !enabled.contains(&install.key) {
            continue;
        }
        let scan = Scan {
            root: install.path.join("skills"),
            source: "plugin",
            rank: 2,
            plugin: Some(&install.label),
            target: SkillTarget::Claude,
            installed_in: None,
        };
        let rows = scan_skill_files(&scan, bundle_store);
        found.extend(rows.into_iter().filter(|s| seen.insert(s.path.clone())));
    }

    let mut out = collapse_links(found);
    out.sort_by_cached_key(|s| (s.name.to_lowercase(), s.rank, s.path.clone()));
    out
}

/// An unreadable lock lists nothing as installed; install and remove surface the error.
fn read_lock(path: &Path) -> SkillLock {
    lock::read(path).unwrap_or_default()
}

/// One row per real folder: the lowest rank wins, a `.claude` link beating `.agents` on a tie.
fn collapse_links(found: Vec<SkillFile>) -> Vec<SkillFile> {
    let mut index: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    let mut out: Vec<SkillFile> = Vec::new();
    let mut scanned_as: Vec<SkillTarget> = Vec::new();
    for row in found {
        let own = row.targets[0];
        let dir = PathBuf::from(&row.dir);
        let key = std::fs::canonicalize(&dir).unwrap_or(dir);
        let Some(&at) = index.get(&key) else {
            index.insert(key, out.len());
            scanned_as.push(own);
            out.push(row);
            continue;
        };
        let kept = &mut out[at];
        let mut targets = std::mem::take(&mut kept.targets);
        targets.extend(row.targets.iter().copied());
        targets.sort();
        targets.dedup();
        let managed = kept.managed || row.managed;
        let installed = kept.installed || row.installed;
        let origin = kept.origin.take().or_else(|| row.origin.clone());
        if row.rank < kept.rank
            || (row.rank == kept.rank
                && own == SkillTarget::Claude
                && scanned_as[at] != SkillTarget::Claude)
        {
            *kept = row;
            scanned_as[at] = own;
        }
        kept.targets = targets;
        kept.managed = managed;
        kept.installed = installed;
        kept.origin = origin;
    }
    out
}

struct Scan<'a> {
    root: PathBuf,
    source: &'a str,
    rank: u32,
    plugin: Option<&'a str>,
    target: SkillTarget,
    /// The store and lock whose links make a row installed; none for plugins.
    installed_in: Option<(&'a Path, &'a SkillLock)>,
}

fn scan_skill_files(scan: &Scan, store_root: Option<&Path>) -> Vec<SkillFile> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&scan.root) else {
        return out;
    };
    for entry in entries.flatten() {
        // Dot entries are install scratch (`.<folder>.tmp-*`), never skills.
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_file = dir.join("SKILL.md");
        let Ok(meta) = std::fs::metadata(&skill_file) else {
            continue;
        };
        if meta.len() > MAX_SKILL_BYTES {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&skill_file) else {
            continue;
        };
        let parsed = crate::docs::frontmatter::parse(&text);
        let pick = |key: &str| {
            parsed
                .fm
                .extra
                .get(key)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let installed = scan
            .installed_in
            .is_some_and(|(store, lock)| layout::owned(&dir, store, &dir_name, lock));
        let origin = scan
            .installed_in
            .filter(|_| installed)
            .and_then(|(_, lock)| lock.skills.get(&dir_name))
            .and_then(|entry| {
                Some(SkillOrigin {
                    source: SkillSourceKind::parse(&entry.source)?,
                    repo: entry.repo.clone(),
                    path: entry.path.clone(),
                    git_ref: entry.git_ref.clone(),
                    commit: entry.commit.clone(),
                })
            });
        let name = pick("name").unwrap_or_else(|| dir_name.clone());
        let agent_ref = match scan.plugin {
            Some(label) => format!("{label}:{dir_name}"),
            None => name.clone(),
        };
        let body = &text[parsed.body_offset..];
        out.push(SkillFile {
            path: skill_file.to_string_lossy().into_owned(),
            dir: dir.to_string_lossy().into_owned(),
            source: scan.source.to_string(),
            plugin: scan.plugin.map(str::to_string),
            rank: scan.rank,
            managed: store_root.is_some_and(|store| crate::agents::link_targets_store(&dir, store)),
            slash_name: name.clone(),
            name,
            agent_ref,
            description: pick("description")
                .or_else(|| first_prose_line(body))
                .unwrap_or_default(),
            user_only: pick("disable-model-invocation").as_deref() == Some("true"),
            chars: body.chars().count() as u32,
            bytes: meta.len() as f64,
            frontmatter: skill_fields(&text, &parsed.fm.extra),
            assets: skill_assets(&dir),
            targets: vec![scan.target],
            installed,
            origin,
        });
    }
    out
}

pub fn read_skill_file(
    worktree: Option<&Path>,
    home: &SkillHome,
    path: &Path,
) -> crate::error::Result<String> {
    let refuse = || crate::error::CoreError::InvalidPath {
        path: path.to_string_lossy().into_owned(),
    };
    if path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
        return Err(refuse());
    }
    // Lexical containment: bundle skills are symlinks whose targets sit outside every root.
    if !skill_file_roots(worktree, home)
        .iter()
        .any(|root| crate::agents::path_is_within(path, root))
    {
        return Err(refuse());
    }
    if std::fs::metadata(path)?.len() > MAX_SKILL_BYTES {
        return Err(refuse());
    }
    Ok(std::fs::read_to_string(path)?)
}

/// `rel_path` inside the folder of `skill_path` (a SKILL.md the read model lists).
pub fn read_skill_asset(
    worktree: Option<&Path>,
    home: &SkillHome,
    skill_path: &Path,
    rel_path: &str,
) -> crate::error::Result<SkillAssetText> {
    let refuse = || crate::error::CoreError::InvalidPath {
        path: rel_path.to_string(),
    };
    let rel = Path::new(rel_path);
    if rel_path.is_empty()
        || !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err(refuse());
    }
    if skill_path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md")
        || !skill_file_roots(worktree, home)
            .iter()
            .any(|root| crate::agents::path_is_within(skill_path, root))
    {
        return Err(refuse());
    }
    let dir = skill_path.parent().ok_or_else(refuse)?;
    // Canonical containment: a symlink inside the folder must not reach a file outside it.
    let real_dir = std::fs::canonicalize(dir)?;
    let real = std::fs::canonicalize(dir.join(rel))?;
    if !real.starts_with(&real_dir) || !real.is_file() {
        return Err(refuse());
    }
    let len = std::fs::metadata(&real)?.len();
    let text = if len > MAX_SKILL_BYTES {
        None
    } else {
        let raw = std::fs::read(&real)?;
        (!raw.contains(&0))
            .then(|| String::from_utf8(raw).ok())
            .flatten()
    };
    Ok(SkillAssetText {
        bytes: len as f64,
        text,
    })
}

fn skill_file_roots(worktree: Option<&Path>, home: &SkillHome) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(wt) = worktree {
        roots.push(wt.join(".claude/skills"));
        roots.push(wt.join(".agents/skills"));
    }
    if let Some(claude) = &home.claude {
        roots.push(claude.join("skills"));
    }
    if let Some(root) = &home.home {
        roots.push(root.join(".agents/skills"));
    }
    let enabled = enabled_plugins(worktree, home.claude.as_deref());
    roots.extend(
        plugin_installs(home.claude.as_deref())
            .into_iter()
            .filter(|install| enabled.contains(&install.key))
            .map(|install| install.path.join("skills")),
    );
    roots
}

fn skill_fields(
    text: &str,
    recovered: &std::collections::BTreeMap<String, String>,
) -> Vec<SkillField> {
    use crate::docs::frontmatter::{extract_block, tokenize_keys, yaml_value_display};

    let Some((block, _)) = extract_block(text) else {
        return Vec::new();
    };
    if let Ok(map) = serde_yaml::from_str::<serde_yaml::Mapping>(block) {
        return map
            .iter()
            .filter_map(|(key, value)| {
                Some(SkillField {
                    key: key.as_str()?.to_string(),
                    value: yaml_value_display(value),
                })
            })
            .collect();
    }
    let mut seen = std::collections::HashSet::new();
    tokenize_keys(block)
        .into_iter()
        .filter(|key| seen.insert(key.clone()))
        .map(|key| {
            let value = recovered.get(&key).cloned().unwrap_or_default();
            SkillField { key, value }
        })
        .collect()
}

fn skill_assets(dir: &Path) -> Vec<SkillAsset> {
    fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<SkillAsset>) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if out.len() >= MAX_ASSETS {
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel_path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                walk(&entry.path(), &rel_path, depth - 1, out);
            } else if rel_path != "SKILL.md" {
                let Ok(meta) = std::fs::metadata(entry.path()) else {
                    continue;
                };
                if meta.is_file() {
                    out.push(SkillAsset {
                        rel_path,
                        bytes: meta.len() as f64,
                    });
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", MAX_ASSET_DEPTH, &mut out);
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_skill_frontmatter() {
        let text = "---\nname: research_codebase\ndescription: Document the codebase.\nmodel: sonnet\n---\n\n# Research\n";
        let (name, desc) = parse_meta(text);
        assert_eq!(name.as_deref(), Some("research_codebase"));
        assert_eq!(desc.as_deref(), Some("Document the codebase."));
    }

    #[test]
    fn falls_back_to_first_prose_line() {
        let text = "# Heading\n\nDo the thing when asked.\n";
        let (name, desc) = parse_meta(text);
        assert!(name.is_none());
        assert_eq!(desc.as_deref(), Some("Do the thing when asked."));
    }

    #[test]
    fn discovers_project_skill_over_user() {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        for (root, desc) in [(wt, "Project demo."), (home.path(), "User demo.")] {
            let skill_dir = root.join(".claude/skills/demo");
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: demo\ndescription: {desc}\n---\n"),
            )
            .unwrap();
        }
        let cmds = list_slash_commands(wt, &SkillHome::under(home.path()), None);
        let demo: Vec<_> = cmds.iter().filter(|c| c.name == "demo").collect();
        assert_eq!(demo.len(), 1, "{demo:?}");
        assert_eq!(demo[0].source, "project");
        assert_eq!(demo[0].description, "Project demo.");
    }

    #[cfg(unix)]
    #[test]
    fn scan_skills_follows_symlinked_skill_directories() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("stored-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: A symlinked skill.\n---\n\nLinked body.\n",
        )
        .unwrap();

        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::os::unix::fs::symlink(&skill, skills_dir.join("linked")).unwrap();

        let entries = scan_skills(&skills_dir);
        let linked = entries
            .iter()
            .find(|entry| entry.dir == "linked")
            .expect("symlinked skill directory should be scanned");
        assert_eq!(linked.description.as_deref(), Some("A symlinked skill."));
        assert_eq!(linked.chars, "\nLinked body.\n".chars().count() as u32);
    }

    #[test]
    fn includes_builtin_commands() {
        let dir = tempfile::tempdir().unwrap();
        let cmds = list_slash_commands(dir.path(), &SkillHome::default(), None);
        for name in ["init", "review", "security-review"] {
            assert!(
                cmds.iter().any(|c| c.name == name && c.source == "builtin"),
                "missing built-in /{name}",
            );
        }
    }

    #[test]
    fn list_skills_project_and_user_same_name_yields_one_project_entry() {
        let home = tempfile::tempdir().unwrap();
        let user_skill = home.path().join(".claude/skills/demo");
        std::fs::create_dir_all(&user_skill).unwrap();
        std::fs::write(
            user_skill.join("SKILL.md"),
            "---\ndescription: User version.\n---\n\nUser body.\n",
        )
        .unwrap();

        let project_dir = tempfile::tempdir().unwrap();
        let wt = project_dir.path();
        let project_skill = wt.join(".claude/skills/demo");
        std::fs::create_dir_all(&project_skill).unwrap();
        std::fs::write(
            project_skill.join("SKILL.md"),
            "---\ndescription: Project version.\n---\n\nProject body.\n",
        )
        .unwrap();

        let skills = list_skills(Some(wt), &SkillHome::under(home.path()));
        let demo: Vec<_> = skills.iter().filter(|s| s.name == "demo").collect();
        assert_eq!(demo.len(), 1, "expected one demo entry, got {demo:?}");
        assert_eq!(demo[0].source, "project");
        assert_eq!(demo[0].description, "Project version.");
    }

    #[test]
    fn chars_counts_body_not_frontmatter_and_user_only_flag() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();

        let counted_dir = wt.join(".claude/skills/counted");
        std::fs::create_dir_all(&counted_dir).unwrap();
        let text = "---\nname: counted\ndescription: A skill with a longer body.\n---\n\nThis body has some words in it.\n";
        std::fs::write(counted_dir.join("SKILL.md"), text).unwrap();

        let reserved_dir = wt.join(".claude/skills/reserved");
        std::fs::create_dir_all(&reserved_dir).unwrap();
        std::fs::write(
            reserved_dir.join("SKILL.md"),
            "---\nname: reserved\ndescription: User-invoked only.\ndisable-model-invocation: true\n---\n\nBody.\n",
        )
        .unwrap();

        let skills = list_skills(Some(wt), &SkillHome::default());

        let counted = skills.iter().find(|s| s.name == "counted").unwrap();
        let body_offset = crate::docs::frontmatter::parse(text).body_offset;
        assert_eq!(counted.chars, text[body_offset..].chars().count() as u32);
        assert!(!counted.user_only);

        let reserved = skills.iter().find(|s| s.name == "reserved").unwrap();
        assert!(reserved.user_only);
    }

    fn install_plugin(home: &Path) -> tempfile::TempDir {
        let base = home.join(".claude/plugins/cache/mkt/demoplug/1.0.0");
        let skill = base.join("skills/greet");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: Greets.\n---\n\nBody.\n",
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".claude/plugins")).unwrap();
        std::fs::write(
            home.join(".claude/plugins/installed_plugins.json"),
            serde_json::json!({
                "version": 2,
                "plugins": { "demoplug@mkt": [{ "installPath": base }] },
            })
            .to_string(),
        )
        .unwrap();
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn plugin_skills_appear_only_once_the_plugin_is_enabled() {
        let home = tempfile::tempdir().unwrap();
        let project = install_plugin(home.path());
        let wt = project.path();

        let disabled = list_skills(Some(wt), &SkillHome::under(home.path()));
        assert!(
            !disabled.iter().any(|s| s.name == "demoplug:greet"),
            "installed-but-disabled plugin should contribute nothing: {:?}",
            disabled.iter().map(|s| &s.name).collect::<Vec<_>>(),
        );

        std::fs::create_dir_all(wt.join(".claude")).unwrap();
        std::fs::write(
            wt.join(".claude/settings.json"),
            r#"{"enabledPlugins": {"demoplug@mkt": true}}"#,
        )
        .unwrap();

        let enabled = list_skills(Some(wt), &SkillHome::under(home.path()));
        let hit = enabled
            .iter()
            .find(|s| s.name == "demoplug:greet")
            .expect("enabled plugin's skill should be offered, qualified `plugin:skill`");
        assert_eq!(hit.source, "plugin");
        assert_eq!(hit.plugin.as_deref(), Some("demoplug"));
    }

    #[test]
    fn local_settings_can_disable_a_user_enabled_plugin() {
        let home = tempfile::tempdir().unwrap();
        let project = install_plugin(home.path());
        let wt = project.path();

        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"enabledPlugins": {"demoplug@mkt": true}}"#,
        )
        .unwrap();
        assert!(list_skills(Some(wt), &SkillHome::under(home.path()))
            .iter()
            .any(|s| s.name == "demoplug:greet"));

        std::fs::create_dir_all(wt.join(".claude")).unwrap();
        std::fs::write(
            wt.join(".claude/settings.local.json"),
            r#"{"enabledPlugins": {"demoplug@mkt": false}}"#,
        )
        .unwrap();
        assert!(
            !list_skills(Some(wt), &SkillHome::under(home.path()))
                .iter()
                .any(|s| s.name == "demoplug:greet"),
            "local settings should win over the user-level enable",
        );
    }

    #[test]
    fn prompt_commands_excluded_from_list_skills_but_kept_in_slash_commands() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        let commands_dir = wt.join(".claude/commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(
            commands_dir.join("sync.md"),
            "---\ndescription: Sync things.\n---\n\nSync.\n",
        )
        .unwrap();

        let skills = list_skills(Some(wt), &SkillHome::default());
        assert!(!skills.iter().any(|s| s.name == "sync"));

        let cmds = list_slash_commands(wt, &SkillHome::default(), None);
        assert!(cmds.iter().any(|c| c.name == "sync"));
    }

    #[test]
    fn slash_commands_are_tagged_skill_or_command() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        std::fs::create_dir_all(wt.join(".claude/commands")).unwrap();
        std::fs::write(
            wt.join(".claude/commands/sync.md"),
            "---\ndescription: Sync things.\n---\n\nSync.\n",
        )
        .unwrap();
        std::fs::create_dir_all(wt.join(".claude/skills/plan")).unwrap();
        std::fs::write(
            wt.join(".claude/skills/plan/SKILL.md"),
            "---\nname: plan\ndescription: Plan things.\n---\n\nPlan.\n",
        )
        .unwrap();

        let cmds = list_slash_commands(wt, &SkillHome::default(), None);
        let kind = |name: &str| cmds.iter().find(|c| c.name == name).map(|c| c.kind);
        assert_eq!(kind("plan"), Some(SlashKind::Skill));
        assert_eq!(kind("sync"), Some(SlashKind::Command));
        assert_eq!(kind("security-review"), Some(SlashKind::Command));
    }

    #[test]
    fn skill_roots_are_the_skills_dirs_that_exist_project_first() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        assert!(
            !skill_roots(Some(wt), &SkillHome::default())
                .iter()
                .any(|r| r.starts_with(wt)),
            "a worktree with no .claude/skills contributes no root"
        );

        std::fs::create_dir_all(wt.join(".claude/skills")).unwrap();
        let roots = skill_roots(Some(wt), &SkillHome::default());
        assert_eq!(roots.first(), Some(&wt.join(".claude/skills")));
        assert!(roots.iter().all(|r| r.ends_with("skills")));
        assert!(roots.iter().all(|r| r.is_dir()));
    }
}
