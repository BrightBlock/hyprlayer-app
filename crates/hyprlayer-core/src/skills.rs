use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SlashKind {
    Skill,
    Command,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub source: String,
    pub kind: SlashKind,
}

#[derive(Debug, Clone, Serialize)]
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

pub fn list_slash_commands(worktree: &Path, home: Option<&Path>) -> Vec<SlashCommand> {
    let mut found: Vec<SlashCommand> = Vec::new();

    let mut roots: Vec<(PathBuf, &str)> = vec![(worktree.join(".claude"), "project")];
    if let Some(home) = home {
        roots.push((home.join(".claude"), "user"));
    }
    for (root, source) in &roots {
        collect_skills(&root.join("skills"), source, &mut found);
        collect_commands(&root.join("commands"), "", source, &mut found);
    }

    collect_plugins(&mut found, home);

    found.extend(builtin_commands());

    let mut seen = std::collections::HashSet::new();
    found.retain(|c| seen.insert(c.name.clone()));
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

pub fn list_skills(worktree: Option<&Path>, home: Option<&Path>) -> Vec<AgentSkill> {
    let mut out: Vec<AgentSkill> = Vec::new();

    let mut roots: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(wt) = worktree {
        roots.push((wt.join(".claude"), "project"));
    }
    if let Some(home) = home {
        roots.push((home.join(".claude"), "user"));
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

    let enabled = enabled_plugins(worktree, home);
    for install in plugin_installs(home) {
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

pub fn skill_roots(worktree: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(wt) = worktree {
        roots.push(wt.join(".claude/skills"));
    }
    if let Some(home) = home {
        roots.push(home.join(".claude/skills"));
    }
    roots.retain(|r| r.is_dir());
    roots
}

struct PluginInstall {
    key: String,
    label: String,
    path: PathBuf,
}

fn plugin_installs(home: Option<&Path>) -> Vec<PluginInstall> {
    let mut out = Vec::new();
    let Some(home) = home else {
        return out;
    };
    let manifest = home.join(".claude/plugins/installed_plugins.json");
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

fn settings_chain(worktree: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(home) = home {
        files.push(home.join(".claude/settings.json"));
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
    home: Option<&Path>,
) -> std::collections::HashSet<String> {
    let mut on = std::collections::HashSet::new();
    for file in settings_chain(worktree, home) {
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

fn collect_plugins(out: &mut Vec<SlashCommand>, home: Option<&Path>) {
    for install in plugin_installs(home) {
        collect_skills(&install.path.join("skills"), "plugin", out);
        collect_commands(&install.path.join("commands"), "", "plugin", out);
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

fn first_prose_line(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('-') && !l.starts_with('#'))
        .map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TmpHome {
        dir: tempfile::TempDir,
        prev: Option<std::ffi::OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }
    impl TmpHome {
        fn new() -> Self {
            let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let prev = std::env::var_os("HOME");
            std::env::set_var("HOME", dir.path());
            Self {
                dir,
                prev,
                _guard: guard,
            }
        }
        fn path(&self) -> &Path {
            self.dir.path()
        }
    }
    impl Drop for TmpHome {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

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
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        let skill_dir = wt.join(".claude/skills/demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo\ndescription: A demo skill.\n---\n",
        )
        .unwrap();
        let cmds = list_slash_commands(wt, None);
        assert!(cmds
            .iter()
            .any(|c| c.name == "demo" && c.source == "project"));
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
        let cmds = list_slash_commands(dir.path(), None);
        for name in ["init", "review", "security-review"] {
            assert!(
                cmds.iter().any(|c| c.name == name && c.source == "builtin"),
                "missing built-in /{name}",
            );
        }
    }

    #[test]
    fn list_skills_project_and_user_same_name_yields_one_project_entry() {
        let home = TmpHome::new();
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

        let skills = list_skills(Some(wt), Some(home.path()));
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

        let skills = list_skills(Some(wt), None);

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
        let home = TmpHome::new();
        let project = install_plugin(home.path());
        let wt = project.path();

        let disabled = list_skills(Some(wt), Some(home.path()));
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

        let enabled = list_skills(Some(wt), Some(home.path()));
        let hit = enabled
            .iter()
            .find(|s| s.name == "demoplug:greet")
            .expect("enabled plugin's skill should be offered, qualified `plugin:skill`");
        assert_eq!(hit.source, "plugin");
        assert_eq!(hit.plugin.as_deref(), Some("demoplug"));
    }

    #[test]
    fn local_settings_can_disable_a_user_enabled_plugin() {
        let home = TmpHome::new();
        let project = install_plugin(home.path());
        let wt = project.path();

        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"enabledPlugins": {"demoplug@mkt": true}}"#,
        )
        .unwrap();
        assert!(list_skills(Some(wt), Some(home.path()))
            .iter()
            .any(|s| s.name == "demoplug:greet"));

        std::fs::create_dir_all(wt.join(".claude")).unwrap();
        std::fs::write(
            wt.join(".claude/settings.local.json"),
            r#"{"enabledPlugins": {"demoplug@mkt": false}}"#,
        )
        .unwrap();
        assert!(
            !list_skills(Some(wt), Some(home.path()))
                .iter()
                .any(|s| s.name == "demoplug:greet"),
            "local settings should win over the user-level enable",
        );
    }

    #[test]
    fn list_skills_without_worktree_never_panics() {
        let skills = list_skills(None, None);
        assert!(skills.iter().all(|s| s.source != "project"));
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

        let skills = list_skills(Some(wt), None);
        assert!(!skills.iter().any(|s| s.name == "sync"));

        let cmds = list_slash_commands(wt, None);
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

        let cmds = list_slash_commands(wt, None);
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
            !skill_roots(Some(wt), None)
                .iter()
                .any(|r| r.starts_with(wt)),
            "a worktree with no .claude/skills contributes no root"
        );

        std::fs::create_dir_all(wt.join(".claude/skills")).unwrap();
        let roots = skill_roots(Some(wt), None);
        assert_eq!(roots.first(), Some(&wt.join(".claude/skills")));
        assert!(roots.iter().all(|r| r.ends_with("skills")));
        assert!(roots.iter().all(|r| r.is_dir()));
    }
}
