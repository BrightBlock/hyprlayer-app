//! Claude Code agent definitions, the `.claude/agents/*.md` files whose YAML
//! frontmatter carries the launch config and whose body is the system prompt.
//!
//! Three sources, in claude's own precedence order: project
//! (`<worktree>/.claude/agents`), user (`~/.claude/agents`), and the agents
//! contributed by installed plugins. Claude reads these files straight off
//! disk, so the Agents view edits them in place, a save patches the
//! frontmatter surgically ([`patch_frontmatter`]) and swaps only the body,
//! leaving keys this app doesn't model, and the file's own formatting, intact.
//!
//! A plugin agent's file belongs to the plugin and is overwritten on its next
//! update, so editing one writes a copy into `~/.claude/agents` (an override)
//! and "restore shipped version" deletes that copy.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::docs::frontmatter::{extract_block, patch_frontmatter, FmPatch, FmValue};
use crate::error::Result;

/// Largest agent file we'll read. These are prompts, a few KB each; past this
/// it is a wrong or hostile file, not an agent.
const MAX_AGENT_BYTES: u64 = 1024 * 1024;

/// The frontmatter keys an agent definition uses. Everything else in the block
/// is preserved verbatim on save.
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
    /// Absolute path to the `.md` file, the identity a save or delete targets.
    pub path: String,
    /// `project` | `user` | `plugin`, which root supplied it.
    pub source: String,
    /// The plugin that ships this agent, when `source` is `plugin`.
    pub plugin: Option<String>,
    /// Frontmatter `name`, falling back to the filename stem.
    pub name: String,
    pub description: String,
    /// Frontmatter `tools`, comma-separated in the file. EMPTY MEANS INHERIT
    /// every tool. It is not the same as an agent with no tools.
    pub tools: Vec<String>,
    /// Frontmatter `skills`, skill names claude preloads into this agent when
    /// it runs. Canonical names: the skill's directory, or `plugin:skill`.
    /// Empty means the key is absent; unlike `tools`, that is simply "none".
    pub skills: Vec<String>,
    /// Frontmatter `model` (`sonnet`, `opus`, …); absent inherits the parent's.
    pub model: Option<String>,
    /// Frontmatter `color`, used by claude's own picker.
    pub color: Option<String>,
    /// Frontmatter `memory`, the persistent-memory scope claude keeps for this
    /// agent between invocations: `user` (every project), `project` (this repo's
    /// `.claude/agent-memory/<name>/`), or `local` (session only). Absent means
    /// the agent remembers nothing across runs. Stored as claude writes it, not
    /// as an enum: an unrecognized scope is preserved rather than dropped.
    pub memory: Option<String>,
    /// Frontmatter `category`, a display-only grouping label the list folders
    /// agents by. Free text; absent means the folder is derived from the
    /// description's stage phrase (see `phaseFolder` on the frontend).
    pub category: Option<String>,
    /// Everything after the frontmatter block, the agent's system prompt.
    pub prompt: String,
    /// File mtime in epoch milliseconds, for the list's Updated column. `None`
    /// when the file has never been written (a new agent) or the OS withholds it.
    pub updated_at: Option<f64>,
    /// For a plugin agent, the file the plugin ships. `path` points at the
    /// user's override instead once one exists.
    pub shipped_path: Option<String>,
    /// A plugin agent the user has overridden, `path` is the override.
    pub modified: bool,
}

/// One permission rule. `action` is `allow` | `ask` | `deny`; `pattern` uses the
/// harness's own `Tool(arg:*)` syntax.
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

/// Launch settings the app keeps for an agent that claude's own frontmatter has
/// no key for, effort, harness, permission posture, folder. Stored app-side
/// keyed by the agent's file path so the `.md` stays a plain claude agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct AgentOverlay {
    pub effort: String,
    pub harness: String,
    /// List-only grouping; `""` is no folder.
    pub folder: String,
    /// `published` | `draft`, a draft is hidden from the composer.
    pub status: String,
    pub permission_mode: String,
    pub rules: Vec<AgentRule>,
    pub network: bool,
    pub outside_worktree: bool,
    /// Paths from [`context_candidates`] stitched onto the prompt at launch.
    pub context: Vec<String>,
    /// Agent file paths this one may spawn with the `Task` tool.
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

/// A doc the editor offers to append to the prompt. Discovered in the worktree,
/// never invented, an absent file is simply not offered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentContextFile {
    /// Worktree-relative path, which is also the key the overlay enables.
    pub path: String,
    pub note: String,
}

/// An MCP server claude is configured with, for the Capabilities tab. Whether a
/// given agent may use it is expressed in its own `tools` list as
/// `mcp__<name>__*`, so there is nothing extra to persist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AgentMcpServer {
    pub name: String,
    /// `project` | `user`, which config declares it.
    pub scope: String,
    /// Transport as declared (`stdio`, `http`, `sse`).
    pub transport: String,
}

/// `<claude_root>/agents`. `pub(crate)` so `agent_bundle` reads the layout
/// from here rather than keeping a second copy of the same join.
pub(crate) fn agents_dir(claude_root: &Path) -> PathBuf {
    claude_root.join("agents")
}

/// The two writable roots an agent can live under, in precedence order.
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

/// The directory a new agent of `source` is written into, if that root exists
/// as a concept (a project source needs an open worktree).
pub fn agents_dir_for(worktree: Option<&Path>, source: &str) -> Option<PathBuf> {
    roots(worktree)
        .into_iter()
        .find(|(_, s)| *s == source)
        .map(|(dir, _)| dir)
}

/// Parse one agent file. Tolerant like the docs parser: a file with no or
/// malformed frontmatter still yields an agent, named after the file, whose
/// whole text is the prompt.
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

