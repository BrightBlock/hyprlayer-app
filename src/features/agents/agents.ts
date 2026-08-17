/**
 * Agent-library pure core.
 *
 * An agent IS a `.claude/agents/<name>.md` file — frontmatter for the launch
 * config, body for the system prompt. The launch settings claude's frontmatter
 * has no key for (effort, harness, permissions, rules, folder) ride alongside
 * in `AppSettings.agents`, keyed by the file's path.
 *
 * Shared vocabulary (models, efforts, harnesses, permission modes) comes from
 * `lib/domain`, never re-declared here.
 */

import type { AgentFile, AgentLibrarySettings, AgentOverlay } from "@/lib/bindings";
import { MODELS } from "@/lib/domain/models";

/** The frontmatter `model` value for "whatever spawned me" — claude's own
 *  spelling, and what an absent key means. */
export const INHERIT_MODEL = "inherit";

/** Display name for a frontmatter `model`. An unknown value is shown as-is —
 *  claude accepts full model ids there too. */
export function modelLabel(model: string | null): string {
  if (!model || model === INHERIT_MODEL) return "Inherit";
  const option = MODELS.find((m) => m.key === model);
  return option ? `${option.label}${option.hint ? ` (${option.hint})` : ""}` : model;
}

/** How much of a description the list row shows before eliding. Long enough for
 *  a real "use this when …" sentence, short enough that the row stays scannable. */
const DESCRIPTION_MAX = 90;

/**
 * The one-line form of an agent's description. Frontmatter descriptions run to
 * whole paragraphs — trigger lists, examples — so the row shows the first
 * sentence, capped, rather than a wall of text ellipsised by the column width.
 * The full text stays available as the row's tooltip.
 */
export function shortDescription(description: string): string {
  const flat = description.replace(/\s+/g, " ").trim();
  const text = flat.match(/^.*?[.!?](?=\s|$)/)?.[0] ?? flat;
  if (text.length <= DESCRIPTION_MAX) return text;
  const cut = text.slice(0, DESCRIPTION_MAX);
  const space = cut.lastIndexOf(" ");
  const kept = space > DESCRIPTION_MAX / 2 ? cut.slice(0, space) : cut;
  return `${kept.replace(/[,;:.\s]+$/, "")}…`;
}

/**
 * Claude's `memory` frontmatter scopes, in the order the editor offers them.
 * An absent key is "off" — the agent starts every invocation with nothing
 * remembered — so "off" is not a scope value, it is the key's absence.
 */
export const MEMORY_SCOPES: ReadonlyArray<{ key: string; label: string; hint: string }> = [
  { key: "local", label: "Session", hint: "Remembers within one session, forgotten after" },
  { key: "project", label: "Project", hint: "Kept in this repo's .claude/agent-memory/<name>/" },
  { key: "user", label: "Everywhere", hint: "Follows the agent across every project" },
];

/** Display name for a `memory` scope. An unrecognized scope — a newer claude,
 *  or a hand-edited file — is shown as-is rather than silently called "off". */
export function memoryLabel(memory: string | null): string {
  if (!memory) return "Off";
  return MEMORY_SCOPES.find((s) => s.key === memory)?.label ?? memory;
}

/** True when the file carries a scope this editor doesn't offer, so the UI can
 *  say so instead of pretending one of its own radios is selected. */
export function hasUnknownMemoryScope(memory: string | null): boolean {
  return !!memory && !MEMORY_SCOPES.some((s) => s.key === memory);
}

/** Which list the top switch shows. System = agents an installed plugin ships. */
export type AgentTab = "yours" | "system";
/** Global lists every agent you own; project narrows to the repo's own. */
export type AgentScope = "global" | "project";

export type EditorTab = "setup" | "prompt" | "capabilities" | "permissions" | "collaborators";

export const EDITOR_TABS: ReadonlyArray<{ key: EditorTab; label: string }> = [
  { key: "setup", label: "Setup" },
  { key: "prompt", label: "Prompt" },
  { key: "capabilities", label: "Capabilities" },
  { key: "permissions", label: "Permissions" },
  { key: "collaborators", label: "Collaborators" },
];

/**
 * The built-in tools an agent's frontmatter can name. Claude Code accepts any
 * tool id — MCP globs included — so this is the set the editor offers as
 * toggles, not a closed vocabulary: anything else in the file is preserved.
 */
