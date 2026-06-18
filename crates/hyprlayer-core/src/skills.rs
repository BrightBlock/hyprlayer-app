//! Discovery of the slash commands claude exposes in a worktree, so the
//! composer can autocomplete `/skill` the way the CLI does.
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
    /// `project` | `user` — which `.claude` root supplied it (for the label).
    pub source: String,
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

    // 1. Project root first, then user — project entries take precedence.
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

/// Commands and skills contributed by installed plugins. The active install of
/// each plugin (and its on-disk path) is read from claude's
/// `~/.claude/plugins/installed_plugins.json`, so we scan exactly the versions
/// claude has loaded — not stale copies left in the cache.
fn collect_plugins(out: &mut Vec<SlashCommand>) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let manifest = home.join(".claude/plugins/installed_plugins.json");
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(plugins) = json.get("plugins").and_then(|p| p.as_object()) else {
        return;
    };
    for entries in plugins.values() {
        let Some(installs) = entries.as_array() else {
            continue;
        };
        for install in installs {
            let Some(path) = install.get("installPath").and_then(|p| p.as_str()) else {
                continue;
            };
            let base = Path::new(path);
            collect_skills(&base.join("skills"), "plugin", out);
            collect_commands(&base.join("commands"), "", "plugin", out);
        }
    }
}

/// Curated built-in slash commands that ship inside the claude binary (so they
/// can't be discovered on disk) and translate to a one-shot worktree task.
/// Session/control commands (`/clear`, `/config`, `/model`, `/resume`, …) are
/// intentionally omitted — they don't make sense sent as a programmatic turn.
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

/// `<root>/skills/<name>/SKILL.md` — the command name is the frontmatter `name`
/// when present, else the directory name.
fn collect_skills(dir: &Path, source: &str, out: &mut Vec<SlashCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let skill_file = entry.path().join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&skill_file) else {
            continue;
        };
        let (name, description) = parse_meta(&text);
        let name = name.unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
        out.push(SlashCommand {
            name,
            description: description.unwrap_or_default(),
            source: source.to_string(),
        });
    }
}

/// `<root>/commands/**/*.md` — the name is the path below `commands/` with
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

/// First non-empty body line that isn't a markdown heading or list item — used
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
}
