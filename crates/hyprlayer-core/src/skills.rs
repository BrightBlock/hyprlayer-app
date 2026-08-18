//! Discovery of the slash commands claude exposes in a worktree, so the
//! composer can autocomplete `/skill` the way the CLI does. Also exposes
//! skills alone, keyed by project path, for the Agents view's `skills:`
//! frontmatter picker.
//!
//! We mirror claude's own sources: skills live under `<root>/.claude/skills/
//! <name>/SKILL.md` and prompt commands under `<root>/.claude/commands/**.md`,
//! at both the project (worktree) and user (`~/.claude`) level. Project entries
//! win over user entries of the same name. We intentionally read the same files
//! claude reads rather than maintaining a hardcoded list, so what the user sees
//! offered is what claude will actually resolve.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One slash command offered in the composer's `/` autocomplete.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    /// What the user types after `/`, e.g. `research_codebase`.
    pub name: String,
    /// One-line summary from the file's frontmatter (or first prose line).
    pub description: String,
    /// `project` | `user`, which `.claude` root supplied it (for the label).
    pub source: String,
}

/// One skill an agent's `skills:` key can name.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// The canonical name claude resolves: the skill's directory, or
    /// `plugin:skill` for a plugin's.
    pub name: String,
    pub description: String,
    /// `project` | `user` | `plugin`.
    pub source: String,
    pub plugin: Option<String>,
    /// Body length in characters. Preloading injects the whole body on every
    /// spawn, so this is a recurring context cost, not a one-off, the card
    /// renders it rather than letting a toggle quietly cost thousands of tokens.
    pub chars: u32,
    /// The skill sets `disable-model-invocation: true`, its author reserved it
    /// for the user to invoke. Preloading injects it anyway, so the card says so.
    pub user_only: bool,
}