export const AGENT_TOOLS: ReadonlyArray<{ key: string; desc: string }> = [
  { key: "Read", desc: "Read files in the worktree" },
  { key: "Write", desc: "Create new files" },
  { key: "Edit", desc: "Modify existing files" },
  { key: "Bash", desc: "Run shell commands" },
  { key: "Glob", desc: "Find files by pattern" },
  { key: "Grep", desc: "Search file contents" },
  { key: "WebFetch", desc: "Fetch a URL" },
  { key: "WebSearch", desc: "Search the web" },
  { key: "Task", desc: "Delegate to a sub-agent" },
  { key: "TodoWrite", desc: "Keep a visible task list" },
  { key: "NotebookEdit", desc: "Edit Jupyter notebooks" },
  { key: "SlashCommand", desc: "Run a saved prompt" },
];

/** The tools a read-only agent keeps. */
const READ_ONLY_TOOLS = ["Read", "Glob", "Grep"];

/** An empty `tools` list means the agent inherits every tool its parent has —
 *  materially different from an agent with none, so the UI says so. */
export function inheritsAllTools(agent: AgentFile): boolean {
  return agent.tools.length === 0;
}

export function toolEnabled(agent: AgentFile, key: string): boolean {
  return inheritsAllTools(agent) || agent.tools.includes(key);
}

/** Turning one tool off has to spell out the rest, since the file expresses
 *  "everything" by omission. Entries the editor doesn't model ride along. */
export function setTool(agent: AgentFile, key: string, on: boolean): string[] {
  const base = inheritsAllTools(agent) ? AGENT_TOOLS.map((t) => t.key) : agent.tools;
  if (!on) return base.filter((t) => t !== key);
  return base.includes(key) ? [...base] : [...base, key];
}

export function readOnlyTools(agent: AgentFile): string[] {
  const extras = agent.tools.filter((t) => !AGENT_TOOLS.some((x) => x.key === t));
  return [...READ_ONLY_TOOLS, ...extras];
}

/** "5 of 12 enabled …" for the Capabilities header. */
export function toolSummary(agent: AgentFile): string {
  if (inheritsAllTools(agent)) {
    return "Inherits every tool from the session that spawns it — turn one off to pin an explicit list";
  }
  const known = agent.tools.filter((t) => AGENT_TOOLS.some((x) => x.key === t)).length;
  return `${known} of ${AGENT_TOOLS.length} enabled · disabled tools are hidden from the model entirely`;
}

/** Tool entries the file names that aren't built-ins or MCP globs — anything
 *  from a newer claude. Surfaced so a save can't look like it dropped them. */
export function extraTools(tools: readonly string[]): string[] {
  return tools.filter(
    (t) => !t.startsWith("mcp__") && !AGENT_TOOLS.some((x) => x.key === t),
  );
}

/** An agent may use `server` when its tools name any of that server's tools. */
export function mcpEnabled(agent: AgentFile, server: string): boolean {
  return agent.tools.some((t) => t.startsWith(`mcp__${server}__`));
}

/** The server's tool entries this agent names, or the wildcard when enabled
 *  wholesale. Empty when the server is off. */
export function mcpTools(agent: AgentFile, server: string): string[] {
  return agent.tools.filter((t) => t.startsWith(`mcp__${server}__`));
}

export function setMcp(agent: AgentFile, server: string, on: boolean): string[] {
  const base = inheritsAllTools(agent) ? AGENT_TOOLS.map((t) => t.key) : agent.tools;
  const without = base.filter((t) => !t.startsWith(`mcp__${server}__`));
  return on ? [...without, `mcp__${server}__*`] : without;
}

/** A blank agent for `source`. `path` is assigned by the backend once the name
 *  is known, so an unsaved agent carries an empty one. */
export function newAgent(source: string): AgentFile {
  return {
    path: "",
    source,
    plugin: null,
    name: "",
    description: "",
    tools: [],
    model: null,
    color: null,
    memory: null,
    prompt: "",
    updatedAt: null,
    shippedPath: null,
    modified: false,
  };
}

/** True while an agent has never been written — the editor's "new" state. */
export function isUnsaved(agent: AgentFile): boolean {
  return agent.path === "";
}

/** A plugin ships this agent; its file is replaced on the plugin's next update,
 *  so edits are saved as a personal override instead. */
export function isSystem(agent: AgentFile): boolean {
  return agent.source === "plugin";
}

/** A personal agent is available everywhere; a project one is scoped to its
 *  repo. Which directory holds the file IS the scope. */
export function isAllProjects(agent: AgentFile): boolean {
  return agent.source !== "project";
}

/** An overlay with every field present — what the editor works against. The
 *  generated type has them all optional (serde `default`). */
export type Overlay = Required<AgentOverlay>;

export const DEFAULT_OVERLAY: Overlay = {
  effort: "high",
  harness: "claude",
  folder: "",
  status: "published",
  permissionMode: "default",
  rules: [],
  network: true,
  outsideWorktree: false,
  context: [],
  delegates: [],
  visibility: "private",
  collaborators: [],
};

