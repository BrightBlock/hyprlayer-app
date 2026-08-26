use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::docs::frontmatter::{extract_block, patch_frontmatter, FmPatch, FmValue};
use crate::error::Result;

const MAX_AGENT_BYTES: u64 = 1024 * 1024;

const KEY_NAME: &str = "name";
const KEY_DESCRIPTION: &str = "description";
const KEY_TOOLS: &str = "tools";
const KEY_MODEL: &str = "model";
const KEY_COLOR: &str = "color";
const KEY_MEMORY: &str = "memory";
const KEY_SKILLS: &str = "skills";
const KEY_CATEGORY: &str = "category";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentFile {
    pub path: String,
    pub source: String,
    pub plugin: Option<String>,
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub model: Option<String>,
    pub color: Option<String>,
    pub memory: Option<String>,
    pub category: Option<String>,
    pub prompt: String,
    pub updated_at: Option<f64>,
    pub shipped_path: Option<String>,
    pub modified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentRule {
    pub id: String,
    pub action: String,
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentCollaborator {
    pub name: String,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentOverlay {
    pub effort: String,
    pub harness: String,
    pub folder: String,
    pub status: String,
    pub permission_mode: String,
    pub rules: Vec<AgentRule>,
    pub network: bool,
    pub outside_worktree: bool,
    pub context: Vec<String>,
    pub delegates: Vec<String>,
    pub visibility: String,
    pub collaborators: Vec<AgentCollaborator>,
}

impl Default for AgentOverlay {
    fn default() -> Self {
        Self {
            effort: "high".into(),
            harness: "claude".into(),
            folder: String::new(),
            status: "published".into(),
            permission_mode: "default".into(),
            rules: Vec::new(),
            network: true,
            outside_worktree: false,
            context: Vec::new(),
            delegates: Vec::new(),
            visibility: "private".into(),
            collaborators: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentContextFile {
    pub path: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentMcpServer {
    pub name: String,
    pub scope: String,
    pub transport: String,
}

pub(crate) fn agents_dir(claude_root: &Path) -> PathBuf {
    claude_root.join("agents")
}

fn roots(worktree: Option<&Path>) -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    if let Some(wt) = worktree {
        out.push((agents_dir(&wt.join(".claude")), "project"));
    }
    if let Some(home) = dirs::home_dir() {
        out.push((agents_dir(&home.join(".claude")), "user"));
    }
    out
}

pub fn agents_dir_for(worktree: Option<&Path>, source: &str) -> Option<PathBuf> {
    roots(worktree)
        .into_iter()
        .find(|(_, s)| *s == source)
        .map(|(dir, _)| dir)
}

pub fn parse_agent(path: &Path, source: &str, text: &str) -> AgentFile {
    let parsed = crate::docs::frontmatter::parse(text);
    let pick = |key: &str| {
        parsed
            .fm
            .extra
            .get(key)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    AgentFile {
        path: path.to_string_lossy().into_owned(),
        source: source.to_string(),
        plugin: None,
        name: pick(KEY_NAME).unwrap_or(stem),
        description: pick(KEY_DESCRIPTION).unwrap_or_default(),
        tools: pick(KEY_TOOLS)
            .as_deref()
            .map(split_list)
            .unwrap_or_default(),
        skills: pick(KEY_SKILLS)
            .as_deref()
            .map(split_list)
            .unwrap_or_default(),
        model: pick(KEY_MODEL),
        color: pick(KEY_COLOR),
        memory: pick(KEY_MEMORY),
        category: pick(KEY_CATEGORY),
        prompt: text[parsed.body_offset..]
            .trim_start_matches('\n')
            .to_string(),
        updated_at: mtime_ms(path),
        shipped_path: None,
        modified: false,
    }
}

fn split_list(raw: &str) -> Vec<String> {
    if let Ok(items) = serde_json::from_str::<Vec<String>>(raw) {
        return items.into_iter().flat_map(|s| split_scalar(&s)).collect();
    }
    split_scalar(raw)
}

fn split_scalar(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut push = |current: &mut String| {
        let entry = current.trim();
        if !entry.is_empty() {
            out.push(entry.to_string());
        }
        current.clear();
    };
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if depth == 0 && (c == ',' || c.is_whitespace()) => push(&mut current),
            _ => current.push(ch),
        }
    }
    push(&mut current);
    out
}

fn mtime_ms(path: &Path) -> Option<f64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
}

fn collect(dir: &Path, source: &str, plugin: Option<&str>, out: &mut Vec<AgentFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if entry
            .metadata()
            .map(|m| m.len() > MAX_AGENT_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut agent = parse_agent(&path, source, &text);
            agent.plugin = plugin.map(|p| p.to_string());
            out.push(agent);
        }
    }
}

fn collect_plugin_agents(out: &mut Vec<AgentFile>) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(home.join(".claude/plugins/installed_plugins.json"))
    else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(plugins) = json.get("plugins").and_then(|p| p.as_object()) else {
        return;
    };
    for (key, entries) in plugins {
        let label = key.split('@').next().unwrap_or(key);
        let Some(installs) = entries.as_array() else {
            continue;
        };
        for install in installs {
            let Some(path) = install.get("installPath").and_then(|p| p.as_str()) else {
                continue;
            };
            collect(&Path::new(path).join("agents"), "plugin", Some(label), out);
        }
    }
}

pub fn list_agents(worktree: Option<&Path>) -> Vec<AgentFile> {
    let mut owned: Vec<AgentFile> = Vec::new();
    for (dir, source) in roots(worktree) {
        let mut found = Vec::new();
        collect(&dir, source, None, &mut found);
        for agent in found {
            if !owned.iter().any(|a| a.name == agent.name) {
                owned.push(agent);
            }
        }
    }

    let mut plugins = Vec::new();
    collect_plugin_agents(&mut plugins);

    let mut out: Vec<AgentFile> = Vec::new();
    for mut shipped in plugins {
        if out.iter().any(|a| a.name == shipped.name) {
            continue;
        }
        match owned.iter().position(|a| a.name == shipped.name) {
            Some(i) => {
                let override_file = owned.remove(i);
                out.push(AgentFile {
                    shipped_path: Some(shipped.path),
                    modified: true,
                    source: "plugin".into(),
                    plugin: shipped.plugin,
                    ..override_file
                });
            }
            None => {
                shipped.shipped_path = Some(shipped.path.clone());
                out.push(shipped);
            }
        }
    }
    out.append(&mut owned);

    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

pub fn render_agent(original: &str, agent: &AgentFile) -> Result<String> {
    let current = parse_agent(Path::new(&agent.path), &agent.source, original);
    let mut patch = FmPatch::default();
    for (key, was, now) in [
        (KEY_NAME, &current.name, &agent.name),
        (KEY_DESCRIPTION, &current.description, &agent.description),
    ] {
        if was != now {
            patch.set.push((key.into(), FmValue::Str(now.clone())));
        }
    }
    for (key, was, now) in [
        (KEY_TOOLS, &current.tools, &agent.tools),
        (KEY_SKILLS, &current.skills, &agent.skills),
    ] {
        if was == now && !now.is_empty() {
            continue;
        }
        if now.is_empty() {
            patch.remove.push(key.into());
        } else {
            patch.set.push((key.into(), FmValue::Str(now.join(", "))));
        }
    }
    for (key, was, now) in [
        (KEY_MODEL, &current.model, &agent.model),
        (KEY_COLOR, &current.color, &agent.color),
        (KEY_MEMORY, &current.memory, &agent.memory),
        (KEY_CATEGORY, &current.category, &agent.category),
    ] {
        match now {
            Some(v) if !v.is_empty() => {
                if was.as_deref() != Some(v.as_str()) {
                    patch.set.push((key.into(), FmValue::Str(v.clone())));
                }
            }
            _ => patch.remove.push(key.into()),
        }
    }

    let patched = patch_frontmatter(original, &patch)?;
    let head_len = extract_block(&patched)
        .map(|(_, offset)| offset)
        .unwrap_or(0);
    let separator = separator_of(original);
    Ok(format!(
        "{}{}{}\n",
        patched[..head_len].trim_end(),
        separator,
        agent.prompt.trim()
    ))
}

fn separator_of(original: &str) -> String {
    let Some((_, body_offset)) = extract_block(original) else {
        return "\n".to_string();
    };
    let blanks = original[body_offset..]
        .chars()
        .take_while(|c| *c == '\n')
        .count();
    "\n".repeat(blanks + 1)
}

pub fn save_agent(agent: &AgentFile) -> Result<AgentFile> {
    let path = PathBuf::from(&agent.path);
    let original = std::fs::read_to_string(&path).unwrap_or_default();
    let contents = render_agent(&original, agent)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, contents.as_bytes())?;
    let mut saved = parse_agent(&path, &agent.source, &contents);
    saved.plugin = agent.plugin.clone();
    saved.shipped_path = agent.shipped_path.clone();
    saved.modified = agent.shipped_path.is_some();
    Ok(saved)
}

pub fn delete_agent(path: &Path) -> Result<()> {
    std::fs::remove_file(path)?;
    Ok(())
}

pub fn file_stem_for(name: &str) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let collapsed = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "agent".to_string()
    } else {
        collapsed
    }
}