/// All slash commands available in `worktree`, deduped and sorted by name.
/// Never errors: a missing/unreadable source just contributes nothing, since
/// the composer degrades to a plain textarea.
///
/// Sources, in precedence order (first wins on a name clash): project `.claude`,
/// user `~/.claude`, installed plugins, then the curated built-ins baked into
/// the claude binary. This mirrors what claude itself resolves a `/command` to.
pub fn list_slash_commands(worktree: &Path) -> Vec<SlashCommand> {
    let mut found: Vec<SlashCommand> = Vec::new();

    // 1. Project root first, then user, project entries take precedence.
    let mut roots: Vec<(PathBuf, &str)> = vec![(worktree.join(".claude"), "project")];
    if let Some(home) = dirs::home_dir() {
        roots.push((home.join(".claude"), "user"));
    }
    for (root, source) in &roots {
        collect_skills(&root.join("skills"), source, &mut found);
        collect_commands(&root.join("commands"), "", source, &mut found);
    }

    // 2. Installed plugins' own commands and skills.
    collect_plugins(&mut found);

    // 3. Built-ins that ship inside the binary (not discoverable on disk).
    found.extend(builtin_commands());

    // First writer wins (the precedence order above); drop later dupes by name.
    let mut seen = std::collections::HashSet::new();
    found.retain(|c| seen.insert(c.name.clone()));
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Every skill visible from `worktree`, project first, then user, then plugins.
/// Sorted by name; a name already claimed by an earlier source is skipped,
/// claude's own precedence. Prompt commands never appear here, claude's
/// `skills:` key can't preload them.
pub fn list_skills(worktree: Option<&Path>) -> Vec<AgentSkill> {
    let mut out: Vec<AgentSkill> = Vec::new();

    let mut roots: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(wt) = worktree {
        roots.push((wt.join(".claude"), "project"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push((home.join(".claude"), "user"));
    }
    for (root, source) in &roots {
        for entry in scan_skills(&root.join("skills")) {
            let name = entry.declared.unwrap_or_else(|| entry.dir.clone());
            if out.iter().any(|s| s.name == name) {
                continue; // earlier source already claimed this name
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

    // Only enabled plugins: claude never registers a disabled plugin's skills,
    // so offering them would write frontmatter that warns "not found" on every
    // spawn.
    let enabled = enabled_plugins(worktree);
    for install in plugin_installs() {
        if !enabled.contains(&install.key) {
            continue;
        }
        for entry in scan_skills(&install.path.join("skills")) {
            // Plugin skills are qualified `<plugin>:<dir>`, the form claude's
            // own resolver matches, and the directory (not the frontmatter
            // name) is the canonical id.
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

/// One installed plugin: its `<plugin>@<marketplace>` manifest key, the label
/// used to qualify its skills, and where its files live.
struct PluginInstall {
    /// The key `enabledPlugins` uses.
    key: String,
    /// The key's half before `@`, how claude qualifies this plugin's skills.
    label: String,
    path: PathBuf,
}

/// Every installed plugin, read from claude's
/// `~/.claude/plugins/installed_plugins.json`. We scan exactly the versions
/// claude has loaded, not stale copies left in the cache. Installed is not the
/// same as enabled, see [`enabled_plugins`].
fn plugin_installs() -> Vec<PluginInstall> {
    let mut out = Vec::new();
    let Some(home) = dirs::home_dir() else {
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

/// The settings files claude merges `enabledPlugins` from, lowest precedence
/// first. Managed (MDM) policy is applied last so it can't be overridden.
fn settings_chain(worktree: Option<&Path>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(home) = dirs::home_dir() {
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

/// The `<plugin>@<marketplace>` keys enabled for `worktree`. An installed
/// plugin claude has not been told to enable contributes nothing at runtime,
/// its skills fail to resolve and warn at every spawn, so absence means off,
/// matching what `claude plugin list` reports.
fn enabled_plugins(worktree: Option<&Path>) -> std::collections::HashSet<String> {
    let mut on = std::collections::HashSet::new();
    for file in settings_chain(worktree) {
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
            // A later file may turn one back off, so take both branches.
            if value.as_bool().unwrap_or(false) {
                on.insert(key.clone());
            } else {
                on.remove(key);
            }
        }
    }
    on
}

/// Commands and skills contributed by installed plugins. Unlike [`list_skills`]
/// this does not filter on enablement, the composer's list is out of scope for
/// that change and has always offered every install.
fn collect_plugins(out: &mut Vec<SlashCommand>) {
    for install in plugin_installs() {
        collect_skills(&install.path.join("skills"), "plugin", out);
        collect_commands(&install.path.join("commands"), "", "plugin", out);
    }
}

/// Curated built-in slash commands that ship inside the claude binary (so they
/// can't be discovered on disk) and translate to a one-shot worktree task.
/// Session/control commands (`/clear`, `/config`, `/model`, `/resume`, …) are
/// intentionally omitted, they don't make sense sent as a programmatic turn.
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
        })
        .collect()
}

/// One `<dir>/skills/<name>/SKILL.md` as found on disk, before either consumer
/// decides what to call it.
struct SkillEntry {
    /// The directory name, claude's canonical name for a non-plugin skill.
    dir: String,
    /// Frontmatter `name`, when the file sets one.
    declared: Option<String>,
    description: Option<String>,
    /// Body length in characters (frontmatter excluded).
    chars: u32,
    /// Frontmatter sets `disable-model-invocation: true`.
    user_only: bool,
}

/// Walk `<dir>/<name>/SKILL.md` once, so both `collect_skills` (composer
/// autocomplete) and `list_skills` (Agents view) read the same files.
fn scan_skills(dir: &Path) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
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

/// `<root>/skills/<name>/SKILL.md`, the command name is the frontmatter `name`
/// when present, else the directory name.
fn collect_skills(dir: &Path, source: &str, out: &mut Vec<SlashCommand>) {
    for entry in scan_skills(dir) {
        let name = entry.declared.unwrap_or(entry.dir);
        out.push(SlashCommand {
            name,
            description: entry.description.unwrap_or_default(),
            source: source.to_string(),
        });
    }
}

/// `<root>/commands/**/*.md`, the name is the path below `commands/` with
/// separators turned into `:` and the `.md` dropped, matching claude's
/// namespacing (e.g. `commands/git/sync.md` → `git:sync`).
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
            });
        }
    }
}

/// Pull `name` and `description` out of a markdown file's `--- … ---` YAML
/// frontmatter, reusing the crate's tolerant frontmatter parser. `name`/
/// `description` aren't part of the thought schema, so they arrive as unknown
/// keys in `extra`. Falls back to the first non-empty, non-heading prose line
/// for the description when the frontmatter omits it, so commands without
/// metadata still read sensibly. Tolerant: anything unparseable just yields
/// `None`.
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

/// First non-empty body line that isn't a markdown heading or list item, used
/// as a description fallback.
fn first_prose_line(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('-') && !l.starts_with('#'))
        .map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp HOME so `~/.claude` resolves into an isolated dir instead of the
    /// real machine's. Mutates process-wide env, so keep this to the one test
    /// that needs it (see the equivalent guard in `opencode/config.rs`).
    /// `HOME` is process-wide and cargo runs tests on parallel threads, so
    /// every `TmpHome` holds this for its lifetime rather than racing.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TmpHome {
        dir: tempfile::TempDir,
        prev: Option<std::ffi::OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }
    impl TmpHome {
        fn new() -> Self {
            // A test that panicked while holding it poisoned nothing we care
            // about, the lock guards an env var, not shared data.
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
        let cmds = list_slash_commands(wt);
        assert!(cmds
            .iter()
            .any(|c| c.name == "demo" && c.source == "project"));
    }

    #[test]
    fn includes_builtin_commands() {
        let dir = tempfile::tempdir().unwrap();
        let cmds = list_slash_commands(dir.path());
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

        let skills = list_skills(Some(wt));
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

        let skills = list_skills(Some(wt));

        let counted = skills.iter().find(|s| s.name == "counted").unwrap();
        let body_offset = crate::docs::frontmatter::parse(text).body_offset;
        assert_eq!(counted.chars, text[body_offset..].chars().count() as u32);
        assert!(!counted.user_only);

        let reserved = skills.iter().find(|s| s.name == "reserved").unwrap();
        assert!(reserved.user_only);
    }

    /// Install one plugin shipping one skill into `home`, returning the
    /// worktree that sees it. The plugin is installed but not yet enabled.
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

    /// An installed-but-disabled plugin contributes nothing: claude never
    /// registers its skills, so offering them would write frontmatter that
    /// warns "not found" at every spawn.
    #[test]
    fn plugin_skills_appear_only_once_the_plugin_is_enabled() {
        let home = TmpHome::new();
        let project = install_plugin(home.path());
        let wt = project.path();

        let disabled = list_skills(Some(wt));
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

        let enabled = list_skills(Some(wt));
        let hit = enabled
            .iter()
            .find(|s| s.name == "demoplug:greet")
            .expect("enabled plugin's skill should be offered, qualified `plugin:skill`");
        assert_eq!(hit.source, "plugin");
        assert_eq!(hit.plugin.as_deref(), Some("demoplug"));
    }

    /// A more specific settings file may turn an enabled plugin back off.
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
        assert!(list_skills(Some(wt))
            .iter()
            .any(|s| s.name == "demoplug:greet"));

        std::fs::create_dir_all(wt.join(".claude")).unwrap();
        std::fs::write(
            wt.join(".claude/settings.local.json"),
            r#"{"enabledPlugins": {"demoplug@mkt": false}}"#,
        )
        .unwrap();
        assert!(
            !list_skills(Some(wt))
                .iter()
                .any(|s| s.name == "demoplug:greet"),
            "local settings should win over the user-level enable",
        );
    }

    #[test]
    fn list_skills_without_worktree_never_panics() {
        let skills = list_skills(None);
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

        let skills = list_skills(Some(wt));
        assert!(!skills.iter().any(|s| s.name == "sync"));

        let cmds = list_slash_commands(wt);
        assert!(cmds.iter().any(|c| c.name == "sync"));
    }
}