export function overlayFor(library: AgentLibrarySettings | undefined, path: string): Overlay {
  return { ...DEFAULT_OVERLAY, ...(library?.overlays?.[path] ?? {}) };
}

/** Status badge text and tone. A plugin agent is always live; yours is a draft
 *  until published. */
export function statusOf(
  agent: AgentFile,
  overlay: Overlay,
): { label: string; tone: "ok" | "warn" } {
  if (isSystem(agent)) return { label: "Active", tone: "ok" };
  return overlay.status === "draft"
    ? { label: "Draft", tone: "warn" }
    : { label: "Published", tone: "ok" };
}

/** claude's frontmatter `color` names one of its own palette entries; map it to
 *  a theme token so the row dot matches the app rather than a raw CSS colour. */
const DOT_COLORS: Record<string, string> = {
  red: "var(--color-bad)",
  orange: "var(--color-accent)",
  yellow: "var(--color-warn)",
  green: "var(--color-ok)",
  blue: "var(--color-research)",
  purple: "var(--color-validate)",
  pink: "var(--color-revise)",
  cyan: "var(--color-implement)",
};

const MUTED_DOT = "color-mix(in oklab, var(--color-ink-dim) 55%, transparent)";

export function dotColor(agent: AgentFile, overlay: Overlay): string {
  if (!isSystem(agent) && overlay.status === "draft") return "var(--color-warn)";
  return (agent.color && DOT_COLORS[agent.color.toLowerCase()]) ?? MUTED_DOT;
}

/** The agents a tab + scope + search admits, in the order the list renders. */
export function visibleAgents(
  agents: readonly AgentFile[],
  filter: { tab: AgentTab; scope: AgentScope; search: string },
): AgentFile[] {
  const q = filter.search.trim().toLowerCase();
  return agents.filter((a) => {
    if (isSystem(a) !== (filter.tab === "system")) return false;
    if (filter.tab === "yours" && filter.scope === "project" && a.source !== "project") return false;
    if (!q) return true;
    return a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q);
  });
}

/** A folder header, or an agent. The list renders these in order. */
export type AgentRow =
  | { kind: "group"; folder: string; count: number; open: boolean }
  | { kind: "agent"; agent: AgentFile; indent: number };

/**
 * Loose agents first, then each folder that has something in it. A collapsed
 * folder still renders its header; an empty one is dropped rather than shown
 * as a dead row.
 */
export function agentRows(
  agents: readonly AgentFile[],
  library: AgentLibrarySettings | undefined,
  folders: readonly string[],
  openFolders: ReadonlySet<string>,
): AgentRow[] {
  const folderOf = (a: AgentFile) => overlayFor(library, a.path).folder;
  const rows: AgentRow[] = [];
  for (const agent of agents) {
    if (!folderOf(agent)) rows.push({ kind: "agent", agent, indent: 0 });
  }
  for (const folder of folders) {
    const inside = agents.filter((a) => folderOf(a) === folder);
    if (inside.length === 0) continue;
    const open = openFolders.has(folder);
    rows.push({ kind: "group", folder, count: inside.length, open });
    if (open) for (const agent of inside) rows.push({ kind: "agent", agent, indent: 14 });
  }
  return rows;
}

/** Length + a rough token count for the prompt editor's counter. ~3.8 chars per
 *  token is the usual English ratio; exact counting needs the tokenizer. */
export function promptStats(prompt: string): string {
  return `${prompt.length} chars · ~${Math.max(1, Math.round(prompt.length / 3.8))} tokens`;
}

/** A copy of `agent` under a new name, unsaved until it is given a path. A
 *  plugin agent duplicates into your own library, losing its plugin origin. */
export function duplicateAgent(agent: AgentFile): AgentFile {
  return {
    ...structuredClone(agent),
    path: "",
    plugin: null,
    source: isSystem(agent) ? "user" : agent.source,
    name: `${agent.name} copy`,
  };
}

/** The blank rule "Add rule" appends. Ids only have to be unique within the
 *  agent, and the list is short. */
export function newRule(existing: readonly { id: string }[]): {
  id: string;
  action: string;
  pattern: string;
} {
  const next = existing.reduce((max, r) => Math.max(max, Number(r.id.slice(1)) || 0), 0) + 1;
  return { id: `r${next}`, action: "ask", pattern: "Bash(:*)" };
}

/** Name and initials for an invited email, the way the People list shows them. */
export function collaboratorFor(email: string): {
  name: string;
  email: string;
  role: string;
} {
  const name = email.split("@")[0].replace(/[._-]+/g, " ").trim();
  return { name: name || email, email, role: "Editor" };
}

export function initialsOf(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}