pub fn context_candidates(worktree: Option<&Path>) -> Vec<AgentContextFile> {
    let Some(wt) = worktree else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if wt.join("CLAUDE.md").is_file() {
        out.push(AgentContextFile {
            path: "CLAUDE.md".into(),
            note: "project root".into(),
        });
    }
    if let Ok(entries) = std::fs::read_dir(wt.join(".claude/rules")) {
        let mut rules: Vec<String> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
            .collect();
        rules.sort();
        for file in rules {
            let stem = file.trim_end_matches(".md").replace('-', " ");
            out.push(AgentContextFile {
                path: format!(".claude/rules/{file}"),
                note: stem,
            });
        }
    }
    if let Ok(entries) = std::fs::read_dir(wt.join("docs/adr")) {
        let count = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .count();
        if count > 0 {
            out.push(AgentContextFile {
                path: "docs/adr/".into(),
                note: format!("{count} decisions"),
            });
        }
    }
    out
}

pub fn mcp_servers(worktree: Option<&Path>) -> Vec<AgentMcpServer> {
    let mut out: Vec<AgentMcpServer> = Vec::new();
    let mut sources: Vec<(PathBuf, &'static str)> = Vec::new();
    if let Some(wt) = worktree {
        sources.push((wt.join(".mcp.json"), "project"));
    }
    if let Some(home) = dirs::home_dir() {
        sources.push((home.join(".claude.json"), "user"));
    }
    for (file, scope) in sources {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(servers) = json.get("mcpServers").and_then(|m| m.as_object()) else {
            continue;
        };
        for (name, cfg) in servers {
            if out.iter().any(|s| s.name == *name) {
                continue;
            }
            let transport =
                cfg.get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or(if cfg.get("url").is_some() {
                        "http"
                    } else {
                        "stdio"
                    });
            out.push(AgentMcpServer {
                name: name.clone(),
                scope: scope.into(),
                transport: transport.to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn agent_deny_rules(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| !n.contains(['(', ')', ',']))
        .map(|n| format!("Agent({n})"))
        .collect()
}

pub fn export_agent(agent: &AgentFile, overlay: &AgentOverlay) -> Result<String> {
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{}.agent.json", file_stem_for(&agent.name)));
    let body = serde_json::json!({
        "name": agent.name,
        "description": agent.description,
        "tools": agent.tools,
        "skills": agent.skills,
        "model": agent.model,
        "color": agent.color,
        "category": agent.category,
        "prompt": agent.prompt,
        "hyprlayer": overlay,
    });
    std::fs::write(
        &target,
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )?;
    Ok(target.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: codebase-locator\ndescription: Finds files\ntools: Grep, Glob, Read\nmodel: sonnet\n---\n\nYou are a specialist.\n";

    #[test]
    fn parses_frontmatter_and_body() {
        let a = parse_agent(Path::new("/x/codebase-locator.md"), "user", SAMPLE);
        assert_eq!(a.name, "codebase-locator");
        assert_eq!(a.description, "Finds files");
        assert_eq!(a.tools, ["Grep", "Glob", "Read"]);
        assert_eq!(a.model.as_deref(), Some("sonnet"));
        assert_eq!(a.prompt, "You are a specialist.\n");
    }

    #[test]
    fn a_file_without_frontmatter_is_all_prompt() {
        let a = parse_agent(Path::new("/x/loose.md"), "user", "just a prompt\n");
        assert_eq!(a.name, "loose");
        assert_eq!(a.prompt, "just a prompt\n");
        assert!(a.tools.is_empty(), "no tools key means inherit everything");
        assert_eq!(a.model, None);
    }

    #[test]
    fn render_round_trips_and_keeps_unmodelled_keys() {
        let original =
            "---\nname: a\ndescription: d\ncolor: blue\nnotes: keep me\n---\n\nold body\n";
        let mut agent = parse_agent(Path::new("/x/a.md"), "user", original);
        agent.name = "renamed".into();
        agent.prompt = "new body".into();
        let out = render_agent(original, &agent).unwrap();

        let back = parse_agent(Path::new("/x/a.md"), "user", &out);
        assert_eq!(back.name, "renamed");
        assert_eq!(back.prompt, "new body\n");
        assert_eq!(back.color.as_deref(), Some("blue"));
        assert!(
            out.contains("notes: keep me"),
            "unmodelled key dropped:\n{out}"
        );
    }

    #[test]
    fn render_round_trips_and_keeps_specific_claude_keys_the_app_does_not_model() {
        let original = "---\nname: a\ndescription: d\ndisallowedTools: Bash(git push:*)\nmaxTurns: 5\nisolation: strict\nobserver: true\n---\n\nBody.\n";
        let mut agent = parse_agent(Path::new("/x/a.md"), "user", original);
        agent.description = "updated".into();
        let out = render_agent(original, &agent).unwrap();
        for line in [
            "disallowedTools: Bash(git push:*)",
            "maxTurns: 5",
            "isolation: strict",
            "observer: true",
        ] {
            assert!(out.contains(line), "key dropped: {line}\n{out}");
        }
    }

    #[test]
    fn clearing_tools_removes_the_key_rather_than_writing_an_empty_one() {
        let mut agent = parse_agent(Path::new("/x/a.md"), "user", SAMPLE);
        agent.tools.clear();
        let out = render_agent(SAMPLE, &agent).unwrap();
        assert!(
            !out.contains("tools:"),
            "empty tools should be absent:\n{out}"
        );
        assert!(parse_agent(Path::new("/x/a.md"), "user", &out)
            .tools
            .is_empty());
    }

    #[test]
    fn clearing_skills_removes_the_key_rather_than_writing_an_empty_one() {
        let with_skills =
            "---\nname: a\ndescription: d\nskills: research_codebase, create_plan\n---\n\nBody.\n";
        let mut agent = parse_agent(Path::new("/x/a.md"), "user", with_skills);
        assert_eq!(agent.skills, ["research_codebase", "create_plan"]);
        agent.skills.clear();
        let out = render_agent(with_skills, &agent).unwrap();
        assert!(
            !out.contains("skills:"),
            "empty skills should be absent:\n{out}"
        );
        assert!(parse_agent(Path::new("/x/a.md"), "user", &out)
            .skills
            .is_empty());
    }

    #[test]
    fn skills_round_trip_from_a_comma_separated_value_and_survive_a_rename() {
        let original =
            "---\nname: locator\ndescription: d\nskills: research_codebase, create_plan\n---\n\nBody.\n";
        let mut agent = parse_agent(Path::new("/x/locator.md"), "user", original);
        assert_eq!(agent.skills, ["research_codebase", "create_plan"]);
        agent.name = "renamed".into();
        let out = render_agent(original, &agent).unwrap();
        let back = parse_agent(Path::new("/x/locator.md"), "user", &out);
        assert_eq!(back.name, "renamed");
        assert_eq!(back.skills, ["research_codebase", "create_plan"]);
    }

    #[test]
    fn flow_sequence_values_parse_as_lists() {
        let with_tools_seq = "---\nname: a\ndescription: d\ntools: [Read, Grep]\n---\n\nBody.\n";
        let a = parse_agent(Path::new("/x/a.md"), "user", with_tools_seq);
        assert_eq!(a.tools, ["Read", "Grep"]);

        let with_skills_seq =
            "---\nname: a\ndescription: d\nskills: [research_codebase, create_plan]\n---\n\nBody.\n";
        let b = parse_agent(Path::new("/x/a.md"), "user", with_skills_seq);
        assert_eq!(b.skills, ["research_codebase", "create_plan"]);
    }

    #[test]
    fn tools_split_is_paren_aware_and_re_renders_unchanged() {
        let original = "---\nname: a\ndescription: d\ntools: Read, Bash(git commit:*), Task(researcher, planner)\n---\n\nBody.\n";
        let agent = parse_agent(Path::new("/x/a.md"), "user", original);
        assert_eq!(
            agent.tools,
            ["Read", "Bash(git commit:*)", "Task(researcher, planner)"]
        );
        let out = render_agent(original, &agent).unwrap();
        assert!(
            out.contains("tools: Read, Bash(git commit:*), Task(researcher, planner)"),
            "paren-bearing entries were mangled on re-render:\n{out}"
        );
    }

    #[test]
    fn whitespace_separated_tools_parse_to_two_entries() {
        let original = "---\nname: a\ndescription: d\ntools: Read Grep\n---\n\nBody.\n";
        let agent = parse_agent(Path::new("/x/a.md"), "user", original);
        assert_eq!(agent.tools, ["Read", "Grep"]);
    }

    #[test]
    fn a_new_file_rendered_from_nothing_with_skills_set_parses_back_with_them() {
        let agent = AgentFile {
            path: "/x/new.md".into(),
            source: "user".into(),
            plugin: None,
            name: "New Agent".into(),
            description: "does things".into(),
            tools: vec![],
            skills: vec!["research_codebase".into(), "create_plan".into()],
            model: None,
            color: None,
            memory: None,
            category: None,
            prompt: "Be helpful.".into(),
            updated_at: None,
            shipped_path: None,
            modified: false,
        };
        let out = render_agent("", &agent).unwrap();
        let back = parse_agent(Path::new("/x/new.md"), "user", &out);
        assert_eq!(back.skills, ["research_codebase", "create_plan"]);
    }

    #[test]
    fn renders_a_brand_new_file_from_nothing() {
        let agent = AgentFile {
            path: "/x/new.md".into(),
            source: "user".into(),
            plugin: None,
            name: "New Agent".into(),
            description: "does things".into(),
            tools: vec!["Read".into()],
            skills: vec![],
            model: Some("sonnet".into()),
            color: None,
            memory: Some("project".into()),
            category: None,
            prompt: "Be helpful.".into(),
            updated_at: None,
            shipped_path: None,
            modified: false,
        };
        let out = render_agent("", &agent).unwrap();
        let back = parse_agent(Path::new("/x/new.md"), "user", &out);
        assert_eq!(back.name, "New Agent");
        assert_eq!(back.tools, ["Read"]);
        assert_eq!(back.memory.as_deref(), Some("project"));
        assert_eq!(back.prompt, "Be helpful.\n");
    }

    #[test]
    fn memory_scope_round_trips_and_clears() {
        let with_memory = "---\nname: researcher\ndescription: d\nmemory: project\n---\n\nBody.\n";
        let parsed = parse_agent(Path::new("/x/researcher.md"), "user", with_memory);
        assert_eq!(parsed.memory.as_deref(), Some("project"));

        let kept = render_agent(with_memory, &parsed).unwrap();
        assert!(
            kept.contains("memory: project"),
            "scope should survive: {kept}"
        );

        let cleared = AgentFile {
            memory: None,
            ..parsed
        };
        let out = render_agent(with_memory, &cleared).unwrap();
        assert!(
            !out.contains("memory:"),
            "cleared scope should drop the key: {out}"
        );
        assert!(out.contains("name: researcher"), "other keys stay: {out}");
    }

    #[test]
    fn unknown_memory_scope_is_preserved() {
        let text = "---\nname: a\ndescription: d\nmemory: some-future-scope\n---\n\nBody.\n";
        let parsed = parse_agent(Path::new("/x/a.md"), "user", text);
        assert_eq!(parsed.memory.as_deref(), Some("some-future-scope"));
        let out = render_agent(text, &parsed).unwrap();
        assert!(out.contains("memory: some-future-scope"));
    }

    #[test]
    fn category_round_trips_and_clears() {
        let with_category = "---\nname: a\ndescription: d\ncategory: Research\n---\n\nBody.\n";
        let parsed = parse_agent(Path::new("/x/a.md"), "user", with_category);
        assert_eq!(parsed.category.as_deref(), Some("Research"));

        let kept = render_agent(with_category, &parsed).unwrap();
        assert!(
            kept.contains("category: Research"),
            "category should survive: {kept}"
        );
        let back = parse_agent(Path::new("/x/a.md"), "user", &kept);
        assert_eq!(back.category.as_deref(), Some("Research"));

        let cleared = AgentFile {
            category: None,
            ..parsed
        };
        let out = render_agent(with_category, &cleared).unwrap();
        assert!(
            !out.contains("category:"),
            "cleared category should drop the key rather than write an empty value: {out}"
        );
        assert!(out.contains("name: a"), "other keys stay: {out}");
    }

    #[test]
    fn adding_a_category_leaves_every_other_line_byte_identical() {
        let original = concat!(
            "---\n",
            "name: inspector\n",
            "description: Audits a plan's claims. Spawn during the Validate stage.\n",
            "tools: Read, Grep\n",
            "model: sonnet\n",
            "---\n",
            "\n",
            "You are the inspector.\n",
        );
        let parsed = parse_agent(Path::new("/x/inspector.md"), "user", original);
        let filed = AgentFile {
            category: Some("Spike".into()),
            ..parsed
        };
        let out = render_agent(original, &filed).unwrap();

        assert!(
            out.contains("description: Audits a plan's claims. Spawn during the Validate stage.\n"),
            "an untouched description must keep its exact bytes: {out}"
        );
        assert!(out.contains("category: Spike"), "category is added: {out}");
        assert!(
            out.contains("---\n\nYou are the inspector."),
            "the blank line after the frontmatter survives: {out:?}"
        );

        let cleared = AgentFile {
            category: None,
            ..parse_agent(Path::new("/x/inspector.md"), "user", &out)
        };
        assert_eq!(
            render_agent(&out, &cleared).unwrap(),
            original,
            "set then clear must round-trip to the original bytes"
        );
    }

    #[test]
    fn file_stems_are_slugged() {
        assert_eq!(file_stem_for("Plan Reviewer"), "plan-reviewer");
        assert_eq!(file_stem_for("  A//B  "), "a-b");
        assert_eq!(file_stem_for("***"), "agent");
    }

    #[test]
    fn overlay_defaults_match_a_fresh_agent() {
        let o = AgentOverlay::default();
        assert_eq!(o.effort, "high");
        assert_eq!(o.harness, "claude");
        assert_eq!(o.permission_mode, "default");
        assert!(
            o.network,
            "agents may reach the network unless told otherwise"
        );
        assert!(!o.outside_worktree);
    }

    #[test]
    fn context_candidates_are_empty_without_a_worktree() {
        assert!(context_candidates(None).is_empty());
    }

    #[test]
    fn agent_deny_rules_render_one_per_name_in_order() {
        let names = vec!["cartographer".to_string(), "foreman".to_string()];
        assert_eq!(
            agent_deny_rules(&names),
            vec!["Agent(cartographer)", "Agent(foreman)"]
        );
    }

    #[test]
    fn agent_deny_rules_drop_names_that_carry_rule_syntax() {
        let names = vec![
            "cartographer".to_string(),
            "weird(name)".to_string(),
            "a,b".to_string(),
            "foreman".to_string(),
        ];
        assert_eq!(
            agent_deny_rules(&names),
            vec!["Agent(cartographer)", "Agent(foreman)"]
        );
    }
}