/// Split a frontmatter list value the way claude's own agent loader does:
/// entries separated by commas or whitespace, except inside parentheses, so
/// `Bash(git commit:*)` stays one entry. A YAML sequence arrives pre-rendered
/// as JSON (`["a","b"]`) because unknown keys are stored that way.
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

/// The file's mtime in epoch ms. Absent for a path that isn't on disk yet.
fn mtime_ms(path: &Path) -> Option<f64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
}

fn collect(dir: &Path, source: &str, plugin: Option<&str>, out: &mut Vec<AgentFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // an absent agents dir is the normal case, not an error
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

/// Agents contributed by installed plugins. The active install of each plugin
/// is read from claude's `installed_plugins.json`, so we scan exactly the
/// versions claude has loaded rather than stale copies left in the cache.
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
        // Keys are `<plugin>@<marketplace>`; the plugin half is the label.
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

/// Every agent visible from `worktree`: the project's own, the user's, and the
/// installed plugins', with an earlier source shadowing a later one of the same
/// name, claude's own precedence. Sorted by name.
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
            continue; // two plugins shipping the same name: first install wins
        }
        // A file of the same name in your own library IS this agent's override,
        // so it edits here rather than showing up twice.
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

/// The file contents for `agent`, patched onto `original` (empty for a new
/// file): frontmatter keys updated surgically, body replaced wholesale.
pub fn render_agent(original: &str, agent: &AgentFile) -> Result<String> {
    // What the file already says. `patch_frontmatter` keeps the exact bytes of
    // any span it is not asked to touch, so a key whose value is unchanged is
    // left out of the patch entirely rather than re-emitted. Without this, a
    // save that adds `category:` also re-quotes an untouched `description`
    // through the YAML emitter, a diff on a line the user never edited, in a
    // file that is usually under version control.
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
    // An empty list means the key is absent: for `tools` that reads as "inherit
    // every tool", for `skills` as "preload none". Neither is an empty string.
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
    // The patcher never touches the body, so splice the new prompt in after the
    // closing `---`. `parse_agent` strips the newlines that separated the two,
    // so carry the original's separator across rather than normalizing every
    // file to exactly one.
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

/// The run of newlines between the closing `---` and the body in `original`,
/// so a save preserves the blank line an author left there. One newline ends
/// the `---` line itself (the caller has trimmed it off the head), and each
/// further one is a blank line the author wrote. Defaults to a single newline
/// for a new file.
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

/// Write `agent` to its own path, creating the agents directory if needed.
/// Returns the agent as re-read from what was written.
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

/// Delete an agent definition. The file IS the agent.
pub fn delete_agent(path: &Path) -> Result<()> {
    std::fs::remove_file(path)?;
    Ok(())
}

/// A filesystem-safe stem for a new agent named `name`.
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

/// Docs in `worktree` worth stitching onto a prompt: the project's CLAUDE.md,
/// each `.claude/rules/*.md`, and the ADR directory as one entry.
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

/// MCP servers claude is configured with: the project's `.mcp.json` plus the
/// user's `~/.claude.json`.
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
                continue; // project declaration wins
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

/// `Agent(<name>)` deny rules for agents that must not be spawnable. Claude's
/// rule parser splits on the first `(` and last `)`, so a name carrying either,
/// or a comma, can't be expressed as a rule and is skipped rather than
/// silently denying something else.
pub fn agent_deny_rules(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| !n.contains(['(', ')', ',']))
        .map(|n| format!("Agent({n})"))
        .collect()
}

/// Write a portable `<slug>.agent.json` (definition + launch settings, no
/// credentials) to the user's Downloads folder. Returns where it landed.
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
        // The filename stands in for a missing `name` so the row is still usable.
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
        // A key the app doesn't model survives the edit.
        assert!(
            out.contains("notes: keep me"),
            "unmodelled key dropped:\n{out}"
        );
    }

    /// Real claude keys the app doesn't model, the ones an agent author is
    /// likely to hand-write next, must survive a save byte-identical.
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

    /// A YAML flow sequence lands in `extra` pre-rendered as JSON, since the
    /// frontmatter parser doesn't know `tools`/`skills` are lists. Both keys
    /// must read that form correctly, the pre-existing `tools` bug this fixes.
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

    /// Claude's own splitter is paren-aware: `Bash(git commit:*)` and
    /// `Task(researcher, planner)` are single entries, and re-rendering must
    /// not mangle them back into fragments.
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

    /// `memory` is claude's own key (scopes `user` / `project` / `local`), so a
    /// file that sets it must round-trip, and clearing it must remove the key
    /// rather than write an empty one, an empty scope is not a legal value.
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

    /// An unrecognized scope is claude's business, not ours, preserve it
    /// verbatim instead of dropping a key we failed to understand.
    #[test]
    fn unknown_memory_scope_is_preserved() {
        let text = "---\nname: a\ndescription: d\nmemory: some-future-scope\n---\n\nBody.\n";
        let parsed = parse_agent(Path::new("/x/a.md"), "user", text);
        assert_eq!(parsed.memory.as_deref(), Some("some-future-scope"));
        let out = render_agent(text, &parsed).unwrap();
        assert!(out.contains("memory: some-future-scope"));
    }

    /// `category` is the folder-grouping key: it must round-trip like `memory`,
    /// and clearing it must remove the key rather than write an empty value.
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

    /// A save that changes one key must not rewrite the others. The
    /// description here contains an apostrophe, which the YAML emitter would
    /// re-quote (`plan's` → `'plan''s'`) if it were re-emitted; the blank line
    /// after the frontmatter is the other thing a naive render loses. Both are
    /// spans the user never edited, in a file that is usually in git.
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

        // And clearing it again returns the file to exactly what it was.
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
